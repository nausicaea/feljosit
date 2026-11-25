from fastapi import FastAPI
from sqlalchemy import select

from feljosit.models import CategorisationRule, Category, CategoryGroup
from tests.conftest import fixture_bytes

from .test_web import Client, _add_account


def _session(app: FastAPI):  # type: ignore[no-untyped-def]
    return app.state.session_factory()


def test_categories_groups_and_rules(client: Client, app: FastAPI) -> None:
    http, token = client
    assert "Groceries" in http.get("/categories").text

    http.post("/categories/groups", data={"csrf_token": token, "name": "Hobbies"})
    db = _session(app)
    group = db.scalar(select(CategoryGroup).where(CategoryGroup.name == "Hobbies"))
    assert group is not None
    http.post(
        "/categories",
        data={"csrf_token": token, "group_id": str(group.id), "name": "Climbing"},
    )
    climbing = db.scalar(select(Category).where(Category.name == "Climbing"))
    assert climbing is not None
    http.post(f"/categories/{climbing.id}/rename", data={"csrf_token": token, "name": "Bouldering"})
    http.post(f"/categories/{climbing.id}/hide", data={"csrf_token": token, "hidden": "true"})
    db.expire_all()
    assert climbing.name == "Bouldering"
    assert climbing.hidden

    # A name that is too short is refused with a message.
    page = http.post("/categories/groups", data={"csrf_token": token, "name": "  "}).text
    assert "1 to 100 characters" in page

    _add_account(http, token, iban="CH8900235000012345678")
    http.post(
        "/import",
        data={"csrf_token": token},
        files={"statement": ("a.xml", fixture_bytes("camt053_v04_private.xml"), "text/xml")},
    )
    rent = db.scalar(select(Category).where(Category.name == "Rent and service charges"))
    assert rent is not None
    page = http.post(
        "/rules",
        data={
            "csrf_token": token,
            "field": "counterparty",
            "match_type": "contains",
            "pattern": "Immo",
            "category_id": str(rent.id),
            "priority": "10",
        },
    ).text
    assert "The rule is added" in page
    page = http.post("/rules/apply", data={"csrf_token": token}).text
    assert "1 transactions are categorised" in page
    rule = db.scalar(select(CategorisationRule))
    assert rule is not None
    http.post(f"/rules/{rule.id}/delete", data={"csrf_token": token})
    db.expire_all()
    assert db.scalar(select(CategorisationRule)) is None
    db.close()


def test_manual_transaction_flow(client: Client) -> None:
    http, token = client
    _add_account(http, token, name="Wallet", account_type="cash", starting_balance="200")
    page = http.post(
        "/transactions",
        data={
            "csrf_token": token,
            "account_id": "1",
            "booking_date": "2026-03-01",
            "amount": "12.50",
            "direction": "outflow",
            "counterparty": "Bakery",
        },
    ).text
    assert "The transaction is added" in page
    assert "-12.50" in page
    page = http.post(
        "/transactions",
        data={
            "csrf_token": token,
            "account_id": "1",
            "booking_date": "2026-03-01",
            "amount": "abc",
            "direction": "inflow",
        },
    ).text
    assert "not valid" in page
    page = http.post(
        "/transactions/2/delete", data={"csrf_token": token, "return_to": "/transactions"}
    ).text
    assert "is deleted" in page
    # The starting balance is not a manual transaction.
    page = http.post("/transactions/1/delete", data={"csrf_token": token}).text
    assert "only delete" in page
    page = http.get("/accounts").text
    assert "200.00" in page
    page = http.post("/accounts/1/close", data={"csrf_token": token, "closed": "true"}).text
    assert "(closed)" in page
