//! The import of credit card CSV files (see ADR-0017).

mod common;

use chrono::NaiveDate;
use common::{FilePart, TestApp, database_with_user, fixture};
use feljosit::cardcsv::{self, parse_file};
use feljosit::models::{Account, AccountType, TransactionSource, User};
use feljosit::services::accounts::{self, ManualTransaction, NewAccount};
use feljosit::services::importer::{ImportRequest, import_statement_file};
use rusqlite::Connection;

const HEADER: &str = "TransactionId,CardId,Date,ValutaDate,Amount,Currency,OriginalAmount,OriginalCurrency,\
MerchantName,MerchantPlace,MerchantCountry,StateType,Details,Type,Exchange Rate";
/// The example line of the issue (with an anonymised transaction and card ID).
const FEE: &str = "TRX2025102300001112211,443318XX00000001,2025-10-23 00:00:00,2025-10-23 00:00:00,\
75.000,CHF,75.000,CHF,,0.000,75.000,BOOKED,Belastung Jahresgebühr,merchant,1.000000";

fn csv(lines: &[&str]) -> Vec<u8> {
    let mut all = vec![HEADER];
    all.extend_from_slice(lines);
    all.join("\n").into_bytes()
}

fn line(changes: &[(&str, &str)]) -> String {
    let names: Vec<&str> = HEADER.split(',').collect();
    let mut values: Vec<String> = FEE.split(',').map(str::to_string).collect();
    for (name, value) in changes {
        let index = names.iter().position(|n| n == name).unwrap();
        values[index] = value.to_string();
    }
    values.join(",")
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

#[test]
fn parse_the_example_line() {
    let statement = parse_file(&csv(&[FEE])).unwrap();
    let [tx] = statement.transactions.as_slice() else { panic!() };
    assert_eq!(tx.booking_date, date(2025, 10, 23));
    assert_eq!(tx.value_date, Some(date(2025, 10, 23)));
    assert_eq!(tx.amount_minor, -7500); // A charge is an outflow.
    assert_eq!(tx.currency, "CHF");
    // No merchant: the details are the counterparty. The numbers in the merchant place
    // and country are not text.
    assert_eq!(tx.counterparty, "Belastung Jahresgebühr");
    assert_eq!(tx.description, "");
    assert_eq!(tx.counterparty_iban, None);
    assert_eq!(tx.bank_reference.as_deref(), Some("TRX2025102300001112211"));
    assert!(tx.fingerprint.starts_with("card:"));
    assert_eq!(statement.account_iban, None);
    assert_eq!(statement.currency.as_deref(), Some("CHF"));
    assert_eq!(statement.skipped_entries, 0);
}

#[test]
fn parse_the_fixture() {
    let statement = parse_file(&fixture("card_transactions.csv")).unwrap();
    assert_eq!(statement.skipped_entries, 1); // The authorisation is not booked.
    let [_fee, migros, cafe, payment] = statement.transactions.as_slice() else { panic!() };
    assert_eq!(migros.counterparty, "Migros");
    assert_eq!(migros.description, "Migros Zürich · Zürich, CHE");
    assert_eq!(migros.value_date, Some(date(2025, 10, 27)));
    assert_eq!(cafe.counterparty, "Café \"Zum Hafen\"");
    assert_eq!(cafe.amount_minor, -2740);
    assert_eq!(
        cafe.description,
        "Café \"Zum Hafen\", Konstanz · Konstanz, DEU · Original amount EUR 29.99, exchange rate 0.913638"
    );
    assert_eq!(payment.amount_minor, 120000); // A payment to the card is an inflow.
    assert_eq!(payment.counterparty, "Ihre Zahlung - Danke");
    let fingerprints: std::collections::HashSet<_> =
        statement.transactions.iter().map(|t| &t.fingerprint).collect();
    assert_eq!(fingerprints.len(), 4);
    // The same fingerprints as the Python version.
    assert_eq!(
        statement.transactions[0].fingerprint,
        "card:61ec6a117c697914d65c87b19d5a0fce680297f943b37b5283b34cda2dc98105"
    );
}

#[test]
fn parse_semicolons_bom_and_windows_encoding() {
    let text = format!("\u{feff}{}\r\n{}", HEADER.replace(',', ";"), FEE.replace(',', ";"));
    let statement = parse_file(text.as_bytes()).unwrap();
    assert_eq!(statement.transactions[0].amount_minor, -7500);
    // Windows-1252: "ü" is one byte (0xFC).
    let cp1252: Vec<u8> = String::from_utf8(csv(&[FEE]))
        .unwrap()
        .chars()
        .map(|c| if c == 'ü' { 0xFC } else { c as u8 })
        .collect();
    let statement = parse_file(&cp1252).unwrap();
    assert_eq!(statement.transactions[0].counterparty, "Belastung Jahresgebühr");
}

#[test]
fn detection() {
    assert!(cardcsv::looks_like_card_csv(&csv(&[FEE])));
    assert!(cardcsv::looks_like_card_csv(&fixture("card_transactions.csv")));
    assert!(!cardcsv::looks_like_card_csv(&fixture("camt053_v04_private.xml")));
    assert!(!cardcsv::looks_like_card_csv(b"Date,Amount\n2025-10-23,75.00\n"));
}

#[test]
fn parse_errors() {
    let long = "x".repeat(1001);
    let extra = format!("{FEE},extra");
    let cases: Vec<(Vec<String>, &str)> = vec![
        (vec![line(&[("Amount", "abc")])], "Line 2 has an amount that is not valid."),
        (vec![line(&[("Amount", "75.005")])], "Line 2 has an amount or a currency"),
        (vec![line(&[("Currency", "Swiss francs")])], "Line 2 has an amount or a currency"),
        (vec![line(&[("Date", "")])], "Line 2 has no date."),
        (vec![line(&[("Date", "23/10/2025")])], "Line 2 has a date that is not valid."),
        (vec![line(&[("TransactionId", "")])], "Line 2 has no transaction ID."),
        (vec![FEE.to_string(), extra], "Line 3 does not have 15 columns."),
        (vec![line(&[("Details", &long)])], "Line 2 has a field that is too long."),
        (
            vec![FEE.to_string(), line(&[("TransactionId", "T2"), ("Currency", "EUR")])],
            "more than one currency",
        ),
        (vec!["\"unclosed".to_string()], "The CSV syntax of the file is not valid."),
    ];
    for (lines, message) in cases {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let error = parse_file(&csv(&refs)).unwrap_err();
        assert!(error.0.contains(message), "{message}: {error}");
    }
}

#[test]
fn error_messages_do_not_include_values() {
    let error = parse_file(&csv(&[&line(&[("Amount", "secret-value")])])).unwrap_err();
    assert!(!error.0.contains("secret"));
}

#[test]
fn missing_columns() {
    let header = HEADER.replace("StateType", "State");
    let data = format!("{header}\n{FEE}");
    assert!(parse_file(data.as_bytes()).unwrap_err().0.contains("columns are missing"));
}

#[test]
fn row_limit() {
    let lines: Vec<String> = (0..3).map(|i| line(&[("TransactionId", &format!("T{i}"))])).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let error = cardcsv::parse_file_with_max_rows(&csv(&refs), 2).unwrap_err();
    assert!(error.0.contains("more than 2 lines"), "{error}");
}

fn card(conn: &Connection, user: &User) -> i64 {
    accounts::create_account(conn, user.id, &NewAccount::new("Credit card", AccountType::CreditCard, "CHF"))
        .unwrap()
        .id
}

fn import(
    conn: &Connection,
    user: &User,
    data: &[u8],
    account_id: Option<i64>,
    name: &str,
) -> Result<feljosit::services::importer::ImportResult, String> {
    let request = ImportRequest { filename: name, data, account_id, budget_currency: None };
    import_statement_file(conn, user.id, &request).map_err(|e| e.to_string())
}

#[test]
fn import_into_the_selected_account() {
    let (conn, user) = database_with_user();
    let card = card(&conn, &user);
    let result = import(&conn, &user, &fixture("card_transactions.csv"), Some(card), "card.csv").unwrap();
    let [item] = result.statements.as_slice() else { panic!() };
    assert_eq!(item.account_id, card);
    assert_eq!(item.new_count, 4);
    assert_eq!(item.skipped_entries, 1);
    assert_eq!(item.balance_matches, None); // The file has no balance.
    let sources: Vec<String> = conn
        .prepare("SELECT DISTINCT source FROM transactions WHERE account_id = ?1")
        .unwrap()
        .query_map([card], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(sources, vec![TransactionSource::CardCsv.as_str()]);
    assert_eq!(accounts::balances(&conn, user.id).unwrap()[&card], -7500 - 4535 - 2740 + 120000);
}

#[test]
fn import_ignores_known_transactions() {
    let (conn, user) = database_with_user();
    let card = card(&conn, &user);
    import(&conn, &user, &csv(&[FEE]), Some(card), "card.csv").unwrap();
    assert!(
        import(&conn, &user, &csv(&[FEE]), Some(card), "card.csv").unwrap_err().contains("imported before")
    );
    // An overlapping export: only the new transactions are added.
    let result = import(&conn, &user, &fixture("card_transactions.csv"), Some(card), "later.csv").unwrap();
    assert_eq!(result.new_count(), 3);
    assert_eq!(result.duplicate_count(), 1);
}

#[test]
fn import_needs_the_account() {
    let (conn, user) = database_with_user();
    let error = import(&conn, &user, &csv(&[FEE]), None, "card.csv").unwrap_err();
    assert!(error.contains("Select the credit card account"));
}

#[test]
fn import_refuses_a_different_currency() {
    let (conn, user) = database_with_user();
    let card = card(&conn, &user);
    let error =
        import(&conn, &user, &csv(&[&line(&[("Currency", "EUR")])]), Some(card), "card.csv").unwrap_err();
    assert!(error.contains("currency"));
}

#[test]
fn payment_from_the_private_account_is_a_transfer() {
    let (conn, user) = database_with_user();
    let private =
        accounts::create_account(&conn, user.id, &NewAccount::new("Private", AccountType::Checking, "CHF"))
            .unwrap();
    accounts::add_manual_transaction(
        &conn,
        &private,
        &ManualTransaction {
            booking_date: date(2025, 10, 29),
            amount_minor: -120000,
            counterparty: "Card issuer",
            memo: "",
            category_id: None,
        },
    )
    .unwrap();
    let card = card(&conn, &user);
    let result = import(&conn, &user, &fixture("card_transactions.csv"), Some(card), "card.csv").unwrap();
    assert_eq!(result.statements[0].matched_transfer_count, 1);
    let is_transfer: bool = conn
        .query_row("SELECT is_transfer FROM transactions WHERE amount_minor = 120000", [], |r| r.get(0))
        .unwrap();
    assert!(is_transfer);
    let _ = Account::get(&conn, card).unwrap();
}

#[test]
fn upload_on_the_import_page() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let user_id: i64 = test.scalar("SELECT id FROM users WHERE username = 'anna'");
    let card = test.app.with_db(|conn| {
        accounts::create_account(
            conn,
            user_id,
            &NewAccount::new("Credit card", AccountType::CreditCard, "CHF"),
        )
        .unwrap()
        .id
    });
    let data = fixture("card_transactions.csv");
    let mut upload = |account_id: &str| {
        http.post_files(
            "/import",
            &[("csrf_token", &token), ("account_id", account_id)],
            &[FilePart { field: "statement", filename: "card.csv", content_type: "text/csv", data: &data }],
        )
    };
    let response = upload("");
    assert_eq!(response.status, 400);
    assert!(response.text().contains("Select the credit card account."));
    let response = upload(&card.to_string());
    assert_eq!(response.status, 200);
    assert!(response.text().contains("4 new transactions"));
    assert!(response.text().contains("1 entries are not booked (pending) and were ignored"));
    let page = http.get(&format!("/transactions?account={card}")).text();
    assert!(page.contains("<strong>Migros</strong>"));
}
