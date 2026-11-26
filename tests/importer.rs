//! Tests of the import service.

mod common;

use common::*;
use feljosit::models::{AccountType, MatchType, RuleField};
use feljosit::services::accounts::{self, ManualTransaction, NewAccount};
use feljosit::services::categorise::{self, RuleValues};
use feljosit::services::importer::ImportError;
use feljosit::services::{Error as ServiceError, users};

fn private(conn: &rusqlite::Connection, user: &feljosit::models::User) -> i64 {
    add_account(conn, user, "Private", AccountType::Checking, PRIVATE_IBAN)
}

fn savings(conn: &rusqlite::Connection, user: &feljosit::models::User) -> i64 {
    add_account(conn, user, "Savings", AccountType::Savings, SAVINGS_IBAN)
}

fn two_accounts_zip() -> Vec<u8> {
    zip_files(&[
        ("private.xml", fixture("camt053_v04_private.xml")),
        ("savings.xml", fixture("camt053_v08_savings.xml")),
    ])
}

/// One XML file with two statements: the savings statement and a copy for PRIVATE_IBAN.
fn two_statements_xml() -> Vec<u8> {
    let text = String::from_utf8(fixture("camt053_v08_savings.xml")).unwrap();
    let start = text.find("<Stmt>").unwrap();
    let end = text.find("</Stmt>").unwrap() + "</Stmt>".len();
    let copy = text[start..end].replacen("<Id>SAV-2026-10</Id>", "<Id>PRIV-2026-10</Id>", 1).replacen(
        &format!("<IBAN>{SAVINGS_IBAN}</IBAN></Id>"),
        &format!("<IBAN>{PRIVATE_IBAN}</IBAN></Id>"),
        1,
    );
    format!("{}{copy}{}", &text[..end], &text[end..]).into_bytes()
}

fn count(conn: &rusqlite::Connection, account_id: i64) -> i64 {
    scalar(conn, &format!("SELECT count(*) FROM transactions WHERE account_id = {account_id}"))
}

#[test]
fn import_finds_account_by_iban_and_checks_balance() {
    let (conn, user) = database_with_user();
    let account_id = private(&conn, &user);
    let result = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    let [stmt] = result.statements.as_slice() else { panic!() };
    assert_eq!(stmt.account_id, account_id);
    assert_eq!(stmt.new_count, 7);
    assert_eq!(stmt.skipped_entries, 1);
    // Starting balance from OPBD and all entries give the closing balance.
    assert_eq!(stmt.balance_matches, Some(true));
    assert_eq!(accounts::balances(&conn, user.id).unwrap()[&account_id], 425005);
}

#[test]
fn import_is_idempotent() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    let error = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap_err();
    assert!(error.contains("imported before"));
    // The same content with a different file hash: no duplicate transactions.
    let mut data = fixture("camt053_v04_private.xml");
    data.push(b'\n');
    let result = import_data(&conn, &user, "again.xml", &data, None).unwrap();
    assert_eq!(result.new_count(), 0);
    assert_eq!(result.duplicate_count(), 7);
}

#[test]
fn import_needs_account() {
    let (conn, user) = database_with_user();
    assert!(
        import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap_err().contains("No account")
    );
}

#[test]
fn import_makes_the_account_if_the_budget_currency_is_given() {
    let (conn, user) = database_with_user();
    let request = feljosit::services::importer::ImportRequest {
        filename: "a.xml",
        data: &fixture("camt053_v04_private.xml"),
        account_id: None,
        budget_currency: Some("CHF"),
    };
    let result = feljosit::services::importer::import_statement_file(&conn, user.id, &request).unwrap();
    assert!(result.statements[0].account_created);
    let account = accounts::get_account(&conn, user.id, result.statements[0].account_id).unwrap();
    assert_eq!(account.iban.as_deref(), Some(PRIVATE_IBAN));
    assert!(account.on_budget);
}

#[test]
fn import_assigns_iban_to_selected_account() {
    let (conn, user) = database_with_user();
    let account_id = add_account(&conn, &user, "Private", AccountType::Checking, "");
    import_fixture(&conn, &user, "camt053_v04_private.xml", Some(account_id)).unwrap();
    assert_eq!(
        accounts::get_account(&conn, user.id, account_id).unwrap().iban.as_deref(),
        Some(PRIVATE_IBAN)
    );
}

#[test]
fn import_refuses_iban_mismatch() {
    let (conn, user) = database_with_user();
    let account_id = add_account(&conn, &user, "Private", AccountType::Checking, SAVINGS_IBAN);
    assert!(
        import_fixture(&conn, &user, "camt053_v04_private.xml", Some(account_id))
            .unwrap_err()
            .contains("IBAN")
    );
}

#[test]
fn import_refuses_account_of_other_user() {
    let (conn, user) = database_with_user();
    let other = users::create_user(&conn, "bob", "another long password").unwrap();
    let other_account = private(&conn, &other);
    let result = import_data(&conn, &user, "a.xml", &fixture("camt053_v04_private.xml"), Some(other_account));
    assert!(matches!(result, Err(ImportError::Service(ServiceError::NotFound(_)))));
}

#[test]
fn import_zip_with_two_accounts() {
    let (conn, user) = database_with_user();
    let private_id = private(&conn, &user);
    let savings_id = savings(&conn, &user);
    let result = import_data(&conn, &user, "both.zip", &two_accounts_zip(), None).unwrap();
    let [private, savings] = result.statements.as_slice() else { panic!() };
    assert_eq!((private.account_id, private.new_count), (private_id, 7));
    assert_eq!((savings.account_id, savings.new_count), (savings_id, 2));
    assert_eq!(private.balance_matches, Some(true));
    assert_eq!(savings.balance_matches, Some(true));
    // The transfer between the two accounts is found in the same import.
    assert_eq!(private.transfer_count, 1);
    assert_eq!(savings.transfer_count, 1);
    let balances = accounts::balances(&conn, user.id).unwrap();
    assert_eq!(balances[&private_id], 425005);
    assert_eq!(balances[&savings_id], 1050250);
    assert_eq!(result.new_count(), 9);
}

#[test]
fn import_one_xml_with_two_accounts() {
    let (conn, user) = database_with_user();
    let private_id = private(&conn, &user);
    let savings_id = savings(&conn, &user);
    let result = import_data(&conn, &user, "both.xml", &two_statements_xml(), None).unwrap();
    let summary: Vec<(i64, String, usize)> =
        result.statements.iter().map(|s| (s.account_id, s.statement_id.clone(), s.new_count)).collect();
    assert_eq!(summary, vec![(savings_id, "SAV-2026-10".into(), 2), (private_id, "PRIV-2026-10".into(), 2)]);
    assert!(result.statements.iter().all(|s| s.balance_matches == Some(true)));
    assert_eq!(count(&conn, private_id), 3); // Starting balance and 2 entries.
    assert_eq!(count(&conn, savings_id), 3);
}

#[test]
fn import_with_two_accounts_refuses_selected_account() {
    let (conn, user) = database_with_user();
    let private_id = private(&conn, &user);
    savings(&conn, &user);
    let error = import_data(&conn, &user, "both.zip", &two_accounts_zip(), Some(private_id)).unwrap_err();
    assert!(error.to_string().contains("different account"));
}

#[test]
fn transfers_between_budget_accounts() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    savings(&conn, &user);
    let first = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    let second = import_fixture(&conn, &user, "camt053_v08_savings.xml", None).unwrap();
    assert_eq!(first.statements[0].transfer_count, 1);
    assert_eq!(second.statements[0].transfer_count, 1);
}

#[test]
fn transfer_to_tracking_account_is_not_a_transfer() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    add_account(&conn, &user, "3a", AccountType::Pillar3a, SAVINGS_IBAN);
    let result = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    assert_eq!(result.statements[0].transfer_count, 0);
}

#[test]
fn rules_and_history_categorise() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    let groceries = category_id(&conn, &user, "Groceries");
    let rule = RuleValues {
        field: RuleField::Counterparty,
        match_type: MatchType::Contains,
        pattern: "migros",
        category_id: groceries,
        priority: 100,
    };
    categorise::create_rule(&conn, user.id, &rule).unwrap();
    let salary = category_id(&conn, &user, "Salary");
    // History: a manual transaction from the employer with a category.
    let cash = add_account(&conn, &user, "Cash", AccountType::Cash, "");
    let account = accounts::get_account(&conn, user.id, cash).unwrap();
    let manual = ManualTransaction {
        booking_date: date(2026, 9, 25),
        amount_minor: 100,
        counterparty: "Muster AG",
        memo: "",
        category_id: Some(salary),
    };
    let tx = accounts::add_manual_transaction(&conn, &account, &manual).unwrap();
    assert_eq!(
        scalar::<i64>(&conn, &format!("SELECT category_id FROM transactions WHERE id = {tx}")),
        salary
    );
    let result = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    assert_eq!(result.statements[0].categorised_count, 2);
    let category_of = |name: &str| -> Option<i64> {
        conn.query_row(
            "SELECT category_id FROM transactions WHERE counterparty = ?1 ORDER BY id DESC LIMIT 1",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(category_of("Debit card payment MIGROS ZUERICH"), Some(groceries));
    assert_eq!(category_of("Muster AG"), Some(salary));
}

#[test]
fn apply_rules_later() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    let rent = category_id(&conn, &user, "Rent and service charges");
    let rule = RuleValues {
        field: RuleField::CounterpartyIban,
        match_type: MatchType::Equals,
        pattern: "CH3880808001234567890",
        category_id: rent,
        priority: 100,
    };
    categorise::create_rule(&conn, user.id, &rule).unwrap();
    assert_eq!(categorise::apply_to_uncategorised(&conn, user.id).unwrap(), 1);
}

#[test]
fn rule_validation() {
    let (conn, user) = database_with_user();
    let category = category_id(&conn, &user, "Groceries");
    let rule = RuleValues {
        field: RuleField::Counterparty,
        match_type: MatchType::Contains,
        pattern: "  ",
        category_id: category,
        priority: 100,
    };
    assert!(categorise::create_rule(&conn, user.id, &rule).is_err());
}

#[test]
fn on_budget_account_needs_budget_currency() {
    let (conn, user) = database_with_user();
    let new = NewAccount::new("EUR", AccountType::Checking, "EUR");
    let error = accounts::create_account(&conn, user.id, &new).unwrap_err();
    assert!(error.to_string().contains("budget currency"));
    let mut new = NewAccount::new("EUR", AccountType::Checking, "EUR");
    new.on_budget = Some(false);
    assert!(!accounts::create_account(&conn, user.id, &new).unwrap().on_budget);
}

#[test]
fn duplicate_iban_refused() {
    let (conn, user) = database_with_user();
    private(&conn, &user);
    let mut new = NewAccount::new("Private", AccountType::Checking, "CHF");
    new.iban = Some(PRIVATE_IBAN);
    assert!(accounts::create_account(&conn, user.id, &new).unwrap_err().to_string().contains("exists"));
}
