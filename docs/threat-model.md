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
| Information disclosure | XXE in a camt.053 file | `defusedxml` refuses DTDs and entities. |
| Information disclosure | Other local users read the database | File mode `0600`, directory mode `0700`, `umask 077`. |
| Information disclosure | A stolen laptop | Full-disk encryption (operator responsibility, see ADR-0003). |
| Information disclosure | Error messages show data | Validation errors do not show the input. No debug mode. |
| Denial of service | "Billion laughs" or ZIP bomb | Entities refused, ZIP limits, upload limit of 10 MB. |
| Denial of service | ReDoS in categorisation rules | Rules use text comparison, not regular expressions. |
| Elevation of privilege | Open redirect | Redirect targets must be local paths. |
| Supply chain | A compromised dependency | `uv.lock`, pinned action SHAs, Renovate with 3-day delay, `pip-audit`, CodeQL. |

## Residual risks

1. The database is not encrypted by the application (see ADR-0003).
2. There is no two-factor authentication (see ADR-0007).
3. Malware that runs as the same user can read the database.
