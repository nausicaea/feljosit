"""Command line interface."""

from __future__ import annotations

import argparse
import copy
import getpass
import logging
import os
import sys
from collections.abc import Sequence
from typing import Any

import uvicorn
from sqlalchemy import Engine, select

from feljosit import __version__
from feljosit.config import LOOPBACK_HOSTS, Settings, load_settings
from feljosit.db import make_engine, make_session_factory, migrate, session_scope
from feljosit.models import User
from feljosit.services import users
from feljosit.services.errors import ServiceError
from feljosit.web.app import create_app


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="feljosit", description="Local-first budgeting.")
    parser.add_argument("--version", action="version", version=f"feljosit {__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    serve = sub.add_parser("serve", help="Start the web application.")
    serve.add_argument("--host", help="Address to bind to (default: 127.0.0.1).")
    serve.add_argument("--port", type=int, help="Port (default: 8765).")

    sub.add_parser("migrate", help="Upgrade the database schema.")

    create = sub.add_parser("create-user", help="Add a user.")
    create.add_argument("username")

    passwd = sub.add_parser("set-password", help="Set a new password for a user.")
    passwd.add_argument("username")
    return parser


def _log_config() -> dict[str, Any]:
    """Return the uvicorn logging configuration with all handlers on stderr.

    By default, uvicorn writes the access log to stdout.
    """
    config = copy.deepcopy(uvicorn.config.LOGGING_CONFIG)
    for handler in config["handlers"].values():
        handler["stream"] = "ext://sys.stderr"
    return config


def _read_password() -> str:
    first = getpass.getpass("Password: ")
    second = getpass.getpass("Password (again): ")
    if first != second:
        raise ServiceError("The two passwords are not the same.")
    return first


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    # New files (database, WAL journal) are only readable by the owner.
    os.umask(0o077)
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")
    settings = load_settings()

    if args.command == "serve":
        host = args.host or settings.host
        port = args.port or settings.port
        if host not in LOOPBACK_HOSTS:
            print(
                f"WARNING: The application listens on {host}. Other computers can connect. "
                "Use a TLS reverse proxy and set FELJOSIT_SECURE_COOKIES=true.",
                file=sys.stderr,
            )
        app = create_app(settings)
        try:
            uvicorn.run(
                app,
                host=host,
                port=port,
                server_header=False,
                proxy_headers=False,
                log_config=_log_config(),
            )
        finally:
            app.state.engine.dispose()
        return 0

    engine = make_engine(settings)
    try:
        return _admin_command(args, engine, settings)
    except ServiceError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1
    finally:
        engine.dispose()


def _admin_command(args: argparse.Namespace, engine: Engine, settings: Settings) -> int:
    migrate(engine, settings)
    if args.command == "migrate":
        print("The database schema is up to date.")
        return 0
    password = _read_password()
    with session_scope(make_session_factory(engine)) as db:
        if args.command == "create-user":
            created = users.create_user(db, args.username, password)
            print(f"The user '{created.username}' is added.")
        else:
            user = db.scalar(select(User).where(User.username == args.username.lower()))
            if user is None:
                raise ServiceError("The user does not exist.")
            users.change_password(db, user, password)
            user.failed_logins = 0
            user.locked_until = None
            print("The password is changed.")
    return 0
