//! Reports: visualisations and statistics.

use chrono::{Datelike, NaiveDate};
use minijinja::context;
use minijinja::value::Value;

use crate::money::format_minor;
use crate::services::budget::{add_months, last_day_of_month, month_start};
use crate::services::reports::{self, MAX_COUNTERPARTIES, SpendingSeries};
use crate::web::charts::{self, Chart, MAX_X_LABELS};
use crate::web::render::month_short;
use crate::web::{AppError, Auth, Ctx, ParamsExt, Response, Result, parse_iso_date};

const PERIODS: &[(&str, i32, &str)] =
    &[("3m", 3, "3 months"), ("6m", 6, "6 months"), ("12m", 12, "12 months"), ("24m", 24, "24 months")];
/// The period from the month of the first transaction to the current month.
const ALL_PERIOD: &str = "all";
const DEFAULT_PERIOD: &str = "12m";
/// The maximum length of a custom period (20 years). This limits the size of the charts.
pub const MAX_MONTHS: i32 = 240;

fn min_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1900, 1, 1).expect("valid")
}

/// Later dates are not possible: the month calculations must stay below year 10000.
fn max_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(9998, 12, 31).expect("valid")
}

fn period_labels() -> Value {
    Value::from_pairs(PERIODS.iter().map(|(k, _, l)| (*k, *l)).chain([(ALL_PERIOD, "All")]))
}

fn is_period(value: &str) -> bool {
    value == ALL_PERIOD || PERIODS.iter().any(|(k, ..)| *k == value)
}

fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// The range of a preset period: the last full months, including the current month.
fn preset_range(period: &str, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let months = PERIODS.iter().find(|(k, ..)| *k == period).map_or(12, |(_, m, _)| *m);
    let end_month = month_start(today);
    (add_months(end_month, -(months - 1)), last_day_of_month(end_month))
}

/// The range of the "All" period and `true` if the range was made shorter.
///
/// The range starts in the month of the first transaction. It stops at the end of the
/// current month, or at the end of the month of the last transaction if that is later.
/// If the range is longer than [`MAX_MONTHS`], only the last [`MAX_MONTHS`] months are
/// used. If there are no transactions, the range is the current month.
pub fn all_range(
    data_range: Option<(NaiveDate, NaiveDate)>,
    today: NaiveDate,
) -> (NaiveDate, NaiveDate, bool) {
    let end_month = month_start(today);
    let Some((first, last)) = data_range else {
        return (end_month, last_day_of_month(end_month), false);
    };
    let end_month = end_month.max(month_start(last.min(max_date())));
    let start = month_start(first).max(month_start(min_date()));
    let earliest = add_months(end_month, -(MAX_MONTHS - 1));
    (start.max(earliest), last_day_of_month(end_month), start < earliest)
}

/// The range of a custom period. Return an error message if the range is not valid.
///
/// If there is no end date, the end is today. If there is no start date, the start is
/// the first day of the month 11 months before the end month.
pub fn custom_range(
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
    today: NaiveDate,
) -> std::result::Result<(NaiveDate, NaiveDate), String> {
    let end = end.unwrap_or(today);
    let in_range = |d: NaiveDate| min_date() <= d && d <= max_date();
    if !in_range(end) || !start.is_none_or(in_range) {
        return Err(format!(
            "Type dates from {} to {}.",
            min_date().format("%d.%m.%Y"),
            max_date().format("%d.%m.%Y")
        ));
    }
    let start = start.unwrap_or_else(|| add_months(month_start(end), -11).max(min_date()));
    if start > end {
        return Err("The start date must be on or before the end date.".into());
    }
    let months = (end.year() - start.year()) * 12 + end.month0() as i32 - start.month0() as i32 + 1;
    if months > MAX_MONTHS {
        return Err(format!("The period must be a maximum of {} years.", MAX_MONTHS / 12));
    }
    Ok((start, end))
}

/// The period of a report page, from the query parameters.
struct ReportPeriod {
    start: NaiveDate,
    end: NaiveDate,
    /// The preset period, or `None` for a custom period.
    period: Option<String>,
    error: Option<String>,
    shortened: bool,
    form_from: String,
    form_to: String,
}

impl ReportPeriod {
    /// The query parameters that select this period on a different report page.
    fn query(&self) -> Value {
        match &self.period {
            None => Value::from_pairs([
                ("from", self.start.format("%Y-%m-%d").to_string()),
                ("to", self.end.format("%Y-%m-%d").to_string()),
            ]),
            Some(period) => Value::from_pairs([("period", period.clone())]),
        }
    }

    fn context(&self) -> Value {
        let period = self.period.clone().unwrap_or_else(|| DEFAULT_PERIOD.into());
        let label = PERIODS.iter().find(|(k, ..)| *k == period).map_or("All", |(_, _, l)| l);
        context! {
            period => self.period.clone(),
            period_label => label,
            periods => period_labels(),
            custom => self.period.is_none(),
            error => self.error.clone(),
            shortened => self.shortened,
            max_years => MAX_MONTHS / 12,
            form_from => self.form_from.clone(),
            form_to => self.form_to.clone(),
            start => self.start.format("%Y-%m-%d").to_string(),
            end => self.end.format("%Y-%m-%d").to_string(),
            period_query => self.query(),
        }
    }

    fn status(&self) -> u16 {
        if self.error.is_some() { 400 } else { 200 }
    }
}

fn resolve_period(ctx: &Ctx<'_>, auth: &Auth) -> Result<ReportPeriod> {
    let mut period = ctx.query.text("period", 3)?;
    let date_from = ctx
        .query
        .get("from")
        .filter(|v| !v.is_empty())
        .map(|v| parse_iso_date(v).ok_or(AppError::Invalid))
        .transpose()?;
    let date_to = ctx
        .query
        .get("to")
        .filter(|v| !v.is_empty())
        .map(|v| parse_iso_date(v).ok_or(AppError::Invalid))
        .transpose()?;
    let today = today();
    let mut error = None;
    let mut custom = date_from.is_some() || date_to.is_some();
    let mut shortened = false;
    if !is_period(&period) {
        period = DEFAULT_PERIOD.into();
    }
    let mut range = None;
    if custom {
        match custom_range(date_from, date_to, today) {
            Ok(r) => range = Some(r),
            Err(message) => {
                error = Some(message);
                custom = false;
            }
        }
    }
    let (start, end) = match range {
        Some(r) => r,
        None if period == ALL_PERIOD => {
            let data_range = reports::transaction_date_range(ctx.db, auth.user.id)?;
            let (s, e, short) = all_range(data_range, today);
            shortened = short;
            (s, e)
        }
        None => preset_range(&period, today),
    };
    let iso = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    Ok(ReportPeriod {
        start,
        end,
        period: (!custom).then_some(period),
        error,
        shortened,
        form_from: date_from.map_or_else(|| iso(start), iso),
        form_to: date_to.map_or_else(|| iso(end), iso),
    })
}

/// The label step and offset for a chart with one value for each month.
///
/// The labels are on calendar boundaries, for example January, April, July and October
/// for a step of 3 months.
fn month_labels(months: &[NaiveDate]) -> (usize, usize) {
    let step = charts::month_label_step(months.len(), MAX_X_LABELS);
    match months.first() {
        None => (step, 0),
        Some(first) => (step, (first.year() * 12 + first.month0() as i32) as usize),
    }
}

/// Format axis labels in whole currency units (for example whole francs).
fn axis_formatter(currency: &str) -> impl Fn(f64) -> String + '_ {
    move |value: f64| {
        let text = format_minor(value.trunc() as i64, currency, false);
        match text.rsplit_once('.') {
            Some((whole, _)) => whole.to_string(),
            None => text,
        }
    }
}

pub fn reports_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let chosen = resolve_period(ctx, &auth)?;
    let (start, end) = (chosen.start, chosen.end);
    let currency = ctx.settings().budget_currency.clone();
    let fmt_axis = axis_formatter(&currency);
    let flows = reports::monthly_flows(ctx.db, auth.user.id, start, end)?;
    let labels: Vec<String> = flows.iter().map(|f| month_short(f.month)).collect();
    let months: Vec<NaiveDate> = flows.iter().map(|f| f.month).collect();
    let (label_step, label_offset) = month_labels(&months);
    let flow_chart = charts::grouped_bars(
        &labels,
        &[
            ("bar-income", flows.iter().map(|f| f.income).collect()),
            ("bar-expense", flows.iter().map(|f| -f.expenses).collect()),
        ],
        &fmt_axis,
        label_step,
        label_offset,
    );
    let spending = reports::spending_by_category(ctx.db, auth.user.id, start, end)?;
    let money_fmt = |v: f64| format_minor(v.trunc() as i64, &currency, false);
    let spending_rows = charts::horizontal_bars(
        &spending.iter().take(15).map(|s| (s.name.clone(), s.total)).collect::<Vec<_>>(),
        &money_fmt,
    );
    let mut worth_charts = Vec::new();
    for (cur, series) in reports::net_worth(ctx.db, auth.user.id, start, end)? {
        let fmt = axis_formatter(&cur);
        let chart = charts::line(
            &series.iter().map(|(m, _)| month_short(*m)).collect::<Vec<_>>(),
            &series.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
            &fmt,
            label_step,
            label_offset,
        );
        worth_charts.push((cur.clone(), crate::web::ser(&chart)));
    }
    let page = context! {
        keep => Value::from_pairs(Vec::<(String, String)>::new()),
        flows => crate::web::ser(&flows),
        flow_chart => crate::web::ser(&flow_chart),
        spending_rows => crate::web::ser(&spending_rows),
        spending_total => spending.iter().map(|s| s.total).sum::<i64>(),
        summary => crate::web::ser(reports::summary(&flows)),
        worth_charts => Value::from_pairs(worth_charts),
    };
    ctx.render(Some(&auth), "reports.html", chosen.status(), context! { ..page, ..chosen.context() })
}

const SORTS: &[(&str, &str)] =
    &[("budget", "Budget order"), ("total", "Largest total"), ("change", "Largest increase")];
const COUNTERPARTY_SORTS: &[(&str, &str)] = &[("total", "Largest total"), ("change", "Largest increase")];

fn bucket_label(start: NaiveDate, size: u32) -> String {
    match size {
        12 => start.year().to_string(),
        3 => format!("Q{} {:02}", start.month0() / 3 + 1, start.year().rem_euclid(100)),
        _ => month_short(start),
    }
}

pub fn sorted_rows(rows: &[SpendingSeries], sort: &str) -> Vec<SpendingSeries> {
    let mut rows = rows.to_vec();
    match sort {
        "total" => rows.sort_by_key(|r| std::cmp::Reverse(r.total)),
        "change" => rows.sort_by(|a, b| {
            let key = |r: &SpendingSeries| (r.change_percent.is_none(), -(r.change_percent.unwrap_or(0.0)));
            let (ka, kb) = (key(a), key(b));
            ka.0.cmp(&kb.0).then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
        }),
        _ => {}
    }
    rows
}

fn month_chart(months: &[NaiveDate], monthly: &[i64], currency: &str) -> Chart {
    let (label_step, label_offset) = month_labels(months);
    let fmt = axis_formatter(currency);
    charts::grouped_bars(
        &months.iter().map(|m| month_short(*m)).collect::<Vec<_>>(),
        &[("bar-expense", monthly.to_vec())],
        &fmt,
        label_step,
        label_offset,
    )
}

fn ordered(items: &[(&str, &str)]) -> Value {
    Value::from_pairs(items.iter().map(|(k, v)| (*k, *v)))
}

fn keep(sort: &str, extra: Option<(&str, String)>) -> Value {
    let mut pairs = vec![("sort".to_string(), sort.to_string())];
    if let Some((k, v)) = extra {
        pairs.push((k.to_string(), v));
    }
    Value::from_pairs(pairs)
}

pub fn category_report_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let chosen = resolve_period(ctx, &auth)?;
    let mut sort = ctx.query.text("sort", 10)?;
    if !SORTS.iter().any(|(k, _)| *k == sort) {
        sort = "budget".into();
    }
    let category = ctx.query.text("category", 20)?;
    let currency = ctx.settings().budget_currency.clone();
    let data = reports::compare_categories(ctx.db, auth.user.id, chosen.start, chosen.end)?;
    let selected =
        data.rows.iter().find(|r| !category.is_empty() && r.key.as_deref() == Some(category.as_str()));
    let detail = match selected {
        Some(row) => Some(context! {
            row => crate::web::ser(row),
            chart => crate::web::ser(month_chart(&data.months, &row.monthly, &currency)),
            counterparties => crate::web::ser(reports::top_counterparties(
                ctx.db, auth.user.id, row.category_id, chosen.start, chosen.end, 15,
            )?),
        }),
        None => None,
    };
    let page = context! {
        data => crate::web::ser(&data),
        rows => crate::web::ser(sorted_rows(&data.rows, &sort)),
        sort => sort.clone(),
        sorts => ordered(SORTS),
        bucket_labels => data.buckets.iter().map(|b| bucket_label(*b, data.bucket_months)).collect::<Vec<_>>(),
        keep => keep(&sort, detail.is_some().then(|| ("category", category.clone()))),
        detail => detail,
    };
    ctx.render(
        Some(&auth),
        "report_categories.html",
        chosen.status(),
        context! { ..page, ..chosen.context() },
    )
}

pub fn counterparty_report_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let chosen = resolve_period(ctx, &auth)?;
    let mut sort = ctx.query.text("sort", 10)?;
    if !COUNTERPARTY_SORTS.iter().any(|(k, _)| *k == sort) {
        sort = "total".into();
    }
    let counterparty = ctx.query.text("counterparty", 140)?;
    let currency = ctx.settings().budget_currency.clone();
    let data =
        reports::compare_counterparties(ctx.db, auth.user.id, chosen.start, chosen.end, MAX_COUNTERPARTIES)?;
    let mut detail = None;
    if !counterparty.is_empty() {
        // The table shows only the largest counterparties. A link from a different page
        // can select any counterparty.
        let selected = match data.rows.iter().find(|r| r.key.as_deref() == Some(counterparty.as_str())) {
            Some(row) => Some(row.clone()),
            None => {
                reports::counterparty_series(ctx.db, auth.user.id, &counterparty, chosen.start, chosen.end)?
            }
        };
        if let Some(row) = selected {
            detail = Some(context! {
                chart => crate::web::ser(month_chart(&data.months, &row.monthly, &currency)),
                categories => crate::web::ser(reports::counterparty_categories(
                    ctx.db, auth.user.id, &counterparty, chosen.start, chosen.end,
                )?),
                row => crate::web::ser(&row),
            });
        }
    }
    let page = context! {
        data => crate::web::ser(&data),
        rows => crate::web::ser(sorted_rows(&data.rows, &sort)),
        sort => sort.clone(),
        sorts => ordered(COUNTERPARTY_SORTS),
        bucket_labels => data.buckets.iter().map(|b| bucket_label(*b, data.bucket_months)).collect::<Vec<_>>(),
        keep => keep(&sort, detail.is_some().then(|| ("counterparty", counterparty.clone()))),
        detail => detail,
    };
    ctx.render(
        Some(&auth),
        "report_counterparties.html",
        chosen.status(),
        context! { ..page, ..chosen.context() },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn custom_ranges() {
        let today = d(2026, 11, 26);
        assert_eq!(custom_range(None, None, today), Ok((d(2025, 12, 1), today)));
        assert_eq!(
            custom_range(Some(d(2026, 1, 5)), Some(d(2026, 2, 1)), today),
            Ok((d(2026, 1, 5), d(2026, 2, 1)))
        );
        assert!(custom_range(Some(d(2026, 3, 1)), Some(d(2026, 2, 1)), today).is_err());
        assert!(custom_range(Some(d(1800, 1, 1)), None, today).is_err());
        assert!(
            custom_range(Some(d(2000, 1, 1)), Some(d(2026, 1, 1)), today).unwrap_err().contains("20 years")
        );
    }

    #[test]
    fn all_ranges() {
        let today = d(2026, 11, 26);
        assert_eq!(all_range(None, today), (d(2026, 11, 1), d(2026, 11, 30), false));
        assert_eq!(
            all_range(Some((d(2026, 2, 3), d(2026, 5, 1))), today),
            (d(2026, 2, 1), d(2026, 11, 30), false)
        );
        let (start, _, shortened) = all_range(Some((d(1990, 1, 1), d(2026, 5, 1))), today);
        assert!(shortened);
        assert_eq!(start, d(2006, 12, 1));
    }

    #[test]
    fn bucket_labels() {
        assert_eq!(bucket_label(d(2026, 4, 1), 3), "Q2 26");
        assert_eq!(bucket_label(d(2026, 1, 1), 12), "2026");
        assert_eq!(bucket_label(d(2026, 1, 1), 1), "Jan 26");
    }
}
