# ADR-0023: Let an operator host Feljosit for a family on a private server

- Status: Proposed
- Date: 2026-11-27
- Supersedes: the deployment model of ADR-0002 ("the data stays on the computer of the user")
- Related: ADR-0007, ADR-0015, ADR-0019, ADR-0021, ADR-0022

## Context

ADR-0002 put the server on the computer of the user. Thus the user was also the
operator. Each family member had to start the application on their own computer,
or the family used one computer together.

With the container image (ADR-0022), one person can operate Feljosit on a home
server or a NAS. The other family members use a browser on their own devices. They
do not have to install or start software.

Then the user and the operator are different persons. The server is accessible
from the home network, not only from `127.0.0.1`. The threat model must include
the other devices on the home network.

Feljosit is not designed for the open internet. It has no two-factor
authentication (ADR-0007), and its HTTP server is small (ADR-0021).

## Options

1. Keep the requirement that the user is the operator.
2. Support two deployment models: the personal computer of the user, and a
   private family server with an operator.
3. Make Feljosit a public internet service.

## Decision

We use option 2.

The deployment models:

| Model | Operator | Users | Network |
| --- | --- | --- | --- |
| Personal computer | The user | 1 (or the persons who use the computer) | Only `127.0.0.1` |
| Family server | One trusted family member | 1 to 10 family members | A private home network or a private VPN, through a TLS reverse proxy |

The roles on a family server:

- The **operator** installs and updates Feljosit, adds the users, sets new
  passwords, and makes the backups. The operator can read the data of all users
  (ADR-0015). All users must trust the operator.
- A **user** is a family member. A user sees only their own data. The users can
  have private data in front of each other. The access control of the application
  keeps the data of the users apart (each query filters by `user_id`).

The rules for a family server:

- The server is not accessible from the internet. No port forwarding on the router.
  For access from outside the home, use a VPN (for example WireGuard).
- A TLS reverse proxy is in front of Feljosit. Feljosit listens only on
  `127.0.0.1` (or the container port is published only on `127.0.0.1`).
- The operator sets `FELJOSIT_SECURE_COOKIES=true` and `FELJOSIT_ALLOWED_HOSTS`.
- The operator adds each user with the command line and gives a temporary
  password. The user changes the password at the first login. The application
  does not force this change. Thus the operator must tell the user.
- The operator uses full-disk encryption on the server and encrypted backups.
- [The operator guide](../operator-guide.md) gives the procedures.

The threat model (docs/threat-model.md) describes the threats of both models.

## Consequences

- Good: The family members need only a browser. Phones and tablets can also connect.
- Good: The data stays in the home of the family. No cloud service gets the data.
- Good: One person makes the updates and the backups for all users.
- Bad: More devices can connect to the server. A device on the home network can
  guess passwords, lock accounts (5 failed logins) or open many slow connections.
  The reverse proxy and the lockout decrease these risks, but do not remove them.
- Bad: There is still no two-factor authentication. ADR-0007 accepted this for
  `localhost`. For a family server, the private network, TLS and the lockout
  compensate. This is the reason that Feljosit must not be on the open internet.
- Bad: The operator can read all data. A family member who does not trust the
  operator must use a personal computer instance.
- Bad: The first user cannot be added in the browser behind a reverse proxy
  (ADR-0019). The operator uses the command line.
