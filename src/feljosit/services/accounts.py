"""Accounts and manual transactions."""

from __future__ import annotations

import secrets
from datetime import date

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from feljosit.iban import is_valid_iban, normalise_iban
from feljosit.models import (
    Account,
    AccountType,
    Category,
    CategoryKind,
    Transaction,
    TransactionSource,
)
from feljosit.money import validate_currency
from feljosit.services.errors import NotFoundError, ServiceError


def get_account(db: Session, user_id: int, account_id: int) -> Account:
    account = db.get(Account, account_id)
    if account is None or account.user_id != user_id:
        raise NotFoundError("account")
    return account


def list_accounts(db: Session, user_id: int, *, include_closed: bool = True) -> list[Account]:
    query = select(Account).where(Account.user_id == user_id)
    if not include_closed:
        query = query.where(Account.closed.is_(False))
    return list(db.scalars(query.order_by(Account.closed, Account.on_budget.desc(), Account.name)))


def balances(db: Session, user_id: int) -> dict[int, int]:
    rows = db.execute(
        select(Transaction.account_id, func.sum(Transaction.amount_minor))
        .where(Transaction.user_id == user_id)
        .group_by(Transaction.account_id)
    )
    return {account_id: int(total or 0) for account_id, total in rows}


def create_account(
    db: Session,
    user_id: int,
    *,
    name: str,
    account_type: AccountType,
    currency: str,
    budget_currency: str,
    iban: str | None = None,
    on_budget: bool | None = None,
    starting_balance_minor: int = 0,
    starting_date: date | None = None,
) -> Account:
    name = name.strip()
    if not 1 <= len(name) <= 100:
        raise ServiceError("The account name must have 1 to 100 characters.")
    currency = validate_currency(currency)
    on_budget = account_type.on_budget_default if on_budget is None else on_budget
    if on_budget and currency != budget_currency:
        raise ServiceError(
            f"An account in the budget must use the budget currency ({budget_currency}). "
            "Make it a tracking account."
        )
    clean_iban: str | None = None
    if iban and iban.strip():
        clean_iban = normalise_iban(iban)
        if not is_valid_iban(clean_iban):
            raise ServiceError("The IBAN is not valid.")
        exists = db.scalar(
            select(Account).where(Account.user_id == user_id, Account.iban == clean_iban)
        )
        if exists is not None:
            raise ServiceError("An account with this IBAN exists already.")
    account = Account(
        user_id=user_id,
        name=name,
        type=account_type,
        on_budget=on_budget,
        currency=currency,
        iban=clean_iban,
    )
    db.add(account)
    db.flush()
    if starting_balance_minor:
        add_starting_balance(db, account, starting_balance_minor, starting_date or date.today())
    return account


def income_category_for_starting_balance(db: Session, user_id: int) -> Category | None:
    """Return the first income category. A starting balance goes to 'Ready to Assign'."""
    return db.scalar(
        select(Category)
        .where(Category.user_id == user_id, Category.kind == CategoryKind.INCOME)
        .order_by(Category.id)
        .limit(1)
    )


def add_starting_balance(db: Session, account: Account, amount_minor: int, on: date) -> None:
    category = None
    if account.on_budget:
        category = income_category_for_starting_balance(db, account.user_id)
    db.add(
        Transaction(
            user_id=account.user_id,
            account_id=account.id,
            booking_date=on,
            value_date=on,
            amount_minor=amount_minor,
            currency=account.currency,
            counterparty="Starting balance",
            description="",
            category_id=category.id if category else None,
            source=TransactionSource.STARTING_BALANCE,
            fingerprint=f"start:{account.id}",
        )
    )


def add_manual_transaction(
    db: Session,
    account: Account,
    *,
    booking_date: date,
    amount_minor: int,
    counterparty: str,
    memo: str = "",
    category_id: int | None = None,
) -> Transaction:
    if account.closed:
        raise ServiceError("The account is closed.")
    if category_id is not None:
        category = db.get(Category, category_id)
        if category is None or category.user_id != account.user_id:
            raise NotFoundError("category")
    tx = Transaction(
        user_id=account.user_id,
        account_id=account.id,
        booking_date=booking_date,
        value_date=booking_date,
        amount_minor=amount_minor,
        currency=account.currency,
        counterparty=counterparty.strip()[:140],
        memo=memo.strip()[:500],
        category_id=category_id if account.on_budget else None,
        source=TransactionSource.MANUAL,
        fingerprint=f"manual:{secrets.token_hex(16)}",
    )
    db.add(tx)
    db.flush()
    return tx


def set_closed(db: Session, account: Account, *, closed: bool) -> None:
    account.closed = closed
