"""Statistics for the reports page."""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from datetime import date

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from feljosit.models import Account, Category, CategoryKind, Transaction
from feljosit.services.budget import add_months, month_start


@dataclass(frozen=True)
class MonthFlow:
    month: date
    income: int
    expenses: int

    @property
    def net(self) -> int:
        return self.income + self.expenses


@dataclass(frozen=True)
class CategoryTotal:
    category_id: int | None
    name: str
    total: int


@dataclass(frozen=True)
class Summary:
    income: int
    expenses: int
    savings_rate_percent: float | None
    average_monthly_expenses: int


def months_between(start: date, end: date) -> list[date]:
    months: list[date] = []
    current = month_start(start)
    while current <= end:
        months.append(current)
        current = add_months(current, 1)
    return months


def transaction_date_range(db: Session, user_id: int) -> tuple[date, date] | None:
    """The booking dates of the first and the last transaction of all accounts.

    Return ``None`` if the user has no transactions.
    """
    row = db.execute(
        select(func.min(Transaction.booking_date), func.max(Transaction.booking_date)).where(
            Transaction.user_id == user_id
        )
    ).one()
    first: date | None = row[0]
    last: date | None = row[1]
    if first is None or last is None:
        return None
    return first, last


def _budget_tx_filter(user_id: int, start: date, end: date) -> list[object]:
    return [
        Transaction.user_id == user_id,
        Account.on_budget.is_(True),
        Transaction.is_transfer.is_(False),
        Transaction.booking_date >= start,
        Transaction.booking_date <= end,
    ]


def monthly_flows(db: Session, user_id: int, start: date, end: date) -> list[MonthFlow]:
    """Income and expenses per month. Expenses are negative values.

    For an uncategorised transaction, the sign decides: an inflow is income and an
    outflow is an expense.
    """
    month_expr = func.date(Transaction.booking_date, "start of month")
    is_inflow = Transaction.amount_minor > 0
    rows = db.execute(
        select(month_expr, Category.kind, is_inflow, func.sum(Transaction.amount_minor))
        .join(Account, Transaction.account_id == Account.id)
        .outerjoin(Category, Transaction.category_id == Category.id)
        .where(*_budget_tx_filter(user_id, start, end))  # type: ignore[arg-type]
        .group_by(month_expr, Category.kind, is_inflow)
    )
    income: dict[date, int] = defaultdict(int)
    expenses: dict[date, int] = defaultdict(int)
    for month_text, kind, inflow, total in rows:
        month = date.fromisoformat(month_text)
        if kind == CategoryKind.INCOME or (kind is None and inflow):
            income[month] += int(total)
        else:
            expenses[month] += int(total)
    return [
        MonthFlow(month=m, income=income.get(m, 0), expenses=expenses.get(m, 0))
        for m in months_between(start, end)
    ]


def spending_by_category(db: Session, user_id: int, start: date, end: date) -> list[CategoryTotal]:
    """Net expenses per expense category, largest first.

    Uncategorised outflows are one row ("Uncategorised").
    """
    rows = db.execute(
        select(Transaction.category_id, Category.name, func.sum(Transaction.amount_minor))
        .join(Account, Transaction.account_id == Account.id)
        .outerjoin(Category, Transaction.category_id == Category.id)
        .where(
            *_budget_tx_filter(user_id, start, end),  # type: ignore[arg-type]
            (Category.kind == CategoryKind.EXPENSE)
            | (Transaction.category_id.is_(None) & (Transaction.amount_minor < 0)),
        )
        .group_by(Transaction.category_id, Category.name)
    )
    totals = [
        CategoryTotal(cid, name or "Uncategorised", -int(total))
        for cid, name, total in rows
        if total and int(total) < 0
    ]
    return sorted(totals, key=lambda t: t.total, reverse=True)


def summary(flows: list[MonthFlow]) -> Summary:
    income = sum(f.income for f in flows)
    expenses = sum(f.expenses for f in flows)
    rate = ((income + expenses) / income * 100) if income > 0 else None
    months_with_data = [f for f in flows if f.income or f.expenses]
    average = round(-expenses / len(months_with_data)) if months_with_data else 0
    return Summary(income, expenses, rate, average)


def net_worth(
    db: Session, user_id: int, start: date, end: date
) -> dict[str, list[tuple[date, int]]]:
    """Total balance of all accounts at the end of each month, for each currency."""
    month_expr = func.date(Transaction.booking_date, "start of month")
    rows = db.execute(
        select(Account.currency, month_expr, func.sum(Transaction.amount_minor))
        .join(Account, Transaction.account_id == Account.id)
        .where(Transaction.user_id == user_id, Transaction.booking_date <= end)
        .group_by(Account.currency, month_expr)
        .order_by(month_expr)
    )
    changes: dict[str, dict[date, int]] = defaultdict(dict)
    for currency, month_text, total in rows:
        changes[currency][date.fromisoformat(month_text)] = int(total)
    months = months_between(start, end)
    result: dict[str, list[tuple[date, int]]] = {}
    for currency, per_month in sorted(changes.items()):
        running = sum(v for m, v in per_month.items() if m < months[0]) if months else 0
        series: list[tuple[date, int]] = []
        for m in months:
            running += per_month.get(m, 0)
            series.append((m, running))
        result[currency] = series
    return result
