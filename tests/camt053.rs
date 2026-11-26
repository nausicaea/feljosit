//! Tests of the camt.053 parser (port of the Python tests).

mod common;

use chrono::NaiveDate;
use common::{fixture, zip_files};
use feljosit::camt053::{self, Limits, parse_file, parse_file_with, parse_xml, parse_xml_with};

fn doc_version(entries: &str, version: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.{version}"><BkToCstmrStmt>
<GrpHdr><MsgId>M1</MsgId></GrpHdr>
<Stmt><Id>S1</Id><Acct><Id><IBAN>CH9300762011623852957</IBAN></Id></Acct>{entries}</Stmt>
</BkToCstmrStmt></Document>"#
    )
    .into_bytes()
}

fn doc(entries: &str) -> Vec<u8> {
    doc_version(entries, "04")
}

fn entry(amount: &str, ind: &str, extra: &str, details: &str) -> String {
    format!(
        r#"<Ntry><Amt Ccy="CHF">{amount}</Amt><CdtDbtInd>{ind}</CdtDbtInd><Sts>BOOK</Sts><BookgDt><Dt>2026-01-02</Dt></BookgDt>{extra}{details}</Ntry>"#
    )
}

fn simple_entry() -> String {
    entry("10.00", "DBIT", "", "")
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn error_of(data: &[u8]) -> String {
    parse_xml(data).unwrap_err().0
}

#[test]
fn parse_swiss_v04_statement() {
    let statements = parse_xml(&fixture("camt053_v04_private.xml")).unwrap();
    let [stmt] = statements.as_slice() else { panic!() };
    assert_eq!(stmt.version, 4);
    assert_eq!(stmt.message_id, "MSG-2026-10-001");
    assert_eq!(stmt.account_iban.as_deref(), Some("CH8900235000012345678"));
    assert_eq!(stmt.currency.as_deref(), Some("CHF"));
    assert_eq!(stmt.opening_balance().unwrap().amount_minor, 100000);
    assert_eq!(stmt.closing_balance().unwrap().amount_minor, 425005);
    assert_eq!(stmt.skipped_entries, 1); // The pending entry.
    let amounts: Vec<i64> = stmt.transactions.iter().map(|t| t.amount_minor).collect();
    // The batch entry (150.00) is split in two transactions (100.00 and 50.00).
    assert_eq!(amounts, vec![-210000, -41235, -8760, -10000, -5000, -50000, 650000]);
    assert_eq!(amounts.iter().sum::<i64>() + 100000, 425005);
}

#[test]
fn counterparty_and_references() {
    let stmt = parse_xml(&fixture("camt053_v04_private.xml")).unwrap().remove(0);
    let [rent, health, card, phone, serafe, transfer, salary] = stmt.transactions.as_slice() else {
        panic!()
    };
    assert_eq!(rent.counterparty, "Immo Beispiel GmbH");
    assert_eq!(rent.counterparty_iban.as_deref(), Some("CH3880808001234567890"));
    assert_eq!(rent.description, "Miete Oktober 2026");
    assert_eq!(rent.booking_date, date(2026, 10, 1));
    assert_eq!(health.reference.as_deref(), Some("210000000003139471430009017"));
    assert_eq!(card.counterparty, "Debit card payment MIGROS ZUERICH");
    assert_eq!(card.value_date, Some(date(2026, 10, 9)));
    assert_eq!(phone.counterparty, "Swisscom (Schweiz) AG");
    assert_eq!(phone.bank_reference.as_deref(), Some("20261015-BATCH-9-1"));
    assert_eq!(serafe.counterparty, "Serafe AG");
    // Spaces in the IBAN are removed.
    assert_eq!(transfer.counterparty_iban.as_deref(), Some("CH0300700110000123456"));
    // For a credit, the counterparty is the debtor.
    assert_eq!(salary.counterparty, "Muster AG");
    assert_eq!(salary.counterparty_iban.as_deref(), Some("CH9300762011623852957"));
}

#[test]
fn parse_v08_party_structure_and_datetime() {
    let stmt = parse_xml(&fixture("camt053_v08_savings.xml")).unwrap().remove(0);
    assert_eq!(stmt.version, 8);
    let [first, interest] = stmt.transactions.as_slice() else { panic!() };
    assert_eq!(first.counterparty, "Anna Muster");
    assert_eq!(first.booking_date, date(2026, 10, 20));
    assert_eq!(interest.counterparty, "Zinsgutschrift");
    assert_eq!(interest.amount_minor, 250);
}

#[test]
fn fingerprints_are_stable_and_unique() {
    let a = parse_xml(&fixture("camt053_v04_private.xml")).unwrap().remove(0).transactions;
    let b = parse_xml(&fixture("camt053_v04_private.xml")).unwrap().remove(0).transactions;
    let fa: Vec<_> = a.iter().map(|t| t.fingerprint.clone()).collect();
    let fb: Vec<_> = b.iter().map(|t| t.fingerprint.clone()).collect();
    assert_eq!(fa, fb);
    let unique: std::collections::HashSet<_> = fa.iter().collect();
    assert_eq!(unique.len(), a.len());
}

/// The fingerprints must be the same as in the Python version. Else an import of an
/// old statement after the update adds the transactions again.
#[test]
fn fingerprints_are_the_same_as_in_the_python_version() {
    let stmt = parse_xml(&fixture("camt053_v04_private.xml")).unwrap().remove(0);
    let fingerprints: Vec<&str> = stmt.transactions.iter().map(|t| t.fingerprint.as_str()).collect();
    assert_eq!(
        fingerprints,
        [
            "camt:e981c135cc9b8627903487d8f37b128dd5515055adbd8291a4dd4d62fa67d958",
            "camt:1048fe2379b86cb7ad0690f801bbb20f513b6e9d3e5f6ec81685cec336d98f97",
            "camt:0088e8ec95e4ce81fd7ddf304a4ba925535fcd09357378de91170d87d4d26b0c",
            "camt:3b2f8804ba10c72732745fe346200cdece33d3919e769e00a313299b1ae79580",
            "camt:d780528b85194a529ed951c10678bcaf70bbed50c0fba69034b128b9083d9905",
            "camt:166618b16d58cb0f37aa41eb0a4b58d0e83b58bbacd429ccf1cddf24d0bbcd3b",
            "camt:76ef8092779418e55f65d3b1fe5e7fd370ed42a5850a21f6f7926ce9b623dfdc",
        ]
    );
}

#[test]
fn identical_entries_without_reference_get_different_fingerprints() {
    let e = entry("10.00", "DBIT", "<AddtlNtryInf>Coffee</AddtlNtryInf>", "");
    let stmt = parse_xml(&doc(&format!("{e}{e}"))).unwrap().remove(0);
    assert_ne!(stmt.transactions[0].fingerprint, stmt.transactions[1].fingerprint);
}

#[test]
fn batch_without_detail_amounts_is_not_split() {
    let details = "<NtryDtls><TxDtls><RltdPties><Cdtr><Nm>A</Nm></Cdtr></RltdPties></TxDtls>\
        <TxDtls><RltdPties><Cdtr><Nm>B</Nm></Cdtr></RltdPties></TxDtls></NtryDtls>";
    let stmt = parse_xml(&doc(&entry("30.00", "DBIT", "", details))).unwrap().remove(0);
    let amounts: Vec<i64> = stmt.transactions.iter().map(|t| t.amount_minor).collect();
    assert_eq!(amounts, vec![-3000]);
}

#[test]
fn batch_with_foreign_currency_detail_is_not_split() {
    let details = r#"<NtryDtls><TxDtls><AmtDtls><TxAmt><Amt Ccy="EUR">10.00</Amt></TxAmt></AmtDtls></TxDtls><TxDtls><AmtDtls><TxAmt><Amt Ccy="EUR">5.00</Amt></TxAmt></AmtDtls></TxDtls></NtryDtls>"#;
    let stmt = parse_xml(&doc(&entry("14.20", "DBIT", "", details))).unwrap().remove(0);
    let amounts: Vec<i64> = stmt.transactions.iter().map(|t| t.amount_minor).collect();
    assert_eq!(amounts, vec![-1420]);
}

#[test]
fn v07_status_code_element() {
    let pending = r#"<Ntry><Amt Ccy="CHF">1.00</Amt><CdtDbtInd>CRDT</CdtDbtInd><Sts><Cd>PDNG</Cd></Sts><BookgDt><Dt>2026-01-02</Dt></BookgDt></Ntry>"#;
    let stmt = parse_xml(&doc_version(&format!("{pending}{}", simple_entry()), "07")).unwrap().remove(0);
    assert_eq!(stmt.skipped_entries, 1);
    assert_eq!(stmt.transactions.len(), 1);
}

#[test]
fn reversal_flag() {
    let stmt = parse_xml(&doc(&entry("10.00", "CRDT", "<RvslInd>true</RvslInd>", ""))).unwrap().remove(0);
    assert!(stmt.transactions[0].reversal);
    assert_eq!(stmt.transactions[0].amount_minor, 1000);
}

#[test]
fn rejects_wrong_documents() {
    let cases: Vec<Vec<u8>> = vec![
        b"not xml".to_vec(),
        b"<Document/>".to_vec(),
        br#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.052.001.04"/>"#.to_vec(),
        doc_version("", "99"),
        br#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04"><BkToCstmrStmt/></Document>"#
            .to_vec(),
    ];
    for data in cases {
        assert!(parse_xml(&data).is_err(), "{}", String::from_utf8_lossy(&data));
    }
    assert_eq!(error_of(b"<Document/>"), "The XML document has no ISO 20022 namespace");
    assert_eq!(error_of(&doc_version("", "99")), "camt.053.001.99 is not supported");
}

#[test]
fn rejects_invalid_amounts() {
    for (amount, ind) in [("-5.00", "DBIT"), ("1.234", "DBIT"), ("abc", "DBIT"), ("5.00", "XXXX")] {
        assert!(parse_xml(&doc(&entry(amount, ind, "", ""))).is_err(), "{amount} {ind}");
    }
}

#[test]
fn rejects_invalid_statement_iban() {
    let data = String::from_utf8(doc(&simple_entry()))
        .unwrap()
        .replace("CH9300762011623852957", "CH9300762011623852958");
    assert!(error_of(data.as_bytes()).contains("IBAN"));
}

#[test]
fn rejects_external_entities_xxe() {
    let data = br#"<?xml version="1.0"?>
<!DOCTYPE d [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04"><BkToCstmrStmt>
<GrpHdr><MsgId>&xxe;</MsgId></GrpHdr></BkToCstmrStmt></Document>"#;
    assert!(error_of(data).contains("forbidden"));
}

#[test]
fn rejects_billion_laughs() {
    let data = br#"<?xml version="1.0"?>
<!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;">]>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04">&lol2;</Document>"#;
    assert!(error_of(data).contains("forbidden"));
}

#[test]
fn rejects_undefined_entities() {
    let data = br#"<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.04">&undefined;</Document>"#;
    assert!(parse_xml(data).is_err());
}

#[test]
fn zip_with_two_statements() {
    let data = zip_files(&[
        ("a.xml", fixture("camt053_v04_private.xml")),
        ("b.xml", fixture("camt053_v08_savings.xml")),
        ("readme.txt", b"ignored".to_vec()),
    ]);
    let ibans: Vec<_> = parse_file(&data).unwrap().into_iter().map(|s| s.account_iban.unwrap()).collect();
    assert_eq!(ibans, vec!["CH8900235000012345678", "CH0300700110000123456"]);
}

#[test]
fn zip_limits() {
    let limits = Limits { max_member_bytes: 1000, ..Limits::default() };
    let mut big = b"<a>".to_vec();
    big.extend(vec![b' '; 5000]);
    big.extend(b"</a>");
    let error = parse_file_with(&zip_files(&[("big.xml", big)]), &limits).unwrap_err();
    assert!(error.0.contains("too large"), "{error}");
    let limits = Limits { max_zip_members: 2, ..Limits::default() };
    let files: Vec<(String, Vec<u8>)> = (0..3).map(|i| (format!("{i}.xml"), b"<a/>".to_vec())).collect();
    let refs: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, d)| (n.as_str(), d.clone())).collect();
    let error = parse_file_with(&zip_files(&refs), &limits).unwrap_err();
    assert!(error.0.contains("too many"), "{error}");
}

#[test]
fn zip_without_xml() {
    let error = parse_file(&zip_files(&[("readme.txt", b"x".to_vec())])).unwrap_err();
    assert!(error.0.contains("no camt.053"));
}

#[test]
fn bad_zip() {
    assert!(parse_file(b"PK\x03\x04garbage").unwrap_err().0.contains("ZIP"));
}

#[test]
fn rejects_amounts_with_wrong_syntax() {
    let long = "1".repeat(13);
    for amount in ["1E999999", "1e2", "NaN", "Infinity", "+5.00", " 5 . 0", long.as_str(), "0x10", "1.123456"]
    {
        let error = error_of(&doc(&entry(amount, "DBIT", "", "")));
        assert!(error.contains("amount"), "{amount}: {error}");
    }
}

#[test]
fn error_messages_do_not_include_file_values() {
    assert!(!error_of(&doc(&entry("SECRET-VALUE", "DBIT", "", ""))).contains("SECRET"));
    assert!(!error_of(&doc(&entry("10.00", "SECRET-IND", "", ""))).contains("SECRET"));
}

#[test]
fn rejects_invalid_dates() {
    for booking in ["<Dt>2026-13-01</Dt>", "<Dt>not-a-date</Dt>", "<DtTm>2026-01-01T99:00:00</DtTm>"] {
        let e = simple_entry().replace("<Dt>2026-01-02</Dt>", booking);
        let error = error_of(&doc(&e));
        assert!(error.contains("date"), "{booking}: {error}");
    }
}

#[test]
fn accepts_date_times() {
    let e = simple_entry().replace("<Dt>2026-01-02</Dt>", "<DtTm>2026-01-03T10:15:00.000+01:00</DtTm>");
    let stmt = parse_xml(&doc(&e)).unwrap().remove(0);
    assert_eq!(stmt.transactions[0].booking_date, date(2026, 1, 3));
}

#[test]
fn rejects_too_many_elements() {
    let limits = Limits { max_elements: 50, ..Limits::default() };
    let error = parse_xml_with(&doc(&"<X/>".repeat(100)), &limits).unwrap_err();
    assert!(error.0.contains("too many elements"), "{error}");
}

#[test]
fn rejects_deep_nesting() {
    let depth = camt053::MAX_DEPTH + 1;
    let data = doc(&format!("{}{}", "<X>".repeat(depth), "</X>".repeat(depth)));
    assert!(error_of(&data).contains("nested too deeply"));
}

#[test]
fn rejects_too_many_attributes() {
    let attrs: Vec<String> = (0..=camt053::MAX_ATTRIBUTES).map(|i| format!(r#"a{i}="x""#)).collect();
    let data = doc(&format!("<X {}/>", attrs.join(" ")));
    assert!(error_of(&data).contains("too many attributes"));
}

#[test]
fn limits_permit_a_large_real_statement() {
    let entries: String =
        (0..5000).map(|i| entry("10.00", "DBIT", &format!("<AcctSvcrRef>R{i}</AcctSvcrRef>"), "")).collect();
    let stmt = parse_xml(&doc(&entries)).unwrap().remove(0);
    assert_eq!(stmt.transactions.len(), 5000);
}

#[test]
fn rejects_dtd_without_entities() {
    assert!(error_of(b"<!DOCTYPE Document><Document/>").contains("forbidden"));
}

#[test]
fn xinclude_is_not_processed() {
    let data = doc(&format!(
        r#"<xi:include xmlns:xi="http://www.w3.org/2001/XInclude" href="file:///etc/passwd"/>{}"#,
        simple_entry()
    ));
    let stmt = parse_xml(&data).unwrap().remove(0);
    assert!(!format!("{stmt:?}").contains("root"));
}

#[test]
fn zip_total_uncompressed_limit() {
    let limits = Limits { max_total_uncompressed_bytes: 6000, ..Limits::default() };
    let mut member = doc(&simple_entry());
    member.extend(vec![b' '; 4000]);
    let error =
        parse_file_with(&zip_files(&[("a.xml", member.clone()), ("b.xml", member)]), &limits).unwrap_err();
    assert!(error.0.contains("uncompressed"), "{error}");
}

#[test]
fn zip_limits_are_not_larger_than_before() {
    // A small ZIP file can expand very much. Keep the limits small (ADR-0014).
    const { assert!(camt053::MAX_MEMBER_BYTES <= 10 * 1024 * 1024) };
    const { assert!(camt053::MAX_TOTAL_UNCOMPRESSED_BYTES <= 25 * 1024 * 1024) };
}

#[test]
fn many_attributes_are_refused_fast() {
    // The parser compares each attribute with the others. Without the pre-scan, this
    // document used the CPU for minutes.
    let mut attributes = String::new();
    let mut i = 0;
    while attributes.len() < 2 * 1024 * 1024 {
        attributes.push_str(&format!(r#"a{i}="" "#));
        i += 1;
    }
    let start = std::time::Instant::now();
    let error = error_of(&doc(&format!("<X {attributes}/>")));
    assert!(error.contains("too many attributes"), "{error}");
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn many_elements_are_refused_before_the_parse() {
    let limits = Limits { max_elements: 1000, ..Limits::default() };
    let error = parse_xml_with(&doc(&"<X>a</X>\n".repeat(2000)), &limits).unwrap_err();
    assert!(error.0.contains("too many elements"), "{error}");
    // Comments, CDATA and quoted ">" do not confuse the pre-scan.
    let entry = r#"<!-- <X> <Y> --><Ntry><Amt Ccy="CHF">10.00</Amt><CdtDbtInd>DBIT</CdtDbtInd><Sts>BOOK</Sts><BookgDt><Dt>2026-01-02</Dt></BookgDt><AddtlNtryInf><![CDATA[a <b> c]]></AddtlNtryInf></Ntry>"#;
    let stmt = parse_xml(&doc(entry)).unwrap().remove(0);
    assert_eq!(stmt.transactions[0].counterparty, "a <b> c");
}

#[test]
fn other_encodings() {
    let utf8 =
        String::from_utf8(doc(&entry("10.00", "DBIT", "<AddtlNtryInf>Zürich</AddtlNtryInf>", ""))).unwrap();
    let latin1_text =
        utf8.replacen(r#"<?xml version="1.0"?>"#, r#"<?xml version="1.0" encoding="ISO-8859-1"?>"#, 1);
    let latin1: Vec<u8> = latin1_text.chars().map(|c| c as u32 as u8).collect();
    assert_eq!(parse_xml(&latin1).unwrap()[0].transactions[0].counterparty, "Zürich");
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend(utf8.encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(parse_xml(&utf16).unwrap()[0].transactions[0].counterparty, "Zürich");
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend(utf8.as_bytes());
    assert!(parse_xml(&bom).is_ok());
    assert!(parse_xml(b"<Document>\xff</Document>").is_err());
}
