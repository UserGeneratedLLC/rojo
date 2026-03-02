from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .db import Database


class _BotRef:
    db: Database | None = None


_bot_ref = _BotRef()
