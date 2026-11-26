# Feljosit

Feljosit is a local-first personal budgeting application for the Swiss
financial sector. It uses the envelope method of YNAB ("give every franc a job").
It imports ISO 20022 camt.053 bank statements from Swiss banks.

Your data stays on your computer. The application sends no data to a cloud service.

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

- Python 3.13 or later.
- [uv](https://docs.astral.sh/uv/).
- Full-disk encryption on your computer (recommended, see [SECURITY.md](SECURITY.md)).

## Procedure: install and start

1. Clone the repository.
2. Install the dependencies:

   ```bash
   uv sync --no-dev
   ```

3. Add a user. The command asks for a password (minimum 12 characters):

   ```bash
   uv run feljosit create-user anna
   ```

4. Start the server:

   ```bash
   uv run feljosit serve
   ```

5. Open <http://127.0.0.1:8765> in your browser.
6. Log in.

For the next steps, read the [user guide](docs/user-guide.md).

## Configuration

Set these environment variables if necessary:

| Variable | Default | Description |
| --- | --- | --- |
| `FELJOSIT_DATA_DIR` | `~/.local/share/feljosit` | Directory for the database. |
| `FELJOSIT_HOST` | `127.0.0.1` | Address of the server. Do not change it without a TLS reverse proxy. |
| `FELJOSIT_PORT` | `8765` | Port of the server. |
| `FELJOSIT_ALLOWED_HOSTS` | `127.0.0.1,localhost,::1` | Permitted values of the `Host` header. |
| `FELJOSIT_SECURE_COOKIES` | `false` | Set to `true` behind a TLS reverse proxy. |

## Backup

The database is one file: `$FELJOSIT_DATA_DIR/feljosit.sqlite3`.

1. Stop the server.
2. Copy the file to an encrypted backup medium.

## Documentation

- [User guide](docs/user-guide.md)
- [Architecture decision records](docs/adr/README.md)
- [Threat model](docs/threat-model.md)
- [Security policy](SECURITY.md)
- [Contributing](CONTRIBUTING.md)
- [Writing rules (ASD-STE100)](docs/writing-style.md)

## Licence

The licence is not decided yet. Until the maintainers add a licence file, all rights are reserved.
