from __future__ import annotations

import logging
import sys

import click

from .bot import AtlasDaemonBot
from .config import Config
from .db import Database
from .security import init_encryption


@click.command()
@click.option("--discord-token", envvar="ATLAS_DAEMON_DISCORD_TOKEN", default="", help="Discord bot token")
@click.option("--anthropic-key", envvar="ATLAS_DAEMON_ANTHROPIC_KEY", default="", help="Anthropic API key")
@click.option("--data-dir", envvar="ATLAS_DAEMON_DATA_DIR", default="./atlas-daemon-data", help="Data directory")
@click.option("--encryption-key", envvar="ATLAS_DAEMON_ENCRYPTION_KEY", default="", help="Fernet encryption key")
@click.option("--sync-interval", envvar="ATLAS_DAEMON_SYNC_INTERVAL", default=300, type=int, help="Sync interval (seconds)")
@click.option("--atlas-binary", envvar="ATLAS_DAEMON_ATLAS_BINARY", default="atlas", help="Path to atlas binary")
def main(
    discord_token: str,
    anthropic_key: str,
    data_dir: str,
    encryption_key: str,
    sync_interval: int,
    atlas_binary: str,
) -> None:
    """Atlas Daemon - Automated Roblox Code Review Bot"""
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )

    if not discord_token:
        click.echo("Error: Discord token is required (--discord-token or ATLAS_DAEMON_DISCORD_TOKEN)", err=True)
        sys.exit(1)
    if not anthropic_key:
        click.echo("Error: Anthropic key is required (--anthropic-key or ATLAS_DAEMON_ANTHROPIC_KEY)", err=True)
        sys.exit(1)

    config = Config.from_env(
        discord_token=discord_token,
        anthropic_key=anthropic_key,
        data_dir=data_dir,
        encryption_key=encryption_key,
        sync_interval=sync_interval,
        atlas_binary=atlas_binary,
    )

    config.data_dir.mkdir(parents=True, exist_ok=True)

    init_encryption(config.encryption_key)

    db = Database(config.data_dir / "atlas-daemon.db")

    import anthropic as anthropic_mod

    claude = anthropic_mod.AsyncAnthropic(api_key=config.anthropic_key)

    bot = AtlasDaemonBot(config, db, claude)
    bot.run(config.discord_token)


if __name__ == "__main__":
    main()
