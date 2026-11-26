//! Parser for ISO 20022 camt.053 bank-to-customer statements.
//!
//! The parser supports the message versions camt.053.001.02 to camt.053.001.13. Swiss
//! banks use these versions as specified in the Swiss Payment Standards (SPS).
//!
//! Security (see ADR-0005 and ADR-0014):
//!
//! * The parser (roxmltree) refuses DTDs. Thus entity declarations and external
//!   references are not possible. This prevents XXE and "billion laughs" attacks.
//! * The parser limits the number of nodes. After the parse, the application limits
//!   the number of elements, the nesting depth and the number of attributes of each
//!   element. This limits the memory and the time that one document can use.
//! * Amounts must have the ISO 20022 decimal syntax without an exponent.
//! * The caller must limit the size of the input before the parser reads it.
//! * ZIP archives (frequent for Swiss e-banking exports) have limits for the number of
//!   files, the size of each file and the total uncompressed size.
//! * Error messages do not include values from the file.

use crate::dates::Date;
use std::collections::HashMap;
use std::fmt;
use std::io::{Cursor, Read};

use roxmltree::{Document, Node, ParsingOptions};
use sha2::{Digest, Sha256};

use crate::iban::{is_valid_iban, normalise_iban};
use crate::money::{Decimal, to_minor, validate_currency};

pub const SUPPORTED_VERSIONS: std::ops::RangeInclusive<u32> = 2..=13;

pub const MAX_ZIP_MEMBERS: usize = 50;
pub const MAX_MEMBER_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 25 * 1024 * 1024;

/// Limits for one XML document. A camt.053 entry has approximately 20 to 80 elements
/// and a nesting depth of less than 15. Each attribute uses `Ccy` or a namespace.
pub const MAX_ELEMENTS: usize = 1_000_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_ATTRIBUTES: usize = 16;

/// The limits of the parser. The tests use smaller limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_zip_members: usize,
    pub max_member_bytes: u64,
    pub max_total_uncompressed_bytes: u64,
    pub max_elements: usize,
    pub max_depth: usize,
    pub max_attributes: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_zip_members: MAX_ZIP_MEMBERS,
            max_member_bytes: MAX_MEMBER_BYTES,
            max_total_uncompressed_bytes: MAX_TOTAL_UNCOMPRESSED_BYTES,
            max_elements: MAX_ELEMENTS,
            max_depth: MAX_DEPTH,
            max_attributes: MAX_ATTRIBUTES,
        }
    }
}

/// The file is not a valid or supported camt.053 statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Camt053Error(pub String);

impl fmt::Display for Camt053Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Camt053Error {}

type Result<T> = std::result::Result<T, Camt053Error>;

fn err<T>(message: &str) -> Result<T> {
    Err(Camt053Error(message.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balance {
    pub type_code: String,
    pub amount_minor: i64,
    pub currency: String,
    pub date: Date,
}

/// One booked movement. A batch entry produces one item for each transaction detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementTransaction {
    pub fingerprint: String,
    pub booking_date: Date,
    pub value_date: Option<Date>,
    pub amount_minor: i64,
    pub currency: String,
    pub counterparty: String,
    pub counterparty_iban: Option<String>,
    pub description: String,
    pub reference: Option<String>,
    pub bank_reference: Option<String>,
    pub reversal: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Statement {
    pub message_id: String,
    pub version: u32,
    pub statement_id: String,
    pub account_iban: Option<String>,
    pub account_other_id: Option<String>,
    pub currency: Option<String>,
    pub balances: Vec<Balance>,
    pub transactions: Vec<StatementTransaction>,
    pub skipped_entries: usize,
    /// The name of the account (`Acct/Nm`), if the bank gives it.
    pub account_name: Option<String>,
    /// The name of the bank (`Acct/Svcr/FinInstnId/Nm`), if the bank gives it.
    pub servicer_name: Option<String>,
}

impl Statement {
    /// Return the first balance that has one of the type codes, for example `CLBD`.
    pub fn balance(&self, codes: &[&str]) -> Option<&Balance> {
        codes.iter().find_map(|code| self.balances.iter().find(|b| b.type_code == *code))
    }

    pub fn opening_balance(&self) -> Option<&Balance> {
        self.balance(&["OPBD", "PRCD"])
    }

    pub fn closing_balance(&self) -> Option<&Balance> {
        self.balance(&["CLBD"])
    }
}

/// Parser state for one document.
struct Ctx {
    message_id: String,
    seen: HashMap<String, usize>,
}

/// All elements that agree with a path such as `RltdPties/Cdtr/Nm`, in document order
/// (as `Element.findall` of Python).
fn find_all<'a, 'i>(el: Node<'a, 'i>, path: &str) -> Vec<Node<'a, 'i>> {
    let mut current = vec![el];
    for segment in path.split('/') {
        current = current
            .iter()
            .flat_map(|node| node.children().filter(|c| c.is_element() && c.tag_name().name() == segment))
            .collect();
    }
    current
}

fn find<'a, 'i>(el: Node<'a, 'i>, path: &str) -> Option<Node<'a, 'i>> {
    fn search<'a, 'i>(node: Node<'a, 'i>, segments: &[&str]) -> Option<Node<'a, 'i>> {
        let Some((first, rest)) = segments.split_first() else {
            return Some(node);
        };
        node.children()
            .filter(|c| c.is_element() && c.tag_name().name() == *first)
            .find_map(|c| search(c, rest))
    }
    let segments: Vec<&str> = path.split('/').collect();
    search(el, &segments)
}

/// The text directly in the element (before its first child element).
fn own_text<'a>(el: Node<'a, '_>) -> Option<&'a str> {
    el.first_child().filter(Node::is_text).and_then(|n| n.text())
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text(el: Option<Node<'_, '_>>, path: &str) -> Option<String> {
    let found = find(el?, path)?;
    let value = collapse(own_text(found)?);
    (!value.is_empty()).then_some(value)
}

fn iso_date(raw: &str) -> Option<Date> {
    let raw: String = raw.chars().take(10).collect();
    crate::dates::parse(&raw, "%Y-%m-%d").or_else(|| crate::dates::parse(&raw, "%Y%m%d"))
}

/// Check the part of an ISO 8601 date-time after the date, for example
/// `T10:15:00.000+01:00`.
fn valid_time_part(rest: &str) -> bool {
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix(['T', ' ']) else {
        return false;
    };
    let (time, zone) = match rest.find(['Z', '+', '-']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let (clock, fraction) = time.split_once(['.', ',']).unwrap_or((time, ""));
    let numbers: Vec<&str> = clock.split(':').collect();
    let two_digits = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit());
    let clock_ok = !numbers.is_empty()
        && numbers.len() <= 3
        && numbers.iter().all(|n| two_digits(n))
        && numbers[0].parse::<u32>().is_ok_and(|h| h <= 24)
        && numbers.get(1).is_none_or(|m| m.parse::<u32>().is_ok_and(|m| m < 60))
        && numbers.get(2).is_none_or(|s| s.parse::<u32>().is_ok_and(|s| s < 61));
    let fraction_ok =
        fraction.bytes().all(|b| b.is_ascii_digit()) && (fraction.is_empty() || numbers.len() == 3);
    let zone_ok = match zone {
        "" | "Z" => true,
        _ => {
            let offset = &zone[1..];
            let parts: Vec<&str> = offset.split(':').collect();
            match parts.as_slice() {
                [hours] if hours.len() == 4 => hours.bytes().all(|b| b.is_ascii_digit()),
                [hours] => two_digits(hours),
                [hours, minutes] => two_digits(hours) && two_digits(minutes),
                _ => false,
            }
        }
    };
    clock_ok && fraction_ok && zone_ok
}

/// Read `<Dt>` or `<DtTm>` below `path`.
fn parse_date(el: Option<Node<'_, '_>>, path: &str) -> Result<Option<Date>> {
    let Some(target) = el.and_then(|e| find(e, path)) else {
        return Ok(None);
    };
    let invalid = || Camt053Error(format!("A date in {path} is not valid"));
    if let Some(raw) = text(Some(target), "Dt") {
        return iso_date(&raw).map(Some).ok_or_else(invalid);
    }
    if let Some(raw) = text(Some(target), "DtTm") {
        let raw: String = raw.chars().take(64).collect();
        let date_len = if raw.get(4..5) == Some("-") { 10 } else { 8 };
        let date = raw.get(..date_len).and_then(iso_date).ok_or_else(invalid)?;
        if !valid_time_part(raw.get(date_len..).unwrap_or("")) {
            return Err(invalid());
        }
        return Ok(Some(date));
    }
    Ok(None)
}

/// ISO 20022 `ActiveOrHistoricCurrencyAndAmount`: decimal, no sign, no exponent. The
/// application limits the integer part to 12 digits (as for typed amounts).
fn valid_amount_syntax(raw: &str) -> bool {
    let (integer, fraction) = raw.split_once('.').unwrap_or((raw, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    (1..=12).contains(&integer.len())
        && digits(integer)
        && (!raw.contains('.') || (1..=5).contains(&fraction.len()))
        && digits(fraction)
}

fn amount(el: Option<Node<'_, '_>>, path: &str) -> Result<Option<(i64, String)>> {
    let Some(amt) = el.and_then(|e| find(e, path)) else {
        return Ok(None);
    };
    let Some(raw) = own_text(amt) else {
        return Ok(None);
    };
    let Some(currency) = amt.attribute("Ccy") else {
        return err("Amount has no currency attribute (Ccy)");
    };
    let raw = raw.trim();
    // Check the syntax before the conversion. This refuses negative values, exponents,
    // NaN, infinity and very large numbers.
    if !valid_amount_syntax(raw) {
        return err("An amount is not valid");
    }
    let invalid = |_| Camt053Error("An amount or a currency is not valid".into());
    let currency = validate_currency(currency).map_err(invalid)?;
    let decimal = Decimal::parse(raw).ok_or_else(|| Camt053Error("An amount is not valid".into()))?;
    let minor = to_minor(&decimal, &currency).map_err(invalid)?;
    Ok(Some((minor, currency)))
}

/// Return +1 for a credit and -1 for a debit.
///
/// For a reversal (`RvslInd`), the indicator already shows the direction of the
/// reversal booking. Thus no special processing is necessary.
fn sign(indicator: Option<&str>) -> Result<i64> {
    match indicator {
        Some("CRDT") => Ok(1),
        Some("DBIT") => Ok(-1),
        _ => err("A credit/debit indicator is not valid"),
    }
}

fn status(entry: Node<'_, '_>) -> Option<String> {
    // Versions 02 to 06: <Sts>BOOK</Sts>. Versions 07 and later: <Sts><Cd>BOOK</Cd></Sts>.
    let sts = find(entry, "Sts")?;
    if let Some(code) = text(Some(sts), "Cd").or_else(|| text(Some(sts), "Prtry")) {
        return Some(code);
    }
    let value = collapse(own_text(sts).unwrap_or(""));
    (!value.is_empty()).then_some(value)
}

fn party_name(details: Node<'_, '_>, role: &str) -> Option<String> {
    // Versions 02 to 07: RltdPties/Dbtr/Nm. Version 08 and later: RltdPties/Dbtr/Pty/Nm.
    text(Some(details), &format!("RltdPties/{role}/Pty/Nm"))
        .or_else(|| text(Some(details), &format!("RltdPties/{role}/Nm")))
}

fn party_iban(details: Node<'_, '_>, role: &str) -> Option<String> {
    let raw = text(Some(details), &format!("RltdPties/{role}Acct/Id/IBAN"))?;
    is_valid_iban(&raw).then(|| normalise_iban(&raw))
}

fn description(details: Option<Node<'_, '_>>, entry: Node<'_, '_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(details) = details {
        for unstructured in find_all(details, "RmtInf/Ustrd") {
            if let Some(t) = own_text(unstructured) {
                parts.push(collapse(t));
            }
        }
        if let Some(additional) = text(Some(details), "AddtlTxInf") {
            parts.push(additional);
        }
    }
    if let Some(entry_info) = text(Some(entry), "AddtlNtryInf")
        && !parts.contains(&entry_info)
    {
        parts.push(entry_info);
    }
    parts.join(" | ").chars().take(2000).collect()
}

fn structured_reference(details: Option<Node<'_, '_>>) -> Option<String> {
    text(details, "RmtInf/Strd/CdtrRefInf/Ref")
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A value of the fingerprint, as the Python version writes it (`None` is empty).
enum Part<'a> {
    Text(Option<&'a str>),
    Int(Option<i64>),
}

impl Ctx {
    fn fingerprint(&mut self, parts: &[Part<'_>]) -> String {
        let base = parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => t.unwrap_or("").to_string(),
                Part::Int(i) => i.map(|v| v.to_string()).unwrap_or_default(),
            })
            .collect::<Vec<_>>()
            .join("\x1f");
        let occurrence = self.seen.get(&base).copied().unwrap_or(0);
        self.seen.insert(base.clone(), occurrence + 1);
        let digest = crate::encoding::hex(Sha256::digest(format!("{base}\x1f{occurrence}").as_bytes()));
        format!("camt:{}", &digest[..64])
    }
}

fn entry_transactions(
    ctx: &mut Ctx,
    entry: Node<'_, '_>,
    account_key: &str,
    out: &mut Vec<StatementTransaction>,
) -> Result<()> {
    let reversal = text(Some(entry), "RvslInd").is_some_and(|v| v.to_lowercase() == "true");
    let Some(entry_amount) = amount(Some(entry), "Amt")? else {
        return err("Entry has no amount");
    };
    let entry_sign = sign(text(Some(entry), "CdtDbtInd").as_deref())?;
    let booking = match parse_date(Some(entry), "BookgDt")? {
        Some(date) => date,
        None => match parse_date(Some(entry), "ValDt")? {
            Some(date) => date,
            None => return err("Entry has no booking date"),
        },
    };
    let value_date = parse_date(Some(entry), "ValDt")?;
    let entry_ref = text(Some(entry), "AcctSvcrRef").or_else(|| text(Some(entry), "NtryRef"));

    let details_list = find_all(entry, "NtryDtls/TxDtls");
    // Only split a batch entry if each detail has its own amount.
    let mut split = details_list.len() > 1
        && details_list.iter().all(|d| find(*d, "Amt").is_some() || find(*d, "AmtDtls/TxAmt/Amt").is_some());
    let mut items: Vec<(Option<Node<'_, '_>>, i64, String, i64)> = Vec::new();
    if split {
        for d in &details_list {
            let found = match amount(Some(*d), "Amt")? {
                Some(a) => Some(a),
                None => amount(Some(*d), "AmtDtls/TxAmt/Amt")?,
            };
            let Some((value, currency)) = found else {
                return err("Transaction detail has no amount");
            };
            let d_sign = match text(Some(*d), "CdtDbtInd") {
                Some(indicator) => sign(Some(&indicator))?,
                None => entry_sign,
            };
            // TxAmt can be in the instructed currency. Use the entry currency only.
            if currency != entry_amount.1 {
                split = false;
                break;
            }
            items.push((Some(*d), value, currency, d_sign));
        }
    }
    if !split {
        items = vec![(details_list.first().copied(), entry_amount.0, entry_amount.1.clone(), entry_sign)];
    }

    for (index, (details, value, currency, item_sign)) in items.into_iter().enumerate() {
        let role = if item_sign < 0 { "Cdtr" } else { "Dbtr" };
        let mut counterparty = details.and_then(|d| party_name(d, role)).unwrap_or_default();
        let counterparty_iban = details.and_then(|d| party_iban(d, role));
        let tx_ref = text(details, "Refs/AcctSvcrRef");
        let bank_reference = tx_ref.clone().or_else(|| entry_ref.clone());
        let desc = description(details, entry);
        if counterparty.is_empty() {
            counterparty = truncate(&text(Some(entry), "AddtlNtryInf").unwrap_or_default(), 140);
        }
        let has_ref = entry_ref.is_some() || tx_ref.is_some();
        let booking_text = crate::dates::format(booking, "%Y-%m-%d");
        let fingerprint = ctx.fingerprint(&[
            Part::Text(Some(account_key)),
            Part::Text(Some(&booking_text)),
            Part::Int(Some(item_sign * value)),
            Part::Text(Some(&currency)),
            Part::Text(entry_ref.as_deref()),
            Part::Text(tx_ref.as_deref()),
            Part::Int(has_ref.then_some(index as i64)),
            Part::Text((!has_ref).then_some(counterparty.as_str())),
            Part::Text((!has_ref).then_some(desc.as_str())),
        ]);
        out.push(StatementTransaction {
            fingerprint,
            booking_date: booking,
            value_date,
            amount_minor: item_sign * value,
            currency,
            counterparty: truncate(&counterparty, 140),
            counterparty_iban,
            description: desc,
            reference: structured_reference(details),
            bank_reference: bank_reference.map(|r| truncate(&r, 35)),
            reversal,
        });
    }
    Ok(())
}

fn parse_balance(bal: Node<'_, '_>) -> Result<Option<Balance>> {
    let code = text(Some(bal), "Tp/CdOrPrtry/Cd").or_else(|| text(Some(bal), "Tp/CdOrPrtry/Prtry"));
    let value = amount(Some(bal), "Amt")?;
    let date = parse_date(Some(bal), "Dt")?;
    let (Some(code), Some((value, currency)), Some(date)) = (code, value, date) else {
        return Ok(None);
    };
    let s = sign(text(Some(bal), "CdtDbtInd").as_deref())?;
    Ok(Some(Balance { type_code: code, amount_minor: s * value, currency, date }))
}

fn parse_statement(ctx: &mut Ctx, version: u32, stmt: Node<'_, '_>) -> Result<Statement> {
    let statement_id = text(Some(stmt), "Id").unwrap_or_default();
    let iban = text(Some(stmt), "Acct/Id/IBAN").map(|raw| normalise_iban(&raw));
    if let Some(iban) = &iban
        && !is_valid_iban(iban)
    {
        return err("The statement account IBAN is not valid");
    }
    let other_id = text(Some(stmt), "Acct/Id/Othr/Id");
    let currency = text(Some(stmt), "Acct/Ccy");
    let mut balances = Vec::new();
    for bal in find_all(stmt, "Bal") {
        if let Some(balance) = parse_balance(bal)? {
            balances.push(balance);
        }
    }
    let mut transactions = Vec::new();
    let mut skipped = 0;
    let account_key = iban.clone().or_else(|| other_id.clone()).unwrap_or_default();
    for entry in find_all(stmt, "Ntry") {
        if let Some(status) = status(entry)
            && status != "BOOK"
        {
            skipped += 1;
            continue;
        }
        entry_transactions(ctx, entry, &account_key, &mut transactions)?;
    }
    Ok(Statement {
        message_id: ctx.message_id.clone(),
        version,
        statement_id,
        account_iban: iban,
        account_other_id: other_id,
        currency,
        balances,
        transactions,
        skipped_entries: skipped,
        account_name: text(Some(stmt), "Acct/Nm"),
        servicer_name: text(Some(stmt), "Acct/Svcr/FinInstnId/Nm"),
    })
}

/// The text of an XML document. ISO 20022 uses UTF-8. The parser of the Python version
/// (expat) also read UTF-16 and ISO-8859-1. Thus this function also reads them.
fn decode_xml(data: &[u8]) -> Result<String> {
    let invalid = || Camt053Error("The file is not well-formed XML".into());
    if let Some(rest) = data.strip_prefix(b"\xef\xbb\xbf") {
        return std::str::from_utf8(rest).map(str::to_string).map_err(|_| invalid());
    }
    let utf16 = |rest: &[u8], big_endian: bool| {
        let units: Vec<u16> =
            rest.chunks_exact(2)
                .map(|c| {
                    if big_endian {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .collect();
        String::from_utf16(&units).map_err(|_| invalid())
    };
    if let Some(rest) = data.strip_prefix(b"\xff\xfe") {
        return utf16(rest, false);
    }
    if let Some(rest) = data.strip_prefix(b"\xfe\xff") {
        return utf16(rest, true);
    }
    let declaration = &data[..data.len().min(200)];
    let declaration = String::from_utf8_lossy(declaration).to_lowercase();
    let declaration = declaration.split("?>").next().unwrap_or("");
    let latin1 = declaration.starts_with("<?xml")
        && ["iso-8859-1", "latin1", "latin-1", "iso8859-1"].iter().any(|name| {
            declaration.contains(&format!("\"{name}\"")) || declaration.contains(&format!("'{name}'"))
        });
    if latin1 {
        return Ok(data.iter().map(|b| char::from(*b)).collect());
    }
    std::str::from_utf8(data).map(str::to_string).map_err(|_| invalid())
}

/// Check the limits before the parse, in one fast pass over the text.
///
/// The parser builds the complete tree, and it compares each attribute of an element
/// with the other attributes of the element. Thus a document with very many elements
/// or with very many attributes on one element can use much memory or much CPU time.
/// This scan counts the start tags, the nesting depth and the `=` signs in each start
/// tag (namespace declarations included) without building a tree. A document that is
/// not well-formed can pass the scan: then the parser refuses it.
fn prescan(text: &str, limits: &Limits) -> Result<()> {
    let bytes = text.as_bytes();
    let skip_to = |from: usize, end: &[u8]| -> usize {
        bytes[from..].windows(end.len()).position(|w| w == end).map_or(bytes.len(), |p| from + p + end.len())
    };
    let mut i = 0;
    let mut depth: usize = 0;
    let mut elements = 0;
    while let Some(offset) = bytes[i..].iter().position(|b| *b == b'<') {
        let start = i + offset;
        let rest = &bytes[start..];
        if rest.starts_with(b"<!--") {
            i = skip_to(start + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = skip_to(start + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = skip_to(start + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            // A DTD: the parser refuses it.
            return Ok(());
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = skip_to(start + 2, b">");
        } else {
            elements += 1;
            if elements > limits.max_elements {
                return err("The XML document contains too many elements");
            }
            // Read to the end of the start tag. A quoted value can contain ">".
            let mut quote = None;
            let mut equals = 0;
            let mut end = start + 1;
            while end < bytes.len() {
                let b = bytes[end];
                match quote {
                    Some(q) if b == q => quote = None,
                    Some(_) => {}
                    None if b == b'"' || b == b'\'' => quote = Some(b),
                    None if b == b'=' => equals += 1,
                    None if b == b'>' => break,
                    None => {}
                }
                end += 1;
            }
            // Namespace declarations are not attributes. Thus permit some more here.
            if equals > 2 * limits.max_attributes {
                return err("An XML element has too many attributes");
            }
            if end > start && bytes.get(end - 1) != Some(&b'/') {
                depth += 1;
                if depth > limits.max_depth {
                    return err("The XML elements are nested too deeply");
                }
            }
            i = (end + 1).min(bytes.len());
        }
        if i >= bytes.len() {
            break;
        }
    }
    Ok(())
}

/// Check the limits for the elements, the depth and the attributes.
fn check_limits(doc: &Document<'_>, limits: &Limits) -> Result<()> {
    let mut depth: Vec<usize> = vec![0; doc.descendants().count() + 1];
    let mut elements = 0;
    for node in doc.descendants() {
        if !node.is_element() {
            continue;
        }
        elements += 1;
        if elements > limits.max_elements {
            return err("The XML document contains too many elements");
        }
        let parent_depth = node.parent().map_or(0, |p| depth[p.id().get_usize()]);
        let own = parent_depth + 1;
        depth[node.id().get_usize()] = own;
        if own > limits.max_depth {
            return err("The XML elements are nested too deeply");
        }
        if node.attributes().len() > limits.max_attributes {
            return err("An XML element has too many attributes");
        }
    }
    Ok(())
}

/// Parse one camt.053 XML document. A document can contain more than one statement.
pub fn parse_xml(data: &[u8]) -> Result<Vec<Statement>> {
    parse_xml_with(data, &Limits::default())
}

/// Parse one camt.053 XML document with the given limits.
pub fn parse_xml_with(data: &[u8], limits: &Limits) -> Result<Vec<Statement>> {
    let decoded = decode_xml(data)?;
    let text_data = decoded.as_str();
    prescan(text_data, limits)?;
    // The parser stops if the document has more nodes (elements, texts, comments …).
    let nodes_limit = u32::try_from(limits.max_elements.saturating_mul(3)).unwrap_or(u32::MAX);
    let options = ParsingOptions { allow_dtd: false, nodes_limit, ..ParsingOptions::default() };
    let doc = match Document::parse_with_options(text_data, options) {
        Ok(doc) => doc,
        Err(roxmltree::Error::DtdDetected | roxmltree::Error::EntityReferenceLoop(_)) => {
            return err("The XML file contains forbidden constructs (DTD or entities)");
        }
        Err(roxmltree::Error::NodesLimitReached) => {
            return err("The XML document contains too many elements");
        }
        Err(_) => return err("The file is not well-formed XML"),
    };
    check_limits(&doc, limits)?;
    let root = doc.root_element();
    let Some(namespace) = root.tag_name().namespace() else {
        return err("The XML document has no ISO 20022 namespace");
    };
    let version = namespace
        .strip_prefix("urn:iso:std:iso:20022:tech:xsd:camt.053.001.")
        .filter(|v| v.len() == 2 && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u32>().ok());
    let Some(version) = version.filter(|_| root.tag_name().name() == "Document") else {
        return err("The file is not a camt.053 document");
    };
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err(Camt053Error(format!("camt.053.001.{version:02} is not supported")));
    }
    let Some(body) = find(root, "BkToCstmrStmt") else {
        return err("The document has no BkToCstmrStmt element");
    };
    let mut ctx =
        Ctx { message_id: text(Some(body), "GrpHdr/MsgId").unwrap_or_default(), seen: HashMap::new() };
    let mut statements = Vec::new();
    for stmt in find_all(body, "Stmt") {
        statements.push(parse_statement(&mut ctx, version, stmt)?);
    }
    if statements.is_empty() {
        return err("The document contains no statement");
    }
    Ok(statements)
}

fn zip_members(data: &[u8], limits: &Limits) -> Result<Vec<Vec<u8>>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|_| Camt053Error("The ZIP file is not valid".into()))?;
    let files = (0..archive.len()).filter(|i| archive.by_index_raw(*i).is_ok_and(|f| !f.is_dir())).count();
    if files > limits.max_zip_members {
        return err("The ZIP file contains too many files");
    }
    let mut total: u64 = 0;
    let mut contents = Vec::new();
    for i in 0..archive.len() {
        let mut member = archive.by_index(i).map_err(|_| Camt053Error("The ZIP file is not valid".into()))?;
        if member.is_dir() || !member.name().is_ok_and(|n| n.to_lowercase().ends_with(".xml")) {
            continue;
        }
        if member.size() > limits.max_member_bytes {
            return err("A file in the ZIP archive is too large");
        }
        // Do not trust the declared size. Read with a limit.
        let mut content = Vec::new();
        (&mut member)
            .take(limits.max_member_bytes + 1)
            .read_to_end(&mut content)
            .map_err(|_| Camt053Error("The ZIP file is not valid".into()))?;
        if content.len() as u64 > limits.max_member_bytes {
            return err("A file in the ZIP archive is too large");
        }
        total += content.len() as u64;
        if total > limits.max_total_uncompressed_bytes {
            return err("The ZIP archive is too large when uncompressed");
        }
        contents.push(content);
    }
    Ok(contents)
}

/// Parse an XML document or a ZIP archive that contains XML documents.
pub fn parse_file(data: &[u8]) -> Result<Vec<Statement>> {
    parse_file_with(data, &Limits::default())
}

/// Parse an XML document or a ZIP archive with the given limits.
pub fn parse_file_with(data: &[u8], limits: &Limits) -> Result<Vec<Statement>> {
    if data.starts_with(b"PK\x03\x04") {
        let mut statements = Vec::new();
        for content in zip_members(data, limits)? {
            statements.extend(parse_xml_with(&content, limits)?);
        }
        if statements.is_empty() {
            return err("The ZIP archive contains no camt.053 XML file");
        }
        return Ok(statements);
    }
    parse_xml_with(data, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_parts() {
        for ok in ["", "T10:00:00", "T10:00:00.123+01:00", "T10:00Z", " 10", "T10:00:00-0500"] {
            assert!(valid_time_part(ok), "{ok}");
        }
        for bad in ["X", "T1:00", "T10:00:00+1", "T25:00", "T10:61"] {
            assert!(!valid_time_part(bad), "{bad}");
        }
    }

    #[test]
    fn amount_syntax() {
        assert!(valid_amount_syntax("12.50"));
        assert!(valid_amount_syntax("0"));
        for bad in ["-1", "1E5", "", ".5", "1.", "1.123456", "1234567890123", "NaN"] {
            assert!(!valid_amount_syntax(bad), "{bad}");
        }
    }
}
