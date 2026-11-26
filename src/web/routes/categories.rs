//! Categories and categorisation rules.

use minijinja::context;
use minijinja::value::Value;
use rusqlite::params;

use super::redirect;
use crate::models::{CategoryGroup, CategoryKind, MatchType, RuleField};
use crate::services::categorise::{self, RuleValues};
use crate::services::{Error as ServiceError, categories};
use crate::web::{AppError, Auth, Ctx, ParamsExt, Response, Result};

fn checked_name(value: &str) -> std::result::Result<String, String> {
    let value = value.trim();
    if !(1..=100).contains(&value.chars().count()) {
        return Err("The name must have 1 to 100 characters.".into());
    }
    Ok(value.to_string())
}

fn groups(ctx: &Ctx<'_>, auth: &Auth) -> Result<Vec<CategoryGroup>> {
    Ok(CategoryGroup::list(ctx.db, auth.user.id)?)
}

fn field_values() -> Vec<&'static str> {
    RuleField::ALL.iter().map(|f| f.as_str()).collect()
}

fn match_type_values() -> Vec<&'static str> {
    MatchType::ALL.iter().map(|m| m.as_str()).collect()
}

pub fn categories_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let groups = groups(ctx, &auth)?;
    let names: std::collections::HashMap<i64, String> =
        groups.iter().flat_map(|g| g.categories.iter()).map(|c| (c.id, c.name.clone())).collect();
    let rules: Vec<Value> = categorise::list_rules(ctx.db, auth.user.id)?
        .into_iter()
        .map(|r| {
            let category_name = names.get(&r.category_id).cloned().unwrap_or_default();
            let rule = crate::web::ser(&r);
            context! { ..rule, ..context! { category_name => category_name } }
        })
        .collect();
    ctx.render(
        Some(&auth),
        "categories.html",
        200,
        context! {
            groups => crate::web::ser(&groups),
            rules => rules,
            kinds => CategoryKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            fields => field_values(),
            match_types => match_type_values(),
        },
    )
}

pub fn create_group(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let name = ctx.form()?.required("name", 100)?;
    match checked_name(&name) {
        Ok(name) => {
            let order: i64 = ctx.db.query_row(
                "SELECT coalesce(max(sort_order), 0) FROM category_groups WHERE user_id = ?1",
                [auth.user.id],
                |row| row.get(0),
            )?;
            ctx.db.execute(
                "INSERT INTO category_groups (user_id, name, sort_order) VALUES (?1, ?2, ?3)",
                params![auth.user.id, name, order + 1],
            )?;
        }
        Err(message) => ctx.flash(&auth, &message, true)?,
    }
    redirect("/categories")
}

pub fn create_category(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let group_id = form.required_int("group_id")?;
    let name = form.required("name", 100)?;
    let kind = match form.get("kind") {
        None => CategoryKind::Expense,
        Some(value) => CategoryKind::parse(value).ok_or(AppError::Invalid)?,
    };
    let group = categories::owned_group(ctx.db, auth.user.id, group_id)?;
    match checked_name(&name) {
        Ok(name) => {
            let count: i64 = ctx.db.query_row(
                "SELECT count(*) FROM categories WHERE group_id = ?1",
                [group.id],
                |row| row.get(0),
            )?;
            ctx.db.execute(
                "INSERT INTO categories (user_id, group_id, name, kind, hidden, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![auth.user.id, group.id, name, kind, count],
            )?;
        }
        Err(message) => ctx.flash(&auth, &message, true)?,
    }
    redirect("/categories")
}

pub fn rename_category(ctx: &mut Ctx<'_>, category_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let name = ctx.form()?.required("name", 100)?;
    let category = categorise::owned_category(ctx.db, auth.user.id, category_id?)?;
    match checked_name(&name) {
        Ok(name) => {
            ctx.db.execute("UPDATE categories SET name = ?1 WHERE id = ?2", params![name, category.id])?;
        }
        Err(message) => ctx.flash(&auth, &message, true)?,
    }
    redirect("/categories")
}

pub fn hide_category(ctx: &mut Ctx<'_>, category_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let hidden = ctx.form()?.boolean("hidden", true)?;
    let category = categorise::owned_category(ctx.db, auth.user.id, category_id?)?;
    ctx.db.execute("UPDATE categories SET hidden = ?1 WHERE id = ?2", params![hidden, category.id])?;
    redirect("/categories")
}

/// The drag-and-drop script asks for JSON. A form without JavaScript gets a redirect.
fn wants_json(ctx: &Ctx<'_>) -> bool {
    ctx.req.header("accept").unwrap_or("").contains("application/json")
}

/// Move a category into a group, before a different category or to the end.
pub fn move_category(ctx: &mut Ctx<'_>, category_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let group_id = form.required_int("group_id")?;
    let before_id = form.text("before_id", 20)?;
    let category = categorise::owned_category(ctx.db, auth.user.id, category_id?)?;
    let group = categories::owned_group(ctx.db, auth.user.id, group_id)?;
    let before = if !before_id.is_empty() && before_id.bytes().all(|b| b.is_ascii_digit()) {
        let id = before_id.parse().map_err(|_| AppError::NotFound)?;
        Some(categorise::owned_category(ctx.db, auth.user.id, id)?)
    } else {
        None
    };
    match categories::move_category(ctx.db, &category, &group, before.as_ref()) {
        Ok(()) => {}
        Err(ServiceError::Service(message)) => {
            if wants_json(ctx) {
                return Ok(Response::json(400, &serde_json::json!({ "error": message })));
            }
            ctx.flash(&auth, &message, true)?;
            return redirect("/categories");
        }
        Err(other) => return Err(other.into()),
    }
    if wants_json(ctx) {
        return Ok(Response::json(200, &serde_json::json!({ "group": group.name })));
    }
    ctx.flash(
        &auth,
        &format!("The category '{}' is in the group '{}' now.", category.name, group.name),
        false,
    )?;
    redirect("/categories")
}

/// The values of the rule form.
fn rule_form(ctx: &mut Ctx<'_>) -> Result<(RuleField, MatchType, String, i64, i64)> {
    let form = ctx.form()?;
    let field = RuleField::parse(&form.required("field", 20)?).ok_or(AppError::Invalid)?;
    let match_type = MatchType::parse(&form.required("match_type", 10)?).ok_or(AppError::Invalid)?;
    let pattern = form.required("pattern", 140)?;
    let category_id = form.required_int("category_id")?;
    let priority = form.int("priority")?.unwrap_or(100);
    if !(0..=10_000).contains(&priority) {
        return Err(AppError::Invalid);
    }
    Ok((field, match_type, pattern, category_id, priority))
}

pub fn create_rule(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let (field, match_type, pattern, category_id, priority) = rule_form(ctx)?;
    let values = RuleValues { field, match_type, pattern: &pattern, category_id, priority };
    match categorise::create_rule(ctx.db, auth.user.id, &values) {
        Ok(_) => ctx.flash(&auth, "The rule is added.", false)?,
        Err(ServiceError::Service(message)) => ctx.flash(&auth, &message, true)?,
        Err(other) => return Err(other.into()),
    }
    redirect("/categories#rules")
}

pub fn edit_rule_page(ctx: &mut Ctx<'_>, rule_id: Result<i64>) -> Result<Response> {
    let auth = ctx.auth()?;
    let rule = categorise::owned_rule(ctx.db, auth.user.id, rule_id?)?;
    let groups = groups(ctx, &auth)?;
    ctx.render(
        Some(&auth),
        "rule_edit.html",
        200,
        context! {
            rule => crate::web::ser(&rule),
            groups => crate::web::ser(&groups),
            fields => field_values(),
            match_types => match_type_values(),
        },
    )
}

pub fn update_rule(ctx: &mut Ctx<'_>, rule_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let (field, match_type, pattern, category_id, priority) = rule_form(ctx)?;
    let rule = categorise::owned_rule(ctx.db, auth.user.id, rule_id?)?;
    let values = RuleValues { field, match_type, pattern: &pattern, category_id, priority };
    match categorise::update_rule(ctx.db, &rule, &values) {
        Ok(()) => {
            ctx.flash(&auth, "The rule is changed.", false)?;
            redirect("/categories#rules")
        }
        Err(ServiceError::Service(message)) => {
            ctx.flash(&auth, &message, true)?;
            redirect(&format!("/rules/{}/edit", rule.id))
        }
        Err(other) => Err(other.into()),
    }
}

pub fn delete_rule(ctx: &mut Ctx<'_>, rule_id: Result<i64>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let rule = categorise::owned_rule(ctx.db, auth.user.id, rule_id?)?;
    categorise::delete_rule(ctx.db, &rule)?;
    redirect("/categories#rules")
}

pub fn apply_rules(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let changed = categorise::apply_to_uncategorised(ctx.db, auth.user.id)?;
    ctx.flash(&auth, &format!("{changed} transactions are categorised."), false)?;
    redirect("/transactions?uncategorised=true")
}
