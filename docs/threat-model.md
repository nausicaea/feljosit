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
4. The dependencies (PyPI, GitHub Actions).

## Threats and controls

| Threat (STRIDE) | Example | Controls |
| --- | --- | --- |
| Spoofing | Password guessing | Argon2id, lockout after 5 failures, minimum length 12. |
| Spoofing | Session theft | Random tokens, only the hash in the database, `HttpOnly`, `SameSite=Strict`, expiry after 12 hours. |
| Tampering | Cross-site request forgery from a different web page | CSRF tokens, Origin check, `SameSite=Strict`. |
| Tampering | DNS rebinding to `127.0.0.1` | `Host` header allow list. |
| Tampering | SQL injection | SQLAlchemy with bound parameters. `LIKE` patterns are escaped. |
| Repudiation | A user denies an import | Each import is recorded with the file hash and the time. |
| Information disclosure | Cross-site scripting | Jinja autoescape, strict Content Security Policy, no inline scripts. |
| Information disclosure | A user reads the data of a different user | Each query filters by `user_id`. Tests check this. |
| Information disclosure | XXE in a camt.053 file | `defusedxml` refuses DTDs, entities and external references. |
| Information disclosure | Other local users read the database | File mode `0600`, directory mode `0700`, `umask 077`. |
| Information disclosure | A stolen laptop | Full-disk encryption (operator responsibility, see ADR-0003). |
| Information disclosure | A copy of the database file (for example a backup) | Encrypted backups (operator responsibility, see SECURITY.md and ADR-0015). |
| Information disclosure | The operator reads the data of all users | Not prevented (ADR-0015). Users who do not trust each other use separate instances. |
| Information disclosure | Error messages show data | Validation errors do not show the input. No debug mode. |
| Denial of service | "Billion laughs" or ZIP bomb | Entities refused, ZIP limits (10 MB for each file, 25 MB in total), upload limit of 10 MB (ADR-0014). |
| Denial of service | A credit card CSV file with very many lines or very long fields | Limits of 100,000 lines and 1,000 characters for each field, upload limit of 10 MB (ADR-0017). |
| Denial of service | A very large or deep XML tree | Limits for the number of elements, the nesting depth and the attributes (ADR-0014). |
| Denial of service | An amount such as `1E999999` uses the CPU | Strict amount syntax before `Decimal()` reads the value (ADR-0014). |
| Denial of service | A long import stops all requests | The import runs in a worker thread. Only one import runs at a time (ADR-0014). |
| Denial of service | A libexpat vulnerability | A warning at start if libexpat is older than 2.7.2. A test checks the CI Python. |
| Denial of service | ReDoS in categorisation rules | Rules use text comparison, not regular expressions. |
| Elevation of privilege | Open redirect | Redirect targets must be local paths. |
| Supply chain | A compromised dependency | Exact versions in `pyproject.toml` and `uv.lock`, pinned action SHAs, Renovate with 7-day delay (ADR-0012), `pip-audit`, CodeQL. |

## Residual risks

1. The database is not encrypted by the application (see ADR-0003 and ADR-0015).
2. There is no two-factor authentication (see ADR-0007).
3. Malware that runs as the same user can read the database.
4. The operator of an instance can read the data of all users of that instance.
   Users who do not trust each other must use separate instances (ADR-0015).
5. An XML element with very many attributes can use approximately 280 MB of memory before the parser refuses it (ADR-0014).
