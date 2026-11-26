//! Transfers from two matching transactions on two accounts.

mod common;

use common::*;
use feljosit::models::{AccountType, MatchType, RuleField};
use feljosit::services::accounts::{self, NewAccount};
use feljosit::services::categorise::{self, RuleValues};
use feljosit::services::transfers;

fn checking(
    conn: &rusqlite::Connection,
    user: &feljosit::models::User,
    name: &str,
) -> feljosit::models::Account {
    plain_account(conn, user, name, AccountType::Checking, "CHF")
}

#[test]
fn matching_pair_is_a_transfer() {
    let (conn, user) = database_with_user();
    let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
    let sent = manual_tx(&conn, &private, date(2026, 10, 30), -45000, None);
    let received = manual_tx(&conn, &card, date(2026, 11, 2), 45000, None); // 3 days later: a weekend.
    let other = manual_tx(&conn, &card, date(2026, 11, 2), 12000, None);
    assert_eq!(transfers::match_transfers(&conn, user.id, None, None).unwrap(), vec![(sent, received)]);
    assert!(is_transfer(&conn, sent) && is_transfer(&conn, received));
    assert!(!is_transfer(&conn, other));
}

#[test]
fn no_transfer() {
    for case in
        ["too_far", "same_account", "budget_and_tracking", "categorised", "other_currency", "same_sign"]
    {
        let (conn, user) = database_with_user();
        let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
        let on = date(2026, 10, 1);
        match case {
            "too_far" => {
                manual_tx(&conn, &private, on, -1000, None);
                manual_tx(&conn, &card, date(2026, 10, 6), 1000, None);
            }
            "same_account" => {
                manual_tx(&conn, &private, on, -1000, None);
                manual_tx(&conn, &private, on, 1000, None);
            }
            "budget_and_tracking" => {
                let pillar = plain_account(&conn, &user, "3a", AccountType::Pillar3a, "CHF");
                manual_tx(&conn, &private, on, -1000, None);
                manual_tx(&conn, &pillar, on, 1000, None);
            }
            "categorised" => {
                manual_tx(&conn, &private, on, -1000, Some(category_id(&conn, &user, "Groceries")));
                manual_tx(&conn, &card, on, 1000, None);
            }
            "other_currency" => {
                let eur = plain_account(&conn, &user, "EUR", AccountType::Checking, "EUR");
                let usd = plain_account(&conn, &user, "USD", AccountType::Checking, "USD");
                manual_tx(&conn, &eur, on, -1000, None);
                manual_tx(&conn, &usd, on, 1000, None);
            }
            _ => {
                manual_tx(&conn, &private, on, -1000, None);
                manual_tx(&conn, &card, on, -1000, None);
            }
        }
        assert!(transfers::match_transfers(&conn, user.id, None, None).unwrap().is_empty(), "{case}");
        assert_eq!(
            scalar::<i64>(&conn, "SELECT count(*) FROM transactions WHERE is_transfer = 1"),
            0,
            "{case}"
        );
    }
}

#[test]
fn ambiguous_candidates_are_not_matched() {
    let (conn, user) = database_with_user();
    let [private, card, savings] = ["Private", "Card", "Savings"].map(|n| checking(&conn, &user, n));
    manual_tx(&conn, &private, date(2026, 10, 10), -5000, None);
    // Two inflows with the same distance: it is not clear which one is the transfer.
    manual_tx(&conn, &card, date(2026, 10, 11), 5000, None);
    manual_tx(&conn, &savings, date(2026, 10, 9), 5000, None);
    assert!(transfers::match_transfers(&conn, user.id, None, None).unwrap().is_empty());
}

#[test]
fn competing_outflows_are_not_matched() {
    let (conn, user) = database_with_user();
    let [private, card, savings] = ["Private", "Card", "Savings"].map(|n| checking(&conn, &user, n));
    manual_tx(&conn, &private, date(2026, 10, 10), -5000, None);
    manual_tx(&conn, &savings, date(2026, 10, 12), -5000, None);
    manual_tx(&conn, &card, date(2026, 10, 11), 5000, None); // The same distance to both outflows.
    assert!(transfers::match_transfers(&conn, user.id, None, None).unwrap().is_empty());
}

#[test]
fn nearest_candidate_wins() {
    let (conn, user) = database_with_user();
    let [private, card, savings] = ["Private", "Card", "Savings"].map(|n| checking(&conn, &user, n));
    let sent = manual_tx(&conn, &private, date(2026, 10, 10), -5000, None);
    let near = manual_tx(&conn, &card, date(2026, 10, 11), 5000, None);
    let far = manual_tx(&conn, &savings, date(2026, 10, 13), 5000, None);
    assert_eq!(transfers::match_transfers(&conn, user.id, None, None).unwrap(), vec![(sent, near)]);
    assert!(!is_transfer(&conn, far));
}

#[test]
fn starting_balances_are_not_matched() {
    let (conn, user) = database_with_user();
    let mut new = NewAccount::new("Private", AccountType::Checking, "CHF");
    new.starting_balance_minor = 100000;
    new.starting_date = Some(date(2026, 1, 1));
    accounts::create_account(&conn, user.id, &new).unwrap();
    let card = checking(&conn, &user, "Card");
    manual_tx(&conn, &card, date(2026, 1, 1), -100000, None);
    assert!(transfers::match_transfers(&conn, user.id, None, None).unwrap().is_empty());
}

#[test]
fn date_range() {
    let (conn, user) = database_with_user();
    let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
    manual_tx(&conn, &private, date(2026, 1, 10), -1000, None);
    manual_tx(&conn, &card, date(2026, 1, 10), 1000, None);
    let june = (Some(date(2026, 6, 1)), Some(date(2026, 6, 30)));
    assert!(transfers::match_transfers(&conn, user.id, june.0, june.1).unwrap().is_empty());
    // A partner can be some days before the range.
    let sent = manual_tx(&conn, &private, date(2026, 5, 30), -2000, None);
    let received = manual_tx(&conn, &card, date(2026, 6, 1), 2000, None);
    assert_eq!(transfers::match_transfers(&conn, user.id, june.0, june.1).unwrap(), vec![(sent, received)]);
}

#[test]
fn apply_rules_finds_transfers() {
    let (conn, user) = database_with_user();
    let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
    let sent = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
    manual_tx(&conn, &card, date(2026, 10, 2), 3000, None);
    assert_eq!(categorise::apply_to_uncategorised(&conn, user.id).unwrap(), 2);
    assert!(is_transfer(&conn, sent));
}

#[test]
fn import_matches_before_rules() {
    // A payment to an account without IBAN in the statement is a transfer. The matching
    // applies before the rules: the rule does not give a category.
    let (conn, user) = database_with_user();
    add_account(&conn, &user, "Private", AccountType::Checking, PRIVATE_IBAN);
    let card = checking(&conn, &user, "Card");
    // The statement has a payment of 412.35 on 05.10.2026.
    let received = manual_tx(&conn, &card, date(2026, 10, 6), 41235, None);
    let rule = RuleValues {
        field: RuleField::Counterparty,
        match_type: MatchType::Contains,
        pattern: "Krankenkasse",
        category_id: category_id(&conn, &user, "Health insurance (KVG/VVG)"),
        priority: 100,
    };
    categorise::create_rule(&conn, user.id, &rule).unwrap();
    let result = import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    let [stmt] = result.statements.as_slice() else { panic!() };
    assert_eq!(stmt.matched_transfer_count, 1);
    assert_eq!(stmt.transfer_count, 1);
    let payment: i64 =
        scalar(&conn, "SELECT id FROM transactions WHERE counterparty = 'Krankenkasse Beispiel'");
    let payment = tx(&conn, payment);
    assert!(payment.is_transfer && payment.category_id.is_none());
    assert!(is_transfer(&conn, received));
    // The other transactions get categories as before.
    assert_eq!(stmt.categorised_count, 0); // No history and no other rule.
}

#[test]
fn match_records_the_pair() {
    let (conn, user) = database_with_user();
    let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
    let sent = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
    let received = manual_tx(&conn, &card, date(2026, 10, 2), 3000, None);
    transfers::match_transfers(&conn, user.id, None, None).unwrap();
    assert_eq!(pairs(&conn), vec![(sent, received, "match".into(), "active".into())]);
    let found = transfers::partners(&conn, &[sent, received]).unwrap();
    assert_eq!(found[&sent].tx.id, received);
    assert_eq!(found[&received].tx.id, sent);
}

#[test]
fn not_a_transfer_changes_both_and_is_remembered() {
    for category in [None, Some("Groceries")] {
        let (conn, user) = database_with_user();
        let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
        let sent = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
        let received = manual_tx(&conn, &card, date(2026, 10, 2), 3000, None);
        transfers::match_transfers(&conn, user.id, None, None).unwrap();
        let category = category.map(|c| category_id(&conn, &user, c));
        let partner = categorise::set_category(&conn, &tx(&conn, sent), category).unwrap();
        assert_eq!(partner.map(|p| p.id), Some(received));
        assert!(!is_transfer(&conn, sent) && !is_transfer(&conn, received));
        assert_eq!(tx(&conn, sent).category_id, category);
        assert_eq!(tx(&conn, received).category_id, None);
        assert_eq!(pairs(&conn)[0].3, "rejected");
        // The matching does not make the rejected pair again.
        if category.is_none() {
            assert!(transfers::match_transfers(&conn, user.id, None, None).unwrap().is_empty());
            assert_eq!(categorise::apply_to_uncategorised(&conn, user.id).unwrap(), 0);
        }
        assert!(!is_transfer(&conn, received));
        assert_eq!(pairs(&conn).len(), 1);
    }
}

#[test]
fn rejected_pair_allows_a_different_partner() {
    let (conn, user) = database_with_user();
    let [private, card, savings] = ["Private", "Card", "Savings"].map(|n| checking(&conn, &user, n));
    let sent = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
    let near = manual_tx(&conn, &card, date(2026, 10, 2), 3000, None);
    let far = manual_tx(&conn, &savings, date(2026, 10, 4), 3000, None);
    assert_eq!(transfers::match_transfers(&conn, user.id, None, None).unwrap(), vec![(sent, near)]);
    categorise::set_category(&conn, &tx(&conn, near), None).unwrap();
    assert_eq!(transfers::match_transfers(&conn, user.id, None, None).unwrap(), vec![(sent, far)]);
}

#[test]
fn uncategorised_ends_a_transfer_without_pair() {
    let (conn, user) = database_with_user();
    let private = checking(&conn, &user, "Private");
    let id = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
    categorise::mark_transfer(&conn, id).unwrap(); // Selected manually as "Transfer".
    assert!(categorise::set_category(&conn, &tx(&conn, id), None).unwrap().is_none());
    assert!(!is_transfer(&conn, id));
}

#[test]
fn delete_ends_a_matched_pair() {
    let (conn, user) = database_with_user();
    let (private, card) = (checking(&conn, &user, "Private"), checking(&conn, &user, "Card"));
    let sent = manual_tx(&conn, &private, date(2026, 10, 1), -3000, None);
    let received = manual_tx(&conn, &card, date(2026, 10, 2), 3000, None);
    transfers::match_transfers(&conn, user.id, None, None).unwrap();
    transfers::before_delete(&conn, sent).unwrap();
    conn.execute("DELETE FROM transactions WHERE id = ?1", [sent]).unwrap();
    assert!(!is_transfer(&conn, received));
    assert!(pairs(&conn).is_empty());
}

#[test]
fn import_records_iban_pairs() {
    let (conn, user) = database_with_user();
    let private = add_account(&conn, &user, "Private", AccountType::Checking, PRIVATE_IBAN);
    let savings = add_account(&conn, &user, "Savings", AccountType::Checking, SAVINGS_IBAN);
    import_fixture(&conn, &user, "camt053_v04_private.xml", None).unwrap();
    // Only one side is imported: a transfer without a pair.
    assert!(pairs(&conn).is_empty());
    import_fixture(&conn, &user, "camt053_v08_savings.xml", None).unwrap();
    let [(outflow, inflow, method, _)] = pairs(&conn).try_into().unwrap();
    assert_eq!(method, "iban");
    let (outflow, inflow) = (tx(&conn, outflow), tx(&conn, inflow));
    assert_eq!((outflow.account_id, inflow.account_id), (private, savings));
    assert_eq!((outflow.amount_minor, inflow.amount_minor), (-50000, 50000));
    // "Not a transfer" also works for a pair from the IBAN.
    categorise::set_category(&conn, &outflow, None).unwrap();
    assert!(!is_transfer(&conn, inflow.id));
}

#[test]
fn transaction_list_shows_partner_and_filters() {
    let test = TestApp::new();
    let (mut http, token) = test.anna();
    let received = test.app.with_db(|conn| {
        let user = feljosit::models::User::by_username(conn, "anna").unwrap().unwrap();
        let (private, card) = (checking(conn, &user, "Private"), checking(conn, &user, "Card"));
        manual_tx(conn, &private, date(2026, 10, 1), -3000, None);
        let received = manual_tx(conn, &card, date(2026, 10, 2), 3000, None);
        manual_tx(conn, &private, date(2026, 10, 3), -999, None);
        transfers::match_transfers(conn, user.id, None, None).unwrap();
        received
    });
    let page = http.get("/transactions").text();
    assert!(
        page.contains('⇄') && page.contains("Card, 02.10.2026") && page.contains("found from the amount"),
        "{page}"
    );
    assert!(http.get("/transactions?matched=true").text().contains("2 transactions."));
    // "Uncategorised" on one side ends the pair and tells the user.
    let response = http.post(
        &format!("/transactions/{received}/category"),
        &[("csrf_token", &token), ("category_id", ""), ("return_to", "/transactions?matched=true")],
    );
    assert_eq!(response.status, 200);
    assert!(response.text().contains("are not a transfer any more"));
    assert!(response.text().contains("0 transactions.")); // The filter stays.
}
