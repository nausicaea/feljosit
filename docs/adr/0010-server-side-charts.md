# ADR-0010: Make the charts on the server as SVG

- Status: Accepted (the language is changed by ADR-0018)
- Date: 2026-11-25

## Context

The reports need bar charts and line charts. Chart libraries for the browser are
large JavaScript dependencies. They often need inline styles, and a strict
Content Security Policy does not permit inline styles. External CDNs would send
data about the user to third parties.

## Decision

The server calculates the chart geometry in Python (`feljosit.web.charts`).
Jinja templates make inline SVG. Jinja escapes all text. Each chart has a table
with the same data for screen readers.

## Consequences

- Good: No JavaScript dependency and no CDN.
- Good: The Content Security Policy can be strict.
- Bad: The charts are simple. They have no zoom and no animation.
