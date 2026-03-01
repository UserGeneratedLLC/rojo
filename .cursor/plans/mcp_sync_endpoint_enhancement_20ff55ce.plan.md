---
name: MCP sync endpoint enhancement
overview: Enhance the `atlas_sync` MCP tool to support modes (standard/manual/fastfail/dryrun), agent-specified override auto-accepts with hash + property verification, and enriched change metadata (studioHash, property before/after values, className, patchType, fsPath) in the response.
todos:
  - id: rust-structs
    content: Add SyncOverride struct (id-based matching, with expectedProperties), expand McpSyncCommand (mode, overrides), expand SyncChange (id, className, patchType, studioHash, defaultSelection, fsPath, properties) in mcp.rs
    status: completed
  - id: rust-handler
    content: Update handle_atlas_sync to parse mode/overrides from params, include in command, handle new statuses (dryrun, fastfail_unresolved), emit structured JSON in response
    status: completed
  - id: rust-tool-schema
    content: Update atlas_sync inputSchema in handle_tools_list with mode and overrides parameters
    status: completed
  - id: rust-api-enrich
    content: In api.rs MCP stream handler, enrich changes with fsPath by looking up Ref in tree's InstigatingSource::Path after receiving plugin result. Handle Ref::from_str failures gracefully (plugin-generated GUIDs for removed nodes).
    status: completed
  - id: plugin-mcpstream
    content: Update McpStream.lua to forward mode and overrides from sync command to onSyncCommand callback
    status: completed
  - id: plugin-startmcpsync
    content: Update startMcpSync in App/init.lua to accept mode/overrides, implement mode logic (manual/fastfail/dryrun) and id-based override matching with studioHash + expectedProperties verification (decodeValue before trueEquals). Thread patch/instanceMap to _buildMcpChangeList.
    status: completed
  - id: plugin-changelist
    content: Expand _buildMcpChangeList signature to receive patch + instanceMap. Encode properties using raw changedProperties (incoming) and encodeProperty via descriptor lookup (current). Skip Source for scripts. Handle encode failures gracefully.
    status: completed
  - id: rust-tests
    content: Update existing mcp.rs tests and add new tests for modes, overrides, enriched SyncChange fields, and new statuses
    status: completed
  - id: test-suite
    content: Full coverage test suite. Rust side - wire type serde for all new structs, mode passthrough, fsPath enrichment (valid Ref, invalid Ref, ProjectNode, missing instance), JSON response block parsing. Plugin side - _buildMcpChangeList property encoding (scripts skip Source, encode failures, multiple property types), override verification (studioHash match/mismatch, expectedProperties match/mismatch/decode, stale id), mode behavior (standard fast-forward, standard user-review, manual always-review, fastfail unresolved, fastfail all-resolved, dryrun returns without applying), end-to-end MCP stream forwarding of mode/overrides.
    status: completed
isProject: false
---

# MCP Sync Endpoint Enhancement

## Context

The current `atlas_sync` tool takes no parameters and has one behavior: if all changes are git-auto-selected, fast-forward; otherwise show Studio UI and block. The agent has no way to control this, no way to auto-accept specific changes, and gets minimal metadata back.

The enhanced endpoint enables an agent workflow like:

1. `atlas_sync(mode: "fastfail")` -- get changes with hashes, fail if any are unresolved
2. `get_script(id)` -- read conflicting script from Studio (future endpoint, uses `id` from sync response)
3. Merge changes, write to filesystem
4. `atlas_sync(overrides: [{id, direction, studioHash}])` -- auto-accept with hash verification

## New Input Schema

```json
{
  "mode": "standard",
  "overrides": [
    {
      "id": "00abcdef01234567890abcdef0123456",
      "direction": "push",
      "studioHash": "a1b2c3...",
      "expectedProperties": {
        "Disabled": false
      }
    },
    {
      "id": "00abcdef01234567890abcdef0123457",
      "direction": "push",
      "expectedProperties": {
        "Position": {"Vector3": [0, 5, 0]},
        "Anchored": true
      }
    }
  ]
}
```

- **mode**: `"standard"` (default) | `"manual"` | `"fastfail"` | `"dryrun"`
- **overrides**: array of auto-accept directives per instance:
  - `id` (required): the server Ref (32-char hex) from a previous sync response. Matched directly via `patchTree:getNode(id)` -- O(1), no ambiguity possible.
  - `direction` (required): `"push"` or `"pull"`
  - `studioHash` (scripts only): SHA1 of git blob of Source, verifies Source hasn't changed since agent last inspected
  - `expectedProperties` (optional): map of property name to expected current Studio value. Plugin verifies each against the live instance. Uses RbxDom encoded format (same as response). If any mismatch, the override is rejected and the change becomes unresolved.

### Why `id` instead of `path`

Instance names are not unique in Roblox -- a parent can have multiple children with the same name. Paths built from `instance.Name` segments (e.g. `Workspace/Part`) are ambiguous when duplicates exist. The server Ref (`id`) is a unique 128-bit identifier per instance, assigned by the server and round-tripped through all responses. Using it eliminates the path ambiguity problem entirely on both the response and override sides, with no need for path escaping or debugId disambiguation.

The `path` field in responses remains for human readability (agents can display it, use it in logs). The `fsPath` field provides the unique Atlas filesystem path for file editing. Neither is used for override matching.

## New Response Change Schema

Currently each change is `{path, direction}`. Enhanced:

```json
{
  "path": "ServerScriptService/MyScript",
  "id": "00abcdef...",
  "direction": "push",
  "patchType": "Edit",
  "className": "Script",
  "studioHash": "a1b2c3...",
  "defaultSelection": "push",
  "fsPath": "src/server/MyScript.server.luau",
  "properties": {
    "Disabled": {
      "current": false,
      "incoming": true
    }
  }
}
```

For a non-script instance with property changes:

```json
{
  "path": "Workspace/MyPart",
  "id": "00abcdef...",
  "direction": "push",
  "patchType": "Edit",
  "className": "Part",
  "defaultSelection": "pull",
  "fsPath": "src/workspace/MyPart.model.json5",
  "properties": {
    "Position": {
      "current": {"Vector3": [0, 5, 0]},
      "incoming": {"Vector3": [10, 5, 0]}
    },
    "Anchored": {
      "current": false,
      "incoming": true
    }
  }
}
```

- `studioHash`: SHA1 of `"blob <len>\0<source>"` (git blob format) of the script's Source in Studio. Only present for script classes. Used by agents for override verification.
- `fsPath`: Atlas filesystem path relative to project root. Enriched server-side from `InstigatingSource::Path`. Null for project-node instances or additions.
- `defaultSelection`: what git auto-selected (`"push"` / `"pull"` / null)
- `properties`: map of changed property name to `{current, incoming}` values in RbxDom encoded format. `current` is the live Studio value, `incoming` is the Atlas value. For scripts, Source is omitted from `properties` (use `studioHash` + `get_script` instead to avoid massive payloads). Only present for Edit patchType.

## Mode Behavior (plugin confirm callback)


| Mode       | Behavior                                                                                                                  |
| ---------- | ------------------------------------------------------------------------------------------------------------------------- |
| `standard` | Current: fast-forward if all pre-selected, otherwise show UI                                                              |
| `manual`   | Always show UI, never auto-accept                                                                                         |
| `fastfail` | If any change has null `defaultSelection` AND no matching override, resolve immediately with `fastfail_unresolved` status |
| `dryrun`   | Compute full change list with hashes, resolve immediately, abort session without applying                                 |


## Override Matching and Verification (plugin side)

For each override, the plugin:

1. **Match by `id`**: Look up the PatchTree node via `patchTree:getNode(override.id)`. If not found (stale id from a previous session), skip the override.
2. **studioHash verification** (scripts): If `override.studioHash` is provided, compute SHA1 of git blob of `node.instance.Source` and compare. Mismatch = override rejected (someone changed the script since the agent last looked).
3. **expectedProperties verification** (any instance): If `override.expectedProperties` is provided, for each property in the map:
  a. **Decode** the expected value via `decodeValue(expectedValue, instanceMap)` -- the override arrives as RbxDom-encoded JSON (e.g. `{"Vector3": [0, 5, 0]}`), which must be decoded to a live Roblox value before comparison.
   b. Read the current Studio value via `getProperty(node.instance, propName)`.
   c. Compare using `trueEquals(decodedExpected, currentValue)` -- fuzzy float comparison, EnumItem handling, etc.
   d. Any mismatch = override rejected (someone changed a property since the agent last looked).
4. If all verifications pass, set the node's selection to `override.direction`, treating it as pre-selected for mode logic.
5. If verification fails, the node remains unresolved. In `fastfail` mode this means the sync fails immediately with the unresolved changes listed. In `standard` mode the user sees them in the Studio UI.

This design ensures: no path ambiguity (id is unique), no path escaping needed, O(1) node lookup, and the agent never silently overwrites concurrent changes from other developers.

## Files to Change

### Rust: `[src/web/mcp.rs](src/web/mcp.rs)`

- Add `SyncOverride` struct: `{id, direction, studio_hash?, expected_properties?}`
- Expand `McpSyncCommand`: add `mode: String`, `overrides: Vec<SyncOverride>`
- Expand `SyncChange`: add `id`, `class_name`, `patch_type`, `studio_hash`, `default_selection`, `fs_path`, `properties` (all optional)
- Update `atlas_sync` tool definition in `handle_tools_list` with new inputSchema
- Update `handle_atlas_sync` to parse mode + overrides from params and include them in the command
- Handle new statuses (`fastfail_unresolved`, `dryrun`) in response formatting
- Update `handle_atlas_sync` response text to include richer change info (structured JSON instead of plain text list)
- Update tests

### Rust: `[src/web/api.rs](src/web/api.rs)` (MCP stream handler)

- Serialize mode + overrides into the WebSocket command to the plugin
- After receiving `McpSyncResult`, enrich each change's `fs_path`:
  - Attempt to parse `change.id` as `Ref` (32-char hex, via `Ref::from_str`). If parse fails (e.g. plugin-generated GUID for removed nodes), leave `fs_path = None` and continue.
  - Look up `tree.get_instance(ref).metadata.instigating_source`
  - If `InstigatingSource::Path(p)`, strip project root prefix, set `fs_path`
  - If instance not found or source is `ProjectNode`, leave `fs_path = None`
- Forward enriched result via oneshot

### Plugin: `[plugin/src/McpStream.lua](plugin/src/McpStream.lua)`

- Forward `data.mode` and `data.overrides` from the sync command to `self._onSyncCommand(data.requestId, data.mode, data.overrides)`

### Plugin: `[plugin/src/App/init.lua](plugin/src/App/init.lua)`

`**startMcpSync(requestId, mode, overrides)`:**

- Accept `mode` and `overrides` parameters
- In the confirm callback (which receives `instanceMap, patch, serverInfo`), after building PatchTree:
  - Apply overrides: for each override, look up node by `patchTree:getNode(override.id)`, verify studioHash for scripts, verify expectedProperties via `decodeValue` + `trueEquals`, set selection
  - `dryrun`: build full change list via `_buildMcpChangeList(patchTree, patch, instanceMap, nil, true)`, resolve with `status: "dryrun"`, return `"Abort"`
  - `fastfail`: check if any node is unresolved after defaults + overrides; if so, resolve with `fastfail_unresolved` + full change list, return `"Abort"`
  - `manual`: force into user-review flow (skip the `allPreSelected` fast-forward)
  - `standard`: current behavior
- All calls to `_buildMcpChangeList` in this function must pass `patch` and `instanceMap` (available as closure variables from the confirm callback)

`**_buildMcpChangeList(patchTree, patch, instanceMap, selections, includeAll)`:**

Signature change: receives the original `patch` object and `instanceMap` in addition to the tree, because:

- `node.changeList` stores **decoded** Roblox values (Vector3 userdata, EnumItem, etc.) that cannot be JSON-serialized
- We need the raw `patch.updated[].changedProperties` for incoming values (already in RbxDom-encoded wire format)
- We need `encodeProperty(instance, propName, descriptor)` to encode current Studio values into the same wire format
- Both require looking up the `RbxDom.findCanonicalPropertyDescriptor(instance.ClassName, propName)` for each property

Implementation:

- Add `includeAll` parameter (for dryrun/fastfail: include all changes regardless of selection)
- Add `className = node.className`
- Add `patchType = node.patchType`
- Add `id = node.id` (server Ref, for fsPath enrichment). Note: for Removed nodes, this may be a plugin-generated GUID rather than a server Ref -- the Rust `fsPath` enrichment must handle `Ref::from_str` parse failures gracefully (set `fsPath = null`).
- Add `defaultSelection = node.defaultSelection`
- For script instances (`node.instance:IsA("LuaSourceContainer")`): compute and add `studioHash` = SHA1 of git blob of `instance.Source`
- Add `properties` map for Edit nodes: build a `patchLookup` from `patch.updated` keyed by `change.id`, then for each property in `patchLookup[node.id].changedProperties`:
  - **Skip** `Source` for script classes (handled by `studioHash` + future `get_script`)
  - `incoming`: use the raw value from `changedProperties[propName]` (already RbxDom-encoded wire format, JSON-safe)
  - `current`: look up descriptor via `RbxDom.findCanonicalPropertyDescriptor(node.className, propName)`, then call `encodeProperty(node.instance, propName, descriptor)` to get the RbxDom-encoded current value
  - Store as `{current = encodedCurrent, incoming = encodedIncoming}`
  - If encoding fails for either side, skip that property (don't crash the response)

## New Statuses


| Status                                                   | When                                     | isError   |
| -------------------------------------------------------- | ---------------------------------------- | --------- |
| `dryrun`                                                 | Dryrun mode, returning what would happen | false     |
| `fastfail_unresolved`                                    | Fastfail mode, unresolved changes exist  | true      |
| (existing) `success`, `empty`, `rejected`, `error`, etc. | Unchanged                                | Unchanged |


## Response Format Enhancement

Currently the MCP tool response is plain text. After this change, for all statuses that include changes (success, fastfail, dryrun, rejected), include a structured JSON block inside the text content so the agent can parse it:

```
Sync completed successfully.

Changes:
- [push] ServerScriptService/MyScript (Edit, Script, fs=src/server/MyScript.server.luau)
- [pull] Workspace/MyPart (Edit, Part, fs=src/workspace/MyPart.model.json5)

<json>
{"status":"success","changes":[{"path":"ServerScriptService/MyScript","id":"00ab...","direction":"push","patchType":"Edit","className":"Script","studioHash":"abc123","defaultSelection":"push","fsPath":"src/server/MyScript.server.luau","properties":{"Disabled":{"current":false,"incoming":true}}},{"path":"Workspace/MyPart","id":"00cd...","direction":"pull","patchType":"Edit","className":"Part","defaultSelection":"pull","fsPath":"src/workspace/MyPart.model.json5","properties":{"Position":{"current":{"Vector3":[0,5,0]},"incoming":{"Vector3":[10,5,0]}}}}]}
</json>
```

The `<json>` block gives the agent machine-readable data while keeping the human-readable summary above it. Note: script Source is deliberately excluded from `properties` to avoid massive payloads -- agents use `studioHash` for verification and the future `get_script` tool to read actual source content when needed.

## Test Suite

Full coverage testing across both Rust and Lua, organized by layer.

### Rust: `src/web/mcp.rs` unit tests

**Wire type serde (extend existing `wire_types` module):**

- `SyncOverride` serializes/deserializes with camelCase (`id`, `direction`, `studioHash`, `expectedProperties`)
- `SyncOverride` defaults: missing `studioHash` and `expectedProperties` deserialize to `None`
- `McpSyncCommand` serializes `mode` and `overrides` fields
- `SyncChange` serializes all new optional fields (`id`, `className`, `patchType`, `studioHash`, `defaultSelection`, `fsPath`, `properties`)
- `SyncChange` omits `None` optional fields (skip_serializing_if)
- `McpSyncResult` with enriched `SyncChange` entries round-trips correctly

**Handler tests (extend existing `handler_tests` module):**

- `handle_tools_list` returns updated inputSchema with `mode` and `overrides`
- `handle_atlas_sync` parses `mode` from params and passes through to command
- `handle_atlas_sync` parses `overrides` from params and passes through to command
- `handle_atlas_sync` defaults `mode` to `"standard"` when omitted
- `handle_atlas_sync` defaults `overrides` to empty array when omitted

**Guard tests (extend existing `atlas_sync_guards` module):**

- Success result with enriched changes includes `<json>` block in response text
- Dryrun status maps to `isError: false`
- `fastfail_unresolved` status maps to `isError: true`
- Empty status still returns `isError: false`
- Rejected with enriched changes includes `<json>` block

### Rust: `src/web/api.rs` fsPath enrichment tests

- Valid Ref resolves to `InstigatingSource::Path` -> fsPath is set (relative to project root)
- Valid Ref with `InstigatingSource::ProjectNode` -> fsPath is None
- Invalid Ref string (plugin GUID) -> fsPath is None, no panic
- Ref for instance not in tree -> fsPath is None
- Multiple changes: only those with valid paths get fsPath

### Plugin: `_buildMcpChangeList` tests (`plugin/src/App/init.spec.lua` or dedicated file)

**Basic enrichment:**

- Edit node includes `id`, `className`, `patchType`, `defaultSelection`
- Add node includes `id`, `className`, `patchType = "Add"`
- Remove node includes `id`, `className`, `patchType = "Remove"`
- Script node includes `studioHash` (correct git blob SHA1)
- Non-script node has no `studioHash`

**Property encoding:**

- Edit node with `changedProperties` produces `properties` map with `{current, incoming}` in RbxDom-encoded format
- `Source` is skipped for script classes
- `Name` change appears in properties as `changedName`
- Failed encode for current value skips that property (no crash)
- Failed encode for incoming value skips that property (no crash)
- Multiple property types: Bool, Number, String, Vector3, Color3, Enum

**includeAll parameter:**

- `includeAll = false` with selections: only includes selected nodes
- `includeAll = false` without selections: includes all with defaultSelection or "push"
- `includeAll = true`: includes every node with patchType regardless of selection

### Plugin: Override verification tests

**id-based matching:**

- Override with valid node id matches correctly
- Override with unknown id is silently skipped
- Multiple overrides match their respective nodes

**studioHash verification:**

- Matching hash: override accepted, node selection set
- Mismatching hash: override rejected, node stays unresolved
- No studioHash on override for script: override accepted (no verification)
- studioHash on non-script: ignored (only Source hashing applies to scripts)

**expectedProperties verification:**

- All properties match: override accepted
- One property mismatch: override rejected entirely
- Empty expectedProperties: override accepted (no verification)
- Property value decoded via `decodeValue` before `trueEquals` comparison
- Fuzzy float comparison works (e.g. `0.30000001` matches `0.3`)
- EnumItem comparison works (e.g. `{Enum = 2}` vs `Enum.Material.Plastic`)

### Plugin: Mode behavior tests

**standard mode:**

- All pre-selected: fast-forwards, resolves `success`, returns Confirm with selections
- Some unresolved: shows Confirming UI, waits for user
- Empty patch: resolves `empty`

**manual mode:**

- All pre-selected: still shows Confirming UI (no fast-forward)
- User confirms: resolves `success`
- User aborts: resolves `rejected`

**fastfail mode:**

- All resolved (via defaults + overrides): fast-forwards, resolves `success`
- Some unresolved after defaults + overrides: resolves `fastfail_unresolved` immediately, returns `"Abort"`, includes full change list with `includeAll = true`
- Override verification failure makes node unresolved, triggers fastfail

**dryrun mode:**

- Resolves immediately with `status: "dryrun"`, returns `"Abort"`
- Change list includes all nodes with full metadata (properties, studioHash)
- Session is aborted without applying any changes
- Works with empty patch (resolves `empty`)

### Plugin: McpStream forwarding tests

- `{type: "sync", requestId, mode, overrides}` forwards all three fields to `onSyncCommand`
- Missing `mode` defaults to `"standard"` (or nil, handled by `startMcpSync`)
- Missing `overrides` defaults to empty table
- Result is JSON-encoded and sent back over WebSocket

