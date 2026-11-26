"""The structure of the categories: groups and the order of the categories."""

from __future__ import annotations

from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Category, CategoryGroup
from feljosit.services.errors import NotFoundError, ServiceError


def owned_group(db: Session, user_id: int, group_id: int) -> CategoryGroup:
    group = db.get(CategoryGroup, group_id)
    if group is None or group.user_id != user_id:
        raise NotFoundError("category group")
    return group


def _ordered(db: Session, group: CategoryGroup) -> list[Category]:
    return list(
        db.scalars(
            select(Category)
            .where(Category.group_id == group.id)
            .order_by(Category.sort_order, Category.name, Category.id)
        )
    )


def move_category(
    db: Session,
    category: Category,
    group: CategoryGroup,
    before: Category | None = None,
) -> None:
    """Move ``category`` into ``group``, before the category ``before``.

    If ``before`` is ``None``, the category becomes the last category of the group.
    The category can stay in its group: then only its position changes. The
    categories of the old and the new group get the sort orders 0, 1, 2, … again.
    The transactions and the budget amounts of the category do not change.
    """
    if group.user_id != category.user_id:
        raise NotFoundError("category group")
    if before is not None:
        if before.id == category.id:
            return
        if before.group_id != group.id:
            raise ServiceError("The target position is not in the target group.")
    old_group = db.get(CategoryGroup, category.group_id) if category.group_id != group.id else None
    target = [c for c in _ordered(db, group) if c.id != category.id]
    index = len(target) if before is None else target.index(before)
    target.insert(index, category)
    category.group_id = group.id
    for order, item in enumerate(target):
        item.sort_order = order
    if old_group is not None:
        remaining = [c for c in _ordered(db, old_group) if c.id != category.id]
        for order, item in enumerate(remaining):
            item.sort_order = order
    db.flush()
    # The relationship lists of the groups were possibly loaded before the change.
    for changed in (group, old_group):
        if changed is not None:
            db.expire(changed, ["categories"])
