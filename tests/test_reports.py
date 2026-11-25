from datetime import date

from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import AccountType, Category, User
from feljosit.services import accounts, reports
from feljosit.web import charts


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
