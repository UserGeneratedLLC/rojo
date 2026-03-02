---
name: atlas-push
description: Safely push filesystem changes into Roblox Studio via Atlas MCP with automatic conflict merging. Use when the user says "atlas push", "sync to studio", "push to studio", "push changes", or wants to get their local code changes into a running Studio session without clobbering other people's work.
---

# Atlas Push

Automated workflow for pushing local filesystem changes into Roblox Studio. Handles conflicts by fetching the Studio version, merging intelligently, and using optimistic concurrency guards (`studioHash`) to prevent clobbering concurrent edits.

**Goal:** Get OUR changes into Studio while preserving everyone else's work.

## Prerequisites

- `atlas serve` must be running (the Atlas MCP server must be reachable)
- Roblox Studio must be open with the Atlas plugin installed and connected to the MCP stream
- The MCP server identifier is `project-0-rojo-atlas`

If the MCP server is not connected, tell the user to check that `atlas serve` is running and Studio has the Atlas plugin active.

## Workflow

### Step 1: Dryrun

Always start with a dryrun. Never apply changes without reviewing them first.

```
CallMcpTool:
  server: "project-0-rojo-atlas"
  toolName: "atlas_sync"
  arguments: { "mode": "dryrun" }
```

**Parse the response:** The response text contains a human-readable summary followed by a `<json>...</json>` block. Extract and parse the JSON. It has this shape:

```json
{
  "status": "dryrun",
  "changes": [
    {
      "path": "ServerScriptService/Main",
      "direction": "push" | "pull" | "unresolved",
      "id": "32-char-hex-ref",
      "className": "Script",
      "patchType": "Add" | "Edit" | "Remove",
      "studioHash": "40-char-sha1-hex",
      "defaultSelection": "push" | "pull" | null,
      "fsPath": "src/server/Main.server.luau",
      "properties": { "PropName": { "current": ..., "incoming": ... } }
    }
  ]
}
```

**If status is `"empty"`:** No changes to sync. Tell the user and stop.

### Step 2: Triage

Categorize every change entry:

| `direction` | Meaning | Action |
|---|---|---|
| `"push"` | Only local changed | Auto-push (no conflict) |
| `"pull"` | Only Studio changed | Accept pull (preserve their work) |
| `"unresolved"` | Both sides changed | Needs merging |

Also note the `patchType`:
- `"Edit"` on a script class (`Script`, `LocalScript`, `ModuleScript`) -- mergeable via `get_script`
- `"Edit"` on a non-script -- property conflict, resolve by examining `properties` field
- `"Add"` / `"Remove"` -- structural, usually one-directional; if unresolved, needs judgment

**If there are zero unresolved changes:** Skip to Step 5 (build overrides for all pushes and proceed).

### Step 3: Merge Unresolved Scripts

For each unresolved change where `className` is a script type (`Script`, `LocalScript`, `ModuleScript`) and `patchType` is `"Edit"`:

1. **Fetch Studio source:**

```
CallMcpTool:
  server: "project-0-rojo-atlas"
  toolName: "get_script"
  arguments: { "id": "<id from the change entry>" }
```

Response includes `source` (Studio's current code), `studioHash` (SHA1 of git blob format), `fsPath`, and `id`. Save the `studioHash` -- you need it for the override.

2. **Read local file:** Use the `fsPath` from the change entry to read the local filesystem version.

3. **Merge:** Produce a merged version that incorporates both local and Studio changes. Rules:
   - **Our changes take priority** -- the user invoked atlas-push because they want their local edits in Studio.
   - **Preserve Studio-only additions** -- if Studio has new code that doesn't conflict with our changes (new functions, new event handlers, new variables in non-overlapping regions), keep them.
   - **For overlapping edits** (both sides changed the same lines/function): prefer our version, but inspect whether the Studio version added something important that ours removed. Use your code understanding to produce a correct result.

4. **Write merged result** to the local filesystem path (overwrite the file). This is what will be pushed to Studio.

### Step 4: Resolve Non-Script and Structural Conflicts

For unresolved changes that are NOT script edits:

- **Non-script property edits:** Examine the `properties` field (has `current` and `incoming` values). Push our version (direction: `"push"`) unless the property is something we didn't intentionally change.
- **Add (unresolved):** If we added something locally and Studio also added something -- push ours.
- **Remove (unresolved):** If we removed something but Studio still has it -- push the removal if it was intentional. If unclear, ask the user.

### Step 4b: Post-Merge Semantic Review

After producing each merged script, review the result for functional issues the merge may have introduced:

- **Duplicate definitions:** Same variable or function defined twice
- **Broken requires:** `require()` paths that reference moved/renamed modules
- **Signature mismatches:** Function gained a parameter on one side but callers on the other side don't pass it
- **Dangling references:** Variables removed by one side but still used by the other
- **Conflicting logic:** Same config value changed to different things; early-return added that skips new code from the other side

**If you spot a clear issue:** Fix it in the merged file before proceeding.

**If the issue is ambiguous** (unclear which side's intent should win): Ask the user. Show both versions of the conflicting section, explain the issue concisely, and ask what they want. Apply their answer and continue immediately.

### Step 4c: Escalation Rules

Throughout Steps 3-4b, if you encounter a conflict you genuinely cannot resolve:

1. Show the user both versions (local and Studio) of the conflicting section
2. Explain the conflict in one or two sentences
3. Ask the user what to do -- use plain text, NOT multiple-choice, so they can give nuanced instructions (e.g. "keep Studio's version of the damage calc but use my version of the config table")
4. Apply their answer immediately
5. Continue to the next conflict or proceed to sync

**Stay in the loop.** The user should never need to re-invoke this skill. Keep asking and applying until all conflicts are resolved.

### Step 5: Build Overrides

Construct the overrides array for the final sync. For every change you want to push:

```json
{
  "id": "<32-char hex id from dryrun>",
  "direction": "push",
  "studioHash": "<studioHash from get_script response>"
}
```

- **Merged scripts:** Use the `studioHash` you got from `get_script` in Step 3. This is the safety guard -- if someone edited the script between your read and now, the override will be rejected instead of silently clobbering.
- **Clean pushes** (direction was already `"push"` in dryrun): Include them in overrides with `direction: "push"`. The `studioHash` from the dryrun change entry can be used if present.
- **Clean pulls** (direction was `"pull"`): Do NOT include these in overrides. Let them resolve naturally (Studio version wins).

### Step 6: Final Sync

```
CallMcpTool:
  server: "project-0-rojo-atlas"
  toolName: "atlas_sync"
  arguments: {
    "mode": "standard",
    "overrides": [ <your overrides array> ]
  }
```

### Step 7: Verify and Retry

Parse the response status:

| Status | Meaning | Action |
|---|---|---|
| `"success"` | All changes applied | Done. Tell user sync succeeded. |
| `"empty"` | Nothing to sync | Done. Already up to date. |
| `"rejected"` | User rejected in Studio UI | Tell user it was rejected. |
| `"fastfail_unresolved"` | Unresolved changes remain | Some overrides didn't match or new conflicts appeared. Loop back to Step 1 with a new dryrun. |

**If an override was rejected** (studioHash mismatch = someone edited the script concurrently):
1. Re-fetch the script via `get_script`
2. Re-merge with the new Studio version
3. Write the updated merge to the filesystem
4. Build new overrides with the fresh `studioHash`
5. Retry the sync

**Keep retrying** until sync succeeds or user explicitly says to stop. Each retry is safe because `studioHash` prevents clobbering.

## Safety Invariants

1. **Never sync without a dryrun first** -- always review what will change.
2. **Always use `studioHash` on overrides** -- this is the optimistic concurrency guard. If the hash doesn't match (someone edited between our read and our push), the override is rejected rather than silently overwriting.
3. **Never discard Studio-only changes silently** -- if Studio has changes we didn't make, either merge them in or explicitly acknowledge discarding them.
4. **Merged files are written to the local filesystem, then pushed** -- we never send content directly to Studio. Atlas handles the actual transfer.
5. **Retry on concurrency failures** -- a rejected studioHash means "try again", not "give up".

## Error Handling

| Error | Recovery |
|---|---|
| MCP server not connected | Tell user to start `atlas serve` and ensure Studio plugin is active |
| Plugin not connected | Tell user to open Studio with Atlas plugin |
| `sync_in_progress` | Wait a moment and retry |
| `already_connected` | Live sync is active; changes sync automatically; no manual push needed |
| `get_script` fails | Fall back to pushing local version only; warn user that Studio version couldn't be read |
| Filesystem write fails | Abort and report error; do not proceed with sync |

## Quick Reference: MCP Tool Calls

**Dryrun:**
```
server: "project-0-rojo-atlas", toolName: "atlas_sync", arguments: { "mode": "dryrun" }
```

**Get script:**
```
server: "project-0-rojo-atlas", toolName: "get_script", arguments: { "id": "<hex>" }
```

**Get script by path (alternative):**
```
server: "project-0-rojo-atlas", toolName: "get_script", arguments: { "fsPath": "src/server/Main.server.luau" }
```

**Sync with overrides:**
```
server: "project-0-rojo-atlas", toolName: "atlas_sync", arguments: { "mode": "standard", "overrides": [...] }
```
