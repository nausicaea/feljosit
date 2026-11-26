//! Import of camt.053 statements and credit card CSV files.

use std::collections::{HashMap, HashSet};
use std::fmt;

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::accounts::{NewAccount, add_starting_balance, create_account, get_account};
use super::categorise::Categoriser;
use super::{Error as ServiceError, transfers};
use crate::camt053::{self, Camt053Error, Statement};
use crate::cardcsv::{self, CardCsvError};
use crate::db::{date_text, now_text};
use crate::iban::format_iban;
use crate::models::{Account, AccountType, NewTransaction, TransactionSource};

pub const MAX_FILENAME: usize = 255;

/// An error of the import. The message is safe to show to the user.
#[derive(Debug)]
pub enum ImportError {
    Service(ServiceError),
    Camt053(Camt053Error),
    CardCsv(CardCsvError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Service(e) => e.fmt(f),
            ImportError::Camt053(e) => e.fmt(f),
            ImportError::CardCsv(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<ServiceError> for ImportError {
    fn from(error: ServiceError) -> ImportError {
        ImportError::Service(error)
    }
}

impl From<rusqlite::Error> for ImportError {
    fn from(error: rusqlite::Error) -> ImportError {
        ImportError::Service(ServiceError::Db(error))
    }
}

fn service<T>(message: &str) -> Result<T, ImportError> {
    Err(ImportError::Service(ServiceError::service(message)))
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StatementResult {
    pub account_id: i64,
    pub account_name: String,
    pub statement_id: String,
    pub new_count: usize,
    pub duplicate_count: usize,
    pub categorised_count: usize,
    pub transfer_count: usize,
    /// Transfers found from a matching transaction on a different account.
    pub matched_transfer_count: usize,
    pub skipped_entries: usize,
    pub account_created: bool,
    pub closing_balance_minor: Option<i64>,
    pub computed_balance_minor: Option<i64>,
    pub balance_matches: Option<bool>,
}

impl StatementResult {
    fn update_balance_check(&mut self) {
        self.balance_matches = match (self.closing_balance_minor, self.computed_balance_minor) {
            (Some(closing), Some(computed)) => Some(closing == computed),
            _ => None,
        };
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportResult {
    pub statements: Vec<StatementResult>,
}

impl ImportResult {
    pub fn new_count(&self) -> usize {
        self.statements.iter().map(|s| s.new_count).sum()
    }

    pub fn duplicate_count(&self) -> usize {
        self.statements.iter().map(|s| s.duplicate_count).sum()
    }
}

/// The name for an account that the import makes.
///
/// Use the account name of the statement. Else use the bank name and the last four
/// characters of the IBAN. Else use the IBAN.
fn new_account_name(statement: &Statement) -> String {
    let iban = statement.account_iban.clone().unwrap_or_default();
    let name = if let Some(name) = &statement.account_name {
        name.clone()
    } else if let Some(servicer) = &statement.servicer_name {
        let chars: Vec<char> = iban.chars().collect();
        let last: String = chars[chars.len().saturating_sub(4)..].iter().collect();
        format!("{servicer} ···{last}")
    } else {
        format!("Account {}", format_iban(&iban))
    };
    name.trim().chars().take(100).collect()
}

fn statement_currency(statement: &Statement) -> Result<String, ImportError> {
    statement
        .currency
        .iter()
        .chain(statement.balances.iter().map(|b| &b.currency))
        .chain(statement.transactions.iter().map(|t| &t.currency))
        .find(|c| !c.is_empty())
        .cloned()
        .map_or_else(|| service("The statement has no currency. Add the account manually."), Ok)
}

/// Make a checking account for the IBAN of the statement.
///
/// The account is in the budget if it uses the budget currency. Else it is a tracking
/// account. The user can change the name and the type later.
fn create_account_for(
    conn: &Connection,
    user_id: i64,
    statement: &Statement,
    budget_currency: &str,
) -> Result<Account, ImportError> {
    let currency = statement_currency(statement)?;
    let name = new_account_name(statement);
    let mut new = NewAccount::new(&name, AccountType::Checking, &currency);
    new.budget_currency = budget_currency;
    new.iban = statement.account_iban.as_deref();
    new.on_budget = Some(currency == budget_currency);
    Ok(create_account(conn, user_id, &new)?)
}

/// Find the format of the file from its content and parse it.
fn parse(data: &[u8]) -> Result<(Vec<Statement>, TransactionSource), ImportError> {
    if cardcsv::looks_like_card_csv(data) {
        let statement = cardcsv::parse_file(data).map_err(ImportError::CardCsv)?;
        return Ok((vec![statement], TransactionSource::CardCsv));
    }
    let statements = camt053::parse_file(data).map_err(ImportError::Camt053)?;
    Ok((statements, TransactionSource::Camt053))
}

fn account_by_iban(conn: &Connection, user_id: i64, iban: &str) -> rusqlite::Result<Option<Account>> {
    conn.query_row(
        &format!("SELECT {} FROM accounts WHERE user_id = ?1 AND iban = ?2", Account::COLUMNS),
        params![user_id, iban],
        Account::from_row,
    )
    .optional()
}

/// Find the account of the statement. Return the account and `true` if it is new.
///
/// If no account is selected and no account has the IBAN of the statement, make an
/// account if `budget_currency` is given.
fn resolve_account(
    conn: &Connection,
    user_id: i64,
    statement: &Statement,
    account_id: Option<i64>,
    budget_currency: Option<&str>,
    source: TransactionSource,
) -> Result<(Account, bool), ImportError> {
    let by_iban = match &statement.account_iban {
        Some(iban) => account_by_iban(conn, user_id, iban)?,
        None => None,
    };
    if account_id.is_none() && source == TransactionSource::CardCsv {
        return service("A credit card file has no IBAN. Select the credit card account.");
    }
    let Some(account_id) = account_id else {
        if let Some(account) = by_iban {
            return Ok((account, false));
        }
        if let (Some(_), Some(budget_currency)) = (&statement.account_iban, budget_currency) {
            return Ok((create_account_for(conn, user_id, statement, budget_currency)?, true));
        }
        if statement.account_iban.is_some() {
            return service(
                "No account has the IBAN of the statement. Select an account or add the IBAN to \
                 an account.",
            );
        }
        return service("The statement has no IBAN. Select the account.");
    };
    let account = get_account(conn, user_id, account_id)?;
    if by_iban.as_ref().is_some_and(|b| b.id != account.id) {
        return service("The statement IBAN belongs to a different account.");
    }
    if let (Some(own), Some(statement_iban)) = (&account.iban, &statement.account_iban)
        && own != statement_iban
    {
        return service("The statement IBAN is not the IBAN of the selected account.");
    }
    Ok((account, false))
}

fn check_account(conn: &Connection, account: &mut Account, statement: &Statement) -> Result<(), ImportError> {
    if account.closed {
        return service("The account is closed.");
    }
    if let Some(currency) = &statement.currency
        && *currency != account.currency
    {
        return Err(ImportError::Service(ServiceError::service(format!(
            "The statement currency ({currency}) is not the account currency ({}).",
            account.currency
        ))));
    }
    if account.iban.is_none()
        && let Some(iban) = &statement.account_iban
    {
        conn.execute("UPDATE accounts SET iban = ?1 WHERE id = ?2", params![iban, account.id])?;
        account.iban = Some(iban.clone());
    }
    Ok(())
}

/// A new transaction that gets a category after the matching of transfers.
struct Pending {
    tx_id: i64,
    statement: usize,
    booking_date: NaiveDate,
    counterparty: String,
    counterparty_iban: Option<String>,
    description: String,
}

/// The values of an import.
#[derive(Debug, Clone)]
pub struct ImportRequest<'a> {
    pub filename: &'a str,
    pub data: &'a [u8],
    /// `None`: the IBAN of each statement gives the account.
    pub account_id: Option<i64>,
    /// If given and no account has the IBAN, the import makes a new account.
    pub budget_currency: Option<&'a str>,
}

/// Parse and import a camt.053 file (XML or ZIP) or a credit card CSV file.
///
/// The import is idempotent. The same file cannot be imported two times, and a
/// transaction that exists already (same fingerprint) is not added again. A credit
/// card CSV file has no IBAN, thus it needs `account_id`. The caller must run the
/// import in a database transaction and roll it back if an error occurs.
pub fn import_statement_file(
    conn: &Connection,
    user_id: i64,
    request: &ImportRequest<'_>,
) -> Result<ImportResult, ImportError> {
    let digest = hex::encode(Sha256::digest(request.data));
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM import_batches WHERE user_id = ?1 AND file_sha256 = ?2",
            params![user_id, digest],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_some() {
        return service("This file was imported before.");
    }

    let (statements, source) = parse(request.data)?;
    let message_id: String = statements[0].message_id.chars().take(35).collect();
    let filename: String = request.filename.chars().take(MAX_FILENAME).collect();
    conn.execute(
        "INSERT INTO import_batches (user_id, filename, file_sha256, message_id, new_count, \
         duplicate_count, imported_at) VALUES (?1, ?2, ?3, ?4, 0, 0, ?5)",
        params![user_id, filename, digest, (!message_id.is_empty()).then_some(message_id), now_text()],
    )?;
    let batch_id = conn.last_insert_rowid();
    let mut result = ImportResult::default();
    // Find or make all accounts first. Then a transfer to an account that this file
    // makes is a transfer too.
    let mut targets = Vec::new();
    for statement in &statements {
        let (mut account, created) =
            resolve_account(conn, user_id, statement, request.account_id, request.budget_currency, source)?;
        check_account(conn, &mut account, statement)?;
        targets.push((statement, account, created));
    }
    let categoriser = Categoriser::new(conn, user_id)?;
    // The order of the methods (see services::categorise): first the transfers from the
    // IBAN, then the transfers from matching transactions, then the rules and the
    // history.
    let mut pending: Vec<Pending> = Vec::new();
    for (statement, account, created) in &targets {
        let index = result.statements.len();
        let mut item =
            import_statement(conn, &categoriser, batch_id, account, statement, source, index, &mut pending)?;
        item.account_created = *created;
        result.statements.push(item);
    }
    match_pending_transfers(conn, user_id, &pending, &mut result)?;
    for item in &pending {
        let is_transfer: bool =
            conn.query_row("SELECT is_transfer FROM transactions WHERE id = ?1", [item.tx_id], |r| r.get(0))?;
        if is_transfer {
            continue;
        }
        let category =
            categoriser.suggest(&item.counterparty, item.counterparty_iban.as_deref(), &item.description);
        if let Some(category_id) = category {
            conn.execute(
                "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
                params![category_id, item.tx_id],
            )?;
            result.statements[item.statement].categorised_count += 1;
        }
    }
    // Record the pairs of the transfers that the IBAN check found.
    let mut stmt = conn
        .prepare("SELECT id FROM transactions WHERE import_batch_id = ?1 AND is_transfer = 1 ORDER BY id")?;
    let transfer_ids: Vec<i64> =
        stmt.query_map([batch_id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    transfers::pair_iban_transfers(conn, &transfer_ids)?;
    conn.execute(
        "UPDATE import_batches SET new_count = ?1, duplicate_count = ?2 WHERE id = ?3",
        params![result.new_count() as i64, result.duplicate_count() as i64, batch_id],
    )?;
    Ok(result)
}

/// Add the new transactions of one statement.
///
/// The new transactions of budget accounts that are not transfers go to `pending`.
/// They get a category after the matching of transfers.
#[allow(clippy::too_many_arguments)]
fn import_statement(
    conn: &Connection,
    categoriser: &Categoriser,
    batch_id: i64,
    account: &Account,
    statement: &Statement,
    source: TransactionSource,
    index: usize,
    pending: &mut Vec<Pending>,
) -> Result<StatementResult, ImportError> {
    let mut result = StatementResult {
        account_id: account.id,
        account_name: account.name.clone(),
        statement_id: statement.statement_id.clone(),
        skipped_entries: statement.skipped_entries,
        ..StatementResult::default()
    };
    let mut stmt = conn.prepare("SELECT fingerprint FROM transactions WHERE account_id = ?1")?;
    let mut existing: HashSet<String> =
        stmt.query_map([account.id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    if existing.is_empty()
        && let Some(opening) = statement.opening_balance().filter(|b| b.amount_minor != 0)
    {
        add_starting_balance(conn, account, opening.amount_minor, opening.date)?;
        existing.insert(format!("start:{}", account.id));
    }

    for item in &statement.transactions {
        if item.currency != account.currency {
            return service("A transaction currency is not the account currency.");
        }
        if existing.contains(&item.fingerprint) {
            result.duplicate_count += 1;
            continue;
        }
        existing.insert(item.fingerprint.clone());
        let reference: Option<String> = item.reference.as_ref().map(|r| r.chars().take(35).collect());
        let is_transfer = categoriser.is_transfer(account, item.counterparty_iban.as_deref());
        let tx_id = NewTransaction {
            user_id: account.user_id,
            account_id: account.id,
            booking_date: item.booking_date,
            value_date: item.value_date,
            amount_minor: item.amount_minor,
            currency: &item.currency,
            counterparty: &item.counterparty,
            counterparty_iban: item.counterparty_iban.as_deref(),
            description: &item.description,
            reference: reference.as_deref(),
            bank_reference: item.bank_reference.as_deref(),
            memo: "",
            category_id: None,
            is_transfer,
            source,
            fingerprint: &item.fingerprint,
            import_batch_id: Some(batch_id),
        }
        .insert(conn)?;
        if is_transfer {
            result.transfer_count += 1;
        } else if account.on_budget {
            pending.push(Pending {
                tx_id,
                statement: index,
                booking_date: item.booking_date,
                counterparty: item.counterparty.clone(),
                counterparty_iban: item.counterparty_iban.clone(),
                description: item.description.clone(),
            });
        }
        result.new_count += 1;
    }

    if let Some(closing) = statement.closing_balance() {
        result.closing_balance_minor = Some(closing.amount_minor);
        result.computed_balance_minor = Some(balance_on(conn, account.id, closing.date)?);
    }
    result.update_balance_check();
    Ok(result)
}

/// Find the transfers between the new transactions and all other transactions.
fn match_pending_transfers(
    conn: &Connection,
    user_id: i64,
    pending: &[Pending],
    result: &mut ImportResult,
) -> Result<(), ImportError> {
    let (Some(first), Some(last)) =
        (pending.iter().map(|p| p.booking_date).min(), pending.iter().map(|p| p.booking_date).max())
    else {
        return Ok(());
    };
    let pairs = transfers::match_transfers(conn, user_id, Some(first), Some(last))?;
    let by_id: HashMap<i64, usize> = pending.iter().map(|p| (p.tx_id, p.statement)).collect();
    for (outflow, inflow) in pairs {
        for tx_id in [outflow, inflow] {
            if let Some(&index) = by_id.get(&tx_id) {
                result.statements[index].transfer_count += 1;
                result.statements[index].matched_transfer_count += 1;
            }
        }
    }
    Ok(())
}

fn balance_on(conn: &Connection, account_id: i64, on: NaiveDate) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT coalesce(sum(amount_minor), 0) FROM transactions WHERE account_id = ?1 \
         AND booking_date <= ?2",
        params![account_id, date_text(on)],
        |row| row.get(0),
    )
}
