# Threat model

This document uses the STRIDE method. It describes the assets, the trust
boundaries, the threats and the controls.

## Assets

1. Transactions, balances and budgets of the users (very sensitive).
2. Passwords and sessions.
3. The availability of the application.

## Trust boundaries

1. The browser and the server. Other web pages in the same browser are not trusted.
2. Other processes and users on the same computer.
3. Uploaded files. A bank statement can be manipulated before the upload.
4. The dependencies (crates.io, GitHub Actions).

## Threats and controls

| Threat (STRIDE) | Example | Controls |
| --- | --- | --- |
| Spoofing | Password guessing | Argon2id, lockout after 5 failures, minimum length 12. |
| Spoofing | Session theft | Random tokens, only the hash in the database, `HttpOnly`, `SameSite=Strict`, expiry after 12 hours. |
| Tampering | Cross-site request forgery from a different web page | CSRF tokens, Origin check, `SameSite=Strict`. |
| Tampering | DNS rebinding to `127.0.0.1` | `Host` header allow list. |
| Tampering | SQL injection | `rusqlite` with bound parameters. `LIKE` patterns are escaped. |
| Repudiation | A user denies an import | Each import is recorded with the file hash and the time. |
| Information disclosure | Cross-site scripting | Template autoescape (minijinja), strict Content Security Policy, no inline scripts. |
| Information disclosure | A user reads the data of a different user | Each query filters by `user_id`. Tests check this. |
| Information disclosure | XXE in a camt.053 file | The parser (`roxmltree`) refuses DTDs. Thus entities and external references are not possible (ADR-0018). |
| Information disclosure | Other local users read the database | File mode `0600`, directory mode `0700`, `umask 077` (Linux, macOS). On Windows, the profile directory of the user. |
| Information disclosure | A stolen laptop | Full-disk encryption (operator responsibility, see ADR-0003). |
| Information disclosure | A copy of the database file (for example a backup) | Encrypted backups (operator responsibility, see SECURITY.md and ADR-0015). |
| Information disclosure | The operator reads the data of all users | Not prevented (ADR-0015). Users who do not trust each other use separate instances. |
| Information disclosure | Error messages show data | Validation errors do not show the input. No debug mode. |
| Denial of service | "Billion laughs" or ZIP bomb | Entities refused, ZIP limits (10 MB for each file, 25 MB in total), upload limit of 10 MB (ADR-0014). |
| Denial of service | A credit card CSV file with very many lines or very long fields | Limits of 100,000 lines and 1,000 characters for each field, upload limit of 10 MB (ADR-0017). |
| Denial of service | A very large or deep XML tree, or very many attributes | A pre-scan applies the limits for the number of elements, the nesting depth and the attributes before the parse (ADR-0014, ADR-0018). |
| Denial of service | An amount such as `1E999999` uses the CPU | Strict amount syntax before the conversion (ADR-0014). |
| Denial of service | A long import stops all requests | The requests run in worker threads, one after the other (ADR-0018). The limits of ADR-0014 keep an import short. |
| Spoofing | Somebody else adds the first user | Only while the database has no user, only from the same computer (ADR-0019). |
| Denial of service | ReDoS in categorisation rules | Rules use text comparison, not regular expressions. |
| Elevation of privilege | Open redirect | Redirect targets must be local paths. |
| Supply chain | A compromised dependency | Exact versions in `Cargo.toml` and `Cargo.lock` (with checksums), pinned action SHAs, Renovate with 7-day delay (ADR-0012), `cargo audit`, CodeQL. |
| Supply chain | A manipulated executable | The release workflow builds the files on GitHub. The release has the file `SHA256SUMS`. The files are not signed (ADR-0018). |

## Residual risks

1. The database is not encrypted by the application (see ADR-0003 and ADR-0015).
2. There is no two-factor authentication (see ADR-0007).
3. Malware that runs as the same user can read the database.
4. The operator of an instance can read the data of all users of that instance.
   Users who do not trust each other must use separate instances (ADR-0015).
5. The executables are not signed. A user must compare the checksum to detect a manipulated file (ADR-0018).
