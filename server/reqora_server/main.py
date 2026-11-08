"""Application factory. Run with:

uvicorn reqora_server.main:create_app --factory
"""

from collections.abc import AsyncIterator
from contextlib import asynccontextmanager

from fastapi import FastAPI, HTTPException
from fastapi.responses import JSONResponse
from sqlalchemy import text
from sqlalchemy.ext.asyncio import async_sessionmaker
from starlette.types import ASGIApp, Message, Receive, Scope, Send

from . import __version__
from .config import Settings
from .db import make_engine
from .ratelimit import RateLimiter
from .routers import account, auth, sync
from .schemas import ServerInfoOut

API_PREFIX = "/api/v1"


class BodySizeLimit:
    """Rejects request bodies over `max_bytes` (by Content-Length, or while streaming)."""

    def __init__(self, app: ASGIApp, max_bytes: int) -> None:
        self.app = app
        self.max_bytes = max_bytes

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        for name, value in scope["headers"]:
            if name == b"content-length" and value.isdigit() and int(value) > self.max_bytes:
                await self._too_large()(scope, receive, send)
                return

        received = 0

        async def limited_receive() -> Message:
            nonlocal received
            message = await receive()
            if message["type"] == "http.request":
                received += len(message.get("body", b""))
                if received > self.max_bytes:
                    raise HTTPException(413, "Request body too large")
            return message

        await self.app(scope, limited_receive, send)

    @staticmethod
    def _too_large() -> JSONResponse:
        return JSONResponse({"detail": "Request body too large"}, status_code=413)


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings()
    engine = make_engine(settings.database_url)

    @asynccontextmanager
    async def lifespan(_app: FastAPI) -> AsyncIterator[None]:
        yield
        await engine.dispose()

    app = FastAPI(
        title="Reqora Sync Server",
        version=__version__,
        description="Accounts and multi-device sync for the Reqora API client.",
        lifespan=lifespan,
    )
    app.state.settings = settings
    app.state.engine = engine
    app.state.sessionmaker = async_sessionmaker(engine, expire_on_commit=False)
    app.state.auth_limiter = RateLimiter(settings.auth_rate_limit, window_seconds=60)

    app.add_middleware(BodySizeLimit, max_bytes=settings.max_request_bytes)
    for router in (auth.router, account.router, sync.router):
        app.include_router(router, prefix=API_PREFIX)

    @app.get(f"{API_PREFIX}/server-info", tags=["meta"])
    async def server_info() -> ServerInfoOut:
        """What a client needs before signing in: is this a Reqora server, how sign-up works, what are the limits."""
        return ServerInfoOut(
            version=__version__,
            registration="open" if settings.open_registration else "invite",
            max_document_bytes=settings.max_document_bytes,
            max_changes_per_push=settings.max_changes_per_push,
            max_request_bytes=settings.max_request_bytes,
        )

    @app.get("/health", tags=["meta"])
    async def health() -> JSONResponse:
        try:
            async with engine.connect() as conn:
                await conn.execute(text("SELECT 1"))
        except Exception:
            return JSONResponse({"status": "error", "database": "unreachable"}, status_code=503)
        return JSONResponse({"status": "ok", "version": __version__})

    return app
