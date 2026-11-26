//! Template rendering (minijinja, a Jinja2 implementation).
//!
//! The templates are in the directory `templates`. The build puts them into the
//! executable. Thus the application is one file.

use crate::dates::{self, Date};
use std::fmt::Write as _;

use minijinja::value::{Kwargs, Value, ValueKind};
use minijinja::{AutoEscape, Environment, Error, ErrorKind, State};

use crate::iban::format_iban;
use crate::money::format_minor;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const TEMPLATES: &[(&str, &str)] = &[
    ("base.html", include_str!("../../templates/base.html")),
    ("macros.html", include_str!("../../templates/macros.html")),
    ("_report_head.html", include_str!("../../templates/_report_head.html")),
    ("_spending_macros.html", include_str!("../../templates/_spending_macros.html")),
    ("account_edit.html", include_str!("../../templates/account_edit.html")),
    ("accounts.html", include_str!("../../templates/accounts.html")),
    ("budget.html", include_str!("../../templates/budget.html")),
    ("categories.html", include_str!("../../templates/categories.html")),
    ("import.html", include_str!("../../templates/import.html")),
    ("login.html", include_str!("../../templates/login.html")),
    ("report_categories.html", include_str!("../../templates/report_categories.html")),
    ("report_counterparties.html", include_str!("../../templates/report_counterparties.html")),
    ("reports.html", include_str!("../../templates/reports.html")),
    ("rule_edit.html", include_str!("../../templates/rule_edit.html")),
    ("settings.html", include_str!("../../templates/settings.html")),
    ("setup.html", include_str!("../../templates/setup.html")),
    ("transactions.html", include_str!("../../templates/transactions.html")),
];

pub const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub fn month_label(value: Date) -> String {
    format!("{} {}", MONTH_NAMES[dates::month0(value) as usize], dates::year(value))
}

pub fn month_short(value: Date) -> String {
    format!("{} {:02}", &MONTH_NAMES[dates::month0(value) as usize][..3], dates::year(value).rem_euclid(100))
}

/// Escape a text for HTML, as MarkupSafe (the escaping of Jinja2) does it.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn date_of(value: &Value) -> Option<Date> {
    let text = value.as_str()?;
    dates::parse(text.get(..10)?, "%Y-%m-%d")
}

fn number(value: &Value) -> Result<f64, Error> {
    if let Some(i) = value.as_i64() {
        return Ok(i as f64);
    }
    f64::try_from(value.clone())
        .map_err(|_| Error::new(ErrorKind::InvalidOperation, "the value is not a number"))
}

fn money(value: Value, currency: Option<String>) -> Result<String, Error> {
    if value.is_none() || value.is_undefined() {
        return Ok(String::new());
    }
    let amount = match value.as_i64() {
        Some(i) => i,
        None => number(&value)?.trunc() as i64,
    };
    Ok(format_minor(amount, currency.as_deref().unwrap_or("CHF"), false))
}

fn swiss_date(value: Value) -> String {
    date_of(&value).map(|d| crate::dates::format(d, "%d.%m.%Y")).unwrap_or_default()
}

fn month_label_filter(value: Value) -> String {
    date_of(&value).map(month_label).unwrap_or_default()
}

fn month_short_filter(value: Value) -> String {
    date_of(&value).map(month_short).unwrap_or_default()
}

fn iban_filter(value: String) -> String {
    format_iban(&value)
}

/// `'%.Nf' % value` of Python.
fn fixed(value: Value, digits: Option<usize>) -> Result<String, Error> {
    Ok(format!("{:.*}", digits.unwrap_or(1), number(&value)?))
}

/// `'%.Nf %%' % value` of Python, for example `12.5 %`.
fn pct(value: Value, digits: Option<usize>) -> Result<String, Error> {
    Ok(format!("{} %", fixed(value, digits)?))
}

/// `'%+d %%' % value if value else '0 %'` of Python, for example `+12 %`.
fn signed_pct(value: Value) -> Result<String, Error> {
    let v = number(&value)?.trunc() as i64;
    Ok(if v == 0 { "0 %".into() } else { format!("{v:+} %") })
}

/// Encode a text for a query string, as Python's `quote_plus`.
pub fn quote_plus(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => out.push(char::from(b)),
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// A query string from pairs, as Python's `urlencode`.
pub fn urlencode_pairs<K: AsRef<str>, V: AsRef<str>>(pairs: &[(K, V)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", quote_plus(k.as_ref()), quote_plus(v.as_ref())))
        .collect::<Vec<_>>()
        .join("&")
}

fn value_text(value: &Value) -> String {
    match value.kind() {
        ValueKind::None | ValueKind::Undefined => String::new(),
        _ => value.as_str().map_or_else(|| value.to_string(), str::to_string),
    }
}

/// The `urlencode` filter: a map gives a query string. A text is quoted.
fn urlencode(value: Value) -> Result<String, Error> {
    if value.kind() == ValueKind::Map {
        let mut pairs = Vec::new();
        for key in value.try_iter()? {
            let item = value.get_item(&key)?;
            pairs.push((value_text(&key), value_text(&item)));
        }
        return Ok(urlencode_pairs(&pairs));
    }
    Ok(quote_plus(&value_text(&value)).replace("%2F", "/"))
}

/// A new map with the items of `value` and then the items of `other`.
fn merge(value: Value, other: Value) -> Result<Value, Error> {
    let mut pairs: Vec<(Value, Value)> = Vec::new();
    for source in [&value, &other] {
        if source.is_undefined() || source.is_none() {
            continue;
        }
        for key in source.try_iter()? {
            let item = source.get_item(&key)?;
            match pairs.iter_mut().find(|(k, _)| *k == key) {
                Some(existing) => existing.1 = item,
                None => pairs.push((key, item)),
            }
        }
    }
    Ok(Value::from_pairs(pairs))
}

fn formatter(out: &mut minijinja::Output, state: &mut State, value: &Value) -> Result<(), Error> {
    if value.is_safe() || matches!(state.auto_escape(), AutoEscape::None) {
        return minijinja::escape_formatter(out, state, value);
    }
    if value.is_undefined() {
        return Ok(());
    }
    out.write_str(&escape_html(&value.to_string())).map_err(Error::from)
}

/// The heat level from 0 to 4: how much a cell is above the average of its row.
pub fn heat(value: i64, average: i64) -> i64 {
    const HEAT_STEPS: [f64; 4] = [0.10, 0.25, 0.50, 1.00];
    if average <= 0 || value <= average {
        return 0;
    }
    let above = value as f64 / average as f64 - 1.0;
    HEAT_STEPS.iter().filter(|step| above >= **step).count() as i64
}

fn heat_function(value: i64, average: i64, _kwargs: Kwargs) -> i64 {
    heat(value, average)
}

/// Make the template environment.
pub fn environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .trim_blocks(true)
            .lstrip_blocks(true)
            .build()
            .expect("The syntax configuration is valid"),
    );
    env.set_formatter(formatter);
    env.add_filter("money", money);
    env.add_filter("iban", iban_filter);
    env.add_filter("month_label", month_label_filter);
    env.add_filter("month_short", month_short_filter);
    env.add_filter("swiss_date", swiss_date);
    env.add_filter("fixed", fixed);
    env.add_filter("pct", pct);
    env.add_filter("signed_pct", signed_pct);
    env.add_filter("urlencode", urlencode);
    env.add_filter("merge", merge);
    env.add_function("heat", heat_function);
    env.add_global("version", VERSION);
    for (name, source) in TEMPLATES {
        env.add_template(name, source).unwrap_or_else(|e| panic!("The template {name} is not valid: {e:#}"));
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_templates_compile() {
        let env = environment();
        for (name, _) in TEMPLATES {
            assert!(env.get_template(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn query_strings() {
        assert_eq!(quote_plus("a b/c&ü"), "a+b%2Fc%26%C3%BC");
        assert_eq!(urlencode_pairs(&[("q", "Migros AG"), ("page", "2")]), "q=Migros+AG&page=2");
    }

    #[test]
    fn escaping() {
        assert_eq!(escape_html("<a href='x'>\"&"), "&lt;a href=&#39;x&#39;&gt;&#34;&amp;");
    }

    #[test]
    fn labels() {
        let d = crate::dates::ymd(2026, 11, 1).unwrap();
        assert_eq!(month_label(d), "November 2026");
        assert_eq!(month_short(d), "Nov 26");
    }

    #[test]
    fn heat_levels() {
        assert_eq!(heat(100, 100), 0);
        assert_eq!(heat(110, 100), 1);
        assert_eq!(heat(200, 100), 4);
        assert_eq!(heat(100, 0), 0);
    }
}
