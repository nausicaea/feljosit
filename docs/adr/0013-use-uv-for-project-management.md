# ADR-0013: Use uv for project and dependency management

- Status: Superseded by ADR-0018
- Date: 2026-11-25
- Related: ADR-0011, ADR-0012

## Context

ADR-0011 says that `uv` manages the dependencies, but it does not give the
reasons. This ADR records the reasons and the options that we compared.

The project has these requirements:

1. Reproducible builds. Each installation must use the same versions of all
   packages, also the indirect dependencies, and must check their hashes.
2. Supply-chain control. Renovate must update the direct and the indirect
   dependencies (ADR-0012). `pip-audit` must examine the exact versions.
3. Low maintenance. One person maintains the project. Few tools are better.
4. Standards. The project metadata must be in `pyproject.toml` (PEP 621).
   Then a change to a different tool is possible.
5. Fast CI.

## Options

We examined the documentation and the source code of the current versions on
2026-11-25: uv, Hatch 1.18.2, Poetry 2.5.2 and Renovate.

### 1. uv

`uv` installs Python, makes the virtual environment, locks, installs, runs
commands and builds the package (`uv_build`).

- Good: `uv.lock` contains the exact versions and hashes of all packages, for
  all platforms. `uv sync --locked` and `uv lock --check` fail if the lock file
  does not agree with `pyproject.toml`.
- Good: One tool does all the tasks.
- Good: Renovate supports `uv.lock`, including weekly lock file maintenance.
  A maintained GitHub Action (`astral-sh/setup-uv`) is available.
  `uv export` gives the input for `pip-audit`.
- Good: It is very fast. A CI job takes less than one minute.
- Good: It uses PEP 621 metadata and dependency groups (PEP 735).
- Bad: A company (Astral) develops it, not the Python Packaging Authority (PyPA).
  If the company changes its direction, the project can be affected.
- Bad: The tool is younger than the other options.

### 2. Hatch

Hatch is a PyPA project. It manages environments, can install Python and
builds the package (`hatchling`).

- Good: It is a PyPA project with open governance.
- Good: It has a good environment matrix, for example tests on more than one
  Python version.
- Good: It uses PEP 621 metadata.
- Good: Since Hatch 1.17.0 (May 2026), it makes standard PEP 751 lock files
  (`pylock.toml`) with exact versions and hashes.
- Bad: Locking is new and optional. You must enable it for each environment
  (`locked = true`) or for all environments (`lock-envs = true`).
- Bad: The default `pip` locker cannot apply a lock file (`hatch dep sync`).
  For this, Hatch needs the `uv` locker. Thus a complete locked workflow with
  Hatch also needs `uv`.
- Bad: Renovate does not update `pylock.toml` files. Thus Renovate cannot do
  lock file maintenance. This does not satisfy requirement 2.

### 3. Poetry

Poetry manages the dependencies and the virtual environment, locks and builds
the package (`poetry-core`).

- Good: It is mature, and many projects use it.
- Good: `poetry.lock` contains exact versions and hashes. Renovate supports it.
- Good: Since Poetry 2, it can use PEP 621 metadata.
- Bad: Older Poetry versions used their own `[tool.poetry]` format. Much
  documentation and many examples still use this format.
- Bad: Python installation (`poetry python install`, since Poetry 2.1) is
  still experimental. An additional tool (for example `pyenv`) is frequently necessary.
- Bad: It is slower than `uv`, both for locking and for installation.

### 4. setuptools with pip

`setuptools` is a build backend. `pip` and `venv` install the packages.

- Good: These tools are the most widely known. They are always available.
- Good: There is no dependency on a company.
- Bad: `setuptools` only builds the package. Locking needs an additional tool,
  for example `pip-tools` (`pip-compile --generate-hashes`). The newer standard
  lock file `pylock.toml` (PEP 751) is an alternative, but the tools do not
  fully support it yet.
- Bad: The maintainer must connect three or four tools: `venv`, `pip`, a lock
  tool and `setuptools`. This does not satisfy requirement 3.

## Decision

We use `uv` (option 1). It is the only option that satisfies all five
requirements with one tool.

Hatch is the strongest alternative, because it is a PyPA project and now has
standard lock files. We will examine it again when Renovate supports
`pylock.toml` and when Hatch locking is mature.

We decrease the risk of the company dependency:

- The project metadata stays standard PEP 621 in `pyproject.toml`.
- Only `uv.lock` and the `uv_build` backend are specific to `uv`.
- `uv export` can write `requirements.txt` with hashes, and `uv` can also
  write `pylock.toml` (PEP 751). Both formats work without `uv`.

## Consequences

- Good: One tool and one lock file give reproducible builds.
- Good: Renovate, `pip-audit` and CI use the same exact versions.
- Good: CI is fast.
- Bad: Contributors must install `uv`.
- Bad: If `uv` is not maintained in the future, we must change the tool.
  We estimate this change at less than one day of work: change the build
  backend in `pyproject.toml`, make a new lock file and change the CI workflows.
