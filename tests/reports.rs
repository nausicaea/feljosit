//! Tests of the reports: income and expenses, spending by category and counterparty.

mod common;

use std::collections::HashMap;

use chrono::{Datelike, NaiveDate};
use common::*;
use feljosit::models::{Account, AccountType};
use feljosit::services::accounts::{self, ManualTransaction};
use feljosit::services::budget::add_months;
use feljosit::services::reports;
use feljosit::web::charts;
use feljosit::web::render::heat;
use feljosit::web::routes::reports::{all_range, custom_range, sorted_rows};

fn setup() -> (rusqlite::Connection, feljosit::models::User, Account, HashMap<String, i64>) {
    let (conn, user) = database_with_user();
    let account = plain_account(&conn, &user, "Private", AccountType::Checking, "CHF");
    let mut stmt = conn.prepare("SELECT name, id FROM categories WHERE user_id = ?1").unwrap();
    let cats =
        stmt.query_map([user.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    drop(stmt);
    (conn, user, account, cats)
}

fn spend(
    conn: &rusqlite::Connection,
    account: &Account,
    on: NaiveDate,
    amount: i64,
    category: Option<i64>,
    counterparty: &str,
) {
    let manual = ManualTransaction {
        booking_date: on,
        amount_minor: amount,
        counterparty,
        memo: "",
        category_id: category,
    };
    accounts::add_manual_transaction(conn, account, &manual).unwrap();
}

fn day(month: NaiveDate, d: u32) -> NaiveDate {
    month.with_day(d).unwrap()
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn flows_spending_and_net_worth() {
    let (conn, user, account, cats) = setup();
    for (on, amount, cat) in [
        (date(2026, 1, 25), 500000, Some("Salary")),
        (date(2026, 1, 3), -200000, Some("Rent and service charges")),
        (date(2026, 2, 3), -50000, Some("Groceries")),
        (date(2026, 2, 4), -1000, None),
        (date(2026, 2, 5), 300, None),
    ] {
        spend(&conn, &account, on, amount, cat.map(|c| cats[c]), "x");
    }
    let (start, end) = (date(2026, 1, 1), date(2026, 3, 31));
    let flows = reports::monthly_flows(&conn, user.id, start, end).unwrap();
    let pairs: Vec<(i64, i64)> = flows.iter().map(|f| (f.income, f.expenses)).collect();
    assert_eq!(pairs, vec![(500000, -200000), (300, -51000), (0, 0)]);
    let spending = reports::spending_by_category(&conn, user.id, start, end).unwrap();
    let rows: Vec<(String, i64)> = spending.iter().map(|s| (s.name.clone(), s.total)).collect();
    assert_eq!(
        rows,
        vec![
            ("Rent and service charges".into(), 200000),
            ("Groceries".into(), 50000),
            ("Uncategorised".into(), 1000)
        ]
    );
    let summary = reports::summary(&flows);
    assert_eq!((summary.savings_rate_percent.unwrap() * 10.0).round() / 10.0, 49.8);
    let worth = reports::net_worth(&conn, user.id, start, end).unwrap();
    let values: Vec<i64> = worth["CHF"].iter().map(|(_, v)| *v).collect();
    assert_eq!(values, vec![300000, 249300, 249300]);
}

#[test]
fn charts_geometry() {
    let fmt = |v: f64| v.to_string();
    let labels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let chart =
        charts::grouped_bars(&labels(&["a", "b"]), &[("x", vec![100, -50]), ("y", vec![0, 30])], &fmt, 1, 0);
    assert_eq!(chart.bars.len(), 4);
    assert!(chart.bars.iter().all(|b| b.height >= 0.0));
    let line = charts::line(&labels(&["a", "b", "c"]), &[1, 5, 3], &fmt, 1, 0);
    assert_eq!(line.line_points.split(' ').count(), 3);
    let rows = charts::horizontal_bars(&[("a".into(), 50), ("b".into(), 25)], &fmt);
    assert_eq!((rows[0].percent, rows[1].percent), (100.0, 50.0));
    assert_eq!((rows[0].share * 10.0).round() / 10.0, 66.7);
    assert!(charts::horizontal_bars(&[], &fmt).is_empty());
}

#[test]
fn custom_ranges() {
    let today = date(2026, 11, 26);
    assert_eq!(
        custom_range(Some(date(2026, 1, 15)), Some(date(2026, 3, 10)), today),
        Ok((date(2026, 1, 15), date(2026, 3, 10)))
    );
    assert_eq!(custom_range(Some(date(2026, 5, 1)), None, today), Ok((date(2026, 5, 1), today)));
    assert_eq!(custom_range(None, Some(date(2026, 6, 30)), today), Ok((date(2025, 7, 1), date(2026, 6, 30))));
    assert_eq!(
        custom_range(Some(date(2006, 12, 1)), Some(date(2026, 11, 30)), today).unwrap().0,
        date(2006, 12, 1)
    );
    assert!(
        custom_range(Some(date(2026, 3, 1)), Some(date(2026, 2, 1)), today)
            .unwrap_err()
            .contains("on or before")
    );
    assert!(
        custom_range(Some(date(2006, 11, 30)), Some(date(2026, 11, 30)), today)
            .unwrap_err()
            .contains("maximum of 20 years")
    );
    assert!(
        custom_range(Some(date(1, 1, 1)), Some(date(2026, 1, 1)), today).unwrap_err().contains("Type dates")
    );
    assert!(custom_range(None, Some(date(9999, 12, 31)), today).unwrap_err().contains("Type dates"));
}

#[test]
fn flows_with_partial_months() {
    let (conn, user, account, _) = setup();
    for (on, amount) in [(date(2026, 1, 5), -100), (date(2026, 1, 20), -200), (date(2026, 2, 25), -400)] {
        spend(&conn, &account, on, amount, None, "x");
    }
    let flows = reports::monthly_flows(&conn, user.id, date(2026, 1, 10), date(2026, 2, 20)).unwrap();
    let pairs: Vec<(NaiveDate, i64)> = flows.iter().map(|f| (f.month, f.expenses)).collect();
    assert_eq!(pairs, vec![(date(2026, 1, 1), -200), (date(2026, 2, 1), 0)]);
    let worth = reports::net_worth(&conn, user.id, date(2026, 1, 10), date(2026, 2, 20)).unwrap();
    assert_eq!(worth["CHF"].iter().map(|(_, v)| *v).collect::<Vec<_>>(), vec![-300, -300]);
}

#[test]
fn all_ranges() {
    let (conn, user, account, _) = setup();
    let today = date(2026, 11, 26);
    assert_eq!(reports::transaction_date_range(&conn, user.id).unwrap(), None);
    assert_eq!(all_range(None, today), (date(2026, 11, 1), date(2026, 11, 30), false));
    for on in [date(2024, 3, 15), date(2025, 6, 1)] {
        spend(&conn, &account, on, -100, None, "x");
    }
    let range = reports::transaction_date_range(&conn, user.id).unwrap();
    assert_eq!(range, Some((date(2024, 3, 15), date(2025, 6, 1))));
    assert_eq!(all_range(range, today), (date(2024, 3, 1), date(2026, 11, 30), false));
    // A transaction after the current month extends the range.
    assert_eq!(all_range(Some((date(2024, 3, 15), date(2027, 2, 3))), today).1, date(2027, 2, 28));
    // More than 20 years: only the last 20 years.
    assert_eq!(
        all_range(Some((date(1990, 1, 1), today)), today),
        (date(2006, 12, 1), date(2026, 11, 30), true)
    );
}

#[test]
fn month_label_step() {
    for (count, step) in
        [(1, 1), (12, 1), (13, 2), (24, 2), (25, 3), (36, 3), (37, 6), (72, 6), (73, 12), (240, 24)]
    {
        assert_eq!(charts::month_label_step(count, charts::MAX_X_LABELS), step, "{count}");
    }
}

#[test]
fn x_labels_are_thinned_on_calendar_boundaries() {
    // 31 months from June 2024 to December 2026: one label for each quarter.
    let months: Vec<String> = (0..31).map(|i| format!("m{i}")).collect();
    let offset = 2024 * 12 + 5;
    let step = charts::month_label_step(months.len(), charts::MAX_X_LABELS);
    assert_eq!(step, 3);
    let fmt = |v: f64| v.to_string();
    let expected: Vec<String> = (1..31).step_by(3).map(|i| format!("m{i}")).collect();
    let bars = charts::grouped_bars(&months, &[("x", (0..31).collect())], &fmt, step, offset);
    assert_eq!(bars.x_labels.iter().map(|l| l.label.clone()).collect::<Vec<_>>(), expected);
    assert_eq!(bars.bars.len(), 31); // All bars stay.
    let line = charts::line(&months, &(0..31).collect::<Vec<_>>(), &fmt, step, offset);
    assert_eq!(line.x_labels.iter().map(|l| l.label.clone()).collect::<Vec<_>>(), expected);
    assert_eq!(line.line_dots.len(), 31);
    assert!(bars.x_labels.len() <= charts::MAX_X_LABELS && line.x_labels.len() <= charts::MAX_X_LABELS);
}

#[test]
fn bucket_months_and_start() {
    for (months, size) in [(1, 1), (12, 1), (13, 3), (36, 3), (37, 12)] {
        assert_eq!(reports::bucket_months(months), size);
    }
    assert_eq!(reports::bucket_start(date(2026, 5, 1), 1), date(2026, 5, 1));
    assert_eq!(reports::bucket_start(date(2026, 5, 1), 3), date(2026, 4, 1));
    assert_eq!(reports::bucket_start(date(2026, 12, 1), 3), date(2026, 10, 1));
    assert_eq!(reports::bucket_start(date(2026, 5, 1), 12), date(2026, 1, 1));
}

#[test]
fn compare_categories_per_month() {
    let (conn, user, account, cats) = setup();
    let (groceries, eating_out) = (cats["Groceries"], cats["Eating out"]);
    let start = date(2026, 1, 1);
    for i in 0..6 {
        let month = add_months(start, i);
        spend(&conn, &account, day(month, 5), -10000, Some(groceries), "x");
        // Eating out: 100.00 in the first 3 months, 200.00 in the last 3 months.
        spend(&conn, &account, day(month, 9), if i < 3 { -10000 } else { -20000 }, Some(eating_out), "x");
    }
    spend(&conn, &account, date(2026, 2, 10), 2000, Some(groceries), "x"); // A refund.
    spend(&conn, &account, date(2026, 3, 1), -5000, None, "Dentist"); // Uncategorised.
    spend(&conn, &account, date(2026, 3, 2), 9000, None, "x"); // An inflow: not spending.
    spend(&conn, &account, date(2026, 3, 3), 500000, Some(cats["Salary"]), "x"); // Income.

    let data = reports::compare_categories(&conn, user.id, start, date(2026, 6, 30)).unwrap();
    assert_eq!(data.bucket_months, 1);
    assert!(data.buckets.len() == 6 && data.has_trend);
    let by_name: HashMap<&str, &reports::SpendingSeries> =
        data.rows.iter().map(|r| (r.name.as_str(), r)).collect();
    let mut names: Vec<&&str> = by_name.keys().collect();
    names.sort();
    assert_eq!(names, vec![&"Eating out", &"Groceries", &"Uncategorised"]);

    let g = by_name["Groceries"];
    assert_eq!(g.monthly, vec![10000, 8000, 10000, 10000, 10000, 10000]);
    assert_eq!(g.values, g.monthly);
    assert_eq!(g.total, 58000);
    assert_eq!(g.average_per_month, 9667);
    assert_eq!(g.recent_average, Some(10000));
    assert_eq!(g.change_percent.unwrap().round(), 7.0);

    let e = by_name["Eating out"];
    assert_eq!(e.recent_average, Some(20000));
    assert!(approx(e.change_percent.unwrap(), 100.0));

    let u = by_name["Uncategorised"];
    assert_eq!(u.category_id, None);
    assert_eq!(u.monthly, vec![0, 0, 5000, 0, 0, 0]);
    assert!(approx(u.change_percent.unwrap(), -100.0)); // 0.00 recently, 16.67 before.
    assert_eq!(u.recent_average, Some(0));

    assert_eq!(data.total, 58000 + 90000 + 5000);
    assert!(approx(data.rows.iter().map(|r| r.share).sum(), 100.0));
    let totals: Vec<i64> =
        (0..data.buckets.len()).map(|i| data.rows.iter().map(|r| r.values[i]).sum()).collect();
    assert_eq!(data.totals, totals);
    // Budget order: the groups of the budget, "Uncategorised" last.
    assert_eq!(data.rows.last().unwrap().category_id, None);
    assert!(data.groups.last().unwrap().uncategorised);
    assert!(data.groups[..data.groups.len() - 1].iter().all(|g| !g.uncategorised));
}

#[test]
fn compare_categories_without_trend_and_empty() {
    let (conn, user, account, cats) = setup();
    spend(&conn, &account, date(2026, 1, 5), -1000, Some(cats["Groceries"]), "x");
    let data = reports::compare_categories(&conn, user.id, date(2026, 1, 1), date(2026, 3, 31)).unwrap();
    assert!(!data.has_trend);
    assert!(data.rows[0].change_percent.is_none() && data.rows[0].recent_average.is_none());
    let empty = reports::compare_categories(&conn, user.id, date(2020, 1, 1), date(2020, 12, 31)).unwrap();
    assert!(empty.rows.is_empty() && empty.total == 0);
}

#[test]
fn compare_categories_per_quarter_uses_average_per_month() {
    let (conn, user, account, cats) = setup();
    // 18 months from March 2025: the first quarter has 1 month, the last has 2 months.
    let start = date(2025, 3, 1);
    for i in 0..18 {
        spend(&conn, &account, day(add_months(start, i), 5), -30000, Some(cats["Groceries"]), "x");
    }
    let data = reports::compare_categories(&conn, user.id, start, date(2026, 8, 31)).unwrap();
    assert_eq!(data.bucket_months, 3);
    assert_eq!(data.buckets[0], date(2025, 1, 1));
    assert_eq!(*data.buckets.last().unwrap(), date(2026, 7, 1));
    // The same spending each month gives the same value in each quarter.
    let [row] = data.rows.as_slice() else { panic!() };
    assert!(row.values.iter().all(|v| *v == 30000));
    assert_eq!(row.total, 18 * 30000);
}

#[test]
fn top_counterparties() {
    let (conn, user, account, cats) = setup();
    let groceries = cats["Groceries"];
    for (amount, name) in [(-3000, "Coop"), (-2000, "Migros"), (-4000, "Migros"), (500, "Coop")] {
        spend(&conn, &account, date(2026, 1, 10), amount, Some(groceries), name);
    }
    spend(&conn, &account, date(2026, 1, 11), -7000, None, "");
    let (start, end) = (date(2026, 1, 1), date(2026, 1, 31));
    let result = reports::top_counterparties(&conn, user.id, Some(groceries), start, end, 15).unwrap();
    let rows: Vec<(String, i64, i64)> = result.iter().map(|c| (c.name.clone(), c.count, c.total)).collect();
    assert_eq!(rows, vec![("Migros".into(), 2, 6000), ("Coop".into(), 2, 2500)]);
    let uncategorised = reports::top_counterparties(&conn, user.id, None, start, end, 15).unwrap();
    assert_eq!(
        uncategorised.iter().map(|c| (c.name.clone(), c.total)).collect::<Vec<_>>(),
        vec![("—".into(), 7000)]
    );
}

#[test]
fn heat_levels() {
    for (value, average, level) in [
        (100, 100, 0),
        (90, 100, 0),
        (109, 100, 0),
        (110, 100, 1),
        (125, 100, 2),
        (150, 100, 3),
        (200, 100, 4),
        (500, 100, 4),
        (100, 0, 0),
        (0, 0, 0),
    ] {
        assert_eq!(heat(value, average), level, "{value} {average}");
    }
}

#[test]
fn sort_orders() {
    let (conn, user, account, cats) = setup();
    for i in 0..6 {
        let month = add_months(date(2026, 1, 1), i);
        spend(&conn, &account, month, -50000, Some(cats["Rent and service charges"]), "x");
        spend(&conn, &account, month, if i < 3 { -1000 } else { -3000 }, Some(cats["Eating out"]), "x");
    }
    let data = reports::compare_categories(&conn, user.id, date(2026, 1, 1), date(2026, 6, 30)).unwrap();
    assert_eq!(sorted_rows(&data.rows, "total")[0].name, "Rent and service charges");
    assert_eq!(sorted_rows(&data.rows, "change")[0].name, "Eating out");
    let budget: Vec<String> = sorted_rows(&data.rows, "budget").into_iter().map(|r| r.name).collect();
    assert_eq!(budget, data.rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>());
}

fn page_setup(counterparty: &str) -> (TestApp, Client, HashMap<String, i64>) {
    let test = TestApp::new();
    let (http, _) = test.anna();
    let cats = test.app.with_db(|conn| {
        let user = feljosit::models::User::by_username(conn, "anna").unwrap().unwrap();
        let account = plain_account(conn, &user, "Private", AccountType::Checking, "CHF");
        let mut stmt = conn.prepare("SELECT name, id FROM categories WHERE user_id = ?1").unwrap();
        let cats: HashMap<String, i64> =
            stmt.query_map([user.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        let today = chrono::Local::now().date_naive().with_day(1).unwrap();
        for i in 0..6 {
            spend(conn, &account, add_months(today, -i), -12345, Some(cats["Groceries"]), counterparty);
        }
        cats
    });
    (test, http, cats)
}

#[test]
fn category_report_page() {
    let (_test, mut http, cats) = page_setup("Migros");
    let page = http.get("/reports/categories?period=6m");
    assert_eq!(page.status, 200);
    assert!(page.text().contains("Groceries") && page.text().contains("123.45"));
    assert!(page.text().contains("Last 3 months"));
    let response = http.get(&format!("/reports/categories?period=6m&category={}", cats["Groceries"]));
    assert_eq!(response.status, 200);
    assert!(response.text().contains(r#"id="detail""#) && response.text().contains("Migros"));
    // Unknown values do not cause errors.
    assert_eq!(http.get("/reports/categories?sort=abc&category=999999").status, 200);
    assert_eq!(http.get("/reports/categories?category=uncategorised").status, 200);
    assert_eq!(http.get("/reports/categories?from=2026-01-01&to=2025-01-01").status, 400);
    // The report tabs keep the period.
    assert!(http.get("/reports?period=24m").text().contains(r#"href="/reports/categories?period=24m""#));
}

#[test]
fn compare_counterparties() {
    let (conn, user, account, cats) = setup();
    let (groceries, eating_out) = (cats["Groceries"], cats["Eating out"]);
    let start = date(2026, 1, 1);
    for i in 0..6 {
        let month = add_months(start, i);
        spend(&conn, &account, day(month, 3), -20000, Some(groceries), "Migros");
        spend(&conn, &account, day(month, 4), if i < 3 { -10000 } else { -15000 }, Some(groceries), "Coop");
        spend(&conn, &account, day(month, 5), -1000, Some(eating_out), "Coop"); // The Coop café.
        spend(&conn, &account, day(month, 6), -500, Some(eating_out), &format!("Kiosk {i}"));
    }
    spend(&conn, &account, date(2026, 2, 10), 3000, Some(groceries), "Migros"); // A refund.
    spend(&conn, &account, date(2026, 3, 1), -4000, None, ""); // No counterparty.
    spend(&conn, &account, date(2026, 3, 2), 500000, Some(cats["Salary"]), "Muster AG"); // Income.

    let end = date(2026, 6, 30);
    let data = reports::compare_counterparties(&conn, user.id, start, end, 3).unwrap();
    let names: Vec<&str> = data.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["Migros", "Coop", reports::NO_COUNTERPARTY]);
    let [migros, coop, nobody] = data.rows.as_slice() else { panic!() };
    assert_eq!(migros.total, 6 * 20000 - 3000);
    assert_eq!(migros.group, "Groceries");
    assert_eq!(coop.group, "Groceries and 1 more");
    assert_eq!(coop.monthly, vec![11000, 11000, 11000, 16000, 16000, 16000]);
    assert!(approx(coop.change_percent.unwrap(), 16000.0 / 11000.0 * 100.0 - 100.0));
    assert_eq!(nobody.key, None); // Cannot be selected.
    // The six kiosks are one row.
    let other = data.other.as_ref().unwrap();
    assert_eq!(other.name, "Other counterparties (6)");
    assert_eq!(other.total, 3000);
    // The total is the same as in the category comparison.
    let categories = reports::compare_categories(&conn, user.id, start, end).unwrap();
    assert_eq!(data.total, categories.total);
    assert_eq!(data.totals, categories.totals);
    assert!(approx(data.rows.iter().chain([other]).map(|r| r.share).sum(), 100.0));
}

#[test]
fn counterparty_series_and_categories() {
    let (conn, user, account, cats) = setup();
    for i in 0..3 {
        let month = add_months(date(2026, 1, 1), i);
        spend(&conn, &account, month, -20000, Some(cats["Groceries"]), "Coop");
        spend(&conn, &account, month, -1000, Some(cats["Eating out"]), "Coop");
        spend(&conn, &account, month, -9000, Some(cats["Groceries"]), "Migros");
    }
    let (start, end) = (date(2026, 1, 1), date(2026, 3, 31));
    let series = reports::counterparty_series(&conn, user.id, "Coop", start, end).unwrap().unwrap();
    assert_eq!(series.monthly, vec![21000, 21000, 21000]);
    assert!(approx(series.share, 70.0));
    assert!(reports::counterparty_series(&conn, user.id, "Nobody", start, end).unwrap().is_none());
    let result = reports::counterparty_categories(&conn, user.id, "Coop", start, end).unwrap();
    let rows: Vec<(String, i64, i64)> = result.iter().map(|c| (c.name.clone(), c.count, c.total)).collect();
    assert_eq!(rows, vec![("Groceries".into(), 3, 60000), ("Eating out".into(), 3, 3000)]);
}

#[test]
fn counterparty_report_page() {
    let (_test, mut http, cats) = page_setup("Migros & Co");
    let page = http.get("/reports/counterparties?period=6m");
    assert_eq!(page.status, 200);
    assert!(page.text().contains("Migros &amp; Co") && page.text().contains("123.45"));
    // The name is encoded in the link.
    assert!(page.text().contains("counterparty=Migros+%26+Co"));
    let detail = http.get("/reports/counterparties?period=6m&counterparty=Migros+%26+Co");
    assert!(detail.text().contains(r#"id="detail""#) && detail.text().contains("Groceries"));
    // An unknown counterparty or sort order does not cause an error.
    assert_eq!(http.get("/reports/counterparties?counterparty=Nobody&sort=x").status, 200);
    let too_long = http.get(&format!("/reports/counterparties?counterparty={}", "x".repeat(141)));
    assert_eq!(too_long.status, 422);
    // The category details link to the counterparty details.
    let category = http.get(&format!("/reports/categories?period=6m&category={}", cats["Groceries"])).text();
    assert!(category.contains("/reports/counterparties?counterparty=Migros+%26+Co"), "{category}");
    assert!(category.contains(r#"href="/reports/counterparties?period=6m""#));
}
