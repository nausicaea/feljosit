//! Import of camt.053 bank statements and credit card CSV files.

use minijinja::context;
use minijinja::value::Value;

use crate::db::parse_datetime;
use crate::services::Error as ServiceError;
use crate::services::accounts;
use crate::services::importer::{ImportError, ImportRequest, import_statement_file};
use crate::web::{AppError, Auth, Ctx, ParamsExt, Response, Result};

fn page(ctx: &Ctx<'_>, auth: &Auth, error: Option<&str>, result: Option<Value>) -> Result<Response> {
    let mut stmt = ctx.db.prepare(
        "SELECT filename, new_count, duplicate_count, imported_at FROM import_batches \
         WHERE user_id = ?1 ORDER BY imported_at DESC LIMIT 20",
    )?;
    let history: Vec<Value> = stmt
        .query_map([auth.user.id], |row| {
            let imported: String = row.get(3)?;
            let text =
                parse_datetime(&imported).map(|t| t.format("%d.%m.%Y %H:%M").to_string()).unwrap_or(imported);
            Ok(context! {
                filename => row.get::<_, String>(0)?,
                new_count => row.get::<_, i64>(1)?,
                duplicate_count => row.get::<_, i64>(2)?,
                imported_at => text,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let accounts: Vec<Value> = accounts::list_accounts(ctx.db, auth.user.id, false)?
        .iter()
        .map(|a| context! { id => a.id, name => a.name.clone(), iban => a.iban.clone() })
        .collect();
    let status = if error.is_some() { 400 } else { 200 };
    ctx.render(
        Some(auth),
        "import.html",
        status,
        context! { accounts => accounts, history => history, error => error, result => result },
    )
}

pub fn import_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    page(ctx, &auth, None, None)
}

pub fn import_upload(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let account_id = form.text("account_id", 20)?;
    let Some(upload) = form.file("statement").cloned() else {
        return Err(AppError::Invalid);
    };
    let max = ctx.settings().max_upload_bytes;
    if upload.data.len() > max {
        return page(ctx, &auth, Some("The file is too large."), None);
    }
    if upload.data.is_empty() {
        return page(ctx, &auth, Some("The file is empty."), None);
    }
    let account_id = (!account_id.is_empty() && account_id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| account_id.parse().ok())
        .flatten();
    let budget_currency = ctx.settings().budget_currency.clone();
    let request = ImportRequest {
        filename: upload.filename.as_deref().unwrap_or("statement.xml"),
        data: &upload.data,
        account_id,
        budget_currency: Some(&budget_currency),
    };
    // A savepoint: if the import fails, nothing of the file is in the database.
    ctx.db.execute_batch("SAVEPOINT import")?;
    match import_statement_file(ctx.db, auth.user.id, &request) {
        Ok(result) => {
            ctx.db.execute_batch("RELEASE import")?;
            page(ctx, &auth, None, Some(crate::web::ser(&result)))
        }
        Err(error) => {
            ctx.db.execute_batch("ROLLBACK TO import; RELEASE import")?;
            match error {
                ImportError::Service(ServiceError::NotFound(_)) => Err(AppError::NotFound),
                ImportError::Service(ServiceError::Db(e)) => Err(e.into()),
                other => page(ctx, &auth, Some(&other.to_string()), None),
            }
        }
    }
}
