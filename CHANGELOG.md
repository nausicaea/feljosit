# Changelog

All important changes are in this file. Commitizen updates it with `cz bump`.

## v0.4.0 (2026-11-26)

### BREAKING CHANGE

- Python and uv are not used any more. Build with cargo. The
default data directory on Windows and macOS changes; the application still
finds a database in ~/.local/share/feljosit.

### Feat

- rewrite Feljosit in Rust as one executable for Windows, macOS and Linux
- **transactions**: sort the transaction list by amount (#26)
- **import**: import credit card transactions from a CSV export (#25)
- **transactions**: select all matching transactions for the bulk category change (#24)
- **transactions**: filter by amount and direction, and give one category to many transactions (#23)
- **transactions**: filter the transaction list by category and transfer kind (#22)
- **import**: find transfers from two matching transactions (#21)
- **transactions**: show the account number of the counterparty (#19)
- **rules**: edit categorisation rules (#18)
- **categories**: move categories into other groups by drag-and-drop (#17)
- **reports**: compare spending per counterparty over time (#16)
- **reports**: compare spending per category over time (#15)
- **import**: add unknown accounts during the import; edit accounts (#14)
- **reports**: add custom date range and "All" period to reports (#9)

### Fix

- **transactions**: readable transaction list on phones (#20)
- **reports**: show fewer month labels on long periods (#10)
- **ui**: keep selects in forms inside their card (#11)
- **budget**: fit the budget table on narrow screens (#12)
- Fix logging and query parameter handling in CLI and web routes (#8)

## v0.3.0 (2026-11-26)

### Feat

- **transactions**: sort the transaction list by amount (#26)
- **import**: import credit card transactions from a CSV export (#25)
- **transactions**: select all matching transactions for the bulk category change (#24)
- **transactions**: filter by amount and direction, and give one category to many transactions (#23)
- **transactions**: filter the transaction list by category and transfer kind (#22)
- **import**: find transfers from two matching transactions (#21)
- **transactions**: show the account number of the counterparty (#19)
- **rules**: edit categorisation rules (#18)
- **categories**: move categories into other groups by drag-and-drop (#17)
- **reports**: compare spending per counterparty over time (#16)
- **reports**: compare spending per category over time (#15)
- **import**: add unknown accounts during the import; edit accounts (#14)

### Fix

- **transactions**: readable transaction list on phones (#20)
- **reports**: show fewer month labels on long periods (#10)
- **ui**: keep selects in forms inside their card (#11)
- **budget**: fit the budget table on narrow screens (#12)

## v0.2.0 (2026-11-25)

### Feat

- add the local-first budgeting application with camt.053 import (#1)

### Fix

- **import**: limit the resources of camt.053 parsing; record the encryption decision (#6)
