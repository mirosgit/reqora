import asyncio
from datetime import UTC, datetime, timedelta

import httpx
import pytest
from fastapi.testclient import TestClient
from sqlalchemy import update

from reqora_server import admin
from reqora_server.main import create_app
from reqora_server.models import Invite

from .conftest import PASSWORD, reset_schema

REGISTER = "/api/v1/auth/register"


@pytest.fixture
def invite_settings(settings):
    settings.open_registration = False
    return settings


@pytest.fixture
def client(invite_settings):
    app = create_app(invite_settings)
    with TestClient(app) as c:
        c.portal.call(reset_schema, app.state.engine)
        yield c


def make_invite(settings, capsys, *args) -> str:
    capsys.readouterr()
    assert admin.main(["create-invite", *args], settings=settings) == 0
    codes = [line.strip() for line in capsys.readouterr().out.splitlines() if line.startswith("  ")]
    assert len(codes) == 1
    return codes[0]


def register(client, email, code=None):
    body = {"email": email, "password": PASSWORD}
    if code is not None:
        body["invite_code"] = code
    return client.post(REGISTER, json=body)


def test_sign_up_needs_an_invite_by_default(client):
    assert client.get("/api/v1/server-info").json()["registration"] == "invite"
    r = register(client, "ada@example.com")
    assert r.status_code == 403
    assert "invite-only" in r.json()["detail"]
    r = register(client, "ada@example.com", "AAAA-BBBB-CCCC")
    assert r.status_code == 403
    assert r.json()["detail"] == "This invite code is invalid, expired or already used"


def test_invite_works_once(client, invite_settings, capsys):
    code = make_invite(invite_settings, capsys, "--note", "Ada")
    assert len(code) == 14 and code.count("-") == 2
    # Case, dashes and spaces don't matter.
    typed = code.replace("-", " ").lower()
    r = register(client, "ada@example.com", typed)
    assert r.status_code == 201, r.text
    assert register(client, "bob@example.com", code).status_code == 403

    capsys.readouterr()
    admin.main(["list-invites", "--all"], settings=invite_settings)
    assert "used by ada@example.com" in capsys.readouterr().out
    admin.main(["list-invites"], settings=invite_settings)
    assert "No open invites" in capsys.readouterr().out


def test_invite_for_one_email(client, invite_settings, capsys):
    code = make_invite(invite_settings, capsys, "--email", "Bob@Example.com")
    assert register(client, "eve@example.com", code).status_code == 403
    assert register(client, "bob@example.com", code).status_code == 201


def test_taken_email_does_not_burn_the_invite(client, invite_settings, capsys):
    register(client, "ada@example.com", make_invite(invite_settings, capsys)).raise_for_status()
    code = make_invite(invite_settings, capsys)
    assert register(client, "ada@example.com", code).status_code == 409
    assert register(client, "bob@example.com", code).status_code == 201


def test_expired_and_revoked_invites(client, invite_settings, capsys):
    expired = make_invite(invite_settings, capsys)

    async def expire():
        async with client.app.state.sessionmaker() as db:
            await db.execute(update(Invite).values(expires_at=datetime.now(UTC) - timedelta(seconds=1)))
            await db.commit()

    client.portal.call(expire)
    assert register(client, "ada@example.com", expired).status_code == 403

    revoked = make_invite(invite_settings, capsys)
    capsys.readouterr()
    admin.main(["list-invites"], settings=invite_settings)
    invite_id = capsys.readouterr().out.splitlines()[1].split()[0]
    assert admin.main(["revoke-invite", invite_id], settings=invite_settings) == 0
    assert register(client, "ada@example.com", revoked).status_code == 403
    assert admin.main(["revoke-invite", invite_id], settings=invite_settings) == 1


def test_admin_invite_validation(invite_settings, client, capsys):
    assert admin.main(["create-invite", "--days", "0"], settings=invite_settings) == 1
    assert admin.main(["create-invite", "--email", "a@example.com", "--count", "2"], settings=invite_settings) == 1
    register(client, "ada@example.com", make_invite(invite_settings, capsys)).raise_for_status()
    assert admin.main(["create-invite", "--email", "ada@example.com"], settings=invite_settings) == 1
    capsys.readouterr()
    assert admin.main(["create-invite", "--count", "3"], settings=invite_settings) == 0
    assert len([line for line in capsys.readouterr().out.splitlines() if line.startswith("  ")]) == 3


def test_open_registration_needs_no_invite(settings):
    settings.open_registration = True
    app = create_app(settings)
    with TestClient(app) as c:
        c.portal.call(reset_schema, app.state.engine)
        assert c.get("/api/v1/server-info").json()["registration"] == "open"
        assert register(c, "ada@example.com").status_code == 201


@pytest.mark.asyncio
async def test_one_invite_two_simultaneous_sign_ups(invite_settings, capsys):
    app = create_app(invite_settings)
    await reset_schema(app.state.engine)
    try:
        code = await asyncio.to_thread(make_invite, invite_settings, capsys)
        async with httpx.AsyncClient(transport=httpx.ASGITransport(app=app), base_url="http://test") as ac:
            results = await asyncio.gather(
                *(
                    ac.post(REGISTER, json={"email": f"user{i}@example.com", "password": PASSWORD, "invite_code": code})
                    for i in range(5)
                )
            )
        assert sorted(r.status_code for r in results) == [201, 403, 403, 403, 403]
    finally:
        await app.state.engine.dispose()
