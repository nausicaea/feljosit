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

- The server listens only on `127.0.0.1`.
- Passwords are hashed with Argon2id. Accounts are locked after 5 failed logins.
- Sessions are kept on the server. The cookie is `HttpOnly` and `SameSite=Strict`.
- All forms have CSRF tokens. The server checks the `Origin` and the `Host` headers.
- A strict Content Security Policy permits no inline scripts and no external sources.
- The XML parser refuses DTDs and entities (XXE and "billion laughs" protection).
- ZIP files and uploads have size limits.
- The database file has mode `0600`.

## Recommendations for operators

1. Use full-disk encryption on the computer.
2. Keep the default address `127.0.0.1`.
3. If other computers must connect, put a TLS reverse proxy in front of the server.
   Then set `FELJOSIT_SECURE_COOKIES=true` and `FELJOSIT_ALLOWED_HOSTS`.
4. Encrypt the backups.
5. Update the application regularly.
