# ADR-0019: Add the first user in the browser

- Status: Accepted
- Date: 2026-11-26
- Related: ADR-0007, ADR-0018

## Context

ADR-0007 says that an administrator adds users with the command line
(`feljosit create-user`). With ADR-0018, a user can start the application with a
double-click. Then there is no terminal. On Windows, many users never use a
terminal. Without a user, nobody can log in.

## Options

1. Keep the command line as the only method.
2. Let each visitor register in the browser.
3. Let the browser add the first user only, while the database has no user.

## Decision

We use option 3.

- If the database has no user, the login page goes to the page `/setup`.
- The page `/setup` adds one user with the same rules as the command line
  (username, password of 12 or more characters, Argon2id).
- The page works only while the database has no user. When a user exists, it
  goes to the login page. The check and the new user are in one database
  transaction.
- The page works only for a client on the same computer: the connection must come
  from a loopback address, and the `Host` header must be a loopback name
  (`127.0.0.1`, `localhost` or `::1`). A reverse proxy sends the public host name.
  Thus behind a reverse proxy, the administrator uses the command line.
- The form has a CSRF token in a cookie and in the form (double submit), as the
  login form. The server also checks the `Origin` header.
- After the setup, the browser has a session of the new user.

More users and forgotten passwords still need the command line.

## Consequences

- Good: A user can start Feljosit without a terminal.
- Good: Public registration is not possible.
- Bad: Between the first start and the setup, another program on the same
  computer can add the first user. Then the real user sees the login page and
  knows that something is wrong. The risk is small, because the attacker must
  already run code on the computer.
