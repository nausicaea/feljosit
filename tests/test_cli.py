from pathlib import Path
from typing import Any

import pytest
import uvicorn

from feljosit import cli


@pytest.fixture
def data_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    monkeypatch.setenv("FELJOSIT_DATA_DIR", str(tmp_path / "data"))
    return tmp_path / "data"


def test_migrate(data_dir: Path, capsys: pytest.CaptureFixture[str]) -> None:
    assert cli.main(["migrate"]) == 0
    assert "up to date" in capsys.readouterr().out
    assert (data_dir / "feljosit.sqlite3").exists()


def test_create_user_and_set_password(
    data_dir: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr("getpass.getpass", lambda _prompt: "a long enough password")
    assert cli.main(["create-user", "anna"]) == 0
    assert cli.main(["create-user", "anna"]) == 1
    assert cli.main(["set-password", "anna"]) == 0
    assert cli.main(["set-password", "nobody"]) == 1
    output = capsys.readouterr()
    assert "is added" in output.out
    assert "exists already" in output.err


def test_password_mismatch(data_dir: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    answers = iter(["first long password", "second long password"])
    monkeypatch.setattr("getpass.getpass", lambda _prompt: next(answers))
    assert cli.main(["create-user", "anna"]) == 1


def test_serve_warns_on_public_address(
    data_dir: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    calls: list[tuple[str, int]] = []
    monkeypatch.setattr("uvicorn.run", lambda _app, host, port, **_kw: calls.append((host, port)))
    assert cli.main(["serve", "--host", "0.0.0.0", "--port", "9000"]) == 0  # noqa: S104
    assert calls == [("0.0.0.0", 9000)]  # noqa: S104
    assert "WARNING" in capsys.readouterr().err


def test_serve_sends_all_logs_to_stderr(data_dir: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    configs: list[dict[str, Any]] = []
    monkeypatch.setattr("uvicorn.run", lambda _app, log_config, **_kw: configs.append(log_config))
    assert cli.main(["serve"]) == 0
    streams = {h["stream"] for h in configs[0]["handlers"].values()}
    assert streams == {"ext://sys.stderr"}
    # The uvicorn default configuration is not changed.
    assert uvicorn.config.LOGGING_CONFIG["handlers"]["access"]["stream"] == "ext://sys.stdout"
