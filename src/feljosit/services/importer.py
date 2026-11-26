"""Import of camt.053 statements."""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from datetime import date

from sqlalchemy import func, select
from sqlalchemy.orm import Session

from feljosit import camt053
from feljosit.iban import format_iban
from feljosit.models import Account, AccountType, ImportBatch, Transaction, TransactionSource
from feljosit.services import transfers
from feljosit.services.accounts import add_starting_balance, create_account, get_account
from feljosit.services.categorise import Categoriser
from feljosit.services.errors import ServiceError

MAX_FILENAME = 255


@dataclass
class StatementResult:
    account_id: int
    account_name: str
    statement_id: str
    new_count: int = 0
    duplicate_count: int = 0
    categorised_count: int = 0
    transfer_count: int = 0
    #: Transfers found from a matching transaction on a different account.
    matched_transfer_count: int = 0
    skipped_entries: int = 0
    account_created: bool = False
    closing_balance_minor: int | None = None
    computed_balance_minor: int | None = None

    @property
    def balance_matches(self) -> bool | None:
        if self.closing_balance_minor is None or self.computed_balance_minor is None:
            return None
        return self.closing_balance_minor == self.computed_balance_minor


@dataclass
class ImportResult:
    statements: list[StatementResult] = field(default_factory=list)

    @property
    def new_count(self) -> int:
        return sum(s.new_count for s in self.statements)

    @property
    def duplicate_count(self) -> int:
        return sum(s.duplicate_count for s in self.statements)


def _new_account_name(statement: camt053.Statement) -> str:
    """The name for an account that the import makes.

    Use the account name of the statement. Else use the bank name and the last four
    characters of the IBAN. Else use the IBAN.
    """
    iban = statement.account_iban or ""
    if statement.account_name:
        name = statement.account_name
    elif statement.servicer_name:
        name = f"{statement.servicer_name} ···{iban[-4:]}"
    else:
        name = f"Account {format_iban(iban)}"
    return name.strip()[:100]


def _statement_currency(statement: camt053.Statement) -> str:
    for currency in (
        statement.currency,
        *(b.currency for b in statement.balances),
        *(t.currency for t in statement.transactions),
    ):
        if currency:
            return currency
    raise ServiceError("The statement has no currency. Add the account manually.")


def _create_account_for(
    db: Session, user_id: int, statement: camt053.Statement, budget_currency: str
) -> Account:
    """Make a checking account for the IBAN of the statement.

    The account is in the budget if it uses the budget currency. Else it is a
    tracking account. The user can change the name and the type later.
    """
    currency = _statement_currency(statement)
    return create_account(
        db,
        user_id,
        name=_new_account_name(statement),
        account_type=AccountType.CHECKING,
        currency=currency,
        budget_currency=budget_currency,
        iban=statement.account_iban,
        on_budget=currency == budget_currency,
    )


def _resolve_account(
    db: Session,
    user_id: int,
    statement: camt053.Statement,
    account_id: int | None,
    budget_currency: str | None,
) -> tuple[Account, bool]:
    """Find the account of the statement. Return the account and ``True`` if it is new.

    If no account is selected and no account has the IBAN of the statement, make an
    account if ``budget_currency`` is given.
    """
    by_iban: Account | None = None
    if statement.account_iban:
        by_iban = db.scalar(
            select(Account).where(
                Account.user_id == user_id, Account.iban == statement.account_iban
            )
        )
    if account_id is None:
        if by_iban is not None:
            return by_iban, False
        if statement.account_iban and budget_currency:
            return _create_account_for(db, user_id, statement, budget_currency), True
        if statement.account_iban:
            raise ServiceError(
                "No account has the IBAN of the statement. Select an account or add the IBAN "
                "to an account."
            )
        raise ServiceError("The statement has no IBAN. Select the account.")
    account = get_account(db, user_id, account_id)
    if by_iban is not None and by_iban.id != account.id:
        raise ServiceError("The statement IBAN belongs to a different account.")
    if account.iban and statement.account_iban and account.iban != statement.account_iban:
        raise ServiceError("The statement IBAN is not the IBAN of the selected account.")
    return account, False


def _check_account(account: Account, statement: camt053.Statement) -> None:
    if account.closed:
        raise ServiceError("The account is closed.")
    if statement.currency and statement.currency != account.currency:
        raise ServiceError(
            f"The statement currency ({statement.currency}) is not the account currency "
            f"({account.currency})."
        )
    if account.iban is None and statement.account_iban:
        account.iban = statement.account_iban


def import_statement_file(
    db: Session,
    user_id: int,
    *,
    filename: str,
    data: bytes,
    account_id: int | None = None,
    budget_currency: str | None = None,
) -> ImportResult:
    """Parse and import a camt.053 file (XML or ZIP).

    If ``account_id`` is ``None``, the IBAN of each statement gives the account. If
    ``budget_currency`` is given and no account has the IBAN, the import makes a new
    account. The import is idempotent. The same file cannot be imported two times, and a
    transaction that exists already (same fingerprint) is not added again.
    Raise :class:`ServiceError` or :class:`~feljosit.camt053.Camt053Error` on errors.
    """
    digest = hashlib.sha256(data).hexdigest()
    exists = db.scalar(
        select(ImportBatch).where(ImportBatch.user_id == user_id, ImportBatch.file_sha256 == digest)
    )
    if exists is not None:
        raise ServiceError("This file was imported before.")

    statements = camt053.parse_file(data)
    batch = ImportBatch(
        user_id=user_id,
        filename=filename[:MAX_FILENAME],
        file_sha256=digest,
        message_id=statements[0].message_id[:35] or None,
    )
    db.add(batch)
    db.flush()
    result = ImportResult()
    # Find or make all accounts first. Then a transfer to an account that this
    # file makes is a transfer too.
    targets = []
    for statement in statements:
        account, created = _resolve_account(db, user_id, statement, account_id, budget_currency)
        _check_account(account, statement)
        targets.append((statement, account, created))
    categoriser = Categoriser(db, user_id)
    # The order of the methods (see feljosit.services.categorise): first the
    # transfers from the IBAN, then the transfers from matching transactions, then
    # the rules and the history.
    pending: list[tuple[Transaction, StatementResult]] = []
    for statement, account, created in targets:
        item = _import_statement(db, categoriser, batch, account, statement, pending)
        item.account_created = created
        result.statements.append(item)
    _match_transfers(db, user_id, pending)
    for tx, item in pending:
        if tx.is_transfer:
            continue
        tx.category_id = categoriser.suggest(tx.counterparty, tx.counterparty_iban, tx.description)
        if tx.category_id is not None:
            item.categorised_count += 1
    db.flush()
    batch.new_count = result.new_count
    batch.duplicate_count = result.duplicate_count
    return result


def _import_statement(
    db: Session,
    categoriser: Categoriser,
    batch: ImportBatch,
    account: Account,
    statement: camt053.Statement,
    pending: list[tuple[Transaction, StatementResult]],
) -> StatementResult:
    """Add the new transactions of one statement.

    The new transactions of budget accounts that are not transfers go to
    ``pending``. They get a category after the matching of transfers.
    """
    result = StatementResult(
        account_id=account.id,
        account_name=account.name,
        statement_id=statement.statement_id,
        skipped_entries=statement.skipped_entries,
    )
    existing = set(
        db.scalars(select(Transaction.fingerprint).where(Transaction.account_id == account.id))
    )
    opening = statement.opening_balance
    if not existing and opening is not None and opening.amount_minor:
        add_starting_balance(db, account, opening.amount_minor, opening.date)
        existing.add(f"start:{account.id}")

    for item in statement.transactions:
        if item.currency != account.currency:
            raise ServiceError("A transaction currency is not the account currency.")
        if item.fingerprint in existing:
            result.duplicate_count += 1
            continue
        existing.add(item.fingerprint)
        tx = Transaction(
            user_id=account.user_id,
            account_id=account.id,
            booking_date=item.booking_date,
            value_date=item.value_date,
            amount_minor=item.amount_minor,
            currency=item.currency,
            counterparty=item.counterparty,
            counterparty_iban=item.counterparty_iban,
            description=item.description,
            reference=item.reference[:35] if item.reference else None,
            bank_reference=item.bank_reference,
            source=TransactionSource.CAMT053,
            fingerprint=item.fingerprint,
            import_batch_id=batch.id,
        )
        if categoriser.is_transfer(account, item.counterparty_iban):
            tx.is_transfer = True
            result.transfer_count += 1
        elif account.on_budget:
            pending.append((tx, result))
        db.add(tx)
        result.new_count += 1

    db.flush()
    closing = statement.closing_balance
    if closing is not None:
        result.closing_balance_minor = closing.amount_minor
        result.computed_balance_minor = _balance_on(db, account, closing.date)
    return result


def _match_transfers(
    db: Session, user_id: int, pending: list[tuple[Transaction, StatementResult]]
) -> None:
    """Find the transfers between the new transactions and all other transactions."""
    if not pending:
        return
    db.flush()
    dates = [tx.booking_date for tx, _ in pending]
    pairs = transfers.match_transfers(db, user_id, min(dates), max(dates))
    by_id = {id(tx): item for tx, item in pending}
    for pair in pairs:
        for tx in pair:
            item = by_id.get(id(tx))
            if item is not None:
                item.transfer_count += 1
                item.matched_transfer_count += 1


def _balance_on(db: Session, account: Account, on: date) -> int:
    total = db.scalar(
        select(func.coalesce(func.sum(Transaction.amount_minor), 0)).where(
            Transaction.account_id == account.id, Transaction.booking_date <= on
        )
    )
    return int(total or 0)
