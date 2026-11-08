"""Document sync: push local changes, pull everything changed since a cursor.

Each user has a revision counter (`users.rev`). Every applied change stores the
document with the next revision, so "changes since N" is `rev > N`. Pushes for
one user are serialized by locking the user row first; that keeps conflict checks
consistent and makes revisions commit in order, so a pull never sees revision 11
while 10 is still in flight.
"""

import json
from typing import Annotated

from fastapi import APIRouter, HTTPException, Query, status
from sqlalchemy import func, select, update
from sqlalchemy.ext.asyncio import AsyncSession

from ..db import utcnow
from ..deps import DB, AppSettings, CurrentAuth
from ..models import Document, User
from ..schemas import ChangeIn, ChangeResult, DocumentOut, PullOut, PushIn, PushOut

router = APIRouter(prefix="/sync", tags=["sync"])

PULL_MAX_LIMIT = 1000


def _to_out(doc: Document) -> DocumentOut:
    return DocumentOut(
        kind=doc.kind,
        id=doc.doc_id,
        rev=doc.rev,
        deleted=doc.deleted,
        data=None if doc.data is None else json.loads(doc.data),
        updated_at=doc.updated_at,
    )


def _serialize(change: ChangeIn) -> str | None:
    if change.deleted:
        return None
    try:
        return json.dumps(change.data, ensure_ascii=False, separators=(",", ":"), allow_nan=False)
    except ValueError:
        raise HTTPException(422, f"{change.kind}/{change.id}: numbers must be finite") from None


async def _lock_user(db: AsyncSession, user_id: int) -> int:
    """Take the per-user write lock and return the user's current revision.

    A no-op UPDATE row-locks the user in PostgreSQL and takes the database write
    lock in SQLite; either way it is held until commit.
    """
    await db.execute(
        update(User).where(User.id == user_id).values(rev=User.rev).execution_options(synchronize_session=False)
    )
    return await db.scalar(select(User.rev).where(User.id == user_id))


@router.post("/push")
async def push(body: PushIn, auth: CurrentAuth, db: DB, settings: AppSettings) -> PushOut:
    """Apply local changes.

    A change applies when `base_rev` matches the server's revision of the document
    (null for a document the server doesn't have, or has only as a tombstone),
    when its content already equals the server copy, or when `force` is set.
    Otherwise it is a conflict and the server copy comes back in `current`.
    A document over the size limit is rejected (`error` says why).
    Changes are independent: a conflict or rejection doesn't stop the others.
    """
    if len(body.changes) > settings.max_changes_per_push:
        raise HTTPException(422, f"At most {settings.max_changes_per_push} changes per push")
    keys = [(c.kind, c.id) for c in body.changes]
    if len(set(keys)) != len(keys):
        raise HTTPException(422, "A document appears more than once in one push")
    texts = [_serialize(c) for c in body.changes]
    if not body.changes:
        return PushOut(results=[])

    uid = auth.user.id
    rev = await _lock_user(db, uid)
    rows = await db.scalars(
        select(Document).where(Document.user_id == uid, Document.doc_id.in_(sorted({c.id for c in body.changes})))
    )
    existing = {(d.kind, d.doc_id): d for d in rows}

    now = utcnow()
    results: list[ChangeResult] = []
    for change, text in zip(body.changes, texts, strict=True):
        if text is not None and len(text.encode()) > settings.max_document_bytes:
            error = f"Larger than the server's {settings.max_document_bytes}-byte document limit"
            results.append(ChangeResult(kind=change.kind, id=change.id, status="rejected", rev=None, error=error))
            continue
        doc = existing.get((change.kind, change.id))
        if doc is not None and doc.deleted == change.deleted and doc.data == text:
            # Already in this state (e.g. the same edit made on two devices): nothing to do.
            results.append(ChangeResult(kind=change.kind, id=change.id, status="applied", rev=doc.rev))
            continue
        base_ok = (doc is None and change.base_rev is None) or (
            doc is not None and (change.base_rev == doc.rev or (doc.deleted and change.base_rev is None))
        )
        if not base_ok and not change.force:
            current = _to_out(doc) if doc is not None else None
            results.append(ChangeResult(kind=change.kind, id=change.id, status="conflict", rev=None, current=current))
            continue
        rev += 1
        if doc is None:
            doc = Document(user_id=uid, kind=change.kind, doc_id=change.id)
            db.add(doc)
            existing[(change.kind, change.id)] = doc
        doc.rev = rev
        doc.deleted = change.deleted
        doc.data = text
        doc.updated_at = now
        results.append(ChangeResult(kind=change.kind, id=change.id, status="applied", rev=rev))

    await db.execute(update(User).where(User.id == uid).values(rev=rev).execution_options(synchronize_session=False))
    await db.commit()
    return PushOut(results=results)


@router.get("/changes", responses={409: {"description": "`since` is ahead of the server; pull again from 0"}})
async def pull(
    auth: CurrentAuth,
    db: DB,
    settings: AppSettings,
    since: Annotated[int, Query(ge=0, description="Cursor from the previous pull; 0 for a full download.")] = 0,
    limit: Annotated[int, Query(ge=1, le=PULL_MAX_LIMIT)] = 500,
) -> PullOut:
    """Documents changed after `since`, oldest first.

    Includes the caller's own pushes; skip a document whose `rev` you already have.
    With `since=0` deleted documents are left out, since a fresh device has nothing
    to delete. Keep pulling while `has_more` is true.
    """
    uid = auth.user.id
    user_rev = await db.scalar(select(User.rev).where(User.id == uid))
    if since > user_rev:
        # The server lost data (restored from a backup, or a different server at
        # the same address): the client's cursor no longer means anything here.
        raise HTTPException(status.HTTP_409_CONFLICT, "Cursor is ahead of the server; pull again with since=0")

    where = [Document.user_id == uid, Document.rev > since]
    if since == 0:
        where.append(Document.deleted.is_(False))

    # Pick the page by size first so a page of huge documents is never loaded whole.
    sizes = (
        await db.execute(
            select(Document.rev, func.coalesce(func.length(Document.data), 0))
            .where(*where)
            .order_by(Document.rev)
            .limit(limit + 1)
        )
    ).all()
    has_more = len(sizes) > limit
    budget, last_rev = 0, None
    for i, (doc_rev, size) in enumerate(sizes[:limit]):
        if i > 0 and budget + size > settings.pull_max_bytes:
            has_more = True
            break
        budget += size
        last_rev = doc_rev

    if last_rev is None:
        docs = []
    else:
        docs = list(await db.scalars(select(Document).where(*where, Document.rev <= last_rev).order_by(Document.rev)))

    # A document can move to a newer revision between the two queries; it then just
    # shows up in a later page. Without more pages, everything up to `user_rev` has
    # been seen (it was committed before `user_rev` was read).
    cursor = last_rev if has_more else max(user_rev, last_rev or 0)
    return PullOut(documents=[_to_out(d) for d in docs], cursor=cursor, has_more=has_more)
