//! Envelope budgeting ("give every franc a job").
//!
//! The rules follow the YNAB method:
//!
//! * Income goes to *Ready to Assign*. An income transaction is a transaction in a
//!   budget account with a category of the kind `income`.
//! * The user assigns money from *Ready to Assign* to expense categories for a month.
//! * `available = available of the previous month (if positive) + assigned + activity`.
//! * A positive *available* value rolls over to the next month.
//! * A negative *available* value (overspending) does not roll over. The application
//!   subtracts it from *Ready to Assign* in the next month.
//! * Transfers and transactions in tracking accounts do not change the budget.

use crate::dates::{self, Date};
use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::categorise::owned_category;
use super::{Error, Result};
use crate::db::{date_text, parse_date};
use crate::models::{Category, CategoryGroup, CategoryKind};

pub const MAX_ASSIGNMENT: i64 = 10_000_000_000_000;

pub fn month_start(value: Date) -> Date {
    dates::first_of_month(value)
}

pub fn add_months(value: Date, months: i32) -> Date {
    dates::from_month_index(dates::month_index(value) + months).expect("The month is valid")
}

/// The last day of the month of `month`.
pub fn last_day_of_month(month: Date) -> Date {
    dates::last_of_month(month)
}

pub fn month_param(value: Date) -> String {
    crate::dates::format(value, "%Y-%m")
}

/// Parse a month (`YYYY-MM`). An empty value gives the month of `default`.
pub fn parse_month(text: Option<&str>, default: Date) -> Result<Date> {
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        return Ok(month_start(default));
    };
    let invalid = || Error::service("The month must have the format YYYY-MM.");
    let (year, month) = text.split_once('-').ok_or_else(invalid)?;
    let parse = |s: &str| {
        let s = s.trim();
        let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<i32>().ok()
    };
    let (year, month) = (parse(year).ok_or_else(invalid)?, parse(month).ok_or_else(invalid)?);
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(invalid());
    }
    crate::dates::ymd(year, month as u32, 1).ok_or_else(invalid)
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryRow {
    pub category_id: i64,
    pub name: String,
    pub assigned: i64,
    pub activity: i64,
    pub available: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRows {
    pub group_id: i64,
    pub name: String,
    pub rows: Vec<CategoryRow>,
    pub assigned: i64,
    pub activity: i64,
    pub available: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MonthBudget {
    pub month: Date,
    pub ready_to_assign: i64,
    pub income: i64,
    pub assigned: i64,
    pub activity: i64,
    pub overspent_last_month: i64,
    pub groups: Vec<GroupRows>,
    pub uncategorised_count: i64,
}

type ByCategoryMonth = HashMap<(i64, Date), i64>;

/// Return (assigned, expense activity, income) per (category, month) / month.
fn monthly(
    conn: &Connection,
    user_id: i64,
) -> Result<(ByCategoryMonth, ByCategoryMonth, HashMap<Date, i64>)> {
    let mut stmt = conn.prepare(
        "SELECT t.category_id, c.kind, date(t.booking_date, 'start of month'), sum(t.amount_minor) \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         JOIN categories c ON t.category_id = c.id \
         WHERE t.user_id = ?1 AND a.on_budget = 1 AND t.is_transfer = 0 \
         GROUP BY t.category_id, c.kind, date(t.booking_date, 'start of month')",
    )?;
    let rows = stmt.query_map([user_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, CategoryKind>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut activity = HashMap::new();
    let mut income: HashMap<Date, i64> = HashMap::new();
    for row in rows {
        let (category_id, kind, month, total) = row?;
        let month = parse_date(&month);
        if kind == CategoryKind::Income {
            *income.entry(month).or_default() += total;
        } else {
            activity.insert((category_id, month), total);
        }
    }
    let mut stmt =
        conn.prepare("SELECT category_id, month, amount_minor FROM budget_assignments WHERE user_id = ?1")?;
    let rows = stmt.query_map([user_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
    })?;
    let mut assigned = HashMap::new();
    for row in rows {
        let (category_id, month, amount) = row?;
        assigned.insert((category_id, parse_date(&month)), amount);
    }
    Ok((assigned, activity, income))
}

pub fn month_budget(conn: &Connection, user_id: i64, month: Date) -> Result<MonthBudget> {
    let month = month_start(month);
    let (assigned, activity, income) = monthly(conn, user_id)?;
    let groups = CategoryGroup::list(conn, user_id)?;
    let expense_categories: Vec<&Category> =
        groups.iter().flat_map(|g| g.categories.iter()).filter(|c| c.kind == CategoryKind::Expense).collect();
    let first = assigned
        .keys()
        .map(|(_, m)| *m)
        .chain(activity.keys().map(|(_, m)| *m))
        .chain(income.keys().copied())
        .min()
        .unwrap_or(month)
        .min(month);

    let mut available: HashMap<i64, i64> = HashMap::new();
    let mut ready: i64 = 0;
    let mut overspent_previous: i64 = 0;
    let mut current = first;
    while current <= month {
        ready += income.get(&current).copied().unwrap_or(0) + overspent_previous;
        let mut overspent_this = 0;
        for category in &expense_categories {
            let carry = available.get(&category.id).copied().unwrap_or(0).max(0);
            let a = assigned.get(&(category.id, current)).copied().unwrap_or(0);
            let act = activity.get(&(category.id, current)).copied().unwrap_or(0);
            let value = carry + a + act;
            available.insert(category.id, value);
            ready -= a;
            if value < 0 {
                overspent_this += value;
            }
        }
        if current == month {
            break;
        }
        overspent_previous = overspent_this;
        current = add_months(current, 1);
    }

    // Money that the user assigned to future months is not ready to assign now.
    ready -= assigned.iter().filter(|((_, m), _)| *m > month).map(|(_, a)| a).sum::<i64>();

    let mut group_rows = Vec::new();
    for group in &groups {
        let rows: Vec<CategoryRow> = group
            .categories
            .iter()
            .filter(|c| c.kind == CategoryKind::Expense && !c.hidden)
            .map(|c| CategoryRow {
                category_id: c.id,
                name: c.name.clone(),
                assigned: assigned.get(&(c.id, month)).copied().unwrap_or(0),
                activity: activity.get(&(c.id, month)).copied().unwrap_or(0),
                available: available.get(&c.id).copied().unwrap_or(0),
            })
            .collect();
        if !rows.is_empty() {
            group_rows.push(GroupRows {
                group_id: group.id,
                name: group.name.clone(),
                assigned: rows.iter().map(|r| r.assigned).sum(),
                activity: rows.iter().map(|r| r.activity).sum(),
                available: rows.iter().map(|r| r.available).sum(),
                rows,
            });
        }
    }

    let uncategorised: i64 = conn.query_row(
        "SELECT count(t.id) FROM transactions t JOIN accounts a ON t.account_id = a.id \
         WHERE t.user_id = ?1 AND a.on_budget = 1 AND t.category_id IS NULL AND t.is_transfer = 0",
        [user_id],
        |row| row.get(0),
    )?;
    Ok(MonthBudget {
        month,
        ready_to_assign: ready,
        income: income.get(&month).copied().unwrap_or(0),
        assigned: group_rows.iter().map(|g| g.assigned).sum(),
        activity: group_rows.iter().map(|g| g.activity).sum(),
        overspent_last_month: overspent_previous,
        groups: group_rows,
        uncategorised_count: uncategorised,
    })
}

/// Set the assigned amount of a category for a month. Return the category.
pub fn assign(
    conn: &Connection,
    user_id: i64,
    category_id: i64,
    month: Date,
    amount_minor: i64,
) -> Result<Category> {
    let category = owned_category(conn, user_id, category_id)?;
    if category.kind != CategoryKind::Expense {
        return Err(Error::service("You can only assign money to expense categories."));
    }
    if amount_minor.abs() > MAX_ASSIGNMENT {
        return Err(Error::service("The amount is too large."));
    }
    let month = date_text(month_start(month));
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM budget_assignments WHERE category_id = ?1 AND month = ?2",
            params![category_id, month],
            |row| row.get(0),
        )
        .optional()?;
    match existing {
        Some(id) => {
            conn.execute(
                "UPDATE budget_assignments SET amount_minor = ?1 WHERE id = ?2",
                params![amount_minor, id],
            )?;
        }
        None => {
            conn.execute(
                "INSERT INTO budget_assignments (user_id, category_id, month, amount_minor) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![user_id, category_id, month, amount_minor],
            )?;
        }
    }
    Ok(category)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> Date {
        crate::dates::ymd(y, m, day).unwrap()
    }

    #[test]
    fn months() {
        assert_eq!(add_months(d(2026, 1, 1), -1), d(2025, 12, 1));
        assert_eq!(add_months(d(2026, 12, 1), 1), d(2027, 1, 1));
        assert_eq!(add_months(d(2026, 3, 1), -15), d(2024, 12, 1));
        assert_eq!(last_day_of_month(d(2024, 2, 1)), d(2024, 2, 29));
    }

    #[test]
    fn parse_months() {
        let today = d(2026, 11, 26);
        assert_eq!(parse_month(None, today).unwrap(), d(2026, 11, 1));
        assert_eq!(parse_month(Some("2026-02"), today).unwrap(), d(2026, 2, 1));
        assert!(parse_month(Some("2026-13"), today).is_err());
        assert!(parse_month(Some("abc"), today).is_err());
    }
}
