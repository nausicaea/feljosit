//! Users, passwords and sessions.

mod common;

use common::*;
use feljosit::db::{datetime_text, now};
use feljosit::services::users;

#[test]
fn invalid_usernames() {
    let conn = database();
    let long = "x".repeat(65);
    for name in ["a", "Has Space", long.as_str(), "-start", "ümlaut"] {
        assert!(users::create_user(&conn, name, PASSWORD).is_err(), "{name}");
    }
}

#[test]
fn password_rules() {
    let conn = database();
    assert!(users::create_user(&conn, "anna", "short").unwrap_err().to_string().contains("at least"));
    assert!(
        users::create_user(&conn, "anna", &"x".repeat(2000)).unwrap_err().to_string().contains("too long")
    );
}

#[test]
fn duplicate_username() {
    let (conn, _) = database_with_user();
    assert!(users::create_user(&conn, "ANNA", PASSWORD).unwrap_err().to_string().contains("exists"));
}

#[test]
fn new_user_has_default_categories() {
    let (conn, user) = database_with_user();
    let count: i64 = scalar(&conn, &format!("SELECT count(*) FROM categories WHERE user_id = {}", user.id));
    assert!(count > 15);
}

#[test]
fn authenticate() {
    let (conn, user) = database_with_user();
    assert_eq!(users::authenticate(&conn, " Anna ", PASSWORD).unwrap().map(|u| u.id), Some(user.id));
    assert!(users::authenticate(&conn, "anna", "wrong").unwrap().is_none());
    assert!(users::authenticate(&conn, "nobody", PASSWORD).unwrap().is_none());
    assert!(users::authenticate(&conn, "anna", &"x".repeat(5000)).unwrap().is_none());
    assert_eq!(scalar::<i64>(&conn, "SELECT failed_logins FROM users"), 1);
    assert!(users::authenticate(&conn, "anna", PASSWORD).unwrap().is_some());
    assert_eq!(scalar::<i64>(&conn, "SELECT failed_logins FROM users"), 0);
}

#[test]
fn lockout_expires() {
    let (conn, _) = database_with_user();
    let set_lock = |seconds: i64| {
        let until = datetime_text(now() + chrono::Duration::seconds(seconds));
        conn.execute("UPDATE users SET locked_until = ?1", [until]).unwrap();
    };
    set_lock(60);
    assert!(users::authenticate(&conn, "anna", PASSWORD).unwrap().is_none());
    set_lock(-1);
    assert!(users::authenticate(&conn, "anna", PASSWORD).unwrap().is_some());
}

#[test]
fn sessions() {
    let (conn, user) = database_with_user();
    let new = users::start_session(&conn, user.id, 60).unwrap();
    let session = users::load_session(&conn, &new.token).unwrap().unwrap();
    assert_eq!(session.user_id, user.id);
    // Only the hash of the token is in the database.
    assert_ne!(session.token_hash, new.token);
    let stored: i64 = conn
        .query_row("SELECT count(*) FROM sessions WHERE token_hash = ?1", [&new.token], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, 0);
    assert!(users::load_session(&conn, "").unwrap().is_none());
    assert!(users::load_session(&conn, &"x".repeat(200)).unwrap().is_none());
    let past = datetime_text(now() - chrono::Duration::seconds(1));
    conn.execute("UPDATE sessions SET expires_at = ?1", [past]).unwrap();
    assert!(users::load_session(&conn, &new.token).unwrap().is_none());
    users::end_session(&conn, &new.token).unwrap();
    assert!(users::load_session(&conn, &new.token).unwrap().is_none());
}

#[test]
fn csrf_matches() {
    assert!(users::csrf_matches("abc", Some("abc")));
    assert!(!users::csrf_matches("abc", Some("abd")));
    assert!(!users::csrf_matches("abc", None));
}

/// A hash from the Python version (argon2-cffi, the same parameters). The users of the
/// Python version must be able to log in after the update.
#[test]
fn hashes_of_the_python_version_are_valid() {
    let python_hash =
        "$argon2id$v=19$m=65536,t=3,p=4$K6pG/pNR/O0edmlU7LE+vw$IR8+Fv2Es0TfIBnwcnWmyFcbAf9+CbbzT2TyIaKW5X8";
    assert!(users::verify_password(python_hash, "a long enough password"));
    assert!(!users::verify_password(python_hash, "A long enough password"));
}

#[test]
fn own_hashes() {
    users::use_fast_hasher_for_tests();
    let own = users::hash_password("a long enough password");
    assert!(own.starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
    assert!(users::verify_password(&own, "a long enough password"));
    assert!(!users::verify_password(&own, "a different password"));
    assert!(!users::verify_password("not a hash", "x"));
    assert!(!users::verify_password("$argon2i$v=19$m=8,t=1,p=1$AAAA$AAAA", "x"));
}

#[test]
fn login_rehashes_old_parameters() {
    let (conn, user) = database_with_user();
    let python_hash =
        "$argon2id$v=19$m=65536,t=3,p=4$K6pG/pNR/O0edmlU7LE+vw$IR8+Fv2Es0TfIBnwcnWmyFcbAf9+CbbzT2TyIaKW5X8";
    conn.execute(
        "UPDATE users SET password_hash = ?1 WHERE id = ?2",
        rusqlite::params![python_hash, user.id],
    )
    .unwrap();
    // The tests use other (small) parameters. Thus the login makes a new hash.
    assert!(users::authenticate(&conn, "anna", "a long enough password").unwrap().is_some());
    let stored: String = scalar(&conn, "SELECT password_hash FROM users");
    assert!(stored.starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
    assert!(users::authenticate(&conn, "anna", "a long enough password").unwrap().is_some());
}
