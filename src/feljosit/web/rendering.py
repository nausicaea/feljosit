"""Template rendering."""

from __future__ import annotations

from datetime import date
from pathlib import Path
from typing import Any

from fastapi import Request
from fastapi.responses import HTMLResponse
from fastapi.templating import Jinja2Templates

from feljosit import __version__
from feljosit.iban import format_iban
from feljosit.money import format_minor
from feljosit.web.deps import Auth

_TEMPLATES = Jinja2Templates(directory=str(Path(__file__).parent / "templates"))
_env = _TEMPLATES.env
_env.autoescape = True
_env.trim_blocks = True
_env.lstrip_blocks = True

MONTH_NAMES = (
    "January", "February", "March", "April", "May", "June", "July",
    "August", "September", "October", "November", "December",
)  # fmt: skip


def _money(amount: int | None, currency: str = "CHF") -> str:
    return "" if amount is None else format_minor(amount, currency)


def _month_label(value: date) -> str:
    return f"{MONTH_NAMES[value.month - 1]} {value.year}"


def month_short(value: date) -> str:
    return f"{MONTH_NAMES[value.month - 1][:3]} {value.year % 100:02d}"


def _swiss_date(value: date | None) -> str:
    return "" if value is None else value.strftime("%d.%m.%Y")


_env.filters["money"] = _money
_env.filters["iban"] = format_iban
_env.filters["month_label"] = _month_label
_env.filters["month_short"] = month_short
_env.filters["swiss_date"] = _swiss_date
_env.globals["version"] = __version__


def render(
    request: Request,
    name: str,
    auth: Auth | None = None,
    *,
    status_code: int = 200,
    **context: Any,
) -> HTMLResponse:
    context["auth"] = auth
    context["csrf_token"] = auth.session.csrf_token if auth else None
    context["flash"] = auth.pop_flash() if auth else None
    context["budget_currency"] = request.app.state.settings.budget_currency
    return _TEMPLATES.TemplateResponse(request, name, context, status_code=status_code)
