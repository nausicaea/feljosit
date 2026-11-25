"""Users, passwords and login sessions."""

from __future__ import annotations

import hashlib
import hmac
import re
import secrets
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerificationError, VerifyMismatchError
from sqlalchemy import delete, select
from sqlalchemy.orm import Session as DbSession

from feljosit.models import Session, User
from feljosit.services.defaults import create_default_categories
from feljosit.services.errors import ServiceError

MIN_PASSWORD_LENGTH = 12
MAX_PASSWORD_LENGTH = 1024
MAX_FAILED_LOGINS = 5
LOCKOUT = timedelta(minutes=15)
_USERNAME_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{1,63}$")

# Argon2id with the RFC 9106 "second recommended" parameters (64 MiB, t=3, p=4).
_hasher = PasswordHasher(time_cost=3, memory_cost=64 * 1024, parallelism=4)
_DUMMY_HASH = _hasher.hash(secrets.token_urlsafe(16))


def _now() -> datetime:
    return datetime.now(UTC)


def hash_token(token: str) -> str:
    return hashlib.sha256(token.encode()).hexdigest()


def validate_password(password: str) -> None:
    if len(password) < MIN_PASSWORD_LENGTH:
        raise ServiceError(f"The password must have at least {MIN_PASSWORD_LENGTH} characters.")
    if len(password) > MAX_PASSWORD_LENGTH:
        raise ServiceError("The password is too long.")


def create_user(db: DbSession, username: str, password: str) -> User:
    username = username.strip().lower()
    if not _USERNAME_RE.fullmatch(username):
        raise ServiceError("The username must have 2 to 64 characters: a-z, 0-9, '.', '_' or '-'.")
    validate_password(password)
    if db.scalar(select(User).where(User.username == username)) is not None:
        raise ServiceError("The username exists already.")
    user = User(username=username, password_hash=_hasher.hash(password))
    db.add(user)
    db.flush()
    create_default_categories(db, user.id)
    return user


def change_password(db: DbSession, user: User, new_password: str) -> None:
    validate_password(new_password)
    user.password_hash = _hasher.hash(new_password)
    # Log out all sessions of the user.
    db.execute(delete(Session).where(Session.user_id == user.id))


def authenticate(db: DbSession, username: str, password: str) -> User | None:
    """Return the user if the credentials are correct.

    The function always calculates one Argon2 hash. Thus the response time does not
    show if the username exists. After :data:`MAX_FAILED_LOGINS` failures, the account
    is locked for :data:`LOCKOUT`.
    """
    if len(password) > MAX_PASSWORD_LENGTH:
        return None
    user = db.scalar(select(User).where(User.username == username.strip().lower()))
    if user is None:
        _verify(_DUMMY_HASH, password)
        return None
    if user.locked_until is not None and user.locked_until > _now():
        _verify(_DUMMY_HASH, password)
        return None
    if not _verify(user.password_hash, password):
        user.failed_logins += 1
        if user.failed_logins >= MAX_FAILED_LOGINS:
            user.locked_until = _now() + LOCKOUT
            user.failed_logins = 0
        return None
    user.failed_logins = 0
    user.locked_until = None
    if _hasher.check_needs_rehash(user.password_hash):
        user.password_hash = _hasher.hash(password)
    return user


def _verify(password_hash: str, password: str) -> bool:
    try:
        return _hasher.verify(password_hash, password)
    except (VerifyMismatchError, VerificationError, InvalidHashError):
        return False


@dataclass(frozen=True)
class NewSession:
    token: str
    csrf_token: str
    expires_at: datetime


def start_session(db: DbSession, user: User, lifetime_seconds: int) -> NewSession:
    token = secrets.token_urlsafe(32)
    csrf = secrets.token_urlsafe(32)
    expires = _now() + timedelta(seconds=lifetime_seconds)
    db.add(
        Session(token_hash=hash_token(token), user_id=user.id, csrf_token=csrf, expires_at=expires)
    )
    db.execute(delete(Session).where(Session.expires_at < _now()))
    return NewSession(token=token, csrf_token=csrf, expires_at=expires)


def load_session(db: DbSession, token: str) -> Session | None:
    if not token or len(token) > 128:
        return None
    session = db.get(Session, hash_token(token))
    if session is None or session.expires_at <= _now():
        return None
    return session


def end_session(db: DbSession, token: str) -> None:
    db.execute(delete(Session).where(Session.token_hash == hash_token(token)))


def csrf_matches(expected: str, received: str | None) -> bool:
    return received is not None and hmac.compare_digest(expected, received)
