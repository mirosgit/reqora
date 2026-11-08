"""Request and response bodies of the HTTP API."""

from datetime import datetime
from typing import Annotated, Any, Literal

from pydantic import BaseModel, ConfigDict, EmailStr, Field, field_validator, model_validator

KIND_PATTERN = r"^[a-z][a-z0-9_]{0,31}$"
DOC_ID_PATTERN = r"^[A-Za-z0-9._:\-]{1,128}$"

Password = Annotated[str, Field(min_length=8, max_length=256)]


class _In(BaseModel):
    model_config = ConfigDict(extra="forbid")


def _clean_name(value: str) -> str:
    return " ".join(value.split())


# --- Accounts ---------------------------------------------------------------


class RegisterIn(_In):
    email: EmailStr
    password: Password
    display_name: str = Field(default="", max_length=100)
    device_name: str = Field(default="", max_length=100)
    invite_code: str = Field(default="", max_length=64, description="Required unless the server has open sign-up.")

    _clean = field_validator("display_name", "device_name")(_clean_name)


class LoginIn(_In):
    email: str = Field(max_length=320)
    password: str = Field(max_length=256)
    device_name: str = Field(default="", max_length=100)

    _clean = field_validator("device_name")(_clean_name)


class UserOut(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    email: str
    display_name: str
    created_at: datetime


class TokenOut(BaseModel):
    token: str
    token_type: Literal["bearer"] = "bearer"
    session_id: str
    expires_at: datetime
    user: UserOut


class SessionOut(BaseModel):
    id: str
    device_name: str
    created_at: datetime
    last_used_at: datetime
    expires_at: datetime
    current: bool


class UpdateMeIn(_In):
    display_name: str | None = Field(default=None, max_length=100)

    _clean = field_validator("display_name")(lambda v: None if v is None else _clean_name(v))


class ChangePasswordIn(_In):
    current_password: str = Field(max_length=256)
    new_password: Password


class DeleteAccountIn(_In):
    password: str = Field(max_length=256)


# --- Sync -------------------------------------------------------------------


class ChangeIn(_In):
    kind: str = Field(pattern=KIND_PATTERN, description="Document type, e.g. `collection` or `request`.")
    id: str = Field(pattern=DOC_ID_PATTERN, description="Client-generated document id (a UUID in Reqora).")
    base_rev: int | None = Field(
        default=None,
        ge=1,
        description="Revision this change was made on top of; null for a document the client created.",
    )
    deleted: bool = False
    data: dict[str, Any] | None = Field(default=None, description="The whole document; omit when `deleted` is true.")
    force: bool = Field(default=False, description="Apply even if the server copy changed since `base_rev`.")

    @model_validator(mode="after")
    def _data_matches_deleted(self) -> "ChangeIn":
        if self.deleted and self.data is not None:
            raise ValueError("a deleted document must not carry data")
        if not self.deleted and self.data is None:
            raise ValueError("data is required unless deleted is true")
        return self


class PushIn(_In):
    changes: list[ChangeIn]


class DocumentOut(BaseModel):
    kind: str
    id: str
    rev: int
    deleted: bool
    data: dict[str, Any] | None
    updated_at: datetime


class ChangeResult(BaseModel):
    kind: str
    id: str
    status: Literal["applied", "conflict", "rejected"]
    rev: int | None = Field(description="Revision of the stored document after an applied change.")
    current: DocumentOut | None = Field(
        default=None, description="On conflict: the server's copy (null if the server has none)."
    )
    error: str | None = Field(default=None, description="On rejection: why the change was not accepted.")


class PushOut(BaseModel):
    results: list[ChangeResult]


class ServerInfoOut(BaseModel):
    name: Literal["reqora-sync"] = "reqora-sync"
    version: str
    registration: Literal["invite", "open"] = Field(
        description="`invite`: signing up needs an invite code from the server's administrator."
    )
    max_document_bytes: int
    max_changes_per_push: int
    max_request_bytes: int


class PullOut(BaseModel):
    documents: list[DocumentOut]
    cursor: int = Field(description="Pass as `since` on the next pull.")
    has_more: bool
