# ADR-0007: Use local accounts and server-side sessions

- Status: Accepted (the first user: see ADR-0019; the risk without two-factor authentication on a family server: see ADR-0023)
- Date: 2026-11-25

## Context

Up to 10 persons can use one installation. Each person must see only their own
data. The application runs on `localhost`, but other programs on the computer
and web pages in the browser can send requests to it.

## Decision

Authentication:

- An administrator adds users with the CLI (`feljosit create-user`). There is no
  public registration.
- Passwords have a minimum of 12 characters. We hash them with Argon2id
  (64 MiB, 3 iterations, 4 lanes, as RFC 9106 recommends).
- After 5 failed logins, the account is locked for 15 minutes.
- The login always calculates one hash. Thus the response time does not show if
  a username exists.

Sessions:

- The session token is 256 random bits. The database keeps only the SHA-256 hash.
- The cookie has the attributes `HttpOnly` and `SameSite=Strict`. With TLS, it
  also has `Secure`.
- A session expires after 12 hours. A logout deletes the session.
  A password change deletes all sessions of the user.

Request protection:

- Each form has a CSRF token (synchronizer token). The login form uses a
  double-submit cookie.
- The server compares the `Origin` header with the `Host` header.
- The server accepts only known `Host` headers. This prevents DNS rebinding.
- The `Referrer-Policy` is `same-origin`. The value `no-referrer` makes browsers
  send `Origin: null`, and then the Origin check fails.
- The Content Security Policy permits only scripts and styles from the server.
- The server limits the request body size.
- The API documentation (`/docs`, `/openapi.json`) is disabled.

Authorisation:

- Each table has a `user_id` column. Each query filters by the user.
- An object of a different user gives HTTP 404, not 403.

## Consequences

- Good: The protection is in the application, not only in the network.
- Good: Tests check each control (see `tests/test_web.py`).
- Bad: There is no shared budget for two users. A household can use one user.
- Bad: There is no two-factor authentication. Because the server listens only on
  `localhost`, we accept this risk.
