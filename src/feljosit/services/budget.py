"""Envelope budgeting ("give every franc a job").

The rules follow the YNAB method:

* Income goes to *Ready to Assign*. An income transaction is a transaction in a
  budget account with a category of the kind ``income``.
* The user assigns money from *Ready to Assign* to expense categories for a month.
* ``available = available of the previous month (if positive) + assigned + activity``.
* A positive *available* value rolls over to the next month.
* A negative *available* value (overspending) does not roll over. The application
  subtracts it from *Ready to Assign* in the next month.
* Transfers and transactions in tracking accounts do not change the budget.
"""

from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass, field
from datetime import date

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from feljosit.models import (
    Account,
    BudgetAssignment,
    Category,
    CategoryGroup,
    CategoryKind,
    Transaction,
)
from feljosit.services.categorise import owned_category
from feljosit.services.errors import ServiceError

MAX_ASSIGNMENT = 10**13


def month_start(value: date) -> date:
    return value.replace(day=1)


def add_months(value: date, months: int) -> date:
    index = value.year * 12 + (value.month - 1) + months
    return date(index // 12, index % 12 + 1, 1)


def parse_month(text: str | None, default: date) -> date:
    if not text:
        return month_start(default)
    try:
        year, month = text.split("-")
        return date(int(year), int(month), 1)
    except ValueError as exc:
        raise ServiceError("The month must have the format YYYY-MM.") from exc


@dataclass
class CategoryRow:
    category_id: int
    name: str
    assigned: int = 0
    activity: int = 0
    available: int = 0


@dataclass
class GroupRows:
    group_id: int
    name: str
    rows: list[CategoryRow] = field(default_factory=list)

    @property
    def assigned(self) -> int:
        return sum(r.assigned for r in self.rows)

    @property
    def activity(self) -> int:
        return sum(r.activity for r in self.rows)

    @property
    def available(self) -> int:
        return sum(r.available for r in self.rows)


@dataclass
class MonthBudget:
    month: date
    ready_to_assign: int
    income: int
    assigned: int
    activity: int
    overspent_last_month: int
    groups: list[GroupRows]
    uncategorised_count: int


def _monthly(
    db: Session, user_id: int
) -> tuple[dict[tuple[int, date], int], dict[tuple[int, date], int], dict[date, int]]:
    """Return (assigned, expense activity, income) per (category, month) / month."""
    month_expr = func.date(Transaction.booking_date, "start of month")
    activity_rows = db.execute(
        select(
            Transaction.category_id, Category.kind, month_expr, func.sum(Transaction.amount_minor)
        )
        .join(Account, Transaction.account_id == Account.id)
        .join(Category, Transaction.category_id == Category.id)
        .where(
            Transaction.user_id == user_id,
            Account.on_budget.is_(True),
            Transaction.is_transfer.is_(False),
        )
        .group_by(Transaction.category_id, Category.kind, month_expr)
    )
    activity: dict[tuple[int, date], int] = {}
    income: dict[date, int] = defaultdict(int)
    for category_id, kind, month_text, total in activity_rows:
        month = date.fromisoformat(month_text)
        if kind == CategoryKind.INCOME:
            income[month] += int(total)
        else:
            activity[(category_id, month)] = int(total)
    assigned = {
        (category_id, month): amount
        for category_id, month, amount in db.execute(
            select(
                BudgetAssignment.category_id, BudgetAssignment.month, BudgetAssignment.amount_minor
            ).where(BudgetAssignment.user_id == user_id)
        )
    }
    return assigned, activity, dict(income)


def month_budget(db: Session, user_id: int, month: date) -> MonthBudget:
    month = month_start(month)
    assigned, activity, income = _monthly(db, user_id)
    groups = list(
        db.scalars(
            select(CategoryGroup)
            .where(CategoryGroup.user_id == user_id)
            .order_by(CategoryGroup.sort_order, CategoryGroup.name)
        )
    )
    expense_categories = [c for g in groups for c in g.categories if c.kind == CategoryKind.EXPENSE]
    months = [m for _, m in assigned] + [m for _, m in activity] + list(income)
    first = min(months, default=month)
    first = min(first, month)

    available: dict[int, int] = defaultdict(int)
    ready = 0
    overspent_previous = 0
    current = first
    while current <= month:
        ready += income.get(current, 0) + overspent_previous
        overspent_this = 0
        for category in expense_categories:
            carry = max(available[category.id], 0)
            a = assigned.get((category.id, current), 0)
            act = activity.get((category.id, current), 0)
            available[category.id] = carry + a + act
            ready -= a
            if available[category.id] < 0:
                overspent_this += available[category.id]
        if current == month:
            break
        overspent_previous = overspent_this
        current = add_months(current, 1)

    # Money that the user assigned to future months is not ready to assign now.
    ready -= sum(a for (_, m), a in assigned.items() if m > month)

    group_rows: list[GroupRows] = []
    for group in groups:
        rows = [
            CategoryRow(
                category_id=c.id,
                name=c.name,
                assigned=assigned.get((c.id, month), 0),
                activity=activity.get((c.id, month), 0),
                available=available[c.id],
            )
            for c in group.categories
            if c.kind == CategoryKind.EXPENSE and not c.hidden
        ]
        if rows:
            group_rows.append(GroupRows(group.id, group.name, rows))

    uncategorised = db.scalar(
        select(func.count(Transaction.id))
        .join(Account, Transaction.account_id == Account.id)
        .where(
            Transaction.user_id == user_id,
            Account.on_budget.is_(True),
            Transaction.category_id.is_(None),
            Transaction.is_transfer.is_(False),
        )
    )
    return MonthBudget(
        month=month,
        ready_to_assign=ready,
        income=income.get(month, 0),
        assigned=sum(g.assigned for g in group_rows),
        activity=sum(g.activity for g in group_rows),
        overspent_last_month=overspent_previous,
        groups=group_rows,
        uncategorised_count=int(uncategorised or 0),
    )


def assign(db: Session, user_id: int, category_id: int, month: date, amount_minor: int) -> None:
    category = owned_category(db, user_id, category_id)
    if category.kind != CategoryKind.EXPENSE:
        raise ServiceError("You can only assign money to expense categories.")
    if abs(amount_minor) > MAX_ASSIGNMENT:
        raise ServiceError("The amount is too large.")
    month = month_start(month)
    row = db.scalar(
        select(BudgetAssignment).where(
            BudgetAssignment.category_id == category_id, BudgetAssignment.month == month
        )
    )
    if row is None:
        db.add(
            BudgetAssignment(
                user_id=user_id, category_id=category_id, month=month, amount_minor=amount_minor
            )
        )
    else:
        row.amount_minor = amount_minor
