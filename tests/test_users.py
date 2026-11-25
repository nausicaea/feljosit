from datetime import UTC, datetime, timedelta

import pytest
from sqlalchemy.orm import Session

from feljosit.models import User
from feljosit.services import users
from feljosit.services.errors import ServiceError
from tests.conftest import PASSWORD


@pytest.mark.parametrize("name", ["a", "Has Space", "x" * 65, "-start", "ümlaut"])
def test_invalid_usernames(db: Session, name: str) -> None:
    with pytest.raises(ServiceError):
        users.create_user(db, name, PASSWORD)


def test_password_rules(db: Session) -> None:
    with pytest.raises(ServiceError, match="at least"):
        users.create_user(db, "anna", "short")
    with pytest.raises(ServiceError, match="too long"):
        users.create_user(db, "anna", "x" * 2000)


def test_duplicate_username(db: Session, user: User) -> None:
    with pytest.raises(ServiceError, match="exists"):
        users.create_user(db, "ANNA", PASSWORD)


def test_new_user_has_default_categories(db: Session, user: User) -> None:
    from sqlalchemy import func, select

    from feljosit.models import Category

    count = db.scalar(select(func.count(Category.id)).where(Category.user_id == user.id))
    assert count is not None
    assert count > 15


def test_authenticate(db: Session, user: User) -> None:
    assert users.authenticate(db, " Anna ", PASSWORD) is user
    assert users.authenticate(db, "anna", "wrong") is None
    assert users.authenticate(db, "nobody", PASSWORD) is None
    assert users.authenticate(db, "anna", "x" * 5000) is None
    assert user.failed_logins == 1
    assert users.authenticate(db, "anna", PASSWORD) is user
    assert user.failed_logins == 0


def test_lockout_expires(db: Session, user: User) -> None:
    user.locked_until = datetime.now(UTC) + timedelta(minutes=1)
    assert users.authenticate(db, "anna", PASSWORD) is None
    user.locked_until = datetime.now(UTC) - timedelta(seconds=1)
    assert users.authenticate(db, "anna", PASSWORD) is user


def test_sessions(db: Session, user: User) -> None:
    new = users.start_session(db, user, 60)
    db.flush()
    session = users.load_session(db, new.token)
    assert session is not None
    assert session.user_id == user.id
    # Only the hash of the token is in the database.
    assert session.token_hash != new.token
    assert users.load_session(db, "") is None
    assert users.load_session(db, "x" * 200) is None
    session.expires_at = datetime.now(UTC) - timedelta(seconds=1)
    assert users.load_session(db, new.token) is None
    users.end_session(db, new.token)
    assert users.load_session(db, new.token) is None


def test_csrf_matches() -> None:
    assert users.csrf_matches("abc", "abc")
    assert not users.csrf_matches("abc", "abd")
    assert not users.csrf_matches("abc", None)
