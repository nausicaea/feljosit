# Architecture decision records

This directory contains the architecture decision records (ADRs) of Feljosit.
An ADR records one important decision, the context and the consequences.

We use the [MADR](https://adr.github.io/madr/) format. We write the ADRs in
ASD-STE100 Simplified Technical English (see [the writing rules](../writing-style.md)).

## Procedure: add an ADR

1. Copy `0000-template.md`.
2. Give the file the next number and a short name.
3. Write the context, the options and the decision.
4. Set the status to "Proposed".
5. Open a pull request.
6. When the maintainers accept the pull request, set the status to "Accepted".

Do not change an accepted ADR. If a decision changes, write a new ADR.
Then set the status of the old ADR to "Superseded by ADR-NNNN".

## Index

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-record-architecture-decisions.md) | Record the architecture decisions | Accepted |
| [0002](0002-local-first-web-application.md) | Use a local-first web application in Python | Accepted |
| [0003](0003-sqlite-data-store.md) | Use SQLite as the data store | Accepted |
| [0004](0004-money-representation.md) | Keep money as integer minor units | Accepted |
| [0005](0005-camt053-parsing.md) | Parse camt.053 safely and for all versions | Accepted (ZIP limits superseded by ADR-0013) |
| [0006](0006-import-deduplication.md) | Prevent duplicate transactions with fingerprints | Accepted |
| [0007](0007-authentication-and-sessions.md) | Use local accounts and server-side sessions | Accepted |
| [0008](0008-envelope-budgeting-rules.md) | Use the envelope budgeting rules of YNAB | Accepted |
| [0009](0009-categorisation.md) | Categorise with rules and history, without regular expressions | Accepted |
| [0010](0010-server-side-charts.md) | Make the charts on the server as SVG | Accepted |
| [0011](0011-development-workflow.md) | Use conventional commits, CI and Renovate | Accepted (Renovate part superseded by ADR-0012) |
| [0012](0012-renovate-best-practices-and-pinning.md) | Use the Renovate best practices, pin all dependencies and wait 7 days | Accepted |
| [0013](0013-xml-resource-limits.md) | Limit the resources that one camt.053 import can use | Proposed |
