---
name: atlas-playtest
description: Quick playtest to catch runtime errors after code changes. Starts a Roblox Studio play session via Atlas MCP, waits for scripts to initialize, collects errors, then stops. Use when the user says "atlas playtest", "playtest", "test in studio", "check for errors", "run the game", or after pushing code to Studio.
---

# Atlas Playtest

Starts a play session in Roblox Studio, waits for scripts to boot, collects runtime errors, then stops. Designed to be fast -- catch obvious breakage, not exhaustive QA.

## Prerequisites

- `atlas serve` must be running with the Atlas MCP server reachable
- Roblox Studio must be open with the Atlas plugin connected
- MCP server identifier: `project-0-rojo-atlas`

## Workflow

### Step 1: Run Play Session

```
CallMcpTool:
  server: "project-0-rojo-atlas"
  toolName: "run_script_in_play_mode"
  arguments: {
    "code": "task.wait(7)\nprint(\"boot complete\")",
    "mode": "start_play",
    "timeout": 15
  }
```

This starts a play session, waits 7 seconds for all scripts to initialize and error if they're going to, then stops automatically.

**If it returns "Previous call to start play session has not been completed":** call `start_stop_play` with `mode: "stop"` first, wait a moment, then retry.

### Step 2: Parse Response

The response contains structured JSON:

```json
{
  "success": true,
  "value": "boot complete",
  "error": null,
  "logs": [{ "level": "output", "message": "...", "ts": 0.0 }],
  "errors": [{ "level": "error", "message": "...", "ts": 0.0 }],
  "duration": 7.1,
  "isTimeout": false
}
```

- `errors` array: contains all `"error"` and `"warning"` level messages from the session
- `logs` array: all messages (including output/info)

### Step 3: Filter Errors

From the `errors` array, keep:

- **All errors** (level `"error"`): runtime errors, require failures, nil indexing, type mismatches, missing modules, etc. These always matter.
- **Severely problematic warnings** (level `"warning"`): only keep warnings that indicate real breakage:
  - "Infinite yield possible" (waiting on something that will never resolve)
  - Security/permission errors
  - Warnings that indicate imminent failure

**Ignore:**
- Routine deprecation notices
- Asset loading warnings
- Cosmetic/informational warnings
- Any message starting with `[Atlas]` (internal plugin messages, already filtered by the tool)

### Step 4: Diagnose and Fix

**If no relevant errors:** Report success. Done.

**If errors found:**

1. **Parse each error message** for the script path. Roblox errors follow the pattern:
   `ServiceName.Path.To.Script:LineNumber: error message`
   e.g. `ServerScriptService.GameManager.PlayerHandler:42: attempt to index nil with 'Character'`

2. **Map the script path to a filesystem path.** The service path segments correspond to the Atlas project tree. Use `get_script` with the instance path if you need the `fsPath`, or infer it from the project structure.

3. **Read the erroring script** from the filesystem.

4. **Diagnose the error** using the error message and line number. Common patterns:
   - `attempt to index nil` -- variable is nil when it shouldn't be
   - `attempt to call a nil value` -- function doesn't exist or module didn't return it
   - `Module code did not return exactly one value` -- bad module return
   - `Requested module experienced an error while loading` -- cascading require failure
   - `X is not a valid member of Y` -- wrong property/child name

5. **Fix the code** and write the corrected file.

6. **If called standalone** (not from atlas-push): tell the user what you fixed. They'll need to sync the fix to Studio themselves.

7. **If the error is ambiguous** or you can't determine the fix: ask the user. Show the error, the relevant code, and ask what they want to do via plain text.

### Plan Mode Behavior

If you're in plan mode when this skill triggers, do NOT run the playtest. Instead, create a plan that:
1. Lists the MCP call to make
2. Notes that errors will need to be diagnosed and fixed
3. Outlines the fix loop

## Quick Reference

**Run playtest:**
```
server: "project-0-rojo-atlas", toolName: "run_script_in_play_mode"
arguments: { "code": "task.wait(7)\nprint(\"boot complete\")", "mode": "start_play", "timeout": 15 }
```

**Stop play (if stuck):**
```
server: "project-0-rojo-atlas", toolName: "start_stop_play", arguments: { "mode": "stop" }
```

**Get script by path (to find fsPath for an erroring script):**
```
server: "project-0-rojo-atlas", toolName: "get_script", arguments: { "fsPath": "src/server/MyScript.server.luau" }
```
