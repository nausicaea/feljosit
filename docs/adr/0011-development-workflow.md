# ADR-0011: Use conventional commits, CI and Renovate

- Status: Accepted. The Renovate part is superseded by ADR-0012. The Python tools are superseded by ADR-0018.
- Date: 2026-11-25

## Context

The project must be maintainable by one person for a long time. Dependencies get
security updates frequently.

## Decision

- Commit messages follow Conventional Commits. Commitizen checks the messages
  (`cz check`) in a pre-commit hook and in CI. Commitizen makes the version
  number and the changelog (`cz bump`).
- `uv` manages the dependencies. `uv.lock` is in the repository. CI uses
  `uv sync --locked`.
- GitHub Actions runs `ruff format`, `ruff check` (with the Bandit rules),
  `mypy --strict`, `pytest` with a minimum coverage of 85 %, `pip-audit` and CodeQL.
- The workflows use actions pinned to a commit SHA and have read-only permissions.
- Renovate opens pull requests for dependency updates each week. Security updates
  come at once. Renovate waits 3 days after a release, to decrease the risk of a
  compromised package.
- Documentation is in ASD-STE100 Simplified Technical English.

## Consequences

- Good: The history and the changelog are clear.
- Good: Dependencies stay current with little work.
- Bad: Contributors must learn the commit format and the writing rules.
