from __future__ import annotations

import asyncio
import json
import logging
import re
from dataclasses import dataclass
from importlib import resources
from typing import TYPE_CHECKING

import anthropic

if TYPE_CHECKING:
    from .db import DBIssue

log = logging.getLogger(__name__)

REVIEWABLE_EXTENSIONS = (".luau", ".lua")
TOKEN_ESTIMATE_DIVISOR = 4
MAX_SINGLE_PASS_TOKENS = 150_000


@dataclass
class Issue:
    file: str
    line_start: int
    line_end: int
    severity: str
    title: str
    explanation: str
    suggestion: str


def _load_system_prompt() -> str:
    return resources.files("atlas_daemon").joinpath("review_prompt.txt").read_text()


def split_diff_by_file(raw_diff: str) -> list[str]:
    chunks: list[str] = []
    current: list[str] = []

    for line in raw_diff.splitlines(keepends=True):
        if line.startswith("diff --git "):
            if current:
                chunks.append("".join(current))
            current = [line]
        else:
            current.append(line)

    if current:
        chunks.append("".join(current))

    return chunks


def extract_file_path(file_diff: str) -> str:
    for line in file_diff.splitlines():
        if line.startswith("+++ b/"):
            return line[6:].strip()
        if line.startswith("+++ "):
            return line[4:].strip()
    return ""


def filter_script_changes(raw_diff: str) -> str:
    filtered = []
    for chunk in split_diff_by_file(raw_diff):
        path = extract_file_path(chunk)
        if any(path.endswith(ext) for ext in REVIEWABLE_EXTENSIONS):
            filtered.append(chunk)
    return "\n".join(filtered)


def format_existing_issues(existing: list[DBIssue]) -> str:
    if not existing:
        return ""
    lines = ["Already-reported unresolved issues (do NOT re-report these):"]
    for e in existing:
        lines.append(f"- [{e.severity}] {e.file_path}:{e.line_start} -- {e.title}")
    return "\n".join(lines)


def parse_issues(text: str) -> list[Issue]:
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```\w*\n?", "", text)
        text = re.sub(r"\n?```$", "", text)
        text = text.strip()

    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        match = re.search(r"\[.*\]", text, re.DOTALL)
        if match:
            try:
                data = json.loads(match.group())
            except json.JSONDecodeError:
                log.warning("Could not parse review response as JSON")
                return []
        else:
            log.warning("No JSON array found in review response")
            return []

    if not isinstance(data, list):
        return []

    issues: list[Issue] = []
    for item in data:
        if not isinstance(item, dict):
            continue
        try:
            issues.append(
                Issue(
                    file=str(item.get("file", "")),
                    line_start=int(item.get("line_start", 0)),
                    line_end=int(item.get("line_end", 0)),
                    severity=str(item.get("severity", "Medium")),
                    title=str(item.get("title", "")),
                    explanation=str(item.get("explanation", "")),
                    suggestion=str(item.get("suggestion", "")),
                )
            )
        except (ValueError, TypeError):
            continue

    return issues


async def review_diff(
    client: anthropic.AsyncAnthropic,
    diff: str,
    existing_issues: list[DBIssue],
) -> list[Issue]:
    script_diff = filter_script_changes(diff)
    if not script_diff:
        return []

    existing_context = format_existing_issues(existing_issues)

    token_estimate = len(script_diff) // TOKEN_ESTIMATE_DIVISOR
    if token_estimate > MAX_SINGLE_PASS_TOKENS:
        return await _review_chunked(client, script_diff, existing_context)

    user_content = f"{existing_context}\n\nReview this diff:\n\n{script_diff}" if existing_context else f"Review this diff:\n\n{script_diff}"

    try:
        response = await client.messages.create(
            model="claude-sonnet-4-20250514",
            max_tokens=4096,
            system=_load_system_prompt(),
            messages=[{"role": "user", "content": user_content}],
        )
        return parse_issues(response.content[0].text)
    except Exception:
        log.exception("Claude API call failed")
        raise


async def _review_chunked(
    client: anthropic.AsyncAnthropic,
    diff: str,
    existing_context: str,
) -> list[Issue]:
    file_diffs = split_diff_by_file(diff)
    file_diffs = [
        fd for fd in file_diffs
        if any(extract_file_path(fd).endswith(ext) for ext in REVIEWABLE_EXTENSIONS)
    ]

    async def review_one(file_diff: str) -> list[Issue]:
        user_content = f"{existing_context}\n\nReview this diff:\n\n{file_diff}" if existing_context else f"Review this diff:\n\n{file_diff}"
        try:
            response = await client.messages.create(
                model="claude-sonnet-4-20250514",
                max_tokens=4096,
                system=_load_system_prompt(),
                messages=[{"role": "user", "content": user_content}],
            )
            return parse_issues(response.content[0].text)
        except Exception:
            log.exception("Claude API call failed for chunked review")
            return []

    results = await asyncio.gather(*[review_one(fd) for fd in file_diffs])
    return [issue for file_issues in results for issue in file_issues]
