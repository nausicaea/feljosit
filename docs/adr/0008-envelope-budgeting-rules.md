# ADR-0008: Use the envelope budgeting rules of YNAB

- Status: Accepted
- Date: 2026-11-25

## Context

The reference application is YNAB ("You Need A Budget"). Users know its method:
"give every franc a job".

## Decision

- Income goes to "Ready to Assign". A transaction is income if its category has
  the kind `income`.
- The user assigns money to expense categories for each month.
- `available = positive available of the previous month + assigned + activity`.
- A positive available amount rolls over to the next month.
- A negative available amount (overspending) does not roll over. The application
  subtracts it from "Ready to Assign" in the next month.
- Money assigned to future months reduces "Ready to Assign" now.
- Budget accounts (private, savings, credit card, cash) are part of the budget.
  Tracking accounts (pillar 3a, investments, loans) are not.
- A transfer between two budget accounts has no category. A transfer from a
  budget account to a tracking account needs a category, because the money
  leaves the budget.
- The application calculates the budget from the transactions each time. It
  does not keep calculated values.

## Consequences

- Good: YNAB users understand the budget.
- Good: There are no calculated values that can become wrong.
- Bad: The calculation reads all months. For 1 to 10 users with some thousand
  transactions, this is fast enough.
- Bad: Split transactions and credit card payment categories are not available yet.
