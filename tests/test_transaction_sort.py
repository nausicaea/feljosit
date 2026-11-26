"""The sort order of the transaction list."""

import re
from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select

from feljosit.models import Account, AccountType, User
from feljosit.services import accounts

Client = tuple[TestClient, str]


@pytest.fixture
def data(client: Client, app: FastAPI) -> None:
    """Transactions in CHF and in JPY (no decimal places)."""
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None

    def account(name: str, currency: str) -> Account:
        return accounts.create_account(
            db,
            user.id,
            name=name,
            account_type=AccountType.CHECKING,
            currency=currency,
            budget_currency=currency,
        )

    chf, jpy = account("Private", "CHF"), account("Yen", "JPY")
    for acc, day, amount, name in [
        (chf, 1, -4500, "Migros"),  # −45.00
        (chf, 2, 250000, "Salary"),  # 2'500.00
        (chf, 3, -120000, "Rent"),  # −1'200.00
        (jpy, 4, -3000, "Ramen"),  # −3'000 JPY
        (chf, 5, -700, "Kiosk"),  # −7.00
    ]:
        accounts.add_manual_transaction(
            db, acc, booking_date=date(2026, 10, day), amount_minor=amount, counterparty=name
        )
    db.commit()
    db.close()


def _order(page: str) -> list[str]:
    return re.findall(r'<td class="tx-party">\s*<strong>([^<]+)</strong>', page)


@pytest.mark.parametrize(
    ("sort", "expected"),
    [
        ("", ["Kiosk", "Ramen", "Rent", "Salary", "Migros"]),
        ("amount_asc", ["Ramen", "Rent", "Migros", "Kiosk", "Salary"]),
        ("amount_desc", ["Salary", "Kiosk", "Migros", "Rent", "Ramen"]),
        ("size_desc", ["Ramen", "Salary", "Rent", "Migros", "Kiosk"]),
        ("unknown", ["Kiosk", "Ramen", "Rent", "Salary", "Migros"]),
    ],
)
def test_sort(client: Client, data: None, sort: str, expected: list[str]) -> None:
    http, _ = client
    page = http.get("/transactions", params={"sort": sort})
    assert page.status_code == 200
    assert _order(page.text) == expected
    selected = sort if sort != "unknown" else ""
    assert f'<option value="{selected}" selected>' in page.text


def test_sort_stays_with_filters_and_changes(client: Client, data: None) -> None:
    http, _ = client
    page = http.get("/transactions", params={"sort": "amount_asc", "direction": "outflow"}).text
    assert _order(page) == ["Ramen", "Rent", "Migros", "Kiosk"]
    assert 'value="/transactions?direction=outflow&amp;sort=amount_asc&amp;page=1"' in page
    # "Remove the filters" keeps the sort order.
    assert '<a href="/transactions?sort=amount_asc">Remove the filters</a>' in page


def test_redirect_keeps_the_sort(client: Client, app: FastAPI, data: None) -> None:
    http, token = client
    response = http.post(
        "/transactions/1/memo",
        data={"csrf_token": token, "memo": "x", "return_to": "/transactions?sort=size_desc"},
        follow_redirects=False,
    )
    assert response.headers["location"] == "/transactions?sort=size_desc"
