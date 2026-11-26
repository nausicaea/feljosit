"""The report "Spending by category" over time."""

from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Account, AccountType, Category, User
from feljosit.services import accounts, reports
from feljosit.services.budget import add_months
from feljosit.web.routes.reports import _heat, _sorted_rows


def _setup(db: Session, user: User) -> tuple[Account, dict[str, int]]:
    account = accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )
    cats = {c.name: c.id for c in db.scalars(select(Category).where(Category.user_id == user.id))}
    return account, cats


def _spend(
    db: Session,
    account: Account,
    on: date,
    amount: int,
    category_id: int | None,
    counterparty: str = "x",
) -> None:
    accounts.add_manual_transaction(
        db,
        account,
        booking_date=on,
        amount_minor=amount,
        counterparty=counterparty,
        category_id=category_id,
    )


@pytest.mark.parametrize(("months", "size"), [(1, 1), (12, 1), (13, 3), (36, 3), (37, 12)])
def test_bucket_months(months: int, size: int) -> None:
    assert reports.bucket_months(months) == size


def test_bucket_start() -> None:
    assert reports.bucket_start(date(2026, 5, 1), 1) == date(2026, 5, 1)
    assert reports.bucket_start(date(2026, 5, 1), 3) == date(2026, 4, 1)
    assert reports.bucket_start(date(2026, 12, 1), 3) == date(2026, 10, 1)
    assert reports.bucket_start(date(2026, 5, 1), 12) == date(2026, 1, 1)


def test_compare_categories_per_month(db: Session, user: User) -> None:
    account, cats = _setup(db, user)
    groceries, eating_out = cats["Groceries"], cats["Eating out"]
    start = date(2026, 1, 1)
    for i in range(6):
        month = add_months(start, i)
        _spend(db, account, month.replace(day=5), -10000, groceries)
        # Eating out: 100.00 in the first 3 months, 200.00 in the last 3 months.
        _spend(db, account, month.replace(day=9), -10000 if i < 3 else -20000, eating_out)
    _spend(db, account, date(2026, 2, 10), 2000, groceries)  # A refund.
    _spend(db, account, date(2026, 3, 1), -5000, None, "Dentist")  # Uncategorised.
    _spend(db, account, date(2026, 3, 2), 9000, None)  # An inflow: not spending.
    _spend(db, account, date(2026, 3, 3), 500000, cats["Salary"])  # Income: not spending.

    data = reports.compare_categories(db, user.id, start, date(2026, 6, 30))
    assert data.bucket_months == 1
    assert len(data.buckets) == 6 and data.has_trend
    by_name = {r.name: r for r in data.rows}
    assert set(by_name) == {"Groceries", "Eating out", "Uncategorised"}

    g = by_name["Groceries"]
    assert g.monthly == (10000, 8000, 10000, 10000, 10000, 10000)
    assert g.values == g.monthly
    assert g.total == 58000
    assert g.average_per_month == 9667
    assert g.recent_average == 10000
    assert g.change_percent is not None and round(g.change_percent) == 7

    e = by_name["Eating out"]
    assert e.recent_average == 20000
    assert e.change_percent == pytest.approx(100.0)

    u = by_name["Uncategorised"]
    assert u.category_id is None
    assert u.monthly == (0, 0, 5000, 0, 0, 0)
    assert u.change_percent == pytest.approx(-100.0)  # 0.00 recently, 16.67 before.
    assert u.recent_average == 0

    assert data.total == 58000 + 90000 + 5000
    assert sum(r.share for r in data.rows) == pytest.approx(100.0)
    assert data.totals == tuple(
        sum(r.values[i] for r in data.rows) for i in range(len(data.buckets))
    )
    # Budget order: the groups of the budget, "Uncategorised" last.
    assert data.rows[-1].category_id is None
    assert data.groups[-1].uncategorised
    assert all(not g.uncategorised for g in data.groups[:-1])


def test_compare_categories_without_trend_and_empty(db: Session, user: User) -> None:
    account, cats = _setup(db, user)
    _spend(db, account, date(2026, 1, 5), -1000, cats["Groceries"])
    data = reports.compare_categories(db, user.id, date(2026, 1, 1), date(2026, 3, 31))
    assert not data.has_trend
    assert data.rows[0].change_percent is None and data.rows[0].recent_average is None
    empty = reports.compare_categories(db, user.id, date(2020, 1, 1), date(2020, 12, 31))
    assert empty.rows == () and empty.total == 0


def test_compare_categories_per_quarter_uses_average_per_month(db: Session, user: User) -> None:
    account, cats = _setup(db, user)
    groceries = cats["Groceries"]
    # 18 months from March 2025: the first quarter has 1 month, the last has 2 months.
    start = date(2025, 3, 1)
    for i in range(18):
        _spend(db, account, add_months(start, i).replace(day=5), -30000, groceries)
    data = reports.compare_categories(db, user.id, start, date(2026, 8, 31))
    assert data.bucket_months == 3
    assert data.buckets[0] == date(2025, 1, 1)
    assert data.buckets[-1] == date(2026, 7, 1)
    # The same spending each month gives the same value in each quarter.
    [row] = data.rows
    assert set(row.values) == {30000}
    assert row.total == 18 * 30000


def test_top_counterparties(db: Session, user: User) -> None:
    account, cats = _setup(db, user)
    groceries = cats["Groceries"]
    for amount, name in [(-3000, "Coop"), (-2000, "Migros"), (-4000, "Migros"), (500, "Coop")]:
        _spend(db, account, date(2026, 1, 10), amount, groceries, name)
    _spend(db, account, date(2026, 1, 11), -7000, None, "")
    result = reports.top_counterparties(db, user.id, groceries, date(2026, 1, 1), date(2026, 1, 31))
    assert [(c.name, c.count, c.total) for c in result] == [("Migros", 2, 6000), ("Coop", 2, 2500)]
    uncategorised = reports.top_counterparties(
        db, user.id, None, date(2026, 1, 1), date(2026, 1, 31)
    )
    assert [(c.name, c.total) for c in uncategorised] == [("—", 7000)]


@pytest.mark.parametrize(
    ("value", "average", "level"),
    [(100, 100, 0), (90, 100, 0), (109, 100, 0), (110, 100, 1), (125, 100, 2), (150, 100, 3),
     (200, 100, 4), (500, 100, 4), (100, 0, 0), (0, 0, 0)],
)  # fmt: skip
def test_heat(value: int, average: int, level: int) -> None:
    assert _heat(value, average) == level


def test_sort_orders(db: Session, user: User) -> None:
    account, cats = _setup(db, user)
    for i in range(6):
        month = add_months(date(2026, 1, 1), i)
        _spend(db, account, month, -50000, cats["Rent and service charges"])
        _spend(db, account, month, -1000 if i < 3 else -3000, cats["Eating out"])
    data = reports.compare_categories(db, user.id, date(2026, 1, 1), date(2026, 6, 30))
    assert next(r.name for r in _sorted_rows(data.rows, "total")) == "Rent and service charges"
    assert next(r.name for r in _sorted_rows(data.rows, "change")) == "Eating out"
    assert _sorted_rows(data.rows, "budget") == list(data.rows)


def test_category_report_page(client: tuple[TestClient, str], app: FastAPI) -> None:
    http, _ = client
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None
    account, cats = _setup(db, user)
    today = date.today().replace(day=1)
    for i in range(6):
        _spend(db, account, add_months(today, -i), -12345, cats["Groceries"], "Migros")
    db.commit()
    db.close()

    page = http.get("/reports/categories?period=6m")
    assert page.status_code == 200
    assert "Groceries" in page.text and "123.45" in page.text
    assert "Last 3 months" in page.text
    response = http.get(f"/reports/categories?period=6m&category={cats['Groceries']}")
    assert response.status_code == 200
    assert 'id="detail"' in response.text and "Migros" in response.text
    # Unknown values do not cause errors.
    assert http.get("/reports/categories?sort=abc&category=999999").status_code == 200
    assert http.get("/reports/categories?category=uncategorised").status_code == 200
    custom = http.get("/reports/categories?from=2026-01-01&to=2025-01-01")
    assert custom.status_code == 400
    # The report tabs keep the period.
    overview = http.get("/reports?period=24m")
    assert 'href="/reports/categories?period=24m"' in overview.text
