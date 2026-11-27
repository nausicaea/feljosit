# Security policy

## Report a vulnerability

Do not open a public issue for a vulnerability.
Use the "Report a vulnerability" function of GitHub (private vulnerability reporting).
We reply in 7 days.

## Supported versions

Only the latest release gets security fixes.

## Security design

Read the [threat model](docs/threat-model.md) and
[ADR-0007](docs/adr/0007-authentication-and-sessions.md).
A summary:

- Two deployment models: the personal computer of the user, or a private family
  server with an operator (ADR-0023). Feljosit is not designed for the open internet.
- By default, the server listens only on `127.0.0.1`. On a family server, a TLS
  reverse proxy is the only entry.
- Passwords are hashed with Argon2id. Accounts are locked after 5 failed logins.
- Sessions are kept on the server. The cookie is `HttpOnly` and `SameSite=Strict`.
- All forms have CSRF tokens. The server checks the `Origin` and the `Host` headers.
- A strict Content Security Policy permits no inline scripts and no external sources.
- The XML parser refuses DTDs and entities (XXE and "billion laughs" protection).
- The XML parser limits the number of elements, the nesting depth and the attributes.
- Amounts in a statement must have a strict decimal syntax. Error messages do not show values from the file.
- ZIP files and uploads have size limits. The requests use the database one after the other.
- The database file has mode `0600` (Linux and macOS).
- The first user can be added in the browser only from the same computer, and only while the database has no user (ADR-0019).

## Recommendations for operators

For a family server, read the [operator guide](docs/operator-guide.md).

1. Use full-disk encryption on the computer or the server.
2. Keep the default address `127.0.0.1`.
3. If other devices must connect, put a TLS reverse proxy in front of the server.
   Then set `FELJOSIT_SECURE_COOKIES=true` and `FELJOSIT_ALLOWED_HOSTS`.
4. Do not make the server accessible from the internet. Use a VPN for access from
   outside the home.
5. Encrypt the backups.
6. Update the application regularly.
7. Compare the checksum of a downloaded file with the file `SHA256SUMS` of the release.
8. Verify the signature of the container image with `cosign verify` (see [README.md](README.md)).
   In a container, publish the port only on `127.0.0.1` of the host.
9. Tell each user that you can read all data. Give each new user a temporary
   password, and tell the user to change it at the first login.

## Code scanning results in the tests

CodeQL (`security-extended`) also examines the files in `tests/`. These results are
known and are not vulnerabilities. Dismiss them with the given reason:

| Rule | Location | Dismissal reason | Explanation |
| --- | --- | --- | --- |
| `rust/hard-coded-cryptographic-value` | `tests/users.rs`, `tests/importer.rs`, `tests/common/mod.rs` (the constant `PASSWORD`) | Used in tests | The tests must use known passwords: they hash a password and verify it, and `tests/users.rs` verifies a hash of version 0.3.0 with its known password. These passwords protect no data. The application has no hard-coded password, salt or key: the salts come from `getrandom`. |
| `rust/cleartext-storage-database` | `tests/common/mod.rs` (the function `scalar`) | False positive | The data is the SQL text `SELECT password_hash FROM users` of `tests/users.rs`. The test reads the Argon2id hash. It does not write a password to the database. |

If a result is in `src/`, examine it. Do not dismiss it without a reason in the
dismissal comment.
