"""The amount and direction filters and the bulk category change of the transaction list."""

from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select

from feljosit.models import Account, AccountType, Category, Transaction, User
from feljosit.services import accounts, transfers

Client = tuple[TestClient, str]
NAMES = ["Premium", "Deductible", "Small bill", "Refund", "Card payment", "Payment", "Deposit"]


@pytest.fixture
def data(client: Client, app: FastAPI) -> dict[str, int]:
    """Transactions of a health insurance and other transactions. Return the IDs."""
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None

    def account(name: str, kind: AccountType = AccountType.CHECKING) -> Account:
        return accounts.create_account(
            db, user.id, name=name, account_type=kind, currency="CHF", budget_currency="CHF"
        )

    def tx(acc: Account, day: int, amount: int, name: str) -> Transaction:
        return accounts.add_manual_transaction(
            db, acc, booking_date=date(2026, 10, day), amount_minor=amount, counterparty=name
        )

    private, card = account("Private"), account("Card")
    pillar = account("3a", AccountType.PILLAR_3A)
    ids = {
        "premium": tx(private, 1, -45_050, "Premium").id,
        "deductible": tx(private, 2, -30_000, "Deductible").id,
        "small": tx(private, 3, -2_995, "Small bill").id,
        "refund": tx(private, 4, 12_000, "Refund").id,
        "card_payment": tx(private, 5, -77_700, "Card payment").id,  # A pair with "Payment".
        "payment": tx(card, 6, 77_700, "Payment").id,
        "deposit": tx(pillar, 7, 50_000, "Deposit").id,  # Tracking account.
    }
    db.flush()
    transfers.match_transfers(db, user.id)
    health = db.scalar(select(Category).where(Category.user_id == user.id))
    assert health is not None
    ids["category"] = health.id
    db.commit()
    db.close()
    return ids


def _names(page: str) -> set[str]:
    return {name for name in NAMES if f"<strong>{name}</strong>" in page}


@pytest.mark.parametrize(
    ("params", "expected"),
    [
        ({"direction": "outflow"}, {"Premium", "Deductible", "Small bill", "Card payment"}),
        ({"direction": "inflow"}, {"Refund", "Payment", "Deposit"}),
        ({"direction": "abc"}, set(NAMES)),
        ({"min": "300"}, {"Premium", "Deductible", "Card payment", "Payment", "Deposit"}),
        ({"min": "300.01"}, {"Premium", "Card payment", "Payment", "Deposit"}),
        ({"max": "300"}, {"Deductible", "Small bill", "Refund"}),
        ({"min": "100", "max": "500", "direction": "outflow"}, {"Premium", "Deductible"}),
        ({"max": "1'000", "direction": "inflow"}, {"Refund", "Payment", "Deposit"}),
        ({"min": "-300", "direction": "outflow"}, {"Premium", "Deductible", "Card payment"}),
        ({"min": "29,95", "max": "29.95"}, {"Small bill"}),
    ],
)
def test_amount_and_direction_filter(
    client: Client, data: dict[str, int], params: dict[str, str], expected: set[str]
) -> None:
    http, _ = client
    page = http.get("/transactions", params=params)
    assert page.status_code == 200
    assert _names(page.text) == expected


def test_invalid_amount_filter_shows_an_error(client: Client, data: dict[str, int]) -> None:
    http, _ = client
    page = http.get("/transactions", params={"min": "abc"}).text
    assert "The amount filter is not valid." in page
    assert _names(page) == set(NAMES)
    page = http.get("/transactions", params={"min": "500", "max": "100"}).text
    assert "The minimum amount is more than the maximum amount." in page
    assert _names(page) == set()


def test_filters_stay_in_the_links(client: Client, data: dict[str, int]) -> None:
    http, _ = client
    page = http.get("/transactions", params={"direction": "outflow", "min": "300"}).text
    assert '<option value="outflow" selected>' in page
    assert 'name="min" value="300"' in page
    assert 'name="return_to" value="/transactions?direction=outflow&amp;min=300&amp;page=1"' in page


def _bulk(http: TestClient, token: str, ids: list[int], category: str) -> str:
    response = http.post(
        "/transactions/bulk-category",
        data={
            "csrf_token": token,
            "tx_id": [str(i) for i in ids],
            "category_id": category,
            "return_to": "/transactions?direction=outflow&min=300&evil=1&page=1",
        },
        follow_redirects=False,
    )
    assert response.status_code == 303
    location: str = response.headers["location"]
    assert location == "/transactions?direction=outflow&min=300&page=1"
    return http.get(location).text


def _state(app: FastAPI, tx_id: int) -> tuple[int | None, bool]:
    db = app.state.session_factory()
    tx = db.get(Transaction, tx_id)
    assert tx is not None
    state = (tx.category_id, tx.is_transfer)
    db.close()
    return state


def test_bulk_category(client: Client, app: FastAPI, data: dict[str, int]) -> None:
    http, token = client
    page = _bulk(http, token, [data["premium"], data["deductible"]], str(data["category"]))
    assert "The category of 2 transactions is changed." in page
    assert _state(app, data["premium"]) == (data["category"], False)
    assert _state(app, data["deductible"]) == (data["category"], False)
    assert _state(app, data["small"]) == (None, False)
    page = _bulk(http, token, [data["deductible"]], "none")
    assert "The category of 1 transactions is changed." in page
    assert _state(app, data["deductible"]) == (None, False)
    _bulk(http, token, [data["small"]], "transfer")
    assert _state(app, data["small"]) == (None, True)


def test_bulk_category_ends_a_transfer_pair(
    client: Client, app: FastAPI, data: dict[str, int]
) -> None:
    http, token = client
    assert _state(app, data["payment"]) == (None, True)
    page = _bulk(http, token, [data["card_payment"]], str(data["category"]))
    assert "1 other transactions are not transfers any more." in page
    assert _state(app, data["card_payment"]) == (data["category"], False)
    assert _state(app, data["payment"]) == (None, False)


def test_bulk_category_with_both_sides_of_a_pair(
    client: Client, app: FastAPI, data: dict[str, int]
) -> None:
    http, token = client
    page = _bulk(http, token, [data["card_payment"], data["payment"]], str(data["category"]))
    assert "other transactions" not in page
    assert _state(app, data["card_payment"]) == (data["category"], False)
    assert _state(app, data["payment"]) == (data["category"], False)


def test_bulk_category_skips_tracking_accounts(
    client: Client, app: FastAPI, data: dict[str, int]
) -> None:
    http, token = client
    page = _bulk(http, token, [data["premium"], data["deposit"]], str(data["category"]))
    assert "The category of 1 transactions is changed." in page
    assert "1 transactions are in tracking accounts." in page
    assert _state(app, data["deposit"]) == (None, False)


@pytest.mark.parametrize(
    ("ids", "category", "message"),
    [
        ([], "category", "Select one or more transactions."),
        (["premium"], "", "Select a category."),
        (["premium"], "abc", "Select a category."),
    ],
)
def test_bulk_category_errors(
    client: Client,
    app: FastAPI,
    data: dict[str, int],
    ids: list[str],
    category: str,
    message: str,
) -> None:
    http, token = client
    value = str(data["category"]) if category == "category" else category
    page = _bulk(http, token, [data[i] for i in ids], value)
    assert message in page
    assert _state(app, data["premium"]) == (None, False)


def test_bulk_category_of_another_user(client: Client, app: FastAPI, data: dict[str, int]) -> None:
    """An unknown transaction or category: nothing changes."""
    http, token = client
    data_ids = [data["premium"], 999_999]
    response = http.post(
        "/transactions/bulk-category",
        data={"csrf_token": token, "tx_id": data_ids, "category_id": str(data["category"])},
        follow_redirects=False,
    )
    assert response.status_code == 404
    response = http.post(
        "/transactions/bulk-category",
        data={"csrf_token": token, "tx_id": [data["premium"]], "category_id": "999999"},
        follow_redirects=False,
    )
    assert response.status_code == 404
    assert _state(app, data["premium"]) == (None, False)


def test_bulk_category_needs_the_csrf_token(
    client: Client, app: FastAPI, data: dict[str, int]
) -> None:
    http, _ = client
    response = http.post(
        "/transactions/bulk-category",
        data={"tx_id": [data["premium"]], "category_id": str(data["category"])},
        follow_redirects=False,
    )
    assert response.status_code == 403
    assert _state(app, data["premium"]) == (None, False)


def test_page_has_the_bulk_form(client: Client, data: dict[str, int]) -> None:
    http, _ = client
    page = http.get("/transactions").text
    assert 'action="/transactions/bulk-category"' in page
    assert f'name="tx_id" value="{data["premium"]}" form="bulk"' in page
    # A tracking account has no category, and thus no checkbox.
    assert f'name="tx_id" value="{data["deposit"]}"' not in page
