//! Accounts.

use minijinja::context;
use minijinja::value::Value;

use super::redirect;
use crate::models::AccountType;
use crate::money::{parse_user_amount, validate_currency};
use crate::services::accounts::{self, NewAccount};
use crate::services::{Error as ServiceError, Result as ServiceResult};
use crate::web::{AppError, Ctx, ParamsExt, Response, Result};

fn account_types() -> Value {
    Value::from_iter(AccountType::ALL.iter().map(|t| context! { value => t.as_str(), label => t.label() }))
}

fn account_type(value: &str) -> Result<AccountType> {
    AccountType::parse(value).ok_or(AppError::Invalid)
}

fn budget_mode(value: &str) -> Option<bool> {
    match value {
        "budget" => Some(true),
        "tracking" => Some(false),
        _ => None,
    }
}

fn account_view(account: &crate::models::Account) -> Value {
    context! {
        id => account.id,
        name => account.name.clone(),
        type => account.account_type.as_str(),
        type_label => account.account_type.label(),
        on_budget => account.on_budget,
        currency => account.currency.clone(),
        iban => account.iban.clone(),
        closed => account.closed,
    }
}

pub fn accounts_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let items = accounts::list_accounts(ctx.db, auth.user.id, true)?;
    let totals = accounts::balances(ctx.db, auth.user.id)?;
    let views: Vec<Value> = items
        .iter()
        .map(|a| {
            let view = account_view(a);
            context! { ..view, ..context! { balance => totals.get(&a.id).copied().unwrap_or(0) } }
        })
        .collect();
    ctx.render(
        Some(&auth),
        "accounts.html",
        200,
        context! {
            accounts => views,
            account_types => account_types(),
            today => crate::dates::format(crate::dates::today(), "%Y-%m-%d"),
        },
    )
}

pub fn create_account(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let name = form.required("name", 100)?;
    let kind = account_type(&form.required("account_type", 20)?)?;
    let currency = form.get("currency").map(str::to_string).unwrap_or_else(|| "CHF".into());
    if currency.chars().count() > 3 {
        return Err(AppError::Invalid);
    }
    let iban = form.text("iban", 50)?;
    let mode = form.get("budget_mode").unwrap_or("default").to_string();
    if mode.chars().count() > 10 {
        return Err(AppError::Invalid);
    }
    let starting_balance = form.text("starting_balance", 32)?;
    let starting_date = form.date("starting_date")?;
    let budget_currency = ctx.settings().budget_currency.clone();
    let result: ServiceResult<crate::models::Account> = (|| {
        let code = validate_currency(&currency)?;
        let start =
            if starting_balance.trim().is_empty() { 0 } else { parse_user_amount(&starting_balance, &code)? };
        let mut new = NewAccount::new(&name, kind, &code);
        new.budget_currency = &budget_currency;
        new.iban = Some(&iban);
        new.on_budget = budget_mode(&mode);
        new.starting_balance_minor = start;
        new.starting_date = starting_date;
        accounts::create_account(ctx.db, auth.user.id, &new)
    })();
    match result {
        Ok(account) => ctx.flash(&auth, &format!("The account '{}' is added.", account.name), false)?,
        Err(ServiceError::Service(message)) => ctx.flash(&auth, &message, true)?,
        Err(other) => return Err(other.into()),
    }
    redirect("/accounts")
}

pub fn close_account(ctx: &mut Ctx<'_>, account_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let closed = ctx.form()?.boolean("closed", true)?;
    let account = accounts::get_account(ctx.db, auth.user.id, account_id?)?;
    accounts::set_closed(ctx.db, &account, closed)?;
    let state = if closed { "closed" } else { "open" };
    ctx.flash(&auth, &format!("The account '{}' is {state}.", account.name), false)?;
    redirect("/accounts")
}

pub fn edit_account_page(ctx: &mut Ctx<'_>, account_id: Result<i64>) -> Result<Response> {
    let auth = ctx.auth()?;
    let account = accounts::get_account(ctx.db, auth.user.id, account_id?)?;
    ctx.render(
        Some(&auth),
        "account_edit.html",
        200,
        context! { account => account_view(&account), account_types => account_types() },
    )
}

pub fn update_account(ctx: &mut Ctx<'_>, account_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let name = form.required("name", 100)?;
    let kind = account_type(&form.required("account_type", 20)?)?;
    let mode = form.get("budget_mode").unwrap_or("default").to_string();
    if mode.chars().count() > 10 {
        return Err(AppError::Invalid);
    }
    let account = accounts::get_account(ctx.db, auth.user.id, account_id?)?;
    let budget_currency = ctx.settings().budget_currency.clone();
    match accounts::update_account(ctx.db, &account, &name, kind, &budget_currency, budget_mode(&mode)) {
        Ok(updated) => {
            ctx.flash(&auth, &format!("The account '{}' is changed.", updated.name), false)?;
            redirect("/accounts")
        }
        Err(ServiceError::Service(message)) => {
            ctx.flash(&auth, &message, true)?;
            redirect(&format!("/accounts/{}/edit", account.id))
        }
        Err(other) => Err(other.into()),
    }
}
