//! The routes of the web application.

mod accounts;
mod auth;
mod budget;
mod categories;
mod imports;
pub mod reports;
mod transactions;

use super::{AppError, Ctx, Response, Result, parse_int};

/// A path parameter that must be a number. Else "422 The request is not valid".
fn id(segment: &str) -> Result<i64> {
    parse_int(segment).ok_or(AppError::Invalid)
}

pub fn dispatch(ctx: &mut Ctx<'_>) -> Result<Response> {
    let method = if ctx.req.method == "HEAD" { "GET".to_string() } else { ctx.req.method.clone() };
    let path = ctx.req.path.clone();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let get = method == "GET";
    let post = method == "POST";
    match (segments.as_slice(), get, post) {
        ([""], true, _) => Ok(Response::redirect("/budget")),
        (["healthz"], true, _) => Ok(Response::text(200, "ok")),
        (["login"], true, _) => auth::login_form(ctx),
        (["login"], _, true) => auth::login(ctx),
        (["setup"], true, _) => auth::setup_form(ctx),
        (["setup"], _, true) => auth::setup(ctx),
        (["logout"], _, true) => auth::logout(ctx),
        (["settings"], true, _) => auth::settings_page(ctx),
        (["settings", "password"], _, true) => auth::change_password(ctx),

        (["budget"], true, _) => budget::budget_page(ctx),
        (["budget", "assign"], _, true) => budget::assign(ctx),

        (["accounts"], true, _) => accounts::accounts_page(ctx),
        (["accounts"], _, true) => accounts::create_account(ctx),
        (["accounts", account, "close"], _, true) => accounts::close_account(ctx, id(account)),
        (["accounts", account, "edit"], true, _) => accounts::edit_account_page(ctx, id(account)),
        (["accounts", account], _, true) => accounts::update_account(ctx, id(account)),

        (["transactions"], true, _) => transactions::transactions_page(ctx),
        (["transactions"], _, true) => transactions::add_transaction(ctx),
        (["transactions", "bulk-category"], _, true) => transactions::set_category_bulk(ctx),
        (["transactions", tx, "category"], _, true) => transactions::set_category(ctx, id(tx)),
        (["transactions", tx, "memo"], _, true) => transactions::set_memo(ctx, id(tx)),
        (["transactions", tx, "delete"], _, true) => transactions::delete_transaction(ctx, id(tx)),

        (["import"], true, _) => imports::import_page(ctx),
        (["import"], _, true) => imports::import_upload(ctx),

        (["categories"], true, _) => categories::categories_page(ctx),
        (["categories"], _, true) => categories::create_category(ctx),
        (["categories", "groups"], _, true) => categories::create_group(ctx),
        (["categories", category, "rename"], _, true) => categories::rename_category(ctx, id(category)),
        (["categories", category, "hide"], _, true) => categories::hide_category(ctx, id(category)),
        (["categories", category, "move"], _, true) => categories::move_category(ctx, id(category)),
        (["rules"], _, true) => categories::create_rule(ctx),
        (["rules", "apply"], _, true) => categories::apply_rules(ctx),
        (["rules", rule, "edit"], true, _) => categories::edit_rule_page(ctx, id(rule)),
        (["rules", rule, "delete"], _, true) => categories::delete_rule(ctx, id(rule)),
        (["rules", rule], _, true) if parse_int(rule).is_some_and(|v| v >= 0) => {
            categories::update_rule(ctx, id(rule))
        }

        (["reports"], true, _) => reports::reports_page(ctx),
        (["reports", "categories"], true, _) => reports::category_report_page(ctx),
        (["reports", "counterparties"], true, _) => reports::counterparty_report_page(ctx),

        _ if known_path(&segments) => Ok(Response::text(405, "Method Not Allowed")),
        _ => Ok(Response::text(404, "Not Found")),
    }
}

/// True if the path exists for a different method.
fn known_path(segments: &[&str]) -> bool {
    matches!(
        segments,
        [""] | ["healthz"]
            | ["login"]
            | ["setup"]
            | ["logout"]
            | ["settings"]
            | ["settings", "password"]
            | ["budget"]
            | ["budget", "assign"]
            | ["accounts"]
            | ["accounts", _]
            | ["accounts", _, "close" | "edit"]
            | ["transactions"]
            | ["transactions", _]
            | ["transactions", _, "category" | "memo" | "delete"]
            | ["import"]
            | ["categories"]
            | ["categories", "groups"]
            | ["categories", _, "rename" | "hide" | "move"]
            | ["rules"]
            | ["rules", _]
            | ["rules", _, "edit" | "delete"]
            | ["reports"]
            | ["reports", "categories" | "counterparties"]
    )
}

/// Redirect with "303 See Other".
fn redirect(location: &str) -> Result<Response> {
    Ok(Response::redirect(location))
}
