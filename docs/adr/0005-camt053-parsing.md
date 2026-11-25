# ADR-0005: Parse camt.053 safely and for all versions

- Status: Accepted
- Date: 2026-11-25

## Context

Swiss banks supply account statements in the ISO 20022 format camt.053, as the
Swiss Payment Standards (SPS) specify. Different banks use different versions:
camt.053.001.04 (SPS 2019 and earlier), camt.053.001.08 (SPS 2022 and later) and
others. Some banks put the XML files in a ZIP archive.

An XML file from the internet is not trusted. XML parsers can be attacked with
external entities (XXE), with entity expansion ("billion laughs") and with ZIP
bombs.

## Decision

- We use `defusedxml` with `forbid_dtd=True` and `forbid_entities=True`.
- We accept the namespaces `camt.053.001.02` to `camt.053.001.13`. We remove the
  namespace and read the elements by name. The differences between the versions
  are small:
  - Status: `<Sts>BOOK</Sts>` (02 to 06) or `<Sts><Cd>BOOK</Cd></Sts>` (07 and later).
  - Parties: `Dbtr/Nm` (02 to 07) or `Dbtr/Pty/Nm` (08 and later).
- We import only booked entries (`BOOK`).
- We split a batch entry (more than one `TxDtls`) only if each detail has its own
  amount in the account currency. Otherwise we import the entry as one transaction.
- For a debit, the counterparty is the creditor. For a credit, it is the debtor.
- We keep the structured creditor reference (for example the QR reference).
- We refuse invalid amounts, indicators, IBANs and currencies.
- For ZIP archives, we limit the number of files (100), the size of each file
  (20 MB) and the total uncompressed size (100 MB). We read with a limit and do
  not trust the sizes in the archive.
- The web layer limits the upload to 10 MB.
- We do not validate against the XSD files. The XSD files of all versions are
  large, and the parser checks the fields that it uses.

## Consequences

- Good: Statements of most Swiss banks are accepted.
- Good: The known XML attacks do not work.
- Bad: A file that is not valid against the XSD can be accepted if the used
  fields are correct.
