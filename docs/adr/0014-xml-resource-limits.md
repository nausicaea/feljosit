# ADR-0014: Limit the resources that one camt.053 import can use

- Status: Proposed
- Date: 2026-11-25
- Supersedes: the ZIP limits of ADR-0005

## Context

ADR-0005 uses `defusedxml`. It refuses DTDs and entities. Tests show that XXE and
"billion laughs" attacks do not work. But a security review found other problems
with manipulated files:

1. An amount such as `1E999999` (60 bytes) used the CPU for approximately 18
   seconds. The import ran in the event loop, thus all requests stopped.
2. A ZIP file of 95 KB expanded to 5 files of 19 MB. The import used 27 seconds
   of CPU and 530 MB of memory.
3. `defusedxml` does not limit the number of elements or the nesting depth. A 10 MB
   file with 2.6 million empty elements used 260 MB of memory.
4. A date that is not valid (for example `2026-13-01`) or a very large amount
   caused an internal server error (HTTP 500).
5. Some error messages included values from the file.
6. Two imports at the same time could race the duplicate check.
7. The protection against some libexpat attacks (CVE-2024-8176, CVE-2025-59375)
   is in libexpat, not in `defusedxml`. The libexpat version depends on the Python
   build.

## Options

1. Keep `defusedxml` and add limits in the application.
2. Use `lxml` with `huge_tree=False` and `resolve_entities=False`.
3. Validate each file against the XSD before the import.

## Decision

We use option 1. `lxml` is a large C dependency, and its defaults are not safe
for all versions. XSD validation does not limit the memory before the parse.

- The parser uses a `TreeBuilder` with limits: 1,000,000 elements, a nesting depth
  of 64 and 16 attributes for each element. A real statement of 7 MB has
  approximately 425,000 elements and parses in less than 1 second.
- An amount must agree with the pattern `\d{1,12}(\.\d{1,5})?`. Exponents, signs,
  NaN and infinity are refused before `Decimal()` reads the value.
- Dates that are not valid and other value errors give a `Camt053Error`.
- Error messages do not include values from the file.
- ZIP limits: 50 files, 10 MB for each file (the same as the upload limit) and
  25 MB in total.
- The import route is synchronous. FastAPI runs it in a worker thread. A lock
  permits only one import at a time. The import is committed before the lock is
  released.
- At start, the application writes a warning if libexpat is older than 2.7.2. A
  test makes sure that the CI Python has these fixes.

## Consequences

- Good: One upload cannot stop the server or use a large quantity of memory.
- Good: A manipulated file gives an error message, not HTTP 500.
- Bad: A ZIP file with more than 25 MB of XML is refused. The user must import the
  files one by one.
- Bad: Imports of different users wait for each other.
- Bad: A file with many attributes on one element can still use approximately
  280 MB of memory before the limit has an effect, because libexpat reads all
  attributes first. The upload limit of 10 MB limits this.
