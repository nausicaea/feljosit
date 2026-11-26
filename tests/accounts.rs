//! The import adds unknown accounts, and the user can edit accounts.

mod common;

use common::*;
use feljosit::camt053::parse_xml;
use feljosit::models::{Account, AccountType, Transaction, TransactionSource};
use feljosit::services::accounts::{self, NewAccount};
use feljosit::services::categorise;
use feljosit::services::importer::{ImportRequest, ImportResult, import_statement_file};
use rusqlite::Connection;

const PRIVATE: &str = "camt053_v04_private.xml";
const SAVINGS: &str = "camt053_v08_savings.xml";

fn import(
    conn: &Connection,
    user: &feljosit::models::User,
    data: &[u8],
    name: &str,
) -> Result<ImportResult, String> {
    let request = ImportRequest { filename: name, data, account_id: None, budget_currency: Some("CHF") };
    import_statement_file(conn, user.id, &request).map_err(|e| e.to_string())
}

/// Add an account name and a bank name to the private statement.
fn with_names(data: &[u8], account: Option<&str>, bank: Option<&str>) -> Vec<u8> {
    let mut text = String::from_utf8(data.to_vec()).unwrap();
    if let Some(account) = account {
        text = text.replacen("<Ccy>CHF</Ccy>\n", &format!("<Ccy>CHF</Ccy>\n<Nm>{account}</Nm>\n"), 1);
    }
    if let Some(bank) = bank {
        text = text.replacen(
            "</Ownr>",
            &format!("</Ownr>\n<Svcr><FinInstnId><Nm>{bank}</Nm></FinInstnId></Svcr>"),
            1,
        );
    }
    text.into_bytes()
}

fn all_accounts(conn: &Connection, user: &feljosit::models::User) -> Vec<Account> {
    accounts::list_accounts(conn, user.id, true).unwrap()
}

fn txs(conn: &Connection, account_id: i64) -> Vec<Transaction> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {} FROM transactions WHERE account_id = ?1 ORDER BY id",
            Transaction::COLUMNS
        ))
        .unwrap();
    stmt.query_map([account_id], Transaction::from_row).unwrap().map(Result::unwrap).collect()
}

#[test]
fn parser_reads_account_and_bank_names() {
    let statement =
        parse_xml(&with_names(&fixture(PRIVATE), Some("Privat"), Some("Bank X"))).unwrap().remove(0);
    assert_eq!(statement.account_name.as_deref(), Some("Privat"));
    assert_eq!(statement.servicer_name.as_deref(), Some("Bank X"));
    let plain = parse_xml(&fixture(PRIVATE)).unwrap().remove(0);
    assert!(plain.account_name.is_none() && plain.servicer_name.is_none());
}

#[test]
fn import_adds_unknown_account() {
    let (conn, user) = database_with_user();
    let result = import(&conn, &user, &fixture(PRIVATE), "statement.xml").unwrap();
    let [stmt] = result.statements.as_slice() else { panic!() };
    assert!(stmt.account_created);
    assert_eq!(stmt.new_count, 7);
    assert_eq!(stmt.balance_matches, Some(true));
    let [account] = all_accounts(&conn, &user).try_into().unwrap();
    assert_eq!(account.name, "Account CH89 0023 5000 0123 4567 8");
    assert_eq!((account.iban.as_deref(), account.currency.as_str()), (Some(PRIVATE_IBAN), "CHF"));
    assert!(account.account_type == AccountType::Checking && account.on_budget);
    assert_eq!(accounts::balances(&conn, user.id).unwrap()[&account.id], 425005);
    // The next statement for the same IBAN uses the same account.
    let mut data = fixture(PRIVATE);
    data.push(b'\n');
    let again = import(&conn, &user, &data, "again.xml").unwrap();
    assert!(!again.statements[0].account_created);
    assert_eq!(all_accounts(&conn, &user).len(), 1);
}

#[test]
fn new_account_name_from_statement() {
    for (account, bank, expected) in
        [(Some("Privatkonto"), Some("Bank X"), "Privatkonto"), (None, Some("Bank X"), "Bank X ···5678")]
    {
        let (conn, user) = database_with_user();
        import(&conn, &user, &with_names(&fixture(PRIVATE), account, bank), "statement.xml").unwrap();
        let names: Vec<String> = all_accounts(&conn, &user).into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec![expected]);
    }
}

#[test]
fn new_account_in_foreign_currency_is_tracking() {
    let (conn, user) = database_with_user();
    let data = String::from_utf8(fixture(SAVINGS)).unwrap().replace("CHF", "EUR");
    import(&conn, &user, data.as_bytes(), "statement.xml").unwrap();
    let [account] = all_accounts(&conn, &user).try_into().unwrap();
    assert_eq!(account.currency, "EUR");
    assert!(!account.on_budget);
}

#[test]
fn import_adds_two_accounts_and_finds_transfers() {
    let (conn, user) = database_with_user();
    let data = zip_files(&[("private.xml", fixture(PRIVATE)), ("savings.xml", fixture(SAVINGS))]);
    let result = import(&conn, &user, &data, "both.zip").unwrap();
    assert_eq!(result.statements.iter().map(|s| s.account_created).collect::<Vec<_>>(), vec![true, true]);
    // The private statement comes first, but its transfer to the new savings account is
    // found: the import adds all accounts before the transactions.
    assert_eq!(result.statements.iter().map(|s| s.transfer_count).collect::<Vec<_>>(), vec![1, 1]);
    assert_eq!(all_accounts(&conn, &user).len(), 2);
}

#[test]
fn new_account_finds_transfers_imported_before() {
    let (conn, user) = database_with_user();
    import(&conn, &user, &fixture(PRIVATE), "statement.xml").unwrap();
    let [private] = all_accounts(&conn, &user).try_into().unwrap();
    let to_savings = txs(&conn, private.id)
        .into_iter()
        .find(|t| t.counterparty_iban.as_deref() == Some(SAVINGS_IBAN))
        .unwrap();
    assert!(!to_savings.is_transfer); // No account has the savings IBAN yet.
    import(&conn, &user, &fixture(SAVINGS), "savings.xml").unwrap();
    let after = tx(&conn, to_savings.id);
    assert!(after.is_transfer);
    assert_eq!(after.category_id, None);
}

#[test]
fn no_new_account_without_budget_currency_or_iban() {
    let (conn, user) = database_with_user();
    let error = import_fixture(&conn, &user, PRIVATE, None).unwrap_err();
    assert!(error.contains("No account has the IBAN"));
    let no_iban = String::from_utf8(fixture(SAVINGS)).unwrap().replacen(
        &format!("<IBAN>{SAVINGS_IBAN}</IBAN></Id>"),
        "<Othr><Id>12345</Id></Othr></Id>",
        1,
    );
    assert!(import(&conn, &user, no_iban.as_bytes(), "statement.xml").unwrap_err().contains("no IBAN"));
    assert!(all_accounts(&conn, &user).is_empty());
}

#[test]
fn selected_account_is_not_replaced() {
    let (conn, user) = database_with_user();
    let account = add_account(&conn, &user, "Savings", AccountType::Savings, SAVINGS_IBAN);
    let request = ImportRequest {
        filename: "x.xml",
        data: &fixture(PRIVATE),
        account_id: Some(account),
        budget_currency: Some("CHF"),
    };
    assert!(import_statement_file(&conn, user.id, &request).unwrap_err().to_string().contains("IBAN"));
    assert_eq!(all_accounts(&conn, &user).len(), 1);
}

#[test]
fn update_account_name_and_type() {
    let (conn, user) = database_with_user();
    import(&conn, &user, &fixture(PRIVATE), "statement.xml").unwrap();
    let [account] = all_accounts(&conn, &user).try_into().unwrap();
    let updated =
        accounts::update_account(&conn, &account, "  UBS private  ", AccountType::CreditCard, "CHF", None)
            .unwrap();
    assert_eq!(
        (updated.name.as_str(), updated.account_type, updated.on_budget),
        ("UBS private", AccountType::CreditCard, true)
    );
    let error =
        accounts::update_account(&conn, &updated, " ", AccountType::Checking, "CHF", None).unwrap_err();
    assert!(error.to_string().contains("1 to 100"));
}

#[test]
fn update_account_refuses_budget_in_foreign_currency() {
    let (conn, user) = database_with_user();
    let data = String::from_utf8(fixture(SAVINGS)).unwrap().replace("CHF", "EUR");
    import(&conn, &user, data.as_bytes(), "statement.xml").unwrap();
    let [account] = all_accounts(&conn, &user).try_into().unwrap();
    let error =
        accounts::update_account(&conn, &account, "EUR", AccountType::Savings, "CHF", None).unwrap_err();
    assert!(error.to_string().contains("budget currency"));
    assert!(!accounts::get_account(&conn, user.id, account.id).unwrap().on_budget);
}

#[test]
fn change_to_tracking_and_back_updates_transactions() {
    let (conn, user) = database_with_user();
    import(&conn, &user, &fixture(PRIVATE), "statement.xml").unwrap();
    import(&conn, &user, &fixture(SAVINGS), "savings.xml").unwrap();
    let all = all_accounts(&conn, &user);
    let private = all.iter().find(|a| a.iban.as_deref() == Some(PRIVATE_IBAN)).unwrap().clone();
    let savings = all.iter().find(|a| a.iban.as_deref() == Some(SAVINGS_IBAN)).unwrap().clone();
    let income: i64 = scalar(
        &conn,
        &format!("SELECT id FROM categories WHERE user_id = {} AND kind = 'income' ORDER BY id", user.id),
    );
    let interest = txs(&conn, savings.id)
        .into_iter()
        .find(|t| t.counterparty_iban.is_none() && t.amount_minor == 250)
        .unwrap();
    categorise::set_category(&conn, &interest, Some(income)).unwrap();

    let transfer_flags = || {
        let to_savings = txs(&conn, private.id)
            .into_iter()
            .find(|t| t.counterparty_iban.as_deref() == Some(SAVINGS_IBAN))
            .unwrap();
        let from_private = txs(&conn, savings.id)
            .into_iter()
            .find(|t| t.counterparty_iban.as_deref() == Some(PRIVATE_IBAN))
            .unwrap();
        (to_savings.is_transfer, from_private.is_transfer)
    };
    assert_eq!(transfer_flags(), (true, true));

    // Savings becomes a 3a account: a tracking account.
    let savings =
        accounts::update_account(&conn, &savings, "3a", AccountType::Pillar3a, "CHF", None).unwrap();
    assert!(!savings.on_budget);
    assert!(txs(&conn, savings.id).iter().all(|t| t.category_id.is_none()));
    // The money now leaves the budget: the movement needs a category.
    assert_eq!(transfer_flags(), (false, false));

    // Back to a budget account.
    accounts::update_account(&conn, &savings, "Savings", AccountType::Pillar3a, "CHF", Some(true)).unwrap();
    assert!(accounts::get_account(&conn, user.id, savings.id).unwrap().on_budget);
    assert_eq!(transfer_flags(), (true, true));
    let start =
        txs(&conn, savings.id).into_iter().find(|t| t.source == TransactionSource::StartingBalance).unwrap();
    assert_eq!(start.category_id, Some(income));
}

#[test]
fn starting_balance_goes_to_ready_to_assign() {
    let (conn, user) = database_with_user();
    let mut new = NewAccount::new("Cash", AccountType::Cash, "CHF");
    new.starting_balance_minor = 5000;
    new.starting_date = Some(date(2026, 1, 1));
    let account = accounts::create_account(&conn, user.id, &new).unwrap();
    let [start] = txs(&conn, account.id).try_into().unwrap();
    assert_eq!(start.source, TransactionSource::StartingBalance);
    assert!(start.category_id.is_some());
}

#[test]
fn web_import_adds_account_and_edit_page() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let text = http.import_fixture(&token, PRIVATE, "");
    assert!(text.contains("The import added the account"));
    let start = text.find(r#"href="/accounts/"#).unwrap() + r#"href="/accounts/"#.len();
    let account_id: String = text[start..].chars().take_while(char::is_ascii_digit).collect();
    let page = http.get(&format!("/accounts/{account_id}/edit"));
    assert_eq!(page.status, 200);
    assert!(page.text().contains("Account CH89 0023 5000 0123 4567 8"));
    let response = http.post(
        &format!("/accounts/{account_id}"),
        &[
            ("csrf_token", &token),
            ("name", "UBS private"),
            ("account_type", "checking"),
            ("budget_mode", "tracking"),
        ],
    );
    assert_eq!(response.status, 200);
    assert!(response.text().contains("The account &#39;UBS private&#39; is changed."));
    assert!(response.text().contains("Tracking"));
    // An error goes back to the edit page.
    let response = http.post(
        &format!("/accounts/{account_id}"),
        &[("csrf_token", &token), ("name", " "), ("account_type", "checking")],
    );
    assert!(response.text().contains("1 to 100 characters"));
    assert_eq!(http.get("/accounts/99999/edit").status, 404);
    // The CSRF token is necessary.
    let response =
        http.post(&format!("/accounts/{account_id}"), &[("name", "x"), ("account_type", "checking")]);
    assert!([400, 403].contains(&response.status));
    // Close and open the account again.
    let response =
        http.post(&format!("/accounts/{account_id}/close"), &[("csrf_token", &token), ("closed", "true")]);
    assert!(response.text().contains("is closed."));
    let response =
        http.post(&format!("/accounts/{account_id}/close"), &[("csrf_token", &token), ("closed", "false")]);
    assert!(response.text().contains("is open."));
}
