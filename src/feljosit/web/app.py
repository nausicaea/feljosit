"""The FastAPI application factory."""

from __future__ import annotations

import logging
from pathlib import Path

from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import PlainTextResponse, RedirectResponse, Response
from fastapi.staticfiles import StaticFiles
from sqlalchemy import Engine
from starlette.exceptions import HTTPException as StarletteHTTPException

from feljosit.camt053 import MIN_EXPAT_VERSION, expat_is_patched
from feljosit.config import Settings
from feljosit.db import make_engine, make_session_factory, migrate
from feljosit.services.errors import NotFoundError
from feljosit.web.deps import CsrfError, LoginRequiredError
from feljosit.web.routes import accounts, auth, budget, categories, imports, reports, transactions
from feljosit.web.security import (
    BodySizeLimitMiddleware,
    HostValidationMiddleware,
    SecurityHeadersMiddleware,
)

log = logging.getLogger(__name__)

#: Space for form fields in addition to the uploaded file.
FORM_OVERHEAD_BYTES = 64 * 1024


def create_app(settings: Settings, engine: Engine | None = None) -> FastAPI:
    """Make the application. Upgrade the database schema first."""
    if not expat_is_patched():
        log.warning(
            "The libexpat of this Python is older than %s. Update Python to get the "
            "XML security fixes.",
            ".".join(map(str, MIN_EXPAT_VERSION)),
        )
    engine = engine or make_engine(settings)
    migrate(engine, settings)

    app = FastAPI(
        title="Feljosit",
        docs_url=None,
        redoc_url=None,
        openapi_url=None,
    )
    app.state.settings = settings
    app.state.engine = engine
    app.state.session_factory = make_session_factory(engine)

    app.mount(
        "/static", StaticFiles(directory=str(Path(__file__).parent / "static")), name="static"
    )
    for module in (auth, budget, accounts, transactions, imports, categories, reports):
        app.include_router(module.router)

    @app.exception_handler(LoginRequiredError)
    async def _login_required(_request: Request, _exc: LoginRequiredError) -> Response:
        return RedirectResponse("/login", status_code=303)

    @app.exception_handler(CsrfError)
    async def _csrf_failed(_request: Request, _exc: CsrfError) -> Response:
        return PlainTextResponse("CSRF validation failed", status_code=403)

    @app.exception_handler(NotFoundError)
    async def _not_found(_request: Request, _exc: NotFoundError) -> Response:
        return PlainTextResponse("Not found", status_code=404)

    @app.exception_handler(RequestValidationError)
    async def _invalid(_request: Request, _exc: RequestValidationError) -> Response:
        # Do not echo the input back to the client.
        return PlainTextResponse("The request is not valid", status_code=422)

    @app.exception_handler(StarletteHTTPException)
    async def _http_error(_request: Request, exc: StarletteHTTPException) -> Response:
        return PlainTextResponse(str(exc.detail), status_code=exc.status_code)

    @app.get("/healthz", include_in_schema=False)
    def _health() -> PlainTextResponse:
        return PlainTextResponse("ok")

    # Middleware order: the last one added runs first.
    app.add_middleware(SecurityHeadersMiddleware, hsts=settings.secure_cookies)
    app.add_middleware(
        BodySizeLimitMiddleware, max_bytes=settings.max_upload_bytes + FORM_OVERHEAD_BYTES
    )
    app.add_middleware(HostValidationMiddleware, allowed_hosts=settings.allowed_hosts)
    return app
