from datetime import UTC, datetime, timedelta

from fastapi.testclient import TestClient
from sqlalchemy import update

from reqora_server.config import Settings
from reqora_server.main import create_app
from reqora_server.models import AuthSession

from .conftest import PASSWORD, bearer, register, reset_schema

API = "/api/v1"


def login(client, email, password=PASSWORD, **extra):
    return client.post(f"{API}/auth/login", json={"email": email, "password": password, **extra})


def test_register_returns_token_and_profile(client):
    out = register(client, "Ada@Example.com", display_name="  Ada   Lovelace ")
    assert out["token"].startswith("rqs_")
    assert out["token_type"] == "bearer"
    assert out["user"]["email"] == "ada@example.com"
    assert out["user"]["display_name"] == "Ada Lovelace"
    me = client.get(f"{API}/me", headers=bearer(out["token"]))
    assert me.status_code == 200
    assert me.json() == out["user"]


def test_register_rejects_duplicates_and_bad_input(client):
    register(client, "ada@example.com")
    dup = client.post(f"{API}/auth/register", json={"email": "ADA@example.com", "password": PASSWORD})
    assert dup.status_code == 409
    assert client.post(f"{API}/auth/register", json={"email": "nope", "password": PASSWORD}).status_code == 422
    assert client.post(f"{API}/auth/register", json={"email": "b@example.com", "password": "short"}).status_code == 422
    extra = client.post(f"{API}/auth/register", json={"email": "c@example.com", "password": PASSWORD, "admin": True})
    assert extra.status_code == 422


def test_login(client):
    register(client, "ada@example.com")
    ok = login(client, " ADA@example.com ", device_name="Laptop")
    assert ok.status_code == 200
    assert client.get(f"{API}/me", headers=bearer(ok.json()["token"])).status_code == 200

    wrong = login(client, "ada@example.com", "wrong password")
    unknown = login(client, "nobody@example.com")
    for r in (wrong, unknown):
        assert r.status_code == 401
        assert r.json()["detail"] == "Wrong email or password"
        assert r.headers["www-authenticate"] == "Bearer"


def test_requests_without_valid_token_are_rejected(client):
    assert client.get(f"{API}/me").status_code == 401
    assert client.get(f"{API}/me", headers=bearer("rqs_made-up")).status_code == 401
    assert client.get(f"{API}/me", headers={"Authorization": "Basic Zm9vOmJhcg=="}).status_code == 401
    assert client.get(f"{API}/sync/changes").status_code == 401


def test_sessions_list_and_revoke(client, user):
    other = login(client, user["user"]["email"], device_name="Desktop").json()
    sessions = client.get(f"{API}/auth/sessions", headers=user["headers"]).json()
    assert len(sessions) == 2
    current = [s for s in sessions if s["current"]]
    assert [s["id"] for s in current] == [user["session_id"]]
    assert {s["device_name"] for s in sessions} >= {"Desktop"}

    assert client.delete(f"{API}/auth/sessions/{other['session_id']}", headers=user["headers"]).status_code == 204
    assert client.get(f"{API}/me", headers=bearer(other["token"])).status_code == 401
    assert client.delete(f"{API}/auth/sessions/{other['session_id']}", headers=user["headers"]).status_code == 404


def test_cannot_revoke_someone_elses_session(client, user):
    stranger = register(client)
    r = client.delete(f"{API}/auth/sessions/{stranger['session_id']}", headers=user["headers"])
    assert r.status_code == 404
    assert client.get(f"{API}/me", headers=bearer(stranger["token"])).status_code == 200


def test_logout_revokes_only_that_token(client, user):
    other = login(client, user["user"]["email"]).json()
    assert client.post(f"{API}/auth/logout", headers=user["headers"]).status_code == 204
    assert client.get(f"{API}/me", headers=user["headers"]).status_code == 401
    assert client.get(f"{API}/me", headers=bearer(other["token"])).status_code == 200


def test_expired_session_is_rejected(client, user):
    async def expire():
        async with client.app.state.sessionmaker() as db:
            past = datetime.now(UTC) - timedelta(seconds=1)
            await db.execute(update(AuthSession).values(expires_at=past))
            await db.commit()

    client.portal.call(expire)
    r = client.get(f"{API}/me", headers=user["headers"])
    assert r.status_code == 401
    assert r.json()["detail"] == "Session expired"


def test_session_use_extends_expiry(client, user):
    async def age_session():
        async with client.app.state.sessionmaker() as db:
            long_ago = datetime.now(UTC) - timedelta(days=10)
            await db.execute(
                update(AuthSession).values(last_used_at=long_ago, expires_at=long_ago + timedelta(days=90))
            )
            await db.commit()

    client.portal.call(age_session)
    client.get(f"{API}/me", headers=user["headers"])
    [session] = client.get(f"{API}/auth/sessions", headers=user["headers"]).json()
    expires = datetime.fromisoformat(session["expires_at"])
    assert expires > datetime.now(UTC) + timedelta(days=89)


def test_update_profile(client, user):
    r = client.patch(f"{API}/me", json={"display_name": "Grace"}, headers=user["headers"])
    assert r.status_code == 200
    assert r.json()["display_name"] == "Grace"
    assert client.get(f"{API}/me", headers=user["headers"]).json()["display_name"] == "Grace"
    assert client.patch(f"{API}/me", json={}, headers=user["headers"]).json()["display_name"] == "Grace"


def test_change_password_logs_out_other_devices(client, user):
    email = user["user"]["email"]
    other = login(client, email).json()
    bad = client.post(
        f"{API}/me/password",
        json={"current_password": "nope nope", "new_password": "a new password"},
        headers=user["headers"],
    )
    assert bad.status_code == 403
    r = client.post(
        f"{API}/me/password",
        json={"current_password": PASSWORD, "new_password": "a new password"},
        headers=user["headers"],
    )
    assert r.status_code == 204
    assert client.get(f"{API}/me", headers=user["headers"]).status_code == 200
    assert client.get(f"{API}/me", headers=bearer(other["token"])).status_code == 401
    assert login(client, email).status_code == 401
    assert login(client, email, "a new password").status_code == 200


def test_delete_account_removes_everything(client, user):
    email = user["user"]["email"]
    client.post(
        f"{API}/sync/push",
        json={"changes": [{"kind": "collection", "id": "c1", "data": {"name": "x"}}]},
        headers=user["headers"],
    )
    assert (
        client.request("DELETE", f"{API}/me", json={"password": "wrong pass"}, headers=user["headers"]).status_code
        == 403
    )
    assert (
        client.request("DELETE", f"{API}/me", json={"password": PASSWORD}, headers=user["headers"]).status_code == 204
    )
    assert client.get(f"{API}/me", headers=user["headers"]).status_code == 401
    assert login(client, email).status_code == 401
    # The email can be used again, and the new account starts empty.
    fresh = register(client, email)
    pulled = client.get(f"{API}/sync/changes", headers=bearer(fresh["token"])).json()
    assert pulled["documents"] == []


def test_login_is_rate_limited(tmp_path):
    settings = Settings(_env_file=None, database_url=f"sqlite+aiosqlite:///{tmp_path / 'x.db'}", auth_rate_limit=3)
    app = create_app(settings)
    with TestClient(app) as c:
        c.portal.call(reset_schema, app.state.engine)
        codes = [login(c, "a@example.com").status_code for _ in range(4)]
        assert codes == [401, 401, 401, 429]
        r = login(c, "a@example.com")
        assert r.status_code == 429
        assert int(r.headers["retry-after"]) >= 1


def test_server_info(client):
    r = client.get(f"{API}/server-info")
    assert r.status_code == 200
    info = r.json()
    assert info["name"] == "reqora-sync"
    assert info["registration"] == "open"
    assert info["max_document_bytes"] == client.app.state.settings.max_document_bytes


def test_health(client):
    r = client.get("/health")
    assert r.status_code == 200
    assert r.json()["status"] == "ok"
