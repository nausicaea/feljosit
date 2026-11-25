# Contributing

## Procedure: prepare the development environment

1. Install [uv](https://docs.astral.sh/uv/).
2. Install the dependencies:

   ```bash
   uv sync
   ```

3. Install the Git hooks:

   ```bash
   uv run --with pre-commit pre-commit install --hook-type pre-commit --hook-type commit-msg
   ```

## Procedure: do the checks before a pull request

1. Format the code:

   ```bash
   uv run ruff format .
   ```

2. Do the lint checks:

   ```bash
   uv run ruff check .
   ```

3. Do the type checks:

   ```bash
   uv run mypy src tests
   ```

4. Do the tests:

   ```bash
   uv run pytest --cov
   ```

CI does the same checks. It also does `pip-audit` and CodeQL.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/). Examples:

- `feat(import): support camt.053.001.10`
- `fix(budget): roll over negative amounts correctly`
- `docs(adr): add ADR-0012 for split transactions`

You can use `uv run cz commit` to write the message.

## Procedure: change the database schema

1. Change `src/feljosit/models.py`.
2. Make a migration:

   ```bash
   uv run python -c "from alembic import command; from sqlalchemy import create_engine; from feljosit.db import alembic_config, migrate; e = create_engine('sqlite:///tmp.sqlite3'); migrate(e); command.revision(alembic_config(e), message='describe the change', autogenerate=True)"
   ```

3. Delete `tmp.sqlite3`.
4. Read the migration file and correct it if necessary.
5. Do the tests. `tests/test_migrations.py` compares the migrations with the models.

## Procedure: make a release

1. Make sure that CI passes on `main`.
2. Increase the version and update the changelog:

   ```bash
   uv run cz bump
   ```

3. Push the commit and the tag:

   ```bash
   git push --follow-tags
   ```

## Rules

- Write an ADR for each important decision (see [docs/adr](docs/adr/README.md)).
- Write the documentation in ASD-STE100 (see [docs/writing-style.md](docs/writing-style.md)).
- Add tests for each change. Add a security test for each security control.
- Do not add real bank data to the repository. Use synthetic test data.
- Filter each database query by the user (`user_id`).
