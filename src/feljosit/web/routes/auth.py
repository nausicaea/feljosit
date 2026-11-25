"""Login and logout."""

from __future__ import annotations

import secrets
from typing import Annotated

from fastapi import APIRouter, Form, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response

from feljosit.services import users
from feljosit.services.errors import ServiceError
from feljosit.web.deps import SESSION_COOKIE, AppSettings, CsrfAuth, CurrentAuth, Db, origin_ok
from feljosit.web.rendering import render

router = APIRouter()
LOGIN_CSRF_COOKIE = "feljosit_login_csrf"


def _login_page(request: Request, error: str | None = None, status_code: int = 200) -> Response:
    token = secrets.token_urlsafe(32)
    response = render(request, "login.html", login_csrf=token, error=error, status_code=status_code)
    response.set_cookie(
        LOGIN_CSRF_COOKIE,
        token,
        httponly=True,
        samesite="strict",
        secure=request.app.state.settings.secure_cookies,
        max_age=3600,
        path="/login",
    )
    return response


@router.get("/login", response_class=HTMLResponse)
def login_form(request: Request) -> Response:
    return _login_page(request)


@router.post("/login")
def login(
    request: Request,
    db: Db,
    settings: AppSettings,
    username: Annotated[str, Form(max_length=64)],
    password: Annotated[str, Form(max_length=1024)],
    csrf_token: Annotated[str, Form(max_length=128)],
) -> Response:
    cookie = request.cookies.get(LOGIN_CSRF_COOKIE)
    if not origin_ok(request) or not users.csrf_matches(cookie or "", csrf_token) or not cookie:
        return _login_page(request, "The form expired. Try again.", 400)
    user = users.authenticate(db, username, password)
    if user is None:
        db.commit()  # Keep the failed-login counter.
        return _login_page(request, "The username or the password is not correct.", 401)
    new = users.start_session(db, user, settings.session_lifetime_seconds)
    response = RedirectResponse("/budget", status_code=303)
    response.set_cookie(
        SESSION_COOKIE,
        new.token,
        httponly=True,
        samesite="strict",
        secure=settings.secure_cookies,
        max_age=settings.session_lifetime_seconds,
    )
    response.delete_cookie(LOGIN_CSRF_COOKIE, path="/login")
    return response


@router.post("/logout")
def logout(request: Request, auth: CsrfAuth) -> Response:
    users.end_session(auth.db, request.cookies.get(SESSION_COOKIE, ""))
    response = RedirectResponse("/login", status_code=303)
    response.delete_cookie(SESSION_COOKIE)
    return response


@router.get("/settings", response_class=HTMLResponse)
def settings_page(request: Request, auth: CurrentAuth) -> Response:
    return render(request, "settings.html", auth)


@router.post("/settings/password")
def change_password(
    request: Request,
    auth: CsrfAuth,
    settings: AppSettings,
    current_password: Annotated[str, Form(max_length=1024)],
    new_password: Annotated[str, Form(max_length=1024)],
    new_password_repeat: Annotated[str, Form(max_length=1024)],
) -> Response:
    if users.authenticate(auth.db, auth.user.username, current_password) is None:
        auth.flash("The current password is not correct.", error=True)
        return RedirectResponse("/settings", status_code=303)
    if new_password != new_password_repeat:
        auth.flash("The two new passwords are not the same.", error=True)
        return RedirectResponse("/settings", status_code=303)
    try:
        users.change_password(auth.db, auth.user, new_password)
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse("/settings", status_code=303)
    new = users.start_session(auth.db, auth.user, settings.session_lifetime_seconds)
    auth.db.flush()
    response = RedirectResponse("/settings", status_code=303)
    response.set_cookie(
        SESSION_COOKIE,
        new.token,
        httponly=True,
        samesite="strict",
        secure=settings.secure_cookies,
        max_age=settings.session_lifetime_seconds,
    )
    session = users.load_session(auth.db, new.token)
    if session is not None:
        session.flash_message = "The password is changed. All other sessions are closed."
    return response
