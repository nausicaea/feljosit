"""End-to-end tests for the web application, including security controls."""

from datetime import date

from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select

from feljosit.models import Account, Category, Transaction, User
from tests.conftest import PASSWORD, create_app_user, csrf_from, fixture_bytes, login

Client = tuple[TestClient, str]


def _import(client: TestClient, token: str, name: str, account_id: str = "") -> str:
    response = client.post(
        "/import",
        data={"csrf_token": token, "account_id": account_id},
        files={"statement": (name, fixture_bytes(name), "application/xml")},
    )
    return response.text


def _add_account(client: TestClient, token: str, **data: str) -> None:
    payload = {"csrf_token": token, "name": "Private", "account_type": "checking"} | data
    response = client.post("/accounts", data=payload)
    assert response.status_code == 200


def test_pages_need_login(app: FastAPI) -> None:
    client = TestClient(app, base_url="http://127.0.0.1")
    for path in ["/budget", "/accounts", "/transactions", "/import", "/categories", "/reports"]:
        response = client.get(path, follow_redirects=False)
        assert response.status_code == 303
        assert response.headers["location"] == "/login"


def test_login_wrong_password_and_lockout(app: FastAPI) -> None:
    create_app_user(app, "anna")
    client = TestClient(app, base_url="http://127.0.0.1")
    for _ in range(5):
        page = client.get("/login")
        response = client.post(
            "/login",
            data={"username": "anna", "password": "wrong", "csrf_token": csrf_from(page.text)},
        )
        assert response.status_code == 401
    page = client.get("/login")
    response = client.post(
        "/login",
        data={"username": "anna", "password": PASSWORD, "csrf_token": csrf_from(page.text)},
    )
    assert response.status_code == 401  # Locked.


def test_login_needs_csrf_cookie(app: FastAPI) -> None:
    create_app_user(app, "anna")
    client = TestClient(app, base_url="http://127.0.0.1")
    response = client.post(
        "/login", data={"username": "anna", "password": PASSWORD, "csrf_token": "forged"}
    )
    assert response.status_code == 400


def test_session_cookie_flags(app: FastAPI) -> None:
    create_app_user(app, "anna")
    client = TestClient(app, base_url="http://127.0.0.1")
    page = client.get("/login")
    response = client.post(
        "/login",
        data={"username": "anna", "password": PASSWORD, "csrf_token": csrf_from(page.text)},
        follow_redirects=False,
    )
    cookie = response.headers["set-cookie"].lower()
    assert "httponly" in cookie
    assert "samesite=strict" in cookie


def test_security_headers(client: Client) -> None:
    http, _ = client
    response = http.get("/budget")
    csp = response.headers["content-security-policy"]
    assert "default-src 'none'" in csp
    assert "script-src 'self'" in csp
    assert "frame-ancestors 'none'" in csp
    assert response.headers["x-content-type-options"] == "nosniff"
    assert response.headers["cache-control"] == "no-store"
    # "no-referrer" makes browsers send "Origin: null". This breaks the Origin check.
    assert response.headers["referrer-policy"] == "same-origin"


def test_no_api_docs(app: FastAPI) -> None:
    client = TestClient(app, base_url="http://127.0.0.1")
    for path in ["/docs", "/redoc", "/openapi.json"]:
        assert client.get(path).status_code == 404


def test_unknown_host_refused(app: FastAPI) -> None:
    """DNS rebinding protection."""
    client = TestClient(app, base_url="http://attacker.example")
    assert client.get("/login").status_code == 400


def test_post_without_csrf_token_refused(client: Client) -> None:
    http, _ = client
    response = http.post("/accounts", data={"name": "X", "account_type": "cash"})
    assert response.status_code == 403
    response = http.post(
        "/accounts", data={"name": "X", "account_type": "cash", "csrf_token": "wrong"}
    )
    assert response.status_code == 403


def test_cross_origin_post_refused(client: Client) -> None:
    http, token = client
    response = http.post(
        "/accounts",
        data={"name": "X", "account_type": "cash", "csrf_token": token},
        headers={"Origin": "http://evil.example"},
    )
    assert response.status_code == 403


def test_logout_ends_session(client: Client) -> None:
    http, token = client
    http.post("/logout", data={"csrf_token": token})
    assert http.get("/budget", follow_redirects=False).status_code == 303


def test_body_size_limit(client: Client) -> None:
    http, token = client
    big = b"x" * (11 * 1024 * 1024)
    response = http.post(
        "/import",
        data={"csrf_token": token},
        files={"statement": ("big.xml", big, "application/xml")},
    )
    assert response.status_code == 413


def test_full_flow_import_categorise_budget_reports(client: Client, app: FastAPI) -> None:
    http, token = client
    _add_account(http, token, iban="CH89 0023 5000 0123 4567 8")
    text = _import(http, token, "camt053_v04_private.xml")
    assert "7 new transactions" in text
    assert "agrees with the account" in text
    # Second import of the same file.
    assert "imported before" in _import(http, token, "camt053_v04_private.xml")

    db = app.state.session_factory()
    tx = db.scalar(select(Transaction).where(Transaction.counterparty == "Muster AG"))
    salary = db.scalar(select(Category).where(Category.name == "Salary"))
    groceries = db.scalar(select(Category).where(Category.name == "Groceries"))
    assert tx is not None and salary is not None and groceries is not None
    db.close()

    response = http.post(
        f"/transactions/{tx.id}/category",
        data={"csrf_token": token, "category_id": str(salary.id), "return_to": "/transactions"},
    )
    assert response.status_code == 200
    month = date(2026, 10, 1).strftime("%Y-%m")
    response = http.post(
        "/budget/assign",
        data={
            "csrf_token": token,
            "category_id": str(groceries.id),
            "month": month,
            "amount": "1'000.00",
        },
    )
    assert response.status_code == 200
    page = http.get(f"/budget?month={month}").text
    # 1000.00 starting balance + 6500.00 salary - 1000.00 assigned.
    assert "CHF 6&#39;500.00" in page or "CHF 6'500.00" in page
    assert http.get("/reports?period=24m").status_code == 200
    assert "Muster AG" in http.get("/transactions?q=muster").text


def test_import_errors_are_shown(client: Client) -> None:
    http, token = client
    response = http.post(
        "/import",
        data={"csrf_token": token},
        files={"statement": ("x.xml", b"<not-camt/>", "application/xml")},
    )
    assert response.status_code == 400
    assert "no ISO 20022 namespace" in response.text
    response = http.post(
        "/import",
        data={"csrf_token": token},
        files={"statement": ("x.xml", b'<D xmlns="urn:x"/>', "application/xml")},
    )
    assert response.status_code == 400
    assert "not a camt.053" in response.text


def test_html_is_escaped(client: Client) -> None:
    http, token = client
    _add_account(http, token, name="<script>alert(1)</script>")
    page = http.get("/accounts").text
    assert "<script>alert(1)</script>" not in page
    assert "&lt;script&gt;" in page


def test_users_cannot_see_or_change_data_of_other_users(app: FastAPI) -> None:
    create_app_user(app, "anna")
    create_app_user(app, "bob")
    anna, anna_token = login(app, "anna")
    bob, bob_token = login(app, "bob")
    _add_account(anna, anna_token, name="Anna secret account", iban="CH8900235000012345678")
    _import(anna, anna_token, "camt053_v04_private.xml")

    db = app.state.session_factory()
    anna_user = db.scalar(select(User).where(User.username == "anna"))
    assert anna_user is not None
    account = db.scalar(select(Account).where(Account.user_id == anna_user.id))
    tx = db.scalar(select(Transaction).where(Transaction.user_id == anna_user.id))
    category = db.scalar(select(Category).where(Category.user_id == anna_user.id))
    assert account is not None and tx is not None and category is not None
    db.close()

    assert "Anna secret account" not in bob.get("/accounts").text
    page = bob.get(f"/transactions?account={account.id}").text
    assert "Muster AG" not in page
    for path, data in [
        (f"/accounts/{account.id}/close", {}),
        (f"/transactions/{tx.id}/category", {"category_id": ""}),
        (f"/transactions/{tx.id}/memo", {"memo": "hacked"}),
        (f"/categories/{category.id}/rename", {"name": "hacked"}),
        ("/budget/assign", {"category_id": str(category.id), "month": "2026-10", "amount": "1"}),
    ]:
        response = bob.post(path, data={"csrf_token": bob_token, **data})
        assert response.status_code == 404, path
    # Bob cannot import into the account of Anna.
    response = bob.post(
        "/import",
        data={"csrf_token": bob_token, "account_id": str(account.id)},
        files={"statement": ("s.xml", fixture_bytes("camt053_v08_savings.xml"), "text/xml")},
    )
    assert response.status_code == 404


def test_open_redirect_prevented(client: Client) -> None:
    http, token = client
    _add_account(http, token, name="Cash", account_type="cash")
    response = http.post(
        "/transactions",
        data={
            "csrf_token": token,
            "account_id": "1",
            "booking_date": "2026-01-01",
            "amount": "5",
            "direction": "outflow",
        },
    )
    assert response.status_code == 200
    response = http.post(
        "/transactions/1/memo",
        data={"csrf_token": token, "memo": "x", "return_to": "//evil.example/"},
        follow_redirects=False,
    )
    assert response.headers["location"] == "/transactions"


def test_change_password_closes_other_sessions(app: FastAPI) -> None:
    create_app_user(app, "anna")
    first, first_token = login(app, "anna")
    second, _ = login(app, "anna")
    response = first.post(
        "/settings/password",
        data={
            "csrf_token": first_token,
            "current_password": PASSWORD,
            "new_password": "a new long password",
            "new_password_repeat": "a new long password",
        },
    )
    assert response.status_code == 200
    assert "password is changed" in response.text
    assert second.get("/budget", follow_redirects=False).status_code == 303
    assert first.get("/budget", follow_redirects=False).status_code == 200


def test_validation_error_does_not_echo_input(client: Client) -> None:
    http, token = client
    response = http.post(
        "/accounts", data={"csrf_token": token, "name": "x", "account_type": "<b>evil</b>"}
    )
    assert response.status_code == 422
    assert "evil" not in response.text


def test_every_form_has_a_csrf_token(client: Client) -> None:
    """Each POST form must contain a non-empty CSRF token (regression test)."""
    import re

    http, token = client
    _add_account(http, token, iban="CH8900235000012345678")
    http.post(
        "/import",
        data={"csrf_token": token},
        files={"statement": ("a.xml", fixture_bytes("camt053_v04_private.xml"), "text/xml")},
    )
    for path in ["/budget", "/accounts", "/transactions", "/import", "/categories", "/settings"]:
        html = http.get(path).text
        forms = re.findall(r'<form method="post".*?</form>', html, flags=re.DOTALL)
        assert forms, path
        for form in forms:
            assert f'name="csrf_token" value="{token}"' in form, (path, form[:200])
