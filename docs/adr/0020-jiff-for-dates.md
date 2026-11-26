# ADR-0020: Use jiff for dates and times

- Status: Accepted
- Date: 2026-11-26
- Supersedes: the date library (`chrono`) of ADR-0018

## Context

ADR-0018 uses `chrono` for dates and times. On 2026-09-14, the RustSec advisory
RUSTSEC-2026-0543 marked `chrono` as unmaintained. No fixed version will come.
`cargo audit` shows a warning in each CI run. An unmaintained library gets no
security fixes.

Feljosit uses dates in 23 files: booking dates, months for the budget and the
reports, today's local date, times in UTC for sessions and locks, `strftime`
formats (`%d.%m.%Y`), parsing and `serde` (dates as `YYYY-MM-DD` for the templates).

## Options

The advisory recommends two libraries. We tested both on 2026-11-26 with the
operations of Feljosit, on Linux and for Windows.

1. `jiff` 1.0.3 (Andrew Gallant, the author of ripgrep and regex).
2. `time` 0.3.55 (the `time-rs` team).

| Criterion | `jiff` | `time` |
| --- | --- | --- |
| API | Version 1.0, stable | Version 0.3 |
| Security advisories | None | Two, both fixed before 0.3.55 |
| Format strings | The same `strftime` syntax as `chrono`. The strings stay the same | A different syntax (`[day].[month].[year]`). All strings change |
| Month arithmetic | Built in (`first_of_month`, `last_of_month`, spans of months) | None |
| Parsing | Strict ISO 8601 and `strptime` | Only with a format description |
| `serde` | `"2026-10-31"`, the same as `chrono` | The same with the feature `serde-human-readable` |
| Today's local date with many threads | Works (time zone database of the system, a bundled copy on Windows) | Works (`localtime_r`). A time-zone change while the program runs is not seen on Linux |
| Size added to a small test program | +400 KB on Linux, +650 KB on Windows | +25 KB on Linux, +30 KB on Windows |

## Decision

We use `jiff` (option 1).

- The change is small and safe. The format strings, the `serde` output and the
  templates stay the same. All tests pass without a change of their values.
- The API is stable (1.0), and the library has no advisories.
- The new module `src/dates.rs` contains all date functions. The other modules
  use only this module. Thus a later change of the library changes one file.
- The month arithmetic of `jiff` replaces a part of our own code.

## Consequences

- Good: `cargo audit` shows no warning.
- Good: All date code is in one module.
- Bad: The executable is larger. Measured with the release settings: Linux from
  5,717,112 to 6,009,720 bytes (+293 KB), Windows from 5,327,360 to 5,899,264
  bytes (+572 KB). Most of the increase on Windows is the bundled time zone
  database. Feljosit needs it to find today's local date on Windows.
