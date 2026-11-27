# Feljosit

Feljosit is a local-first personal budgeting application for the Swiss
financial sector. It uses the envelope method of YNAB ("give every franc a job").
It imports ISO 20022 camt.053 bank statements from Swiss banks.

Your data stays on your computer, or on the private server of your family. The
application sends no data to a cloud service.

You can use Feljosit in two ways:

- On your own computer: start the file (see "Procedure: start Feljosit").
- On a family server: one family member (the operator) hosts Feljosit in the home
  network. The other family members use a browser. See the
  [operator guide](docs/operator-guide.md).

Feljosit is not designed for the open internet.

> [!NOTE]
> **Claude wrote this project almost entirely.** Claude is an AI model from Anthropic. Claude wrote
> almost all of the code, the tests, the Docker files and the documentation in Claude Code
> sessions. A human gave the requirements, examined the results and merged the changes.
>
> - Model: Claude Opus 5.5 (`claude-opus-5-5`), through Claude Code
> - See [Authorship](#authorship).

## Features

- Import of camt.053 statements (versions 02 to 13, XML or ZIP), as the Swiss
  Payment Standards specify. The import ignores duplicates and compares the
  closing balance.
- Import of credit card transactions from a CSV export (see ADR-0017).
- A transaction list that you can filter and sort by date or amount.
- Automatic categorisation with rules and with the history of your transactions.
  Detection of transfers between your accounts.
- More than one account, with these types: private account, savings account,
  credit card, cash, pillar 3a, investment and loan. Budget accounts and tracking accounts.
- Envelope budget: "Ready to Assign", assigned, activity and available amounts
  for each month, with rollover.
- Reports: income and expenses for each month, spending by category, savings rate
  and net worth.
- 1 to 10 users. Each user has a separate budget.
- Swiss formats: `1'234.50`, `31.12.2026`, IBAN and QR reference.

## Requirements

- Windows 10 or later, macOS 11 or later, or Linux (x86-64 or ARM64).
- A web browser.
- Full-disk encryption on your computer (recommended, see [SECURITY.md](SECURITY.md)).

You do not have to install other software. Feljosit is one executable file. It
contains the web server, the database engine (SQLite) and all pages.

## Procedure: start Feljosit

1. Download the file for your computer from the
   [releases](https://github.com/nausicaea/feljosit/releases):

   | Computer | File |
   | --- | --- |
   | Windows | `feljosit-VERSION-windows-x86_64.exe` |
   | Mac with Apple silicon (M1 and later) | `feljosit-VERSION-macos-aarch64` |
   | Mac with an Intel processor | `feljosit-VERSION-macos-x86_64` |
   | Linux (most computers) | `feljosit-VERSION-linux-x86_64` |
   | Linux on ARM (for example Raspberry Pi) | `feljosit-VERSION-linux-aarch64` |

2. Copy the file to a folder, for example your "Programs" or "Applications" folder.
3. Start the file:
   - Windows: double-click the file.
   - macOS and Linux: open a terminal in the folder and type the name of the file,
     for example `./feljosit-VERSION-macos-aarch64`.
4. The browser opens <http://127.0.0.1:8765>. At the first start, add your user.
5. To stop Feljosit, close its window or press Ctrl+C in the terminal. On macOS and
   Linux, the server also stops on the signal `SIGTERM`.

For the next steps, read the [user guide](docs/user-guide.md).

## Procedure: start Feljosit in a container

Each release also has a signed container image for Linux (x86-64 and ARM64):
`ghcr.io/nausicaea/feljosit`. The image has no shell. The application runs as
the user `nonroot` (UID 65532). The database is in the volume `/data`.

> [!WARNING]
> Publish the port only on `127.0.0.1` of the host. If other devices must connect,
> use a TLS reverse proxy (see the [operator guide](docs/operator-guide.md)).

1. Start the container:

   ```bash
   docker run --detach --name feljosit --publish 127.0.0.1:8765:8765 --volume feljosit-data:/data ghcr.io/nausicaea/feljosit:VERSION
   ```

2. Add the first user. The command asks for a password:

   ```bash
   docker exec -it feljosit feljosit create-user NAME
   ```

3. Open <http://127.0.0.1:8765> in the browser.
4. To stop the container, type `docker stop feljosit`.

The browser cannot add the first user in a container (see
[ADR-0022](docs/adr/0022-signed-container-image.md)). On `docker stop`, the server
completes the open requests (maximum 5 seconds), closes the database and stops.

With Docker Compose, use this file `compose.yaml`:

```yaml
services:
  feljosit:
    image: ghcr.io/nausicaea/feljosit:VERSION
    ports:
      - "127.0.0.1:8765:8765"
    volumes:
      - feljosit-data:/data
volumes:
  feljosit-data:
```

### Procedure: verify the container image

1. Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/).
2. Verify the signature:

   ```bash
   cosign verify ghcr.io/nausicaea/feljosit:VERSION --certificate-identity-regexp '^https://github\.com/nausicaea/feljosit/\.github/workflows/release\.yml@refs/tags/v' --certificate-oidc-issuer https://token.actions.githubusercontent.com
   ```

3. Show the provenance and the SBOM:

   ```bash
   docker buildx imagetools inspect ghcr.io/nausicaea/feljosit:VERSION --format '{{ json .Provenance }}'
   ```

   ```bash
   docker buildx imagetools inspect ghcr.io/nausicaea/feljosit:VERSION --format '{{ json .SBOM }}'
   ```

To build the image from the source code, type `docker build -t feljosit .`.

The files are not signed. At the first start, the operating system shows a warning:

- Windows (SmartScreen): click "More info", then "Run anyway".
- macOS (Gatekeeper): in Finder, right-click the file, click "Open", then click
  "Open" again. Or type `xattr -d com.apple.quarantine FILE` in a terminal.
- Linux: make the file executable with `chmod +x FILE`.

Compare the checksum of the file with the file `SHA256SUMS` of the release.

## Commands

You can also use Feljosit in a terminal:

| Command | Description |
| --- | --- |
| `feljosit` | Start the server and open the browser. |
| `feljosit serve` | Start the server. Add `--open` to open the browser, `--port 9000` for a different port. |
| `feljosit create-user NAME` | Add a user. The command asks for a password (minimum 12 characters). |
| `feljosit set-password NAME` | Set a new password. This also unlocks a locked user. |
| `feljosit migrate` | Upgrade the database schema. The server also does this when it starts. |
| `feljosit --help` | Show all commands. |

## Configuration

Set these environment variables if necessary:

| Variable | Default | Description |
| --- | --- | --- |
| `FELJOSIT_DATA_DIR` | See below | Directory for the database. |
| `FELJOSIT_HOST` | `127.0.0.1` | Address of the server. Do not change it without a TLS reverse proxy. |
| `FELJOSIT_PORT` | `8765` | Port of the server. |
| `FELJOSIT_ALLOWED_HOSTS` | `127.0.0.1,localhost,::1` | Permitted values of the `Host` header. |
| `FELJOSIT_SECURE_COOKIES` | `false` | Set to `true` behind a TLS reverse proxy. |

The default data directory:

| System | Directory |
| --- | --- |
| Windows | `%APPDATA%\feljosit` |
| macOS | `~/Library/Application Support/feljosit` |
| Linux | `~/.local/share/feljosit` (or `$XDG_DATA_HOME/feljosit`) |

## Update from version 0.3.0 (Python)

Version 0.3.0 was a Python application. The new version uses the same database
file, the same passwords and the same import rules (see
[ADR-0018](docs/adr/0018-one-executable-in-rust.md)).

1. Stop the old server.
2. Make a backup of the database file.
3. Start the new file. It finds the database of version 0.3.0 in
   `~/.local/share/feljosit`, also on Windows and macOS. If you used
   `FELJOSIT_DATA_DIR`, set it again.

## Backup

The database is one file: `feljosit.sqlite3` in the data directory.

1. Stop the server.
2. Copy the file to an encrypted backup medium.

## Documentation

- [User guide](docs/user-guide.md)
- [Operator guide for a family server](docs/operator-guide.md)
- [Architecture decision records](docs/adr/README.md)
- [Threat model](docs/threat-model.md)
- [Security policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md) (with the procedure to build Feljosit from the source code)
- [Writing rules (ASD-STE100)](docs/writing-style.md)

## Authorship

Claude, an AI model of Anthropic, wrote this project with Claude Code: all the
source code (first in Python, then in Rust), the tests, the templates, the
documentation, the architecture decision records and the CI workflows.

The maintainer gave the requirements and the decisions, examined the changes and
merged them. The maintainer made only these changes without Claude:

- The version changes with `cz bump` (version numbers and changelog).
- One change to `.pre-commit-config.yaml` on 25 November 2026.

The Git history shows the work of Claude: each such commit has the trailer
`Co-Authored-By: Claude` or has Claude as its author.

The time frame of the development:

| Date | Work |
| --- | --- |
| 25 November 2026 | The first commit (16:59 UTC). The Python version 0.1.0 and 0.2.0. |
| 26 November 2026 | The Python version 0.3.0. The rewrite in Rust as one executable file (0.4.0). |

All of the work took less than two days.

## Licence

Feljosit is in the public domain. The maintainer releases it with
[The Unlicense](UNLICENSE). You can copy, change, publish, use, sell and
distribute it for each purpose, without conditions.

The executable file also contains libraries of other authors (for example
`minijinja`, `rusqlite`, `argon2` and `jiff`). Their licences stay valid. Most of
them use the MIT or the Apache 2.0 licence. SQLite is in the public domain.
`Cargo.lock` lists all libraries.
