import asyncio
import io

from fastapi.testclient import TestClient

from reqora_server import admin
from reqora_server.db import make_engine
from reqora_server.main import create_app

from .conftest import PASSWORD, reset_schema

API = "/api/v1"


def run_admin(settings, monkeypatch, *argv, stdin=""):
    monkeypatch.setattr("sys.stdin", io.StringIO(stdin))
    return admin.main(list(argv), settings=settings)


def login(client, email, password):
    return client.post(f"{API}/auth/login", json={"email": email, "password": password})


def test_admin_commands(settings, monkeypatch, capsys):
    engine = make_engine(settings.database_url)
    asyncio.run(reset_schema(engine))
    asyncio.run(engine.dispose())

    assert (
        run_admin(
            settings,
            monkeypatch,
            "create-user",
            "Ada@Example.com",
            "--name",
            "Ada",
            "--password-stdin",
            stdin=PASSWORD + "\n",
        )
        == 0
    )
    assert run_admin(settings, monkeypatch, "create-user", "ada@example.com", "--password-stdin", stdin=PASSWORD) == 1
    assert "already has an account" in capsys.readouterr().err
    assert run_admin(settings, monkeypatch, "create-user", "not-an-email", "--password-stdin", stdin=PASSWORD) == 1
    assert run_admin(settings, monkeypatch, "create-user", "b@example.com", "--password-stdin", stdin="short") == 1

    app = create_app(settings)
    with TestClient(app) as client:
        token = login(client, "ada@example.com", PASSWORD).json()["token"]
        client.post(
            f"{API}/sync/push",
            json={"changes": [{"kind": "collection", "id": "c1", "data": {"name": "x"}}]},
            headers={"Authorization": f"Bearer {token}"},
        )

        capsys.readouterr()
        assert run_admin(settings, monkeypatch, "list-users") == 0
        out = capsys.readouterr().out
        assert "ada@example.com" in out and "Ada" in out

        # A new password signs out every device.
        assert (
            run_admin(
                settings,
                monkeypatch,
                "set-password",
                "ada@example.com",
                "--password-stdin",
                stdin="brand new password\n",
            )
            == 0
        )
        assert "1 device(s) signed out" in capsys.readouterr().out
        assert client.get(f"{API}/me", headers={"Authorization": f"Bearer {token}"}).status_code == 401
        assert login(client, "ada@example.com", PASSWORD).status_code == 401
        assert login(client, "ada@example.com", "brand new password").status_code == 200

        assert run_admin(settings, monkeypatch, "delete-user", "ada@example.com", "--yes") == 0
        assert login(client, "ada@example.com", "brand new password").status_code == 401
        assert run_admin(settings, monkeypatch, "delete-user", "ada@example.com", "--yes") == 1
