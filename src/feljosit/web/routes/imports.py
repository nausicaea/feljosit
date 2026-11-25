"""Import of camt.053 bank statements."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, File, Form, Request, UploadFile
from fastapi.responses import HTMLResponse, Response
from sqlalchemy import select

from feljosit.camt053 import Camt053Error
from feljosit.models import ImportBatch
from feljosit.services import accounts
from feljosit.services.errors import ServiceError
from feljosit.services.importer import import_statement_file
from feljosit.web.deps import AppSettings, CsrfAuth, CurrentAuth
from feljosit.web.rendering import render

router = APIRouter()


def _page(request: Request, auth: CurrentAuth, **context: object) -> Response:
    history = list(
        auth.db.scalars(
            select(ImportBatch)
            .where(ImportBatch.user_id == auth.user.id)
            .order_by(ImportBatch.imported_at.desc())
            .limit(20)
        )
    )
    status = 400 if context.get("error") else 200
    return render(
        request,
        "import.html",
        auth,
        accounts=accounts.list_accounts(auth.db, auth.user.id, include_closed=False),
        history=history,
        status_code=status,
        **context,
    )


@router.get("/import", response_class=HTMLResponse)
def import_page(request: Request, auth: CurrentAuth) -> Response:
    return _page(request, auth)


@router.post("/import", response_class=HTMLResponse)
async def import_upload(
    request: Request,
    auth: CsrfAuth,
    settings: AppSettings,
    statement: Annotated[UploadFile, File()],
    account_id: Annotated[str, Form(max_length=20)] = "",
) -> Response:
    data = await statement.read(settings.max_upload_bytes + 1)
    if len(data) > settings.max_upload_bytes:
        return _page(request, auth, error="The file is too large.")
    if not data:
        return _page(request, auth, error="The file is empty.")
    try:
        result = import_statement_file(
            auth.db,
            auth.user.id,
            filename=statement.filename or "statement.xml",
            data=data,
            account_id=int(account_id) if account_id.isdigit() else None,
        )
    except (ServiceError, Camt053Error) as exc:
        auth.db.rollback()
        return _page(request, auth, error=str(exc))
    return _page(request, auth, result=result)
