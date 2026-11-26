//! The transaction list: filters, sort orders, account numbers and the bulk change.

mod common;

use std::collections::{BTreeSet, HashMap};

use common::*;
use feljosit::models::{AccountType, User};
use feljosit::services::accounts::{self, ManualTransaction, NewAccount};
use feljosit::services::{categorise, transfers};

fn names_in(page: &str, candidates: &[&str]) -> BTreeSet<String> {
    candidates
        .iter()
        .filter(|n| page.contains(&format!("<strong>{n}</strong>")))
        .map(|n| n.to_string())
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn add(
    conn: &rusqlite::Connection,
    account: &feljosit::models::Account,
    day: u32,
    amount: i64,
    name: &str,
    category: Option<i64>,
) -> i64 {
    let manual = ManualTransaction {
        booking_date: date(2026, 10, day),
        amount_minor: amount,
        counterparty: name,
        memo: "",
        category_id: category,
    };
    accounts::add_manual_transaction(conn, account, &manual).unwrap()
}

// The category filter, with the transfer choices.

const FILTER_NAMES: &[&str] =
    &["Card payment", "Payment received", "Manual transfer", "Migros", "Kiosk", "Deposit"];

/// Transactions of each kind. Return the IDs that the tests need.
fn filter_data(test: &TestApp) -> HashMap<&'static str, i64> {
    test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        let private = plain_account(conn, &user, "Private", AccountType::Checking, "CHF");
        let card = plain_account(conn, &user, "Card", AccountType::Checking, "CHF");
        let pillar = plain_account(conn, &user, "3a", AccountType::Pillar3a, "CHF");
        let groceries = category_id(conn, &user, "Groceries");
        add(conn, &private, 1, -3000, "Card payment", None); // A matched pair with the next one.
        add(conn, &card, 2, 3000, "Payment received", None);
        let manual = add(conn, &private, 3, -1000, "Manual transfer", None);
        categorise::mark_transfer(conn, manual).unwrap(); // Selected manually as "Transfer": no pair.
        add(conn, &private, 4, -4500, "Migros", Some(groceries));
        add(conn, &private, 5, -700, "Kiosk", None); // Uncategorised.
        add(conn, &pillar, 6, 50000, "Deposit", None); // Tracking account: not "uncategorised".
        transfers::match_transfers(conn, user.id, None, None).unwrap();
        HashMap::from([("groceries", groceries), ("private", private.id)])
    })
}

#[test]
fn category_filter() {
    let all: &[&str] = FILTER_NAMES;
    for (category, expected) in [
        ("", all),
        ("uncategorised", &["Kiosk"][..]),
        ("transfers", &["Card payment", "Payment received", "Manual transfer"][..]),
        ("matched", &["Card payment", "Payment received"][..]),
        ("unmatched", &["Manual transfer"][..]),
        ("abc", all),
    ] {
        let test = TestApp::new();
        let (mut http, _) = test.anna();
        filter_data(&test);
        let page = http.get(&format!("/transactions?category={category}"));
        assert_eq!(page.status, 200);
        assert_eq!(names_in(&page.text(), FILTER_NAMES), set(expected), "{category}");
        if ["uncategorised", "transfers", "matched", "unmatched"].contains(&category) {
            assert!(page.text().contains(&format!(r#"<option value="{category}" selected>"#)));
        }
    }
}

#[test]
fn filter_by_category_id_and_account() {
    let test = TestApp::new();
    let (mut http, _) = test.anna();
    let data = filter_data(&test);
    let page = http.get(&format!("/transactions?category={}", data["groceries"])).text();
    assert_eq!(names_in(&page, FILTER_NAMES), set(&["Migros"]));
    assert!(page.contains(&format!(r#"<option value="{}" selected>"#, data["groceries"])));
    let page = http.get(&format!("/transactions?category=transfers&account={}", data["private"])).text();
    assert_eq!(names_in(&page, FILTER_NAMES), set(&["Card payment", "Manual transfer"]));
}

#[test]
fn old_links_still_work() {
    // The budget page, the import result and the reports link with the old parameters.
    let test = TestApp::new();
    let (mut http, _) = test.anna();
    filter_data(&test);
    for value in ["uncategorised", "matched"] {
        let page = http.get(&format!("/transactions?{value}=true")).text();
        assert!(page.contains(&format!(r#"<option value="{value}" selected>"#)));
        // The links of the page use the new parameter.
        assert!(
            page.contains(&format!(r#"name="return_to" value="/transactions?category={value}&amp;page=1""#))
        );
    }
}

#[test]
fn filter_stays_after_a_change() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let data = filter_data(&test);
    let kiosk: i64 = test.scalar("SELECT id FROM transactions WHERE counterparty = 'Kiosk'");
    let response = http.post_no_follow(
        &format!("/transactions/{kiosk}/category"),
        &[
            ("csrf_token", &token),
            ("category_id", &data["groceries"].to_string()),
            ("return_to", "/transactions?category=uncategorised&page=1"),
        ],
    );
    assert_eq!(response.location(), "/transactions?category=uncategorised&page=1");
    assert!(http.get(response.location()).text().contains("0 transactions."));
}

// The sort order.

/// Transactions in CHF and in JPY (no decimal places).
fn sort_data(test: &TestApp) {
    test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        let account = |name: &str, currency: &str| {
            let mut new = NewAccount::new(name, AccountType::Checking, currency);
            new.budget_currency = currency;
            accounts::create_account(conn, user.id, &new).unwrap()
        };
        let (chf, jpy) = (account("Private", "CHF"), account("Yen", "JPY"));
        for (acc, day, amount, name) in [
            (&chf, 1, -4500, "Migros"),  // −45.00
            (&chf, 2, 250000, "Salary"), // 2'500.00
            (&chf, 3, -120000, "Rent"),  // −1'200.00
            (&jpy, 4, -3000, "Ramen"),   // −3'000 JPY
            (&chf, 5, -700, "Kiosk"),    // −7.00
        ] {
            add(conn, acc, day, amount, name, None);
        }
    });
}

fn order(page: &str) -> Vec<String> {
    page.split(r#"<td class="tx-party">"#)
        .skip(1)
        .map(|part| {
            let start = part.find("<strong>").unwrap() + "<strong>".len();
            part[start..part.find("</strong>").unwrap()].to_string()
        })
        .collect()
}

#[test]
fn sort() {
    for (sort, expected) in [
        ("", ["Kiosk", "Ramen", "Rent", "Salary", "Migros"]),
        ("amount_asc", ["Ramen", "Rent", "Migros", "Kiosk", "Salary"]),
        ("amount_desc", ["Salary", "Kiosk", "Migros", "Rent", "Ramen"]),
        ("size_desc", ["Ramen", "Salary", "Rent", "Migros", "Kiosk"]),
        ("unknown", ["Kiosk", "Ramen", "Rent", "Salary", "Migros"]),
    ] {
        let test = TestApp::new();
        let (mut http, _) = test.anna();
        sort_data(&test);
        let page = http.get(&format!("/transactions?sort={sort}"));
        assert_eq!(page.status, 200);
        assert_eq!(order(&page.text()), expected, "{sort}");
        let selected = if sort == "unknown" { "" } else { sort };
        assert!(page.text().contains(&format!(r#"<option value="{selected}" selected>"#)));
    }
}

#[test]
fn sort_stays_with_filters_and_changes() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    sort_data(&test);
    let page = http.get("/transactions?sort=amount_asc&direction=outflow").text();
    assert_eq!(order(&page), ["Ramen", "Rent", "Migros", "Kiosk"]);
    assert!(page.contains(r#"value="/transactions?direction=outflow&amp;sort=amount_asc&amp;page=1""#));
    // "Remove the filters" keeps the sort order.
    assert!(page.contains(r#"<a href="/transactions?sort=amount_asc">Remove the filters</a>"#));
    let response = http.post_no_follow(
        "/transactions/1/memo",
        &[("csrf_token", &token), ("memo", "x"), ("return_to", "/transactions?sort=size_desc")],
    );
    assert_eq!(response.location(), "/transactions?sort=size_desc");
}

// The account number of the counterparty.

const MUSTER_IBAN: &str = "CH93 0076 2011 6238 5295 7";

fn import_private(http: &mut Client, token: &str) {
    assert!(http.import_fixture(token, "camt053_v04_private.xml", "").contains("7 new transactions"));
}

#[test]
fn account_numbers_are_hidden_by_default() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    import_private(&mut http, &token);
    let page = http.get("/transactions").text();
    assert!(page.contains("Muster AG"));
    assert!(!page.contains(MUSTER_IBAN));
    assert!(page.contains(r#"name="iban" value="true""#));
    assert!(!page.contains(r#"name="iban" value="true" checked"#));
}

#[test]
fn show_account_numbers() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    import_private(&mut http, &token);
    let page = http.get("/transactions?iban=true").text();
    assert!(page.contains(&format!("IBAN {MUSTER_IBAN}")));
    assert!(page.contains(r#"name="iban" value="true" checked"#));
    // The option is not a filter: there is no "Remove the filters" link.
    assert!(!page.contains("Remove the filters"));
    // With a filter, the link removes the filter and keeps the option.
    let page = http.get("/transactions?iban=true&q=muster").text();
    assert!(page.contains(r#"href="/transactions?iban=true">Remove the filters"#));
    // The forms of each row go back to the list with the option.
    assert!(page.contains(r#"name="return_to" value="/transactions?q=muster&amp;iban=true&amp;page=1""#));
}

#[test]
fn option_stays_after_a_change() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    import_private(&mut http, &token);
    let tx: i64 = test.scalar("SELECT id FROM transactions WHERE counterparty = 'Muster AG'");
    let response = http.post_no_follow(
        &format!("/transactions/{tx}/memo"),
        &[("csrf_token", &token), ("memo", "Bonus"), ("return_to", "/transactions?iban=true&page=1")],
    );
    assert_eq!(response.status, 303);
    assert_eq!(response.location(), "/transactions?iban=true&page=1");
    assert_eq!(test.scalar::<String>(&format!("SELECT memo FROM transactions WHERE id = {tx}")), "Bonus");
}

#[test]
fn search_for_an_account_number() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    import_private(&mut http, &token);
    for text in ["CH93+0076", "ch9300762011", "2011+6238"] {
        let page = http.get(&format!("/transactions?q={text}")).text();
        assert!(page.contains("1 transactions."), "{text}");
        assert!(page.contains("Muster AG"));
    }
    // A short text or a text without a digit does not search the IBANs. Else "ch" finds
    // all transactions with a Swiss IBAN.
    assert!(!http.get("/transactions?q=ch").text().contains("Krankenkasse Beispiel"));
    // Special characters of LIKE are not wildcards.
    assert!(http.get("/transactions?q=CH9%25").text().contains("0 transactions."));
}

// The amount and direction filters and the bulk category change.

const NAMES: &[&str] =
    &["Premium", "Deductible", "Small bill", "Refund", "Card payment", "Payment", "Deposit"];

/// Transactions of a health insurance and other transactions. Return the IDs.
fn bulk_data(test: &TestApp) -> HashMap<&'static str, i64> {
    test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        let private = plain_account(conn, &user, "Private", AccountType::Checking, "CHF");
        let card = plain_account(conn, &user, "Card", AccountType::Checking, "CHF");
        let pillar = plain_account(conn, &user, "3a", AccountType::Pillar3a, "CHF");
        let mut ids = HashMap::from([
            ("premium", add(conn, &private, 1, -45_050, "Premium", None)),
            ("deductible", add(conn, &private, 2, -30_000, "Deductible", None)),
            ("small", add(conn, &private, 3, -2_995, "Small bill", None)),
            ("refund", add(conn, &private, 4, 12_000, "Refund", None)),
            ("card_payment", add(conn, &private, 5, -77_700, "Card payment", None)), // A pair with "Payment".
            ("payment", add(conn, &card, 6, 77_700, "Payment", None)),
            ("deposit", add(conn, &pillar, 7, 50_000, "Deposit", None)), // Tracking account.
        ]);
        transfers::match_transfers(conn, user.id, None, None).unwrap();
        ids.insert(
            "category",
            scalar(conn, &format!("SELECT id FROM categories WHERE user_id = {} ORDER BY id", user.id)),
        );
        ids
    })
}

fn state(test: &TestApp, tx_id: i64) -> (Option<i64>, bool) {
    test.app.with_db(|conn| {
        let t = tx(conn, tx_id);
        (t.category_id, t.is_transfer)
    })
}

fn bulk_setup() -> (TestApp, Client, String, HashMap<&'static str, i64>) {
    let test = TestApp::new();
    let (http, token) = test.anna();
    let data = bulk_data(&test);
    (test, http, token, data)
}

#[test]
fn amount_and_direction_filter() {
    let (_test, mut http, _, _) = bulk_setup();
    for (query, expected) in [
        ("direction=outflow", &["Premium", "Deductible", "Small bill", "Card payment"][..]),
        ("direction=inflow", &["Refund", "Payment", "Deposit"][..]),
        ("direction=abc", NAMES),
        ("min=300", &["Premium", "Deductible", "Card payment", "Payment", "Deposit"][..]),
        ("min=300.01", &["Premium", "Card payment", "Payment", "Deposit"][..]),
        ("max=300", &["Deductible", "Small bill", "Refund"][..]),
        ("min=100&max=500&direction=outflow", &["Premium", "Deductible"][..]),
        ("max=1%27000&direction=inflow", &["Refund", "Payment", "Deposit"][..]),
        ("min=-300&direction=outflow", &["Premium", "Deductible", "Card payment"][..]),
        ("min=29%2C95&max=29.95", &["Small bill"][..]),
    ] {
        let page = http.get(&format!("/transactions?{query}"));
        assert_eq!(page.status, 200);
        assert_eq!(names_in(&page.text(), NAMES), set(expected), "{query}");
    }
}

#[test]
fn invalid_amount_filter_shows_an_error() {
    let (_test, mut http, _, _) = bulk_setup();
    let page = http.get("/transactions?min=abc").text();
    assert!(page.contains("The amount filter is not valid."));
    assert_eq!(names_in(&page, NAMES), set(NAMES));
    let page = http.get("/transactions?min=500&max=100").text();
    assert!(page.contains("The minimum amount is more than the maximum amount."));
    assert!(names_in(&page, NAMES).is_empty());
}

#[test]
fn filters_stay_in_the_links() {
    let (_test, mut http, _, _) = bulk_setup();
    let page = http.get("/transactions?direction=outflow&min=300").text();
    assert!(page.contains(r#"<option value="outflow" selected>"#));
    assert!(page.contains(r#"name="min" value="300""#));
    assert!(
        page.contains(r#"name="return_to" value="/transactions?direction=outflow&amp;min=300&amp;page=1""#)
    );
}

fn bulk(http: &mut Client, token: &str, ids: &[i64], category: &str) -> String {
    let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
    let mut form = vec![
        ("csrf_token", token),
        ("category_id", category),
        ("return_to", "/transactions?direction=outflow&min=300&evil=1&page=1"),
    ];
    for id in &ids {
        form.push(("tx_id", id));
    }
    let response = http.post_no_follow("/transactions/bulk-category", &form);
    assert_eq!(response.status, 303);
    assert_eq!(response.location(), "/transactions?direction=outflow&min=300&page=1");
    http.get(response.location()).text()
}

#[test]
fn bulk_category() {
    let (test, mut http, token, data) = bulk_setup();
    let category = data["category"].to_string();
    let page = bulk(&mut http, &token, &[data["premium"], data["deductible"]], &category);
    assert!(page.contains("The category of 2 transactions is changed."));
    assert_eq!(state(&test, data["premium"]), (Some(data["category"]), false));
    assert_eq!(state(&test, data["deductible"]), (Some(data["category"]), false));
    assert_eq!(state(&test, data["small"]), (None, false));
    let page = bulk(&mut http, &token, &[data["deductible"]], "none");
    assert!(page.contains("The category of 1 transactions is changed."));
    assert_eq!(state(&test, data["deductible"]), (None, false));
    bulk(&mut http, &token, &[data["small"]], "transfer");
    assert_eq!(state(&test, data["small"]), (None, true));
}

#[test]
fn bulk_category_ends_a_transfer_pair() {
    let (test, mut http, token, data) = bulk_setup();
    assert_eq!(state(&test, data["payment"]), (None, true));
    let page = bulk(&mut http, &token, &[data["card_payment"]], &data["category"].to_string());
    assert!(page.contains("1 other transactions are not transfers any more."));
    assert_eq!(state(&test, data["card_payment"]), (Some(data["category"]), false));
    assert_eq!(state(&test, data["payment"]), (None, false));
}

#[test]
fn bulk_category_with_both_sides_of_a_pair() {
    let (test, mut http, token, data) = bulk_setup();
    let page =
        bulk(&mut http, &token, &[data["card_payment"], data["payment"]], &data["category"].to_string());
    assert!(!page.contains("other transactions"));
    assert_eq!(state(&test, data["card_payment"]), (Some(data["category"]), false));
    assert_eq!(state(&test, data["payment"]), (Some(data["category"]), false));
}

#[test]
fn bulk_category_skips_tracking_accounts() {
    let (test, mut http, token, data) = bulk_setup();
    let page = bulk(&mut http, &token, &[data["premium"], data["deposit"]], &data["category"].to_string());
    assert!(page.contains("The category of 1 transactions is changed."));
    assert!(page.contains("1 transactions are in tracking accounts."));
    assert_eq!(state(&test, data["deposit"]), (None, false));
}

#[test]
fn bulk_category_errors() {
    for (ids, category, message) in [
        (&[][..], "category", "Select one or more transactions."),
        (&["premium"][..], "", "Select a category."),
        (&["premium"][..], "abc", "Select a category."),
    ] {
        let (test, mut http, token, data) = bulk_setup();
        let value = if category == "category" { data["category"].to_string() } else { category.to_string() };
        let ids: Vec<i64> = ids.iter().map(|i| data[i]).collect();
        let page = bulk(&mut http, &token, &ids, &value);
        assert!(page.contains(message), "{message}");
        assert_eq!(state(&test, data["premium"]), (None, false));
    }
}

#[test]
fn bulk_category_of_another_user() {
    // An unknown transaction or category: nothing changes.
    let (test, mut http, token, data) = bulk_setup();
    let premium = data["premium"].to_string();
    let category = data["category"].to_string();
    let response = http.post_no_follow(
        "/transactions/bulk-category",
        &[("csrf_token", &token), ("tx_id", &premium), ("tx_id", "999999"), ("category_id", &category)],
    );
    assert_eq!(response.status, 404);
    let response = http.post_no_follow(
        "/transactions/bulk-category",
        &[("csrf_token", &token), ("tx_id", &premium), ("category_id", "999999")],
    );
    assert_eq!(response.status, 404);
    assert_eq!(state(&test, data["premium"]), (None, false));
}

#[test]
fn bulk_category_needs_the_csrf_token() {
    let (test, mut http, _, data) = bulk_setup();
    let response = http.post_no_follow(
        "/transactions/bulk-category",
        &[("tx_id", &data["premium"].to_string()), ("category_id", &data["category"].to_string())],
    );
    assert_eq!(response.status, 403);
    assert_eq!(state(&test, data["premium"]), (None, false));
}

#[test]
fn page_has_the_bulk_form() {
    let (_test, mut http, _, data) = bulk_setup();
    let page = http.get("/transactions").text();
    assert!(page.contains(r#"action="/transactions/bulk-category""#));
    assert!(page.contains(&format!(r#"name="tx_id" value="{}" form="bulk""#, data["premium"])));
    // A tracking account has no category, and thus no checkbox.
    assert!(!page.contains(&format!(r#"name="tx_id" value="{}""#, data["deposit"])));
}

/// Send "Select all matching transactions" with the filters of the page.
fn bulk_all(
    http: &mut Client,
    token: &str,
    filters: &[(&str, &str)],
    expected: i64,
    category: &str,
) -> String {
    let expected = expected.to_string();
    let mut form = vec![
        ("csrf_token", token),
        ("scope", "all"),
        ("expected", expected.as_str()),
        ("category_id", category),
        ("return_to", "/transactions?page=1"),
    ];
    form.extend_from_slice(filters);
    let response = http.post_no_follow("/transactions/bulk-category", &form);
    assert_eq!(response.status, 303);
    http.get(response.location()).text()
}

#[test]
fn page_offers_all_matching_transactions() {
    let (_test, mut http, _, _) = bulk_setup();
    // 7 transactions. The deposit is in a tracking account.
    assert!(http.get("/transactions").text().contains("Select all 6 matching transactions"));
    let page = http.get("/transactions?direction=outflow&min=300").text();
    assert!(page.contains("Select all 3 matching transactions"));
    assert!(page.contains(r#"<input type="hidden" name="expected" value="3">"#));
    assert!(page.contains(r#"<input type="hidden" name="direction" value="outflow">"#));
    assert!(page.contains(r#"<input type="hidden" name="min" value="300">"#));
    assert!(!http.get("/transactions?q=Deposit").text().contains("matching transactions"));
}

#[test]
fn bulk_category_of_all_matching_transactions() {
    // The scope "all" ignores the checkboxes of the rows.
    let (test, mut http, token, data) = bulk_setup();
    let small = data["small"].to_string();
    let page = bulk_all(
        &mut http,
        &token,
        &[("direction", "outflow"), ("min", "300"), ("tx_id", &small)],
        3,
        &data["category"].to_string(),
    );
    assert!(page.contains("The category of 3 transactions is changed."));
    assert!(page.contains("1 other transactions are not transfers any more."));
    for name in ["premium", "deductible", "card_payment"] {
        assert_eq!(state(&test, data[name]), (Some(data["category"]), false), "{name}");
    }
    assert_eq!(state(&test, data["small"]), (None, false));
    assert_eq!(state(&test, data["payment"]), (None, false));
}

#[test]
fn bulk_category_of_all_skips_tracking_accounts() {
    let (test, mut http, token, data) = bulk_setup();
    let page = bulk_all(&mut http, &token, &[], 6, "none");
    assert!(page.contains("The category of 6 transactions is changed."));
    assert!(!page.contains("tracking accounts"));
    assert_eq!(state(&test, data["deposit"]), (None, false));
}

#[test]
fn bulk_category_of_all_errors() {
    for (filters, expected, message) in [
        (&[("direction", "outflow"), ("min", "300")][..], 2, "The matching transactions changed"),
        (&[("direction", "outflow"), ("min", "300")][..], 4, "The matching transactions changed"),
        (&[("min", "abc")][..], 6, "The amount filter is not valid."),
        (&[("min", "500"), ("max", "100")][..], 0, "The minimum amount is more than the maximum"),
        (&[("q", "Deposit")][..], 0, "Select one or more transactions."),
    ] {
        let (test, mut http, token, data) = bulk_setup();
        let page = bulk_all(&mut http, &token, filters, expected, &data["category"].to_string());
        assert!(page.contains(message), "{message}");
        assert_eq!(state(&test, data["premium"]), (None, false));
    }
}

#[test]
fn pages_of_the_list() {
    let test = TestApp::new();
    let (mut http, _) = test.anna();
    test.app.with_db(|conn| {
        let user = User::by_username(conn, "anna").unwrap().unwrap();
        let account = plain_account(conn, &user, "Private", AccountType::Checking, "CHF");
        for i in 0..105 {
            add(conn, &account, 1 + (i % 28) as u32, -100 - i, &format!("Shop {i}"), None);
        }
    });
    let page = http.get("/transactions").text();
    assert!(page.contains("Page 1 of 2"));
    assert!(page.contains("page=2"));
    let second = http.get("/transactions?page=2").text();
    assert!(second.contains("Page 2 of 2"));
    assert_eq!(second.matches(r#"<td class="tx-party">"#).count(), 5);
    assert_eq!(http.get("/transactions?page=0").status, 422);
}
