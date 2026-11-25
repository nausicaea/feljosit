import io
import zipfile
from datetime import date

import pytest

from feljosit import camt053
from feljosit.camt053 import Camt053Error, parse_file, parse_xml
from tests.conftest import fixture_bytes

NS = "urn:iso:std:iso:20022:tech:xsd:camt.053.001.{v}"


def _doc(entries: str, version: str = "04", balances: str = "") -> bytes:
    return f"""<?xml version="1.0"?>
<Document xmlns="{NS.format(v=version)}"><BkToCstmrStmt>
<GrpHdr><MsgId>M1</MsgId></GrpHdr>
<Stmt><Id>S1</Id><Acct><Id><IBAN>CH9300762011623852957</IBAN></Id></Acct>{balances}{entries}</Stmt>
</BkToCstmrStmt></Document>""".encode()


def _entry(amount: str = "10.00", ind: str = "DBIT", extra: str = "", details: str = "") -> str:
    return (
        f'<Ntry><Amt Ccy="CHF">{amount}</Amt><CdtDbtInd>{ind}</CdtDbtInd><Sts>BOOK</Sts>'
        f"<BookgDt><Dt>2026-01-02</Dt></BookgDt>{extra}{details}</Ntry>"
    )


def test_parse_swiss_v04_statement() -> None:
    [stmt] = parse_xml(fixture_bytes("camt053_v04_private.xml"))
    assert stmt.version == 4
    assert stmt.message_id == "MSG-2026-10-001"
    assert stmt.account_iban == "CH8900235000012345678"
    assert stmt.currency == "CHF"
    assert stmt.opening_balance is not None and stmt.opening_balance.amount_minor == 100000
    assert stmt.closing_balance is not None and stmt.closing_balance.amount_minor == 425005
    assert stmt.skipped_entries == 1  # The pending entry.
    amounts = [t.amount_minor for t in stmt.transactions]
    # The batch entry (150.00) is split in two transactions (100.00 and 50.00).
    assert amounts == [-210000, -41235, -8760, -10000, -5000, -50000, 650000]
    assert sum(amounts) + 100000 == 425005


def test_counterparty_and_references() -> None:
    [stmt] = parse_xml(fixture_bytes("camt053_v04_private.xml"))
    rent, health, card, phone, serafe, transfer, salary = stmt.transactions
    assert rent.counterparty == "Immo Beispiel GmbH"
    assert rent.counterparty_iban == "CH3880808001234567890"
    assert rent.description == "Miete Oktober 2026"
    assert rent.booking_date == date(2026, 10, 1)
    assert health.reference == "210000000003139471430009017"
    assert card.counterparty == "Debit card payment MIGROS ZUERICH"
    assert card.value_date == date(2026, 10, 9)
    assert phone.counterparty == "Swisscom (Schweiz) AG"
    assert phone.bank_reference == "20261015-BATCH-9-1"
    assert serafe.counterparty == "Serafe AG"
    # Spaces in the IBAN are removed.
    assert transfer.counterparty_iban == "CH0300700110000123456"
    # For a credit, the counterparty is the debtor.
    assert salary.counterparty == "Muster AG"
    assert salary.counterparty_iban == "CH9300762011623852957"


def test_parse_v08_party_structure_and_datetime() -> None:
    [stmt] = parse_xml(fixture_bytes("camt053_v08_savings.xml"))
    assert stmt.version == 8
    first, interest = stmt.transactions
    assert first.counterparty == "Anna Muster"
    assert first.booking_date == date(2026, 10, 20)
    assert interest.counterparty == "Zinsgutschrift"
    assert interest.amount_minor == 250


def test_fingerprints_are_stable_and_unique() -> None:
    a = parse_xml(fixture_bytes("camt053_v04_private.xml"))[0].transactions
    b = parse_xml(fixture_bytes("camt053_v04_private.xml"))[0].transactions
    assert [t.fingerprint for t in a] == [t.fingerprint for t in b]
    assert len({t.fingerprint for t in a}) == len(a)


def test_identical_entries_without_reference_get_different_fingerprints() -> None:
    entry = _entry(extra="<AddtlNtryInf>Coffee</AddtlNtryInf>")
    [stmt] = parse_xml(_doc(entry + entry))
    first, second = stmt.transactions
    assert first.fingerprint != second.fingerprint


def test_batch_without_detail_amounts_is_not_split() -> None:
    details = (
        "<NtryDtls><TxDtls><RltdPties><Cdtr><Nm>A</Nm></Cdtr></RltdPties></TxDtls>"
        "<TxDtls><RltdPties><Cdtr><Nm>B</Nm></Cdtr></RltdPties></TxDtls></NtryDtls>"
    )
    [stmt] = parse_xml(_doc(_entry("30.00", details=details)))
    assert [t.amount_minor for t in stmt.transactions] == [-3000]


def test_batch_with_foreign_currency_detail_is_not_split() -> None:
    details = (
        '<NtryDtls><TxDtls><AmtDtls><TxAmt><Amt Ccy="EUR">10.00</Amt></TxAmt></AmtDtls></TxDtls>'
        '<TxDtls><AmtDtls><TxAmt><Amt Ccy="EUR">5.00</Amt></TxAmt></AmtDtls></TxDtls></NtryDtls>'
    )
    [stmt] = parse_xml(_doc(_entry("14.20", details=details)))
    assert [t.amount_minor for t in stmt.transactions] == [-1420]


def test_v07_status_code_element() -> None:
    entry = (
        '<Ntry><Amt Ccy="CHF">1.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Sts><Cd>PDNG</Cd></Sts>'
        "<BookgDt><Dt>2026-01-02</Dt></BookgDt></Ntry>"
    )
    [stmt] = parse_xml(_doc(entry + _entry(), version="07"))
    assert stmt.skipped_entries == 1
    assert len(stmt.transactions) == 1


def test_reversal_flag() -> None:
    [stmt] = parse_xml(_doc(_entry(ind="CRDT", extra="<RvslInd>true</RvslInd>")))
    assert stmt.transactions[0].reversal
    assert stmt.transactions[0].amount_minor == 1000


@pytest.mark.parametrize(
    "data",
    [
        b"not xml",
        b"<Document/>",
        b'<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.04"/>',
        _doc("", version="99"),
        b'<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04">'
        b"<BkToCstmrStmt/></Document>",
    ],
)
def test_rejects_wrong_documents(data: bytes) -> None:
    with pytest.raises(Camt053Error):
        parse_xml(data)


@pytest.mark.parametrize(
    ("amount", "ind"),
    [("-5.00", "DBIT"), ("1.234", "DBIT"), ("abc", "DBIT"), ("5.00", "XXXX")],
)
def test_rejects_invalid_amounts(amount: str, ind: str) -> None:
    with pytest.raises(Camt053Error):
        parse_xml(_doc(_entry(amount, ind)))


def test_rejects_invalid_statement_iban() -> None:
    data = _doc(_entry()).replace(b"CH9300762011623852957", b"CH9300762011623852958")
    with pytest.raises(Camt053Error, match="IBAN"):
        parse_xml(data)


def test_rejects_external_entities_xxe() -> None:
    data = b"""<?xml version="1.0"?>
<!DOCTYPE d [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04"><BkToCstmrStmt>
<GrpHdr><MsgId>&xxe;</MsgId></GrpHdr></BkToCstmrStmt></Document>"""
    with pytest.raises(Camt053Error, match="forbidden"):
        parse_xml(data)


def test_rejects_billion_laughs() -> None:
    data = b"""<?xml version="1.0"?>
<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04">&lol2;</Document>"""
    with pytest.raises(Camt053Error, match="forbidden"):
        parse_xml(data)


def _zip(files: dict[str, bytes]) -> bytes:
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, content in files.items():
            archive.writestr(name, content)
    return buffer.getvalue()


def test_zip_with_two_statements() -> None:
    data = _zip(
        {
            "a.xml": fixture_bytes("camt053_v04_private.xml"),
            "b.xml": fixture_bytes("camt053_v08_savings.xml"),
            "readme.txt": b"ignored",
        }
    )
    statements = parse_file(data)
    assert [s.account_iban for s in statements] == [
        "CH8900235000012345678",
        "CH0300700110000123456",
    ]


def test_zip_limits(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(camt053, "MAX_MEMBER_BYTES", 1000)
    with pytest.raises(Camt053Error, match="too large"):
        parse_file(_zip({"big.xml": b"<a>" + b" " * 5000 + b"</a>"}))
    monkeypatch.setattr(camt053, "MAX_ZIP_MEMBERS", 2)
    with pytest.raises(Camt053Error, match="too many"):
        parse_file(_zip({f"{i}.xml": b"<a/>" for i in range(3)}))


def test_zip_without_xml() -> None:
    with pytest.raises(Camt053Error, match=r"no camt\.053"):
        parse_file(_zip({"readme.txt": b"x"}))


def test_bad_zip() -> None:
    with pytest.raises(Camt053Error, match="ZIP"):
        parse_file(b"PK\x03\x04garbage")
