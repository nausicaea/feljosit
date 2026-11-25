# ADR-0004: Keep money as integer minor units

- Status: Accepted
- Date: 2026-11-25

## Context

Binary floating point numbers cannot show 0.10 exactly. Budget calculations
must be exact to the Rappen.

Swiss users have accounts in CHF and sometimes in EUR or USD. A budget with
more than one currency needs exchange rates. Exchange rates make the budget
difficult to understand.

## Decision

- We keep each amount as a signed integer in the minor unit of the currency
  (for CHF: Rappen). A negative amount is an outflow.
- We parse amounts with `decimal.Decimal`. We refuse amounts with more decimal
  places than the currency permits (ISO 4217).
- The budget has one currency (default: CHF).
- A budget account must use the budget currency. An account in a different
  currency must be a tracking account.
- The application formats amounts in the Swiss style: `1'234.50`.

## Consequences

- Good: There are no rounding errors.
- Good: The budget is simple.
- Bad: A EUR account cannot be part of the budget. The net worth report shows
  one chart for each currency.
