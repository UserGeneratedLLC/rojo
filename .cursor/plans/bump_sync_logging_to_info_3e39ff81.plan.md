---
name: Bump sync logging to info
overview: Promote all sync-related change logging in the Rojo Studio plugin from Debug/Trace to Info level, so that every change received, confirmed, sent, and applied is visible in the Studio output console at the default log level.
todos:
  - id: log-padding
    content: Remove 15-space padding from log tags in plugin/log/init.lua (TRACE_TAG, INFO_TAG, DEBUG_TAG)
    status: completed
  - id: serve-session
    content: Bump Debug/Trace to Info in ServeSession.lua (received, confirmed, applied summaries + per-item decisions). Add combined patch summary after WebSocket message combining.
    status: completed
  - id: change-batcher
    content: Bump batch summary in ChangeBatcher/init.lua line 115 from Debug to Info
    status: completed
  - id: api-context
    content: Bump sending-to-server logs in ApiContext.lua from Debug/Trace to Info (summary + per-item operations)
    status: completed
  - id: apply-patch
    content: Bump patch application logs in Reconciler/applyPatch.lua from Debug to Info (summary + per-update details)
    status: completed
isProject: false
---

# Bump Sync Change Logging to Info

## Current State

The plugin's `Log` module (at `plugin/log/init.lua`) supports levels: Error(0), Warning(1), Info(2), Debug(3), Trace(4). Default log level is `"Info"`, meaning Debug and Trace messages are hidden unless the user manually changes the setting.

Most sync-flow messages are currently at Debug or Trace, so users can't see what changes are being received/sent/confirmed without digging into settings.

## Files to Change

### 0. [plugin/log/init.lua](plugin/log/init.lua) -- Fix excessive log padding

The log tags have 15 hardcoded spaces prepended for alignment with Studio's timestamp column, but it creates absurdly wide output -- especially for multi-line messages where `addTags` replaces every `\n` with `\n` + the full padded tag.

```lua
-- Current (lines 15-16) -- repeats tag on every line of multi-line messages:
local function addTags(tag, message)
    return tag .. message:gsub("\n", "\n" .. tag)
end

-- Fix: just prepend the tag once
local function addTags(tag, message)
    return tag .. message
end
```

```lua
-- Current (lines 19-21) -- 15 spaces of padding before each tag:
local TRACE_TAG = (" "):rep(15) .. "[Rojo-Trace] "
local INFO_TAG = (" "):rep(15) .. "[Rojo-Info] "
local DEBUG_TAG = (" "):rep(15) .. "[Rojo-Debug] "

-- Fix: remove the padding
local TRACE_TAG = "[Rojo-Trace] "
local INFO_TAG = "[Rojo-Info] "
local DEBUG_TAG = "[Rojo-Debug] "
```

Note: `WARN_TAG` on line 22 already has no padding (it goes through `warn()` which has its own formatting).

### 1. [plugin/src/ServeSession.lua](plugin/src/ServeSession.lua) -- Received / Confirmed / Applied

**Received from server (WebSocket):**

- Line 222: `Log.debug` -> `Log.info` -- "Received {} messages from Rojo server"
- After line 228 (after combining messages into `combinedPatch`): add an `Log.info` summary of what's IN the combined patch (removals, additions, updates counts), similar to how `applyPatch.lua` does it. Currently the contents are only visible at Trace via the raw packet dump.

**Confirmation selections:**

- Line 699: `Log.debug` -> `Log.info` -- "User selections: {} push, {} pull, {} ignored"
- Line 713: `Log.trace` -> `Log.info` -- "[Push] Update: {}"
- Line 726: `Log.trace` -> `Log.info` -- "[Pull] Update: {}"
- Line 757: `Log.trace` -> `Log.info` -- "[Push] Delete: {}"
- Line 792: `Log.trace` -> `Log.info` -- "[Push] Add: {}.{}"
- Lines 772, 797, 828 are already Info -- no change needed

**Applied to Studio (summary):**

- Line 813: `Log.debug` -> `Log.info` -- "Applying to Studio: {} additions, {} removals, {} updates"
- Line 583: `Log.debug` -> `Log.info` -- unapplied patch summary (important for diagnosing sync issues)

### 2. [plugin/src/ChangeBatcher/init.lua](plugin/src/ChangeBatcher/init.lua) -- Outgoing batched changes

- Line 115: `Log.debug` -> `Log.info` -- "Two-way sync: {} updates, {} additions, {} removals"
- Lines 70, 73 stay at Trace -- these fire per-property-per-frame and would flood the console if promoted

### 3. [plugin/src/ApiContext.lua](plugin/src/ApiContext.lua) -- Sent to server

- Line 202: `Log.debug` -> `Log.info` -- "Sending to server: {} removals, {} additions, {} updates"
- Line 212: `Log.trace` -> `Log.info` -- "[Syncback] Remove ID: {}"
- Line 218: `Log.trace` -> `Log.info` -- "[Syncback] Add {} ({})"
- Line 228: `Log.trace` -> `Log.info` -- "[Syncback] Update ID {} ({} properties)"

### 4. [plugin/src/Reconciler/applyPatch.lua](plugin/src/Reconciler/applyPatch.lua) -- Applied to Studio (per-item)

- Line 36: `Log.debug` -> `Log.info` -- "Applying patch to Studio: {} removals, {} additions, {} updates"
- Line 141: `Log.debug` -> `Log.info` -- "[Studio] Update: {} - {}"
- Lines 56, 106 are already Info -- no change needed

## What stays at Debug/Trace (intentionally)

- ChangeBatcher per-property detection (lines 70, 73) -- fires every frame, too noisy
- Raw WebSocket packet dump (ApiContext line 299) -- huge payloads, not useful for comparison
- Precommit/postcommit hook registration (ServeSession lines 182, 189, 203, 210) -- lifecycle plumbing, not sync data
- Internal fallback/replace logic (ServeSession lines 561, 566) -- internal recovery mechanism

## Summary of Sync Flow Visibility After Changes

```
Server → Plugin:  "Received N messages" + combined patch summary (counts)
Confirmation:     "User selections: X push, Y pull, Z ignored" + per-item decisions
Apply to Studio:  "Applying patch: N removals, M additions, K updates" + per-item details
Studio → Server:  "Two-way sync: X updates, Y additions, Z removals" (batch summary)
Send to Server:   "Sending to server: N removals, M additions, K updates" + per-item details
```

All of these will now appear in the Studio output console at the default Info log level.