//! Find transfers from two matching transactions on two accounts.
//!
//! The import finds most transfers from the counterparty IBAN (see
//! [`super::categorise`]). Some statements do not show the IBAN of the other account,
//! for example the statement of a credit card. This module finds a transfer from the
//! two transactions of the movement: an outflow on one account and an inflow of the
//! same amount on a different account, some days apart.
//!
//! The rules prevent wrong transfers, because a wrong transfer hides an expense:
//!
//! - Only uncategorised transactions that are not transfers. A category that the user
//!   set never changes. Starting balances are not included.
//! - The two accounts belong to the user and have the same budget setting (two budget
//!   accounts or two tracking accounts, see ADR-0008).
//! - The same currency and the opposite amount.
//! - The booking dates are at most [`MAX_DAYS`] days apart.
//! - The pair is unambiguous: each transaction is the only nearest candidate of the
//!   other. If two candidates have the same distance in days, no transaction is marked.
//! - The user did not reject the pair before.
//!
//! The table `transfer_pairs` records each pair, also the pairs that the import finds
//! from the IBAN. Thus the user can see the partner of a transfer, and "not a
//! transfer" changes the two transactions together.

use crate::dates::{self, Date};
use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::Result;
use crate::db::{date_text, now_text, parse_date};
use crate::models::{Transaction, TransactionSource, TransferMethod, TransferPair, TransferStatus};

/// The maximum number of days between the two bookings of one transfer.
pub const MAX_DAYS: i64 = 4;

#[derive(Debug, Clone)]
struct Candidate {
    id: i64,
    account_id: i64,
    booking_date: Date,
    on_budget: bool,
}

fn days_between(a: Date, b: Date) -> i64 {
    dates::days_between(a, b)
}

/// The only nearest possible partner of `candidate`, or `None`.
fn best<'a>(candidate: &Candidate, others: &[&'a Candidate]) -> Option<&'a Candidate> {
    let possible: Vec<(i64, &Candidate)> = others
        .iter()
        .filter(|other| {
            other.account_id != candidate.account_id
                && other.on_budget == candidate.on_budget
                && days_between(other.booking_date, candidate.booking_date) <= MAX_DAYS
        })
        .map(|other| (days_between(other.booking_date, candidate.booking_date), *other))
        .collect();
    let nearest = possible.iter().map(|(days, _)| *days).min()?;
    let found: Vec<&Candidate> =
        possible.iter().filter(|(days, _)| *days == nearest).map(|(_, c)| *c).collect();
    if found.len() == 1 { Some(found[0]) } else { None }
}

/// Mark the matching pairs of transactions as transfers. Return the pairs
/// (outflow ID, inflow ID).
///
/// `start` and `end` limit the transactions that can start a pair (for example the
/// dates of one import). A partner can be up to [`MAX_DAYS`] days outside this range.
pub fn match_transfers(
    conn: &Connection,
    user_id: i64,
    start: Option<Date>,
    end: Option<Date>,
) -> Result<Vec<(i64, i64)>> {
    let mut sql = String::from(
        "SELECT t.id, t.account_id, t.booking_date, t.amount_minor, t.currency, a.on_budget \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         WHERE t.user_id = ?1 AND t.is_transfer = 0 AND t.category_id IS NULL \
         AND t.source != ?2 AND t.amount_minor != 0",
    );
    let mut values: Vec<rusqlite::types::Value> =
        vec![user_id.into(), TransactionSource::StartingBalance.as_str().to_string().into()];
    if let Some(start) = start {
        values.push(date_text(dates::add_days(start, -MAX_DAYS)).into());
        sql.push_str(&format!(" AND t.booking_date >= ?{}", values.len()));
    }
    if let Some(end) = end {
        values.push(date_text(dates::add_days(end, MAX_DAYS)).into());
        sql.push_str(&format!(" AND t.booking_date <= ?{}", values.len()));
    }
    sql.push_str(" ORDER BY t.id");

    // Group by currency and absolute amount: only these can match.
    let mut keys: Vec<(String, i64)> = Vec::new();
    let mut outflows: HashMap<(String, i64), Vec<Candidate>> = HashMap::new();
    let mut inflows: HashMap<(String, i64), Vec<Candidate>> = HashMap::new();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values), |row| {
        Ok((
            Candidate {
                id: row.get(0)?,
                account_id: row.get(1)?,
                booking_date: parse_date(&row.get::<_, String>(2)?),
                on_budget: row.get(5)?,
            },
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (candidate, amount, currency) = row?;
        let key = (currency, amount.abs());
        if amount < 0 {
            if !outflows.contains_key(&key) {
                keys.push(key.clone());
            }
            outflows.entry(key).or_default().push(candidate);
        } else {
            inflows.entry(key).or_default().push(candidate);
        }
    }

    let in_range = |date: Date| start.is_none_or(|s| date >= s) && end.is_none_or(|e| date <= e);
    let rejected = rejected_pairs(conn, user_id)?;
    let mut pairs = Vec::new();
    let empty = Vec::new();
    for key in &keys {
        let sent = &outflows[key];
        let received = inflows.get(key).unwrap_or(&empty);
        for out in sent {
            let allowed: Vec<&Candidate> =
                received.iter().filter(|c| !rejected.contains(&(out.id, c.id))).collect();
            let Some(partner) = best(out, &allowed) else {
                continue;
            };
            let others: Vec<&Candidate> =
                sent.iter().filter(|c| !rejected.contains(&(c.id, partner.id))).collect();
            if best(partner, &others).is_none_or(|b| b.id != out.id) {
                continue;
            }
            if !(in_range(out.booking_date) || in_range(partner.booking_date)) {
                continue;
            }
            conn.execute(
                "UPDATE transactions SET is_transfer = 1 WHERE id IN (?1, ?2)",
                params![out.id, partner.id],
            )?;
            add_pair(conn, user_id, out.id, partner.id, TransferMethod::Match)?;
            pairs.push((out.id, partner.id));
        }
    }
    Ok(pairs)
}

fn rejected_pairs(conn: &Connection, user_id: i64) -> Result<HashSet<(i64, i64)>> {
    let mut stmt =
        conn.prepare("SELECT outflow_id, inflow_id FROM transfer_pairs WHERE user_id = ?1 AND status = ?2")?;
    let rows =
        stmt.query_map(params![user_id, TransferStatus::Rejected], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn add_pair(
    conn: &Connection,
    user_id: i64,
    outflow_id: i64,
    inflow_id: i64,
    method: TransferMethod,
) -> Result<()> {
    conn.execute(
        "INSERT INTO transfer_pairs (user_id, outflow_id, inflow_id, method, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![user_id, outflow_id, inflow_id, method, TransferStatus::Active, now_text()],
    )?;
    Ok(())
}

/// The active pair of a transaction, or `None`.
pub fn active_pair(conn: &Connection, tx_id: i64) -> Result<Option<TransferPair>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {} FROM transfer_pairs WHERE status = ?1 AND (outflow_id = ?2 OR inflow_id = ?2) \
                 ORDER BY id LIMIT 1",
                TransferPair::COLUMNS
            ),
            params![TransferStatus::Active, tx_id],
            TransferPair::from_row,
        )
        .optional()?)
}

/// The other transaction of a transfer pair, for the transaction list.
#[derive(Debug, Clone)]
pub struct Partner {
    pub tx: Transaction,
    pub method: TransferMethod,
}

/// The partners of the transactions that are in an active pair, by transaction ID.
pub fn partners(conn: &Connection, tx_ids: &[i64]) -> Result<HashMap<i64, Partner>> {
    let mut result = HashMap::new();
    if tx_ids.is_empty() {
        return Ok(result);
    }
    let list = tx_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT {} FROM transfer_pairs WHERE status = ?1 AND (outflow_id IN ({list}) OR inflow_id IN ({list})) \
         ORDER BY id",
        TransferPair::COLUMNS
    ))?;
    let pairs: Vec<TransferPair> =
        stmt.query_map([TransferStatus::Active], TransferPair::from_row)?.collect::<rusqlite::Result<_>>()?;
    for pair in pairs {
        let outflow = Transaction::get(conn, pair.outflow_id)?;
        let inflow = Transaction::get(conn, pair.inflow_id)?;
        if let (Some(outflow), Some(inflow)) = (outflow, inflow) {
            result.insert(pair.outflow_id, Partner { tx: inflow, method: pair.method });
            result.insert(pair.inflow_id, Partner { tx: outflow, method: pair.method });
        }
    }
    Ok(result)
}

/// Record the pairs of transfers that the import found from the IBAN.
///
/// For each transfer without a pair, the partner is the only nearest transfer on the
/// account with the counterparty IBAN, with the opposite amount, at most [`MAX_DAYS`]
/// days apart and without a pair. Return the number of new pairs.
pub fn pair_iban_transfers(conn: &Connection, tx_ids: &[i64]) -> Result<usize> {
    let mut count = 0;
    for &tx_id in tx_ids {
        let Some(tx) = Transaction::get(conn, tx_id)? else {
            continue;
        };
        let Some(iban) = tx.counterparty_iban.as_deref().filter(|i| !i.is_empty()) else {
            continue;
        };
        if !tx.is_transfer || active_pair(conn, tx.id)?.is_some() {
            continue;
        }
        let other: Option<i64> = conn
            .query_row(
                "SELECT id FROM accounts WHERE user_id = ?1 AND iban = ?2",
                params![tx.user_id, iban],
                |row| row.get(0),
            )
            .optional()?;
        let Some(other) = other else {
            continue;
        };
        let mut stmt = conn.prepare(
            "SELECT id, booking_date FROM transactions WHERE account_id = ?1 AND is_transfer = 1 \
             AND currency = ?2 AND amount_minor = ?3 AND booking_date >= ?4 AND booking_date <= ?5 \
             ORDER BY id",
        )?;
        let rows: Vec<(i64, String)> = stmt
            .query_map(
                params![
                    other,
                    tx.currency,
                    -tx.amount_minor,
                    date_text(dates::add_days(tx.booking_date, -MAX_DAYS)),
                    date_text(dates::add_days(tx.booking_date, MAX_DAYS)),
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?
            .collect::<rusqlite::Result<_>>()?;
        let mut candidates = Vec::new();
        for (id, date) in rows {
            if active_pair(conn, id)?.is_none() {
                candidates.push((id, days_between(parse_date(&date), tx.booking_date)));
            }
        }
        let Some(nearest) = candidates.iter().map(|(_, days)| *days).min() else {
            continue;
        };
        let found: Vec<i64> =
            candidates.iter().filter(|(_, days)| *days == nearest).map(|(id, _)| *id).collect();
        if found.len() != 1 {
            continue;
        }
        let (outflow, inflow) = if tx.amount_minor < 0 { (tx.id, found[0]) } else { (found[0], tx.id) };
        add_pair(conn, tx.user_id, outflow, inflow, TransferMethod::Iban)?;
        count += 1;
    }
    Ok(count)
}

/// The user says that `tx_id` is not a transfer. Return the partner, if any.
///
/// Both transactions of the pair stop being transfers. They stay uncategorised. The
/// pair gets the status "rejected", so that the matching does not make it again.
pub fn reject_transfer(conn: &Connection, tx_id: i64) -> Result<Option<Transaction>> {
    conn.execute("UPDATE transactions SET is_transfer = 0 WHERE id = ?1", [tx_id])?;
    let Some(pair) = active_pair(conn, tx_id)? else {
        return Ok(None);
    };
    conn.execute(
        "UPDATE transfer_pairs SET status = ?1 WHERE id = ?2",
        params![TransferStatus::Rejected, pair.id],
    )?;
    let partner = pair.partner_of(tx_id);
    conn.execute("UPDATE transactions SET is_transfer = 0, category_id = NULL WHERE id = ?1", [partner])?;
    Ok(Transaction::get(conn, partner)?)
}

/// Prepare the deletion of a transaction.
///
/// The pair is deleted with the transaction. If the application found the pair from
/// matching amounts, the partner is not a transfer any more.
pub fn before_delete(conn: &Connection, tx_id: i64) -> Result<()> {
    if let Some(pair) = active_pair(conn, tx_id)?
        && pair.method == TransferMethod::Match
    {
        conn.execute("UPDATE transactions SET is_transfer = 0 WHERE id = ?1", [pair.partner_of(tx_id)])?;
    }
    Ok(())
}
