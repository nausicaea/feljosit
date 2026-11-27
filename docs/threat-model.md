# Threat model

This document uses the STRIDE method. It describes the deployment models, the
persons, the assets, the trust boundaries, the threats and the controls.

## Deployment models

Feljosit has two deployment models (ADR-0023):

| Model | Operator | Users | Network |
| --- | --- | --- | --- |
| Personal computer | The user | The persons who use the computer | Only `127.0.0.1` |
| Family server | One trusted family member | 1 to 10 family members | A private home network or a private VPN, through a TLS reverse proxy |

Out of scope: Feljosit on the open internet, and an operator that the users do
not trust. The controls of this document are not sufficient for these cases.

## Persons

| Person | Trust |
| --- | --- |
| Operator | Trusted by all users. Controls the server, the code, the database and the backups. Can read all data (ADR-0015). |
| User | A family member. Trusted only with their own data. Must not see the data of a different user. |
| Device on the home network | Not trusted. For example a guest device, a smart TV or a compromised phone. |
| Other web pages in the browser of a user | Not trusted. |
| Internet | Not trusted. Must not be able to connect to the server. |

## Assets

1. Transactions, balances and budgets of the users (very sensitive).
2. Passwords and sessions.
3. The availability of the application.

## Trust boundaries

1. The browser and the server. Other web pages in the same browser are not trusted.
2. The home network and the server (family server only). The TLS reverse proxy is
   the only entry.
3. One user and a different user of the same instance.
4. Other processes and users on the same computer or server.
5. Uploaded files. A bank statement can be manipulated before the upload.
6. The dependencies (crates.io, GitHub Actions, base images) and the published
   executables and container images.

## Threats and controls

| Threat (STRIDE) | Example | Controls |
| --- | --- | --- |
| Spoofing | Password guessing from a device on the home network | Argon2id, minimum length 12. A lock for 15 minutes after 5 failures from one client, and after 50 failures from all clients (ADR-0024). No access from the internet (operator responsibility, ADR-0023). |
| Spoofing | The operator knows the temporary password of a new user | Passwords from the command line are temporary. The user must change the password before other pages open (ADR-0024). |
| Spoofing | A device sends a false `X-Forwarded-For` header to avoid the limits | Feljosit reads the header only from `FELJOSIT_TRUSTED_PROXIES`, from the right. The values that the device adds to the left are ignored (ADR-0024). |
| Spoofing | Session theft | Random tokens, only the hash in the database, `HttpOnly`, `SameSite=Strict`, expiry after 12 hours. On a family server: TLS, `Secure` and HSTS (`FELJOSIT_SECURE_COOKIES=true`). |
| Spoofing | A device on the home network reads the password or the session cookie | TLS reverse proxy. Feljosit listens only on `127.0.0.1` behind the proxy. The application shows a warning for a different address. |
| Spoofing | Somebody else adds the first user | Only while the database has no user, only from the same computer (ADR-0019). Behind a reverse proxy, only the operator can add users (command line). |
| Tampering | Cross-site request forgery from a different web page | CSRF tokens, Origin check, `SameSite=Strict`. |
| Tampering | DNS rebinding to `127.0.0.1` or to the server | `Host` header allow list (`FELJOSIT_ALLOWED_HOSTS`). |
| Tampering | SQL injection | `rusqlite` with bound parameters. `LIKE` patterns are escaped. |
| Repudiation | A user denies an import | Each import is recorded with the file hash and the time. |
| Information disclosure | Cross-site scripting | Template autoescape (minijinja), strict Content Security Policy, no inline scripts. |
| Information disclosure | A user reads the data of a different user | Each query filters by `user_id`. An object of a different user gives HTTP 404. Tests check this. |
| Information disclosure | XXE in a camt.053 file | The parser (`roxmltree`) refuses DTDs. Thus entities and external references are not possible (ADR-0018). |
| Information disclosure | Other local users read the database | File mode `0600`, directory mode `0700`, `umask 077` (Linux, macOS). On Windows, the profile directory of the user. In the container, the user `nonroot` owns the volume. |
| Information disclosure | A stolen laptop or server | Full-disk encryption (operator responsibility, see ADR-0003). |
| Information disclosure | A copy of the database file (for example a backup) | Encrypted backups (operator responsibility, see SECURITY.md and ADR-0015). |
| Information disclosure | The operator reads the data of all users | Not prevented (ADR-0015). The users must trust the operator. A user who does not trust the operator uses a personal computer instance. |
| Information disclosure | Error messages show data | Validation errors do not show the input. No debug mode. |
| Denial of service | A device on the home network locks the account of a user with 5 wrong passwords | The lock is only for that device. The lock for all clients needs 50 failures (ADR-0024). The lock stops after 15 minutes. The operator can unlock with `feljosit set-password`. |
| Denial of service | A device on the home network opens many slow connections | 10 seconds for the headers, 120 seconds for the complete request, a maximum of 16 connections for each client and 64 for all clients (ADR-0024). The TLS reverse proxy receives the connections first. |
| Denial of service | "Billion laughs" or ZIP bomb | Entities refused, ZIP limits (10 MB for each file, 25 MB in total), upload limit of 10 MB (ADR-0014). |
| Denial of service | A credit card CSV file with very many lines or very long fields | Limits of 100,000 lines and 1,000 characters for each field, upload limit of 10 MB (ADR-0017). |
| Denial of service | A very large or deep XML tree, or very many attributes | A pre-scan applies the limits for the number of elements, the nesting depth and the attributes before the parse (ADR-0014, ADR-0018). |
| Denial of service | An amount such as `1E999999` uses the CPU | Strict amount syntax before the conversion (ADR-0014). |
| Denial of service | A long import stops all requests | The requests use the database one after the other (ADR-0018). The limits of ADR-0014 keep an import short. |
| Denial of service | ReDoS in categorisation rules | Rules use text comparison, not regular expressions. |
| Elevation of privilege | Open redirect | Redirect targets must be local paths. |
| Elevation of privilege | A vulnerability in Feljosit gives access to the server | In the container: no shell, the user `nonroot`, a minimal base image (ADR-0022). |
| Supply chain | A compromised dependency | Exact versions in `Cargo.toml` and `Cargo.lock` (with checksums), pinned action SHAs and image digests, Renovate with 7-day delay (ADR-0012), `cargo audit`, CodeQL. |
| Supply chain | A manipulated executable | The release workflow builds the files on GitHub. The release has the file `SHA256SUMS`. The executables are not signed (ADR-0018). |
| Supply chain | A manipulated container image | The image is signed with cosign (keyless) and has a provenance and an SBOM. The operator verifies the signature (ADR-0022). |

## Residual risks

1. The database is not encrypted by the application (see ADR-0003 and ADR-0015).
2. There is no two-factor authentication (see ADR-0007). On a family server, a
   device on the home network can guess passwords. The lock after 5 failures makes
   this slow. This is a reason that Feljosit must not be on the open internet (ADR-0023).
3. An attacker with 10 or more IPv4 addresses (or IPv6 /64 networks) can lock a user
   for all clients for 15 minutes (ADR-0024).
4. Without a correct `FELJOSIT_TRUSTED_PROXIES`, all devices behind the reverse
   proxy have the same client key. Then one device can lock a user for all devices,
   and the limit of 16 connections is for the proxy only. If the setting includes
   untrusted devices, they can send a false address (ADR-0024).
5. Four or more devices together can use all 64 connections. A reverse proxy with
   connection limits decreases this risk.
6. Malware that runs as the same user (or on the server) can read the database.
7. The operator of an instance can read the data of all users of that instance.
   Users who do not trust the operator must use a separate instance (ADR-0015).
8. The executables are not signed. A user must compare the checksum to detect a
   manipulated file (ADR-0018). The container image is signed (ADR-0022).
