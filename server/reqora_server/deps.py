"""FastAPI dependencies: settings, DB session, the authenticated user, rate limiting."""

import math
from collections.abc import AsyncIterator
from dataclasses import dataclass
from datetime import timedelta
from typing import Annotated

from fastapi import Depends, HTTPException, Request, status
from fastapi.security import HTTPAuthorizationCredentials, HTTPBearer
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from .config import Settings
from .db import utcnow
from .models import AuthSession, User
from .security import hash_token

# Refresh a session's last-use time (and push its expiry forward) at most this often,
# so ordinary requests don't each cost a write.
SESSION_TOUCH_INTERVAL = timedelta(minutes=5)


def get_settings(request: Request) -> Settings:
    return request.app.state.settings


async def get_db(request: Request) -> AsyncIterator[AsyncSession]:
    async with request.app.state.sessionmaker() as session:
        yield session


AppSettings = Annotated[Settings, Depends(get_settings)]
DB = Annotated[AsyncSession, Depends(get_db)]

_bearer = HTTPBearer(auto_error=False, description="Session token from /auth/login or /auth/register.")


def unauthorized(detail: str) -> HTTPException:
    return HTTPException(status.HTTP_401_UNAUTHORIZED, detail, headers={"WWW-Authenticate": "Bearer"})


@dataclass
class Auth:
    user: User
    session: AuthSession


async def get_auth(
    db: DB,
    settings: AppSettings,
    credentials: Annotated[HTTPAuthorizationCredentials | None, Depends(_bearer)],
) -> Auth:
    if credentials is None:
        raise unauthorized("Not authenticated")
    row = (
        await db.execute(
            select(AuthSession, User)
            .join(User, User.id == AuthSession.user_id)
            .where(AuthSession.token_hash == hash_token(credentials.credentials))
        )
    ).first()
    if row is None:
        raise unauthorized("Invalid or revoked token")
    session, user = row
    now = utcnow()
    if session.expires_at <= now:
        await db.delete(session)
        await db.commit()
        raise unauthorized("Session expired")
    if now - session.last_used_at >= SESSION_TOUCH_INTERVAL:
        session.last_used_at = now
        session.expires_at = now + timedelta(days=settings.session_ttl_days)
    # End the transaction here: endpoints that write start a fresh one, which on
    # SQLite means they take the write lock first instead of upgrading a stale read.
    await db.commit()
    return Auth(user=user, session=session)


CurrentAuth = Annotated[Auth, Depends(get_auth)]


def auth_rate_limit(request: Request) -> None:
    key = request.client.host if request.client else "unknown"
    retry_after = request.app.state.auth_limiter.hit(key)
    if retry_after is not None:
        raise HTTPException(
            status.HTTP_429_TOO_MANY_REQUESTS,
            "Too many attempts, try again later",
            headers={"Retry-After": str(max(1, math.ceil(retry_after)))},
        )
