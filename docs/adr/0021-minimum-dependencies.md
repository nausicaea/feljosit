# ADR-0021: Use the minimum number of external libraries

- Status: Accepted
- Date: 2026-11-26
- Supersedes: the HTTP server (`axum` on `tokio`) and the CSV library (`csv`) of ADR-0018
- Related: ADR-0002, ADR-0012, ADR-0018

## Context

Each external library is code that we do not review, but that runs with the data
of the user. Each library can get a vulnerability, become unmaintained (see
ADR-0020) or be compromised (supply chain attack). Each library also makes
Renovate pull requests (ADR-0012).

Version 0.4.0 has 21 direct libraries and one test library. `Cargo.lock` has 135
packages. Many libraries do small tasks:

| Library | Task in Feljosit |
| --- | --- |
| `axum`, `tokio` (with `hyper`, `tower`, `http` and others) | Receive a request and send the response. `App::handle` is already synchronous |
| `open` | Open the browser |
| `rpassword` | Read a password without an echo |
| `dirs` | Find the data directory |
| `csv` | Read the fields of the card CSV file. The parser already checks the syntax itself |
| `minijinja-contrib` | Three Python methods in the templates (`items`, `startswith`, `get`) |
| `serde_json` | Two JSON responses with one text value |
| `form_urlencoded`, `hex`, `base64` | Small text encodings |
| `tempfile` (tests) | Temporary directories |

## Options

1. Keep all libraries.
2. Keep only the libraries for difficult or security-critical tasks. Replace the
   others with the standard library and small functions with tests.

## Decision

We use option 2. These libraries stay:

| Library | Reason |
| --- | --- |
| `argon2`, `sha2`, `getrandom` | Cryptography. Do not write it yourself |
| `rusqlite` (with SQLite) | Database |
| `minijinja`, `serde` | Templates (Jinja2 syntax) and the data for the templates |
| `roxmltree` | XML parser with protection against DTDs and entities (ADR-0014) |
| `zip` | ZIP archives with Deflate |
| `jiff` | Dates, time zones (ADR-0020) |
| `libc` (Unix) | `umask` and the terminal settings. `getrandom` and `zip` also use it |

The replacements:

- **HTTP server** (`src/web/server.rs`): HTTP/1.1 with `std::net`. One thread for
  each connection, at most 64 connections, one request for each connection
  (`connection: close`), time limits for read and write, a limit of 64 KiB and
  100 lines for the headers, and the body limit of the application. The server
  refuses `Transfer-Encoding` (status 501). Browsers send forms with
  `Content-Length`, and this prevents request smuggling through a reverse proxy.
  The server removes header values with line ends. A panic in a handler gives
  status 500.
- **Browser** (`src/cli.rs`): the server starts `xdg-open`, `open` (macOS) or
  `rundll32.exe url.dll,FileProtocolHandler` (Windows) directly, without a shell.
- **Password** (`src/terminal.rs`): `termios` on Unix, `GetConsoleMode` and
  `SetConsoleMode` of `kernel32.dll` on Windows.
- **Data directory** (`src/config.rs`): `XDG_DATA_HOME` or `~/.local/share` on
  Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows. Only
  absolute paths are used.
- **CSV** (`src/cardcsv.rs`): the state machine of the syntax check also returns
  the fields.
- **Templates**: `x|items`, `path is startingwith('/x')` and `map[key]` of
  `minijinja`.
- **Encodings** (`src/encoding.rs`): hex, Base64 (with the test vectors of
  RFC 4648), form data and JSON strings.
- **Tests**: a `TempDir` in `tests/common/mod.rs`.

## Consequences

- Good: 10 direct libraries and no test library. `Cargo.lock` has 80 packages, not 135.
- Good: the release executable for Linux is 5.2 MB, not 6.0 MB.
- Good: fewer `cargo audit` warnings and fewer Renovate pull requests.
- Bad: we maintain approximately 600 more lines of code. The tests cover them.
- Bad: the HTTP server is simple. It has no keep-alive, no HTTP/2 and no chunked
  request bodies. For one person on the same computer this is sufficient
  (ADR-0002). For other computers, a TLS reverse proxy stays necessary.
- Bad: Ctrl+C stops the process immediately. The server does not wait for open
  requests. SQLite keeps the database consistent (each change is a transaction).
- Rule: before you add a library, write in the pull request why the standard library
  or a small function is not sufficient.
