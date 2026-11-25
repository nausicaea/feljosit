from datetime import date

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Account, AccountType, Category, User
from feljosit.services import accounts, budget
from feljosit.services.errors import ServiceError


@pytest.fixture
def account(db: Session, user: User) -> Account:
    return accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )


def _cat(db: Session, user: User, name: str) -> int:
    category = db.scalar(select(Category).where(Category.user_id == user.id, Category.name == name))
    assert category is not None
    return category.id


def _tx(db: Session, account: Account, on: date, amount: int, category_id: int | None) -> None:
    accounts.add_manual_transaction(
        db,
        account,
        booking_date=on,
        amount_minor=amount,
        counterparty="x",
        category_id=category_id,
    )


JAN, FEB, MAR = date(2026, 1, 1), date(2026, 2, 1), date(2026, 3, 1)


def _row(data: budget.MonthBudget, category_id: int) -> budget.CategoryRow:
    for group in data.groups:
        for row in group.rows:
            if row.category_id == category_id:
                return row
    raise AssertionError("row not found")


def test_income_goes_to_ready_to_assign(db: Session, user: User, account: Account) -> None:
    _tx(db, account, date(2026, 1, 25), 500000, _cat(db, user, "Salary"))
    data = budget.month_budget(db, user.id, JAN)
    assert data.ready_to_assign == 500000
    assert data.income == 500000


def test_assign_and_rollover(db: Session, user: User, account: Account) -> None:
    salary, food = _cat(db, user, "Salary"), _cat(db, user, "Groceries")
    _tx(db, account, date(2026, 1, 5), 100000, salary)
    budget.assign(db, user.id, food, JAN, 60000)
    _tx(db, account, date(2026, 1, 10), -45000, food)
    jan = budget.month_budget(db, user.id, JAN)
    assert jan.ready_to_assign == 40000
    assert _row(jan, food).available == 15000
    assert _row(jan, food).activity == -45000
    # The positive available amount rolls over.
    feb = budget.month_budget(db, user.id, FEB)
    assert _row(feb, food).available == 15000
    assert _row(feb, food).assigned == 0
    assert feb.ready_to_assign == 40000


def test_overspending_reduces_next_month(db: Session, user: User, account: Account) -> None:
    salary, food = _cat(db, user, "Salary"), _cat(db, user, "Groceries")
    _tx(db, account, date(2026, 1, 5), 100000, salary)
    budget.assign(db, user.id, food, JAN, 10000)
    _tx(db, account, date(2026, 1, 10), -25000, food)
    jan = budget.month_budget(db, user.id, JAN)
    assert _row(jan, food).available == -15000
    assert jan.ready_to_assign == 90000
    feb = budget.month_budget(db, user.id, FEB)
    # The negative amount does not roll over in the category ...
    assert _row(feb, food).available == 0
    # ... but it reduces Ready to Assign.
    assert feb.ready_to_assign == 75000
    assert feb.overspent_last_month == -15000
    assert budget.month_budget(db, user.id, MAR).ready_to_assign == 75000


def test_future_assignments_reduce_ready_to_assign(
    db: Session, user: User, account: Account
) -> None:
    salary, food = _cat(db, user, "Salary"), _cat(db, user, "Groceries")
    _tx(db, account, date(2026, 1, 5), 100000, salary)
    budget.assign(db, user.id, food, MAR, 30000)
    assert budget.month_budget(db, user.id, JAN).ready_to_assign == 70000


def test_reassign_overwrites(db: Session, user: User, account: Account) -> None:
    food = _cat(db, user, "Groceries")
    budget.assign(db, user.id, food, JAN, 100)
    budget.assign(db, user.id, food, JAN, 200)
    assert _row(budget.month_budget(db, user.id, JAN), food).assigned == 200


def test_cannot_assign_to_income(db: Session, user: User) -> None:
    with pytest.raises(ServiceError):
        budget.assign(db, user.id, _cat(db, user, "Salary"), JAN, 100)


def test_tracking_accounts_and_transfers_do_not_change_budget(db: Session, user: User) -> None:
    tracking = accounts.create_account(
        db,
        user.id,
        name="3a",
        account_type=AccountType.PILLAR_3A,
        currency="CHF",
        budget_currency="CHF",
        starting_balance_minor=5000000,
        starting_date=date(2026, 1, 1),
    )
    assert not tracking.on_budget
    data = budget.month_budget(db, user.id, JAN)
    assert data.ready_to_assign == 0
    assert data.uncategorised_count == 0


def test_uncategorised_count(db: Session, user: User, account: Account) -> None:
    _tx(db, account, date(2026, 1, 5), -100, None)
    assert budget.month_budget(db, user.id, JAN).uncategorised_count == 1


def test_month_helpers() -> None:
    assert budget.add_months(date(2026, 12, 1), 1) == date(2027, 1, 1)
    assert budget.add_months(date(2026, 1, 1), -1) == date(2025, 12, 1)
    assert budget.parse_month("2026-07", date.today()) == date(2026, 7, 1)
    with pytest.raises(ServiceError):
        budget.parse_month("July", date.today())
