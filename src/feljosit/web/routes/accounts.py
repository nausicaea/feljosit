"""Accounts."""

from __future__ import annotations

from datetime import date
from typing import Annotated

from fastapi import APIRouter, Form, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response

from feljosit.models import AccountType
from feljosit.money import MoneyError, parse_user_amount, validate_currency
from feljosit.services import accounts
from feljosit.services.errors import ServiceError
from feljosit.web.deps import AppSettings, CsrfAuth, CurrentAuth
from feljosit.web.rendering import render

router = APIRouter()


@router.get("/accounts", response_class=HTMLResponse)
def accounts_page(request: Request, auth: CurrentAuth) -> Response:
    items = accounts.list_accounts(auth.db, auth.user.id)
    totals = accounts.balances(auth.db, auth.user.id)
    return render(
        request,
        "accounts.html",
        auth,
        accounts=items,
        balances=totals,
        account_types=list(AccountType),
        today=date.today().isoformat(),
    )


@router.post("/accounts")
def create_account(
    auth: CsrfAuth,
    settings: AppSettings,
    name: Annotated[str, Form(max_length=100)],
    account_type: Annotated[AccountType, Form()],
    currency: Annotated[str, Form(max_length=3)] = "CHF",
    iban: Annotated[str, Form(max_length=50)] = "",
    budget_mode: Annotated[str, Form(max_length=10)] = "default",
    starting_balance: Annotated[str, Form(max_length=32)] = "",
    starting_date: Annotated[date | None, Form()] = None,
) -> Response:
    on_budget = {"budget": True, "tracking": False}.get(budget_mode)
    try:
        code = validate_currency(currency)
        start = parse_user_amount(starting_balance, code) if starting_balance.strip() else 0
        account = accounts.create_account(
            auth.db,
            auth.user.id,
            name=name,
            account_type=account_type,
            currency=code,
            budget_currency=settings.budget_currency,
            iban=iban,
            on_budget=on_budget,
            starting_balance_minor=start,
            starting_date=starting_date,
        )
    except (ServiceError, MoneyError) as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse("/accounts", status_code=303)
    auth.flash(f"The account '{account.name}' is added.")
    return RedirectResponse("/accounts", status_code=303)


@router.post("/accounts/{account_id}/close")
def close_account(
    auth: CsrfAuth,
    account_id: int,
    closed: Annotated[bool, Form()] = True,
) -> Response:
    account = accounts.get_account(auth.db, auth.user.id, account_id)
    accounts.set_closed(auth.db, account, closed=closed)
    auth.flash(f"The account '{account.name}' is {'closed' if closed else 'open'}.")
    return RedirectResponse("/accounts", status_code=303)


@router.get("/accounts/{account_id}/edit", response_class=HTMLResponse)
def edit_account_page(request: Request, auth: CurrentAuth, account_id: int) -> Response:
    account = accounts.get_account(auth.db, auth.user.id, account_id)
    return render(
        request,
        "account_edit.html",
        auth,
        account=account,
        account_types=list(AccountType),
    )


@router.post("/accounts/{account_id}")
def update_account(
    auth: CsrfAuth,
    settings: AppSettings,
    account_id: int,
    name: Annotated[str, Form(max_length=100)],
    account_type: Annotated[AccountType, Form()],
    budget_mode: Annotated[str, Form(max_length=10)] = "default",
) -> Response:
    account = accounts.get_account(auth.db, auth.user.id, account_id)
    try:
        accounts.update_account(
            auth.db,
            account,
            name=name,
            account_type=account_type,
            budget_currency=settings.budget_currency,
            on_budget={"budget": True, "tracking": False}.get(budget_mode),
        )
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse(f"/accounts/{account.id}/edit", status_code=303)
    auth.flash(f"The account '{account.name}' is changed.")
    return RedirectResponse("/accounts", status_code=303)
