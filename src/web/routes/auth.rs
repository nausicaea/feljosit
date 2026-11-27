//! Login, logout, the first user and the settings.

use minijinja::context;

use super::redirect;
use crate::services::{Error as ServiceError, users};
use crate::web::{
    AppError, Cookie, Ctx, LOGIN_CSRF_COOKIE, ParamsExt, Response, Result, SESSION_COOKIE, SETUP_CSRF_COOKIE,
};

/// A page with a form that is not in a login session: the login and the setup page.
/// The CSRF token of the form is also in a cookie ("double submit").
fn form_page(
    ctx: &Ctx<'_>,
    template: &str,
    cookie: &str,
    path: &str,
    error: Option<&str>,
    status: u16,
) -> Result<Response> {
    let token = users::random_token();
    let mut response =
        ctx.render(None, template, status, context! { login_csrf => token.clone(), error => error })?;
    response.set_cookie(&Cookie {
        name: cookie,
        value: &token,
        max_age: 3600,
        path,
        secure: ctx.settings().secure_cookies,
    });
    Ok(response)
}

fn login_page(ctx: &Ctx<'_>, error: Option<&str>, status: u16) -> Result<Response> {
    form_page(ctx, "login.html", LOGIN_CSRF_COOKIE, "/login", error, status)
}

fn form_token_ok(ctx: &mut Ctx<'_>, cookie_name: &str) -> Result<bool> {
    let cookie = ctx.req.cookie(cookie_name).unwrap_or_default();
    let origin_ok = ctx.origin_ok();
    let token = ctx.form()?.required("csrf_token", 128)?;
    Ok(origin_ok && !cookie.is_empty() && users::csrf_matches(&cookie, Some(&token)))
}

pub fn login_form(ctx: &mut Ctx<'_>) -> Result<Response> {
    if users::count_users(ctx.db)? == 0 {
        return redirect("/setup");
    }
    login_page(ctx, None, 200)
}

pub fn login(ctx: &mut Ctx<'_>) -> Result<Response> {
    let form = ctx.form()?;
    let username = form.required("username", 64)?;
    let password = form.required("password", 1024)?;
    if !form_token_ok(ctx, LOGIN_CSRF_COOKIE)? {
        return login_page(ctx, Some("The form expired. Try again."), 400);
    }
    let Some(user) = users::authenticate(ctx.db, &username, &password, &ctx.client_key())? else {
        // The handler returns normally: the failed-login counter is kept.
        return login_page(ctx, Some("The username or the password is not correct."), 401);
    };
    let new = users::start_session(ctx.db, user.id, ctx.settings().session_lifetime_seconds)?;
    let target = if user.must_change_password { "/settings" } else { "/budget" };
    let mut response = Response::redirect(target);
    ctx.session_cookie(&mut response, &new.token);
    response.delete_cookie(LOGIN_CSRF_COOKIE, "/login", ctx.settings().secure_cookies);
    Ok(response)
}

/// True if the request comes from this computer: the connection comes from a loopback
/// address, and the `Host` header is a loopback name. A reverse proxy on this computer
/// also connects from a loopback address, but it sends the public host name.
fn local_request(ctx: &Ctx<'_>) -> bool {
    let host = ctx.req.header("host").unwrap_or("").to_lowercase();
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or("").to_string(),
        None => host.rsplit_once(':').map_or(host.as_str(), |(n, _)| n).to_string(),
    };
    ctx.req.peer_is_loopback && crate::config::LOOPBACK_HOSTS.contains(&name.as_str())
}

/// The first start: make the first user in the browser. This is only possible while
/// the database has no user, and only from the same computer.
pub fn setup_form(ctx: &mut Ctx<'_>) -> Result<Response> {
    if users::count_users(ctx.db)? > 0 {
        return redirect("/login");
    }
    if !local_request(ctx) {
        return Ok(Response::text(403, "Add the first user on the computer of the server."));
    }
    form_page(ctx, "setup.html", SETUP_CSRF_COOKIE, "/setup", None, 200)
}

pub fn setup(ctx: &mut Ctx<'_>) -> Result<Response> {
    let form = ctx.form()?;
    let username = form.required("username", 64)?;
    let password = form.required("password", 1024)?;
    let repeat = form.required("password_repeat", 1024)?;
    if users::count_users(ctx.db)? > 0 {
        return redirect("/login");
    }
    if !local_request(ctx) {
        return Ok(Response::text(403, "Add the first user on the computer of the server."));
    }
    let page = |ctx: &Ctx<'_>, error: &str| {
        form_page(ctx, "setup.html", SETUP_CSRF_COOKIE, "/setup", Some(error), 400)
    };
    if !form_token_ok(ctx, SETUP_CSRF_COOKIE)? {
        return page(ctx, "The form expired. Try again.");
    }
    if password != repeat {
        return page(ctx, "The two passwords are not the same.");
    }
    let user = match users::create_user(ctx.db, &username, &password) {
        Ok(user) => user,
        Err(ServiceError::Service(message)) => return page(ctx, &message),
        Err(other) => return Err(other.into()),
    };
    let new = users::start_session(ctx.db, user.id, ctx.settings().session_lifetime_seconds)?;
    users::set_flash(
        ctx.db,
        &users::hash_token(&new.token),
        &format!("The user '{}' is added. Add an account or import a bank statement.", user.username),
        false,
    )?;
    let mut response = Response::redirect("/accounts");
    ctx.session_cookie(&mut response, &new.token);
    response.delete_cookie(SETUP_CSRF_COOKIE, "/setup", ctx.settings().secure_cookies);
    Ok(response)
}

pub fn logout(ctx: &mut Ctx<'_>) -> Result<Response> {
    ctx.csrf_auth()?;
    users::end_session(ctx.db, &ctx.req.cookie(SESSION_COOKIE).unwrap_or_default())?;
    let mut response = Response::redirect("/login");
    response.delete_cookie(SESSION_COOKIE, "/", ctx.settings().secure_cookies);
    Ok(response)
}

pub fn settings_page(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.auth()?;
    let must_change = auth.user.must_change_password;
    ctx.render(Some(&auth), "settings.html", 200, context! { must_change_password => must_change })
}

pub fn change_password(ctx: &mut Ctx<'_>) -> Result<Response> {
    let auth = ctx.csrf_auth()?;
    let form = ctx.form()?;
    let current = form.required("current_password", 1024)?;
    let new_password = form.required("new_password", 1024)?;
    let repeat = form.required("new_password_repeat", 1024)?;
    if users::authenticate(ctx.db, &auth.user.username, &current, &ctx.client_key())?.is_none() {
        ctx.flash(&auth, "The current password is not correct.", true)?;
        return redirect("/settings");
    }
    if new_password != repeat {
        ctx.flash(&auth, "The two new passwords are not the same.", true)?;
        return redirect("/settings");
    }
    if new_password == current {
        ctx.flash(&auth, "The new password must be different from the current password.", true)?;
        return redirect("/settings");
    }
    match users::change_password(ctx.db, auth.user.id, &new_password) {
        Ok(()) => {}
        Err(ServiceError::Service(message)) => {
            ctx.flash(&auth, &message, true)?;
            return redirect("/settings");
        }
        Err(other) => return Err(AppError::from(other)),
    }
    let new = users::start_session(ctx.db, auth.user.id, ctx.settings().session_lifetime_seconds)?;
    users::set_flash(
        ctx.db,
        &users::hash_token(&new.token),
        "The password is changed. All other sessions are closed.",
        false,
    )?;
    let mut response = Response::redirect("/settings");
    ctx.session_cookie(&mut response, &new.token);
    Ok(response)
}
