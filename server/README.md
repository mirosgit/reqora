# Reqora Sync Server

Accounts and multi-device sync for [Reqora](../README.md), built with FastAPI and SQLAlchemy.
Users sign up with an email and password. Each device they log in on gets its own session token,
and their collections, requests, environments, history and settings stay in sync across devices.

- **Login**: email + password (Argon2id), one revocable token per device, sliding 90-day expiry,
  per-IP rate limiting on login and sign-up
- **Invite-only sign-up** by default: single-use codes, optionally tied to an email, created by whoever
  has access to the server (`python -m reqora_server.admin create-invite`)
- **Sync**: the client pushes changed documents and pulls whatever changed since its last cursor.
  Optimistic concurrency per document with conflict detection, deletions as tombstones, paged pulls
- **Storage**: SQLite for a small self-hosted install, PostgreSQL for anything bigger.
  Schema migrations with Alembic
- **Admin tool**: create and revoke invites, create accounts, reset passwords, delete accounts

**To put it on a server, follow [DEPLOY.md](DEPLOY.md).** It covers a VPS with Docker Compose and automatic
HTTPS (Caddy), backups, updates, an existing nginx, a home network without a domain, and running without Docker.

In the app, users pick the server under **⚙ → Sync & account…** and sign in there.

## Running

### Locally

Requires Python 3.11+.

```bash
cd server
python3 -m venv .venv
.venv/bin/pip install -e ".[dev]"
.venv/bin/alembic upgrade head                     # creates ./reqora.db
.venv/bin/uvicorn reqora_server.main:create_app --factory --reload
.venv/bin/python -m reqora_server.admin create-invite      # a code to create the first account with
```

Interactive API docs: <http://127.0.0.1:8000/docs>.

### Docker (SQLite)

```bash
docker build -t reqora-server server
docker run -d -p 127.0.0.1:8000:8000 -v reqora-data:/data reqora-server
```

The container applies migrations on start and keeps the database in the `/data` volume.

### Docker Compose (PostgreSQL + HTTPS)

`docker-compose.yml` is the production setup: PostgreSQL, the server, and Caddy getting a Let's Encrypt
certificate for your domain. See [DEPLOY.md](DEPLOY.md).

```bash
cd server
cp .env.example .env      # set REQORA_DOMAIN and POSTGRES_PASSWORD
docker compose up -d
docker compose exec server python -m reqora_server.admin create-invite --email you@example.com
```

### In production

- Put it behind an HTTPS reverse proxy (Caddy, nginx, ...). Tokens and synced data,
  including saved passwords and API keys in requests and environments, travel in plain JSON,
  so TLS is required. Data is stored unencrypted in the database.
- Set `FORWARDED_ALLOW_IPS` to the proxy's address so the server sees real client IPs.
  Otherwise everyone shares the proxy's rate-limit bucket.
- The rate limiter keeps its counts in memory, per process. With `--workers N`, each worker counts separately.
- Back up the database. If a backup is restored, clients notice their cursor is ahead of the server
  (`409` on pull) and download everything again.

## Configuration

Environment variables (a `.env` file in the working directory also works):

| Variable | Default | Meaning |
|---|---|---|
| `REQORA_DATABASE_URL` | `sqlite+aiosqlite:///./reqora.db` | SQLAlchemy async URL, e.g. `postgresql+asyncpg://user:pass@host/db` |
| `REQORA_OPEN_REGISTRATION` | `false` | `false`: signing up needs an invite code. `true`: anyone who can reach the server can sign up |
| `REQORA_INVITE_TTL_DAYS` | `7` | How long a new invite code is valid (`create-invite --days` overrides it) |
| `REQORA_SESSION_TTL_DAYS` | `90` | A device is logged out after this many days without use |
| `REQORA_AUTH_RATE_LIMIT` | `10` | Login/sign-up attempts per IP per minute (`0` turns it off) |
| `REQORA_MAX_REQUEST_BYTES` | `33554432` | Largest request body (32 MiB) |
| `REQORA_MAX_DOCUMENT_BYTES` | `1048576` | Largest single synced document (1 MiB of JSON) |
| `REQORA_MAX_CHANGES_PER_PUSH` | `500` | Most changes in one push |
| `REQORA_PULL_MAX_BYTES` | `16777216` | Soft cap on one pull page (16 MiB) |

## API

All endpoints are under `/api/v1` except `/health`. Authenticated endpoints take
`Authorization: Bearer <token>`. Errors come back as `{"detail": "..."}`.

| Method | Path | Body | Result |
|---|---|---|---|
| `POST` | `/auth/register` | `email`, `password` (8+ chars), `display_name`?, `device_name`?, `invite_code` (unless open sign-up) | `201` token (below); `409` email taken; `403` invite code missing, invalid, expired or used |
| `POST` | `/auth/login` | `email`, `password`, `device_name`? | token; `401` wrong credentials; `429` rate-limited |
| `POST` | `/auth/logout` | | `204`, revokes the token used |
| `GET` | `/auth/sessions` | | logged-in devices: `id`, `device_name`, `created_at`, `last_used_at`, `expires_at`, `current` |
| `DELETE` | `/auth/sessions/{id}` | | `204`, logs that device out |
| `GET` | `/me` | | `id`, `email`, `display_name`, `created_at` |
| `PATCH` | `/me` | `display_name` | updated profile |
| `POST` | `/me/password` | `current_password`, `new_password` | `204`, logs out every other device; `403` wrong password |
| `DELETE` | `/me` | `password` | `204`, deletes the account and all its data |
| `POST` | `/sync/push` | `changes: [...]` | per-change results (see below) |
| `GET` | `/sync/changes?since=N&limit=500` | | documents changed after `N` |
| `GET` | `/server-info` | | public: `name` (`"reqora-sync"`), `version`, `registration` (`"invite"` or `"open"`), `max_document_bytes`, `max_changes_per_push`, `max_request_bytes` |
| `GET` | `/health` | | `200 {"status": "ok"}`, or `503` if the database is unreachable |

Login and sign-up return:

```json
{
  "token": "rqs_…",
  "token_type": "bearer",
  "session_id": "5f0c…",
  "expires_at": "2027-02-06T12:00:00Z",
  "user": {"id": 1, "email": "ada@example.com", "display_name": "Ada", "created_at": "…"}
}
```

If `device_name` is omitted, the `User-Agent` is used, so a client should send something like
`"Reqora on Linux (laptop)"`. The device list shows it.

## Sync protocol

The server stores **documents**: JSON objects identified by `(kind, id)`, private to each user.
It doesn't interpret their contents; the client decides what the kinds are. `kind` is a lowercase slug
(`collection`, `request`, ...) and `id` is the client's id (Reqora uses UUIDs).

Each user has a revision counter. Every applied change gets the next number, so a document's `rev`
says both "which version" and "when, relative to every other change of this user".

### Push

```http
POST /api/v1/sync/push
```

```json
{
  "changes": [
    {"kind": "request", "id": "8d1e…", "base_rev": 14, "data": {"name": "List users", "method": "GET"}},
    {"kind": "request", "id": "77ab…", "base_rev": null, "data": {"name": "New one"}},
    {"kind": "environment", "id": "c2f0…", "base_rev": 9, "deleted": true}
  ]
}
```

`base_rev` is the revision the client's edit was made on top of: the last `rev` it received for that
document, or `null` for a document it created. A change is **applied** when:

- `base_rev` equals the server's current `rev`, or
- the server has no such document (or only a tombstone) and `base_rev` is `null`, or
- the new content is identical to what the server already has (no new revision is created), or
- `force` is `true` (overwrite whatever is there).

A document larger than `REQORA_MAX_DOCUMENT_BYTES` is **rejected** (`"status": "rejected"`, with the reason
in `error`). Otherwise it's a **conflict**, and the server's copy comes back so the client can resolve it right away:

```json
{
  "results": [
    {"kind": "request", "id": "8d1e…", "status": "conflict", "rev": null,
     "current": {"kind": "request", "id": "8d1e…", "rev": 17, "deleted": false, "data": {…}, "updated_at": "…"}},
    {"kind": "request", "id": "77ab…", "status": "applied", "rev": 18, "current": null},
    {"kind": "environment", "id": "c2f0…", "status": "applied", "rev": 19, "current": null}
  ]
}
```

Results are in the same order as the changes, and each change succeeds or fails on its own.
Every result also has an `error` field, which is `null` unless the change was rejected.
A deletion leaves a tombstone (`deleted: true`, `data: null`) so other devices find out about it.

### Pull

```http
GET /api/v1/sync/changes?since=12
```

```json
{
  "documents": [{"kind": "request", "id": "8d1e…", "rev": 17, "deleted": false, "data": {…}, "updated_at": "…"}],
  "cursor": 19,
  "has_more": false
}
```

Documents come oldest first. Store `cursor` and send it as `since` next time; keep pulling while
`has_more` is `true`. `since=0` downloads everything and leaves tombstones out. The response
includes the client's own pushes; skip any document whose `rev` the client already has.
A `409` means the cursor is ahead of the server (it was restored from a backup or replaced):
throw the cursor away and pull from `0`.

### Client algorithm

Reqora implements this in `src/sync.rs`, the HTTP calls included.

Keep, per account: `cursor`, and for every document its last known `rev` plus a "dirty" flag.

1. On login, pull from `since=0` and merge into local data.
2. When the user saves something, mark the affected documents dirty.
3. Sync on save (debounced) and every 30–60 s:
   1. **Push** dirty documents with their `base_rev`. On `applied`, record the new `rev` and clear dirty.
      On `conflict`, resolve with `current`: take the server copy, keep the local one (push again with
      `base_rev = current.rev`), or keep both (save the local copy under a new id).
   2. **Pull** from `cursor` until `has_more` is `false`. For each document: same `rev` as local, skip;
      local copy dirty, treat it as a conflict; otherwise apply it (`deleted` means remove locally).
4. On logout, call `/auth/logout` and drop the token.

### Suggested layout for Reqora data

This is what the client is expected to send; the server doesn't enforce it.

| kind | id | data |
|---|---|---|
| `collection` | collection id | `Collection` without `requests`, plus `request_ids` (order in the sidebar) |
| `request` | request id | `Request` plus `collection_id` |
| `environment` | environment id | `Environment` |
| `history` | entry id | `HistoryEntry`; when the client trims history to its limit, it pushes deletions |
| `settings` | `default` | synced preferences, e.g. `author_name`, `active_environment` |

Requests get their own documents so that editing two requests of one collection on two devices
doesn't conflict. Device-specific state (open tabs in `session.json`, window layout) stays local.

## Development

```bash
.venv/bin/pytest                    # tests on SQLite
.venv/bin/ruff check . && .venv/bin/ruff format --check .

# The same tests on PostgreSQL (tables in that database are dropped and recreated):
docker run -d --rm --name reqora-pg -e POSTGRES_PASSWORD=test -p 127.0.0.1:55432:5432 postgres:17-alpine
REQORA_TEST_DATABASE_URL=postgresql+asyncpg://postgres:test@127.0.0.1:55432/postgres .venv/bin/pytest
```

The client's sync engine has a test that runs two simulated devices against a live server:

```bash
.venv/bin/alembic upgrade head && REQORA_OPEN_REGISTRATION=true .venv/bin/uvicorn reqora_server.main:create_app --factory &
cd .. && REQORA_SYNC_TEST_SERVER=http://127.0.0.1:8000 cargo test two_devices
```

After changing `reqora_server/models.py`, add a migration and check that it matches
(`tests/test_migrations.py` fails until it does):

```bash
.venv/bin/alembic revision --autogenerate -m "describe the change"
```

Layout:

| Path | Contents |
|---|---|
| `reqora_server/main.py` | app factory, body size limit, `/server-info`, `/health` |
| `reqora_server/config.py` | settings from `REQORA_*` environment variables |
| `reqora_server/models.py`, `db.py` | tables, engine, UTC datetime column type |
| `reqora_server/security.py` | password hashing, session tokens |
| `reqora_server/deps.py` | DB session, current user, rate limiting |
| `reqora_server/routers/` | `auth.py`, `account.py`, `sync.py` |
| `reqora_server/admin.py` | command-line account administration |
| `migrations/` | Alembic migrations |
| `docker-compose.yml`, `Caddyfile`, `.env.example` | production deployment (see [DEPLOY.md](DEPLOY.md)) |
