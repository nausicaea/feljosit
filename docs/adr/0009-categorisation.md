# ADR-0009: Categorise with rules and history, without regular expressions

- Status: Accepted
- Date: 2026-11-25

## Context

A household has some hundred transactions each month. Manual categorisation
takes too much time. Most transactions repeat (rent, health insurance, shops).

## Decision

The application uses these steps, in this sequence:

1. Transfer detection: the counterparty IBAN is the IBAN of a different account of the user.
2. Rules: the user makes rules (field, "contains" or "equals", text, category, priority).
3. History: the category of the last transaction with the same counterparty IBAN
   or the same counterparty name.

The rules use case-insensitive text comparison. They do not use regular
expressions, because a bad regular expression can stop the server (ReDoS).

## Consequences

- Good: Most repeated transactions get a category automatically.
- Good: The rules are simple and safe.
- Bad: Rules are less powerful than regular expressions.
