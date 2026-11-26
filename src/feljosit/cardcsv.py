"""Parser for the CSV export of credit card transactions.

The file has one header line and one line for each card transaction, for example::

    TransactionId,CardId,Date,ValutaDate,Amount,Currency,OriginalAmount,
    OriginalCurrency,MerchantName,MerchantPlace,MerchantCountry,StateType,Details,
    Type,Exchange Rate

(one line in the file). See ADR-0017.

* A positive ``Amount`` is a charge to the card (an outflow). A negative amount is a
  credit, for example a payment to the card or a refund.
* Only lines with the ``StateType`` ``BOOKED`` become transactions. The parser
  counts the other lines (for example pending authorisations) as skipped entries.
* ``TransactionId`` gives the fingerprint. Thus an import of an overlapping file does
  not add a transaction two times.
* The file has no IBAN and no balance. The user selects the account.

Security: the caller limits the size of the input. The parser limits the number of
lines and the length of the fields. Error messages do not include values from the
file.
"""

from __future__ import annotations

import csv
import hashlib
import io
import re
from datetime import date, datetime
from decimal import Decimal

from feljosit.camt053 import Statement, StatementTransaction
from feljosit.money import MoneyError, format_minor, to_minor, validate_currency

#: The columns that the parser needs. The other columns are optional.
REQUIRED_COLUMNS = ("TransactionId", "Date", "Amount", "Currency", "StateType")
MAX_ROWS = 100_000
MAX_FIELD_CHARS = 1_000

_AMOUNT_RE = re.compile(r"^[+-]?\d{1,12}(\.\d{1,5})?$")
#: The export puts numbers in the merchant fields of some lines (for example fees).
_HAS_LETTER_RE = re.compile(r"[^\W\d_]")


class CardCsvError(ValueError):
    """The file is not a valid credit card CSV file."""


def _decode(data: bytes) -> str:
    try:
        return data.decode("utf-8-sig")
    except UnicodeDecodeError:
        # Older exports from Windows programs.
        return data.decode("cp1252", errors="replace")


def _header_line(text: str) -> str:
    return text.lstrip("﻿").split("\n", 1)[0].strip()


def _delimiter(header: str) -> str:
    return ";" if header.count(";") > header.count(",") else ","


def looks_like_card_csv(data: bytes) -> bool:
    """Return true if the first line of the file is the header of a credit card CSV file."""
    header = _header_line(_decode(data[:4096]))
    names = {name.strip().strip('"') for name in header.split(_delimiter(header))}
    return all(column in names for column in REQUIRED_COLUMNS)


def _date(text: str, line: int) -> date | None:
    """Parse ``2025-10-23 00:00:00``, ``2025-10-23`` or ``23.10.2025``. Empty: ``None``."""
    text = text.strip()
    if not text:
        return None
    for value, pattern in ((text[:10], "%Y-%m-%d"), (text[:10], "%d.%m.%Y")):
        try:
            return datetime.strptime(value, pattern).date()
        except ValueError:
            continue
    raise CardCsvError(f"Line {line} has a date that is not valid.")


def _decimal(text: str, line: int) -> Decimal:
    cleaned = text.strip().replace("'", "")
    if not _AMOUNT_RE.fullmatch(cleaned):
        raise CardCsvError(f"Line {line} has an amount that is not valid.")
    return Decimal(cleaned)


def _text(value: str) -> str:
    """A text field. A field without a letter (for example ``0.000``) is empty."""
    value = " ".join(value.split())
    return value if _HAS_LETTER_RE.search(value) else ""


def _original(amount: str, currency: str) -> str:
    """Format the original amount, for example ``EUR 29.99``. Else keep the text."""
    try:
        minor = to_minor(Decimal(amount.strip().replace("'", "")), validate_currency(currency))
    except (MoneyError, ArithmeticError):
        return f"{currency} {amount}"
    return format_minor(minor, currency, with_currency=True)


def _description(get: dict[str, str], counterparty: str, currency: str) -> str:
    parts = []
    details = _text(get.get("Details", ""))
    if details and details != counterparty:
        parts.append(details)
    place = ", ".join(
        p for p in (_text(get.get("MerchantPlace", "")), _text(get.get("MerchantCountry", ""))) if p
    )
    if place:
        parts.append(place)
    original_currency = get.get("OriginalCurrency", "").strip().upper()
    original_amount = get.get("OriginalAmount", "").strip()
    if original_currency and original_currency != currency and original_amount:
        original = f"Original amount {_original(original_amount, original_currency)}"
        rate = get.get("Exchange Rate", "").strip()
        if rate:
            original += f", exchange rate {rate}"
        parts.append(original)
    return " · ".join(parts)


def _transaction(get: dict[str, str], line: int) -> StatementTransaction:
    transaction_id = get["TransactionId"].strip()
    if not transaction_id:
        raise CardCsvError(f"Line {line} has no transaction ID.")
    try:
        currency = validate_currency(get["Currency"])
        # A charge is positive in the file. In the application, an outflow is negative.
        amount_minor = -to_minor(_decimal(get["Amount"], line), currency)
    except MoneyError as exc:
        raise CardCsvError(f"Line {line} has an amount or a currency that is not valid.") from exc
    booking_date = _date(get["Date"], line)
    if booking_date is None:
        raise CardCsvError(f"Line {line} has no date.")
    counterparty = _text(get.get("MerchantName", "")) or _text(get.get("Details", ""))
    digest = hashlib.sha256(transaction_id.encode()).hexdigest()
    return StatementTransaction(
        fingerprint=f"card:{digest[:64]}",
        booking_date=booking_date,
        value_date=_date(get.get("ValutaDate", ""), line),
        amount_minor=amount_minor,
        currency=currency,
        counterparty=counterparty[:140],
        counterparty_iban=None,
        description=_description(get, counterparty, currency),
        reference=None,
        bank_reference=transaction_id[:35],
        reversal=False,
    )


def parse_file(data: bytes) -> Statement:
    """Parse a credit card CSV file. Raise :class:`CardCsvError` if it is not valid."""
    text = _decode(data).lstrip("﻿")
    reader = csv.reader(io.StringIO(text), delimiter=_delimiter(_header_line(text)), strict=True)
    try:
        header = [name.strip() for name in next(reader, [])]
        missing = [column for column in REQUIRED_COLUMNS if column not in header]
        if missing:
            raise CardCsvError("The file is not a credit card CSV file: columns are missing.")
        transactions: list[StatementTransaction] = []
        skipped = 0
        for row in reader:
            line = reader.line_num
            if not any(field.strip() for field in row):
                continue
            if line > MAX_ROWS + 1:
                raise CardCsvError(f"The file has more than {MAX_ROWS} lines.")
            if len(row) != len(header):
                raise CardCsvError(f"Line {line} does not have {len(header)} columns.")
            if any(len(field) > MAX_FIELD_CHARS for field in row):
                raise CardCsvError(f"Line {line} has a field that is too long.")
            get = dict(zip(header, row, strict=True))
            if get["StateType"].strip().upper() != "BOOKED":
                skipped += 1
                continue
            transactions.append(_transaction(get, line))
    except csv.Error as exc:
        raise CardCsvError("The CSV syntax of the file is not valid.") from exc
    currencies = {tx.currency for tx in transactions}
    if len(currencies) > 1:
        raise CardCsvError("The file has transactions in more than one currency.")
    return Statement(
        message_id="",
        version=0,
        statement_id="",
        account_iban=None,
        account_other_id=None,
        currency=next(iter(currencies), None),
        balances=(),
        transactions=tuple(transactions),
        skipped_entries=skipped,
    )
