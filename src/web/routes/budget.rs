//! The budget page.

use minijinja::context;

use super::redirect;
use crate::money::parse_user_amount;
use crate::services::Error as ServiceError;
use crate::services::budget::{self, add_months, month_param, month_start};
use crate::web::{AppError, Ctx, ParamsExt, Response, Result};

fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

pub fn budget_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let month = ctx.query.text("month", 7)?;
    let selected = budget::parse_month(Some(&month), today()).unwrap_or_else(|_| month_start(today()));
    let data = budget::month_budget(ctx.db, auth.user.id, selected)?;
    ctx.render(
        Some(&auth),
        "budget.html",
        200,
        context! {
            data => crate::web::ser(&data),
            month_param => month_param(selected),
            previous => month_param(add_months(selected, -1)),
            next => month_param(add_months(selected, 1)),
        },
    )
}

fn valid_month_syntax(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit)
}

pub fn assign(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let category_id = form.required_int("category_id")?;
    let month = form.required("month", 7)?;
    if !valid_month_syntax(&month) {
        return Err(AppError::Invalid);
    }
    let amount = form.required("amount", 32)?;
    let selected = budget::parse_month(Some(&month), today()).unwrap_or_else(|_| month_start(today()));
    // Build the redirect only from the parsed date, never from the raw form value.
    let target = format!("/budget?month={}", month_param(selected));
    let currency = ctx.settings().budget_currency.clone();
    let result = parse_user_amount(if amount.is_empty() { "0" } else { &amount }, &currency)
        .map_err(ServiceError::from)
        .and_then(|value| budget::assign(ctx.db, auth.user.id, category_id, selected, value));
    match result {
        // The anchor uses the ID from the database, not the form value.
        Ok(category) => redirect(&format!("{target}#cat-{}", category.id)),
        Err(ServiceError::Service(message)) => {
            ctx.flash(&auth, &message, true)?;
            redirect(&target)
        }
        Err(other) => Err(other.into()),
    }
}
