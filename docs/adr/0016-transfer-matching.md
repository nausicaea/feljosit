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
- The user did not reject this pair before.

The import applies the methods in this order: IBAN, matching, rules, history.
Thus a rule or the history does not give a category to a transfer. The button
"Apply the rules to uncategorised transactions" also applies the matching.

The table `transfer_pairs` records each pair: the outflow, the inflow, the method
(`iban` or `match`) and the status (`active` or `rejected`). The import also
records the pairs that it finds from the IBAN, if both sides are imported.

- The transaction list shows the partner of a transfer, and a filter shows the
  matched transfers.
- If the user gives a category to one transaction of a pair, or selects
  "Uncategorised", both transactions stop being transfers. The pair gets the status
  `rejected`, and the matching does not make it again.
- If the user deletes a manual transaction of a matched pair, the partner stops
  being a transfer.
- The column `is_transfer` stays. The budget and the reports use only this column.
  A transfer can have no pair, for example if only one side is imported or the
  user selected "Transfer" manually.

## Consequences

- Good: Transfers to and from accounts without IBAN are found.
- Good: A category that the user set never changes.
- Good: The user can check the matched transfers and correct a wrong transfer
  with one action.
- Bad: Two unrelated transactions with the same amount can be a wrong transfer,
  for example a payment and a refund of the same amount on two accounts. The
  import result shows the number of these transfers and links to them.
- Bad: The transfers from before this change have no pair. "Not a transfer"
  changes only one transaction of these transfers.
