//! The web application: routing, authentication, CSRF protection and security headers.

pub mod charts;
pub mod http;
pub mod render;
pub mod routes;
pub mod server;

use std::sync::Mutex;

use minijinja::Environment;
use minijinja::value::Value;
use rusqlite::Connection;

use crate::config::Settings;
use crate::models::User;
use crate::services::{self, users};
pub use http::{Cookie, Params, Request, Response, Upload};

pub const SESSION_COOKIE: &str = "feljosit_session";
pub const LOGIN_CSRF_COOKIE: &str = "feljosit_login_csrf";
pub const SETUP_CSRF_COOKIE: &str = "feljosit_setup_csrf";
/// Space for form fields in addition to the uploaded file.
pub const FORM_OVERHEAD_BYTES: usize = 64 * 1024;

pub const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
    img-src 'self'; font-src 'self'; connect-src 'self'; form-action 'self'; \
    frame-ancestors 'none'; base-uri 'none'";

pub const SECURITY_HEADERS: &[(&str, &str)] = &[
    ("content-security-policy", CONTENT_SECURITY_POLICY),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "same-origin"),
    ("cross-origin-opener-policy", "same-origin"),
    ("cross-origin-resource-policy", "same-origin"),
    ("permissions-policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
];

const STATIC_FILES: &[(&str, &str, &[u8])] = &[
    ("/static/app.css", "text/css; charset=utf-8", include_bytes!("../../static/app.css")),
    ("/static/app.js", "text/javascript; charset=utf-8", include_bytes!("../../static/app.js")),
    ("/static/icon.svg", "image/svg+xml", include_bytes!("../../static/icon.svg")),
];

/// The errors that stop a request. The database transaction of the request is rolled
/// back.
#[derive(Debug)]
pub enum AppError {
    /// The request needs a logged-in user.
    LoginRequired,
    /// The CSRF token or the Origin header is not correct.
    Csrf,
    NotFound,
    /// A parameter of the request is not valid.
    Invalid,
    /// A different HTTP error.
    Http(u16, &'static str),
    /// An error that the handler did not expect.
    Internal(String),
}

impl From<services::Error> for AppError {
    fn from(error: services::Error) -> AppError {
        match error {
            services::Error::NotFound(_) => AppError::NotFound,
            other => AppError::Internal(other.to_string()),
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> AppError {
        AppError::Internal(error.to_string())
    }
}

impl From<minijinja::Error> for AppError {
    fn from(error: minijinja::Error) -> AppError {
        AppError::Internal(format!("{error:#}"))
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// The application. It keeps one database connection. The requests use it one after
/// the other. This is sufficient for 1 to 10 users on one computer, and it prevents
/// races (for example two imports of the same file).
pub struct App {
    pub settings: Settings,
    conn: Mutex<Connection>,
    templates: Environment<'static>,
}

/// The logged-in user and the login session.
#[derive(Debug, Clone)]
pub struct Auth {
    pub user: User,
    pub session: users::Session,
}

/// The state of one request.
pub struct Ctx<'a> {
    pub app: &'a App,
    pub db: &'a Connection,
    pub req: &'a Request,
    pub query: Params,
    form: Option<Params>,
}

impl App {
    /// Make the application. Upgrade the database schema first.
    pub fn new(settings: Settings, mut conn: Connection) -> std::result::Result<App, String> {
        crate::db::migrate(&mut conn)?;
        Ok(App { settings, conn: Mutex::new(conn), templates: render::environment() })
    }

    /// Run a function with the database connection (for the command line and tests).
    pub fn with_db<T>(&self, f: impl FnOnce(&mut Connection) -> T) -> T {
        let mut conn = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut conn)
    }

    /// The maximum size of a request body.
    pub fn max_body_bytes(&self) -> usize {
        self.settings.max_upload_bytes + FORM_OVERHEAD_BYTES
    }

    /// Handle one request.
    pub fn handle(&self, req: &Request) -> Response {
        let response = self.handle_inner(req);
        self.finish(req, response)
    }

    /// Add the security headers to a response.
    pub fn finish(&self, req: &Request, mut response: Response) -> Response {
        for (name, value) in SECURITY_HEADERS {
            response.set_header(name, value);
        }
        if !req.path.starts_with("/static/") {
            response.set_header("cache-control", "no-store");
        }
        if self.settings.secure_cookies {
            response.set_header("strict-transport-security", "max-age=31536000");
        }
        if req.method == "HEAD" {
            response.body.clear();
        }
        response
    }

    fn host_allowed(&self, req: &Request) -> bool {
        let host = req.header("host").unwrap_or("").to_lowercase();
        let name = if let Some(rest) = host.strip_prefix('[') {
            rest.split(']').next().unwrap_or("").to_string()
        } else {
            host.rsplit_once(':').map_or(host.as_str(), |(name, _)| name).to_string()
        };
        self.settings.allowed_hosts.iter().any(|h| h.to_lowercase() == name)
    }

    fn handle_inner(&self, req: &Request) -> Response {
        // Refuse requests with an unknown Host header. This prevents DNS rebinding attacks.
        if !self.host_allowed(req) {
            return Response::text(400, "Invalid host header");
        }
        let declared = req.header("content-length");
        let too_large = declared
            .is_some_and(|d| d.parse::<usize>().map_or(true, |n| n > self.max_body_bytes()))
            || req.body.len() > self.max_body_bytes();
        if too_large {
            return Response::text(413, "Request body too large");
        }
        if matches!(req.method.as_str(), "GET" | "HEAD")
            && let Some((_, content_type, data)) = STATIC_FILES.iter().find(|(path, ..)| *path == req.path)
        {
            return Response::new(200, content_type, data.to_vec());
        }
        let mut conn = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tx = match conn.transaction() {
            Ok(tx) => tx,
            Err(error) => return internal_error(&error.to_string()),
        };
        let mut ctx = Ctx { app: self, db: &tx, req, query: Params::from_query(&req.query), form: None };
        let result = routes::dispatch(&mut ctx);
        match result {
            Ok(response) => match tx.commit() {
                Ok(()) => response,
                Err(error) => internal_error(&error.to_string()),
            },
            Err(error) => {
                drop(tx);
                error_response(error)
            }
        }
    }
}

fn internal_error(message: &str) -> Response {
    crate::db::log::warning(&format!("Internal error: {message}"));
    Response::text(500, "Internal Server Error")
}

fn error_response(error: AppError) -> Response {
    match error {
        AppError::LoginRequired => Response::redirect("/login"),
        AppError::Csrf => Response::text(403, "CSRF validation failed"),
        AppError::NotFound => Response::text(404, "Not found"),
        // Do not echo the input back to the client.
        AppError::Invalid => Response::text(422, "The request is not valid"),
        AppError::Http(status, message) => Response::text(status, message),
        AppError::Internal(message) => internal_error(&message),
    }
}

impl Ctx<'_> {
    pub fn settings(&self) -> &Settings {
        &self.app.settings
    }

    /// The form of a POST request.
    pub fn form(&mut self) -> Result<&Params> {
        if self.form.is_none() {
            let parsed = http::parse_form(self.req.header("content-type"), &self.req.body)
                .ok_or(AppError::Http(400, "The form data is not valid"))?;
            self.form = Some(parsed);
        }
        Ok(self.form.as_ref().expect("The form is parsed"))
    }

    /// The logged-in user. Else stop with a redirect to the login page.
    pub fn auth(&self) -> Result<Auth> {
        let token = self.req.cookie(SESSION_COOKIE).unwrap_or_default();
        let session = users::load_session(self.db, &token)?.ok_or(AppError::LoginRequired)?;
        let user = User::by_id(self.db, session.user_id)?.ok_or(AppError::LoginRequired)?;
        Ok(Auth { user, session })
    }

    /// Compare the Origin (or Referer) header with the Host header.
    pub fn origin_ok(&self) -> bool {
        let Some(origin) = self.req.header("origin").or_else(|| self.req.header("referer")) else {
            // Browsers send Origin for all POST requests. Non-browser clients have no
            // ambient credentials, thus the CSRF token is sufficient.
            return true;
        };
        if origin == "null" {
            return false;
        }
        let netloc = origin
            .split_once("://")
            .map_or("", |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .rsplit('@')
            .next()
            .unwrap_or("");
        netloc.eq_ignore_ascii_case(self.req.header("host").unwrap_or(""))
    }

    /// The logged-in user of a POST request with a correct CSRF token.
    pub fn csrf_auth(&mut self) -> Result<Auth> {
        let auth = self.auth()?;
        let origin_ok = self.origin_ok();
        let token = self.form()?.get("csrf_token").map(str::to_string);
        if !origin_ok || !users::csrf_matches(&auth.session.csrf_token, token.as_deref()) {
            return Err(AppError::Csrf);
        }
        Ok(auth)
    }

    /// Set the message for the next page.
    pub fn flash(&self, auth: &Auth, message: &str, error: bool) -> Result<()> {
        users::set_flash(self.db, &auth.session.token_hash, message, error)?;
        Ok(())
    }

    /// Render a template. `auth` gives the navigation and the message of the session.
    pub fn render(&self, auth: Option<&Auth>, name: &str, status: u16, page: Value) -> Result<Response> {
        let flash = match auth {
            Some(auth) => users::pop_flash(self.db, &auth.session.token_hash)?,
            None => None,
        };
        let base = minijinja::context! {
            auth => auth.map(|a| minijinja::context! { user => minijinja::context! { username => a.user.username.clone() } }),
            csrf_token => auth.map(|a| a.session.csrf_token.clone()),
            flash => flash.map(|(message, error)| vec![Value::from(message), Value::from(error)]),
            budget_currency => self.settings().budget_currency.clone(),
            request_path => self.req.path.clone(),
        };
        let template = self.app.templates.get_template(name)?;
        let html = template.render(minijinja::context! { ..page, ..base })?;
        Ok(Response::html(status, html))
    }

    /// Set the session cookie for a new login session.
    pub fn session_cookie(&self, response: &mut Response, token: &str) {
        response.set_cookie(&Cookie {
            name: SESSION_COOKIE,
            value: token,
            max_age: self.settings().session_lifetime_seconds,
            path: "/",
            secure: self.settings().secure_cookies,
        });
    }
}

/// Read typed values from query or form parameters. A value that is not valid stops
/// the request with "422 The request is not valid" (as FastAPI).
pub trait ParamsExt {
    fn text(&self, name: &str, max_chars: usize) -> Result<String>;
    fn required(&self, name: &str, max_chars: usize) -> Result<String>;
    fn int(&self, name: &str) -> Result<Option<i64>>;
    fn required_int(&self, name: &str) -> Result<i64>;
    fn optional_id(&self, name: &str) -> Result<Option<i64>>;
    fn boolean(&self, name: &str, default: bool) -> Result<bool>;
    fn date(&self, name: &str) -> Result<Option<crate::dates::Date>>;
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

pub fn parse_int(value: &str) -> Option<i64> {
    let trimmed = value.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse().ok()
}

impl ParamsExt for Params {
    /// An optional text. Missing: empty.
    fn text(&self, name: &str, max_chars: usize) -> Result<String> {
        let value = self.get(name).unwrap_or("");
        if value.chars().count() > max_chars {
            return Err(AppError::Invalid);
        }
        Ok(value.to_string())
    }

    fn required(&self, name: &str, max_chars: usize) -> Result<String> {
        if self.get(name).is_none() {
            return Err(AppError::Invalid);
        }
        self.text(name, max_chars)
    }

    fn int(&self, name: &str) -> Result<Option<i64>> {
        self.get(name).map(|v| parse_int(v).ok_or(AppError::Invalid)).transpose()
    }

    fn required_int(&self, name: &str) -> Result<i64> {
        self.int(name)?.ok_or(AppError::Invalid)
    }

    /// An optional ID. An empty value is `None`.
    fn optional_id(&self, name: &str) -> Result<Option<i64>> {
        match self.get(name) {
            None | Some("") => Ok(None),
            Some(value) => parse_int(value).map(Some).ok_or(AppError::Invalid),
        }
    }

    fn boolean(&self, name: &str, default: bool) -> Result<bool> {
        match self.get(name) {
            None => Ok(default),
            Some(value) => parse_bool(value).ok_or(AppError::Invalid),
        }
    }

    /// An optional ISO date. An empty value is `None`.
    fn date(&self, name: &str) -> Result<Option<crate::dates::Date>> {
        match self.get(name) {
            None | Some("") => Ok(None),
            Some(value) => parse_iso_date(value).map(Some).ok_or(AppError::Invalid),
        }
    }
}

/// Parse a date `YYYY-MM-DD`.
pub fn parse_iso_date(value: &str) -> Option<crate::dates::Date> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    crate::dates::parse(value, "%Y-%m-%d")
}

/// A template value from a serialisable Rust value.
pub fn ser<T: serde::Serialize>(value: T) -> Value {
    Value::from(minijinja::value::Serde(value))
}
