"""Categories and categorisation rules."""

from __future__ import annotations

from typing import Annotated

from fastapi import APIRouter, Form, Request
from fastapi.responses import HTMLResponse, JSONResponse, RedirectResponse, Response
from sqlalchemy import func, select

from feljosit.models import (
    CategorisationRule,
    Category,
    CategoryGroup,
    CategoryKind,
    MatchType,
    RuleField,
)
from feljosit.services import categories, categorise
from feljosit.services.errors import NotFoundError, ServiceError
from feljosit.web.deps import Auth, CsrfAuth, CurrentAuth
from feljosit.web.rendering import render

router = APIRouter()


def _name(value: str) -> str:
    value = value.strip()
    if not 1 <= len(value) <= 100:
        raise ServiceError("The name must have 1 to 100 characters.")
    return value


def _groups(auth: Auth) -> list[CategoryGroup]:
    return list(
        auth.db.scalars(
            select(CategoryGroup)
            .where(CategoryGroup.user_id == auth.user.id)
            .order_by(CategoryGroup.sort_order, CategoryGroup.name)
        )
    )


@router.get("/categories", response_class=HTMLResponse)
def categories_page(request: Request, auth: CurrentAuth) -> Response:
    groups = _groups(auth)
    rules = list(
        auth.db.scalars(
            select(CategorisationRule)
            .where(CategorisationRule.user_id == auth.user.id)
            .order_by(CategorisationRule.priority, CategorisationRule.id)
        )
    )
    return render(
        request,
        "categories.html",
        auth,
        groups=groups,
        rules=rules,
        kinds=list(CategoryKind),
        fields=list(RuleField),
        match_types=list(MatchType),
    )


@router.post("/categories/groups")
def create_group(auth: CsrfAuth, name: Annotated[str, Form(max_length=100)]) -> Response:
    try:
        order: int = (
            auth.db.scalar(
                select(func.coalesce(func.max(CategoryGroup.sort_order), 0)).where(
                    CategoryGroup.user_id == auth.user.id
                )
            )
            or 0
        )
        auth.db.add(CategoryGroup(user_id=auth.user.id, name=_name(name), sort_order=order + 1))
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
    return RedirectResponse("/categories", status_code=303)


@router.post("/categories")
def create_category(
    auth: CsrfAuth,
    group_id: Annotated[int, Form()],
    name: Annotated[str, Form(max_length=100)],
    kind: Annotated[CategoryKind, Form()] = CategoryKind.EXPENSE,
) -> Response:
    group = auth.db.get(CategoryGroup, group_id)
    if group is None or group.user_id != auth.user.id:
        raise NotFoundError("category group")
    try:
        auth.db.add(
            Category(
                user_id=auth.user.id,
                group_id=group.id,
                name=_name(name),
                kind=kind,
                sort_order=len(group.categories),
            )
        )
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
    return RedirectResponse("/categories", status_code=303)


@router.post("/categories/{category_id}/rename")
def rename_category(
    auth: CsrfAuth, category_id: int, name: Annotated[str, Form(max_length=100)]
) -> Response:
    category = categorise.owned_category(auth.db, auth.user.id, category_id)
    try:
        category.name = _name(name)
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
    return RedirectResponse("/categories", status_code=303)


@router.post("/categories/{category_id}/hide")
def hide_category(
    auth: CsrfAuth, category_id: int, hidden: Annotated[bool, Form()] = True
) -> Response:
    category = categorise.owned_category(auth.db, auth.user.id, category_id)
    category.hidden = hidden
    return RedirectResponse("/categories", status_code=303)


def _wants_json(request: Request) -> bool:
    """The drag-and-drop script asks for JSON. A form without JavaScript gets a redirect."""
    return "application/json" in request.headers.get("accept", "")


@router.post("/categories/{category_id}/move")
def move_category(
    request: Request,
    auth: CsrfAuth,
    category_id: int,
    group_id: Annotated[int, Form()],
    before_id: Annotated[str, Form(max_length=20)] = "",
) -> Response:
    """Move a category into a group, before a different category or to the end."""
    category = categorise.owned_category(auth.db, auth.user.id, category_id)
    group = categories.owned_group(auth.db, auth.user.id, group_id)
    before = (
        categorise.owned_category(auth.db, auth.user.id, int(before_id))
        if before_id.isdigit()
        else None
    )
    try:
        categories.move_category(auth.db, category, group, before)
    except ServiceError as exc:
        if _wants_json(request):
            return JSONResponse({"error": str(exc)}, status_code=400)
        auth.flash(str(exc), error=True)
        return RedirectResponse("/categories", status_code=303)
    if _wants_json(request):
        return JSONResponse({"group": group.name})
    auth.flash(f"The category '{category.name}' is in the group '{group.name}' now.")
    return RedirectResponse("/categories", status_code=303)


@router.post("/rules")
def create_rule(
    auth: CsrfAuth,
    field: Annotated[RuleField, Form()],
    match_type: Annotated[MatchType, Form()],
    pattern: Annotated[str, Form(max_length=140)],
    category_id: Annotated[int, Form()],
    priority: Annotated[int, Form(ge=0, le=10_000)] = 100,
) -> Response:
    try:
        categorise.create_rule(
            auth.db,
            auth.user.id,
            field=field,
            match_type=match_type,
            pattern=pattern,
            category_id=category_id,
            priority=priority,
        )
        auth.flash("The rule is added.")
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
    return RedirectResponse("/categories#rules", status_code=303)


@router.get("/rules/{rule_id}/edit", response_class=HTMLResponse)
def edit_rule_page(request: Request, auth: CurrentAuth, rule_id: int) -> Response:
    rule = categorise.owned_rule(auth.db, auth.user.id, rule_id)
    return render(
        request,
        "rule_edit.html",
        auth,
        rule=rule,
        groups=_groups(auth),
        fields=list(RuleField),
        match_types=list(MatchType),
    )


@router.post("/rules/{rule_id:int}")
def update_rule(
    auth: CsrfAuth,
    rule_id: int,
    field: Annotated[RuleField, Form()],
    match_type: Annotated[MatchType, Form()],
    pattern: Annotated[str, Form(max_length=140)],
    category_id: Annotated[int, Form()],
    priority: Annotated[int, Form(ge=0, le=10_000)] = 100,
) -> Response:
    rule = categorise.owned_rule(auth.db, auth.user.id, rule_id)
    try:
        categorise.update_rule(
            auth.db,
            rule,
            field=field,
            match_type=match_type,
            pattern=pattern,
            category_id=category_id,
            priority=priority,
        )
    except ServiceError as exc:
        auth.flash(str(exc), error=True)
        return RedirectResponse(f"/rules/{rule.id}/edit", status_code=303)
    auth.flash("The rule is changed.")
    return RedirectResponse("/categories#rules", status_code=303)


@router.post("/rules/{rule_id}/delete")
def delete_rule(auth: CsrfAuth, rule_id: int) -> Response:
    rule = categorise.owned_rule(auth.db, auth.user.id, rule_id)
    auth.db.delete(rule)
    return RedirectResponse("/categories#rules", status_code=303)


@router.post("/rules/apply")
def apply_rules(auth: CsrfAuth) -> Response:
    changed = categorise.apply_to_uncategorised(auth.db, auth.user.id)
    auth.flash(f"{changed} transactions are categorised.")
    return RedirectResponse("/transactions?uncategorised=true", status_code=303)
