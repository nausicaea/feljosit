from datetime import date

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import AccountType, Category, User
from feljosit.services import accounts, reports
from feljosit.web import charts
from feljosit.web.routes.reports import all_range, custom_range


def test_flows_spending_and_net_worth(db: Session, user: User) -> None:
    account = accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )
    cats = {c.name: c.id for c in db.scalars(select(Category).where(Category.user_id == user.id))}
    for on, amount, cat in [
        (date(2026, 1, 25), 500000, "Salary"),
        (date(2026, 1, 3), -200000, "Rent and service charges"),
        (date(2026, 2, 3), -50000, "Groceries"),
        (date(2026, 2, 4), -1000, None),
        (date(2026, 2, 5), 300, None),
    ]:
        accounts.add_manual_transaction(
            db,
            account,
            booking_date=on,
            amount_minor=amount,
            counterparty="x",
            category_id=cats[cat] if cat else None,
        )
    flows = reports.monthly_flows(db, user.id, date(2026, 1, 1), date(2026, 3, 31))
    assert [(f.income, f.expenses) for f in flows] == [(500000, -200000), (300, -51000), (0, 0)]
    spending = reports.spending_by_category(db, user.id, date(2026, 1, 1), date(2026, 3, 31))
    assert [(s.name, s.total) for s in spending] == [
        ("Rent and service charges", 200000),
        ("Groceries", 50000),
        ("Uncategorised", 1000),
    ]
    summary = reports.summary(flows)
    assert summary.savings_rate_percent is not None
    assert round(summary.savings_rate_percent, 1) == 49.8
    worth = reports.net_worth(db, user.id, date(2026, 1, 1), date(2026, 3, 31))
    assert [v for _, v in worth["CHF"]] == [300000, 249300, 249300]


def test_charts_geometry() -> None:
    chart = charts.grouped_bars(["a", "b"], [("x", [100, -50]), ("y", [0, 30])], str)
    assert len(chart.bars) == 4
    assert all(b.height >= 0 for b in chart.bars)
    line = charts.line(["a", "b", "c"], [1, 5, 3], str)
    assert len(line.line_points.split()) == 3
    rows = charts.horizontal_bars([("a", 50), ("b", 25)], str)
    assert rows[0].percent == 100 and rows[1].percent == 50
    assert round(rows[0].share, 1) == 66.7
    assert charts.horizontal_bars([], str) == []


def test_custom_range() -> None:
    today = date(2026, 11, 26)
    assert custom_range(date(2026, 1, 15), date(2026, 3, 10), today) == (
        date(2026, 1, 15),
        date(2026, 3, 10),
    )
    assert custom_range(date(2026, 5, 1), None, today) == (date(2026, 5, 1), today)
    assert custom_range(None, date(2026, 6, 30), today) == (date(2025, 7, 1), date(2026, 6, 30))
    assert custom_range(date(2006, 12, 1), date(2026, 11, 30), today)[0] == date(2006, 12, 1)
    with pytest.raises(ValueError, match="on or before"):
        custom_range(date(2026, 3, 1), date(2026, 2, 1), today)
    with pytest.raises(ValueError, match="maximum of 20 years"):
        custom_range(date(2006, 11, 30), date(2026, 11, 30), today)
    with pytest.raises(ValueError, match="Type dates"):
        custom_range(date(1, 1, 1), date(2026, 1, 1), today)
    with pytest.raises(ValueError, match="Type dates"):
        custom_range(None, date(9999, 12, 31), today)


def test_flows_with_partial_months(db: Session, user: User) -> None:
    account = accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )
    for on, amount in [
        (date(2026, 1, 5), -100),
        (date(2026, 1, 20), -200),
        (date(2026, 2, 25), -400),
    ]:
        accounts.add_manual_transaction(
            db, account, booking_date=on, amount_minor=amount, counterparty="x", category_id=None
        )
    flows = reports.monthly_flows(db, user.id, date(2026, 1, 10), date(2026, 2, 20))
    assert [(f.month, f.expenses) for f in flows] == [
        (date(2026, 1, 1), -200),
        (date(2026, 2, 1), 0),
    ]
    worth = reports.net_worth(db, user.id, date(2026, 1, 10), date(2026, 2, 20))
    assert [v for _, v in worth["CHF"]] == [-300, -300]


def test_all_range(db: Session, user: User) -> None:
    today = date(2026, 11, 26)
    assert reports.transaction_date_range(db, user.id) is None
    assert all_range(None, today) == (date(2026, 11, 1), date(2026, 11, 30), False)
    account = accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )
    for on in [date(2024, 3, 15), date(2025, 6, 1)]:
        accounts.add_manual_transaction(
            db, account, booking_date=on, amount_minor=-100, counterparty="x", category_id=None
        )
    data_range = reports.transaction_date_range(db, user.id)
    assert data_range == (date(2024, 3, 15), date(2025, 6, 1))
    assert all_range(data_range, today) == (date(2024, 3, 1), date(2026, 11, 30), False)
    # A transaction after the current month extends the range.
    assert all_range((date(2024, 3, 15), date(2027, 2, 3)), today)[1] == date(2027, 2, 28)
    # More than 20 years: only the last 20 years.
    assert all_range((date(1990, 1, 1), today), today) == (
        date(2006, 12, 1),
        date(2026, 11, 30),
        True,
    )
