//! Shared test helpers: fixtures, a database with a user and an HTTP test client.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use feljosit::config::Settings;
use feljosit::db;
use feljosit::models::User;
use feljosit::services::users;
use feljosit::web::{App, Request, Response};
use rusqlite::Connection;

pub const PASSWORD: &str = "correct horse battery staple";

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("The fixture exists")
}

/// A ZIP archive with the files.
pub fn zip_files(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut buffer);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in files {
            archive.start_file(*name, options).unwrap();
            archive.write_all(data).unwrap();
        }
        archive.finish().unwrap();
    }
    buffer.into_inner()
}

/// A migrated database in memory.
pub fn database() -> Connection {
    users::use_fast_hasher_for_tests();
    let mut conn = db::open_in_memory().unwrap();
    db::migrate(&mut conn).unwrap();
    conn
}

/// A migrated database in memory with the user "anna".
pub fn database_with_user() -> (Connection, User) {
    let conn = database();
    let user = users::create_user(&conn, "anna", PASSWORD).unwrap();
    (conn, user)
}

/// A new empty directory in the temporary directory of the system. The directory and
/// its contents are deleted at the end of the test.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn tempdir() -> TempDir {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    loop {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
        let name = format!(
            "feljosit-test-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let path = std::env::temp_dir().join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return TempDir(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("Cannot make a temporary directory: {error}"),
        }
    }
}

/// An application with a database in a temporary directory.
pub struct TestApp {
    pub app: Arc<App>,
    pub dir: TempDir,
}

impl TestApp {
    pub fn new() -> TestApp {
        TestApp::with_settings(|_| {})
    }

    pub fn with_settings(change: impl FnOnce(&mut Settings)) -> TestApp {
        users::use_fast_hasher_for_tests();
        let dir = tempdir();
        let mut settings = Settings::new(dir.path().join("data"));
        change(&mut settings);
        let conn = db::open(&settings).unwrap();
        let app = App::new(settings, conn).unwrap();
        TestApp { app: Arc::new(app), dir }
    }

    pub fn create_user(&self, username: &str) -> User {
        self.app.with_db(|conn| users::create_user(conn, username, PASSWORD).unwrap())
    }

    pub fn client(&self) -> Client {
        Client { app: self.app.clone(), cookies: Vec::new(), host: "127.0.0.1".into(), loopback: true }
    }

    /// A client that is logged in as `username`, and its CSRF token.
    pub fn login(&self, username: &str) -> (Client, String) {
        let mut client = self.client();
        let page = client.get("/login");
        let response = client.post_no_follow(
            "/login",
            &[("username", username), ("password", PASSWORD), ("csrf_token", &csrf_from(&page.text()))],
        );
        assert_eq!(response.status, 303, "{}", response.text_body());
        let token = csrf_from(&client.get("/budget").text());
        (client, token)
    }

    /// Add the user "anna" and log in.
    pub fn anna(&self) -> (Client, String) {
        self.create_user("anna");
        self.login("anna")
    }

    /// Run a query that returns one value.
    pub fn scalar<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.app.with_db(|conn| conn.query_row(sql, [], |row| row.get(0)).unwrap())
    }
}

/// The value of the first CSRF token in a page.
pub fn csrf_from(html: &str) -> String {
    let marker = r#"name="csrf_token" value=""#;
    let start = html.find(marker).expect("No CSRF token in the page") + marker.len();
    let end = html[start..].find('"').unwrap();
    html[start..start + end].to_string()
}

/// A response of the test client.
pub struct TestResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl TestResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn location(&self) -> &str {
        self.header("location").unwrap_or("")
    }

    pub fn text_body(&self) -> String {
        self.text()
    }
}

/// A file for a multipart form.
pub struct FilePart<'a> {
    pub field: &'a str,
    pub filename: &'a str,
    pub content_type: &'a str,
    pub data: &'a [u8],
}

struct StoredCookie {
    name: String,
    value: String,
    path: String,
}

/// A client that keeps the cookies, as a browser.
pub struct Client {
    pub app: Arc<App>,
    cookies: Vec<StoredCookie>,
    pub host: String,
    pub loopback: bool,
}

pub fn encode(pairs: &[(&str, &str)]) -> String {
    feljosit::web::render::urlencode_pairs(pairs)
}

impl Client {
    fn cookie_header(&self, path: &str) -> Option<String> {
        let values: Vec<String> = self
            .cookies
            .iter()
            .filter(|c| path.starts_with(&c.path))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        (!values.is_empty()).then(|| values.join("; "))
    }

    fn store_cookies(&mut self, response: &Response) {
        for (_, value) in response.headers.iter().filter(|(n, _)| n == "set-cookie") {
            let mut parts = value.split(';').map(str::trim);
            let (name, cookie_value) = parts.next().unwrap().split_once('=').unwrap();
            let mut path = "/".to_string();
            let mut delete = false;
            for attribute in parts {
                let lower = attribute.to_lowercase();
                if let Some(p) = attribute.strip_prefix("Path=") {
                    path = p.to_string();
                }
                if lower == "max-age=0" {
                    delete = true;
                }
            }
            self.cookies.retain(|c| !(c.name == name && c.path == path));
            if !delete {
                self.cookies.push(StoredCookie { name: name.into(), value: cookie_value.into(), path });
            }
        }
    }

    /// Send a request. Do not follow redirects.
    pub fn send(
        &mut self,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> TestResponse {
        let target = target.split('#').next().unwrap();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let mut all_headers: Vec<(String, String)> = vec![("host".into(), self.host.clone())];
        if let Some(cookie) = self.cookie_header(path) {
            all_headers.push(("cookie".into(), cookie));
        }
        for (name, value) in headers {
            all_headers.push((name.to_lowercase(), value.to_string()));
        }
        if !body.is_empty() || method == "POST" {
            all_headers.push(("content-length".into(), body.len().to_string()));
        }
        let request = Request {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers: all_headers,
            body,
            peer_is_loopback: self.loopback,
        };
        let response = self.app.handle(&request);
        self.store_cookies(&response);
        TestResponse { status: response.status, headers: response.headers, body: response.body }
    }

    fn follow(&mut self, mut response: TestResponse) -> TestResponse {
        let mut count = 0;
        while matches!(response.status, 301 | 302 | 303 | 307) && count < 10 {
            let location = response.location().to_string();
            response = self.send("GET", &location, &[], Vec::new());
            count += 1;
        }
        response
    }

    pub fn get(&mut self, target: &str) -> TestResponse {
        let response = self.send("GET", target, &[], Vec::new());
        self.follow(response)
    }

    pub fn get_no_follow(&mut self, target: &str) -> TestResponse {
        self.send("GET", target, &[], Vec::new())
    }

    pub fn post_no_follow(&mut self, target: &str, form: &[(&str, &str)]) -> TestResponse {
        self.post_with_headers(target, form, &[])
    }

    pub fn post(&mut self, target: &str, form: &[(&str, &str)]) -> TestResponse {
        let response = self.post_no_follow(target, form);
        self.follow(response)
    }

    pub fn post_with_headers(
        &mut self,
        target: &str,
        form: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> TestResponse {
        let mut all = vec![("content-type", "application/x-www-form-urlencoded")];
        all.extend_from_slice(headers);
        self.send("POST", target, &all, encode(form).into_bytes())
    }

    /// Post a multipart form with files. Follow the redirects.
    pub fn post_files(
        &mut self,
        target: &str,
        form: &[(&str, &str)],
        files: &[FilePart<'_>],
    ) -> TestResponse {
        let boundary = "----feljosit-test-boundary-7d3b";
        let mut body = Vec::new();
        for (name, value) in form {
            body.extend(
                format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                    .bytes(),
            );
        }
        for file in files {
            body.extend(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n",
                    file.field, file.filename, file.content_type
                )
                .bytes(),
            );
            body.extend_from_slice(file.data);
            body.extend(b"\r\n");
        }
        body.extend(format!("--{boundary}--\r\n").bytes());
        let content_type = format!("multipart/form-data; boundary={boundary}");
        let response = self.send("POST", target, &[("content-type", &content_type)], body);
        self.follow(response)
    }

    /// Upload a statement on the import page. Return the page.
    pub fn import(&mut self, token: &str, name: &str, data: &[u8], account_id: &str) -> TestResponse {
        self.post_files(
            "/import",
            &[("csrf_token", token), ("account_id", account_id)],
            &[FilePart { field: "statement", filename: name, content_type: "application/xml", data }],
        )
    }

    pub fn import_fixture(&mut self, token: &str, name: &str, account_id: &str) -> String {
        self.import(token, name, &fixture(name), account_id).text()
    }

    /// Add an account with the form. Additional fields replace the defaults.
    pub fn add_account(&mut self, token: &str, fields: &[(&str, &str)]) {
        let mut values: HashMap<&str, &str> =
            HashMap::from([("csrf_token", token), ("name", "Private"), ("account_type", "checking")]);
        for (k, v) in fields {
            values.insert(k, v);
        }
        let pairs: Vec<(&str, &str)> = values.into_iter().collect();
        let response = self.post("/accounts", &pairs);
        assert_eq!(response.status, 200);
    }
}

/// Add an account in CHF with the budget currency CHF.
pub fn add_account(
    conn: &Connection,
    user: &User,
    name: &str,
    kind: feljosit::models::AccountType,
    iban: &str,
) -> i64 {
    let mut new = feljosit::services::accounts::NewAccount::new(name, kind, "CHF");
    new.iban = Some(iban);
    feljosit::services::accounts::create_account(conn, user.id, &new).unwrap().id
}

/// The ID of the category of the user with the name.
pub fn category_id(conn: &Connection, user: &User, name: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM categories WHERE user_id = ?1 AND name = ?2",
        rusqlite::params![user.id, name],
        |r| r.get(0),
    )
    .unwrap()
}

/// Import data. Return the result or the error message.
pub fn import_data(
    conn: &Connection,
    user: &User,
    filename: &str,
    data: &[u8],
    account_id: Option<i64>,
) -> Result<feljosit::services::importer::ImportResult, feljosit::services::importer::ImportError> {
    let request =
        feljosit::services::importer::ImportRequest { filename, data, account_id, budget_currency: None };
    feljosit::services::importer::import_statement_file(conn, user.id, &request)
}

/// Import a fixture file.
pub fn import_fixture(
    conn: &Connection,
    user: &User,
    name: &str,
    account_id: Option<i64>,
) -> Result<feljosit::services::importer::ImportResult, String> {
    import_data(conn, user, name, &fixture(name), account_id).map_err(|e| e.to_string())
}

/// Run a query that returns one value.
pub fn scalar<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str) -> T {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

pub fn date(y: i32, m: u32, d: u32) -> feljosit::dates::Date {
    feljosit::dates::ymd(y, m, d).unwrap()
}

pub const PRIVATE_IBAN: &str = "CH8900235000012345678";
pub const SAVINGS_IBAN: &str = "CH0300700110000123456";

/// Add an account without IBAN. An account in a currency other than CHF is a tracking
/// account.
pub fn plain_account(
    conn: &Connection,
    user: &User,
    name: &str,
    kind: feljosit::models::AccountType,
    currency: &str,
) -> feljosit::models::Account {
    let mut new = feljosit::services::accounts::NewAccount::new(name, kind, currency);
    if currency != "CHF" {
        new.on_budget = Some(false);
    }
    feljosit::services::accounts::create_account(conn, user.id, &new).unwrap()
}

/// Add a manual transaction. Return its ID.
pub fn manual_tx(
    conn: &Connection,
    account: &feljosit::models::Account,
    on: feljosit::dates::Date,
    amount: i64,
    category_id: Option<i64>,
) -> i64 {
    let manual = feljosit::services::accounts::ManualTransaction {
        booking_date: on,
        amount_minor: amount,
        counterparty: "x",
        memo: "",
        category_id,
    };
    feljosit::services::accounts::add_manual_transaction(conn, account, &manual).unwrap()
}

pub fn tx(conn: &Connection, id: i64) -> feljosit::models::Transaction {
    feljosit::models::Transaction::get(conn, id).unwrap().unwrap()
}

pub fn is_transfer(conn: &Connection, id: i64) -> bool {
    tx(conn, id).is_transfer
}

/// The pairs: (outflow, inflow, method, status), in the order of the IDs.
pub fn pairs(conn: &Connection) -> Vec<(i64, i64, String, String)> {
    let mut stmt =
        conn.prepare("SELECT outflow_id, inflow_id, method, status FROM transfer_pairs ORDER BY id").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
