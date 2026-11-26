"""Transactions: list, categorise, manual entry."""

from __future__ import annotations

import math
from datetime import date
from decimal import Decimal
from typing import Annotated
from urllib.parse import parse_qsl, urlencode, urlsplit

from fastapi import APIRouter, Form, Query, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response
from pydantic import BeforeValidator
from sqlalchemy import ColumnElement, and_, false, func, or_, select

from feljosit.iban import normalise_iban
from feljosit.models import (
    Account,
    CategoryGroup,
    Transaction,
    TransactionSource,
    TransferMethod,
    TransferPair,
    TransferStatus,
)
from feljosit.money import MoneyError, exponent, parse_user_amount, parse_user_decimal
from feljosit.services import accounts, categorise, transfers
from feljosit.services.errors import NotFoundError, ServiceError
from feljosit.web.deps import CsrfAuth, CurrentAuth
from feljosit.web.rendering import render

router = APIRouter()
PAGE_SIZE = 100


def _empty_as_none(value: object) -> object:
    """Change an empty query value to ``None``.

    The filter form sends ``account=`` when the user selects "All accounts".
    """
    return None if value == "" else value


OptionalId = Annotated[int | None, BeforeValidator(_empty_as_none), Query()]


def _get_tx(auth: CurrentAuth, tx_id: int) -> Transaction:
    tx = auth.db.get(Transaction, tx_id)
    if tx is None or tx.user_id != auth.user.id:
        raise NotFoundError("transaction")
    return tx


def _escape_like(text: str) -> str:
    return text.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_")


#: The special values of the category filter. A number is a category ID.
CATEGORY_FILTERS = {
    "uncategorised": "— Uncategorised —",
    "transfers": "All transfers",
    "matched": "Transfers found from the amount",
    "unmatched": "Unmatched transfers (no partner)",
}


def _pair_ids(auth: CurrentAuth, method: TransferMethod | None = None) -> ColumnElement[bool]:
    """Transactions that are in an active pair (of ``method``, if given)."""
    conditions = [
        TransferPair.user_id == auth.user.id,
        TransferPair.status == TransferStatus.ACTIVE,
    ]
    if method is not None:
        conditions.append(TransferPair.method == method)
    return or_(
        Transaction.id.in_(select(TransferPair.outflow_id).where(*conditions)),
        Transaction.id.in_(select(TransferPair.inflow_id).where(*conditions)),
    )


def _category_filter(auth: CurrentAuth, category: str) -> list[ColumnElement[bool]]:
    """The conditions for a value of the category filter. An unknown value: no filter."""
    if category == "uncategorised":
        return [
            Transaction.category_id.is_(None),
            Transaction.is_transfer.is_(False),
            Account.on_budget.is_(True),
        ]
    if category == "transfers":
        return [Transaction.is_transfer.is_(True)]
    if category == "matched":
        return [Transaction.is_transfer.is_(True), _pair_ids(auth, TransferMethod.MATCH)]
    if category == "unmatched":
        return [Transaction.is_transfer.is_(True), ~_pair_ids(auth)]
    if category.isdigit():
        return [Transaction.category_id == int(category)]
    return []


#: The values of the direction filter.
DIRECTIONS = {"outflow": "Outflows (−)", "inflow": "Inflows (+)"}


def _direction_filter(direction: str) -> list[ColumnElement[bool]]:
    if direction == "outflow":
        return [Transaction.amount_minor < 0]
    if direction == "inflow":
        return [Transaction.amount_minor > 0]
    return []


def _amount_limit(text: str) -> Decimal | None:
    """Parse a limit of the amount filter. An empty text: no limit.

    The limit applies to the amount without its sign. Raise :class:`MoneyError` if
    the text is not an amount.
    """
    return abs(parse_user_decimal(text)) if text.strip() else None


def _amount_filter(
    auth: CurrentAuth, minimum: Decimal | None, maximum: Decimal | None
) -> list[ColumnElement[bool]]:
    """Compare the amount without its sign with the limits.

    The limits are in the currency of each transaction. Thus the filter makes one
    condition for each currency of the user.
    """
    if minimum is None and maximum is None:
        return []
    currencies = auth.db.scalars(
        select(Transaction.currency).where(Transaction.user_id == auth.user.id).distinct()
    )
    size = func.abs(Transaction.amount_minor)
    alternatives = []
    for currency in currencies:
        conditions = [Transaction.currency == currency]
        exp = exponent(currency)
        if minimum is not None:
            conditions.append(size >= math.ceil(minimum.scaleb(exp)))
        if maximum is not None:
            conditions.append(size <= math.floor(maximum.scaleb(exp)))
        alternatives.append(and_(*conditions))
    return [or_(*alternatives)] if alternatives else [false()]


@router.get("/transactions", response_class=HTMLResponse)
def transactions_page(
    request: Request,
    auth: CurrentAuth,
    account: OptionalId = None,
    category: Annotated[str, Query(max_length=20)] = "",
    q: Annotated[str, Query(max_length=100)] = "",
    direction: Annotated[str, Query(max_length=10)] = "",
    amount_min: Annotated[str, Query(alias="min", max_length=32)] = "",
    amount_max: Annotated[str, Query(alias="max", max_length=32)] = "",
    page: Annotated[int, Query(ge=1, le=100_000)] = 1,
    iban: Annotated[bool, Query()] = False,
    uncategorised: Annotated[bool, Query()] = False,
    matched: Annotated[bool, Query()] = False,
) -> Response:
    """The transaction list.

    ``category`` is a category ID or a value of ``CATEGORY_FILTERS``. ``direction`` is
    a value of ``DIRECTIONS``. ``min`` and ``max`` are limits for the amount without
    its sign. ``iban`` shows
    the account number of the counterparty. ``uncategorised`` and ``matched`` are the
    parameters of older links. They are the same as ``category=uncategorised`` and
    ``category=matched``.
    """
    if uncategorised:
        category = "uncategorised"
    elif matched:
        category = "matched"
    if not (category in CATEGORY_FILTERS or category.isdigit()):
        category = ""
    filters = [Transaction.user_id == auth.user.id]
    if account is not None:
        filters.append(Transaction.account_id == account)
    filters += _category_filter(auth, category)
    if direction not in DIRECTIONS:
        direction = ""
    filters += _direction_filter(direction)
    amount_error = None
    try:
        minimum, maximum = _amount_limit(amount_min), _amount_limit(amount_max)
    except MoneyError:
        amount_error = (
            "The amount filter is not valid. Type an amount, for example 300 or 1'250.50."
        )
        minimum = maximum = None
        amount_min = amount_max = ""
    if minimum is not None and maximum is not None and minimum > maximum:
        amount_error = "The minimum amount is more than the maximum amount."
    filters += _amount_filter(auth, minimum, maximum)
    if q.strip():
        pattern = f"%{_escape_like(q.strip())}%"
        conditions = [
            Transaction.counterparty.ilike(pattern, escape="\\"),
            Transaction.description.ilike(pattern, escape="\\"),
            Transaction.memo.ilike(pattern, escape="\\"),
        ]
        # The IBANs are stored without spaces: "CH93 0076" finds "CH9300762…". Only a
        # text with a digit and 4 or more characters can be part of an IBAN. Else a
        # search for "ch" finds all Swiss IBANs.
        compact = normalise_iban(q)
        if len(compact) >= 4 and any(c.isdigit() for c in compact):
            iban_pattern = f"%{_escape_like(compact)}%"
            conditions.append(Transaction.counterparty_iban.ilike(iban_pattern, escape="\\"))
        filters.append(or_(*conditions))
    base = select(Transaction).join(Account, Transaction.account_id == Account.id).where(*filters)
    total = auth.db.scalar(select(func.count()).select_from(base.subquery())) or 0
    items = list(
        auth.db.scalars(
            base.order_by(Transaction.booking_date.desc(), Transaction.id.desc())
            .limit(PAGE_SIZE)
            .offset((page - 1) * PAGE_SIZE)
        )
    )
    groups = list(
        auth.db.scalars(
            select(CategoryGroup)
            .where(CategoryGroup.user_id == auth.user.id)
            .order_by(CategoryGroup.sort_order, CategoryGroup.name)
        )
    )
    params = {
        k: v
        for k, v in {
            "account": account,
            "category": category or None,
            "q": q or None,
            "direction": direction or None,
            "min": amount_min.strip() or None,
            "max": amount_max.strip() or None,
        }.items()
        if v is not None
    }
    # A display option, not a filter: "Remove the filters" keeps it.
    display = {"iban": "true"} if iban else {}
    return render(
        request,
        "transactions.html",
        auth,
        transactions=items,
        partners=transfers.partners(auth.db, items),
        total=total,
        page=page,
        pages=max((total + PAGE_SIZE - 1) // PAGE_SIZE, 1),
        query_string=urlencode({**params, **display}),
        filters_active=bool(params),
        display_query=urlencode(display),
        show_iban=iban,
        return_to="/transactions?" + urlencode({**params, **display, "page": page}),
        accounts=accounts.list_accounts(auth.db, auth.user.id),
        groups=groups,
        filter_account=account,
        filter_category=category,
        category_filters=CATEGORY_FILTERS,
        q=q,
        filter_direction=direction,
        directions=DIRECTIONS,
        amount_min=amount_min.strip(),
        amount_max=amount_max.strip(),
        amount_error=amount_error,
        today=date.today().isoformat(),
    )


#: Query parameters that the transaction list accepts. A redirect keeps only these.
_LIST_PARAMS = (
    "account",
    "category",
    "uncategorised",
    "q",
    "direction",
    "min",
    "max",
    "page",
    "matched",
    "iban",
)


def _back(return_to: str) -> RedirectResponse:
    """Go back to the transaction list with the same filters.

    The redirect target is always the local path ``/transactions``. Only known query
    parameters are copied from ``return_to``. This prevents an open redirect.
    """
    parts = urlsplit(return_to)
    params = [(k, v) for k, v in parse_qsl(parts.query) if k in _LIST_PARAMS]
    query = urlencode(params)
    return RedirectResponse(f"/transactions?{query}" if query else "/transactions", 303)


@router.post("/transactions/{tx_id}/category")
def set_category(
    auth: CsrfAuth,
    tx_id: int,
    category_id: Annotated[str, Form(max_length=20)] = "",
    return_to: Annotated[str, Form(max_length=500)] = "/transactions",
) -> Response:
    tx = _get_tx(auth, tx_id)
    try:
        if category_id == "transfer":
            tx.category_id = None
            tx.is_transfer = True
        else:
            partner = categorise.set_category(
                auth.db, tx, int(category_id) if category_id else None
            )
            if partner is not None:
                auth.flash(
                    f"This transaction and the transaction of {partner.booking_date:%d.%m.%Y} "
                    f"on '{partner.account.name}' are not a transfer any more. "
                    "Give a category to the other transaction too."
                )
    except (ServiceError, ValueError) as exc:
        message = str(exc) if isinstance(exc, ServiceError) else "The category is not valid."
        auth.flash(message, error=True)
    return _back(return_to)


@router.post("/transactions/bulk-category")
def set_category_bulk(
    auth: CsrfAuth,
    tx_id: Annotated[list[int] | None, Form()] = None,
    category_id: Annotated[str, Form(max_length=20)] = "",
    return_to: Annotated[str, Form(max_length=500)] = "/transactions",
) -> Response:
    """Give one category to the selected transactions.

    ``category_id`` is a category ID, ``none`` (uncategorised) or ``transfer``. The
    changes are the same as the changes of :func:`set_category` for each transaction.
    Transactions in tracking accounts have no category. The function does not change
    them.
    """
    ids = list(dict.fromkeys(tx_id or []))
    if not ids:
        auth.flash("Select one or more transactions.", error=True)
        return _back(return_to)
    if len(ids) > PAGE_SIZE:
        auth.flash(f"Select a maximum of {PAGE_SIZE} transactions.", error=True)
        return _back(return_to)
    to_transfer = category_id == "transfer"
    target: int | None = None
    if not to_transfer and category_id != "none":
        if not category_id.isdigit():
            auth.flash("Select a category.", error=True)
            return _back(return_to)
        target = categorise.owned_category(auth.db, auth.user.id, int(category_id)).id
    selected = list(
        auth.db.scalars(
            select(Transaction).where(Transaction.user_id == auth.user.id, Transaction.id.in_(ids))
        )
    )
    if len(selected) != len(ids):
        raise NotFoundError("transaction")
    changed = tracking = 0
    partners: list[Transaction] = []
    for tx in selected:
        if not tx.account.on_budget:
            tracking += 1
            continue
        if to_transfer:
            tx.category_id = None
            tx.is_transfer = True
        else:
            partner = categorise.set_category(auth.db, tx, target)
            if partner is not None and partner.id not in ids:
                partners.append(partner)
        changed += 1
    messages = [f"The category of {changed} transactions is changed."]
    if tracking:
        messages.append(f"{tracking} transactions are in tracking accounts. They have no category.")
    if partners:
        messages.append(
            f"{len(partners)} other transactions are not transfers any more. "
            "Give a category to them too. Select 'Uncategorised' in the category filter."
        )
    auth.flash(" ".join(messages))
    return _back(return_to)


@router.post("/transactions/{tx_id}/memo")
def set_memo(
    auth: CsrfAuth,
    tx_id: int,
    memo: Annotated[str, Form(max_length=500)] = "",
    return_to: Annotated[str, Form(max_length=500)] = "/transactions",
) -> Response:
    tx = _get_tx(auth, tx_id)
    tx.memo = memo.strip()
    return _back(return_to)


@router.post("/transactions/{tx_id}/delete")
def delete_transaction(
    auth: CsrfAuth,
    tx_id: int,
    return_to: Annotated[str, Form(max_length=500)] = "/transactions",
) -> Response:
    tx = _get_tx(auth, tx_id)
    if tx.source is not TransactionSource.MANUAL:
        auth.flash("You can only delete transactions that you added manually.", error=True)
    else:
        transfers.before_delete(auth.db, tx)
        auth.db.delete(tx)
        auth.flash("The transaction is deleted.")
    return _back(return_to)


@router.post("/transactions")
def add_transaction(
    auth: CsrfAuth,
    account_id: Annotated[int, Form()],
    booking_date: Annotated[date, Form()],
    amount: Annotated[str, Form(max_length=32)],
    direction: Annotated[str, Form(pattern="^(outflow|inflow)$")],
    counterparty: Annotated[str, Form(max_length=140)] = "",
    memo: Annotated[str, Form(max_length=500)] = "",
    category_id: Annotated[str, Form(max_length=20)] = "",
) -> Response:
    account = accounts.get_account(auth.db, auth.user.id, account_id)
    try:
        value = abs(parse_user_amount(amount, account.currency))
        if direction == "outflow":
            value = -value
        accounts.add_manual_transaction(
            auth.db,
            account,
            booking_date=booking_date,
            amount_minor=value,
            counterparty=counterparty,
            memo=memo,
            category_id=int(category_id) if category_id.isdigit() else None,
        )
    except (ServiceError, MoneyError) as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse("/transactions", status_code=303)
    auth.flash("The transaction is added.")
    return RedirectResponse(f"/transactions?account={account.id}", status_code=303)
