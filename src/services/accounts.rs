//! Accounts and manual transactions.

use crate::dates::Date;
use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Error, Result};
use crate::db::now_text;
use crate::iban::{is_valid_iban, normalise_iban};
use crate::models::{Account, AccountType, Category, CategoryKind, NewTransaction, TransactionSource};
use crate::money::validate_currency;

pub fn get_account(conn: &Connection, user_id: i64, account_id: i64) -> Result<Account> {
    match Account::get(conn, account_id)? {
        Some(account) if account.user_id == user_id => Ok(account),
        _ => Err(Error::NotFound("account")),
    }
}

pub fn list_accounts(conn: &Connection, user_id: i64, include_closed: bool) -> Result<Vec<Account>> {
    let filter = if include_closed { "" } else { " AND closed = 0" };
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM accounts WHERE user_id = ?1{filter} ORDER BY closed, on_budget DESC, name",
        Account::COLUMNS
    ))?;
    let rows = stmt.query_map([user_id], Account::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The balance of each account of the user, by account ID.
pub fn balances(conn: &Connection, user_id: i64) -> Result<HashMap<i64, i64>> {
    let mut stmt = conn.prepare(
        "SELECT account_id, sum(amount_minor) FROM transactions WHERE user_id = ?1 \
         GROUP BY account_id",
    )?;
    let rows = stmt.query_map([user_id], |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?)))?;
    let mut out = HashMap::new();
    for row in rows {
        let (id, total) = row?;
        out.insert(id, total.unwrap_or(0));
    }
    Ok(out)
}

fn checked_name(name: &str) -> Result<String> {
    let name = name.trim();
    if !(1..=100).contains(&name.chars().count()) {
        return Err(Error::service("The account name must have 1 to 100 characters."));
    }
    Ok(name.to_string())
}

fn budget_currency_error(budget_currency: &str) -> Error {
    Error::service(format!(
        "An account in the budget must use the budget currency ({budget_currency}). \
         Make it a tracking account."
    ))
}

/// The values of a new account.
#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    pub name: &'a str,
    pub account_type: AccountType,
    pub currency: &'a str,
    pub budget_currency: &'a str,
    pub iban: Option<&'a str>,
    /// `None`: the type gives the budget setting.
    pub on_budget: Option<bool>,
    pub starting_balance_minor: i64,
    pub starting_date: Option<Date>,
}

impl<'a> NewAccount<'a> {
    pub fn new(name: &'a str, account_type: AccountType, currency: &'a str) -> NewAccount<'a> {
        NewAccount {
            name,
            account_type,
            currency,
            budget_currency: "CHF",
            iban: None,
            on_budget: None,
            starting_balance_minor: 0,
            starting_date: None,
        }
    }
}

pub fn create_account(conn: &Connection, user_id: i64, new: &NewAccount<'_>) -> Result<Account> {
    let name = checked_name(new.name)?;
    let currency = validate_currency(new.currency)?;
    let on_budget = new.on_budget.unwrap_or(new.account_type.on_budget_default());
    if on_budget && currency != new.budget_currency {
        return Err(budget_currency_error(new.budget_currency));
    }
    let mut clean_iban = None;
    if let Some(iban) = new.iban.filter(|i| !i.trim().is_empty()) {
        let iban = normalise_iban(iban);
        if !is_valid_iban(&iban) {
            return Err(Error::service("The IBAN is not valid."));
        }
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts WHERE user_id = ?1 AND iban = ?2",
                params![user_id, iban],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(Error::service("An account with this IBAN exists already."));
        }
        clean_iban = Some(iban);
    }
    conn.execute(
        "INSERT INTO accounts (user_id, name, type, on_budget, currency, iban, closed, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![user_id, name, new.account_type, on_budget, currency, clean_iban, now_text()],
    )?;
    let account = Account::get(conn, conn.last_insert_rowid())?.expect("The account was just added");
    // Movements to this IBAN that are imported already can be transfers now.
    update_incoming_transfers(conn, &account)?;
    if new.starting_balance_minor != 0 {
        let on = new.starting_date.unwrap_or_else(crate::dates::today);
        add_starting_balance(conn, &account, new.starting_balance_minor, on)?;
    }
    Ok(account)
}

/// Return the first income category. A starting balance goes to "Ready to Assign".
pub fn income_category_for_starting_balance(conn: &Connection, user_id: i64) -> Result<Option<Category>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {} FROM categories WHERE user_id = ?1 AND kind = ?2 ORDER BY id LIMIT 1",
                Category::COLUMNS
            ),
            params![user_id, CategoryKind::Income],
            Category::from_row,
        )
        .optional()?)
}

pub fn add_starting_balance(conn: &Connection, account: &Account, amount_minor: i64, on: Date) -> Result<()> {
    let category =
        if account.on_budget { income_category_for_starting_balance(conn, account.user_id)? } else { None };
    let fingerprint = format!("start:{}", account.id);
    NewTransaction {
        user_id: account.user_id,
        account_id: account.id,
        booking_date: on,
        value_date: Some(on),
        amount_minor,
        currency: &account.currency,
        counterparty: "Starting balance",
        counterparty_iban: None,
        description: "",
        reference: None,
        bank_reference: None,
        memo: "",
        category_id: category.map(|c| c.id),
        is_transfer: false,
        source: TransactionSource::StartingBalance,
        fingerprint: &fingerprint,
        import_batch_id: None,
    }
    .insert(conn)?;
    Ok(())
}

/// The values of a manual transaction.
#[derive(Debug, Clone)]
pub struct ManualTransaction<'a> {
    pub booking_date: Date,
    pub amount_minor: i64,
    pub counterparty: &'a str,
    pub memo: &'a str,
    pub category_id: Option<i64>,
}

pub fn add_manual_transaction(
    conn: &Connection,
    account: &Account,
    manual: &ManualTransaction<'_>,
) -> Result<i64> {
    if account.closed {
        return Err(Error::service("The account is closed."));
    }
    if let Some(category_id) = manual.category_id {
        match Category::get(conn, category_id)? {
            Some(category) if category.user_id == account.user_id => {}
            _ => return Err(Error::NotFound("category")),
        }
    }
    let counterparty: String = manual.counterparty.trim().chars().take(140).collect();
    let memo: String = manual.memo.trim().chars().take(500).collect();
    let fingerprint = format!("manual:{}", hex::encode(super::users::random_bytes::<16>()));
    let id = NewTransaction {
        user_id: account.user_id,
        account_id: account.id,
        booking_date: manual.booking_date,
        value_date: Some(manual.booking_date),
        amount_minor: manual.amount_minor,
        currency: &account.currency,
        counterparty: &counterparty,
        counterparty_iban: None,
        description: "",
        reference: None,
        bank_reference: None,
        memo: &memo,
        category_id: if account.on_budget { manual.category_id } else { None },
        is_transfer: false,
        source: TransactionSource::Manual,
        fingerprint: &fingerprint,
        import_batch_id: None,
    }
    .insert(conn)?;
    Ok(id)
}

pub fn set_closed(conn: &Connection, account: &Account, closed: bool) -> Result<()> {
    conn.execute("UPDATE accounts SET closed = ?1 WHERE id = ?2", params![closed, account.id])?;
    Ok(())
}

/// Change the name, the type and the budget setting of an account.
///
/// If `on_budget` is `None`, the type gives the budget setting. If the budget setting
/// changes, [`reclassify`] updates the transactions.
pub fn update_account(
    conn: &Connection,
    account: &Account,
    name: &str,
    account_type: AccountType,
    budget_currency: &str,
    on_budget: Option<bool>,
) -> Result<Account> {
    let name = checked_name(name)?;
    let on_budget = on_budget.unwrap_or(account_type.on_budget_default());
    if on_budget && account.currency != budget_currency {
        return Err(budget_currency_error(budget_currency));
    }
    conn.execute(
        "UPDATE accounts SET name = ?1, type = ?2, on_budget = ?3 WHERE id = ?4",
        params![name, account_type, on_budget, account.id],
    )?;
    let updated = Account::get(conn, account.id)?.expect("The account exists");
    if on_budget != account.on_budget {
        reclassify(conn, &updated)?;
    }
    Ok(updated)
}

/// Update the transactions after the budget setting of `account` changed.
///
/// - A tracking account has no categories. A budget account gives its starting
///   balance to "Ready to Assign". Its other transactions stay uncategorised.
/// - A transfer is a movement between two own accounts with the same budget setting
///   (see ADR-0008). The transfer flag changes for the movements between this account
///   and the other accounts. A new transfer has no category.
fn reclassify(conn: &Connection, account: &Account) -> Result<()> {
    if !account.on_budget {
        conn.execute("UPDATE transactions SET category_id = NULL WHERE account_id = ?1", [account.id])?;
    } else if let Some(income) = income_category_for_starting_balance(conn, account.user_id)? {
        conn.execute(
            "UPDATE transactions SET category_id = ?1 WHERE account_id = ?2 AND source = ?3 \
             AND category_id IS NULL",
            params![income.id, account.id, TransactionSource::StartingBalance],
        )?;
    }
    let own = own_ibans(conn, account.user_id)?;
    // Movements of this account to other own accounts.
    let mut stmt = conn.prepare(
        "SELECT id, counterparty_iban FROM transactions WHERE account_id = ?1 \
         AND counterparty_iban IS NOT NULL",
    )?;
    let outgoing: Vec<(i64, String)> = stmt
        .query_map([account.id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (tx_id, iban) in outgoing {
        if Some(&iban) == account.iban.as_ref() {
            continue;
        }
        if let Some((_, other_on_budget)) = own.get(&iban) {
            set_transfer(conn, tx_id, *other_on_budget == account.on_budget)?;
        }
    }
    // Movements of the other own accounts to this account.
    update_incoming_transfers(conn, account)
}

/// The accounts of the user that have an IBAN: IBAN → (account ID, on budget).
pub fn own_ibans(conn: &Connection, user_id: i64) -> Result<HashMap<String, (i64, bool)>> {
    let mut stmt =
        conn.prepare("SELECT id, iban, on_budget FROM accounts WHERE user_id = ?1 AND iban IS NOT NULL")?;
    let rows = stmt.query_map([user_id], |row| Ok((row.get::<_, String>(1)?, (row.get(0)?, row.get(2)?))))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Set the transfer flag of the movements from the other own accounts to `account`.
///
/// A movement is a transfer if the two accounts have the same budget setting.
pub fn update_incoming_transfers(conn: &Connection, account: &Account) -> Result<()> {
    let Some(iban) = account.iban.as_deref().filter(|i| !i.is_empty()) else {
        return Ok(());
    };
    let mut stmt = conn.prepare(
        "SELECT t.id, a.on_budget FROM transactions t JOIN accounts a ON a.id = t.account_id \
         WHERE t.user_id = ?1 AND t.account_id != ?2 AND t.counterparty_iban = ?3",
    )?;
    let incoming: Vec<(i64, bool)> = stmt
        .query_map(params![account.user_id, account.id, iban], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (tx_id, on_budget) in incoming {
        set_transfer(conn, tx_id, on_budget == account.on_budget)?;
    }
    Ok(())
}

fn set_transfer(conn: &Connection, tx_id: i64, is_transfer: bool) -> Result<()> {
    if is_transfer {
        conn.execute("UPDATE transactions SET is_transfer = 1, category_id = NULL WHERE id = ?1", [tx_id])?;
    } else {
        conn.execute("UPDATE transactions SET is_transfer = 0 WHERE id = ?1", [tx_id])?;
    }
    Ok(())
}
