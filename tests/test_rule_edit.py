"""Edit categorisation rules."""

import pytest
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy import select
from sqlalchemy.orm import Session

from feljosit.models import CategorisationRule, Category, MatchType, RuleField, User
from feljosit.services import categorise, users
from feljosit.services.errors import NotFoundError, ServiceError


def _category(db: Session, user_id: int, name: str) -> Category:
    category = db.scalar(select(Category).where(Category.user_id == user_id, Category.name == name))
    assert category is not None
    return category


def _rule(db: Session, user: User) -> CategorisationRule:
    return categorise.create_rule(
        db,
        user.id,
        field=RuleField.COUNTERPARTY,
        match_type=MatchType.CONTAINS,
        pattern="Migros",
        category_id=_category(db, user.id, "Groceries").id,
    )


def test_update_rule(db: Session, user: User) -> None:
    rule = _rule(db, user)
    eating_out = _category(db, user.id, "Eating out")
    categorise.update_rule(
        db,
        rule,
        field=RuleField.DESCRIPTION,
        match_type=MatchType.EQUALS,
        pattern="  Migros Restaurant  ",
        category_id=eating_out.id,
        priority=5,
    )
    db.expire_all()
    stored = categorise.owned_rule(db, user.id, rule.id)
    assert (stored.field, stored.match_type, stored.pattern) == (
        RuleField.DESCRIPTION,
        MatchType.EQUALS,
        "Migros Restaurant",
    )
    assert (stored.category_id, stored.priority) == (eating_out.id, 5)


@pytest.mark.parametrize(
    ("pattern", "priority", "message"),
    [(" ", 100, "1 to 140"), ("x" * 141, 100, "1 to 140"), ("x", 10_001, "priority")],
)
def test_update_rule_checks_values(
    db: Session, user: User, pattern: str, priority: int, message: str
) -> None:
    rule = _rule(db, user)
    with pytest.raises(ServiceError, match=message):
        categorise.update_rule(
            db,
            rule,
            field=rule.field,
            match_type=rule.match_type,
            pattern=pattern,
            category_id=rule.category_id,
            priority=priority,
        )
    assert rule.pattern == "Migros"


def test_update_rule_refuses_category_of_other_user(db: Session, user: User) -> None:
    rule = _rule(db, user)
    other = users.create_user(db, "bob", "another long password")
    with pytest.raises(NotFoundError):
        categorise.update_rule(
            db,
            rule,
            field=rule.field,
            match_type=rule.match_type,
            pattern="x",
            category_id=_category(db, other.id, "Groceries").id,
            priority=1,
        )
    with pytest.raises(NotFoundError):
        categorise.owned_rule(db, other.id, rule.id)


def test_edit_rule_pages(client: tuple[TestClient, str], app: FastAPI) -> None:
    http, token = client
    db = app.state.session_factory()
    user = db.scalar(select(User).where(User.username == "anna"))
    assert user is not None
    rule_id = _rule(db, user).id
    eating_out = _category(db, user.id, "Eating out").id
    db.commit()
    db.close()

    assert f'href="/rules/{rule_id}/edit"' in http.get("/categories").text
    page = http.get(f"/rules/{rule_id}/edit")
    assert page.status_code == 200
    assert 'value="Migros"' in page.text
    assert 'value="counterparty" selected' in page.text

    data = {
        "csrf_token": token,
        "field": "description",
        "match_type": "equals",
        "pattern": "Coop",
        "category_id": str(eating_out),
        "priority": "7",
    }
    response = http.post(f"/rules/{rule_id}", data=data)
    assert response.status_code == 200
    assert "The rule is changed." in response.text
    assert "Coop" in response.text
    db = app.state.session_factory()
    rule = db.get(CategorisationRule, rule_id)
    assert rule is not None
    assert (rule.field, rule.match_type, rule.pattern, rule.category_id, rule.priority) == (
        RuleField.DESCRIPTION,
        MatchType.EQUALS,
        "Coop",
        eating_out,
        7,
    )
    db.close()

    # An error goes back to the edit page with a message.
    response = http.post(f"/rules/{rule_id}", data=data | {"pattern": "   "})
    assert "1 to 140 characters" in response.text
    assert f'action="/rules/{rule_id}"' in response.text
    # "Apply" is not a rule ID.
    applied = http.post("/rules/apply", data={"csrf_token": token})
    assert "transactions are categorised" in applied.text
    # Unknown rules and the CSRF token.
    assert http.get("/rules/999999/edit").status_code == 404
    assert http.post("/rules/999999", data=data).status_code == 404
    response = http.post(f"/rules/{rule_id}", data=data | {"csrf_token": "wrong"})
    assert response.status_code in {400, 403}
