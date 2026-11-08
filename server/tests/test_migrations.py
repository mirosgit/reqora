import asyncio
from pathlib import Path

from alembic import command
from alembic.autogenerate import compare_metadata
from alembic.config import Config
from alembic.migration import MigrationContext
from sqlalchemy import text
from sqlalchemy.ext.asyncio import create_async_engine

from reqora_server.db import Base

from .conftest import TEST_DATABASE_URL

SERVER_DIR = Path(__file__).resolve().parent.parent


def alembic_config(url: str) -> Config:
    cfg = Config(str(SERVER_DIR / "alembic.ini"))
    cfg.attributes["database_url"] = url
    cfg.attributes["configure_logger"] = False
    return cfg


async def drop_everything(url: str) -> None:
    engine = create_async_engine(url)
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.drop_all)
        await conn.execute(text("DROP TABLE IF EXISTS alembic_version"))
    await engine.dispose()


async def schema_diff(url: str) -> list:
    engine = create_async_engine(url)
    async with engine.connect() as conn:
        diff = await conn.run_sync(lambda c: compare_metadata(MigrationContext.configure(c), Base.metadata))
    await engine.dispose()
    return diff


def test_migrations_match_models(tmp_path):
    url = TEST_DATABASE_URL or f"sqlite+aiosqlite:///{tmp_path / 'migrated.db'}"
    asyncio.run(drop_everything(url))
    cfg = alembic_config(url)

    command.upgrade(cfg, "head")
    diff = asyncio.run(schema_diff(url))
    assert diff == [], f"models and migrations differ; add a migration: {diff}"

    command.downgrade(cfg, "base")
