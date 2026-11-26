import io
import zipfile
from datetime import date

import pytest
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import AccountType, Category, MatchType, RuleField, Transaction, User
from feljosit.services import accounts, categorise
from feljosit.services.errors import ServiceError
from feljosit.services.importer import import_statement_file
from tests.conftest import fixture_bytes

PRIVATE_IBAN = "CH8900235000012345678"
SAVINGS_IBAN = "CH0300700110000123456"


def _account(db: Session, user: User, iban: str = PRIVATE_IBAN, **kw: object) -> int:
    account = accounts.create_account(
        db,
        user.id,
        name=kw.pop("name", "Private"),  # type: ignore[arg-type]
        account_type=kw.pop("account_type", AccountType.CHECKING),  # type: ignore[arg-type]
        currency="CHF",
        budget_currency="CHF",
        iban=iban,
        **kw,  # type: ignore[arg-type]
    )
    return account.id


def _category(db: Session, user: User, name: str) -> Category:
    category = db.scalar(select(Category).where(Category.user_id == user.id, Category.name == name))
    assert category is not None
    return category


def _import(db: Session, user: User, name: str, account_id: int | None = None):  # type: ignore[no-untyped-def]
    return import_statement_file(
        db, user.id, filename=name, data=fixture_bytes(name), account_id=account_id
    )


def test_import_finds_account_by_iban_and_checks_balance(db: Session, user: User) -> None:
    account_id = _account(db, user)
    result = _import(db, user, "camt053_v04_private.xml")
    [stmt] = result.statements
    assert stmt.account_id == account_id
    assert stmt.new_count == 7
    assert stmt.skipped_entries == 1
    # Starting balance from OPBD and all entries give the closing balance.
    assert stmt.balance_matches is True
    assert accounts.balances(db, user.id)[account_id] == 425005


def test_import_is_idempotent(db: Session, user: User) -> None:
    _account(db, user)
    _import(db, user, "camt053_v04_private.xml")
    with pytest.raises(ServiceError, match="imported before"):
        _import(db, user, "camt053_v04_private.xml")
    # The same content with a different file hash: no duplicate transactions.
    data = fixture_bytes("camt053_v04_private.xml") + b"\n"
    result = import_statement_file(db, user.id, filename="again.xml", data=data)
    assert result.new_count == 0
    assert result.duplicate_count == 7


def test_import_needs_account(db: Session, user: User) -> None:
    with pytest.raises(ServiceError, match="No account"):
        _import(db, user, "camt053_v04_private.xml")


def test_import_assigns_iban_to_selected_account(db: Session, user: User) -> None:
    account_id = _account(db, user, iban="")
    _import(db, user, "camt053_v04_private.xml", account_id)
    assert accounts.get_account(db, user.id, account_id).iban == PRIVATE_IBAN


def test_import_refuses_iban_mismatch(db: Session, user: User) -> None:
    account_id = _account(db, user, iban=SAVINGS_IBAN)
    with pytest.raises(ServiceError, match="IBAN"):
        _import(db, user, "camt053_v04_private.xml", account_id)


def test_import_refuses_account_of_other_user(db: Session, user: User) -> None:
    from feljosit.services import users
    from feljosit.services.errors import NotFoundError

    other = users.create_user(db, "bob", "another long password")
    other_account = _account(db, other)
    with pytest.raises(NotFoundError):
        _import(db, user, "camt053_v04_private.xml", other_account)


def _zip(files: dict[str, bytes]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            archive.writestr(name, content)
    return buffer.getvalue()


def _two_accounts_zip() -> bytes:
    return _zip(
        {
            "private.xml": fixture_bytes("camt053_v04_private.xml"),
            "savings.xml": fixture_bytes("camt053_v08_savings.xml"),
        }
    )


def _two_statements_xml() -> bytes:
    """One XML file with two statements: the savings statement and a copy for PRIVATE_IBAN."""
    text = fixture_bytes("camt053_v08_savings.xml").decode()
    start = text.index("<Stmt>")
    end = text.index("</Stmt>") + len("</Stmt>")
    copy = (
        text[start:end]
        .replace("<Id>SAV-2026-10</Id>", "<Id>PRIV-2026-10</Id>", 1)
        .replace(f"<IBAN>{SAVINGS_IBAN}</IBAN></Id>", f"<IBAN>{PRIVATE_IBAN}</IBAN></Id>", 1)
    )
    return (text[:end] + copy + text[end:]).encode()


def _transaction_count(db: Session, account_id: int) -> int:
    return len(list(db.scalars(select(Transaction).where(Transaction.account_id == account_id))))


def test_import_zip_with_two_accounts(db: Session, user: User) -> None:
    private_id = _account(db, user)
    savings_id = _account(
        db, user, iban=SAVINGS_IBAN, name="Savings", account_type=AccountType.SAVINGS
    )
    result = import_statement_file(db, user.id, filename="both.zip", data=_two_accounts_zip())
    private, savings = result.statements
    assert (private.account_id, private.new_count) == (private_id, 7)
    assert (savings.account_id, savings.new_count) == (savings_id, 2)
    assert private.balance_matches is True
    assert savings.balance_matches is True
    # The transfer between the two accounts is found in the same import.
    assert private.transfer_count == 1
    assert savings.transfer_count == 1
    balances = accounts.balances(db, user.id)
    assert balances[private_id] == 425005
    assert balances[savings_id] == 1050250
    assert result.new_count == 9


def test_import_one_xml_with_two_accounts(db: Session, user: User) -> None:
    private_id = _account(db, user)
    savings_id = _account(
        db, user, iban=SAVINGS_IBAN, name="Savings", account_type=AccountType.SAVINGS
    )
    result = import_statement_file(db, user.id, filename="both.xml", data=_two_statements_xml())
    assert [(s.account_id, s.statement_id, s.new_count) for s in result.statements] == [
        (savings_id, "SAV-2026-10", 2),
        (private_id, "PRIV-2026-10", 2),
    ]
    assert all(s.balance_matches for s in result.statements)
    assert _transaction_count(db, private_id) == 3  # Starting balance and 2 entries.
    assert _transaction_count(db, savings_id) == 3


def test_import_with_two_accounts_refuses_selected_account(db: Session, user: User) -> None:
    private_id = _account(db, user)
    _account(db, user, iban=SAVINGS_IBAN, name="Savings", account_type=AccountType.SAVINGS)
    with pytest.raises(ServiceError, match="different account"):
        import_statement_file(
            db, user.id, filename="both.zip", data=_two_accounts_zip(), account_id=private_id
        )


def test_transfers_between_budget_accounts(db: Session, user: User) -> None:
    _account(db, user)
    _account(db, user, iban=SAVINGS_IBAN, name="Savings", account_type=AccountType.SAVINGS)
    first = _import(db, user, "camt053_v04_private.xml")
    second = _import(db, user, "camt053_v08_savings.xml")
    assert first.statements[0].transfer_count == 1
    assert second.statements[0].transfer_count == 1


def test_transfer_to_tracking_account_is_not_a_transfer(db: Session, user: User) -> None:
    _account(db, user)
    _account(db, user, iban=SAVINGS_IBAN, name="3a", account_type=AccountType.PILLAR_3A)
    result = _import(db, user, "camt053_v04_private.xml")
    assert result.statements[0].transfer_count == 0


def test_rules_and_history_categorise(db: Session, user: User) -> None:
    _account(db, user)
    groceries = _category(db, user, "Groceries")
    categorise.create_rule(
        db,
        user.id,
        field=RuleField.COUNTERPARTY,
        match_type=MatchType.CONTAINS,
        pattern="migros",
        category_id=groceries.id,
    )
    salary = _category(db, user, "Salary")
    # History: a manual transaction from the employer with a category.
    account_id = _account(db, user, iban="", name="Cash", account_type=AccountType.CASH)
    tx = accounts.add_manual_transaction(
        db,
        accounts.get_account(db, user.id, account_id),
        booking_date=date(2026, 9, 25),
        amount_minor=100,
        counterparty="Muster AG",
        category_id=salary.id,
    )
    assert tx.category_id == salary.id
    result = _import(db, user, "camt053_v04_private.xml")
    assert result.statements[0].categorised_count == 2
    by_name = {
        t.counterparty: t.category_id
        for t in db.scalars(select(Transaction).where(Transaction.user_id == user.id))
    }
    assert by_name["Debit card payment MIGROS ZUERICH"] == groceries.id
    assert by_name["Muster AG"] == salary.id


def test_apply_rules_later(db: Session, user: User) -> None:
    _account(db, user)
    _import(db, user, "camt053_v04_private.xml")
    rent = _category(db, user, "Rent and service charges")
    categorise.create_rule(
        db,
        user.id,
        field=RuleField.COUNTERPARTY_IBAN,
        match_type=MatchType.EQUALS,
        pattern="CH3880808001234567890",
        category_id=rent.id,
    )
    assert categorise.apply_to_uncategorised(db, user.id) == 1


def test_rule_validation(db: Session, user: User) -> None:
    category = _category(db, user, "Groceries")
    with pytest.raises(ServiceError):
        categorise.create_rule(
            db,
            user.id,
            field=RuleField.COUNTERPARTY,
            match_type=MatchType.CONTAINS,
            pattern="  ",
            category_id=category.id,
        )


def test_on_budget_account_needs_budget_currency(db: Session, user: User) -> None:
    with pytest.raises(ServiceError, match="budget currency"):
        accounts.create_account(
            db,
            user.id,
            name="EUR",
            account_type=AccountType.CHECKING,
            currency="EUR",
            budget_currency="CHF",
        )
    account = accounts.create_account(
        db,
        user.id,
        name="EUR",
        account_type=AccountType.CHECKING,
        currency="EUR",
        budget_currency="CHF",
        on_budget=False,
    )
    assert not account.on_budget


def test_duplicate_iban_refused(db: Session, user: User) -> None:
    _account(db, user)
    with pytest.raises(ServiceError, match="exists"):
        _account(db, user)
