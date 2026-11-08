import os
import uuid

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.ext.asyncio import AsyncEngine

from reqora_server import models  # noqa: F401  (registers the tables)
from reqora_server.config import Settings
from reqora_server.db import Base
from reqora_server.main import create_app

# Run the suite against PostgreSQL by pointing this at a scratch database
# (its tables are dropped and recreated for every test).
TEST_DATABASE_URL = os.environ.get("REQORA_TEST_DATABASE_URL")

PASSWORD = "correct horse battery"


async def reset_schema(engine: AsyncEngine) -> None:
    async with engine.begin() as conn:
        await conn.run_sync(Base.metadata.drop_all)
        await conn.run_sync(Base.metadata.create_all)


@pytest.fixture
def settings(tmp_path) -> Settings:
    url = TEST_DATABASE_URL or f"sqlite+aiosqlite:///{tmp_path / 'test.db'}"
    # Most tests aren't about invites, so sign-up is open here (test_invites.py turns it off).
    return Settings(_env_file=None, database_url=url, auth_rate_limit=0, open_registration=True)


@pytest.fixture
def client(settings):
    app = create_app(settings)
    with TestClient(app) as c:
        c.portal.call(reset_schema, app.state.engine)
        yield c


def register(client: TestClient, email: str | None = None, **extra) -> dict:
    email = email or f"user-{uuid.uuid4().hex[:8]}@example.com"
    r = client.post("/api/v1/auth/register", json={"email": email, "password": PASSWORD, **extra})
    assert r.status_code == 201, r.text
    return r.json()


def bearer(token: str) -> dict:
    return {"Authorization": f"Bearer {token}"}


@pytest.fixture
def user(client) -> dict:
    """A registered account: the /register response plus ready-made auth headers."""
    out = register(client, display_name="Ada")
    out["headers"] = bearer(out["token"])
    return out
