"""Shared test fixtures."""

from __future__ import annotations

import re
from collections.abc import Iterator
from pathlib import Path

import pytest
from argon2 import PasswordHasher
from fastapi import FastAPI
from fastapi.testclient import TestClient
from sqlalchemy.orm import Session

from feljosit.config import Settings
from feljosit.db import make_engine, make_session_factory, migrate
from feljosit.models import User
from feljosit.services import users

FIXTURES = Path(__file__).parent / "fixtures"
PASSWORD = "correct horse battery staple"


@pytest.fixture(autouse=True)
def fast_password_hasher(monkeypatch: pytest.MonkeyPatch) -> None:
    """Use small Argon2 parameters. The production parameters are slow on purpose."""
    hasher = PasswordHasher(time_cost=1, memory_cost=8, parallelism=1)
    monkeypatch.setattr(users, "_hasher", hasher)
    monkeypatch.setattr(users, "_DUMMY_HASH", hasher.hash("dummy"))


@pytest.fixture
def settings(tmp_path: Path) -> Settings:
    return Settings(data_dir=tmp_path / "data")


@pytest.fixture
def db(settings: Settings) -> Iterator[Session]:
    engine = make_engine(settings)
    migrate(engine, settings)
    session = make_session_factory(engine)()
    try:
        yield session
    finally:
        session.close()
        engine.dispose()


@pytest.fixture
def user(db: Session) -> User:
    created = users.create_user(db, "anna", PASSWORD)
    db.commit()
    return created


@pytest.fixture
def app(settings: Settings) -> Iterator[FastAPI]:
    from feljosit.web.app import create_app

    application = create_app(settings)
    yield application
    application.state.engine.dispose()


def create_app_user(app: FastAPI, username: str) -> None:
    db = app.state.session_factory()
    try:
        users.create_user(db, username, PASSWORD)
        db.commit()
    finally:
        db.close()


def csrf_from(html: str) -> str:
    match = re.search(r'name="csrf_token" value="([^"]+)"', html)
    assert match, "No CSRF token in the page"
    return match.group(1)


def login(app: FastAPI, username: str) -> tuple[TestClient, str]:
    client = TestClient(app, base_url="http://127.0.0.1")
    page = client.get("/login")
    response = client.post(
        "/login",
        data={"username": username, "password": PASSWORD, "csrf_token": csrf_from(page.text)},
        follow_redirects=False,
    )
    assert response.status_code == 303, response.text
    token = csrf_from(client.get("/budget").text)
    return client, token


@pytest.fixture
def client(app: FastAPI) -> tuple[TestClient, str]:
    create_app_user(app, "anna")
    return login(app, "anna")


def fixture_bytes(name: str) -> bytes:
    return (FIXTURES / name).read_bytes()
