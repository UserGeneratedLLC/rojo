from __future__ import annotations

import asyncio
import logging
from pathlib import Path

log = logging.getLogger(__name__)


async def _run(
    *cmd: str, cwd: Path | None = None, timeout: float = 300
) -> tuple[str, str]:
    proc = await asyncio.create_subprocess_exec(
        *cmd,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        cwd=str(cwd) if cwd else None,
    )
    stdout_bytes, stderr_bytes = await asyncio.wait_for(
        proc.communicate(), timeout=timeout
    )
    stdout = stdout_bytes.decode(errors="replace") if stdout_bytes else ""
    stderr = stderr_bytes.decode(errors="replace") if stderr_bytes else ""

    if proc.returncode != 0:
        raise RuntimeError(
            f"Command failed (exit {proc.returncode}): {' '.join(cmd)}\n{stderr}"
        )
    return stdout, stderr


async def atlas_clone(
    atlas_binary: str,
    place_id: int,
    path: Path,
    opencloud_key: str,
) -> None:
    log.info("Cloning place %d into %s", place_id, path)
    await _run(
        atlas_binary,
        "--opencloud",
        opencloud_key,
        "clone",
        str(place_id),
        "--path",
        str(path),
        "--skip-git",
        "--skip-rules",
        timeout=600,
    )


async def atlas_syncback(
    atlas_binary: str,
    working_dir: Path,
    place_id: int,
    opencloud_key: str,
) -> str:
    log.info("Syncing place %d in %s", place_id, working_dir)
    stdout, _ = await _run(
        atlas_binary,
        "--opencloud",
        opencloud_key,
        "syncback",
        "--download",
        str(place_id),
        "--working-dir",
        str(working_dir),
        "--incremental",
        "--list",
        timeout=600,
    )
    return stdout


async def git_init(cwd: Path) -> None:
    await _run("git", "init", cwd=cwd)


async def git_add_all(cwd: Path) -> None:
    await _run("git", "add", ".", cwd=cwd)


async def git_commit(cwd: Path, message: str) -> None:
    await _run("git", "commit", "-m", message, "--allow-empty", cwd=cwd)


async def git_diff(cwd: Path) -> str:
    try:
        stdout, _ = await _run("git", "diff", "HEAD", cwd=cwd)
        return stdout
    except RuntimeError:
        return ""


async def git_diff_name_only(cwd: Path) -> list[str]:
    try:
        stdout, _ = await _run("git", "diff", "HEAD", "--name-only", cwd=cwd)
        return [line.strip() for line in stdout.splitlines() if line.strip()]
    except RuntimeError:
        return []
