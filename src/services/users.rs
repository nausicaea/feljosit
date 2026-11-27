//! Users, passwords and login sessions.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use argon2::{Algorithm, Argon2, Params, Version};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::defaults::create_default_categories;
use super::{Error, Result};
use crate::dates::{self, Timestamp};
use crate::db::{datetime_text, now, now_text, parse_datetime};
use crate::models::User;

pub const MIN_PASSWORD_LENGTH: usize = 12;
pub const MAX_PASSWORD_LENGTH: usize = 1024;
/// Failed logins for one user from one client before the lock (ADR-0024).
pub const MAX_FAILED_LOGINS: i64 = 5;
/// Failed logins for one user from all clients together before the lock of the user.
/// This limit stops an attacker with many addresses. It is high, thus one device
/// cannot lock the user for the other devices.
pub const MAX_FAILED_LOGINS_ALL_CLIENTS: i64 = 50;
pub const LOCKOUT_MINUTES: i64 = 15;
/// The failed logins of a client without a lock are forgotten after this time.
const FAILURE_MEMORY_SECONDS: i64 = 24 * 60 * 60;

/// Argon2id parameters: memory in KiB, iterations, lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HashParams {
    memory: u32,
    time: u32,
    lanes: u32,
}

/// The RFC 9106 "second recommended" parameters (64 MiB, t=3, p=4). They are the same
/// as in the Python version (argon2-cffi), thus the stored hashes stay valid.
const PRODUCTION: HashParams = HashParams { memory: 64 * 1024, time: 3, lanes: 4 };
/// Small parameters for the tests. The production parameters are slow on purpose.
const FAST: HashParams = HashParams { memory: 8, time: 1, lanes: 1 };
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

static USE_FAST_HASHER: AtomicBool = AtomicBool::new(false);

/// Use small Argon2 parameters. Only for tests.
pub fn use_fast_hasher_for_tests() {
    USE_FAST_HASHER.store(true, Ordering::SeqCst);
}

fn params() -> HashParams {
    if USE_FAST_HASHER.load(Ordering::SeqCst) { FAST } else { PRODUCTION }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("The operating system did not supply random bytes");
    bytes
}

/// A random token with 32 bytes of entropy, URL-safe (as Python's `token_urlsafe(32)`).
pub fn random_token() -> String {
    crate::encoding::base64_url_safe_no_pad(&random_bytes::<32>())
}

fn argon2(p: HashParams) -> Argon2<'static> {
    let params =
        Params::new(p.memory, p.time, p.lanes, Some(HASH_BYTES)).expect("The Argon2 parameters are valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hash a password. The result is a PHC string, for example `$argon2id$v=19$m=65536,...`.
pub fn hash_password(password: &str) -> String {
    let p = params();
    let salt = random_bytes::<SALT_BYTES>();
    let mut out = [0u8; HASH_BYTES];
    argon2(p)
        .hash_password_into(password.as_bytes(), &salt, &mut out)
        .expect("Argon2 hashing with valid parameters does not fail");
    format!(
        "$argon2id$v=19$m={},t={},p={}${}${}",
        p.memory,
        p.time,
        p.lanes,
        crate::encoding::base64_standard_no_pad(&salt),
        crate::encoding::base64_standard_no_pad(&out)
    )
}

/// The parts of a PHC string.
struct ParsedHash {
    params: HashParams,
    salt: Vec<u8>,
    hash: Vec<u8>,
}

fn parse_hash(encoded: &str) -> Option<ParsedHash> {
    let parts: Vec<&str> = encoded.split('$').collect();
    let [empty, algorithm, version, params, salt, hash] = parts.as_slice() else {
        return None;
    };
    if !empty.is_empty() || *algorithm != "argon2id" || *version != "v=19" {
        return None;
    }
    let mut p = HashParams { memory: 0, time: 0, lanes: 0 };
    for item in params.split(',') {
        let (key, value) = item.split_once('=')?;
        let value: u32 = value.parse().ok()?;
        match key {
            "m" => p.memory = value,
            "t" => p.time = value,
            "p" => p.lanes = value,
            _ => return None,
        }
    }
    let salt = crate::encoding::base64_standard_no_pad_decode(salt)?;
    let hash = crate::encoding::base64_standard_no_pad_decode(hash)?;
    if hash.is_empty() || hash.len() > 1024 || p.memory > 4 * 1024 * 1024 {
        return None;
    }
    Some(ParsedHash { params: p, salt, hash })
}

/// Return true if the password agrees with the PHC string.
pub fn verify_password(encoded: &str, password: &str) -> bool {
    let Some(parsed) = parse_hash(encoded) else {
        return false;
    };
    let Ok(argon_params) =
        Params::new(parsed.params.memory, parsed.params.time, parsed.params.lanes, Some(parsed.hash.len()))
    else {
        return false;
    };
    let mut out = vec![0u8; parsed.hash.len()];
    if Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params)
        .hash_password_into(password.as_bytes(), &parsed.salt, &mut out)
        .is_err()
    {
        return false;
    }
    constant_time_eq(&out, &parsed.hash)
}

/// Return true if the hash does not use the current parameters.
fn needs_rehash(encoded: &str) -> bool {
    parse_hash(encoded)
        .is_none_or(|p| p.params != params() || p.hash.len() != HASH_BYTES || p.salt.len() != SALT_BYTES)
}

fn dummy_hash() -> &'static str {
    static FAST_DUMMY: OnceLock<String> = OnceLock::new();
    static PRODUCTION_DUMMY: OnceLock<String> = OnceLock::new();
    let cell = if params() == FAST { &FAST_DUMMY } else { &PRODUCTION_DUMMY };
    cell.get_or_init(|| hash_password(&random_token()))
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn hash_token(token: &str) -> String {
    crate::encoding::hex(Sha256::digest(token.as_bytes()))
}

pub fn validate_password(password: &str) -> Result<()> {
    let length = password.chars().count();
    if length < MIN_PASSWORD_LENGTH {
        return Err(Error::service(format!(
            "The password must have at least {MIN_PASSWORD_LENGTH} characters."
        )));
    }
    if length > MAX_PASSWORD_LENGTH {
        return Err(Error::service("The password is too long."));
    }
    Ok(())
}

fn valid_username(username: &str) -> bool {
    let bytes = username.as_bytes();
    let allowed = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    (2..=64).contains(&bytes.len())
        && allowed(&bytes[0])
        && bytes[1..].iter().all(|b| allowed(b) || matches!(b, b'.' | b'_' | b'-'))
}

pub fn count_users(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT count(*) FROM users", [], |row| row.get(0))
}

pub fn create_user(conn: &Connection, username: &str, password: &str) -> Result<User> {
    let username = username.trim().to_lowercase();
    if !valid_username(&username) {
        return Err(Error::service("The username must have 2 to 64 characters: a-z, 0-9, '.', '_' or '-'."));
    }
    validate_password(password)?;
    if User::by_username(conn, &username)?.is_some() {
        return Err(Error::service("The username exists already."));
    }
    conn.execute(
        "INSERT INTO users (username, password_hash, failed_logins, locked_until, created_at) \
         VALUES (?1, ?2, 0, NULL, ?3)",
        params![username, hash_password(password), now_text()],
    )?;
    let id = conn.last_insert_rowid();
    create_default_categories(conn, id)?;
    Ok(User::by_id(conn, id)?.expect("The user was just added"))
}

/// Set a new password chosen by the user. The password is not temporary.
pub fn change_password(conn: &Connection, user_id: i64, new_password: &str) -> Result<()> {
    validate_password(new_password)?;
    conn.execute(
        "UPDATE users SET password_hash = ?1, must_change_password = 0 WHERE id = ?2",
        params![hash_password(new_password), user_id],
    )?;
    // Log out all sessions of the user.
    conn.execute("DELETE FROM sessions WHERE user_id = ?1", [user_id])?;
    Ok(())
}

/// Mark the password of the user as temporary (or not). A user with a temporary
/// password must change it before other pages open (ADR-0024).
pub fn set_temporary_password(conn: &Connection, user_id: i64, temporary: bool) -> Result<()> {
    conn.execute("UPDATE users SET must_change_password = ?1 WHERE id = ?2", params![temporary, user_id])?;
    Ok(())
}

fn locked(until: Option<&str>) -> bool {
    until.and_then(parse_datetime).is_some_and(|t| t > now())
}

/// Return the user if the credentials are correct. `client` identifies the device
/// (see [`crate::net::client_key`]).
///
/// The function always calculates one Argon2 hash. Thus the response time does not
/// show if the username exists or if the user is locked. Two limits apply (ADR-0024):
///
/// - After [`MAX_FAILED_LOGINS`] failures from one client, the user is locked for
///   [`LOCKOUT_MINUTES`] minutes for that client only.
/// - After [`MAX_FAILED_LOGINS_ALL_CLIENTS`] failures from all clients, the user is
///   locked for [`LOCKOUT_MINUTES`] minutes for all clients.
pub fn authenticate(conn: &Connection, username: &str, password: &str, client: &str) -> Result<Option<User>> {
    if password.chars().count() > MAX_PASSWORD_LENGTH {
        return Ok(None);
    }
    let Some(user) = User::by_username(conn, &username.trim().to_lowercase())? else {
        verify_password(dummy_hash(), password);
        return Ok(None);
    };
    forget_old_failures(conn)?;
    let client_lock: Option<String> = conn
        .query_row(
            "SELECT locked_until FROM login_failures WHERE user_id = ?1 AND client = ?2",
            params![user.id, client],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if locked(user.locked_until.as_deref()) || locked(client_lock.as_deref()) {
        verify_password(dummy_hash(), password);
        return Ok(None);
    }
    if !verify_password(&user.password_hash, password) {
        record_failure(conn, &user, client)?;
        return Ok(None);
    }
    conn.execute("UPDATE users SET failed_logins = 0, locked_until = NULL WHERE id = ?1", [user.id])?;
    conn.execute("DELETE FROM login_failures WHERE user_id = ?1 AND client = ?2", params![user.id, client])?;
    if needs_rehash(&user.password_hash) {
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![hash_password(password), user.id],
        )?;
    }
    Ok(User::by_id(conn, user.id)?)
}

fn record_failure(conn: &Connection, user: &User, client: &str) -> Result<()> {
    let lock_until = datetime_text(dates::add_seconds(now(), LOCKOUT_MINUTES * 60));
    let failed = user.failed_logins + 1;
    if failed >= MAX_FAILED_LOGINS_ALL_CLIENTS {
        conn.execute(
            "UPDATE users SET failed_logins = 0, locked_until = ?1 WHERE id = ?2",
            params![lock_until, user.id],
        )?;
    } else {
        conn.execute("UPDATE users SET failed_logins = ?1 WHERE id = ?2", params![failed, user.id])?;
    }
    conn.execute(
        "INSERT INTO login_failures (user_id, client, failed_logins, locked_until, updated_at) \
         VALUES (?1, ?2, 1, NULL, ?3) \
         ON CONFLICT (user_id, client) DO UPDATE SET \
         failed_logins = failed_logins + 1, updated_at = excluded.updated_at",
        params![user.id, client, now_text()],
    )?;
    conn.execute(
        "UPDATE login_failures SET failed_logins = 0, locked_until = ?3 \
         WHERE user_id = ?1 AND client = ?2 AND failed_logins >= ?4",
        params![user.id, client, lock_until, MAX_FAILED_LOGINS],
    )?;
    Ok(())
}

/// Delete the failed logins of clients without a lock after one day. Thus the table
/// stays small.
fn forget_old_failures(conn: &Connection) -> Result<()> {
    let before = datetime_text(dates::add_seconds(now(), -FAILURE_MEMORY_SECONDS));
    conn.execute(
        "DELETE FROM login_failures WHERE updated_at < ?1 AND (locked_until IS NULL OR locked_until < ?2)",
        params![before, now_text()],
    )?;
    Ok(())
}

/// Set a new password from the command line. This also removes all locks of the
/// user. A `temporary` password must be changed at the next login.
pub fn reset_password(conn: &Connection, username: &str, password: &str, temporary: bool) -> Result<()> {
    let user = User::by_username(conn, &username.trim().to_lowercase())?
        .ok_or_else(|| Error::service("The user does not exist."))?;
    change_password(conn, user.id, password)?;
    set_temporary_password(conn, user.id, temporary)?;
    conn.execute("UPDATE users SET failed_logins = 0, locked_until = NULL WHERE id = ?1", [user.id])?;
    conn.execute("DELETE FROM login_failures WHERE user_id = ?1", [user.id])?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub token: String,
    pub csrf_token: String,
    pub expires_at: Timestamp,
}

/// A login session from the database.
#[derive(Debug, Clone)]
pub struct Session {
    pub token_hash: String,
    pub user_id: i64,
    pub csrf_token: String,
    pub flash_message: Option<String>,
    pub flash_is_error: bool,
}

pub fn start_session(conn: &Connection, user_id: i64, lifetime_seconds: i64) -> Result<NewSession> {
    let token = random_token();
    let csrf = random_token();
    let expires = dates::add_seconds(now(), lifetime_seconds);
    let created = now_text();
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, csrf_token, flash_message, flash_is_error, \
         created_at, expires_at) VALUES (?1, ?2, ?3, NULL, 0, ?4, ?5)",
        params![hash_token(&token), user_id, csrf, created, datetime_text(expires)],
    )?;
    conn.execute("DELETE FROM sessions WHERE expires_at < ?1", [now_text()])?;
    Ok(NewSession { token, csrf_token: csrf, expires_at: expires })
}

pub fn load_session(conn: &Connection, token: &str) -> Result<Option<Session>> {
    if token.is_empty() || token.len() > 128 {
        return Ok(None);
    }
    let found = conn
        .query_row(
            "SELECT token_hash, user_id, csrf_token, flash_message, flash_is_error, expires_at \
             FROM sessions WHERE token_hash = ?1",
            [hash_token(token)],
            |row| {
                Ok((
                    Session {
                        token_hash: row.get(0)?,
                        user_id: row.get(1)?,
                        csrf_token: row.get(2)?,
                        flash_message: row.get(3)?,
                        flash_is_error: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    Ok(found.and_then(|(session, expires)| parse_datetime(&expires).filter(|e| *e > now()).map(|_| session)))
}

pub fn end_session(conn: &Connection, token: &str) -> Result<()> {
    conn.execute("DELETE FROM sessions WHERE token_hash = ?1", [hash_token(token)])?;
    Ok(())
}

/// Set the message for the next page of a session.
pub fn set_flash(conn: &Connection, token_hash: &str, message: &str, error: bool) -> Result<()> {
    let message: String = message.chars().take(500).collect();
    conn.execute(
        "UPDATE sessions SET flash_message = ?1, flash_is_error = ?2 WHERE token_hash = ?3",
        params![message, error, token_hash],
    )?;
    Ok(())
}

/// Remove and return the message for the next page of a session.
pub fn pop_flash(conn: &Connection, token_hash: &str) -> Result<Option<(String, bool)>> {
    let found: Option<(Option<String>, bool)> = conn
        .query_row(
            "SELECT flash_message, flash_is_error FROM sessions WHERE token_hash = ?1",
            [token_hash],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match found {
        Some((Some(message), error)) if !message.is_empty() => {
            conn.execute(
                "UPDATE sessions SET flash_message = NULL, flash_is_error = 0 WHERE token_hash = ?1",
                [token_hash],
            )?;
            Ok(Some((message, error)))
        }
        _ => Ok(None),
    }
}

pub fn csrf_matches(expected: &str, received: Option<&str>) -> bool {
    received.is_some_and(|r| constant_time_eq(expected.as_bytes(), r.as_bytes()))
}
