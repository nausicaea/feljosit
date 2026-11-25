"""Parser for ISO 20022 camt.053 bank-to-customer statements.

The parser supports the message versions camt.053.001.02 to camt.053.001.13.
Swiss banks use these versions as specified in the Swiss Payment Standards (SPS).

Security (see ADR-0005):

* The parser uses :mod:`defusedxml`. It refuses DTDs, entity declarations and
  external references. This prevents XXE and "billion laughs" attacks.
* The tree builder limits the number of elements, the nesting depth and the
  number of attributes of each element. This limits the memory and the time
  that one document can use.
* Amounts must have the ISO 20022 decimal syntax without an exponent. Thus a
  value such as ``1E999999`` cannot use the CPU for a long time.
* The caller must limit the size of the input before the parser reads it.
* ZIP archives (frequent for Swiss e-banking exports) have limits for the number
  of files, the size of each file and the total uncompressed size.
* Error messages do not include values from the file.
"""

from __future__ import annotations

import hashlib
import io
import pyexpat
import re
import zipfile
from collections.abc import Iterator
from dataclasses import dataclass, field
from datetime import date, datetime
from decimal import Decimal, InvalidOperation
from xml.etree.ElementTree import Element, TreeBuilder

import defusedxml.ElementTree as SafeET
from defusedxml import DefusedXmlException

from feljosit.iban import is_valid_iban, normalise_iban
from feljosit.money import MoneyError, to_minor, validate_currency

_NAMESPACE_RE = re.compile(r"^urn:iso:std:iso:20022:tech:xsd:camt\.053\.001\.(\d{2})$")
SUPPORTED_VERSIONS = range(2, 14)

MAX_ZIP_MEMBERS = 50
MAX_MEMBER_BYTES = 10 * 1024 * 1024
MAX_TOTAL_UNCOMPRESSED_BYTES = 25 * 1024 * 1024

#: Limits for one XML document. A camt.053 entry has approximately 20 to 80 elements
#: and a nesting depth of less than 15. Each attribute uses ``Ccy`` or a namespace.
MAX_ELEMENTS = 1_000_000
MAX_DEPTH = 64
MAX_ATTRIBUTES = 16

#: The first libexpat release that has the fixes for CVE-2024-8176 and CVE-2025-59375.
MIN_EXPAT_VERSION = (2, 7, 2)

#: ISO 20022 ``ActiveOrHistoricCurrencyAndAmount``: decimal, no sign, no exponent.
#: The application limits the integer part to 12 digits (as for typed amounts).
_AMOUNT_RE = re.compile(r"^\d{1,12}(\.\d{1,5})?$")


class Camt053Error(ValueError):
    """The file is not a valid or supported camt.053 statement."""


def expat_is_patched() -> bool:
    """Return true if the libexpat of this Python has the known security fixes."""
    return pyexpat.version_info >= MIN_EXPAT_VERSION


class _LimitedTreeBuilder(TreeBuilder):
    """A tree builder that refuses documents that are too large or too deep."""

    def __init__(self) -> None:
        super().__init__()
        self._elements = 0
        self._depth = 0

    def start(self, tag: str, attrs: dict[str, str]) -> Element:
        self._elements += 1
        self._depth += 1
        if self._elements > MAX_ELEMENTS:
            raise Camt053Error("The XML document contains too many elements")
        if self._depth > MAX_DEPTH:
            raise Camt053Error("The XML elements are nested too deeply")
        if len(attrs) > MAX_ATTRIBUTES:
            raise Camt053Error("An XML element has too many attributes")
        return super().start(tag, attrs)

    def end(self, tag: str) -> Element:
        self._depth -= 1
        return super().end(tag)


@dataclass(frozen=True)
class Balance:
    type_code: str
    amount_minor: int
    currency: str
    date: date


@dataclass(frozen=True)
class StatementTransaction:
    """One booked movement. A batch entry produces one item for each transaction detail."""

    fingerprint: str
    booking_date: date
    value_date: date | None
    amount_minor: int
    currency: str
    counterparty: str
    counterparty_iban: str | None
    description: str
    reference: str | None
    bank_reference: str | None
    reversal: bool


@dataclass(frozen=True)
class Statement:
    message_id: str
    version: int
    statement_id: str
    account_iban: str | None
    account_other_id: str | None
    currency: str | None
    balances: tuple[Balance, ...]
    transactions: tuple[StatementTransaction, ...]
    skipped_entries: int = 0

    def balance(self, *codes: str) -> Balance | None:
        """Return the first balance that has one of the type codes, for example ``CLBD``."""
        for code in codes:
            for bal in self.balances:
                if bal.type_code == code:
                    return bal
        return None

    @property
    def opening_balance(self) -> Balance | None:
        return self.balance("OPBD", "PRCD")

    @property
    def closing_balance(self) -> Balance | None:
        return self.balance("CLBD")


@dataclass
class _Ctx:
    """Parser state for one document."""

    message_id: str
    seen: dict[str, int] = field(default_factory=dict)


def _local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _strip_namespaces(root: Element) -> None:
    for el in root.iter():
        el.tag = _local(el.tag)


def _text(el: Element | None, path: str) -> str | None:
    if el is None:
        return None
    found = el.find(path)
    if found is None or found.text is None:
        return None
    value = " ".join(found.text.split())
    return value or None


def _parse_date(el: Element | None, path: str) -> date | None:
    """Read ``<Dt>`` or ``<DtTm>`` below ``path``."""
    if el is None:
        return None
    target = el.find(path)
    if target is None:
        return None
    try:
        raw = _text(target, "Dt")
        if raw:
            return date.fromisoformat(raw[:10])
        raw = _text(target, "DtTm")
        if raw:
            return datetime.fromisoformat(raw[:64]).date()
    except ValueError as exc:
        raise Camt053Error(f"A date in {path} is not valid") from exc
    return None


def _amount(el: Element | None, path: str = "Amt") -> tuple[int, str] | None:
    if el is None:
        return None
    amt = el.find(path)
    if amt is None or amt.text is None:
        return None
    currency = amt.get("Ccy")
    if currency is None:
        raise Camt053Error("Amount has no currency attribute (Ccy)")
    raw = amt.text.strip()
    # Check the syntax before Decimal() reads the value. This refuses negative
    # values, exponents ("1E999999"), NaN, infinity and very large numbers.
    if not _AMOUNT_RE.fullmatch(raw):
        raise Camt053Error("An amount is not valid")
    try:
        currency = validate_currency(currency)
        return to_minor(Decimal(raw), currency), currency
    except (InvalidOperation, MoneyError) as exc:
        raise Camt053Error("An amount or a currency is not valid") from exc


def _sign(indicator: str | None) -> int:
    """Return +1 for a credit and -1 for a debit.

    For a reversal (``RvslInd``), the indicator already shows the direction of the
    reversal booking. Thus no special processing is necessary.
    """
    if indicator not in {"CRDT", "DBIT"}:
        raise Camt053Error("A credit/debit indicator is not valid")
    return 1 if indicator == "CRDT" else -1


def _status(entry: Element) -> str | None:
    # Versions 02 to 06: <Sts>BOOK</Sts>. Versions 07 and later: <Sts><Cd>BOOK</Cd></Sts>.
    sts = entry.find("Sts")
    if sts is None:
        return None
    code = _text(sts, "Cd") or _text(sts, "Prtry")
    if code:
        return code
    return " ".join((sts.text or "").split()) or None


def _party_name(details: Element, role: str) -> str | None:
    # Versions 02 to 07: RltdPties/Dbtr/Nm. Version 08 and later: RltdPties/Dbtr/Pty/Nm.
    return _text(details, f"RltdPties/{role}/Pty/Nm") or _text(details, f"RltdPties/{role}/Nm")


def _party_iban(details: Element, role: str) -> str | None:
    raw = _text(details, f"RltdPties/{role}Acct/Id/IBAN")
    if raw and is_valid_iban(raw):
        return normalise_iban(raw)
    return None


def _description(details: Element | None, entry: Element) -> str:
    parts: list[str] = []
    if details is not None:
        parts.extend(" ".join(u.text.split()) for u in details.findall("RmtInf/Ustrd") if u.text)
        addtl = _text(details, "AddtlTxInf")
        if addtl:
            parts.append(addtl)
    entry_info = _text(entry, "AddtlNtryInf")
    if entry_info and entry_info not in parts:
        parts.append(entry_info)
    return " | ".join(parts)[:2000]


def _structured_reference(details: Element | None) -> str | None:
    if details is None:
        return None
    return _text(details, "RmtInf/Strd/CdtrRefInf/Ref")


def _fingerprint(ctx: _Ctx, *parts: object) -> str:
    base = "\x1f".join("" if p is None else str(p) for p in parts)
    occurrence = ctx.seen.get(base, 0)
    ctx.seen[base] = occurrence + 1
    digest = hashlib.sha256(f"{base}\x1f{occurrence}".encode()).hexdigest()
    return f"camt:{digest[:64]}"


def _entry_transactions(
    ctx: _Ctx, entry: Element, account_key: str
) -> Iterator[StatementTransaction]:
    reversal = (_text(entry, "RvslInd") or "").lower() == "true"
    entry_amount = _amount(entry)
    if entry_amount is None:
        raise Camt053Error("Entry has no amount")
    sign = _sign(_text(entry, "CdtDbtInd"))
    booking = _parse_date(entry, "BookgDt") or _parse_date(entry, "ValDt")
    if booking is None:
        raise Camt053Error("Entry has no booking date")
    value_date = _parse_date(entry, "ValDt")
    entry_ref = _text(entry, "AcctSvcrRef") or _text(entry, "NtryRef")

    details_list = entry.findall("NtryDtls/TxDtls")
    # Only split a batch entry if each detail has its own amount.
    split = len(details_list) > 1 and all(
        d.find("Amt") is not None or d.find("AmtDtls/TxAmt/Amt") is not None for d in details_list
    )
    items: list[tuple[Element | None, int, str, int]] = []
    if split:
        for d in details_list:
            amt = _amount(d) or _amount(d, "AmtDtls/TxAmt/Amt")
            if amt is None:  # pragma: no cover - excluded by the check above
                raise Camt053Error("Transaction detail has no amount")
            d_ind = _text(d, "CdtDbtInd")
            d_sign = _sign(d_ind) if d_ind else sign
            # TxAmt can be in the instructed currency. Use the entry currency only.
            if amt[1] != entry_amount[1]:
                split = False
                break
            items.append((d, amt[0], amt[1], d_sign))
    if not split:
        first = details_list[0] if details_list else None
        items = [(first, entry_amount[0], entry_amount[1], sign)]

    for index, (details, amount, currency, item_sign) in enumerate(items):
        role = "Cdtr" if item_sign < 0 else "Dbtr"
        counterparty = (details is not None and _party_name(details, role)) or ""
        counterparty_iban = _party_iban(details, role) if details is not None else None
        tx_ref = _text(details, "Refs/AcctSvcrRef") if details is not None else None
        bank_reference = tx_ref or entry_ref
        description = _description(details, entry)
        if not counterparty:
            counterparty = (_text(entry, "AddtlNtryInf") or "")[:140]
        fingerprint = _fingerprint(
            ctx,
            account_key,
            booking.isoformat(),
            item_sign * amount,
            currency,
            entry_ref,
            tx_ref,
            index if entry_ref or tx_ref else None,
            None if entry_ref or tx_ref else counterparty,
            None if entry_ref or tx_ref else description,
        )
        yield StatementTransaction(
            fingerprint=fingerprint,
            booking_date=booking,
            value_date=value_date,
            amount_minor=item_sign * amount,
            currency=currency,
            counterparty=counterparty[:140],
            counterparty_iban=counterparty_iban,
            description=description,
            reference=_structured_reference(details),
            bank_reference=bank_reference[:35] if bank_reference else None,
            reversal=reversal,
        )


def _parse_balance(bal: Element) -> Balance | None:
    code = _text(bal, "Tp/CdOrPrtry/Cd") or _text(bal, "Tp/CdOrPrtry/Prtry")
    amount = _amount(bal)
    bal_date = _parse_date(bal, "Dt")
    if code is None or amount is None or bal_date is None:
        return None
    sign = _sign(_text(bal, "CdtDbtInd"))
    return Balance(code, sign * amount[0], amount[1], bal_date)


def _parse_statement(ctx: _Ctx, version: int, stmt: Element) -> Statement:
    statement_id = _text(stmt, "Id") or ""
    raw_iban = _text(stmt, "Acct/Id/IBAN")
    iban = normalise_iban(raw_iban) if raw_iban else None
    if iban is not None and not is_valid_iban(iban):
        raise Camt053Error("The statement account IBAN is not valid")
    other_id = _text(stmt, "Acct/Id/Othr/Id")
    currency = _text(stmt, "Acct/Ccy")
    balances = tuple(b for b in (_parse_balance(el) for el in stmt.findall("Bal")) if b)

    transactions: list[StatementTransaction] = []
    skipped = 0
    account_key = iban or other_id or ""
    for entry in stmt.findall("Ntry"):
        status = _status(entry)
        if status is not None and status != "BOOK":
            skipped += 1
            continue
        transactions.extend(_entry_transactions(ctx, entry, account_key))
    return Statement(
        message_id=ctx.message_id,
        version=version,
        statement_id=statement_id,
        account_iban=iban,
        account_other_id=other_id,
        currency=currency,
        balances=balances,
        transactions=tuple(transactions),
        skipped_entries=skipped,
    )


def parse_xml(data: bytes) -> list[Statement]:
    """Parse one camt.053 XML document. A document can contain more than one statement."""
    parser = SafeET.DefusedXMLParser(
        target=_LimitedTreeBuilder(),
        forbid_dtd=True,
        forbid_entities=True,
        forbid_external=True,
    )
    try:
        parser.feed(data)
        root = parser.close()
    except DefusedXmlException as exc:
        raise Camt053Error("The XML file contains forbidden constructs (DTD or entities)") from exc
    except SafeET.ParseError as exc:
        raise Camt053Error("The file is not well-formed XML") from exc

    if not root.tag.startswith("{"):
        raise Camt053Error("The XML document has no ISO 20022 namespace")
    namespace = root.tag[1:].split("}", 1)[0]
    match = _NAMESPACE_RE.fullmatch(namespace)
    if match is None or _local(root.tag) != "Document":
        raise Camt053Error("The file is not a camt.053 document")
    version = int(match.group(1))
    if version not in SUPPORTED_VERSIONS:
        raise Camt053Error(f"camt.053.001.{version:02d} is not supported")

    _strip_namespaces(root)
    body = root.find("BkToCstmrStmt")
    if body is None:
        raise Camt053Error("The document has no BkToCstmrStmt element")
    ctx = _Ctx(message_id=_text(body, "GrpHdr/MsgId") or "")
    try:
        statements = [_parse_statement(ctx, version, stmt) for stmt in body.findall("Stmt")]
    except Camt053Error:
        raise
    except (ValueError, ArithmeticError) as exc:
        # Safety net: an unexpected value must not cause an internal server error.
        raise Camt053Error("The statement contains a value that is not valid") from exc
    if not statements:
        raise Camt053Error("The document contains no statement")
    return statements


def _iter_zip(data: bytes) -> Iterator[tuple[str, bytes]]:
    try:
        archive = zipfile.ZipFile(io.BytesIO(data))
    except zipfile.BadZipFile as exc:
        raise Camt053Error("The ZIP file is not valid") from exc
    with archive:
        members = [m for m in archive.infolist() if not m.is_dir()]
        if len(members) > MAX_ZIP_MEMBERS:
            raise Camt053Error("The ZIP file contains too many files")
        total = 0
        for member in members:
            if not member.filename.lower().endswith(".xml"):
                continue
            if member.file_size > MAX_MEMBER_BYTES:
                raise Camt053Error("A file in the ZIP archive is too large")
            # Do not trust the declared size. Read with a limit.
            with archive.open(member) as handle:
                content = handle.read(MAX_MEMBER_BYTES + 1)
            if len(content) > MAX_MEMBER_BYTES:
                raise Camt053Error("A file in the ZIP archive is too large")
            total += len(content)
            if total > MAX_TOTAL_UNCOMPRESSED_BYTES:
                raise Camt053Error("The ZIP archive is too large when uncompressed")
            yield member.filename, content


def parse_file(data: bytes) -> list[Statement]:
    """Parse an XML document or a ZIP archive that contains XML documents."""
    if data[:4] == b"PK\x03\x04":
        statements: list[Statement] = []
        for _name, content in _iter_zip(data):
            statements.extend(parse_xml(content))
        if not statements:
            raise Camt053Error("The ZIP archive contains no camt.053 XML file")
        return statements
    return parse_xml(data)
