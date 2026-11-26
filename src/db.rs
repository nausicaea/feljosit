//! Database connection, migrations and value conversions.
//!
//! The schema is the same as the schema of the Python version (Alembic revisions
//! 0001 and 0002). The table `alembic_version` keeps the revision. Thus the two
//! versions can open the same database file.

use std::fs;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::config::Settings;
use crate::dates::{self, Date, Timestamp};

/// The migrations in order: revision and SQL.
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001", include_str!("../migrations/0001_initial_schema.sql")),
    ("0002", include_str!("../migrations/0002_transfer_pairs.sql")),
];

/// The format of a date-time value in the database (UTC, as SQLAlchemy writes it).
pub const DATETIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.6f";

pub fn now() -> Timestamp {
    dates::now()
}

/// The text of a date-time value for the database.
pub fn datetime_text(value: Timestamp) -> String {
    dates::format_utc(value, DATETIME_FORMAT)
}

/// The current time as text for the database.
pub fn now_text() -> String {
    datetime_text(now())
}

/// Parse a date-time value from the database.
pub fn parse_datetime(text: &str) -> Option<Timestamp> {
    dates::parse_utc(text, "%Y-%m-%d %H:%M:%S%.f").or_else(|| dates::parse_utc(text, "%Y-%m-%dT%H:%M:%S%.f"))
}

/// Parse a date value (`YYYY-MM-DD`) from the database.
pub fn parse_date(text: &str) -> Date {
    dates::parse(text.get(..10).unwrap_or(text), "%Y-%m-%d").unwrap_or(Date::MIN)
}

pub fn date_text(value: Date) -> String {
    dates::format(value, "%Y-%m-%d")
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL; PRAGMA secure_delete=ON;")?;
    let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    Ok(())
}

/// Make the data directory. Only the owner can read and write it (mode 0700).
pub fn ensure_data_dir(settings: &Settings) -> std::io::Result<()> {
    fs::create_dir_all(&settings.data_dir)?;
    restrict(&settings.data_dir, 0o700)
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> std::io::Result<()> {
    // On Windows, the files in the profile of the user are private by default.
    Ok(())
}

/// Set the database files to mode 0600 (only the owner can read and write).
pub fn restrict_database_file(settings: &Settings) -> std::io::Result<()> {
    let path = settings.database_path();
    for suffix in ["", "-wal", "-shm"] {
        let file = path.with_file_name(format!(
            "{}{suffix}",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("feljosit.sqlite3")
        ));
        if file.exists() {
            restrict(&file, 0o600)?;
        }
    }
    Ok(())
}

/// Open the database file of the settings. Make the data directory first if necessary.
pub fn open(settings: &Settings) -> Result<Connection, String> {
    ensure_data_dir(settings)
        .map_err(|e| format!("Cannot make the data directory {}: {e}", settings.data_dir.display()))?;
    let path = settings.database_path();
    if !path.exists() {
        // Make the file with mode 0600 before SQLite opens it.
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).map_err(|e| format!("Cannot make {}: {e}", path.display()))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    configure(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Open a database in memory (for tests).
pub fn open_in_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    Ok(conn)
}

/// The current revision of the schema, or `None` for an empty database.
pub fn current_revision(conn: &Connection) -> rusqlite::Result<Option<String>> {
    let has_table: bool = conn.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'alembic_version'",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(None);
    }
    conn.query_row("SELECT version_num FROM alembic_version", [], |row| row.get(0)).optional()
}

/// Upgrade the database schema to the latest revision.
pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    let current = current_revision(conn).map_err(|e| e.to_string())?;
    let start = match &current {
        None => 0,
        Some(revision) => {
            MIGRATIONS.iter().position(|(r, _)| r == revision).map(|i| i + 1).ok_or_else(|| {
                format!(
                    "The database has the schema revision {revision}. This version of Feljosit does \
                     not know it. Use a newer version of Feljosit."
                )
            })?
        }
    };
    if start == MIGRATIONS.len() {
        return Ok(());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if current.is_none() {
        tx.execute_batch(
            "CREATE TABLE alembic_version (\n\tversion_num VARCHAR(32) NOT NULL, \n\t\
             CONSTRAINT alembic_version_pkc PRIMARY KEY (version_num)\n);",
        )
        .map_err(|e| e.to_string())?;
    }
    for (revision, sql) in &MIGRATIONS[start..] {
        tx.execute_batch(sql).map_err(|e| format!("Migration {revision} failed: {e}"))?;
        tx.execute("DELETE FROM alembic_version", []).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO alembic_version (version_num) VALUES (?1)", [revision])
            .map_err(|e| e.to_string())?;
        log::info(&format!("The database schema is upgraded to revision {revision}."));
    }
    tx.commit().map_err(|e| e.to_string())
}

/// The latest schema revision.
pub fn head_revision() -> &'static str {
    MIGRATIONS[MIGRATIONS.len() - 1].0
}

/// Simple log output on stderr.
pub mod log {
    pub fn info(message: &str) {
        eprintln!("INFO {message}");
    }

    pub fn warning(message: &str) {
        eprintln!("WARNING {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(current_revision(&conn).unwrap().as_deref(), Some("0002"));
    }

    #[test]
    fn unknown_revision_is_refused() {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        conn.execute("UPDATE alembic_version SET version_num = '9999'", []).unwrap();
        assert!(migrate(&mut conn).unwrap_err().contains("9999"));
    }

    #[test]
    fn datetime_round_trip() {
        let value = parse_datetime("2026-11-25 10:11:12.123456").unwrap();
        assert_eq!(datetime_text(value), "2026-11-25 10:11:12.123456");
        assert!(parse_datetime("2026-11-25 10:11:12").is_some());
    }
}
