"""The category filter of the transaction list, with the transfer choices."""

from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select

from feljosit.models import Account, AccountType, Category, Transaction, User
from feljosit.services import accounts, transfers

Client = tuple[TestClient, str]


@pytest.fixture
def data(client: Client, app: FastAPI) -> dict[str, int]:
    """Transactions of each kind. Return the IDs that the tests need."""
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None

    def account(name: str, kind: AccountType = AccountType.CHECKING) -> Account:
        return accounts.create_account(
            db, user.id, name=name, account_type=kind, currency="CHF", budget_currency="CHF"
        )

    def tx(
        acc: Account, day: int, amount: int, name: str, category_id: int | None = None
    ) -> Transaction:
        return accounts.add_manual_transaction(
            db,
            acc,
            booking_date=date(2026, 10, day),
            amount_minor=amount,
            counterparty=name,
            category_id=category_id,
        )

    private, card = account("Private"), account("Card")
    pillar = account("3a", AccountType.PILLAR_3A)
    groceries = db.scalar(select(Category).where(Category.name == "Groceries"))
    assert groceries is not None
    tx(private, 1, -3000, "Card payment")  # A matched pair with the next one.
    tx(card, 2, 3000, "Payment received")
    manual = tx(private, 3, -1000, "Manual transfer")
    manual.is_transfer = True  # Selected manually as "Transfer": no pair.
    tx(private, 4, -4500, "Migros", groceries.id)
    tx(private, 5, -700, "Kiosk")  # Uncategorised.
    tx(pillar, 6, 50000, "Deposit")  # Tracking account: not "uncategorised".
    db.flush()
    transfers.match_transfers(db, user.id)
    db.commit()
    ids = {"groceries": groceries.id, "private": private.id}
    db.close()
    return ids


def _names(page: str) -> set[str]:
    candidates = [
        "Card payment",
        "Payment received",
        "Manual transfer",
        "Migros",
        "Kiosk",
        "Deposit",
    ]
    return {name for name in candidates if f"<strong>{name}</strong>" in page}


@pytest.mark.parametrize(
    ("category", "expected"),
    [
        ("", {"Card payment", "Payment received", "Manual transfer", "Migros", "Kiosk", "Deposit"}),
        ("uncategorised", {"Kiosk"}),
        ("transfers", {"Card payment", "Payment received", "Manual transfer"}),
        ("matched", {"Card payment", "Payment received"}),
        ("unmatched", {"Manual transfer"}),
        (
            "abc",
            {"Card payment", "Payment received", "Manual transfer", "Migros", "Kiosk", "Deposit"},
        ),
    ],
)
def test_category_filter(
    client: Client, data: dict[str, int], category: str, expected: set[str]
) -> None:
    http, _ = client
    page = http.get("/transactions", params={"category": category})
    assert page.status_code == 200
    assert _names(page.text) == expected
    if category in {"uncategorised", "transfers", "matched", "unmatched"}:
        assert f'<option value="{category}" selected>' in page.text


def test_filter_by_category_id_and_account(client: Client, data: dict[str, int]) -> None:
    http, _ = client
    page = http.get("/transactions", params={"category": str(data["groceries"])}).text
    assert _names(page) == {"Migros"}
    assert f'<option value="{data["groceries"]}" selected>' in page
    page = http.get(
        "/transactions", params={"category": "transfers", "account": str(data["private"])}
    ).text
    assert _names(page) == {"Card payment", "Manual transfer"}


@pytest.mark.parametrize(
    ("old", "value"), [("uncategorised", "uncategorised"), ("matched", "matched")]
)
def test_old_links_still_work(client: Client, data: dict[str, int], old: str, value: str) -> None:
    """The budget page, the import result and the reports link with the old parameters."""
    http, _ = client
    page = http.get(f"/transactions?{old}=true").text
    assert f'<option value="{value}" selected>' in page
    # The links of the page use the new parameter.
    assert f'name="return_to" value="/transactions?category={value}&amp;page=1"' in page


def test_filter_stays_after_a_change(client: Client, app: FastAPI, data: dict[str, int]) -> None:
    http, token = client
    db = app.state.session_factory()
    kiosk = db.scalar(select(Transaction).where(Transaction.counterparty == "Kiosk"))
    assert kiosk is not None
    db.close()
    response = http.post(
        f"/transactions/{kiosk.id}/category",
        data={
            "csrf_token": token,
            "category_id": str(data["groceries"]),
            "return_to": "/transactions?category=uncategorised&page=1",
        },
        follow_redirects=False,
    )
    assert response.headers["location"] == "/transactions?category=uncategorised&page=1"
    page = http.get(response.headers["location"]).text
    assert "0 transactions." in page
