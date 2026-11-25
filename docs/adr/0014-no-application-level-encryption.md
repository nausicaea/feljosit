# ADR-0014: Do not encrypt the data in the application

- Status: Rejected
- Date: 2026-11-25

## Context

The database is not encrypted by the application (ADR-0003). We examined three
requirements:

1. Encryption at rest of the database file.
2. Encryption of the rows of each user with a key of that user.
3. A host that is not trusted, and more than one device for each user.

The threats are a stolen computer, copies of the database file, and an operator
who reads the data of other users.

## Options

1. Keep the current design: file permissions, full-disk encryption and encrypted
   backups (ADR-0003).
2. Encrypt the database file with SQLCipher.
3. Encrypt the rows of each user with a key from the password of that user.
4. Keep the data and the key only on the devices of the user. Synchronise the
   devices with end-to-end encryption. This is option 2 of ADR-0002.

## Decision

We keep option 1. We reject options 2, 3 and 4.

- Option 2: Full-disk encryption and encrypted backups give the same protection.
  The Python bindings need a special build (ADR-0003). If the key is near the
  database, encryption gives no protection. If a passphrase supplies the key,
  the operator must type it at each start. Malware that runs as the same user
  can read the key in memory.
- Option 3: The operator controls the code. A malicious operator can change the
  code and record the passwords or the keys at the next login. Thus option 3 does
  not protect the users from the operator. The access control (each query filters
  by `user_id`) already prevents that a user reads the data of a different user.
  Option 3 also has high costs:
  - The reports and the budget use SQL `SUM` and date functions.
  - The search uses `LIKE`, and the categorisation compares counterparty names.
  - The unique constraint on `(user_id, iban)` needs plaintext.
  - A password reset with `feljosit set-password` destroys the data if there is
    no recovery key.
- Option 4: Only this option protects the users from a host that is not trusted.
  It is a new architecture and supersedes ADR-0002. The camt.053 parser moves to
  the browser or to a desktop application, and it loses the XML protections of
  Python (ADR-0005, ADR-0013). The cost is too high for 1 to 10 users.

## Consequences

- Good: The design stays simple. The reports and the search use SQL.
- Good: The installation needs no special build.
- Bad: The operator of an instance can read the data of all users of that
  instance. The threat model records this risk.
- Bad: The application cannot be used on a host that is not trusted.
- If these requirements come back, start from option 4. Write a new ADR that
  supersedes ADR-0002.
