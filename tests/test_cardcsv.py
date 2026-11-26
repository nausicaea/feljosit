"""The import of credit card CSV files (see ADR-0017)."""

from datetime import date

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit import cardcsv
from feljosit.cardcsv import CardCsvError
from feljosit.models import AccountType, Transaction, TransactionSource, User
from feljosit.services import accounts
from feljosit.services.errors import ServiceError
from feljosit.services.importer import import_statement_file
from tests.conftest import fixture_bytes

HEADER = (
    "TransactionId,CardId,Date,ValutaDate,Amount,Currency,OriginalAmount,OriginalCurrency,"
    "MerchantName,MerchantPlace,MerchantCountry,StateType,Details,Type,Exchange Rate"
)
#: The example line of the issue (with an anonymised transaction and card ID).
FEE = (
    "TRX2025102300001112211,443318XX00000001,2025-10-23 00:00:00,2025-10-23 00:00:00,"
    "75.000,CHF,75.000,CHF,,0.000,75.000,BOOKED,Belastung Jahresgebühr,merchant,1.000000"
)


def _csv(*lines: str, header: str = HEADER) -> bytes:
    return "\n".join([header, *lines]).encode()


def test_parse_the_example_line() -> None:
    statement = cardcsv.parse_file(_csv(FEE))
    [tx] = statement.transactions
    assert tx.booking_date == date(2025, 10, 23)
    assert tx.value_date == date(2025, 10, 23)
    assert tx.amount_minor == -7500  # A charge is an outflow.
    assert tx.currency == "CHF"
    # No merchant: the details are the counterparty. The numbers in the merchant
    # place and country are not text.
    assert tx.counterparty == "Belastung Jahresgebühr"
    assert tx.description == ""
    assert tx.counterparty_iban is None
    assert tx.bank_reference == "TRX2025102300001112211"
    assert tx.fingerprint.startswith("card:")
    assert statement.account_iban is None
    assert statement.currency == "CHF"
    assert statement.skipped_entries == 0


def test_parse_the_fixture() -> None:
    statement = cardcsv.parse_file(fixture_bytes("card_transactions.csv"))
    assert statement.skipped_entries == 1  # The authorisation is not booked.
    _fee, migros, cafe, payment = statement.transactions
    assert migros.counterparty == "Migros"
    assert migros.description == "Migros Zürich · Zürich, CHE"
    assert migros.value_date == date(2025, 10, 27)
    assert cafe.counterparty == 'Café "Zum Hafen"'
    assert cafe.amount_minor == -2740
    assert cafe.description == (
        'Café "Zum Hafen", Konstanz · Konstanz, DEU · '
        "Original amount EUR 29.99, exchange rate 0.913638"
    )
    assert payment.amount_minor == 120000  # A payment to the card is an inflow.
    assert payment.counterparty == "Ihre Zahlung - Danke"
    assert len({tx.fingerprint for tx in statement.transactions}) == 4


def test_parse_semicolons_bom_and_windows_encoding() -> None:
    text = "﻿" + "\r\n".join([HEADER.replace(",", ";"), FEE.replace(",", ";")])
    [tx] = cardcsv.parse_file(text.encode("utf-8")).transactions
    assert tx.amount_minor == -7500
    [tx] = cardcsv.parse_file(_csv(FEE).decode().encode("cp1252")).transactions
    assert tx.counterparty == "Belastung Jahresgebühr"


def test_detection() -> None:
    assert cardcsv.looks_like_card_csv(_csv(FEE))
    assert cardcsv.looks_like_card_csv(fixture_bytes("card_transactions.csv"))
    assert not cardcsv.looks_like_card_csv(fixture_bytes("camt053_v04_private.xml"))
    assert not cardcsv.looks_like_card_csv(b"Date,Amount\n2025-10-23,75.00\n")


def _line(**changes: str) -> str:
    values = dict(zip(HEADER.split(","), FEE.split(","), strict=True))
    values.update(changes)
    return ",".join(values.values())


@pytest.mark.parametrize(
    ("lines", "message"),
    [
        ((_line(Amount="abc"),), "Line 2 has an amount that is not valid."),
        ((_line(Amount="75.005"),), "Line 2 has an amount or a currency"),
        ((_line(Currency="Swiss francs"),), "Line 2 has an amount or a currency"),
        ((_line(Date=""),), "Line 2 has no date."),
        ((_line(Date="23/10/2025"),), "Line 2 has a date that is not valid."),
        ((_line(TransactionId=""),), "Line 2 has no transaction ID."),
        ((FEE, FEE + ",extra"), "Line 3 does not have 15 columns."),
        ((_line(Details="x" * 1001),), "Line 2 has a field that is too long."),
        ((FEE, _line(TransactionId="T2", Currency="EUR")), "more than one currency"),
        (('"unclosed',), "The CSV syntax of the file is not valid."),
    ],
)
def test_parse_errors(lines: tuple[str, ...], message: str) -> None:
    with pytest.raises(CardCsvError, match=message):
        cardcsv.parse_file(_csv(*lines))


def test_error_messages_do_not_include_values() -> None:
    with pytest.raises(CardCsvError) as info:
        cardcsv.parse_file(_csv(_line(Amount="secret-value")))
    assert "secret" not in str(info.value)


def test_missing_columns() -> None:
    with pytest.raises(CardCsvError, match="columns are missing"):
        cardcsv.parse_file(_csv(FEE, header=HEADER.replace("StateType", "State")))


def test_row_limit(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(cardcsv, "MAX_ROWS", 2)
    lines = [_line(TransactionId=f"T{i}") for i in range(3)]
    with pytest.raises(CardCsvError, match="more than 2 lines"):
        cardcsv.parse_file(_csv(*lines))


def _card(db: Session, user: User) -> int:
    return accounts.create_account(
        db,
        user.id,
        name="Credit card",
        account_type=AccountType.CREDIT_CARD,
        currency="CHF",
        budget_currency="CHF",
    ).id


def _import(db: Session, user: User, data: bytes, account_id: int | None, name: str = "card.csv"):  # type: ignore[no-untyped-def]
    return import_statement_file(db, user.id, filename=name, data=data, account_id=account_id)


def test_import_into_the_selected_account(db: Session, user: User) -> None:
    card = _card(db, user)
    result = _import(db, user, fixture_bytes("card_transactions.csv"), card)
    [item] = result.statements
    assert item.account_id == card
    assert item.new_count == 4
    assert item.skipped_entries == 1
    assert item.balance_matches is None  # The file has no balance.
    rows = list(db.scalars(select(Transaction).where(Transaction.account_id == card)))
    assert {tx.source for tx in rows} == {TransactionSource.CARD_CSV}
    assert accounts.balances(db, user.id)[card] == -7500 - 4535 - 2740 + 120000


def test_import_ignores_known_transactions(db: Session, user: User) -> None:
    card = _card(db, user)
    _import(db, user, _csv(FEE), card)
    with pytest.raises(ServiceError, match="imported before"):
        _import(db, user, _csv(FEE), card)
    # An overlapping export: only the new transactions are added.
    result = _import(db, user, fixture_bytes("card_transactions.csv"), card, "later.csv")
    assert result.new_count == 3
    assert result.duplicate_count == 1


def test_import_needs_the_account(db: Session, user: User) -> None:
    with pytest.raises(ServiceError, match="Select the credit card account"):
        _import(db, user, _csv(FEE), None)


def test_import_refuses_a_different_currency(db: Session, user: User) -> None:
    card = _card(db, user)
    with pytest.raises(ServiceError, match="currency"):
        _import(db, user, _csv(_line(Currency="EUR")), card)


def test_payment_from_the_private_account_is_a_transfer(db: Session, user: User) -> None:
    """The payment to the card and the debit on the private account are one transfer."""
    private = accounts.create_account(
        db,
        user.id,
        name="Private",
        account_type=AccountType.CHECKING,
        currency="CHF",
        budget_currency="CHF",
    )
    accounts.add_manual_transaction(
        db,
        private,
        booking_date=date(2025, 10, 29),
        amount_minor=-120000,
        counterparty="Card issuer",
    )
    card = _card(db, user)
    result = _import(db, user, fixture_bytes("card_transactions.csv"), card)
    assert result.statements[0].matched_transfer_count == 1
    payment = db.scalar(select(Transaction).where(Transaction.amount_minor == 120000))
    assert payment is not None
    assert payment.is_transfer


def test_upload_on_the_import_page(client: tuple[TestClient, str], app: FastAPI) -> None:
    http, token = client
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None
    card = _card(db, user)
    db.commit()
    db.close()

    def upload(account_id: str):  # type: ignore[no-untyped-def]
        return http.post(
            "/import",
            data={"csrf_token": token, "account_id": account_id},
            files={"statement": ("card.csv", fixture_bytes("card_transactions.csv"), "text/csv")},
        )

    response = upload("")
    assert response.status_code == 400
    assert "Select the credit card account." in response.text
    response = upload(str(card))
    assert response.status_code == 200
    assert "4 new transactions" in response.text
    assert "1 entries are not booked (pending) and were ignored" in response.text
    page = http.get("/transactions", params={"account": str(card)}).text
    assert "<strong>Migros</strong>" in page
