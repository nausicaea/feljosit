"""The budget page."""

from __future__ import annotations

from datetime import date
from typing import Annotated

from fastapi import APIRouter, Form, Query, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response

from feljosit.money import MoneyError, parse_user_amount
from feljosit.services import budget
from feljosit.services.errors import ServiceError
from feljosit.web.deps import AppSettings, CsrfAuth, CurrentAuth
from feljosit.web.rendering import render

router = APIRouter()


@router.get("/")
def home() -> Response:
    return RedirectResponse("/budget", status_code=303)


@router.get("/budget", response_class=HTMLResponse)
def budget_page(
    request: Request,
    auth: CurrentAuth,
    month: Annotated[str | None, Query(max_length=7)] = None,
) -> Response:
    try:
        selected = budget.parse_month(month, date.today())
    except ServiceError:
        selected = budget.month_start(date.today())
    data = budget.month_budget(auth.db, auth.user.id, selected)
    return render(
        request,
        "budget.html",
        auth,
        data=data,
        month_param=selected.strftime("%Y-%m"),
        previous=budget.add_months(selected, -1).strftime("%Y-%m"),
        next=budget.add_months(selected, 1).strftime("%Y-%m"),
    )


@router.post("/budget/assign")
def assign(
    auth: CsrfAuth,
    settings: AppSettings,
    category_id: Annotated[int, Form()],
    month: Annotated[str, Form(max_length=7)],
    amount: Annotated[str, Form(max_length=32)],
) -> Response:
    try:
        selected = budget.parse_month(month, date.today())
        value = parse_user_amount(amount or "0", settings.budget_currency)
        budget.assign(auth.db, auth.user.id, category_id, selected, value)
    except (ServiceError, MoneyError) as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse(f"/budget?month={month}", status_code=303)
    return RedirectResponse(f"/budget?month={selected:%Y-%m}#cat-{category_id}", status_code=303)
