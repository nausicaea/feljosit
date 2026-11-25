"""Application configuration.

The configuration comes from environment variables. All values have safe defaults.
"""

from __future__ import annotations

import os
import stat
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 8765
LOOPBACK_HOSTS = frozenset({"127.0.0.1", "::1", "localhost"})


def _default_data_dir() -> Path:
    xdg = os.environ.get("XDG_DATA_HOME")
    base = Path(xdg) if xdg else Path.home() / ".local" / "share"
    return base / "feljosit"


def _parse_bool(value: str | None, *, default: bool) -> bool:
    if value is None:
        return default
    return value.strip().lower() in {"1", "true", "yes", "on"}


@dataclass(frozen=True)
class Settings:
    """Runtime settings. Use :func:`load_settings` to make an instance."""

    data_dir: Path
    host: str = DEFAULT_HOST
    port: int = DEFAULT_PORT
    #: Set to true when a TLS reverse proxy is in front of the application.
    secure_cookies: bool = False
    #: Host header values that the application accepts (DNS rebinding protection).
    allowed_hosts: tuple[str, ...] = ("127.0.0.1", "localhost", "::1")
    session_lifetime_seconds: int = 12 * 60 * 60
    max_upload_bytes: int = 10 * 1024 * 1024
    budget_currency: str = "CHF"
    database_url_override: str | None = field(default=None)

    @property
    def database_path(self) -> Path:
        return self.data_dir / "feljosit.sqlite3"

    @property
    def database_url(self) -> str:
        if self.database_url_override:
            return self.database_url_override
        return f"sqlite:///{self.database_path}"


def load_settings(env: dict[str, str] | None = None) -> Settings:
    """Read the settings from environment variables with the prefix ``FELJOSIT_``."""
    env = dict(os.environ) if env is None else env
    data_dir = Path(env.get("FELJOSIT_DATA_DIR") or _default_data_dir()).expanduser()
    hosts_raw = env.get("FELJOSIT_ALLOWED_HOSTS")
    allowed_hosts = (
        tuple(h.strip() for h in hosts_raw.split(",") if h.strip())
        if hosts_raw
        else ("127.0.0.1", "localhost", "::1")
    )
    return Settings(
        data_dir=data_dir,
        host=env.get("FELJOSIT_HOST", DEFAULT_HOST),
        port=int(env.get("FELJOSIT_PORT", str(DEFAULT_PORT))),
        secure_cookies=_parse_bool(env.get("FELJOSIT_SECURE_COOKIES"), default=False),
        allowed_hosts=allowed_hosts,
        database_url_override=env.get("FELJOSIT_DATABASE_URL") or None,
    )


def ensure_data_dir(settings: Settings) -> None:
    """Make the data directory. Only the owner can read and write it (mode 0700)."""
    settings.data_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    settings.data_dir.chmod(stat.S_IRWXU)
