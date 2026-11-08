FROM python:3.12-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

WORKDIR /app

COPY pyproject.toml README.md ./
COPY reqora_server ./reqora_server
RUN pip install .

COPY alembic.ini ./
COPY migrations ./migrations

RUN useradd --system --uid 10001 --home-dir /app reqora \
    && mkdir /data \
    && chown reqora /data
USER reqora

# SQLite in a volume by default; set REQORA_DATABASE_URL to use PostgreSQL instead.
ENV REQORA_DATABASE_URL=sqlite+aiosqlite:////data/reqora.db
VOLUME /data
EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s \
    CMD python -c "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8000/health', timeout=4)"

# Migrations first, then the server. Behind a reverse proxy, set FORWARDED_ALLOW_IPS
# to the proxy's address so rate limiting sees real client IPs.
CMD ["sh", "-c", "alembic upgrade head && exec uvicorn reqora_server.main:create_app --factory --host 0.0.0.0 --port 8000 --proxy-headers"]
