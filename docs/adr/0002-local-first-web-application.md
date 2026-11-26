# ADR-0002: Use a local-first web application in Python

- Status: Accepted (the Python stack is superseded by ADR-0018)
- Date: 2026-11-25

## Context

Feljosit is a personal budgeting application for 1 to 10 users, for example one
household. Bank data is very sensitive. The users must keep the data on their
own computer. No cloud service must receive the data.

The application must be easy to maintain by one person.

## Options

1. A desktop application (for example Tauri or Qt).
2. A browser-only application that keeps the data in IndexedDB.
3. A small web server on the local computer with server-rendered HTML.

## Decision

We use option 3. The stack is:

- Python 3.13, FastAPI and Uvicorn.
- Jinja2 templates. The server makes all HTML.
- SQLAlchemy 2 and Alembic (see ADR-0003).
- Almost no JavaScript. The pages also work without JavaScript.

The server listens on `127.0.0.1` by default. Thus other computers cannot
connect. Users in the same household can use one computer, or an operator can
put a TLS reverse proxy in front of the server.

The layers are:

- `feljosit.camt053`, `feljosit.money`, `feljosit.iban`: pure functions, no database.
- `feljosit.services`: business logic. The web layer and the CLI use it.
- `feljosit.web`: HTTP routes, templates and security middleware.

## Consequences

- Good: The data stays on the computer of the user.
- Good: Python has good libraries for XML security and password hashing.
- Good: Server-rendered HTML is simple and permits a strict Content Security Policy.
- Bad: The user must start a server process. The README gives the procedure.
- Bad: There is no mobile application.
