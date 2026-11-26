"""Move categories into other groups (drag-and-drop and the group list)."""

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import Category, CategoryGroup, User
from feljosit.services import categories, users
from feljosit.services.errors import NotFoundError, ServiceError


def _groups(db: Session, user: User) -> dict[str, CategoryGroup]:
    return {
        g.name: g for g in db.scalars(select(CategoryGroup).where(CategoryGroup.user_id == user.id))
    }


def _names(db: Session, group: CategoryGroup) -> list[str]:
    db.refresh(group)
    return [c.name for c in group.categories]


def _category(db: Session, user: User, name: str) -> Category:
    category = db.scalar(select(Category).where(Category.user_id == user.id, Category.name == name))
    assert category is not None
    return category


def test_move_into_other_group(db: Session, user: User) -> None:
    groups = _groups(db, user)
    fixed, daily = groups["Fixed costs"], groups["Daily life"]
    before_fixed = _names(db, fixed)
    before_daily = _names(db, daily)
    groceries = _category(db, user, "Groceries")

    # To the end of the group.
    categories.move_category(db, groceries, fixed)
    assert _names(db, fixed) == [*before_fixed, "Groceries"]
    assert _names(db, daily) == [n for n in before_daily if n != "Groceries"]
    # The sort orders have no gaps.
    assert [c.sort_order for c in fixed.categories] == list(range(len(fixed.categories)))
    assert [c.sort_order for c in daily.categories] == list(range(len(daily.categories)))

    # Back, before the category that is the first of the group now.
    first = _names(db, daily)[0]
    categories.move_category(db, groceries, daily, _category(db, user, first))
    assert _names(db, daily)[:2] == ["Groceries", first]
    assert _names(db, fixed) == before_fixed


def test_move_inside_the_group(db: Session, user: User) -> None:
    fixed = _groups(db, user)["Fixed costs"]
    names = _names(db, fixed)
    last = _category(db, user, names[-1])
    first = _category(db, user, names[0])
    categories.move_category(db, last, fixed, first)
    assert _names(db, fixed) == [names[-1], *names[:-1]]
    # Before itself: no change.
    categories.move_category(db, last, fixed, last)
    assert _names(db, fixed) == [names[-1], *names[:-1]]


def test_move_into_empty_group(db: Session, user: User) -> None:
    empty = CategoryGroup(user_id=user.id, name="Empty", sort_order=99)
    db.add(empty)
    db.flush()
    groceries = _category(db, user, "Groceries")
    categories.move_category(db, groceries, empty)
    assert _names(db, empty) == ["Groceries"]
    assert groceries.sort_order == 0


def test_move_errors(db: Session, user: User) -> None:
    groups = _groups(db, user)
    groceries = _category(db, user, "Groceries")
    rent = _category(db, user, "Rent and service charges")
    # The position must be in the target group.
    assert rent.group_id != groups["Daily life"].id
    with pytest.raises(ServiceError, match="target group"):
        categories.move_category(db, groceries, groups["Daily life"], rent)
    other = users.create_user(db, "bob", "another long password")
    other_group = db.scalar(select(CategoryGroup).where(CategoryGroup.user_id == other.id))
    assert other_group is not None
    with pytest.raises(NotFoundError):
        categories.move_category(db, groceries, other_group)
    with pytest.raises(NotFoundError):
        categories.owned_group(db, user.id, other_group.id)


def _ids(app: FastAPI) -> tuple[int, int, int, int]:
    """The IDs of "Groceries", "Rent and service charges", "Fixed costs" and "Daily life"."""
    db = app.state.session_factory()
    try:
        groceries = db.scalar(select(Category).where(Category.name == "Groceries"))
        rent = db.scalar(select(Category).where(Category.name == "Rent and service charges"))
        fixed = db.scalar(select(CategoryGroup).where(CategoryGroup.name == "Fixed costs"))
        daily = db.scalar(select(CategoryGroup).where(CategoryGroup.name == "Daily life"))
        assert groceries and rent and fixed and daily
        return groceries.id, rent.id, fixed.id, daily.id
    finally:
        db.close()


def _group_of(app: FastAPI, category_id: int) -> int:
    db = app.state.session_factory()
    try:
        category = db.get(Category, category_id)
        assert category is not None
        return int(category.group_id)
    finally:
        db.close()


def test_move_with_the_form(client: tuple[TestClient, str], app: FastAPI) -> None:
    """Without JavaScript: the group list of the category sends a normal form."""
    http, token = client
    groceries, _, fixed, _ = _ids(app)
    page = http.get("/categories")
    assert f'action="/categories/{groceries}/move"' in page.text
    assert 'data-category-id="' in page.text and 'class="drag-handle"' in page.text
    response = http.post(
        f"/categories/{groceries}/move", data={"csrf_token": token, "group_id": str(fixed)}
    )
    assert response.status_code == 200
    assert "is in the group &#39;Fixed costs&#39; now" in response.text
    assert _group_of(app, groceries) == fixed


def test_move_with_json(client: tuple[TestClient, str], app: FastAPI) -> None:
    """The drag-and-drop script asks for JSON."""
    http, token = client
    groceries, rent, fixed, daily = _ids(app)
    headers = {"Accept": "application/json"}
    response = http.post(
        f"/categories/{groceries}/move",
        data={"csrf_token": token, "group_id": str(fixed), "before_id": str(rent)},
        headers=headers,
    )
    assert response.status_code == 200
    assert response.json() == {"group": "Fixed costs"}
    assert _group_of(app, groceries) == fixed
    # A position in a different group.
    response = http.post(
        f"/categories/{groceries}/move",
        data={"csrf_token": token, "group_id": str(daily), "before_id": str(rent)},
        headers=headers,
    )
    assert response.status_code == 400
    assert "target group" in response.json()["error"]
    assert _group_of(app, groceries) == fixed
    # Unknown IDs.
    response = http.post(
        "/categories/999999/move", data={"csrf_token": token, "group_id": str(daily)}
    )
    assert response.status_code == 404
    response = http.post(
        f"/categories/{groceries}/move", data={"csrf_token": token, "group_id": "999999"}
    )
    assert response.status_code == 404
    # The CSRF token is necessary.
    response = http.post(f"/categories/{groceries}/move", data={"group_id": str(daily)})
    assert response.status_code in {400, 403}
    assert _group_of(app, groceries) == fixed
