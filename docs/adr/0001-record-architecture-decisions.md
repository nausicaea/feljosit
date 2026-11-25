# ADR-0001: Record the architecture decisions

- Status: Accepted
- Date: 2026-11-25

## Context

Feljosit has few maintainers. A new maintainer must understand why the
application works as it does. Comments in the code do not give this history.

## Decision

We record each important decision in an ADR. We use the MADR format.
We write the ADRs in ASD-STE100 Simplified Technical English.

## Consequences

- Good: New maintainers can read the reasons for the design.
- Good: A pull request that changes the architecture must include an ADR.
- Bad: Each important decision needs some more work.
