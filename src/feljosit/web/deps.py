"""FastAPI dependencies: database session, current user and CSRF protection."""

from __future__ import annotations

from collections.abc import Iterator
from dataclasses import dataclass
from typing import Annotated
from urllib.parse import urlsplit

from fastapi import Depends, Request
from sqlalchemy.orm import Session as DbSession

from feljosit.config import Settings
from feljosit.models import Session, User
from feljosit.services import users

SESSION_COOKIE = "feljosit_session"


class LoginRequiredError(Exception):
    """The request needs a logged-in user."""


class CsrfError(Exception):
    """The CSRF token or the Origin header is not correct."""


def get_settings(request: Request) -> Settings:
    settings: Settings = request.app.state.settings
    return settings


def get_db(request: Request) -> Iterator[DbSession]:
    db: DbSession = request.app.state.session_factory()
    try:
        yield db
        db.commit()
    except Exception:
        db.rollback()
        raise
    finally:
        db.close()


Db = Annotated[DbSession, Depends(get_db)]


@dataclass
class Auth:
    """The logged-in user and the login session."""

    user: User
    session: Session
    db: DbSession

    def flash(self, message: str, *, error: bool = False) -> None:
        self.session.flash_message = message[:500]
        self.session.flash_is_error = error

    def pop_flash(self) -> tuple[str, bool] | None:
        if not self.session.flash_message:
            return None
        message = (self.session.flash_message, self.session.flash_is_error)
        self.session.flash_message = None
        self.session.flash_is_error = False
        return message


def get_auth(request: Request, db: Db) -> Auth:
    token = request.cookies.get(SESSION_COOKIE, "")
    session = users.load_session(db, token)
    if session is None:
        raise LoginRequiredError
    return Auth(user=session.user, session=session, db=db)


def origin_ok(request: Request) -> bool:
    """Compare the Origin (or Referer) header with the Host header."""
    origin = request.headers.get("origin") or request.headers.get("referer")
    if origin is None:
        # Browsers send Origin for all POST requests. Non-browser clients have
        # no ambient credentials, thus the CSRF token is sufficient.
        return True
    if origin == "null":
        return False
    return urlsplit(origin).netloc.lower() == request.headers.get("host", "").lower()


async def verify_csrf(request: Request, auth: Annotated[Auth, Depends(get_auth)]) -> Auth:
    form = await request.form()
    token = form.get("csrf_token")
    if not origin_ok(request) or not users.csrf_matches(
        auth.session.csrf_token, token if isinstance(token, str) else None
    ):
        raise CsrfError
    return auth


CurrentAuth = Annotated[Auth, Depends(get_auth)]
CsrfAuth = Annotated[Auth, Depends(verify_csrf)]
AppSettings = Annotated[Settings, Depends(get_settings)]
