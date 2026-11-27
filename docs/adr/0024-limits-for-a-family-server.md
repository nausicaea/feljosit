# ADR-0024: Limit failed logins and connections for each client, and force a change of temporary passwords

- Status: Proposed
- Date: 2026-11-27
- Supersedes: the lockout rule of ADR-0007 ("after 5 failed logins, the account is locked")
- Related: ADR-0007, ADR-0021, ADR-0023

## Context

ADR-0023 lets an operator host Feljosit for a family. Then other devices on the
home network can connect. The threat model had three residual risks for this model:

1. The lockout was for each username. One device could keep the account of a user
   locked with 5 wrong passwords in each 15 minutes.
2. The operator gave a temporary password, but the application did not force the
   user to change it. Thus the operator could know the password of a user.
3. The server had a limit of 64 connections and a time limit of 30 seconds for each
   read. A device could keep all connections open with one byte in each 30 seconds.

Behind a reverse proxy, all connections come from the address of the proxy. The
proxy gives the address of the client in the header `X-Forwarded-For`. A client can
also send this header with a false value.

## Options

For the lockout:

1. Keep the lockout for each username.
2. Lock each pair of user and client. Do not lock the user for all clients.
3. Lock each pair of user and client. Also lock the user for all clients after
   many more failures.

For the temporary passwords:

1. Tell the user to change the password (documentation only).
2. Mark passwords from the command line as temporary. Open no other page until the
   user changes the password.

For the slow connections:

1. Recommend only the limits of the reverse proxy.
2. Add a time limit for the complete request and a limit of connections for each client.

## Decision

Client address:

- The new setting `FELJOSIT_TRUSTED_PROXIES` is a list of addresses and networks
  (for example `172.30.87.1` or `fd00::/8`). The default is empty.
- Only if the connection comes from a trusted proxy, Feljosit reads
  `X-Forwarded-For`. It reads the values from the right and ignores the addresses
  of trusted proxies. The first other address is the client. The values to the left
  of it are not trusted.
- The client key is the IPv4 address, or the /64 network of an IPv6 address. A
  device can easily use many addresses in its IPv6 network.
- The log shows the client address, not the address of the proxy.

Lockout (option 3):

- After 5 failed logins from one client, the user is locked for 15 minutes for that
  client only. The new table `login_failures` keeps the counters (migration 0003).
- After 50 failed logins from all clients together, the user is locked for 15
  minutes for all clients. This stops an attacker with many addresses. One device
  cannot reach this limit, because it is locked after 5 failures.
- A successful login resets the counter of the client and the counter of all clients.
- The counters of clients without a lock are deleted after 24 hours.
- `feljosit set-password` removes all locks of the user.
- The response does not show a lock. It is the same as for a wrong password.

Temporary passwords (option 2):

- The new column `users.must_change_password` marks a temporary password.
- `feljosit create-user` and `feljosit set-password` make a temporary password.
  The option `--permanent` makes a password that is not temporary (for a person who
  adds their own user on their own computer).
- The first user in the browser (ADR-0019) chooses the password. It is not temporary.
- With a temporary password, the user can only open the settings page, change the
  password and log out. Each other page goes to the settings page.
- The new password must be different from the current password.

Slow connections (option 2):

- The request line and the headers must arrive in 10 seconds.
- The complete request must arrive in 120 seconds. An upload of 10 MB then needs
  approximately 85 kB/s.
- A late request gets "408 Request Timeout".
- One client can have a maximum of 16 connections at the same time. For a direct
  connection, the server counts the connection immediately. Behind a trusted proxy,
  the server counts it after the headers, before the body. A browser uses a
  maximum of approximately 6 connections for each server.
- The limit of 64 connections for all clients stays.

## Consequences

- Good: One device on the home network cannot lock a user for the other devices.
- Good: The operator does not know the password that a user uses.
- Good: A slow device cannot keep connections open for a long time.
- Good: The operator sees the address of each device in the log.
- Bad: The operator must set `FELJOSIT_TRUSTED_PROXIES` correctly. Without it,
  all clients behind the proxy have the same key. Then the lockout is again for
  each username (as before), and the connection limit is for the proxy only.
- Bad: With a wrong `FELJOSIT_TRUSTED_PROXIES` (for example a network with untrusted
  devices), a device can send a false address and avoid the limit for one client.
  The limit of 50 failures for all clients still applies.
- Bad: An attacker with 10 or more IPv4 addresses (or /64 networks) can still lock a
  user for all clients.
- Bad: The schema revision 0003 is new. Feljosit 0.3.0 (Python) cannot open the
  database after the upgrade. Make a backup before the update.
