from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Config:
    discord_token: str
    anthropic_key: str
    data_dir: Path = field(default_factory=lambda: Path("./atlas-daemon-data"))
    encryption_key: str = ""
    sync_interval: int = 300
    max_games_per_server: int = 5
    atlas_binary: str = "atlas"

    @classmethod
    def from_env(cls, **overrides: object) -> Config:
        def _get(key: str, default: str = "") -> str:
            override = overrides.get(key.lower().removeprefix("atlas_daemon_"))
            if override is not None:
                return str(override)
            return os.environ.get(f"ATLAS_DAEMON_{key}", default)

        return cls(
            discord_token=_get("DISCORD_TOKEN"),
            anthropic_key=_get("ANTHROPIC_KEY"),
            data_dir=Path(_get("DATA_DIR", "./atlas-daemon-data")),
            encryption_key=_get("ENCRYPTION_KEY"),
            sync_interval=int(_get("SYNC_INTERVAL", "300")),
            max_games_per_server=int(_get("MAX_GAMES_PER_SERVER", "5")),
            atlas_binary=_get("ATLAS_BINARY", "atlas"),
        )
