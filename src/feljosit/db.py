"""Database engine, sessions and migrations."""

from __future__ import annotations

import os
import stat
from collections.abc import Iterator
from contextlib import contextmanager
from importlib import resources
from typing import Any

from alembic import command
from alembic.config import Config
from sqlalchemy import Engine, create_engine, event
from sqlalchemy.orm import Session, sessionmaker

from feljosit.config import Settings, ensure_data_dir


def _sqlite_pragmas(dbapi_connection: Any, _record: Any) -> None:
    cursor = dbapi_connection.cursor()
    cursor.execute("PRAGMA foreign_keys=ON")
    cursor.execute("PRAGMA journal_mode=WAL")
    cursor.execute("PRAGMA synchronous=NORMAL")
    cursor.execute("PRAGMA secure_delete=ON")
    cursor.close()


def make_engine(settings: Settings) -> Engine:
    """Make the SQLAlchemy engine. Make the data directory first if necessary."""
    if settings.database_url_override is None:
        ensure_data_dir(settings)
        if not settings.database_path.exists():
            # Make the file with mode 0600 before SQLite opens it.
            fd = os.open(settings.database_path, os.O_CREAT | os.O_WRONLY, 0o600)
            os.close(fd)
    engine = create_engine(settings.database_url)
    if engine.dialect.name == "sqlite":
        event.listen(engine, "connect", _sqlite_pragmas)
    return engine


def restrict_database_file(settings: Settings) -> None:
    """Set the database files to mode 0600 (only the owner can read and write)."""
    if settings.database_url_override is not None:
        return
    for suffix in ("", "-wal", "-shm"):
        path = settings.database_path.with_name(settings.database_path.name + suffix)
        if path.exists():
            path.chmod(stat.S_IRUSR | stat.S_IWUSR)


def alembic_config(engine: Engine) -> Config:
    cfg = Config()
    cfg.set_main_option("script_location", str(resources.files("feljosit").joinpath("migrations")))
    cfg.attributes["engine"] = engine
    return cfg


def migrate(engine: Engine, settings: Settings | None = None) -> None:
    """Upgrade the database schema to the latest revision."""
    command.upgrade(alembic_config(engine), "head")
    if settings is not None:
        restrict_database_file(settings)


def make_session_factory(engine: Engine) -> sessionmaker[Session]:
    return sessionmaker(bind=engine, expire_on_commit=False)


@contextmanager
def session_scope(factory: sessionmaker[Session]) -> Iterator[Session]:
    """Open a session. Commit if no error occurs, otherwise roll back."""
    session = factory()
    try:
        yield session
        session.commit()
    except Exception:
        session.rollback()
        raise
    finally:
        session.close()
