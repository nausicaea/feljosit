# ADR-0017: Import credit card transactions from a CSV file

- Status: Proposed
- Date: 2026-11-26

## Context

Swiss credit card companies do not supply camt.053 statements. Their apps and web
portals export the card transactions as a CSV file. The file has one line for each
transaction and these columns: `TransactionId`, `CardId`, `Date`, `ValutaDate`,
`Amount`, `Currency`, `OriginalAmount`, `OriginalCurrency`, `MerchantName`,
`MerchantPlace`, `MerchantCountry`, `StateType`, `Details`, `Type` and
`Exchange Rate`.

The file has no IBAN, no opening balance and no closing balance. Some lines (for
example fees) have numbers in the merchant columns.

## Options

1. A general CSV import with a column assignment that the user makes.
2. A parser for this one format. Its result is the same data structure as the
   result of the camt.053 parser.

## Decision

We use option 2, because it needs no configuration and it can use the import, the
duplicate check (ADR-0006), the categorisation (ADR-0009) and the transfer matching
(ADR-0016) without change.

- The import page finds the format from the content of the file: a header line with
  the columns `TransactionId`, `Date`, `Amount`, `Currency` and `StateType`.
  Comma and semicolon are both possible separators. The encoding is UTF-8 (with or
  without a byte order mark) or Windows-1252.
- The file has no IBAN, thus the user must select the account.
- A positive `Amount` is a charge (an outflow). A negative `Amount` is a credit, for
  example a payment to the card or a refund.
- Only lines with the `StateType` `BOOKED` become transactions. The import counts
  the other lines as entries that are not booked.
- The fingerprint is the SHA-256 hash of `TransactionId`. An overlapping export
  does not add a transaction two times.
- `MerchantName` is the counterparty. If it is empty, `Details` is the counterparty.
  The description contains `Details`, the place and the country of the merchant,
  and the original amount with the exchange rate for a foreign currency. A field
  without a letter (for example `0.000`) is ignored.
- All transactions of one file must be in one currency, the currency of the account.
- The transactions have the source `card_csv`.
- Limits: 100,000 lines, 1,000 characters for each field, and the upload limit of
  10 MB. Error messages show the line number, but not the values of the file.

## Consequences

- Good: The user imports a credit card the same way as a bank account.
- Good: The payment of the card bill from a bank account becomes a transfer
  automatically (ADR-0016).
- Bad: The import cannot check the balance, because the file has no balance.
- Bad: A different CSV format needs a new parser. If many formats become
  necessary, option 1 is better.
- Bad: If the sign convention of the export changes, all amounts are wrong. The
  balance of the card account then shows the problem.
