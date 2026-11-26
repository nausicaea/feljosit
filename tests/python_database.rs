//! The Rust version uses the database of the Python version (Feljosit 0.3.0) without a
//! change. The fixture is a dump of a database that the Python version made.

mod common;

use common::*;
use feljosit::db;
use feljosit::services::accounts;
use feljosit::web::App;

fn python_database(path: &std::path::Path) {
    let dump = std::fs::read_to_string(fixture_path("python_0.3.0.sql")).unwrap();
    let conn = rusqlite::Connection::open(path).unwrap();
    // The dump has the tables in alphabetical order, not in the order of the foreign keys.
    conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
    conn.execute_batch(&dump).unwrap();
}

#[test]
fn the_python_database_works() {
    let dir = tempfile::tempdir().unwrap();
    let settings = feljosit::config::Settings::new(dir.path().to_path_buf());
    python_database(&settings.database_path());
    let mut conn = db::open(&settings).unwrap();
    assert_eq!(db::current_revision(&conn).unwrap().as_deref(), Some(db::head_revision()));
    db::migrate(&mut conn).unwrap();
    let balances = accounts::balances(&conn, 1).unwrap();
    assert_eq!(balances[&1], 425005);

    users_can_log_in_and_use_the_pages(App::new(settings, conn).unwrap());
}

fn users_can_log_in_and_use_the_pages(app: App) {
    feljosit::services::users::use_fast_hasher_for_tests();
    let test = TestApp { app: std::sync::Arc::new(app), dir: tempfile::tempdir().unwrap() };
    // The password hash of the Python version is valid.
    let (mut http, token) = test.login("anna");
    let page = http.get("/budget?month=2026-10").text();
    assert!(page.contains("Groceries"));
    assert!(page.contains("500.00")); // The assigned amount.
    assert!(http.get("/transactions").text().contains("Muster AG"));
    assert!(http.get("/categories").text().contains("migros"));
    // The import of the same file again is refused: the hash of the file is in the
    // database. A changed file gives no duplicates: the fingerprints are the same.
    assert!(http.import_fixture(&token, "camt053_v04_private.xml", "").contains("imported before"));
    let mut data = fixture("camt053_v04_private.xml");
    data.push(b'\n');
    let text = http.import(&token, "again.xml", &data, "").text();
    assert!(text.contains("0 new transactions"), "{text}");
    assert!(text.contains("7 transactions were imported before"));
    assert_eq!(http.get("/reports?period=all").status, 200);
}
