"""Alembic environment. The engine comes from ``config.attributes['engine']``."""

from __future__ import annotations

from typing import Any, Literal

from alembic import context
from alembic.autogenerate.api import AutogenContext

from feljosit.models import Base, UTCDateTime

config = context.config
target_metadata = Base.metadata


def render_item(type_: str, obj: Any, autogen_context: AutogenContext) -> str | Literal[False]:
    """Render custom column types as standard SQLAlchemy types in migration files."""
    if type_ == "type" and isinstance(obj, UTCDateTime):
        return "sa.DateTime()"
    return False


def run_migrations() -> None:
    engine = config.attributes.get("engine")
    if engine is None:
        raise RuntimeError("Alembic needs an engine in config.attributes['engine']")
    with engine.begin() as connection:
        context.configure(
            connection=connection,
            target_metadata=target_metadata,
            render_as_batch=True,
            compare_type=True,
            render_item=render_item,
        )
        with context.begin_transaction():
            context.run_migrations()


run_migrations()
