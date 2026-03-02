from __future__ import annotations

import logging
from typing import TYPE_CHECKING

import anthropic
import discord
from discord import app_commands

from . import _bot_ref, daemon, security

if TYPE_CHECKING:
    from .config import Config
    from .db import Database

log = logging.getLogger(__name__)


class AtlasDaemonBot(discord.Client):
    def __init__(
        self,
        config: Config,
        db: Database,
        anthropic_client: anthropic.AsyncAnthropic,
    ) -> None:
        intents = discord.Intents.default()
        intents.guilds = True
        intents.guild_messages = True
        super().__init__(intents=intents)

        self.config = config
        self.db = db
        self.claude = anthropic_client
        self.tree = app_commands.CommandTree(self)

        _bot_ref.db = db

        self._register_commands()

    def _register_commands(self) -> None:
        tree = self.tree

        @tree.command(name="approve-server", description="Whitelist this server (or a remote one by ID) to use the bot")
        @app_commands.describe(server_id="Optional: remote server ID to approve")
        async def approve_server(interaction: discord.Interaction, server_id: str | None = None) -> None:
            if not self.db.is_admin(str(interaction.user.id)):
                await interaction.response.send_message("Only bot admins can approve servers.", ephemeral=True)
                return

            target_id = server_id or str(interaction.guild_id)
            server_name = None
            if not server_id and interaction.guild:
                server_name = interaction.guild.name

            self.db.approve_server(target_id, server_name, str(interaction.user.id))
            await interaction.response.send_message(f"Server `{target_id}` approved.", ephemeral=True)

        @tree.command(name="revoke-server", description="Remove a server from the whitelist")
        @app_commands.describe(server_id="Server ID to revoke")
        async def revoke_server(interaction: discord.Interaction, server_id: str) -> None:
            if not self.db.is_admin(str(interaction.user.id)):
                await interaction.response.send_message("Only bot admins can revoke servers.", ephemeral=True)
                return

            daemon.stop_all_for_server(self.db, server_id)
            self.db.revoke_server(server_id)
            await interaction.response.send_message(f"Server `{server_id}` revoked.", ephemeral=True)

        @tree.command(name="set-api-key", description="Set your OpenCloud API key (requires legacy-asset:manage scope)")
        @app_commands.describe(key="Your OpenCloud API key")
        async def set_api_key(interaction: discord.Interaction, key: str) -> None:
            if not self._check_server(interaction):
                return

            encrypted = security.encrypt_api_key(key)
            for game in self.db.get_games_for_server(str(interaction.guild_id)):
                if game.added_by == str(interaction.user.id):
                    self.db._conn.execute(
                        "UPDATE games SET api_key_encrypted = ? WHERE id = ?",
                        (encrypted, game.id),
                    )
            self.db._conn.commit()
            await interaction.response.send_message(
                "API key updated for all your games in this server.", ephemeral=True
            )

        @tree.command(name="add-game", description="Start monitoring a Roblox game")
        @app_commands.describe(
            place_id="The Roblox Place ID to monitor",
            channel="Channel to post reviews in (defaults to current)",
        )
        async def add_game(
            interaction: discord.Interaction,
            place_id: int,
            channel: discord.TextChannel | None = None,
        ) -> None:
            if not await self._check_server_async(interaction):
                return

            server_id = str(interaction.guild_id)

            if self.db.count_games_for_server(server_id) >= self.config.max_games_per_server:
                await interaction.response.send_message(
                    f"This server has reached the maximum of {self.config.max_games_per_server} monitored games.",
                    ephemeral=True,
                )
                return

            if self.db.get_game(server_id, place_id):
                await interaction.response.send_message(
                    f"PlaceId {place_id} is already being monitored in this server.",
                    ephemeral=True,
                )
                return

            await interaction.response.defer(ephemeral=True)

            target_channel = channel or interaction.channel
            if not isinstance(target_channel, discord.TextChannel):
                await interaction.followup.send("Please specify a text channel.", ephemeral=True)
                return

            games = self.db.get_games_for_server(server_id)
            user_games = [g for g in games if g.added_by == str(interaction.user.id)]
            api_key_encrypted: bytes | None = None
            if user_games:
                api_key_encrypted = user_games[0].api_key_encrypted

            if api_key_encrypted is None:
                await interaction.followup.send(
                    "No API key found. Please run `/set-api-key` first, then try again.",
                    ephemeral=True,
                )
                return

            try:
                opencloud_key = security.decrypt_api_key(api_key_encrypted)
                game_dir = await daemon.init_game(
                    self.config, place_id, server_id, opencloud_key
                )
            except Exception as exc:
                await interaction.followup.send(
                    f"Failed to initialize game: ```{str(exc)[:1500]}```", ephemeral=True
                )
                return

            game_id = self.db.add_game(
                server_id=server_id,
                place_id=place_id,
                channel_id=str(target_channel.id),
                api_key_encrypted=api_key_encrypted,
                added_by=str(interaction.user.id),
                working_dir=str(game_dir),
            )

            game = self.db.get_game(server_id, place_id)
            if game:
                daemon.start_sync_loop(game, self.config, self.db, self, self.claude)

            await interaction.followup.send(
                f"Now monitoring PlaceId {place_id} in {target_channel.mention}. "
                f"Syncing every {self.config.sync_interval}s.",
                ephemeral=True,
            )

        @tree.command(name="remove-game", description="Stop monitoring a game")
        @app_commands.describe(place_id="The Place ID to stop monitoring")
        async def remove_game(interaction: discord.Interaction, place_id: int) -> None:
            if not await self._check_server_async(interaction):
                return

            server_id = str(interaction.guild_id)
            game = self.db.get_game(server_id, place_id)
            if not game:
                await interaction.response.send_message(
                    f"PlaceId {place_id} is not being monitored.", ephemeral=True
                )
                return

            if game.added_by != str(interaction.user.id) and not self.db.is_admin(str(interaction.user.id)):
                await interaction.response.send_message("You can only remove games you added.", ephemeral=True)
                return

            daemon.stop_sync_loop(game.id)
            self.db.remove_game(server_id, place_id)
            await interaction.response.send_message(f"Stopped monitoring PlaceId {place_id}.", ephemeral=True)

        @tree.command(name="reset-game", description="Delete local clone and re-sync from scratch")
        @app_commands.describe(place_id="The Place ID to reset")
        async def reset_game(interaction: discord.Interaction, place_id: int) -> None:
            if not await self._check_server_async(interaction):
                return

            server_id = str(interaction.guild_id)
            game = self.db.get_game(server_id, place_id)
            if not game:
                await interaction.response.send_message(
                    f"PlaceId {place_id} is not being monitored.", ephemeral=True
                )
                return

            if game.added_by != str(interaction.user.id) and not self.db.is_admin(str(interaction.user.id)):
                await interaction.response.send_message("You can only reset games you added.", ephemeral=True)
                return

            await interaction.response.defer(ephemeral=True)

            daemon.stop_sync_loop(game.id)

            import shutil
            from pathlib import Path

            working_dir = Path(game.working_dir)
            if working_dir.exists():
                shutil.rmtree(working_dir, ignore_errors=True)

            try:
                opencloud_key = security.decrypt_api_key(game.api_key_encrypted)
                await daemon.init_game(self.config, place_id, server_id, opencloud_key)
            except Exception as exc:
                await interaction.followup.send(
                    f"Failed to re-initialize: ```{str(exc)[:1500]}```", ephemeral=True
                )
                return

            self.db._conn.execute(
                "DELETE FROM issues WHERE game_id = ?", (game.id,)
            )
            self.db._conn.execute(
                "UPDATE games SET last_sync_at = NULL, last_error = NULL, error_count = 0 WHERE id = ?",
                (game.id,),
            )
            self.db._conn.commit()

            game = self.db.get_game(server_id, place_id)
            if game:
                daemon.start_sync_loop(game, self.config, self.db, self, self.claude)

            await interaction.followup.send(f"PlaceId {place_id} has been reset and re-synced.", ephemeral=True)

        @tree.command(name="status", description="Show monitored games and sync status")
        async def status(interaction: discord.Interaction) -> None:
            if not await self._check_server_async(interaction):
                return

            games = self.db.get_games_for_server(str(interaction.guild_id))
            if not games:
                await interaction.response.send_message("No games are being monitored in this server.", ephemeral=True)
                return

            lines = []
            for g in games:
                status_icon = "🔴" if g.last_error else "🟢"
                unresolved = len(self.db.get_unresolved_issues(g.id))
                lines.append(
                    f"{status_icon} **PlaceId {g.place_id}** -- "
                    f"Last sync: {g.last_sync_at or 'Never'} -- "
                    f"Unresolved: {unresolved}"
                )
                if g.last_error:
                    error_preview = g.last_error[:100] + ("..." if len(g.last_error) > 100 else "")
                    lines.append(f"  └ Error: {error_preview}")

            await interaction.response.send_message("\n".join(lines), ephemeral=True)

        @tree.command(name="list-games", description="List all monitored games in this server")
        async def list_games(interaction: discord.Interaction) -> None:
            if not await self._check_server_async(interaction):
                return

            games = self.db.get_games_for_server(str(interaction.guild_id))
            if not games:
                await interaction.response.send_message("No games monitored.", ephemeral=True)
                return

            lines = [
                f"• PlaceId **{g.place_id}** → <#{g.channel_id}> (added by <@{g.added_by}>)"
                for g in games
            ]
            await interaction.response.send_message("\n".join(lines), ephemeral=True)

    def _check_server(self, interaction: discord.Interaction) -> bool:
        if not interaction.guild_id:
            return False
        return self.db.is_server_allowed(str(interaction.guild_id))

    async def _check_server_async(self, interaction: discord.Interaction) -> bool:
        if not interaction.guild_id:
            await interaction.response.send_message("This command can only be used in a server.", ephemeral=True)
            return False
        if not self.db.is_server_allowed(str(interaction.guild_id)):
            await interaction.response.send_message(
                "This server is not authorized. Ask a bot admin to run `/approve-server`.",
                ephemeral=True,
            )
            return False
        return True

    async def on_ready(self) -> None:
        log.info("Bot ready as %s", self.user)
        await self.tree.sync()
        log.info("Slash commands synced")

        for game in self.db.get_all_games():
            if self.db.is_server_allowed(game.discord_server_id):
                daemon.start_sync_loop(game, self.config, self.db, self, self.claude)
                log.info("Started sync loop for PlaceId %d", game.place_id)
