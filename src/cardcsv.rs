//! Parser for the CSV export of credit card transactions.
//!
//! The file has one header line and one line for each card transaction, for example:
//!
//! ```text
//! TransactionId,CardId,Date,ValutaDate,Amount,Currency,OriginalAmount,OriginalCurrency,
//! MerchantName,MerchantPlace,MerchantCountry,StateType,Details,Type,Exchange Rate
//! ```
//!
//! (one line in the file). See ADR-0017.
//!
//! * A positive `Amount` is a charge to the card (an outflow). A negative amount is a
//!   credit, for example a payment to the card or a refund.
//! * Only lines with the `StateType` `BOOKED` become transactions. The parser counts
//!   the other lines (for example pending authorisations) as skipped entries.
//! * `TransactionId` gives the fingerprint. Thus an import of an overlapping file does
//!   not add a transaction two times.
//! * The file has no IBAN and no balance. The user selects the account.
//!
//! Security: the caller limits the size of the input. The parser limits the number of
//! lines and the length of the fields. Error messages do not include values from the
//! file.

use crate::dates::Date;
use std::collections::{BTreeSet, HashMap};
use std::fmt;

use sha2::{Digest, Sha256};

use crate::camt053::{Statement, StatementTransaction};
use crate::money::{Decimal, format_minor, to_minor, validate_currency};

/// The columns that the parser needs. The other columns are optional.
pub const REQUIRED_COLUMNS: &[&str] = &["TransactionId", "Date", "Amount", "Currency", "StateType"];
pub const MAX_ROWS: u64 = 100_000;
pub const MAX_FIELD_CHARS: usize = 1_000;

/// The file is not a valid credit card CSV file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardCsvError(pub String);

impl fmt::Display for CardCsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CardCsvError {}

type Result<T> = std::result::Result<T, CardCsvError>;

/// The characters of Windows-1252 from 0x80 to 0x9F. `None`: not defined.
const CP1252_HIGH: [Option<char>; 32] = [
    Some('€'),
    None,
    Some('‚'),
    Some('ƒ'),
    Some('„'),
    Some('…'),
    Some('†'),
    Some('‡'),
    Some('ˆ'),
    Some('‰'),
    Some('Š'),
    Some('‹'),
    Some('Œ'),
    None,
    Some('Ž'),
    None,
    None,
    Some('‘'),
    Some('’'),
    Some('“'),
    Some('”'),
    Some('•'),
    Some('–'),
    Some('—'),
    Some('˜'),
    Some('™'),
    Some('š'),
    Some('›'),
    Some('œ'),
    None,
    Some('ž'),
    Some('Ÿ'),
];

fn decode_cp1252(data: &[u8]) -> String {
    data.iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)].unwrap_or('\u{FFFD}'),
            _ => char::from(b),
        })
        .collect()
}

fn decode(data: &[u8]) -> String {
    let data_without_bom = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    match std::str::from_utf8(data_without_bom) {
        Ok(text) => text.to_string(),
        // Older exports from Windows programs.
        Err(_) => decode_cp1252(data),
    }
}

fn header_line(text: &str) -> &str {
    text.trim_start_matches('\u{feff}').split('\n').next().unwrap_or("").trim()
}

fn delimiter(header: &str) -> u8 {
    if header.matches(';').count() > header.matches(',').count() { b';' } else { b',' }
}

/// Return true if the first line of the file is the header of a credit card CSV file.
pub fn looks_like_card_csv(data: &[u8]) -> bool {
    let prefix = &data[..data.len().min(4096)];
    let text = decode(prefix);
    let header = header_line(&text);
    let delim = char::from(delimiter(header));
    let names: BTreeSet<&str> = header.split(delim).map(|n| n.trim().trim_matches('"')).collect();
    REQUIRED_COLUMNS.iter().all(|column| names.contains(column))
}

/// Parse `2025-10-23 00:00:00`, `2025-10-23` or `23.10.2025`. Empty: `None`.
fn date(text: &str, line: u64) -> Result<Option<Date>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: String = text.chars().take(10).collect();
    for pattern in ["%Y-%m-%d", "%d.%m.%Y"] {
        if let Some(parsed) = crate::dates::parse(&value, pattern) {
            return Ok(Some(parsed));
        }
    }
    Err(CardCsvError(format!("Line {line} has a date that is not valid.")))
}

fn decimal(text: &str, line: u64) -> Result<Decimal> {
    let cleaned = text.trim().replace('\'', "");
    let (sign, rest) = match cleaned.strip_prefix(['+', '-']) {
        Some(rest) => (&cleaned[..1], rest),
        None => ("", cleaned.as_str()),
    };
    let (integer, fraction) = rest.split_once('.').unwrap_or((rest, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let ok = (1..=12).contains(&integer.len())
        && digits(integer)
        && digits(fraction)
        && (!rest.contains('.') || (1..=5).contains(&fraction.len()));
    let invalid = || CardCsvError(format!("Line {line} has an amount that is not valid."));
    if !ok {
        return Err(invalid());
    }
    Decimal::parse(&format!("{sign}{rest}")).ok_or_else(invalid)
}

/// A text field. A field without a letter (for example `0.000`) is empty.
fn text_field(value: &str) -> String {
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().any(char::is_alphabetic) { value } else { String::new() }
}

/// Format the original amount, for example `EUR 29.99`. Else keep the text.
fn original(amount: &str, currency: &str) -> String {
    let parsed = Decimal::parse(&amount.trim().replace('\'', ""))
        .ok_or(())
        .and_then(|d| validate_currency(currency).map(|c| (d, c)).map_err(|_| ()))
        .and_then(|(d, c)| to_minor(&d, &c).map_err(|_| ()));
    match parsed {
        Ok(minor) => format_minor(minor, currency, true),
        Err(()) => format!("{currency} {amount}"),
    }
}

fn get<'a>(row: &'a HashMap<&str, &str>, name: &str) -> &'a str {
    row.get(name).copied().unwrap_or("")
}

fn description(row: &HashMap<&str, &str>, counterparty: &str, currency: &str) -> String {
    let mut parts = Vec::new();
    let details = text_field(get(row, "Details"));
    if !details.is_empty() && details != counterparty {
        parts.push(details);
    }
    let place = [text_field(get(row, "MerchantPlace")), text_field(get(row, "MerchantCountry"))]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    if !place.is_empty() {
        parts.push(place);
    }
    let original_currency = get(row, "OriginalCurrency").trim().to_uppercase();
    let original_amount = get(row, "OriginalAmount").trim();
    if !original_currency.is_empty() && original_currency != currency && !original_amount.is_empty() {
        let mut text = format!("Original amount {}", original(original_amount, &original_currency));
        let rate = get(row, "Exchange Rate").trim();
        if !rate.is_empty() {
            text.push_str(&format!(", exchange rate {rate}"));
        }
        parts.push(text);
    }
    parts.join(" · ")
}

fn transaction(row: &HashMap<&str, &str>, line: u64) -> Result<StatementTransaction> {
    let transaction_id = get(row, "TransactionId").trim();
    if transaction_id.is_empty() {
        return Err(CardCsvError(format!("Line {line} has no transaction ID.")));
    }
    let money_error = || CardCsvError(format!("Line {line} has an amount or a currency that is not valid."));
    let currency = validate_currency(get(row, "Currency")).map_err(|_| money_error())?;
    // A charge is positive in the file. In the application, an outflow is negative.
    let amount_minor =
        -to_minor(&decimal(get(row, "Amount"), line)?, &currency).map_err(|_| money_error())?;
    let Some(booking_date) = date(get(row, "Date"), line)? else {
        return Err(CardCsvError(format!("Line {line} has no date.")));
    };
    let mut counterparty = text_field(get(row, "MerchantName"));
    if counterparty.is_empty() {
        counterparty = text_field(get(row, "Details"));
    }
    let digest = hex::encode(Sha256::digest(transaction_id.as_bytes()));
    Ok(StatementTransaction {
        fingerprint: format!("card:{}", &digest[..64]),
        booking_date,
        value_date: date(get(row, "ValutaDate"), line)?,
        amount_minor,
        description: description(row, &counterparty, &currency),
        currency,
        counterparty: counterparty.chars().take(140).collect(),
        counterparty_iban: None,
        reference: None,
        bank_reference: Some(transaction_id.chars().take(35).collect()),
        reversal: false,
    })
}

/// Check the quotes as Python's `csv` module with `strict=True`: a quoted field must
/// end before the end of the file, and a delimiter or a line end must follow it.
fn strict_syntax_ok(text: &str, delim: u8) -> bool {
    #[derive(PartialEq)]
    enum State {
        FieldStart,
        Unquoted,
        Quoted,
        QuoteInQuoted,
    }
    let delim = char::from(delim);
    let mut state = State::FieldStart;
    for ch in text.chars() {
        state = match state {
            State::FieldStart | State::Unquoted if ch == delim || ch == '\n' || ch == '\r' => {
                State::FieldStart
            }
            State::FieldStart if ch == '"' => State::Quoted,
            State::FieldStart | State::Unquoted => State::Unquoted,
            State::Quoted if ch == '"' => State::QuoteInQuoted,
            State::Quoted => State::Quoted,
            State::QuoteInQuoted if ch == '"' => State::Quoted,
            State::QuoteInQuoted if ch == delim || ch == '\n' || ch == '\r' => State::FieldStart,
            State::QuoteInQuoted => return false,
        };
    }
    state != State::Quoted
}

/// Parse a credit card CSV file.
pub fn parse_file(data: &[u8]) -> Result<Statement> {
    parse_file_with_max_rows(data, MAX_ROWS)
}

/// Parse a credit card CSV file with a different row limit (for the tests).
pub fn parse_file_with_max_rows(data: &[u8], max_rows: u64) -> Result<Statement> {
    let decoded = decode(data);
    let text = decoded.trim_start_matches('\u{feff}');
    let syntax_error = || CardCsvError("The CSV syntax of the file is not valid.".into());
    let delim = delimiter(header_line(text));
    if !strict_syntax_ok(text, delim) {
        return Err(syntax_error());
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = reader.records();
    let header: Vec<String> = match records.next() {
        Some(record) => record.map_err(|_| syntax_error())?.iter().map(|n| n.trim().to_string()).collect(),
        None => Vec::new(),
    };
    if REQUIRED_COLUMNS.iter().any(|column| !header.iter().any(|h| h == column)) {
        return Err(CardCsvError("The file is not a credit card CSV file: columns are missing.".into()));
    }
    let mut transactions = Vec::new();
    let mut skipped = 0;
    for record in records {
        let record = record.map_err(|_| syntax_error())?;
        let line = record.position().map_or(0, |p| p.line());
        if record.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        if line > max_rows + 1 {
            return Err(CardCsvError(format!("The file has more than {max_rows} lines.")));
        }
        if record.len() != header.len() {
            return Err(CardCsvError(format!("Line {line} does not have {} columns.", header.len())));
        }
        if record.iter().any(|field| field.chars().count() > MAX_FIELD_CHARS) {
            return Err(CardCsvError(format!("Line {line} has a field that is too long.")));
        }
        let row: HashMap<&str, &str> = header.iter().map(String::as_str).zip(record.iter()).collect();
        if get(&row, "StateType").trim().to_uppercase() != "BOOKED" {
            skipped += 1;
            continue;
        }
        transactions.push(transaction(&row, line)?);
    }
    let currencies: BTreeSet<&str> = transactions.iter().map(|t| t.currency.as_str()).collect();
    if currencies.len() > 1 {
        return Err(CardCsvError("The file has transactions in more than one currency.".into()));
    }
    let currency = currencies.into_iter().next().map(str::to_string);
    Ok(Statement { currency, transactions, skipped_entries: skipped, ..Statement::default() })
}
