//! Automatic transaction categorisation.
//!
//! The order of the methods:
//!
//! 1. Transfers. If the counterparty IBAN is the IBAN of another account of the same
//!    user, and both accounts are in the budget (or both are tracking accounts), the
//!    transaction is a transfer. A transfer has no category. A transfer from a budget
//!    account to a tracking account (for example, to pillar 3a) needs a category,
//!    because the money leaves the budget.
//! 2. Rules. The user makes rules (field, match type, pattern). The rule with the
//!    lowest priority number applies first. The rules use simple text comparison and
//!    not regular expressions. This prevents ReDoS attacks.
//! 3. History. If the user categorised a transaction with the same counterparty IBAN
//!    or counterparty name before, the most recent category applies.
//!
//! Between 1 and 2, [`super::transfers`] finds the transfers that have no IBAN: two
//! matching transactions on two accounts.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use super::accounts::own_ibans;
use super::transfers::{match_transfers, reject_transfer};
use super::{Error, Result, casefold};
use crate::models::{Account, CategorisationRule, Category, MatchType, RuleField, Transaction};

/// The values of a transaction that a rule can compare.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub counterparty: &'a str,
    pub counterparty_iban: Option<&'a str>,
    pub description: &'a str,
}

pub fn rule_matches(rule: &CategorisationRule, candidate: &Candidate<'_>) -> bool {
    let value = match rule.field {
        RuleField::Counterparty => candidate.counterparty,
        RuleField::CounterpartyIban => candidate.counterparty_iban.unwrap_or(""),
        RuleField::Description => candidate.description,
    };
    let value = casefold(value);
    let pattern = casefold(&rule.pattern);
    if pattern.is_empty() {
        return false;
    }
    match rule.match_type {
        MatchType::Equals => value == pattern,
        MatchType::Contains => value.contains(&pattern),
    }
}

pub fn list_rules(conn: &Connection, user_id: i64) -> Result<Vec<CategorisationRule>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM categorisation_rules WHERE user_id = ?1 ORDER BY priority, id",
        CategorisationRule::COLUMNS
    ))?;
    let rows = stmt.query_map([user_id], CategorisationRule::from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Categorise transactions for one user. Load the rules and the history one time.
pub struct Categoriser {
    rules: Vec<CategorisationRule>,
    own_ibans: HashMap<String, (i64, bool)>,
    by_iban: HashMap<String, i64>,
    by_name: HashMap<String, i64>,
}

impl Categoriser {
    pub fn new(conn: &Connection, user_id: i64) -> Result<Categoriser> {
        let mut categoriser = Categoriser {
            rules: list_rules(conn, user_id)?,
            own_ibans: own_ibans(conn, user_id)?,
            by_iban: HashMap::new(),
            by_name: HashMap::new(),
        };
        let mut stmt = conn.prepare(
            "SELECT counterparty, counterparty_iban, category_id FROM transactions \
             WHERE user_id = ?1 AND category_id IS NOT NULL ORDER BY booking_date, id",
        )?;
        let rows = stmt.query_map([user_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?))
        })?;
        for row in rows {
            let (name, iban, category_id) = row?;
            categoriser.learn(&name, iban.as_deref(), Some(category_id));
        }
        Ok(categoriser)
    }

    pub fn is_transfer(&self, account: &Account, counterparty_iban: Option<&str>) -> bool {
        let Some(iban) = counterparty_iban.filter(|i| !i.is_empty()) else {
            return false;
        };
        match self.own_ibans.get(iban) {
            Some((other_id, other_on_budget)) => {
                *other_id != account.id && *other_on_budget == account.on_budget
            }
            None => false,
        }
    }

    pub fn suggest(
        &self,
        counterparty: &str,
        counterparty_iban: Option<&str>,
        description: &str,
    ) -> Option<i64> {
        let candidate = Candidate { counterparty, counterparty_iban, description };
        if let Some(rule) = self.rules.iter().find(|rule| rule_matches(rule, &candidate)) {
            return Some(rule.category_id);
        }
        if let Some(category) = counterparty_iban.filter(|i| !i.is_empty()).and_then(|i| self.by_iban.get(i))
        {
            return Some(*category);
        }
        if counterparty.is_empty() {
            return None;
        }
        self.by_name.get(&casefold(counterparty)).copied()
    }

    pub fn learn(&mut self, counterparty: &str, counterparty_iban: Option<&str>, category_id: Option<i64>) {
        let Some(category_id) = category_id else {
            return;
        };
        if let Some(iban) = counterparty_iban.filter(|i| !i.is_empty()) {
            self.by_iban.insert(iban.to_string(), category_id);
        }
        if !counterparty.is_empty() {
            self.by_name.insert(casefold(counterparty), category_id);
        }
    }
}

pub fn owned_category(conn: &Connection, user_id: i64, category_id: i64) -> Result<Category> {
    match Category::get(conn, category_id)? {
        Some(category) if category.user_id == user_id => Ok(category),
        _ => Err(Error::NotFound("category")),
    }
}

pub fn owned_transaction(conn: &Connection, user_id: i64, tx_id: i64) -> Result<Transaction> {
    match Transaction::get(conn, tx_id)? {
        Some(tx) if tx.user_id == user_id => Ok(tx),
        _ => Err(Error::NotFound("transaction")),
    }
}

/// Set the category of a transaction (`None`: uncategorised).
///
/// If the transaction is a transfer, it stops being a transfer. If it is in a transfer
/// pair, the other transaction of the pair stops being a transfer too (see
/// [`reject_transfer`]). Return that transaction.
pub fn set_category(
    conn: &Connection,
    tx: &Transaction,
    category_id: Option<i64>,
) -> Result<Option<Transaction>> {
    if let Some(category_id) = category_id {
        owned_category(conn, tx.user_id, category_id)?;
        let account = Account::get(conn, tx.account_id)?.ok_or(Error::NotFound("account"))?;
        if !account.on_budget {
            return Err(Error::service("Transactions in tracking accounts have no category."));
        }
    }
    let partner = if tx.is_transfer { reject_transfer(conn, tx.id)? } else { None };
    conn.execute("UPDATE transactions SET category_id = ?1 WHERE id = ?2", params![category_id, tx.id])?;
    Ok(partner)
}

/// Mark a transaction as a transfer without a category.
pub fn mark_transfer(conn: &Connection, tx_id: i64) -> Result<()> {
    conn.execute("UPDATE transactions SET category_id = NULL, is_transfer = 1 WHERE id = ?1", [tx_id])?;
    Ok(())
}

/// Check the values of a rule. Return the pattern without spaces at the ends.
fn checked_rule_values(
    conn: &Connection,
    user_id: i64,
    pattern: &str,
    priority: i64,
    category_id: i64,
) -> Result<String> {
    let pattern = pattern.trim();
    if !(1..=140).contains(&pattern.chars().count()) {
        return Err(Error::service("The pattern must have 1 to 140 characters."));
    }
    if !(0..=10_000).contains(&priority) {
        return Err(Error::service("The priority must be between 0 and 10000."));
    }
    owned_category(conn, user_id, category_id)?;
    Ok(pattern.to_string())
}

pub fn owned_rule(conn: &Connection, user_id: i64, rule_id: i64) -> Result<CategorisationRule> {
    let rule = conn.query_row(
        &format!("SELECT {} FROM categorisation_rules WHERE id = ?1", CategorisationRule::COLUMNS),
        [rule_id],
        CategorisationRule::from_row,
    );
    match rule {
        Ok(rule) if rule.user_id == user_id => Ok(rule),
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Err(Error::NotFound("rule")),
        Err(error) => Err(error.into()),
    }
}

/// The values of a rule.
#[derive(Debug, Clone)]
pub struct RuleValues<'a> {
    pub field: RuleField,
    pub match_type: MatchType,
    pub pattern: &'a str,
    pub category_id: i64,
    pub priority: i64,
}

pub fn create_rule(conn: &Connection, user_id: i64, values: &RuleValues<'_>) -> Result<i64> {
    let pattern = checked_rule_values(conn, user_id, values.pattern, values.priority, values.category_id)?;
    conn.execute(
        "INSERT INTO categorisation_rules (user_id, field, match_type, pattern, category_id, priority) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![user_id, values.field, values.match_type, pattern, values.category_id, values.priority],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Change all values of a rule.
///
/// The change applies to the next imports. Transactions that have a category already
/// keep it. "Apply the rules to uncategorised transactions" applies the changed rule
/// to the uncategorised transactions.
pub fn update_rule(conn: &Connection, rule: &CategorisationRule, values: &RuleValues<'_>) -> Result<()> {
    let pattern =
        checked_rule_values(conn, rule.user_id, values.pattern, values.priority, values.category_id)?;
    conn.execute(
        "UPDATE categorisation_rules SET field = ?1, match_type = ?2, pattern = ?3, \
         category_id = ?4, priority = ?5 WHERE id = ?6",
        params![values.field, values.match_type, pattern, values.category_id, values.priority, rule.id],
    )?;
    Ok(())
}

pub fn delete_rule(conn: &Connection, rule: &CategorisationRule) -> Result<()> {
    conn.execute("DELETE FROM categorisation_rules WHERE id = ?1", [rule.id])?;
    Ok(())
}

/// Find transfers and apply the rules and the history to the uncategorised transactions.
///
/// The matching of transfers ([`match_transfers`]) applies first, to all dates. Return
/// the number of changed transactions.
pub fn apply_to_uncategorised(conn: &Connection, user_id: i64) -> Result<usize> {
    let matched = 2 * match_transfers(conn, user_id, None, None)?.len();
    let categoriser = Categoriser::new(conn, user_id)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM transactions JOIN accounts ON transactions.account_id = accounts.id \
         WHERE transactions.user_id = ?1 AND transactions.category_id IS NULL \
         AND transactions.is_transfer = 0 AND accounts.on_budget = 1 \
         ORDER BY transactions.booking_date, transactions.id",
        Transaction::COLUMNS
    ))?;
    let txs: Vec<Transaction> =
        stmt.query_map([user_id], Transaction::from_row)?.collect::<rusqlite::Result<_>>()?;
    let mut accounts: HashMap<i64, Account> = HashMap::new();
    let mut changed = 0;
    for tx in txs {
        if !accounts.contains_key(&tx.account_id)
            && let Some(account) = Account::get(conn, tx.account_id)?
        {
            accounts.insert(account.id, account);
        }
        let Some(account) = accounts.get(&tx.account_id) else {
            continue;
        };
        if categoriser.is_transfer(account, tx.counterparty_iban.as_deref()) {
            conn.execute("UPDATE transactions SET is_transfer = 1 WHERE id = ?1", [tx.id])?;
            changed += 1;
            continue;
        }
        if let Some(suggestion) =
            categoriser.suggest(&tx.counterparty, tx.counterparty_iban.as_deref(), &tx.description)
        {
            conn.execute(
                "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
                params![suggestion, tx.id],
            )?;
            changed += 1;
        }
    }
    Ok(matched + changed)
}
