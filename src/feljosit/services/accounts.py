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
    # Movements to this IBAN that are imported already can be transfers now.
    update_incoming_transfers(db, account)
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


def update_account(
    db: Session,
    account: Account,
    *,
    name: str,
    account_type: AccountType,
    budget_currency: str,
    on_budget: bool | None = None,
) -> None:
    """Change the name, the type and the budget setting of an account.

    If ``on_budget`` is ``None``, the type gives the budget setting. If the budget
    setting changes, :func:`_reclassify` updates the transactions.
    """
    name = name.strip()
    if not 1 <= len(name) <= 100:
        raise ServiceError("The account name must have 1 to 100 characters.")
    on_budget = account_type.on_budget_default if on_budget is None else on_budget
    if on_budget and account.currency != budget_currency:
        raise ServiceError(
            f"An account in the budget must use the budget currency ({budget_currency}). "
            "Make it a tracking account."
        )
    account.name = name
    account.type = account_type
    if on_budget != account.on_budget:
        account.on_budget = on_budget
        db.flush()
        _reclassify(db, account)


def _reclassify(db: Session, account: Account) -> None:
    """Update the transactions after the budget setting of ``account`` changed.

    - A tracking account has no categories. A budget account gives its starting
      balance to "Ready to Assign". Its other transactions stay uncategorised.
    - A transfer is a movement between two own accounts with the same budget
      setting (see ADR-0008). The transfer flag changes for the movements between
      this account and the other accounts. A new transfer has no category.
    """
    income = income_category_for_starting_balance(db, account.user_id)
    for tx in db.scalars(select(Transaction).where(Transaction.account_id == account.id)):
        if not account.on_budget:
            tx.category_id = None
        elif tx.source == TransactionSource.STARTING_BALANCE and tx.category_id is None:
            tx.category_id = income.id if income else None

    own = {
        iban: (account_id, on_budget)
        for account_id, iban, on_budget in db.execute(
            select(Account.id, Account.iban, Account.on_budget).where(
                Account.user_id == account.user_id, Account.iban.is_not(None)
            )
        )
    }
    # Movements of this account to other own accounts.
    outgoing = db.scalars(
        select(Transaction).where(
            Transaction.account_id == account.id,
            Transaction.counterparty_iban.in_([i for i in own if i != account.iban]),
        )
    )
    for tx in outgoing:
        _, other_on_budget = own[tx.counterparty_iban or ""]
        _set_transfer(tx, other_on_budget == account.on_budget)
    # Movements of the other own accounts to this account.
    update_incoming_transfers(db, account)


def update_incoming_transfers(db: Session, account: Account) -> None:
    """Set the transfer flag of the movements from the other own accounts to ``account``.

    A movement is a transfer if the two accounts have the same budget setting.
    """
    if not account.iban:
        return
    incoming = db.scalars(
        select(Transaction).where(
            Transaction.user_id == account.user_id,
            Transaction.account_id != account.id,
            Transaction.counterparty_iban == account.iban,
        )
    )
    for tx in incoming:
        _set_transfer(tx, tx.account.on_budget == account.on_budget)


def _set_transfer(tx: Transaction, is_transfer: bool) -> None:
    if is_transfer:
        tx.category_id = None
    tx.is_transfer = is_transfer
