# ADR-0016: Find transfers from matching transactions

- Status: Proposed
- Date: 2026-11-26

## Context

The import finds a transfer from the counterparty IBAN (ADR-0009). Some
statements do not show the IBAN of the other account. Examples: the statement of
a credit card, a payment to a credit card company, or an account that has no IBAN
in the application. These transfers are expenses and income in the budget, and the
user must correct them manually.

A transfer has two transactions: an outflow on one account and an inflow of the
same amount on a different account. Banks often book the two transactions on
different days, for example over a weekend.

A wrong transfer is worse than a missing transfer: it hides an expense, and the
user possibly does not see it.

## Options

1. Do nothing. The user selects "Transfer" manually.
2. Match the two transactions automatically, with strict rules.
3. Suggest matching pairs. The user accepts each pair.

## Decision

We use option 2, because the user wants the transfers automatically, and strict
rules make wrong transfers rare. Two transactions are a transfer if all these
conditions are true:

- Both are uncategorised and are not transfers. Starting balances are not included.
- The accounts are different, belong to the same user, and have the same budget
  setting (ADR-0008).
- The currency is the same, and the amounts are opposite.
- The booking dates are at most 4 days apart.
- The pair is unambiguous: each transaction is the only nearest candidate of the
  other.

The import applies the methods in this order: IBAN, matching, rules, history.
Thus a rule or the history does not give a category to a transfer. The button
"Apply the rules to uncategorised transactions" also applies the matching.

## Consequences

- Good: Transfers to and from accounts without IBAN are found.
- Good: A category that the user set never changes.
- Bad: Two unrelated transactions with the same amount can be a wrong transfer,
  for example a payment and a refund of the same amount on two accounts. The
  import result shows the number of these transfers, so that the user can check
  them.
- Bad: The application does not store which two transactions are a pair. If the
  user gives a category to one transaction of a pair, the other one stays a
  transfer.
