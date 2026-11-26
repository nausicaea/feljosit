"""The import adds unknown accounts, and the user can edit accounts."""

import io
import re
import zipfile

import pytest
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.camt053 import parse_xml
from feljosit.models import (
    Account,
    AccountType,
    Category,
    CategoryKind,
    Transaction,
    TransactionSource,
    User,
)
from feljosit.services import accounts, categorise
from feljosit.services.errors import ServiceError
from feljosit.services.importer import import_statement_file
from tests.conftest import fixture_bytes

PRIVATE_IBAN = "CH8900235000012345678"
SAVINGS_IBAN = "CH0300700110000123456"
PRIVATE = "camt053_v04_private.xml"
SAVINGS = "camt053_v08_savings.xml"


def _import(db: Session, user: User, data: bytes, name: str = "statement.xml"):  # type: ignore[no-untyped-def]
    return import_statement_file(db, user.id, filename=name, data=data, budget_currency="CHF")


def _with_names(data: bytes, *, account: str | None = None, bank: str | None = None) -> bytes:
    """Add an account name and a bank name to the private statement."""
    text = data.decode()
    if account:
        text = text.replace("<Ccy>CHF</Ccy>\n", f"<Ccy>CHF</Ccy>\n<Nm>{account}</Nm>\n", 1)
    if bank:
        text = text.replace(
            "</Ownr>", f"</Ownr>\n<Svcr><FinInstnId><Nm>{bank}</Nm></FinInstnId></Svcr>", 1
        )
    return text.encode()


def _accounts(db: Session, user: User) -> list[Account]:
    return list(db.scalars(select(Account).where(Account.user_id == user.id)))


def _txs(db: Session, account: Account) -> list[Transaction]:
    return list(db.scalars(select(Transaction).where(Transaction.account_id == account.id)))


def test_parser_reads_account_and_bank_names() -> None:
    [statement] = parse_xml(_with_names(fixture_bytes(PRIVATE), account="Privat", bank="Bank X"))
    assert statement.account_name == "Privat"
    assert statement.servicer_name == "Bank X"
    [plain] = parse_xml(fixture_bytes(PRIVATE))
    assert plain.account_name is None and plain.servicer_name is None


def test_import_adds_unknown_account(db: Session, user: User) -> None:
    result = _import(db, user, fixture_bytes(PRIVATE))
    [stmt] = result.statements
    assert stmt.account_created is True
    assert stmt.new_count == 7
    assert stmt.balance_matches is True
    [account] = _accounts(db, user)
    assert account.name == "Account CH89 0023 5000 0123 4567 8"
    assert (account.iban, account.currency) == (PRIVATE_IBAN, "CHF")
    assert account.type == AccountType.CHECKING and account.on_budget
    assert accounts.balances(db, user.id)[account.id] == 425005
    # The next statement for the same IBAN uses the same account.
    again = import_statement_file(
        db,
        user.id,
        filename="again.xml",
        data=fixture_bytes(PRIVATE) + b"\n",
        budget_currency="CHF",
    )
    assert again.statements[0].account_created is False
    assert len(_accounts(db, user)) == 1


@pytest.mark.parametrize(
    ("names", "expected"),
    [
        ({"account": "Privatkonto", "bank": "Bank X"}, "Privatkonto"),
        ({"bank": "Bank X"}, "Bank X ···5678"),
    ],
)
def test_new_account_name_from_statement(
    db: Session, user: User, names: dict[str, str], expected: str
) -> None:
    _import(db, user, _with_names(fixture_bytes(PRIVATE), **names))
    assert [a.name for a in _accounts(db, user)] == [expected]


def test_new_account_in_foreign_currency_is_tracking(db: Session, user: User) -> None:
    data = fixture_bytes(SAVINGS).replace(b"CHF", b"EUR")
    _import(db, user, data)
    [account] = _accounts(db, user)
    assert account.currency == "EUR"
    assert not account.on_budget


def test_import_adds_two_accounts_and_finds_transfers(db: Session, user: User) -> None:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("private.xml", fixture_bytes(PRIVATE))
        archive.writestr("savings.xml", fixture_bytes(SAVINGS))
    result = _import(db, user, buffer.getvalue(), "both.zip")
    assert [s.account_created for s in result.statements] == [True, True]
    # The private statement comes first, but its transfer to the new savings
    # account is found: the import adds all accounts before the transactions.
    assert [s.transfer_count for s in result.statements] == [1, 1]
    assert len(_accounts(db, user)) == 2


def test_new_account_finds_transfers_imported_before(db: Session, user: User) -> None:
    _import(db, user, fixture_bytes(PRIVATE))
    [private] = _accounts(db, user)
    to_savings = next(t for t in _txs(db, private) if t.counterparty_iban == SAVINGS_IBAN)
    assert to_savings.is_transfer is False  # No account has the savings IBAN yet.
    _import(db, user, fixture_bytes(SAVINGS), "savings.xml")
    after = db.get(Transaction, to_savings.id)
    assert after is not None
    assert after.is_transfer is True
    assert after.category_id is None


def test_no_new_account_without_budget_currency_or_iban(db: Session, user: User) -> None:
    with pytest.raises(ServiceError, match="No account has the IBAN"):
        import_statement_file(db, user.id, filename="x.xml", data=fixture_bytes(PRIVATE))
    no_iban = fixture_bytes(SAVINGS).replace(
        f"<IBAN>{SAVINGS_IBAN}</IBAN></Id>".encode(), b"<Othr><Id>12345</Id></Othr></Id>", 1
    )
    with pytest.raises(ServiceError, match="no IBAN"):
        _import(db, user, no_iban)
    assert _accounts(db, user) == []


def test_selected_account_is_not_replaced(db: Session, user: User) -> None:
    account = accounts.create_account(
        db,
        user.id,
        name="Savings",
        account_type=AccountType.SAVINGS,
        currency="CHF",
        budget_currency="CHF",
        iban=SAVINGS_IBAN,
    )
    with pytest.raises(ServiceError, match="IBAN"):
        import_statement_file(
            db,
            user.id,
            filename="x.xml",
            data=fixture_bytes(PRIVATE),
            account_id=account.id,
            budget_currency="CHF",
        )
    assert len(_accounts(db, user)) == 1


def test_update_account_name_and_type(db: Session, user: User) -> None:
    _import(db, user, fixture_bytes(PRIVATE))
    [account] = _accounts(db, user)
    accounts.update_account(
        db,
        account,
        name="  UBS private  ",
        account_type=AccountType.CREDIT_CARD,
        budget_currency="CHF",
    )
    assert (account.name, account.type, account.on_budget) == (
        "UBS private",
        AccountType.CREDIT_CARD,
        True,
    )
    with pytest.raises(ServiceError, match="1 to 100"):
        accounts.update_account(
            db, account, name=" ", account_type=AccountType.CHECKING, budget_currency="CHF"
        )


def test_update_account_refuses_budget_in_foreign_currency(db: Session, user: User) -> None:
    _import(db, user, fixture_bytes(SAVINGS).replace(b"CHF", b"EUR"))
    [account] = _accounts(db, user)
    with pytest.raises(ServiceError, match="budget currency"):
        accounts.update_account(
            db, account, name="EUR", account_type=AccountType.SAVINGS, budget_currency="CHF"
        )
    assert not account.on_budget


def test_change_to_tracking_and_back_updates_transactions(db: Session, user: User) -> None:
    _import(db, user, fixture_bytes(PRIVATE))
    _import(db, user, fixture_bytes(SAVINGS), "savings.xml")
    private, savings = sorted(_accounts(db, user), key=lambda a: a.iban != PRIVATE_IBAN)
    income = db.scalar(
        select(Category)
        .where(Category.user_id == user.id, Category.kind == CategoryKind.INCOME)
        .order_by(Category.id)
    )
    assert income is not None
    interest = next(
        t for t in _txs(db, savings) if t.counterparty_iban is None and t.amount_minor == 250
    )
    categorise.set_category(db, interest, income.id)

    def transfer_flags() -> tuple[bool, bool]:
        to_savings = next(t for t in _txs(db, private) if t.counterparty_iban == SAVINGS_IBAN)
        from_private = next(t for t in _txs(db, savings) if t.counterparty_iban == PRIVATE_IBAN)
        return to_savings.is_transfer, from_private.is_transfer

    assert transfer_flags() == (True, True)

    # Savings becomes a 3a account: a tracking account.
    accounts.update_account(
        db, savings, name="3a", account_type=AccountType.PILLAR_3A, budget_currency="CHF"
    )
    assert not savings.on_budget
    assert all(t.category_id is None for t in _txs(db, savings))
    # The money now leaves the budget: the movement needs a category.
    assert transfer_flags() == (False, False)

    # Back to a budget account.
    accounts.update_account(
        db,
        savings,
        name="Savings",
        account_type=AccountType.PILLAR_3A,
        budget_currency="CHF",
        on_budget=True,
    )
    assert accounts.get_account(db, user.id, savings.id).on_budget
    assert transfer_flags() == (True, True)
    start = next(t for t in _txs(db, savings) if t.source == TransactionSource.STARTING_BALANCE)
    assert start.category_id == income.id


def test_web_import_adds_account_and_edit_page(client: tuple[TestClient, str]) -> None:
    http, token = client
    response = http.post(
        "/import",
        data={"csrf_token": token, "account_id": ""},
        files={"statement": (PRIVATE, fixture_bytes(PRIVATE), "application/xml")},
    )
    assert "The import added the account" in response.text
    match = re.search(r'href="/accounts/(\d+)/edit"', response.text)
    assert match
    account_id = match.group(1)
    page = http.get(f"/accounts/{account_id}/edit")
    assert page.status_code == 200
    assert "Account CH89 0023 5000 0123 4567 8" in page.text
    response = http.post(
        f"/accounts/{account_id}",
        data={
            "csrf_token": token,
            "name": "UBS private",
            "account_type": "checking",
            "budget_mode": "tracking",
        },
    )
    assert response.status_code == 200
    assert "The account &#39;UBS private&#39; is changed." in response.text
    assert "Tracking" in response.text
    # An error goes back to the edit page.
    response = http.post(
        f"/accounts/{account_id}",
        data={"csrf_token": token, "name": " ", "account_type": "checking"},
    )
    assert "1 to 100 characters" in response.text
    assert http.get("/accounts/99999/edit").status_code == 404
    # The CSRF token is necessary.
    response = http.post(f"/accounts/{account_id}", data={"name": "x", "account_type": "checking"})
    assert response.status_code in {400, 403}
