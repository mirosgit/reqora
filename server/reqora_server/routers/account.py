"""The signed-in user's profile, password and account deletion."""

from fastapi import APIRouter, HTTPException, Response, status
from fastapi.concurrency import run_in_threadpool
from sqlalchemy import delete, update

from ..deps import DB, CurrentAuth
from ..models import AuthSession, Document, User
from ..schemas import ChangePasswordIn, DeleteAccountIn, UpdateMeIn, UserOut
from ..security import hash_password, verify_password

router = APIRouter(prefix="/me", tags=["account"])


async def _check_password(user: User, password: str) -> None:
    if not await run_in_threadpool(verify_password, user.password_hash, password):
        raise HTTPException(status.HTTP_403_FORBIDDEN, "Wrong password")


@router.get("")
async def get_me(auth: CurrentAuth) -> UserOut:
    return UserOut.model_validate(auth.user)


@router.patch("")
async def update_me(body: UpdateMeIn, auth: CurrentAuth, db: DB) -> UserOut:
    if body.display_name is not None:
        auth.user.display_name = body.display_name
        await db.commit()
    return UserOut.model_validate(auth.user)


@router.post("/password", status_code=status.HTTP_204_NO_CONTENT, responses={403: {"description": "Wrong password"}})
async def change_password(body: ChangePasswordIn, auth: CurrentAuth, db: DB) -> Response:
    """Change the password and log out every other device."""
    await _check_password(auth.user, body.current_password)
    new_hash = await run_in_threadpool(hash_password, body.new_password)
    await db.execute(update(User).where(User.id == auth.user.id).values(password_hash=new_hash))
    await db.execute(delete(AuthSession).where(AuthSession.user_id == auth.user.id, AuthSession.id != auth.session.id))
    await db.commit()
    return Response(status_code=status.HTTP_204_NO_CONTENT)


@router.delete("", status_code=status.HTTP_204_NO_CONTENT, responses={403: {"description": "Wrong password"}})
async def delete_account(body: DeleteAccountIn, auth: CurrentAuth, db: DB) -> Response:
    """Delete the account together with all of its synced data. Cannot be undone."""
    await _check_password(auth.user, body.password)
    uid = auth.user.id
    await db.execute(delete(Document).where(Document.user_id == uid))
    await db.execute(delete(AuthSession).where(AuthSession.user_id == uid))
    await db.execute(delete(User).where(User.id == uid))
    await db.commit()
    return Response(status_code=status.HTTP_204_NO_CONTENT)
