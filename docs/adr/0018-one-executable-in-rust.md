# ADR-0018: Make Feljosit one executable file in Rust

- Status: Accepted
- Date: 2026-11-26
- Supersedes: the stack of ADR-0002, ADR-0013, the parser library of ADR-0005 and ADR-0014
- Related: ADR-0003, ADR-0007, ADR-0012

## Context

Version 0.3.0 is a Python application. To start it, a user must install Python
3.13 and `uv`, get the source code, and run commands in a terminal. Many
users of a household budget cannot do this. They know how to copy a file and
how to open it with a double-click.

The application must run on Windows, Linux and macOS without an installation.
Copying one file must be sufficient. If the project uses a compiled language,
the maintainer wants Rust, Java or Kotlin, not Go.

The design of ADR-0002 stays correct: a small web server on the local computer,
HTML from the server, SQLite. Only the packaging and the language are the
problem.

## Options

1. Keep Python and package it with a freezer (PyInstaller, Nuitka or
   PyApp). The freezer puts the interpreter and the packages in one file.
2. Rewrite in Java or Kotlin, and supply a JAR file or a `jlink` image.
3. Rewrite in Rust, and supply one static executable for each platform.

### 1. A Python freezer

- Good: No rewrite. The tests stay.
- Bad: A frozen file unpacks itself to a temporary directory at each start.
  The start takes seconds. Antivirus programs often block such files.
- Bad: The file is large (40 to 80 MB) and contains a copy of OpenSSL, libexpat
  and other C libraries. Updates of these libraries need a new build.
- Bad: The build must run on each target platform, and the freezers often
  break with new Python versions.

### 2. Java or Kotlin

- Good: Mature libraries for HTTP, SQLite (JDBC) and XML.
- Bad: A JAR file needs an installed Java runtime. This is an installation.
- Bad: A `jlink` image or a GraalVM native image removes this need, but it is a
  directory or a large file (50 MB and more). Native images of applications
  that use reflection (JDBC, template engines) need much configuration.
- Bad: The JVM uses more memory than necessary for 1 to 10 users.

### 3. Rust

- Good: The compiler makes one executable file (approximately 6 MB) that needs
  no runtime. On Linux, the file is static (musl) and runs on all distributions.
  On Windows, the C runtime is in the file.
- Good: The templates, the CSS, the JavaScript and the SQL migrations are in the
  file. SQLite is compiled into the file (`rusqlite` with `bundled`).
- Good: The start takes less than one second. The memory use is small.
- Good: The type system and the borrow checker prevent many errors, for example
  data races between requests.
- Bad: A rewrite of approximately 6,000 lines takes time and can add errors.
- Bad: Contributors must know Rust.

## Decision

We use option 3: Rust. These libraries replace the Python libraries:

| Task | Python (0.3.0) | Rust |
| --- | --- | --- |
| HTTP server | FastAPI, Uvicorn | `axum` on `tokio` |
| Templates | Jinja2 | `minijinja` (a Jinja2 implementation) |
| Database | SQLAlchemy, Alembic | `rusqlite` with SQLite in the executable |
| Passwords | `argon2-cffi` | `argon2` |
| XML | `defusedxml` (expat) | `roxmltree` |
| ZIP, CSV | standard library | `zip`, `csv` |
| Dates | standard library | `chrono` |

These rules keep the change safe for the users:

- **The same database.** The SQL migrations make the same schema as the Alembic
  migrations of 0.3.0. The table `alembic_version` keeps the revision. Thus the
  Rust version opens a database of the Python version without a change. The
  values have the same formats (dates as `YYYY-MM-DD`, times in UTC as
  `YYYY-MM-DD HH:MM:SS.ffffff`, booleans as 0 and 1).
- **The same passwords.** The Argon2id hashes have the same PHC format and the
  same parameters. Users log in with their old password.
- **The same fingerprints.** The fingerprints of the transactions (ADR-0006) use
  the same algorithm. Thus an import of an old statement does not add duplicates.
- **The same behaviour.** The routes, the forms, the HTML, the security headers
  and the messages are the same. The tests of the Python version are ported to
  Rust. A test opens a database that the Python version made.

### The template engine

Version 0.3.0 has 16 Jinja2 templates with approximately 780 template tags.
The templates use template inheritance (`extends` and `block`), macros that read
the variables of the page (`with context`), calculations (`chart.left - 6`),
comparisons, filters with arguments and `for … if …` loops. Many tests compare
the HTML exactly.

We compared three template engines on 2026-11-26:

1. `minijinja` 3.1.0: a Jinja2 implementation. It reads the templates when the
   application starts and checks the values when it makes a page.
2. `askama` 0.16.1: Jinja-like templates. A macro converts each template to Rust
   code when the project compiles. Each page has a Rust structure with its values.
3. `handlebars`: logic-less templates (Mustache syntax).

To compare `minijinja` and `askama` with real data, we converted the budget page
(`base.html`, `macros.html`, `budget.html`) to both.

| Criterion | `minijinja` | `askama` |
| --- | --- | --- |
| Changes to the templates | 39 changed lines in 13 of the 16 templates, for example `'%.1f' % x` becomes `x \| fixed(1)` | About 10 changes in the 3 templates of the budget page alone. Estimate for all templates: 1 to 2 days |
| Template syntax | Jinja2. Imports, macros, `set`, inline `if … else` and the tests `is none` and `is true` are the same | Different: `{% import … as m %}` and `{% call m::csrf(token) %}`, `let` instead of `set`, Rust expressions (`if let Some(x) = …`, `flash.0`, `starts_with`), no inline `if … else` with single quotes |
| Macros | Read the variables of the page | Get each value as an argument |
| Values of a page | Dynamic values. The handlers give structures (with `serde`) or maps | One Rust structure for each page. The common values (user, CSRF token, message) must be in each structure |
| Errors | When the page is made. Example: `.items()` on a list gave HTTP 500 until a test opened the page | When the project compiles. Example: `{% if count %}` with a number is refused, because it is not a boolean |
| Templates in the executable | `include_str!` and a list in `render.rs` | Automatic |
| Speed | Fast enough. A page takes less than 1 ms | Faster, but the difference has no effect for 1 to 10 users |
| Maintainer | Armin Ronacher (the author of Jinja2) | The `askama-rs` organisation |

Handlebars does not have template inheritance with blocks, calculations or
comparisons. Each of these needs a helper in Rust, or the handlers must calculate
more values. Thus a change to Handlebars is a complete rewrite of the templates.
We did not continue with Handlebars.

We use `minijinja` (option 1). The conversion from Jinja2 is small and safe. The
macros and the syntax stay the same, and the HTML stays the same. `askama` gives
the better checks, but the conversion needs 1 to 2 days and many new Rust
structures. A conversion during the rewrite to Rust adds a second large risk.

We decrease the risk of errors that `minijinja` shows only when it makes a page:

- A unit test compiles all templates (`render.rs`).
- The tests open each page, also with the optional parts (import result, report
  details, transfer partners, errors).

We will examine `askama` again when the templates change much, or if template
errors that the tests do not find get into a release. The templates stay close
to Jinja2. Thus a later conversion stays possible.

The layers stay the same as in ADR-0002:

- `camt053`, `cardcsv`, `money`, `iban`: functions without a database.
- `services`: business logic. The web layer and the command line use it.
- `web`: routes, templates, sessions and security headers.

The web layer has one database connection behind a mutex. Each request runs in
one database transaction on a worker thread. Thus the requests run one after
the other. This is sufficient for 1 to 10 users, and it replaces the import lock
of ADR-0014.

The executable starts the server and opens the browser when the user starts it
without arguments, for example with a double-click. The data is in the data
directory of the operating system:

- Linux: `~/.local/share/feljosit` (or `$XDG_DATA_HOME/feljosit`), the same as 0.3.0.
- macOS: `~/Library/Application Support/feljosit`.
- Windows: `%APPDATA%\feljosit`.

`FELJOSIT_DATA_DIR` can change the directory. Version 0.3.0 used
`~/.local/share/feljosit` on all systems. If the new directory has no database
but that directory has one, the application uses that directory.

The XML parser `roxmltree` refuses DTDs. Thus entities and external references
are not possible (ADR-0005). It does not limit the attributes of one element, and
it builds the complete tree. Thus a pre-scan reads the text one time before the
parse and applies the limits of ADR-0014 (1,000,000 elements, a nesting depth of
64, 16 attributes). A test with 10 MB files shows: the pre-scan refuses each
manipulated file in less than 0.1 seconds with less than 25 MB of memory. The
parser also reads UTF-16 and ISO-8859-1, as expat did.

GitHub Actions makes the executables for each release (`release.yml`): Linux
(x86-64 and ARM64, static), Windows (x86-64) and macOS (Apple silicon and Intel).
CI runs the tests on Linux, Windows and macOS. `cargo audit` replaces `pip-audit`.
Cargo replaces `uv` (ADR-0013): `Cargo.lock` has the exact versions and the
checksums of all packages, and `Cargo.toml` pins the direct dependencies with `=`
(ADR-0012). `rust-toolchain.toml` pins the Rust version.

## Consequences

- Good: Users copy one file and open it. No installation is necessary.
- Good: One file of approximately 6 MB contains the complete application.
- Good: The users of 0.3.0 keep their database and their passwords.
- Good: Manipulated XML files use less memory than with the Python version.
- Bad: The executables are not signed. Windows SmartScreen and macOS Gatekeeper
  show a warning at the first start. The README gives the procedure.
- Bad: The requests run one after the other. A long import makes the other
  requests wait. For 1 to 10 users, this is acceptable.
- Bad: Contributors must know Rust.
- Bad: `cargo audit` shows that `chrono` is not maintained (RUSTSEC-2026-0543,
  a warning, not a vulnerability). We will replace it with a maintained date library.
