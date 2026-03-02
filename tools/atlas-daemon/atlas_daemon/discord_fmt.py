from __future__ import annotations

from typing import TYPE_CHECKING

import discord

if TYPE_CHECKING:
    from .db import Game
    from .review import Issue

SEVERITY_COLORS = {
    "Low": 0x2ECC71,
    "Medium": 0xF1C40F,
    "High": 0xE67E22,
    "Critical": 0xE74C3C,
}


def make_issue_embed(issue: Issue) -> discord.Embed:
    color = SEVERITY_COLORS.get(issue.severity, 0x95A5A6)
    title = f"[{issue.severity}] {issue.title}"
    if len(title) > 256:
        title = title[:253] + "..."

    description_parts = [f"**File:** `{issue.file}`"]
    if issue.line_start:
        if issue.line_end and issue.line_end != issue.line_start:
            description_parts.append(f"**Lines:** {issue.line_start}-{issue.line_end}")
        else:
            description_parts.append(f"**Line:** {issue.line_start}")

    description_parts.append("")
    description_parts.append(issue.explanation)

    if issue.suggestion:
        description_parts.append("")
        suggestion_text = issue.suggestion
        if len(suggestion_text) > 800:
            suggestion_text = suggestion_text[:797] + "..."
        description_parts.append(f"**Suggested fix:**\n```lua\n{suggestion_text}\n```")

    description = "\n".join(description_parts)
    if len(description) > 4096:
        description = description[:4093] + "..."

    return discord.Embed(title=title, description=description, color=color)


def make_summary_embed(
    place_id: int,
    new_issues: list[Issue],
    auto_resolved_count: int,
    changed_files: list[str],
    lines_added: int,
    lines_removed: int,
) -> discord.Embed:
    severity_counts = {"Critical": 0, "High": 0, "Medium": 0, "Low": 0}
    for issue in new_issues:
        severity_counts[issue.severity] = severity_counts.get(issue.severity, 0) + 1

    counts_str = "  |  ".join(
        f"{count} {sev}" for sev, count in severity_counts.items()
    )

    description_parts = [
        f"**{len(new_issues)} new issue{'s' if len(new_issues) != 1 else ''} found**",
    ]
    if auto_resolved_count:
        description_parts[0] += f" | {auto_resolved_count} auto-resolved"

    description_parts.append(f"\n{counts_str}")
    description_parts.append(
        f"\nReviewed {len(changed_files)} changed script{'s' if len(changed_files) != 1 else ''} "
        f"({lines_added} lines added, {lines_removed} removed)"
    )

    return discord.Embed(
        title=f"Sync Review -- PlaceId {place_id}",
        description="\n".join(description_parts),
        color=0x3498DB,
    )


def make_error_embed(place_id: int, error: str, error_type: str, last_sync: str | None) -> discord.Embed:
    description = error
    if len(description) > 1500:
        description = description[:1497] + "..."

    embed = discord.Embed(
        title=f"Sync Error -- PlaceId {place_id}",
        description=f"```\n{description}\n```",
        color=0xE74C3C,
    )
    embed.add_field(name="Error Type", value=error_type, inline=True)
    embed.add_field(name="Last Successful Sync", value=last_sync or "Never", inline=True)
    embed.set_footer(text="Will retry next cycle")
    return embed


class IssueButtons(discord.ui.View):
    def __init__(self, issue_db_id: int) -> None:
        super().__init__(timeout=None)
        self.issue_db_id = issue_db_id

    @discord.ui.button(label="Mark Fixed", style=discord.ButtonStyle.success, custom_id="mark_fixed")
    async def mark_fixed(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        from . import _bot_ref

        if _bot_ref.db is None:
            await interaction.response.send_message("Database not available.", ephemeral=True)
            return

        _bot_ref.db.resolve_issue(self.issue_db_id, resolved_by=str(interaction.user.id))

        embed = interaction.message.embeds[0] if interaction.message and interaction.message.embeds else None
        if embed:
            embed.set_footer(text=f"Resolved by {interaction.user.display_name}")
            embed.color = 0x95A5A6
            if interaction.message:
                await interaction.message.edit(embed=embed, view=None)
        await interaction.response.send_message("Marked as fixed.", ephemeral=True)

    @discord.ui.button(label="Copy Context", style=discord.ButtonStyle.secondary, custom_id="copy_context")
    async def copy_context(self, interaction: discord.Interaction, button: discord.ui.Button) -> None:
        from . import _bot_ref

        if _bot_ref.db is None:
            await interaction.response.send_message("Database not available.", ephemeral=True)
            return

        issue = _bot_ref.db.get_issue_by_message_id(str(interaction.message.id)) if interaction.message else None
        if not issue:
            await interaction.response.send_message("Issue not found.", ephemeral=True)
            return

        context = (
            f"File: {issue.file_path}\n"
            f"Lines: {issue.line_start}-{issue.line_end}\n"
            f"Severity: {issue.severity}\n"
            f"Title: {issue.title}\n\n"
            f"Explanation: {issue.explanation}\n\n"
            f"Suggestion: {issue.suggestion}"
        )
        if len(context) > 1900:
            context = context[:1897] + "..."

        await interaction.response.send_message(f"```\n{context}\n```", ephemeral=True)


def count_diff_lines(diff: str) -> tuple[int, int]:
    added = 0
    removed = 0
    for line in diff.splitlines():
        if line.startswith("+") and not line.startswith("+++"):
            added += 1
        elif line.startswith("-") and not line.startswith("---"):
            removed += 1
    return added, removed
