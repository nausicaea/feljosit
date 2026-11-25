from alembic.autogenerate import compare_metadata
from alembic.migration import MigrationContext
from sqlalchemy import Engine, create_engine

from feljosit.config import Settings
from feljosit.db import make_engine, migrate
from feljosit.models import Base


def test_migrations_match_models(settings: Settings) -> None:
    """The migrations must make the same schema as the models. Otherwise add a migration."""
    engine: Engine = make_engine(settings)
    migrate(engine, settings)
    with engine.connect() as connection:
        context = MigrationContext.configure(connection, opts={"compare_type": True})
        diff = compare_metadata(context, Base.metadata)
    engine.dispose()
    assert diff == []


def test_database_file_is_private(settings: Settings) -> None:
    engine = make_engine(settings)
    migrate(engine, settings)
    engine.dispose()
    assert settings.database_path.stat().st_mode & 0o077 == 0
    assert settings.data_dir.stat().st_mode & 0o077 == 0


def test_migrate_is_idempotent(settings: Settings) -> None:
    engine = create_engine(settings.database_url)
    make_engine(settings).dispose()
    migrate(engine)
    migrate(engine)
    engine.dispose()
