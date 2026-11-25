# ADR-0006: Prevent duplicate transactions with fingerprints

- Status: Accepted
- Date: 2026-11-25

## Context

Users download statements for periods that overlap. They also import the same
file two times by mistake. A transaction must not be in the database two times.

## Decision

- The application keeps the SHA-256 hash of each imported file. It refuses a
  file with the same hash.
- Each transaction has a fingerprint. The fingerprint is unique for each account.
  - If the bank supplies a reference (`AcctSvcrRef`), the fingerprint uses the
    account, the date, the amount and the reference.
  - If there is no reference, the fingerprint uses the account, the date, the
    amount, the counterparty, the description and an occurrence counter. The
    counter separates two identical transactions in one statement.
- If a fingerprint exists already, the import ignores the transaction and counts
  it as a duplicate.
- When the first statement of an account is imported, the opening balance
  (`OPBD`) becomes the starting balance. After the import, the application
  compares the closing balance (`CLBD`) with the account balance.

## Consequences

- Good: An import can be done again without risk.
- Good: The balance check shows missing statements.
- Bad: If a bank changes the reference of a transaction, a duplicate is possible.
