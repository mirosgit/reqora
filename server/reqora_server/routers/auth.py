"""Registration, login and per-device sessions."""

import uuid
from datetime import timedelta

from fastapi import APIRouter, Depends, HTTPException, Request, Response, status
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import delete, select, update
from sqlalchemy.exc import IntegrityError

from ..config import Settings
from ..db import utcnow
from ..deps import DB, AppSettings, CurrentAuth, auth_rate_limit, unauthorized
from ..models import AuthSession, Invite, User
from ..schemas import LoginIn, RegisterIn, SessionOut, TokenOut, UserOut
from ..security import (
    hash_password,
    hash_token,
    new_token,
    normalize_invite_code,
    password_needs_rehash,
    verify_password,
)

router = APIRouter(prefix="/auth", tags=["auth"])

INVITE_NEEDED = "This server is invite-only: ask its administrator for an invite code"
# Deliberately the same for every way a code can be wrong, so codes can't be probed.
INVITE_INVALID = "This invite code is invalid, expired or already used"


def _device_name(given: str, request: Request) -> str:
    return given or request.headers.get("user-agent", "")[:100]


def _open_session(db: DB, user: User, device_name: str, settings: Settings) -> TokenOut:
    token = new_token()
    now = utcnow()
    session = AuthSession(
        id=str(uuid.uuid4()),
        user_id=user.id,
        token_hash=hash_token(token),
        device_name=device_name,
        created_at=now,
        last_used_at=now,
        expires_at=now + timedelta(days=settings.session_ttl_days),
    )
    db.add(session)
    return TokenOut(
        token=token, session_id=session.id, expires_at=session.expires_at, user=UserOut.model_validate(user)
    )


@router.post(
    "/register",
    status_code=status.HTTP_201_CREATED,
    dependencies=[Depends(auth_rate_limit)],
    responses={
        403: {"description": "Invite code missing or invalid"},
        409: {"description": "Email already registered"},
    },
)
async def register(body: RegisterIn, request: Request, db: DB, settings: AppSettings) -> TokenOut:
    """Create an account. Unless the server has open sign-up, this needs an invite code
    (`python -m reqora_server.admin create-invite` on the server)."""
    email = body.email.strip().lower()
    invite = None
    if not settings.open_registration:
        code = normalize_invite_code(body.invite_code)
        if not code:
            raise HTTPException(status.HTTP_403_FORBIDDEN, INVITE_NEEDED)
        invite = await db.scalar(select(Invite).where(Invite.code_hash == hash_token(code)))
        if (
            invite is None
            or invite.used_at is not None
            or invite.expires_at <= utcnow()
            or (invite.email is not None and invite.email != email)
        ):
            raise HTTPException(status.HTTP_403_FORBIDDEN, INVITE_INVALID)
    if await db.scalar(select(User.id).where(User.email == email)) is not None:
        raise HTTPException(status.HTTP_409_CONFLICT, "An account with this email already exists")
    user = User(
        email=email,
        display_name=body.display_name,
        password_hash=await run_in_threadpool(hash_password, body.password),
        rev=0,
        created_at=utcnow(),
    )
    db.add(user)
    try:
        await db.flush()
    except IntegrityError:  # registered concurrently
        await db.rollback()
        raise HTTPException(status.HTTP_409_CONFLICT, "An account with this email already exists") from None
    if invite is not None:
        # Claim the code in the same transaction; only one of two concurrent sign-ups gets it.
        claimed = await db.execute(
            update(Invite)
            .where(Invite.id == invite.id, Invite.used_at.is_(None))
            .values(used_at=utcnow(), used_by_email=email)
            .execution_options(synchronize_session=False)
        )
        if claimed.rowcount != 1:
            await db.rollback()
            raise HTTPException(status.HTTP_403_FORBIDDEN, INVITE_INVALID)
    out = _open_session(db, user, _device_name(body.device_name, request), settings)
    await db.commit()
    return out


@router.post("/login", dependencies=[Depends(auth_rate_limit)], responses={401: {"description": "Wrong credentials"}})
async def login(body: LoginIn, request: Request, db: DB, settings: AppSettings) -> TokenOut:
    email = body.email.strip().lower()
    user = await db.scalar(select(User).where(User.email == email))
    ok = await run_in_threadpool(verify_password, user.password_hash if user else None, body.password)
    if user is None or not ok:
        raise unauthorized("Wrong email or password")
    if password_needs_rehash(user.password_hash):
        user.password_hash = await run_in_threadpool(hash_password, body.password)
    await db.execute(delete(AuthSession).where(AuthSession.user_id == user.id, AuthSession.expires_at <= utcnow()))
    out = _open_session(db, user, _device_name(body.device_name, request), settings)
    await db.commit()
    return out


@router.post("/logout", status_code=status.HTTP_204_NO_CONTENT)
async def logout(auth: CurrentAuth, db: DB) -> Response:
    """Revoke the token used for this request."""
    await db.execute(delete(AuthSession).where(AuthSession.id == auth.session.id))
    await db.commit()
    return Response(status_code=status.HTTP_204_NO_CONTENT)


@router.get("/sessions")
async def list_sessions(auth: CurrentAuth, db: DB) -> list[SessionOut]:
    """Devices currently logged in to this account, most recently used first."""
    rows = await db.scalars(
        select(AuthSession)
        .where(AuthSession.user_id == auth.user.id, AuthSession.expires_at > utcnow())
        .order_by(AuthSession.last_used_at.desc())
    )
    return [
        SessionOut(
            id=s.id,
            device_name=s.device_name,
            created_at=s.created_at,
            last_used_at=s.last_used_at,
            expires_at=s.expires_at,
            current=s.id == auth.session.id,
        )
        for s in rows
    ]


@router.delete("/sessions/{session_id}", status_code=status.HTTP_204_NO_CONTENT, responses={404: {}})
async def revoke_session(session_id: str, auth: CurrentAuth, db: DB) -> Response:
    """Log out one device."""
    result = await db.execute(
        delete(AuthSession).where(AuthSession.id == session_id, AuthSession.user_id == auth.user.id)
    )
    if result.rowcount == 0:
        raise HTTPException(status.HTTP_404_NOT_FOUND, "Session not found")
    await db.commit()
    return Response(status_code=status.HTTP_204_NO_CONTENT)
