from __future__ import annotations

import asyncio
import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import TYPE_CHECKING

import anthropic
import discord

from . import atlas_cli, discord_fmt, review, security

if TYPE_CHECKING:
    from .config import Config
    from .db import Database, Game

log = logging.getLogger(__name__)

_sync_tasks: dict[int, asyncio.Task[None]] = {}


async def init_game(
    config: Config, place_id: int, server_id: str, opencloud_key: str
) -> Path:
    game_dir = config.data_dir / server_id / str(place_id)
    game_dir.mkdir(parents=True, exist_ok=True)

    await atlas_cli.atlas_clone(config.atlas_binary, place_id, game_dir, opencloud_key)
    await atlas_cli.git_init(game_dir)
    await atlas_cli.git_add_all(game_dir)
    await atlas_cli.git_commit(game_dir, "Initial sync")

    return game_dir


def start_sync_loop(
    game: Game,
    config: Config,
    db: Database,
    bot: discord.Client,
    claude: anthropic.AsyncAnthropic,
) -> None:
    if game.id in _sync_tasks and not _sync_tasks[game.id].done():
        return

    task = asyncio.create_task(
        _sync_loop(game, config, db, bot, claude), name=f"sync-{game.place_id}"
    )
    _sync_tasks[game.id] = task


def stop_sync_loop(game_id: int) -> None:
    task = _sync_tasks.pop(game_id, None)
    if task and not task.done():
        task.cancel()


def stop_all_for_server(db: Database, server_id: str) -> None:
    for game in db.get_games_for_server(server_id):
        stop_sync_loop(game.id)


async def _sync_loop(
    game: Game,
    config: Config,
    db: Database,
    bot: discord.Client,
    claude: anthropic.AsyncAnthropic,
) -> None:
    await bot.wait_until_ready()
    log.info("Starting sync loop for PlaceId %d (server %s)", game.place_id, game.discord_server_id)

    while True:
        try:
            api_key = security.decrypt_api_key(game.api_key_encrypted)
            working_dir = Path(game.working_dir)

            await atlas_cli.atlas_syncback(
                config.atlas_binary, working_dir, game.place_id, api_key
            )

            diff = await atlas_cli.git_diff(working_dir)
            if diff:
                changed_files = await atlas_cli.git_diff_name_only(working_dir)
                script_files = [
                    f for f in changed_files
                    if any(f.endswith(ext) for ext in review.REVIEWABLE_EXTENSIONS)
                ]

                auto_resolved_ids = _auto_resolve(db, game.id, changed_files)
                for rid in auto_resolved_ids:
                    await _update_resolved_embed(bot, db, rid)

                if script_files:
                    existing = db.get_unresolved_issues(game.id)
                    try:
                        issues = await review.review_diff(claude, diff, existing)
                    except Exception as exc:
                        _handle_error(db, bot, game, str(exc), "Claude API error")
                        issues = []

                    new_issues = [
                        i for i in issues
                        if not db.final_dedup(game.id, i.file, i.title)
                    ]

                    if new_issues:
                        await _post_issues(bot, db, game, new_issues, diff, changed_files)

                await atlas_cli.git_add_all(working_dir)
                await atlas_cli.git_commit(
                    working_dir,
                    f"Sync {datetime.now(timezone.utc).strftime('%Y-%m-%d %H:%M:%S UTC')}",
                )
                db.update_last_sync(game.id)

        except asyncio.CancelledError:
            log.info("Sync loop cancelled for PlaceId %d", game.place_id)
            return
        except Exception as exc:
            error_str = str(exc)
            error_type = "Syncback error"
            if "HTTP 403" in error_str or "HTTP 401" in error_str:
                error_type = "Download failure (auth)"
            elif "Failed to download" in error_str:
                error_type = "Download failure"
            elif "atlas syncback failed" in error_str:
                error_type = "Syncback crash"
            elif "git" in error_str.lower():
                error_type = "Git error"

            should_report = db.update_last_error(game.id, error_str)
            if should_report:
                await _send_error(bot, game, error_str, error_type)

            log.exception("Sync loop error for PlaceId %d", game.place_id)

        await asyncio.sleep(config.sync_interval)


def _auto_resolve(db: Database, game_id: int, changed_files: list[str]) -> list[int]:
    resolved_ids = []
    for issue in db.get_unresolved_issues(game_id):
        if issue.file_path in changed_files:
            db.resolve_issue(issue.id, resolved_by="auto", reason="file modified")
            resolved_ids.append(issue.id)
    return resolved_ids


async def _update_resolved_embed(bot: discord.Client, db: Database, issue_id: int) -> None:
    from .db import DBIssue

    row = db._conn.execute("SELECT * FROM issues WHERE id = ?", (issue_id,)).fetchone()
    if not row:
        return
    issue = DBIssue(*row)
    if not issue.discord_message_id:
        return

    try:
        for guild in bot.guilds:
            for channel in guild.text_channels:
                try:
                    msg = await channel.fetch_message(int(issue.discord_message_id))
                    if msg.embeds:
                        embed = msg.embeds[0]
                        embed.set_footer(text="Resolved -- code was modified")
                        embed.color = 0x95A5A6
                        await msg.edit(embed=embed, view=None)
                    return
                except (discord.NotFound, discord.Forbidden):
                    continue
    except Exception:
        log.debug("Could not update resolved embed for issue %d", issue_id)


async def _post_issues(
    bot: discord.Client,
    db: Database,
    game: Game,
    issues: list[review.Issue],
    diff: str,
    changed_files: list[str],
) -> None:
    channel = bot.get_channel(int(game.channel_id))
    if not channel or not isinstance(channel, discord.TextChannel):
        log.warning("Channel %s not found for PlaceId %d", game.channel_id, game.place_id)
        return

    lines_added, lines_removed = discord_fmt.count_diff_lines(diff)
    script_files = [
        f for f in changed_files
        if any(f.endswith(ext) for ext in review.REVIEWABLE_EXTENSIONS)
    ]

    auto_resolved_count = len([
        i for i in db.get_unresolved_issues(game.id)
    ])

    summary = discord_fmt.make_summary_embed(
        game.place_id, issues, 0, script_files, lines_added, lines_removed
    )

    has_critical = any(i.severity == "Critical" for i in issues)
    content = "@here" if has_critical else None

    await channel.send(content=content, embed=summary)

    for issue in issues:
        embed = discord_fmt.make_issue_embed(issue)
        issue_db_id = db.add_issue(
            game_id=game.id,
            discord_message_id=None,
            file_path=issue.file,
            line_start=issue.line_start,
            line_end=issue.line_end,
            severity=issue.severity,
            title=issue.title,
            explanation=issue.explanation,
            suggestion=issue.suggestion,
        )

        view = discord_fmt.IssueButtons(issue_db_id)
        msg = await channel.send(embed=embed, view=view)
        db.update_issue_message_id(issue_db_id, str(msg.id))


async def _send_error(
    bot: discord.Client, game: Game, error: str, error_type: str
) -> None:
    channel = bot.get_channel(int(game.channel_id))
    if not channel or not isinstance(channel, discord.TextChannel):
        return

    embed = discord_fmt.make_error_embed(
        game.place_id, error, error_type, game.last_sync_at
    )
    await channel.send(embed=embed)


def _handle_error(
    db: Database,
    bot: discord.Client,
    game: Game,
    error: str,
    error_type: str,
) -> None:
    should_report = db.update_last_error(game.id, error)
    if should_report:
        asyncio.create_task(_send_error(bot, game, error, error_type))
