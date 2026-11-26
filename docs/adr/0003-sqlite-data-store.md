# ADR-0003: Use SQLite as the data store

- Status: Accepted (Alembic is superseded by ADR-0018)
- Date: 2026-11-25

## Context

The application has few users and little data (some thousand transactions each
year). The data must be easy to back up. The installation must be simple.

## Options

1. SQLite.
2. SQLite with encryption (SQLCipher).
3. PostgreSQL.

## Decision

We use SQLite in WAL mode. Alembic manages the schema. The application applies
the migrations when it starts.

The database file has mode `0600` and the data directory has mode `0700`.
Thus only the owner of the files can read them. The application sets
`PRAGMA secure_delete=ON`. Thus SQLite writes zeros over deleted data.

We do not use SQLCipher. The Python bindings need a special build, and this
makes the installation and the maintenance difficult. We recommend full-disk
encryption of the operating system (FileVault, BitLocker, LUKS) instead.

Some reports use SQLite date functions (`date(x, 'start of month')`).

## Consequences

- Good: A backup is one copy of one file.
- Good: No database server is necessary.
- Bad: The data is not encrypted by the application. The security relies on
  full-disk encryption and on file permissions.
- Bad: A change to another database needs changes in the reports.
