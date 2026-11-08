"""Server configuration, read from `REQORA_*` environment variables (or a `.env` file)."""

from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_prefix="REQORA_", env_file=".env", extra="ignore")

    # SQLAlchemy async URL. SQLite is fine for a small self-hosted install;
    # use PostgreSQL (postgresql+asyncpg://...) for anything bigger.
    database_url: str = "sqlite+aiosqlite:///./reqora.db"

    # By default signing up needs an invite code, created on the server with
    # `python -m reqora_server.admin create-invite`. True lets anyone who can reach
    # the server create an account (only sensible on a trusted network).
    open_registration: bool = False

    # How long a new invite code stays valid, unless given on the command line.
    invite_ttl_days: int = 7

    # A session (one per logged-in device) expires after this many days without use.
    session_ttl_days: int = 90

    # Login/registration attempts allowed per client IP per minute. 0 disables the limit.
    auth_rate_limit: int = 10

    # Largest accepted request body, and the largest single synced document (serialized JSON).
    max_request_bytes: int = 32 * 1024 * 1024
    max_document_bytes: int = 1024 * 1024

    # Most changes accepted in one push.
    max_changes_per_push: int = 500

    # Soft cap on the document payload returned by one pull page (at least one document is always returned).
    pull_max_bytes: int = 16 * 1024 * 1024
