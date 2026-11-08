"""In-memory sliding-window rate limiter.

It is per process: with several workers each one counts separately, so the
effective limit is `limit * workers`. Good enough to slow down password guessing.
"""

import time
from collections import deque


class RateLimiter:
    MAX_KEYS = 10_000

    def __init__(self, limit: int, window_seconds: float = 60.0) -> None:
        self.limit = limit
        self.window = window_seconds
        self._hits: dict[str, deque[float]] = {}

    def hit(self, key: str) -> float | None:
        """Record an attempt. Returns None if allowed, otherwise seconds until the next one is."""
        if self.limit <= 0:
            return None
        now = time.monotonic()
        hits = self._hits.setdefault(key, deque())
        while hits and hits[0] <= now - self.window:
            hits.popleft()
        if len(hits) >= self.limit:
            return hits[0] + self.window - now
        hits.append(now)
        if len(self._hits) > self.MAX_KEYS:
            self._prune(now)
        return None

    def _prune(self, now: float) -> None:
        stale = [k for k, hits in self._hits.items() if not hits or hits[-1] <= now - self.window]
        for k in stale:
            del self._hits[k]
