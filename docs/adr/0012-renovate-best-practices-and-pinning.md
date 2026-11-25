# ADR-0012: Use the Renovate best practices, pin all dependencies and wait 7 days

- Status: Accepted
- Date: 2026-11-25
- Supersedes: the Renovate part of ADR-0011

## Context

ADR-0011 configured Renovate with `config:recommended` and a waiting time of
3 days. The dependencies in `pyproject.toml` used ranges (`>=`).

Attackers sometimes publish compromised versions of popular packages. The
registries usually find and remove these versions in some days. Feljosit is an
application, not a library. Thus exact versions do not cause problems for other projects.

## Decision

- Renovate extends `config:best-practices`. This preset contains
  `config:recommended`, pinned digests for actions and Docker images,
  configuration migration, abandoned-package detection and weekly lock file maintenance.
- Renovate pins all dependencies to exact versions (`:pinAllExceptPeerDependencies`).
  This includes the runtime dependencies, the development tools and `uv_build`.
- The Python version (`requires-python`) stays a range. A pin to one Python
  release would block the installation on newer releases.
- Renovate waits 7 days after a release before it proposes an update
  (`minimumReleaseAge`). With `internalChecksFilter: strict`, Renovate does not
  open a pull request before the 7 days are complete.
- Security updates do not wait. They come at once.

## Consequences

- Good: Builds are reproducible from `pyproject.toml` alone, not only from `uv.lock`.
- Good: A compromised release has 7 days to be found before it gets into the project.
- Good: The configuration follows the recommendations of the Renovate maintainers.
- Bad: Each dependency update needs a Renovate pull request.
- Bad: Normal fixes arrive 7 days later.
