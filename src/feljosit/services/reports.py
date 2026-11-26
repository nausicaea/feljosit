"""Statistics for the reports page."""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from datetime import date

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from feljosit.models import Account, Category, CategoryGroup, CategoryKind, Transaction
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


# Spending by category over time.

RECENT_MONTHS = 3
"""The trend compares the average of the last months of the period with the months before."""
MIN_MONTHS_FOR_TREND = 6


def bucket_months(month_count: int) -> int:
    """The length of one column in months: a month, a quarter or a year.

    A table with more than 12 columns is difficult to read.
    """
    if month_count <= 12:
        return 1
    if month_count <= 36:
        return 3
    return 12


def bucket_start(month: date, size: int) -> date:
    """The first month of the bucket that contains ``month``.

    The buckets are calendar months, quarters or years.
    """
    index = (month.year * 12 + month.month - 1) // size * size
    return date(index // 12, index % 12 + 1, 1)


@dataclass(frozen=True)
class CategorySeries:
    """The spending of one category in a period. Spending is positive, refunds are negative."""

    category_id: int | None
    name: str
    group: str
    #: Spending for each month of the period.
    monthly: tuple[int, ...]
    #: Average spending per month in each bucket (column) of the table. A bucket at the
    #: start or the end of the period can have fewer months than the others.
    values: tuple[int, ...]
    total: int
    average_per_month: int
    #: Part of the spending of all categories, in percent.
    share: float
    #: Average per month of the last ``RECENT_MONTHS`` months.
    recent_average: int | None
    #: Change of ``recent_average`` against the average of the months before, in percent.
    change_percent: float | None


@dataclass(frozen=True)
class CategoryGroupSeries:
    name: str
    rows: tuple[CategorySeries, ...]
    #: Average spending per month in each bucket, as in ``CategorySeries.values``.
    values: tuple[int, ...]
    total: int

    @property
    def uncategorised(self) -> bool:
        return self.rows[0].category_id is None


@dataclass(frozen=True)
class CategoryComparison:
    months: tuple[date, ...]
    #: The first month of each bucket (column).
    buckets: tuple[date, ...]
    bucket_months: int
    rows: tuple[CategorySeries, ...]
    groups: tuple[CategoryGroupSeries, ...]
    #: Average spending per month of all categories in each bucket.
    totals: tuple[int, ...]
    total: int
    has_trend: bool


# The order of the budget page: group order, group name, category order, category name.
# "Uncategorised" is the last row.
_SortKey = tuple[bool, int, str, int, str]


def _trend(monthly: list[int]) -> tuple[int | None, float | None]:
    if len(monthly) < MIN_MONTHS_FOR_TREND:
        return None, None
    recent = monthly[-RECENT_MONTHS:]
    before = monthly[:-RECENT_MONTHS]
    recent_average = round(sum(recent) / len(recent))
    before_average = sum(before) / len(before)
    if before_average <= 0:
        return recent_average, None
    return recent_average, (recent_average - before_average) / before_average * 100


def compare_categories(db: Session, user_id: int, start: date, end: date) -> CategoryComparison:
    """Spending per expense category and month, for the comparison of categories over time.

    The same transactions as in :func:`spending_by_category`: expense categories and
    uncategorised outflows of budget accounts, without transfers. Categories without
    spending in the period are not included.
    """
    month_expr = func.date(Transaction.booking_date, "start of month")
    rows = db.execute(
        select(
            Transaction.category_id,
            Category.name,
            CategoryGroup.name,
            CategoryGroup.sort_order,
            Category.sort_order,
            month_expr,
            func.sum(Transaction.amount_minor),
        )
        .join(Account, Transaction.account_id == Account.id)
        .outerjoin(Category, Transaction.category_id == Category.id)
        .outerjoin(CategoryGroup, Category.group_id == CategoryGroup.id)
        .where(
            *_budget_tx_filter(user_id, start, end),  # type: ignore[arg-type]
            (Category.kind == CategoryKind.EXPENSE)
            | (Transaction.category_id.is_(None) & (Transaction.amount_minor < 0)),
        )
        .group_by(Transaction.category_id, month_expr)
    )
    months = months_between(start, end)
    index = {m: i for i, m in enumerate(months)}
    size = bucket_months(len(months))
    buckets = sorted({bucket_start(m, size) for m in months})
    bucket_index = {b: i for i, b in enumerate(buckets)}
    bucket_sizes = [0] * len(buckets)
    for m in months:
        bucket_sizes[bucket_index[bucket_start(m, size)]] += 1

    def per_month(sums: list[int]) -> tuple[int, ...]:
        return tuple(round(v / n) for v, n in zip(sums, bucket_sizes, strict=True))

    found: dict[int | None, tuple[str, str, _SortKey, list[int]]] = {}
    for category_id, name, group, group_order, order, month_text, total in rows:
        if category_id not in found:
            key: _SortKey = (
                category_id is None,
                group_order or 0,
                group or "",
                order or 0,
                name or "",
            )
            found[category_id] = (
                name or "Uncategorised",
                group or "Uncategorised",
                key,
                [0] * len(months),
            )
        found[category_id][3][index[date.fromisoformat(month_text)]] -= int(total)

    grand_total = sum(sum(m) for *_, m in found.values())
    series: list[tuple[_SortKey, CategorySeries]] = []
    for category_id, (name, group, sort_key, monthly) in found.items():
        if not any(monthly):
            continue
        total = sum(monthly)
        sums = [0] * len(buckets)
        for m, v in zip(months, monthly, strict=True):
            sums[bucket_index[bucket_start(m, size)]] += v
        recent, change = _trend(monthly)
        series.append(
            (
                sort_key,
                CategorySeries(
                    category_id=category_id,
                    name=name,
                    group=group,
                    monthly=tuple(monthly),
                    values=per_month(sums),
                    total=total,
                    average_per_month=round(total / len(months)) if months else 0,
                    share=(total / grand_total * 100) if grand_total > 0 else 0.0,
                    recent_average=recent,
                    change_percent=change,
                ),
            )
        )
    ordered = tuple(s for _, s in sorted(series, key=lambda item: item[0]))
    groups: list[CategoryGroupSeries] = []
    for row in ordered:
        if groups and groups[-1].name == row.group:
            last = groups[-1]
            groups[-1] = CategoryGroupSeries(
                last.name,
                (*last.rows, row),
                tuple(a + b for a, b in zip(last.values, row.values, strict=True)),
                last.total + row.total,
            )
        else:
            groups.append(CategoryGroupSeries(row.group, (row,), row.values, row.total))
    totals = tuple(sum(r.values[i] for r in ordered) for i in range(len(buckets)))
    return CategoryComparison(
        months=tuple(months),
        buckets=tuple(buckets),
        bucket_months=size,
        rows=ordered,
        groups=tuple(groups),
        totals=totals,
        total=sum(r.total for r in ordered),
        has_trend=len(months) >= MIN_MONTHS_FOR_TREND,
    )


@dataclass(frozen=True)
class CounterpartyTotal:
    name: str
    count: int
    total: int


def top_counterparties(
    db: Session,
    user_id: int,
    category_id: int | None,
    start: date,
    end: date,
    limit: int = 15,
) -> list[CounterpartyTotal]:
    """The counterparties with the most spending in one category (``None``: uncategorised)."""
    category_filter = (
        (Transaction.category_id.is_(None) & (Transaction.amount_minor < 0))
        if category_id is None
        else (Transaction.category_id == category_id)
    )
    total = func.sum(Transaction.amount_minor)
    rows = db.execute(
        select(Transaction.counterparty, func.count(), total)
        .join(Account, Transaction.account_id == Account.id)
        .where(*_budget_tx_filter(user_id, start, end), category_filter)  # type: ignore[arg-type]
        .group_by(Transaction.counterparty)
        .order_by(total)
        .limit(limit)
    )
    return [
        CounterpartyTotal(name or "—", int(count), -int(amount)) for name, count, amount in rows
    ]
