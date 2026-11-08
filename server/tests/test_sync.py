import asyncio

import httpx
import pytest

from reqora_server.main import create_app

from .conftest import PASSWORD, bearer, register, reset_schema

PUSH = "/api/v1/sync/push"
PULL = "/api/v1/sync/changes"


def change(id, data=None, *, kind="request", base_rev=None, deleted=False, force=False):
    c = {"kind": kind, "id": id, "base_rev": base_rev, "deleted": deleted, "force": force}
    if not deleted:
        c["data"] = data if data is not None else {"name": id}
    return c


def push(client, headers, *changes):
    r = client.post(PUSH, json={"changes": list(changes)}, headers=headers)
    assert r.status_code == 200, r.text
    return r.json()["results"]


def pull(client, headers, since=0, **params):
    r = client.get(PULL, params={"since": since, **params}, headers=headers)
    assert r.status_code == 200, r.text
    return r.json()


def test_push_then_pull_on_another_device(client, user):
    laptop = user["headers"]
    desktop = bearer(
        client.post("/api/v1/auth/login", json={"email": user["user"]["email"], "password": PASSWORD}).json()["token"]
    )
    coll = {"name": "Users API", "variables": [{"key": "base_url", "value": "https://api.example.com"}]}
    results = push(client, laptop, change("c1", coll, kind="collection"), change("r1", {"method": "GET", "url": "/x"}))
    assert [(r["status"], r["rev"]) for r in results] == [("applied", 1), ("applied", 2)]

    page = pull(client, desktop)
    assert page["cursor"] == 2 and not page["has_more"]
    docs = {(d["kind"], d["id"]): d for d in page["documents"]}
    assert docs[("collection", "c1")]["data"] == coll
    assert docs[("collection", "c1")]["rev"] == 1
    assert docs[("request", "r1")]["data"] == {"method": "GET", "url": "/x"}

    assert pull(client, desktop, since=2)["documents"] == []


def test_update_with_matching_base_rev(client, user):
    h = user["headers"]
    [created] = push(client, h, change("r1", {"v": 1}))
    [updated] = push(client, h, change("r1", {"v": 2}, base_rev=created["rev"]))
    assert updated == {"kind": "request", "id": "r1", "status": "applied", "rev": 2, "current": None, "error": None}
    [doc] = pull(client, h, since=1)["documents"]
    assert doc["data"] == {"v": 2} and doc["rev"] == 2


def test_stale_base_rev_is_a_conflict(client, user):
    h = user["headers"]
    push(client, h, change("r1", {"v": 1}))
    push(client, h, change("r1", {"v": 2}, base_rev=1))  # another device got there first
    [res] = push(client, h, change("r1", {"v": 3}, base_rev=1))
    assert res["status"] == "conflict"
    assert res["rev"] is None
    assert res["current"]["rev"] == 2 and res["current"]["data"] == {"v": 2}
    # Re-based on the server copy, it goes through.
    [res] = push(client, h, change("r1", {"v": 3}, base_rev=2))
    assert res["status"] == "applied" and res["rev"] == 3


def test_creating_an_existing_document_is_a_conflict(client, user):
    h = user["headers"]
    push(client, h, change("r1", {"v": 1}))
    [res] = push(client, h, change("r1", {"v": 2}))
    assert res["status"] == "conflict"
    [res] = push(client, h, change("missing", {"v": 1}, base_rev=7))
    assert res["status"] == "conflict" and res["current"] is None


def test_force_overwrites(client, user):
    h = user["headers"]
    push(client, h, change("r1", {"v": 1}))
    push(client, h, change("r1", {"v": 2}, base_rev=1))
    [res] = push(client, h, change("r1", {"v": "mine"}, base_rev=1, force=True))
    assert res["status"] == "applied" and res["rev"] == 3


def test_identical_content_is_not_a_conflict(client, user):
    h = user["headers"]
    push(client, h, change("r1", {"v": 1}))
    push(client, h, change("r1", {"v": 2}, base_rev=1))
    # Same edit made on a device that still had rev 1: nothing to resolve, no new revision.
    [res] = push(client, h, change("r1", {"v": 2}, base_rev=1))
    assert res == {"kind": "request", "id": "r1", "status": "applied", "rev": 2, "current": None, "error": None}
    assert pull(client, h, since=2)["documents"] == []


def test_delete_leaves_a_tombstone(client, user):
    h = user["headers"]
    push(client, h, change("r1"), change("r2"))
    [res] = push(client, h, change("r1", base_rev=1, deleted=True))
    assert res["status"] == "applied" and res["rev"] == 3

    incremental = pull(client, h, since=2)["documents"]
    assert incremental == [
        {
            "kind": "request",
            "id": "r1",
            "rev": 3,
            "deleted": True,
            "data": None,
            "updated_at": incremental[0]["updated_at"],
        }
    ]
    # A fresh device doesn't need tombstones.
    full = pull(client, h)
    assert [d["id"] for d in full["documents"]] == ["r2"]
    assert full["cursor"] == 3

    # Editing a document someone deleted is a conflict; recreating it on top of the tombstone works.
    [res] = push(client, h, change("r1", {"v": 9}, base_rev=1))
    assert res["status"] == "conflict" and res["current"]["deleted"] is True
    [res] = push(client, h, change("r1", {"v": 9}))
    assert res["status"] == "applied" and res["rev"] == 4


def test_kinds_are_separate_namespaces(client, user):
    h = user["headers"]
    results = push(client, h, change("same", {"a": 1}, kind="collection"), change("same", {"b": 2}, kind="request"))
    assert [r["status"] for r in results] == ["applied", "applied"]
    [res] = push(client, h, change("same", {"a": 2}, kind="collection", base_rev=1))
    assert res["status"] == "applied"


def test_one_conflict_does_not_block_other_changes(client, user):
    h = user["headers"]
    push(client, h, change("r1"))
    results = push(client, h, change("r1", {"x": 1}), change("r2"), change("r3"))
    assert [r["status"] for r in results] == ["conflict", "applied", "applied"]
    assert [r["rev"] for r in results[1:]] == [2, 3]


def test_users_do_not_see_each_others_documents(client, user):
    other = register(client)
    other_h = bearer(other["token"])
    push(client, user["headers"], change("r1", {"secret": True}))
    assert pull(client, other_h)["documents"] == []
    # Same document id in another account is unrelated.
    [res] = push(client, other_h, change("r1", {"mine": True}))
    assert res["status"] == "applied" and res["rev"] == 1
    [doc] = pull(client, user["headers"])["documents"]
    assert doc["data"] == {"secret": True}


def test_pull_pagination(client, user):
    h = user["headers"]
    push(client, h, *[change(f"r{i}") for i in range(7)])
    seen, cursor, pages = [], 0, 0
    while True:
        page = pull(client, h, since=cursor, limit=3)
        seen += [d["id"] for d in page["documents"]]
        cursor = page["cursor"]
        pages += 1
        if not page["has_more"]:
            break
    assert seen == [f"r{i}" for i in range(7)]
    assert cursor == 7 and pages == 3


def test_pull_page_respects_byte_budget(client, user):
    client.app.state.settings.pull_max_bytes = 2500
    h = user["headers"]
    push(client, h, *[change(f"r{i}", {"blob": "x" * 1000}) for i in range(5)])
    page = pull(client, h)
    assert len(page["documents"]) == 2 and page["has_more"]
    # A single document over the budget still comes through on its own.
    client.app.state.settings.pull_max_bytes = 10
    page = pull(client, h, since=page["cursor"])
    assert len(page["documents"]) == 1 and page["has_more"]


def test_cursor_ahead_of_server_asks_for_full_resync(client, user):
    push(client, user["headers"], change("r1"))
    r = client.get(PULL, params={"since": 50}, headers=user["headers"])
    assert r.status_code == 409


def test_push_validation(client, user):
    h = user["headers"]

    def status_of(*changes):
        return client.post(PUSH, json={"changes": list(changes)}, headers=h).status_code

    assert status_of(change("r1"), change("r1")) == 422  # same document twice
    assert status_of({"kind": "request", "id": "r1"}) == 422  # no data, not deleted
    assert status_of({"kind": "request", "id": "r1", "deleted": True, "data": {}}) == 422
    assert status_of(change("r1", kind="Bad Kind")) == 422
    assert status_of(change("has spaces")) == 422
    assert status_of(change("x" * 129)) == 422
    assert status_of({**change("r1"), "data": [1, 2]}) == 422  # must be an object
    assert status_of({**change("r1"), "baseRev": 1}) == 422  # unknown field
    assert status_of(change("r1", base_rev=0)) == 422
    assert client.post(PUSH, json={"changes": []}, headers=h).json() == {"results": []}


def test_push_size_limits(client, user):
    settings = client.app.state.settings
    settings.max_document_bytes = 100
    settings.max_changes_per_push = 3
    h = user["headers"]
    # An oversized document is rejected on its own; the rest of the push goes through.
    results = push(client, h, change("r1", {"blob": "x" * 200}), change("r2", {"small": True}))
    assert results[0]["status"] == "rejected" and results[0]["rev"] is None
    assert "100-byte" in results[0]["error"]
    assert results[1]["status"] == "applied" and results[1]["rev"] == 1
    assert [d["id"] for d in pull(client, h)["documents"]] == ["r2"]

    many = client.post(PUSH, json={"changes": [change(f"r{i}") for i in range(4)]}, headers=h)
    assert many.status_code == 422


def test_request_body_limit(tmp_path, settings):
    settings.max_request_bytes = 1000
    from fastapi.testclient import TestClient

    app = create_app(settings)
    with TestClient(app) as c:
        c.portal.call(reset_schema, app.state.engine)
        h = bearer(register(c)["token"])
        r = c.post(PUSH, json={"changes": [change("r1", {"blob": "x" * 2000})]}, headers=h)
        assert r.status_code == 413

        def chunks():  # no Content-Length: the limit applies while reading the stream
            yield b'{"changes": [{"kind": "request", "id": "r1", "data": {"blob": "'
            yield b"x" * 2000
            yield b'"}}]}'

        r = c.post(PUSH, content=chunks(), headers={**h, "Content-Type": "application/json"})
        assert r.status_code == 413


def test_unicode_and_nested_data_round_trip(client, user):
    data = {
        "name": "Grüße – wörld ✓",
        "body": {"mode": "raw", "raw": '{"a": [1, 2.5, null, true]}'},
        "headers": [{"key": "X-Ä", "value": "日本"}],
        "big": 12345678901234567890,
    }
    push(client, user["headers"], change("r1", data))
    [doc] = pull(client, user["headers"])["documents"]
    assert doc["data"] == data


@pytest.mark.asyncio
async def test_concurrent_pushes_are_serialized(settings):
    app = create_app(settings)
    await reset_schema(app.state.engine)
    transport = httpx.ASGITransport(app=app)
    try:
        async with httpx.AsyncClient(transport=transport, base_url="http://test") as ac:
            r = await ac.post("/api/v1/auth/register", json={"email": "race@example.com", "password": PASSWORD})
            h = bearer(r.json()["token"])
            await ac.post(PUSH, json={"changes": [change("doc", {"v": 0})]}, headers=h)

            # Ten devices edit the same document from the same base: exactly one wins.
            async def edit(i):
                r = await ac.post(PUSH, json={"changes": [change("doc", {"v": i}, base_rev=1)]}, headers=h)
                assert r.status_code == 200, r.text
                return r.json()["results"][0]

            results = await asyncio.gather(*(edit(i) for i in range(1, 11)))
            assert sorted(r["status"] for r in results) == ["applied"] + ["conflict"] * 9

            # Ten devices create different documents at once: revisions stay unique and gap-free.
            async def create(i):
                r = await ac.post(PUSH, json={"changes": [change(f"new{i}"), change(f"new{i}b")]}, headers=h)
                assert r.status_code == 200, r.text
                return [x["rev"] for x in r.json()["results"]]

            revs = sorted(rev for pair in await asyncio.gather(*(create(i) for i in range(10))) for rev in pair)
            assert revs == list(range(3, 23))

            page = (await ac.get(PULL, params={"since": 0}, headers=h)).json()
            assert page["cursor"] == 22
            assert len(page["documents"]) == 21
    finally:
        await app.state.engine.dispose()
