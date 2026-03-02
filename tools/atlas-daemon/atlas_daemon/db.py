from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

GLOBAL_ADMIN_IDS = ["176643682689089545", "1010301288946339920"]

SCHEMA = """\
CREATE TABLE IF NOT EXISTS allowed_servers (
    discord_server_id TEXT PRIMARY KEY,
    server_name TEXT,
    approved_by TEXT NOT NULL,
    approved_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS admins (
    discord_user_id TEXT PRIMARY KEY,
    added_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS games (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    discord_server_id TEXT NOT NULL REFERENCES allowed_servers(discord_server_id) ON DELETE CASCADE,
    place_id INTEGER NOT NULL,
    channel_id TEXT NOT NULL,
    api_key_encrypted BLOB NOT NULL,
    added_by TEXT NOT NULL,
    working_dir TEXT NOT NULL,
    last_sync_at TEXT,
    last_error TEXT,
    last_error_at TEXT,
    error_count INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now')),
    UNIQUE(discord_server_id, place_id)
);

CREATE TABLE IF NOT EXISTS issues (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    game_id INTEGER NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    discord_message_id TEXT,
    file_path TEXT NOT NULL,
    line_start INTEGER,
    line_end INTEGER,
    severity TEXT NOT NULL,
    title TEXT NOT NULL,
    explanation TEXT,
    suggestion TEXT,
    resolved INTEGER DEFAULT 0,
    resolved_by TEXT,
    resolved_reason TEXT,
    resolved_at TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_issues_game_unresolved ON issues(game_id, resolved);
CREATE INDEX IF NOT EXISTS idx_games_server ON games(discord_server_id);
"""


@dataclass
class Game:
    id: int
    discord_server_id: str
    place_id: int
    channel_id: str
    api_key_encrypted: bytes
    added_by: str
    working_dir: str
    last_sync_at: str | None
    last_error: str | None
    last_error_at: str | None
    error_count: int
    created_at: str


@dataclass
class DBIssue:
    id: int
    game_id: int
    discord_message_id: str | None
    file_path: str
    line_start: int | None
    line_end: int | None
    severity: str
    title: str
    explanation: str | None
    suggestion: str | None
    resolved: bool
    resolved_by: str | None
    resolved_reason: str | None
    resolved_at: str | None
    created_at: str


class Database:
    def __init__(self, db_path: Path) -> None:
        db_path.parent.mkdir(parents=True, exist_ok=True)
        self._conn = sqlite3.connect(str(db_path), check_same_thread=False)
        self._conn.execute("PRAGMA journal_mode=WAL")
        self._conn.execute("PRAGMA foreign_keys=ON")
        self._conn.executescript(SCHEMA)
        self._seed_admins()

    def _seed_admins(self) -> None:
        for uid in GLOBAL_ADMIN_IDS:
            self._conn.execute(
                "INSERT OR IGNORE INTO admins (discord_user_id) VALUES (?)", (uid,)
            )
        self._conn.commit()

    def is_admin(self, user_id: str) -> bool:
        row = self._conn.execute(
            "SELECT 1 FROM admins WHERE discord_user_id = ?", (user_id,)
        ).fetchone()
        return row is not None

    # -- Server whitelist --

    def is_server_allowed(self, server_id: str) -> bool:
        row = self._conn.execute(
            "SELECT 1 FROM allowed_servers WHERE discord_server_id = ?", (server_id,)
        ).fetchone()
        return row is not None

    def approve_server(
        self, server_id: str, server_name: str | None, approved_by: str
    ) -> None:
        self._conn.execute(
            "INSERT OR REPLACE INTO allowed_servers (discord_server_id, server_name, approved_by) VALUES (?, ?, ?)",
            (server_id, server_name, approved_by),
        )
        self._conn.commit()

    def revoke_server(self, server_id: str) -> None:
        self._conn.execute(
            "DELETE FROM allowed_servers WHERE discord_server_id = ?", (server_id,)
        )
        self._conn.commit()

    # -- Games --

    def add_game(
        self,
        server_id: str,
        place_id: int,
        channel_id: str,
        api_key_encrypted: bytes,
        added_by: str,
        working_dir: str,
    ) -> int:
        cur = self._conn.execute(
            "INSERT INTO games (discord_server_id, place_id, channel_id, api_key_encrypted, added_by, working_dir) "
            "VALUES (?, ?, ?, ?, ?, ?)",
            (server_id, place_id, channel_id, api_key_encrypted, added_by, working_dir),
        )
        self._conn.commit()
        return cur.lastrowid  # type: ignore[return-value]

    def remove_game(self, server_id: str, place_id: int) -> bool:
        cur = self._conn.execute(
            "DELETE FROM games WHERE discord_server_id = ? AND place_id = ?",
            (server_id, place_id),
        )
        self._conn.commit()
        return cur.rowcount > 0

    def get_game(self, server_id: str, place_id: int) -> Game | None:
        row = self._conn.execute(
            "SELECT * FROM games WHERE discord_server_id = ? AND place_id = ?",
            (server_id, place_id),
        ).fetchone()
        return Game(*row) if row else None

    def get_games_for_server(self, server_id: str) -> list[Game]:
        rows = self._conn.execute(
            "SELECT * FROM games WHERE discord_server_id = ?", (server_id,)
        ).fetchall()
        return [Game(*r) for r in rows]

    def get_all_games(self) -> list[Game]:
        rows = self._conn.execute("SELECT * FROM games").fetchall()
        return [Game(*r) for r in rows]

    def count_games_for_server(self, server_id: str) -> int:
        row = self._conn.execute(
            "SELECT COUNT(*) FROM games WHERE discord_server_id = ?", (server_id,)
        ).fetchone()
        return row[0] if row else 0

    def update_last_sync(self, game_id: int) -> None:
        now = datetime.now(timezone.utc).isoformat()
        self._conn.execute(
            "UPDATE games SET last_sync_at = ?, last_error = NULL, last_error_at = NULL, error_count = 0 WHERE id = ?",
            (now, game_id),
        )
        self._conn.commit()

    def update_last_error(self, game_id: int, error: str) -> bool:
        """Update error state. Returns True if this is a new error or throttle window expired."""
        row = self._conn.execute(
            "SELECT last_error, error_count FROM games WHERE id = ?", (game_id,)
        ).fetchone()

        now = datetime.now(timezone.utc).isoformat()
        if row and row[0] == error:
            new_count = (row[1] or 0) + 1
            self._conn.execute(
                "UPDATE games SET error_count = ? WHERE id = ?",
                (new_count, game_id),
            )
            self._conn.commit()
            return new_count == 1 or new_count % 12 == 0
        else:
            self._conn.execute(
                "UPDATE games SET last_error = ?, last_error_at = ?, error_count = 1 WHERE id = ?",
                (error, now, game_id),
            )
            self._conn.commit()
            return True

    # -- Issues --

    def get_unresolved_issues(self, game_id: int) -> list[DBIssue]:
        rows = self._conn.execute(
            "SELECT * FROM issues WHERE game_id = ? AND resolved = 0", (game_id,)
        ).fetchall()
        return [DBIssue(*r) for r in rows]

    def add_issue(
        self,
        game_id: int,
        discord_message_id: str | None,
        file_path: str,
        line_start: int | None,
        line_end: int | None,
        severity: str,
        title: str,
        explanation: str | None,
        suggestion: str | None,
    ) -> int:
        cur = self._conn.execute(
            "INSERT INTO issues (game_id, discord_message_id, file_path, line_start, line_end, "
            "severity, title, explanation, suggestion) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (game_id, discord_message_id, file_path, line_start, line_end, severity, title, explanation, suggestion),
        )
        self._conn.commit()
        return cur.lastrowid  # type: ignore[return-value]

    def resolve_issue(
        self, issue_id: int, resolved_by: str, reason: str = "manual"
    ) -> None:
        now = datetime.now(timezone.utc).isoformat()
        self._conn.execute(
            "UPDATE issues SET resolved = 1, resolved_by = ?, resolved_reason = ?, resolved_at = ? WHERE id = ?",
            (resolved_by, reason, now, issue_id),
        )
        self._conn.commit()

    def update_issue_message_id(self, issue_id: int, message_id: str) -> None:
        self._conn.execute(
            "UPDATE issues SET discord_message_id = ? WHERE id = ?",
            (message_id, issue_id),
        )
        self._conn.commit()

    def get_issue_by_message_id(self, message_id: str) -> DBIssue | None:
        row = self._conn.execute(
            "SELECT * FROM issues WHERE discord_message_id = ?", (message_id,)
        ).fetchone()
        return DBIssue(*row) if row else None

    def final_dedup(self, game_id: int, file_path: str, title: str) -> bool:
        """Returns True if this (file, title) pair already exists as unresolved."""
        row = self._conn.execute(
            "SELECT 1 FROM issues WHERE game_id = ? AND file_path = ? AND title = ? AND resolved = 0",
            (game_id, file_path, title),
        ).fetchone()
        return row is not None
