"""Account administration from the command line.

    python -m reqora_server.admin create-invite [--email EMAIL] [--note TEXT] [--days N] [--count N]
    python -m reqora_server.admin list-invites [--all]
    python -m reqora_server.admin revoke-invite ID
    python -m reqora_server.admin list-users
    python -m reqora_server.admin create-user EMAIL [--name NAME]
    python -m reqora_server.admin set-password EMAIL
    python -m reqora_server.admin delete-user EMAIL [--yes]

Passwords are asked for interactively, or read from one line of stdin with
`--password-stdin`. In Docker: `docker compose exec server python -m reqora_server.admin ...`.
"""

import argparse
import asyncio
import getpass
import sys
from datetime import timedelta

from pydantic import validate_email
from pydantic_core import PydanticCustomError
from sqlalchemy import delete, func, select
from sqlalchemy.ext.asyncio import AsyncSession

from .config import Settings
from .db import make_engine, utcnow
from .models import AuthSession, Document, Invite, User
from .security import hash_password, hash_token, new_invite_code, normalize_invite_code

MIN_PASSWORD = 8


class AdminError(Exception):
    pass


def read_password(from_stdin: bool) -> str:
    if from_stdin:
        password = sys.stdin.readline().rstrip("\r\n")
    else:
        password = getpass.getpass("New password: ")
        if getpass.getpass("Repeat it: ") != password:
            raise AdminError("The passwords don't match")
    if len(password) < MIN_PASSWORD:
        raise AdminError(f"Use at least {MIN_PASSWORD} characters")
    return password


async def find_user(db: AsyncSession, email: str) -> User:
    user = await db.scalar(select(User).where(User.email == email.strip().lower()))
    if user is None:
        raise AdminError(f"No account with email {email}")
    return user


async def list_users(db: AsyncSession) -> None:
    docs = (
        select(Document.user_id, func.count().label("n")).where(Document.deleted.is_(False)).group_by(Document.user_id)
    )
    devices = select(AuthSession.user_id, func.count().label("n")).where(AuthSession.expires_at > utcnow())
    docs, devices = docs.subquery(), devices.group_by(AuthSession.user_id).subquery()
    rows = await db.execute(
        select(User.email, User.display_name, User.created_at, docs.c.n, devices.c.n)
        .outerjoin(docs, docs.c.user_id == User.id)
        .outerjoin(devices, devices.c.user_id == User.id)
        .order_by(User.created_at)
    )
    print(f"{'EMAIL':<36} {'NAME':<24} {'CREATED':<17} {'DOCS':>6} {'DEVICES':>8}")
    for email, name, created, n_docs, n_devices in rows:
        print(f"{email:<36} {name:<24} {created:%Y-%m-%d %H:%M} {n_docs or 0:>6} {n_devices or 0:>8}")


def _clean_email(email: str) -> str:
    try:
        return validate_email(email.strip())[1].lower()
    except PydanticCustomError as e:
        raise AdminError(f"Invalid email: {e}") from None


async def create_user(db: AsyncSession, email: str, name: str, password: str) -> None:
    email = _clean_email(email)
    if await db.scalar(select(User.id).where(User.email == email)) is not None:
        raise AdminError(f"{email} already has an account")
    db.add(
        User(email=email, display_name=name.strip(), password_hash=hash_password(password), rev=0, created_at=utcnow())
    )
    await db.commit()
    print(f"Created {email}")


async def set_password(db: AsyncSession, email: str, password: str) -> None:
    user = await find_user(db, email)
    user.password_hash = hash_password(password)
    result = await db.execute(delete(AuthSession).where(AuthSession.user_id == user.id))
    await db.commit()
    print(f"Password changed for {user.email}; {result.rowcount} device(s) signed out")


async def delete_user(db: AsyncSession, email: str) -> None:
    user = await find_user(db, email)
    await db.execute(delete(Document).where(Document.user_id == user.id))
    await db.execute(delete(AuthSession).where(AuthSession.user_id == user.id))
    await db.execute(delete(User).where(User.id == user.id))
    await db.commit()
    print(f"Deleted {user.email} and all of its data")


async def create_invites(db: AsyncSession, email: str | None, note: str, days: int, count: int) -> list[str]:
    if days < 1 or count < 1:
        raise AdminError("--days and --count must be at least 1")
    if email is not None:
        email = _clean_email(email)
        if count > 1:
            raise AdminError("An invite tied to an email can only be created one at a time")
        if await db.scalar(select(User.id).where(User.email == email)) is not None:
            raise AdminError(f"{email} already has an account")
    now = utcnow()
    expires = now + timedelta(days=days)
    codes = [new_invite_code() for _ in range(count)]
    for code in codes:
        db.add(
            Invite(
                code_hash=hash_token(normalize_invite_code(code)),
                email=email,
                note=note.strip(),
                created_at=now,
                expires_at=expires,
            )
        )
    await db.commit()
    what = "Invite code" if count == 1 else f"{count} invite codes"
    who = f" for {email}" if email else ""
    print(f"{what}{who}, valid until {expires:%Y-%m-%d %H:%M} UTC:")
    for code in codes:
        print(f"  {code}")
    print("Each code works once. In Reqora: ⚙ → Sync & account… → Create account, then enter the code.")
    print("Only a hash of each code is stored, so they can't be shown again.")
    return codes


async def list_invites(db: AsyncSession, show_all: bool) -> None:
    now = utcnow()
    query = select(Invite).order_by(Invite.created_at)
    if not show_all:
        query = query.where(Invite.used_at.is_(None), Invite.expires_at > now)
    invites = list(await db.scalars(query))
    if not invites:
        print("No invites." if show_all else "No open invites.")
        return
    print(f"{'ID':>4} {'FOR':<30} {'NOTE':<24} {'EXPIRES (UTC)':<17} STATUS")
    for i in invites:
        if i.used_at is not None:
            status = f"used by {i.used_by_email} on {i.used_at:%Y-%m-%d}"
        elif i.expires_at <= now:
            status = "expired"
        else:
            status = "open"
        print(f"{i.id:>4} {i.email or 'anyone':<30} {i.note:<24} {i.expires_at:%Y-%m-%d %H:%M} {status}")


async def revoke_invite(db: AsyncSession, invite_id: int) -> None:
    result = await db.execute(delete(Invite).where(Invite.id == invite_id, Invite.used_at.is_(None)))
    await db.commit()
    if result.rowcount == 0:
        raise AdminError(f"No unused invite with id {invite_id}")
    print(f"Invite {invite_id} revoked")


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="python -m reqora_server.admin", description="Manage Reqora sync accounts.")
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("create-invite", help="Create single-use sign-up codes")
    p.add_argument("--email", help="Only this email can use the code")
    p.add_argument("--note", default="", help="Who it's for, shown by list-invites")
    p.add_argument("--days", type=int, help="Days the code stays valid (default: REQORA_INVITE_TTL_DAYS, 7)")
    p.add_argument("--count", type=int, default=1, help="Number of codes to create")
    p = sub.add_parser("list-invites", help="List open invites")
    p.add_argument("--all", action="store_true", help="Include used and expired ones")
    p = sub.add_parser("revoke-invite", help="Invalidate an unused invite")
    p.add_argument("id", type=int, help="ID from list-invites")
    sub.add_parser("list-users", help="List accounts")
    p = sub.add_parser("create-user", help="Create an account directly, with a password you choose")
    p.add_argument("email")
    p.add_argument("--name", default="", help="Display name")
    p.add_argument("--password-stdin", action="store_true", help="Read the password from stdin")
    p = sub.add_parser("set-password", help="Set a new password and sign out all of the account's devices")
    p.add_argument("email")
    p.add_argument("--password-stdin", action="store_true", help="Read the password from stdin")
    p = sub.add_parser("delete-user", help="Delete an account and all of its synced data")
    p.add_argument("email")
    p.add_argument("--yes", action="store_true", help="Don't ask for confirmation")
    return parser.parse_args(argv)


async def run(args: argparse.Namespace, settings: Settings, password: str) -> None:
    engine = make_engine(settings.database_url)
    try:
        async with AsyncSession(engine, expire_on_commit=False) as db:
            if args.command == "create-invite":
                await create_invites(
                    db, args.email, args.note, settings.invite_ttl_days if args.days is None else args.days, args.count
                )
            elif args.command == "list-invites":
                await list_invites(db, args.all)
            elif args.command == "revoke-invite":
                await revoke_invite(db, args.id)
            elif args.command == "list-users":
                await list_users(db)
            elif args.command == "create-user":
                await create_user(db, args.email, args.name, password)
            elif args.command == "set-password":
                await set_password(db, args.email, password)
            elif args.command == "delete-user":
                await delete_user(db, args.email)
    finally:
        await engine.dispose()


def main(argv: list[str] | None = None, settings: Settings | None = None) -> int:
    args = parse_args(argv)
    try:
        # Ask everything up front, before the event loop starts.
        password = read_password(args.password_stdin) if args.command in ("create-user", "set-password") else ""
        if args.command == "delete-user" and not args.yes:
            if input(f"Delete {args.email} and all of its data? Type 'yes': ").strip().lower() != "yes":
                raise AdminError("Cancelled")
        asyncio.run(run(args, settings or Settings(), password))
    except AdminError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
