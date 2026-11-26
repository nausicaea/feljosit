# Contributing

Feljosit is written in Rust (see [ADR-0018](docs/adr/0018-one-executable-in-rust.md)).

## Procedure: prepare the development environment

1. Install [rustup](https://rustup.rs/). The file `rust-toolchain.toml` gives the
   Rust version. `rustup` installs it automatically at the first `cargo` command.
2. On Linux, install a C compiler (for example `gcc`). The build compiles SQLite.
   On macOS, install the Xcode command line tools. On Windows, install the
   "Desktop development with C++" workload of the Visual Studio Build Tools.
3. Install the Git hooks:

   ```bash
   pipx run pre-commit install --hook-type pre-commit --hook-type commit-msg
   ```

## Procedure: build and start

1. Build the executable:

   ```bash
   cargo build --release
   ```

2. The file is `target/release/feljosit` (`feljosit.exe` on Windows).
3. Start it with a separate data directory for the development:

   ```bash
   FELJOSIT_DATA_DIR=./data cargo run -- serve --open
   ```

## Procedure: do the checks before a pull request

1. Format the code:

   ```bash
   cargo fmt --all
   ```

2. Do the lint checks:

   ```bash
   cargo clippy --all-targets --locked -- -D warnings
   ```

3. Do the tests:

   ```bash
   cargo test --locked
   ```

CI does the same checks on Linux, Windows and macOS. It also does
`cargo audit` and CodeQL.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/). Examples:

- `feat(import): support camt.053.001.10`
- `fix(budget): roll over negative amounts correctly`
- `docs(adr): add ADR-0012 for split transactions`

You can use `pipx run --spec commitizen cz commit` to write the message.

## Procedure: change the database schema

1. Add a file `migrations/NNNN_description.sql` with the next number.
2. Add the file to the list `MIGRATIONS` in `src/db.rs`.
3. Change the structures in `src/models.rs` and the queries.
4. Do the tests. The application applies the new migration when it starts. The
   table `alembic_version` keeps the number of the last migration.

Do not change a migration that is in a release. The databases of the users
already have it.

## Procedure: make a release

1. Make sure that CI passes on `main`.
2. Increase the version in `Cargo.toml` and `Cargo.lock` and update the changelog:

   ```bash
   pipx run --spec commitizen==4.20.1 cz bump
   ```

3. Push the commit and the tag:

   ```bash
   git push --follow-tags
   ```

4. The workflow `release.yml` builds the executables for Windows, macOS and Linux.
   It attaches them and the file `SHA256SUMS` to a GitHub release.

## Rules

- Write an ADR for each important decision (see [docs/adr](docs/adr/README.md)).
- Write the documentation in ASD-STE100 (see [docs/writing-style.md](docs/writing-style.md)).
- Add tests for each change. Add a security test for each security control.
- Do not add real bank data to the repository. Use synthetic test data.
- Filter each database query by the user (`user_id`).
- Use bound parameters (`?1`) in SQL. Do not put values from a request into SQL text.
- A contribution is in the public domain, as the rest of the project ([UNLICENSE](UNLICENSE)).
