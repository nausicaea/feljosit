//! Tests of the envelope budget.

mod common;

use common::*;
use feljosit::models::{Account, AccountType};
use feljosit::services::accounts::{self, NewAccount};
use feljosit::services::budget::{self, CategoryRow, MonthBudget};

fn setup() -> (rusqlite::Connection, feljosit::models::User, Account) {
    let (conn, user) = database_with_user();
    let account = plain_account(&conn, &user, "Private", AccountType::Checking, "CHF");
    (conn, user, account)
}

fn jan() -> feljosit::dates::Date {
    date(2026, 1, 1)
}

fn row(data: &MonthBudget, category_id: i64) -> CategoryRow {
    data.groups.iter().flat_map(|g| g.rows.iter()).find(|r| r.category_id == category_id).cloned().unwrap()
}

#[test]
fn income_goes_to_ready_to_assign() {
    let (conn, user, account) = setup();
    manual_tx(&conn, &account, date(2026, 1, 25), 500000, Some(category_id(&conn, &user, "Salary")));
    let data = budget::month_budget(&conn, user.id, jan()).unwrap();
    assert_eq!(data.ready_to_assign, 500000);
    assert_eq!(data.income, 500000);
}

#[test]
fn assign_and_rollover() {
    let (conn, user, account) = setup();
    let (salary, food) = (category_id(&conn, &user, "Salary"), category_id(&conn, &user, "Groceries"));
    manual_tx(&conn, &account, date(2026, 1, 5), 100000, Some(salary));
    budget::assign(&conn, user.id, food, jan(), 60000).unwrap();
    manual_tx(&conn, &account, date(2026, 1, 10), -45000, Some(food));
    let january = budget::month_budget(&conn, user.id, jan()).unwrap();
    assert_eq!(january.ready_to_assign, 40000);
    assert_eq!(row(&january, food).available, 15000);
    assert_eq!(row(&january, food).activity, -45000);
    // The positive available amount rolls over.
    let february = budget::month_budget(&conn, user.id, date(2026, 2, 1)).unwrap();
    assert_eq!(row(&february, food).available, 15000);
    assert_eq!(row(&february, food).assigned, 0);
    assert_eq!(february.ready_to_assign, 40000);
}

#[test]
fn overspending_reduces_next_month() {
    let (conn, user, account) = setup();
    let (salary, food) = (category_id(&conn, &user, "Salary"), category_id(&conn, &user, "Groceries"));
    manual_tx(&conn, &account, date(2026, 1, 5), 100000, Some(salary));
    budget::assign(&conn, user.id, food, jan(), 10000).unwrap();
    manual_tx(&conn, &account, date(2026, 1, 10), -25000, Some(food));
    let january = budget::month_budget(&conn, user.id, jan()).unwrap();
    assert_eq!(row(&january, food).available, -15000);
    assert_eq!(january.ready_to_assign, 90000);
    let february = budget::month_budget(&conn, user.id, date(2026, 2, 1)).unwrap();
    // The negative amount does not roll over in the category ...
    assert_eq!(row(&february, food).available, 0);
    // ... but it reduces Ready to Assign.
    assert_eq!(february.ready_to_assign, 75000);
    assert_eq!(february.overspent_last_month, -15000);
    assert_eq!(budget::month_budget(&conn, user.id, date(2026, 3, 1)).unwrap().ready_to_assign, 75000);
}

#[test]
fn future_assignments_reduce_ready_to_assign() {
    let (conn, user, account) = setup();
    let (salary, food) = (category_id(&conn, &user, "Salary"), category_id(&conn, &user, "Groceries"));
    manual_tx(&conn, &account, date(2026, 1, 5), 100000, Some(salary));
    budget::assign(&conn, user.id, food, date(2026, 3, 1), 30000).unwrap();
    assert_eq!(budget::month_budget(&conn, user.id, jan()).unwrap().ready_to_assign, 70000);
}

#[test]
fn reassign_overwrites() {
    let (conn, user, _) = setup();
    let food = category_id(&conn, &user, "Groceries");
    budget::assign(&conn, user.id, food, jan(), 100).unwrap();
    budget::assign(&conn, user.id, food, jan(), 200).unwrap();
    assert_eq!(row(&budget::month_budget(&conn, user.id, jan()).unwrap(), food).assigned, 200);
}

#[test]
fn cannot_assign_to_income() {
    let (conn, user, _) = setup();
    assert!(budget::assign(&conn, user.id, category_id(&conn, &user, "Salary"), jan(), 100).is_err());
}

#[test]
fn tracking_accounts_and_transfers_do_not_change_budget() {
    let (conn, user) = database_with_user();
    let mut new = NewAccount::new("3a", AccountType::Pillar3a, "CHF");
    new.starting_balance_minor = 5000000;
    new.starting_date = Some(jan());
    let tracking = accounts::create_account(&conn, user.id, &new).unwrap();
    assert!(!tracking.on_budget);
    let data = budget::month_budget(&conn, user.id, jan()).unwrap();
    assert_eq!(data.ready_to_assign, 0);
    assert_eq!(data.uncategorised_count, 0);
}

#[test]
fn uncategorised_count() {
    let (conn, user, account) = setup();
    manual_tx(&conn, &account, date(2026, 1, 5), -100, None);
    assert_eq!(budget::month_budget(&conn, user.id, jan()).unwrap().uncategorised_count, 1);
}

#[test]
fn month_helpers() {
    assert_eq!(budget::add_months(date(2026, 12, 1), 1), date(2027, 1, 1));
    assert_eq!(budget::add_months(date(2026, 1, 1), -1), date(2025, 12, 1));
    assert_eq!(budget::parse_month(Some("2026-07"), jan()).unwrap(), date(2026, 7, 1));
    assert!(budget::parse_month(Some("July"), jan()).is_err());
}
