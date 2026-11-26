//! Transactions: list, categorise, manual entry.

use std::collections::{BTreeMap, HashSet};

use minijinja::context;
use minijinja::value::Value;
use rusqlite::types::Value as SqlValue;

use super::redirect;
use crate::iban::normalise_iban;
use crate::models::{Account, CategoryGroup, Transaction, TransactionSource, TransferMethod, TransferStatus};
use crate::money::{Decimal, exponent, parse_user_amount, parse_user_decimal};
use crate::services::accounts::{self, ManualTransaction};
use crate::services::categorise::{self, owned_transaction};
use crate::services::{Error as ServiceError, transfers};
use crate::web::render::urlencode_pairs;
use crate::web::{AppError, Auth, Ctx, Params, ParamsExt, Response, Result, parse_iso_date};

pub const PAGE_SIZE: i64 = 100;

/// The special values of the category filter. A number is a category ID.
const CATEGORY_FILTERS: &[(&str, &str)] = &[
    ("uncategorised", "— Uncategorised —"),
    ("transfers", "All transfers"),
    ("matched", "Transfers found from the amount"),
    ("unmatched", "Unmatched transfers (no partner)"),
];

/// The values of the direction filter.
const DIRECTIONS: &[(&str, &str)] = &[("outflow", "Outflows (−)"), ("inflow", "Inflows (+)")];

/// The sort orders of the transaction list. The empty value is the default.
const SORT_ORDERS: &[(&str, &str)] = &[
    ("", "Date, newest first"),
    ("amount_asc", "Amount, lowest first (largest outflows)"),
    ("amount_desc", "Amount, highest first (largest inflows)"),
    ("size_desc", "Amount without sign, largest first"),
];

/// Query parameters that the transaction list accepts. A redirect keeps only these.
const LIST_PARAMS: &[&str] = &[
    "account",
    "category",
    "uncategorised",
    "q",
    "direction",
    "min",
    "max",
    "page",
    "matched",
    "iban",
    "sort",
];

fn ordered_map(items: &[(&str, &str)]) -> Value {
    Value::from_pairs(items.iter().map(|(k, v)| (*k, *v)))
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// SQL conditions with their parameters.
#[derive(Default)]
struct Conditions {
    sql: Vec<String>,
    values: Vec<SqlValue>,
}

impl Conditions {
    /// Add a condition. `?` in `sql` is the next parameter.
    fn push(&mut self, sql: &str, values: Vec<SqlValue>) {
        let mut out = String::new();
        let mut values = values.into_iter();
        for ch in sql.chars() {
            if ch == '?' {
                self.values.push(values.next().expect("a value for each parameter"));
                out.push_str(&format!("?{}", self.values.len()));
            } else {
                out.push(ch);
            }
        }
        self.sql.push(out);
    }

    fn where_clause(&self) -> String {
        self.sql.join(" AND ")
    }
}

fn pair_ids(user_id: i64, method: Option<TransferMethod>) -> String {
    let method = method.map(|m| format!(" AND method = '{}'", m.as_str())).unwrap_or_default();
    let conditions =
        format!("user_id = {user_id} AND status = '{}'{method}", TransferStatus::Active.as_str());
    format!(
        "(t.id IN (SELECT outflow_id FROM transfer_pairs WHERE {conditions}) \
         OR t.id IN (SELECT inflow_id FROM transfer_pairs WHERE {conditions}))"
    )
}

/// The conditions of the filters of the transaction list.
struct ListFilter {
    conditions: Conditions,
    /// The query parameters of the filters, without the empty values.
    params: Vec<(String, String)>,
    category: String,
    direction: String,
    amount_min: String,
    amount_max: String,
    /// An error message for the amount filter, or `None`.
    error: Option<String>,
}

fn currencies(ctx: &Ctx<'_>, user_id: i64) -> Result<Vec<String>> {
    let mut stmt = ctx.db.prepare("SELECT DISTINCT currency FROM transactions WHERE user_id = ?1")?;
    let rows = stmt.query_map([user_id], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A limit of the amount filter. An empty text: no limit. The limit applies to the
/// amount without its sign.
fn amount_limit(text: &str) -> std::result::Result<Option<Decimal>, ()> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    parse_user_decimal(text).map(|d| Some(d.abs())).map_err(|_| ())
}

/// `value × 10^exp`, rounded up or down. `value` has a maximum of 3 decimal places.
fn scaled(value: &Decimal, exp: u32, up: bool) -> i128 {
    let thousandths = value.thousandths();
    let numerator = thousandths * 10i128.pow(exp);
    let floor = numerator.div_euclid(1000);
    if up && numerator.rem_euclid(1000) != 0 { floor + 1 } else { floor }
}

struct FilterInput<'a> {
    account: Option<i64>,
    category: &'a str,
    q: &'a str,
    direction: &'a str,
    amount_min: &'a str,
    amount_max: &'a str,
}

/// Make the conditions of the filters. Unknown values: no filter.
///
/// The transaction list and "Select all matching transactions" use this function. Thus
/// both use the same transactions.
fn list_filter(ctx: &Ctx<'_>, auth: &Auth, input: &FilterInput<'_>) -> Result<ListFilter> {
    let user_id = auth.user.id;
    let mut category = input.category.to_string();
    if !(CATEGORY_FILTERS.iter().any(|(k, _)| *k == category) || is_digits(&category)) {
        category.clear();
    }
    let mut c = Conditions::default();
    c.push("t.user_id = ?", vec![user_id.into()]);
    if let Some(account) = input.account {
        c.push("t.account_id = ?", vec![account.into()]);
    }
    match category.as_str() {
        "uncategorised" => c.push("t.category_id IS NULL AND t.is_transfer = 0 AND a.on_budget = 1", vec![]),
        "transfers" => c.push("t.is_transfer = 1", vec![]),
        "matched" => c.push(
            &format!("t.is_transfer = 1 AND {}", pair_ids(user_id, Some(TransferMethod::Match))),
            vec![],
        ),
        "unmatched" => c.push(&format!("t.is_transfer = 1 AND NOT {}", pair_ids(user_id, None)), vec![]),
        "" => {}
        digits => match digits.parse::<i64>() {
            Ok(id) => c.push("t.category_id = ?", vec![id.into()]),
            Err(_) => c.push("0", vec![]),
        },
    }
    let direction = if DIRECTIONS.iter().any(|(k, _)| *k == input.direction) { input.direction } else { "" };
    match direction {
        "outflow" => c.push("t.amount_minor < 0", vec![]),
        "inflow" => c.push("t.amount_minor > 0", vec![]),
        _ => {}
    }
    let mut error = None;
    let (mut amount_min, mut amount_max) = (input.amount_min.to_string(), input.amount_max.to_string());
    let (minimum, maximum) = match (amount_limit(&amount_min), amount_limit(&amount_max)) {
        (Ok(min), Ok(max)) => (min, max),
        _ => {
            error = Some(
                "The amount filter is not valid. Type an amount, for example 300 or 1'250.50.".to_string(),
            );
            amount_min.clear();
            amount_max.clear();
            (None, None)
        }
    };
    if let (Some(min), Some(max)) = (&minimum, &maximum)
        && min.thousandths() > max.thousandths()
    {
        error = Some("The minimum amount is more than the maximum amount.".to_string());
    }
    if minimum.is_some() || maximum.is_some() {
        // The limits are in the currency of each transaction. Thus the filter makes one
        // condition for each currency of the user.
        let mut alternatives = Vec::new();
        let mut values = Vec::new();
        for currency in currencies(ctx, user_id)? {
            let exp = exponent(&currency);
            let mut parts = vec!["t.currency = ?".to_string()];
            values.push(SqlValue::from(currency));
            if let Some(min) = &minimum {
                parts.push("abs(t.amount_minor) >= ?".into());
                values.push(SqlValue::from(
                    scaled(min, exp, true).clamp(i64::MIN as i128, i64::MAX as i128) as i64
                ));
            }
            if let Some(max) = &maximum {
                parts.push("abs(t.amount_minor) <= ?".into());
                values.push(SqlValue::from(
                    scaled(max, exp, false).clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                ));
            }
            alternatives.push(format!("({})", parts.join(" AND ")));
        }
        if alternatives.is_empty() {
            c.push("0", vec![]);
        } else {
            c.push(&format!("({})", alternatives.join(" OR ")), values);
        }
    }
    let q = input.q.trim();
    if !q.is_empty() {
        let pattern = format!("%{}%", escape_like(q));
        let mut sql = "(lower(t.counterparty) LIKE lower(?) ESCAPE '\\' \
             OR lower(t.description) LIKE lower(?) ESCAPE '\\' \
             OR lower(t.memo) LIKE lower(?) ESCAPE '\\'"
            .to_string();
        let mut values: Vec<SqlValue> = vec![pattern.clone().into(), pattern.clone().into(), pattern.into()];
        // The IBANs are stored without spaces: "CH93 0076" finds "CH9300762…". Only a
        // text with a digit and 4 or more characters can be part of an IBAN. Else a
        // search for "ch" finds all Swiss IBANs.
        let compact = normalise_iban(input.q);
        if compact.chars().count() >= 4 && compact.chars().any(|ch| ch.is_ascii_digit()) {
            sql.push_str(" OR lower(t.counterparty_iban) LIKE lower(?) ESCAPE '\\'");
            values.push(format!("%{}%", escape_like(&compact)).into());
        }
        sql.push(')');
        c.push(&sql, values);
    }
    let mut params = Vec::new();
    if let Some(account) = input.account {
        params.push(("account".to_string(), account.to_string()));
    }
    for (name, value) in [
        ("category", category.as_str()),
        ("q", input.q),
        ("direction", direction),
        ("min", amount_min.trim()),
        ("max", amount_max.trim()),
    ] {
        if !value.is_empty() {
            params.push((name.to_string(), value.to_string()));
        }
    }
    Ok(ListFilter {
        conditions: c,
        params,
        category,
        direction: direction.to_string(),
        amount_min: amount_min.trim().to_string(),
        amount_max: amount_max.trim().to_string(),
        error,
    })
}

const FROM: &str = "FROM transactions t JOIN accounts a ON t.account_id = a.id";

fn count(ctx: &Ctx<'_>, filter: &ListFilter, extra: &str) -> Result<i64> {
    let sql = format!("SELECT count(*) {FROM} WHERE {}{extra}", filter.conditions.where_clause());
    Ok(ctx
        .db
        .query_row(&sql, rusqlite::params_from_iter(filter.conditions.values.iter()), |row| row.get(0))?)
}

/// The sort order of the transaction list. An unknown value: newest first.
///
/// The amounts are in minor units, and the currencies can have a different number of
/// decimal places. Thus the sort key changes each amount to the same scale (3 decimal
/// places). Transactions with the same amount are newest first.
fn order_by(ctx: &Ctx<'_>, auth: &Auth, sort: &str) -> Result<String> {
    let newest = "t.booking_date DESC, t.id DESC";
    if sort.is_empty() || !SORT_ORDERS.iter().any(|(k, _)| *k == sort) {
        return Ok(newest.into());
    }
    let factors: Vec<(String, i64)> = currencies(ctx, auth.user.id)?
        .into_iter()
        .filter(|c| exponent(c) != 2)
        .map(|c| {
            let factor = 10i64.pow(3 - exponent(&c));
            (c, factor)
        })
        .collect();
    let scale = if factors.is_empty() {
        "10".to_string()
    } else {
        let cases: String =
            factors.iter().map(|(c, f)| format!(" WHEN '{}' THEN {f}", c.replace('\'', "''"))).collect();
        format!("(CASE t.currency{cases} ELSE 10 END)")
    };
    let key = format!("(t.amount_minor * {scale})");
    Ok(match sort {
        "amount_asc" => format!("{key} ASC, {newest}"),
        "amount_desc" => format!("{key} DESC, {newest}"),
        _ => format!("abs({key}) DESC, {newest}"),
    })
}

/// The values of a transaction for the templates.
fn tx_view(tx: &Transaction, account: Option<&Account>) -> Value {
    let tx_value = crate::web::ser(tx);
    let account = account.map(|a| context! { id => a.id, name => a.name.clone(), on_budget => a.on_budget });
    context! { ..context! { account => account }, ..tx_value }
}

pub fn transactions_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let query = ctx.query.clone();
    let account = query.optional_id("account")?;
    let mut category = query.text("category", 20)?;
    let q = query.text("q", 100)?;
    let direction = query.text("direction", 10)?;
    let amount_min = query.text("min", 32)?;
    let amount_max = query.text("max", 32)?;
    let mut sort = query.text("sort", 20)?;
    let page = query.int("page")?.unwrap_or(1);
    if !(1..=100_000).contains(&page) {
        return Err(AppError::Invalid);
    }
    let show_iban = query.boolean("iban", false)?;
    // The parameters of older links. They are the same as `category=uncategorised` and
    // `category=matched`.
    if query.boolean("uncategorised", false)? {
        category = "uncategorised".into();
    } else if query.boolean("matched", false)? {
        category = "matched".into();
    }
    let input = FilterInput {
        account,
        category: &category,
        q: &q,
        direction: &direction,
        amount_min: &amount_min,
        amount_max: &amount_max,
    };
    let found = list_filter(ctx, &auth, &input)?;
    let total = count(ctx, &found, "")?;
    // The transactions that "Select all matching transactions" changes.
    let selectable = count(ctx, &found, " AND a.on_budget = 1")?;
    let sql = format!(
        "SELECT {} {FROM} WHERE {} ORDER BY {} LIMIT {PAGE_SIZE} OFFSET {}",
        Transaction::COLUMNS.replace("transactions.", "t."),
        found.conditions.where_clause(),
        order_by(ctx, &auth, &sort)?,
        (page - 1) * PAGE_SIZE
    );
    let mut stmt = ctx.db.prepare(&sql)?;
    let items: Vec<Transaction> = stmt
        .query_map(rusqlite::params_from_iter(found.conditions.values.iter()), Transaction::from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let account_list = accounts::list_accounts(ctx.db, auth.user.id, true)?;
    let by_id: BTreeMap<i64, &Account> = account_list.iter().map(|a| (a.id, a)).collect();
    let mut partners = BTreeMap::new();
    let ids: Vec<i64> = items.iter().map(|t| t.id).collect();
    for (tx_id, partner) in transfers::partners(ctx.db, &ids)? {
        let account = match by_id.get(&partner.tx.account_id) {
            Some(a) => Some((*a).clone()),
            None => Account::get(ctx.db, partner.tx.account_id)?,
        };
        partners.insert(
            tx_id,
            context! { tx => tx_view(&partner.tx, account.as_ref()), method => partner.method.as_str() },
        );
    }
    let groups = CategoryGroup::list(ctx.db, auth.user.id)?;
    if !SORT_ORDERS.iter().any(|(k, _)| *k == sort) {
        sort.clear();
    }
    // Display options, not filters: "Remove the filters" keeps them.
    let mut display: Vec<(String, String)> = Vec::new();
    if !sort.is_empty() {
        display.push(("sort".into(), sort.clone()));
    }
    if show_iban {
        display.push(("iban".into(), "true".into()));
    }
    let all_params: Vec<(String, String)> = found.params.iter().chain(display.iter()).cloned().collect();
    let mut return_params = all_params.clone();
    return_params.push(("page".into(), page.to_string()));
    let views: Vec<Value> = items.iter().map(|t| tx_view(t, by_id.get(&t.account_id).copied())).collect();
    let account_views: Vec<Value> = account_list
        .iter()
        .map(|a| context! { id => a.id, name => a.name.clone(), closed => a.closed })
        .collect();
    let filter_params = Value::from_pairs(found.params.iter().map(|(k, v)| (k.clone(), v.clone())));
    ctx.render(
        Some(&auth),
        "transactions.html",
        200,
        context! {
            transactions => views,
            partners => Value::from_pairs(partners),
            total => total,
            page => page,
            pages => ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1),
            query_string => urlencode_pairs(&all_params),
            filters_active => !found.params.is_empty(),
            display_query => urlencode_pairs(&display),
            show_iban => show_iban,
            sort => sort,
            sort_orders => ordered_map(SORT_ORDERS),
            return_to => format!("/transactions?{}", urlencode_pairs(&return_params)),
            accounts => account_views,
            groups => crate::web::ser(&groups),
            filter_account => account,
            filter_category => found.category,
            category_filters => ordered_map(CATEGORY_FILTERS),
            q => q,
            filter_direction => found.direction,
            directions => ordered_map(DIRECTIONS),
            amount_min => found.amount_min,
            amount_max => found.amount_max,
            amount_error => found.error,
            filter_params => filter_params,
            selectable => selectable,
            today => chrono::Local::now().date_naive().format("%Y-%m-%d").to_string(),
        },
    )
}

/// Go back to the transaction list with the same filters.
///
/// The redirect target is always the local path `/transactions`. Only known query
/// parameters are copied from `return_to`. This prevents an open redirect.
fn back(return_to: &str) -> Result<Response> {
    let without_fragment = return_to.split('#').next().unwrap_or("");
    let query = without_fragment.split_once('?').map_or("", |(_, q)| q);
    let params: Vec<(String, String)> = Params::from_query(query)
        .pairs
        .into_iter()
        .filter(|(k, v)| !v.is_empty() && LIST_PARAMS.contains(&k.as_str()))
        .collect();
    let query = urlencode_pairs(&params);
    redirect(&if query.is_empty() { "/transactions".to_string() } else { format!("/transactions?{query}") })
}

fn return_to(form: &Params) -> Result<String> {
    Ok(form.get("return_to").map(str::to_string).unwrap_or_else(|| "/transactions".into()))
        .and_then(|v| if v.chars().count() > 500 { Err(AppError::Invalid) } else { Ok(v) })
}

pub fn set_category(ctx: &mut Ctx<'_>, tx_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let category_id = form.text("category_id", 20)?;
    let target = return_to(form)?;
    let tx = owned_transaction(ctx.db, auth.user.id, tx_id?)?;
    if category_id == "transfer" {
        categorise::mark_transfer(ctx.db, tx.id)?;
        return back(&target);
    }
    let parsed = if category_id.is_empty() {
        Ok(None)
    } else {
        crate::web::parse_int(&category_id).map(Some).ok_or(())
    };
    let result = match parsed {
        Err(()) => Err(ServiceError::service("The category is not valid.")),
        Ok(id) => categorise::set_category(ctx.db, &tx, id),
    };
    match result {
        Ok(Some(partner)) => {
            let account = Account::get(ctx.db, partner.account_id)?.map(|a| a.name).unwrap_or_default();
            ctx.flash(
                &auth,
                &format!(
                    "This transaction and the transaction of {} on '{account}' are not a transfer any \
                     more. Give a category to the other transaction too.",
                    partner.booking_date.format("%d.%m.%Y")
                ),
                false,
            )?;
        }
        Ok(None) => {}
        Err(ServiceError::Service(message)) => ctx.flash(&auth, &message, true)?,
        Err(other) => return Err(other.into()),
    }
    back(&target)
}

/// Give the category `target` (or "Transfer") to each transaction. Return a message.
fn assign_category(
    ctx: &Ctx<'_>,
    selected: &[Transaction],
    to_transfer: bool,
    target: Option<i64>,
) -> Result<String> {
    let selected_ids: HashSet<i64> = selected.iter().map(|t| t.id).collect();
    let mut changed = 0;
    let mut tracking = 0;
    let mut partners = HashSet::new();
    for tx in selected {
        let on_budget = Account::get(ctx.db, tx.account_id)?.is_some_and(|a| a.on_budget);
        if !on_budget {
            tracking += 1;
            continue;
        }
        if to_transfer {
            categorise::mark_transfer(ctx.db, tx.id)?;
        } else {
            // Read the transaction again: an earlier change can change it.
            let current = Transaction::get(ctx.db, tx.id)?.ok_or(AppError::NotFound)?;
            if let Some(partner) = categorise::set_category(ctx.db, &current, target)?
                && !selected_ids.contains(&partner.id)
            {
                partners.insert(partner.id);
            }
        }
        changed += 1;
    }
    let mut messages = vec![format!("The category of {changed} transactions is changed.")];
    if tracking > 0 {
        messages.push(format!("{tracking} transactions are in tracking accounts. They have no category."));
    }
    if !partners.is_empty() {
        messages.push(format!(
            "{} other transactions are not transfers any more. Give a category to them too. \
             Select 'Uncategorised' in the category filter.",
            partners.len()
        ));
    }
    Ok(messages.join(" "))
}

/// Give one category to the selected transactions.
///
/// `category_id` is a category ID, `none` (uncategorised) or `transfer`. The changes
/// are the same as the changes of [`set_category`] for each transaction. Transactions
/// in tracking accounts have no category. The function does not change them.
///
/// With `scope=all`, the function ignores `tx_id`. It changes all the transactions of
/// budget accounts that agree with the filters (`account` to `max`, the same as the
/// transaction list). `expected` is the number of these transactions that the page
/// showed. If the number is different now, the function changes nothing.
pub fn set_category_bulk(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?.clone();
    let tx_ids: Vec<i64> = form
        .all("tx_id")
        .iter()
        .map(|v| crate::web::parse_int(v).ok_or(AppError::Invalid))
        .collect::<Result<_>>()?;
    let category_id = form.text("category_id", 20)?;
    let target_url = return_to(&form)?;
    let scope = form.text("scope", 10)?;
    let expected = form.optional_id("expected")?;
    let account = form.optional_id("account")?;
    let category = form.text("category", 20)?;
    let q = form.text("q", 100)?;
    let direction = form.text("direction", 10)?;
    let amount_min = form.text("min", 32)?;
    let amount_max = form.text("max", 32)?;

    let to_transfer = category_id == "transfer";
    let mut target = None;
    if !to_transfer && category_id != "none" {
        if !is_digits(&category_id) {
            ctx.flash(&auth, "Select a category.", true)?;
            return back(&target_url);
        }
        let id = category_id.parse().map_err(|_| AppError::NotFound)?;
        target = Some(categorise::owned_category(ctx.db, auth.user.id, id)?.id);
    }
    let selected: Vec<Transaction> = if scope == "all" {
        let input = FilterInput {
            account,
            category: &category,
            q: &q,
            direction: &direction,
            amount_min: &amount_min,
            amount_max: &amount_max,
        };
        let found = list_filter(ctx, &auth, &input)?;
        if let Some(error) = &found.error {
            ctx.flash(&auth, error, true)?;
            return back(&target_url);
        }
        let sql = format!(
            "SELECT {} {FROM} WHERE {} AND a.on_budget = 1 ORDER BY t.id",
            Transaction::COLUMNS.replace("transactions.", "t."),
            found.conditions.where_clause()
        );
        let mut stmt = ctx.db.prepare(&sql)?;
        let rows: Vec<Transaction> = stmt
            .query_map(rusqlite::params_from_iter(found.conditions.values.iter()), Transaction::from_row)?
            .collect::<rusqlite::Result<_>>()?;
        if Some(rows.len() as i64) != expected {
            ctx.flash(
                &auth,
                "The matching transactions changed after the page was shown. Nothing is changed. \
                 Examine the list and do the change again.",
                true,
            )?;
            return back(&target_url);
        }
        rows
    } else {
        let mut ids: Vec<i64> = Vec::new();
        for id in tx_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() > PAGE_SIZE as usize {
            ctx.flash(&auth, &format!("Select a maximum of {PAGE_SIZE} transactions."), true)?;
            return back(&target_url);
        }
        let mut rows = Vec::new();
        for id in &ids {
            match Transaction::get(ctx.db, *id)? {
                Some(tx) if tx.user_id == auth.user.id => rows.push(tx),
                _ => return Err(AppError::NotFound),
            }
        }
        rows.sort_by_key(|t| t.id);
        rows
    };
    if selected.is_empty() {
        ctx.flash(&auth, "Select one or more transactions.", true)?;
        return back(&target_url);
    }
    let message = assign_category(ctx, &selected, to_transfer, target)?;
    ctx.flash(&auth, &message, false)?;
    back(&target_url)
}

pub fn set_memo(ctx: &mut Ctx<'_>, tx_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let memo = form.text("memo", 500)?;
    let target = return_to(form)?;
    let tx = owned_transaction(ctx.db, auth.user.id, tx_id?)?;
    ctx.db
        .execute("UPDATE transactions SET memo = ?1 WHERE id = ?2", rusqlite::params![memo.trim(), tx.id])?;
    back(&target)
}

pub fn delete_transaction(ctx: &mut Ctx<'_>, tx_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let target = return_to(ctx.form()?)?;
    let tx = owned_transaction(ctx.db, auth.user.id, tx_id?)?;
    if tx.source != TransactionSource::Manual {
        ctx.flash(&auth, "You can only delete transactions that you added manually.", true)?;
    } else {
        transfers::before_delete(ctx.db, tx.id)?;
        ctx.db.execute("DELETE FROM transactions WHERE id = ?1", [tx.id])?;
        ctx.flash(&auth, "The transaction is deleted.", false)?;
    }
    back(&target)
}

pub fn add_transaction(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let account_id = form.required_int("account_id")?;
    let booking_date = parse_iso_date(&form.required("booking_date", 10)?).ok_or(AppError::Invalid)?;
    let amount = form.required("amount", 32)?;
    let direction = form.required("direction", 10)?;
    if direction != "outflow" && direction != "inflow" {
        return Err(AppError::Invalid);
    }
    let counterparty = form.text("counterparty", 140)?;
    let memo = form.text("memo", 500)?;
    let category_id = form.text("category_id", 20)?;
    let account = accounts::get_account(ctx.db, auth.user.id, account_id)?;
    let result =
        parse_user_amount(&amount, &account.currency).map_err(ServiceError::from).and_then(|value| {
            let value = if direction == "outflow" { -value.abs() } else { value.abs() };
            let manual = ManualTransaction {
                booking_date,
                amount_minor: value,
                counterparty: &counterparty,
                memo: &memo,
                category_id: if is_digits(&category_id) { category_id.parse().ok() } else { None },
            };
            accounts::add_manual_transaction(ctx.db, &account, &manual)
        });
    match result {
        Ok(_) => {
            ctx.flash(&auth, "The transaction is added.", false)?;
            redirect(&format!("/transactions?account={}", account.id))
        }
        Err(ServiceError::Service(message)) => {
            ctx.flash(&auth, &message, true)?;
            redirect("/transactions")
        }
        Err(other) => Err(other.into()),
    }
}
