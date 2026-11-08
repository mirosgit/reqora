"""Password hashing (Argon2id) and opaque session tokens."""

import hashlib
import secrets

from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerificationError

TOKEN_PREFIX = "rqs_"

_hasher = PasswordHasher()
# Verified against when the account doesn't exist, so a login for an unknown
# email takes as long as one with a wrong password.
_DUMMY_HASH = _hasher.hash("reqora-dummy-password")


def hash_password(password: str) -> str:
    return _hasher.hash(password)


def verify_password(password_hash: str | None, password: str) -> bool:
    try:
        return _hasher.verify(password_hash or _DUMMY_HASH, password) and password_hash is not None
    except (VerificationError, InvalidHashError):
        return False


def password_needs_rehash(password_hash: str) -> bool:
    return _hasher.check_needs_rehash(password_hash)


# Invite codes are typed by hand: no 0/O, 1/I/L or U. 12 characters ≈ 59 bits.
INVITE_ALPHABET = "ABCDEFGHJKMNPQRSTVWXYZ23456789"


def new_invite_code() -> str:
    chars = "".join(secrets.choice(INVITE_ALPHABET) for _ in range(12))
    return "-".join(chars[i : i + 4] for i in range(0, 12, 4))


def normalize_invite_code(code: str) -> str:
    """Case and dashes/spaces don't matter when typing a code."""
    return "".join(c for c in code.upper() if c.isalnum())


def new_token() -> str:
    return TOKEN_PREFIX + secrets.token_urlsafe(32)


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()
