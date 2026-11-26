//! Statistics for the reports pages.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, params};
use serde::Serialize;

use super::Result;
use super::budget::{add_months, month_start};
use crate::db::{date_text, parse_date};
use crate::models::CategoryKind;
use crate::money::round_half_even;

#[derive(Debug, Clone, Serialize)]
pub struct MonthFlow {
    pub month: NaiveDate,
    pub income: i64,
    pub expenses: i64,
    pub net: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryTotal {
    pub category_id: Option<i64>,
    pub name: String,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub income: i64,
    pub expenses: i64,
    pub savings_rate_percent: Option<f64>,
    pub average_monthly_expenses: i64,
}

pub fn months_between(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut months = Vec::new();
    let mut current = month_start(start);
    while current <= end {
        months.push(current);
        current = add_months(current, 1);
    }
    months
}

/// The booking dates of the first and the last transaction of all accounts. Return
/// `None` if the user has no transactions.
pub fn transaction_date_range(conn: &Connection, user_id: i64) -> Result<Option<(NaiveDate, NaiveDate)>> {
    let (first, last): (Option<String>, Option<String>) = conn.query_row(
        "SELECT min(booking_date), max(booking_date) FROM transactions WHERE user_id = ?1",
        [user_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(match (first, last) {
        (Some(first), Some(last)) => Some((parse_date(&first), parse_date(&last))),
        _ => None,
    })
}

/// The conditions for the transactions of the reports: budget accounts, no transfers,
/// the period. Parameters: ?1 user, ?2 start, ?3 end.
const BUDGET_TX_FILTER: &str = "t.user_id = ?1 AND a.on_budget = 1 AND t.is_transfer = 0 \
    AND t.booking_date >= ?2 AND t.booking_date <= ?3";

/// Expense categories and uncategorised outflows: the spending of the reports.
const SPENDING_FILTER: &str = "(c.kind = 'expense' OR (t.category_id IS NULL AND t.amount_minor < 0))";

const MONTH_EXPR: &str = "date(t.booking_date, 'start of month')";

/// Income and expenses per month. Expenses are negative values.
///
/// For an uncategorised transaction, the sign decides: an inflow is income and an
/// outflow is an expense.
pub fn monthly_flows(
    conn: &Connection,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<MonthFlow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MONTH_EXPR}, c.kind, t.amount_minor > 0, sum(t.amount_minor) \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         LEFT OUTER JOIN categories c ON t.category_id = c.id \
         WHERE {BUDGET_TX_FILTER} GROUP BY {MONTH_EXPR}, c.kind, t.amount_minor > 0"
    ))?;
    let rows = stmt.query_map(params![user_id, date_text(start), date_text(end)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<CategoryKind>>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut income: HashMap<NaiveDate, i64> = HashMap::new();
    let mut expenses: HashMap<NaiveDate, i64> = HashMap::new();
    for row in rows {
        let (month, kind, inflow, total) = row?;
        let month = parse_date(&month);
        if kind == Some(CategoryKind::Income) || (kind.is_none() && inflow) {
            *income.entry(month).or_default() += total;
        } else {
            *expenses.entry(month).or_default() += total;
        }
    }
    Ok(months_between(start, end)
        .into_iter()
        .map(|m| {
            let income = income.get(&m).copied().unwrap_or(0);
            let expenses = expenses.get(&m).copied().unwrap_or(0);
            MonthFlow { month: m, income, expenses, net: income + expenses }
        })
        .collect())
}

/// Net expenses per expense category, largest first.
///
/// Uncategorised outflows are one row ("Uncategorised").
pub fn spending_by_category(
    conn: &Connection,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CategoryTotal>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.category_id, c.name, sum(t.amount_minor) \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         LEFT OUTER JOIN categories c ON t.category_id = c.id \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} GROUP BY t.category_id, c.name"
    ))?;
    let rows = stmt.query_map(params![user_id, date_text(start), date_text(end)], |row| {
        Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<i64>>(2)?))
    })?;
    let mut totals = Vec::new();
    for row in rows {
        let (category_id, name, total) = row?;
        if let Some(total) = total.filter(|t| *t < 0) {
            totals.push(CategoryTotal {
                category_id,
                name: name.unwrap_or_else(|| "Uncategorised".into()),
                total: -total,
            });
        }
    }
    // A stable sort, largest first (as Python's `sorted(..., reverse=True)`).
    totals.sort_by_key(|t| std::cmp::Reverse(t.total));
    Ok(totals)
}

pub fn summary(flows: &[MonthFlow]) -> Summary {
    let income: i64 = flows.iter().map(|f| f.income).sum();
    let expenses: i64 = flows.iter().map(|f| f.expenses).sum();
    let rate = (income > 0).then(|| (income + expenses) as f64 / income as f64 * 100.0);
    let months_with_data = flows.iter().filter(|f| f.income != 0 || f.expenses != 0).count();
    let average =
        if months_with_data > 0 { round_half_even(-expenses as f64 / months_with_data as f64) } else { 0 };
    Summary { income, expenses, savings_rate_percent: rate, average_monthly_expenses: average }
}

/// Total balance of all accounts at the end of each month, for each currency.
pub fn net_worth(
    conn: &Connection,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<BTreeMap<String, Vec<(NaiveDate, i64)>>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT a.currency, {MONTH_EXPR}, sum(t.amount_minor) \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         WHERE t.user_id = ?1 AND t.booking_date <= ?2 \
         GROUP BY a.currency, {MONTH_EXPR} ORDER BY {MONTH_EXPR}"
    ))?;
    let rows = stmt.query_map(params![user_id, date_text(end)], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
    })?;
    let mut changes: BTreeMap<String, HashMap<NaiveDate, i64>> = BTreeMap::new();
    for row in rows {
        let (currency, month, total) = row?;
        changes.entry(currency).or_default().insert(parse_date(&month), total);
    }
    let months = months_between(start, end);
    let mut result = BTreeMap::new();
    for (currency, per_month) in changes {
        let mut running: i64 = match months.first() {
            Some(first) => per_month.iter().filter(|(m, _)| *m < first).map(|(_, v)| v).sum(),
            None => 0,
        };
        let series = months
            .iter()
            .map(|m| {
                running += per_month.get(m).copied().unwrap_or(0);
                (*m, running)
            })
            .collect();
        result.insert(currency, series);
    }
    Ok(result)
}

// Spending by category over time.

/// The trend compares the average of the last months of the period with the months
/// before.
pub const RECENT_MONTHS: usize = 3;
pub const MIN_MONTHS_FOR_TREND: usize = 6;

/// The length of one column in months: a month, a quarter or a year.
///
/// A table with more than 12 columns is difficult to read.
pub fn bucket_months(month_count: usize) -> u32 {
    if month_count <= 12 {
        1
    } else if month_count <= 36 {
        3
    } else {
        12
    }
}

/// The first month of the bucket that contains `month`. The buckets are calendar
/// months, quarters or years.
pub fn bucket_start(month: NaiveDate, size: u32) -> NaiveDate {
    let index = (month.year() * 12 + month.month0() as i32).div_euclid(size as i32) * size as i32;
    NaiveDate::from_ymd_opt(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1).expect("valid month")
}

/// The spending of one row (a category or a counterparty) in a period.
///
/// Spending is positive. Refunds are negative.
#[derive(Debug, Clone, Serialize)]
pub struct SpendingSeries {
    /// The value that selects the row in the URL: a category ID, "uncategorised" or a
    /// counterparty name. `None` if the row cannot be selected.
    pub key: Option<String>,
    pub name: String,
    /// The category group of a category. The main category of a counterparty.
    pub group: String,
    /// Spending for each month of the period.
    pub monthly: Vec<i64>,
    /// Average spending per month in each bucket (column) of the table. A bucket at the
    /// start or the end of the period can have fewer months than the others.
    pub values: Vec<i64>,
    pub total: i64,
    pub average_per_month: i64,
    /// Part of the spending of all rows, in percent.
    pub share: f64,
    /// Average per month of the last [`RECENT_MONTHS`] months.
    pub recent_average: Option<i64>,
    /// Change of `recent_average` against the average of the months before, in percent.
    pub change_percent: Option<f64>,
    /// The category ID of a category row. `None` for "Uncategorised" and counterparties.
    pub category_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryGroupSeries {
    pub name: String,
    pub rows: Vec<SpendingSeries>,
    /// Average spending per month in each bucket, as in `SpendingSeries::values`.
    pub values: Vec<i64>,
    pub total: i64,
    pub uncategorised: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpendingComparison {
    pub months: Vec<NaiveDate>,
    /// The first month of each bucket (column).
    pub buckets: Vec<NaiveDate>,
    pub bucket_months: u32,
    pub rows: Vec<SpendingSeries>,
    /// Average spending per month of all rows in each bucket.
    pub totals: Vec<i64>,
    pub total: i64,
    pub has_trend: bool,
    /// The category groups (only for the category comparison).
    pub groups: Vec<CategoryGroupSeries>,
    /// The sum of the rows that are not shown (only for the counterparty comparison).
    pub other: Option<SpendingSeries>,
}

pub const UNCATEGORISED: &str = "uncategorised";

fn trend(monthly: &[i64]) -> (Option<i64>, Option<f64>) {
    if monthly.len() < MIN_MONTHS_FOR_TREND {
        return (None, None);
    }
    let (before, recent) = monthly.split_at(monthly.len() - RECENT_MONTHS);
    let recent_average = round_half_even(recent.iter().sum::<i64>() as f64 / recent.len() as f64);
    let before_average = before.iter().sum::<i64>() as f64 / before.len() as f64;
    if before_average <= 0.0 {
        return (Some(recent_average), None);
    }
    (Some(recent_average), Some((recent_average as f64 - before_average) / before_average * 100.0))
}

/// The months and the buckets (columns) of a period.
struct Grid {
    months: Vec<NaiveDate>,
    size: u32,
    buckets: Vec<NaiveDate>,
    bucket_sizes: Vec<i64>,
}

impl Grid {
    fn of(start: NaiveDate, end: NaiveDate) -> Grid {
        let months = months_between(start, end);
        let size = bucket_months(months.len());
        let mut buckets: Vec<NaiveDate> = months.iter().map(|m| bucket_start(*m, size)).collect();
        buckets.dedup();
        let mut sizes = vec![0; buckets.len()];
        for m in &months {
            let index = buckets.iter().position(|b| *b == bucket_start(*m, size)).expect("bucket");
            sizes[index] += 1;
        }
        Grid { months, size, buckets, bucket_sizes: sizes }
    }

    fn month_index(&self) -> HashMap<NaiveDate, usize> {
        self.months.iter().enumerate().map(|(i, m)| (*m, i)).collect()
    }

    /// The average per month of each bucket.
    fn bucket_values(&self, monthly: &[i64]) -> Vec<i64> {
        let mut sums = vec![0i64; self.buckets.len()];
        for (m, v) in self.months.iter().zip(monthly) {
            let index = self.buckets.iter().position(|b| *b == bucket_start(*m, self.size)).expect("bucket");
            sums[index] += v;
        }
        sums.iter().zip(&self.bucket_sizes).map(|(v, n)| round_half_even(*v as f64 / *n as f64)).collect()
    }

    fn series(
        &self,
        key: Option<String>,
        name: String,
        group: String,
        monthly: Vec<i64>,
        grand_total: i64,
        category_id: Option<i64>,
    ) -> SpendingSeries {
        let total: i64 = monthly.iter().sum();
        let (recent, change) = trend(&monthly);
        SpendingSeries {
            key,
            name,
            group,
            values: self.bucket_values(&monthly),
            total,
            average_per_month: if self.months.is_empty() {
                0
            } else {
                round_half_even(total as f64 / self.months.len() as f64)
            },
            share: if grand_total > 0 { total as f64 / grand_total as f64 * 100.0 } else { 0.0 },
            recent_average: recent,
            change_percent: change,
            category_id,
            monthly,
        }
    }

    fn comparison(
        &self,
        rows: Vec<SpendingSeries>,
        groups: Vec<CategoryGroupSeries>,
        other: Option<SpendingSeries>,
    ) -> SpendingComparison {
        let shown: Vec<&SpendingSeries> = rows.iter().chain(other.iter()).collect();
        SpendingComparison {
            months: self.months.clone(),
            buckets: self.buckets.clone(),
            bucket_months: self.size,
            totals: (0..self.buckets.len()).map(|i| shown.iter().map(|r| r.values[i]).sum()).collect(),
            total: shown.iter().map(|r| r.total).sum(),
            has_trend: self.months.len() >= MIN_MONTHS_FOR_TREND,
            rows,
            groups,
            other,
        }
    }
}

fn period_params(user_id: i64, start: NaiveDate, end: NaiveDate) -> [rusqlite::types::Value; 3] {
    [user_id.into(), date_text(start).into(), date_text(end).into()]
}

/// The order of the budget page: group order, group name, category order, category
/// name. "Uncategorised" is the last row.
type SortKey = (bool, i64, String, i64, String);

/// Spending per expense category and month, for the comparison of categories over time.
///
/// The same transactions as in [`spending_by_category`]: expense categories and
/// uncategorised outflows of budget accounts, without transfers. Categories without
/// spending in the period are not included.
pub fn compare_categories(
    conn: &Connection,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<SpendingComparison> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.category_id, c.name, g.name, g.sort_order, c.sort_order, {MONTH_EXPR}, \
         sum(t.amount_minor) FROM transactions t JOIN accounts a ON t.account_id = a.id \
         LEFT OUTER JOIN categories c ON t.category_id = c.id \
         LEFT OUTER JOIN category_groups g ON c.group_id = g.id \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} GROUP BY t.category_id, {MONTH_EXPR}"
    ))?;
    let grid = Grid::of(start, end);
    let index = grid.month_index();
    let mut order: Vec<Option<i64>> = Vec::new();
    let mut found: HashMap<Option<i64>, (String, String, SortKey, Vec<i64>)> = HashMap::new();
    let rows = stmt.query_map(rusqlite::params_from_iter(period_params(user_id, start, end)), |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;
    for row in rows {
        let (category_id, name, group, group_order, cat_order, month, total) = row?;
        let entry = found.entry(category_id).or_insert_with(|| {
            order.push(category_id);
            let sort_key = (
                category_id.is_none(),
                group_order.unwrap_or(0),
                group.clone().unwrap_or_default(),
                cat_order.unwrap_or(0),
                name.clone().unwrap_or_default(),
            );
            (
                name.clone().unwrap_or_else(|| "Uncategorised".into()),
                group.clone().unwrap_or_else(|| "Uncategorised".into()),
                sort_key,
                vec![0; grid.months.len()],
            )
        });
        if let Some(&i) = index.get(&parse_date(&month)) {
            entry.3[i] -= total;
        }
    }

    let grand_total: i64 = found.values().map(|(.., m)| m.iter().sum::<i64>()).sum();
    let mut series: Vec<(SortKey, SpendingSeries)> = Vec::new();
    for category_id in order {
        let (name, group, sort_key, monthly) = found.remove(&category_id).expect("found");
        if monthly.iter().all(|v| *v == 0) {
            continue;
        }
        let key = match category_id {
            None => UNCATEGORISED.to_string(),
            Some(id) => id.to_string(),
        };
        series.push((sort_key, grid.series(Some(key), name, group, monthly, grand_total, category_id)));
    }
    series.sort_by(|a, b| a.0.cmp(&b.0));
    let ordered: Vec<SpendingSeries> = series.into_iter().map(|(_, s)| s).collect();
    let mut groups: Vec<CategoryGroupSeries> = Vec::new();
    for row in &ordered {
        match groups.last_mut() {
            Some(last) if last.name == row.group => {
                last.values = last.values.iter().zip(&row.values).map(|(a, b)| a + b).collect();
                last.total += row.total;
                last.rows.push(row.clone());
            }
            _ => groups.push(CategoryGroupSeries {
                name: row.group.clone(),
                rows: vec![row.clone()],
                values: row.values.clone(),
                total: row.total,
                uncategorised: row.key.as_deref() == Some(UNCATEGORISED),
            }),
        }
    }
    Ok(grid.comparison(ordered, groups, None))
}

/// The counterparty comparison shows this number of counterparties. The rest is one row.
pub const MAX_COUNTERPARTIES: usize = 30;
pub const NO_COUNTERPARTY: &str = "(No counterparty)";

const SPENDING_FROM: &str = "FROM transactions t JOIN accounts a ON t.account_id = a.id \
    LEFT OUTER JOIN categories c ON t.category_id = c.id";

/// Spending per counterparty and month, for the comparison of counterparties over time.
///
/// The same transactions as in [`compare_categories`]. The rows are the `limit`
/// counterparties with the most spending, largest first. `other` is the sum of the
/// other counterparties. Thus the total is the same as in [`compare_categories`].
pub fn compare_counterparties(
    conn: &Connection,
    user_id: i64,
    start: NaiveDate,
    end: NaiveDate,
    limit: usize,
) -> Result<SpendingComparison> {
    let grid = Grid::of(start, end);
    let index = grid.month_index();
    let mut monthly_by_name: HashMap<String, Vec<i64>> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT t.counterparty, {MONTH_EXPR}, sum(t.amount_minor) {SPENDING_FROM} \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} GROUP BY t.counterparty, {MONTH_EXPR}"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(period_params(user_id, start, end)), |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
    })?;
    for row in rows {
        let (name, month, total) = row?;
        let monthly =
            monthly_by_name.entry(name.unwrap_or_default()).or_insert_with(|| vec![0; grid.months.len()]);
        if let Some(&i) = index.get(&parse_date(&month)) {
            monthly[i] -= total;
        }
    }

    // The main category of each counterparty: the category with the most spending.
    let mut main: HashMap<String, (i64, String, usize)> = HashMap::new();
    let mut stmt = conn.prepare(&format!(
        "SELECT t.counterparty, c.name, sum(t.amount_minor) {SPENDING_FROM} \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} GROUP BY t.counterparty, t.category_id \
         ORDER BY t.counterparty, t.category_id"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(period_params(user_id, start, end)), |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?))
    })?;
    for row in rows {
        let (name, category, total) = row?;
        let spent = -total;
        let entry = main.entry(name.unwrap_or_default()).or_insert((0, String::new(), 0));
        if entry.2 == 0 || spent > entry.0 {
            entry.0 = spent;
            entry.1 = category.unwrap_or_else(|| "Uncategorised".into());
        }
        entry.2 += 1;
    }
    let group_label = |name: &str| match main.get(name) {
        Some((_, label, count)) if *count > 1 => format!("{label} and {} more", count - 1),
        Some((_, label, _)) => label.clone(),
        None => String::new(),
    };

    let grand_total: i64 = monthly_by_name.values().map(|m| m.iter().sum::<i64>()).sum();
    let mut by_total: Vec<(String, Vec<i64>)> =
        monthly_by_name.into_iter().filter(|(_, m)| m.iter().any(|v| *v != 0)).collect();
    by_total.sort_by(|a, b| {
        let (ta, tb) = (a.1.iter().sum::<i64>(), b.1.iter().sum::<i64>());
        tb.cmp(&ta).then_with(|| a.0.cmp(&b.0))
    });
    let rest = by_total.split_off(by_total.len().min(limit));
    let shown: Vec<SpendingSeries> = by_total
        .into_iter()
        .map(|(name, monthly)| {
            let group = group_label(&name);
            let (key, label) = if name.is_empty() {
                (None, NO_COUNTERPARTY.to_string())
            } else {
                (Some(name.clone()), name)
            };
            grid.series(key, label, group, monthly, grand_total, None)
        })
        .collect();
    let other = (!rest.is_empty()).then(|| {
        let monthly = (0..grid.months.len()).map(|i| rest.iter().map(|(_, m)| m[i]).sum()).collect();
        grid.series(
            None,
            format!("Other counterparties ({})", rest.len()),
            String::new(),
            monthly,
            grand_total,
            None,
        )
    });
    Ok(grid.comparison(shown, Vec::new(), other))
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterpartyTotal {
    pub name: String,
    pub count: i64,
    pub total: i64,
}

/// The counterparties with the most spending in one category (`None`: uncategorised).
pub fn top_counterparties(
    conn: &Connection,
    user_id: i64,
    category_id: Option<i64>,
    start: NaiveDate,
    end: NaiveDate,
    limit: usize,
) -> Result<Vec<CounterpartyTotal>> {
    let category_filter = match category_id {
        None => "(t.category_id IS NULL AND t.amount_minor < 0)".to_string(),
        Some(id) => format!("t.category_id = {id}"),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT t.counterparty, count(*), sum(t.amount_minor) \
         FROM transactions t JOIN accounts a ON t.account_id = a.id \
         WHERE {BUDGET_TX_FILTER} AND {category_filter} GROUP BY t.counterparty \
         ORDER BY sum(t.amount_minor) LIMIT {limit}"
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(period_params(user_id, start, end)), |row| {
        Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, count, amount) = row?;
        let name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| "—".into());
        out.push(CounterpartyTotal { name, count, total: -amount });
    }
    Ok(out)
}

/// The spending of one counterparty, also if it is not in the comparison table.
///
/// `share` is the part of the spending of all counterparties. Return `None` if the
/// counterparty has no spending in the period.
pub fn counterparty_series(
    conn: &Connection,
    user_id: i64,
    counterparty: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Option<SpendingSeries>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MONTH_EXPR}, t.counterparty = ?4, sum(t.amount_minor) {SPENDING_FROM} \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} GROUP BY {MONTH_EXPR}, t.counterparty = ?4"
    ))?;
    let grid = Grid::of(start, end);
    let index = grid.month_index();
    let mut monthly = vec![0; grid.months.len()];
    let mut grand_total = 0;
    let rows = stmt.query_map(params![user_id, date_text(start), date_text(end), counterparty], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, i64>(2)?))
    })?;
    for row in rows {
        let (month, selected, total) = row?;
        grand_total -= total;
        if selected && let Some(&i) = index.get(&parse_date(&month)) {
            monthly[i] -= total;
        }
    }
    if monthly.iter().all(|v| *v == 0) {
        return Ok(None);
    }
    Ok(Some(grid.series(
        Some(counterparty.to_string()),
        counterparty.to_string(),
        String::new(),
        monthly,
        grand_total,
        None,
    )))
}

#[derive(Debug, Clone, Serialize)]
pub struct CounterpartyCategory {
    pub category_id: Option<i64>,
    pub name: String,
    pub count: i64,
    pub total: i64,
}

/// The spending of one counterparty in each category, largest first.
pub fn counterparty_categories(
    conn: &Connection,
    user_id: i64,
    counterparty: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<CounterpartyCategory>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.category_id, c.name, count(*), sum(t.amount_minor) {SPENDING_FROM} \
         WHERE {BUDGET_TX_FILTER} AND {SPENDING_FILTER} AND t.counterparty = ?4 \
         GROUP BY t.category_id, c.name ORDER BY sum(t.amount_minor), c.name"
    ))?;
    let rows = stmt.query_map(params![user_id, date_text(start), date_text(end), counterparty], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (category_id, name, count, amount) = row?;
        out.push(CounterpartyCategory {
            category_id,
            name: name.unwrap_or_else(|| "Uncategorised".into()),
            count,
            total: -amount,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, 1).unwrap()
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket_months(12), 1);
        assert_eq!(bucket_months(13), 3);
        assert_eq!(bucket_months(37), 12);
        assert_eq!(bucket_start(d(2026, 5), 3), d(2026, 4));
        assert_eq!(bucket_start(d(2026, 5), 12), d(2026, 1));
    }

    #[test]
    fn trends() {
        assert_eq!(trend(&[100, 100, 100, 200, 200, 200]), (Some(200), Some(100.0)));
        assert_eq!(trend(&[0, 0, 0, 200, 200, 200]), (Some(200), None));
        assert_eq!(trend(&[1, 2, 3]), (None, None));
    }
}
