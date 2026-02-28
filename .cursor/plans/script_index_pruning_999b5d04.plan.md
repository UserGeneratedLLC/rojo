---
name: Script index pruning
overview: "Scripts-only mode: add script index to RojoTree for fast server-side pruning, send syncScriptsOnly to plugin so it skips non-script two-way sync work, and add server-side defense filtering on write. Comprehensive test fixtures for all paths."
todos:
  - id: script-index
    content: "Add `script_refs: HashSet<Ref>` to `RojoTree` and maintain it in `insert_instance()`, `remove()`, `new()`"
    status: completed
  - id: class-change
    content: Handle ClassName changes in `apply_update_child()` with `update_script_tracking()`
    status: completed
  - id: optimize-read
    content: Rewrite `handle_api_read` scripts-only path to use script index + ancestor walk
    status: completed
  - id: server-info
    content: Add `syncScriptsOnly` to ServerInfoResponse and populate from project config
    status: completed
  - id: ws-inject-ancestors
    content: Fix filter_subscribe_message_for_scripts to inject missing ancestor instances when a new script appears in a previously-pruned subtree
    status: completed
  - id: plugin-filter
    content: "Plugin: skip non-script instances in ChangeBatcher when syncScriptsOnly is true"
    status: completed
  - id: plugin-descendant-added
    content: "Plugin: connect DescendantAdded on synced services to detect new scripts created in Studio outside the mapped tree"
    status: completed
  - id: server-write-filter
    content: "Server: filter non-script updates in handle_api_write when scripts-only mode"
    status: completed
  - id: tests
    content: "Add test fixtures: read-API pruning, WebSocket ancestor injection, write-API filtering, new-script-in-pruned-subtree"
    status: completed
  - id: cleanup
    content: Remove now-unused `collect_scripts_and_ancestors` helper, share `is_script_class`
    status: completed
isProject: false
---

# Scripts-Only Mode: Full Pruning (Server + Plugin)

## Problem

When `syncScriptsOnly` is enabled:

1. **Server read path**: `handle_api_read` walks the entire descendant tree (O(n)) just to find scripts -- wasteful for large places with 100K+ non-script instances.
2. **Plugin two-way sync**: Plugin has no knowledge of scripts-only mode. It batches and sends property changes for non-script ancestor containers back to the server, which processes them unnecessarily.
3. **Server write path**: `handle_api_write` processes all incoming updates regardless of scripts-only mode -- no defense-in-depth filtering.

## Solution

Three-pronged approach:

- **Server**: Script index on `RojoTree` for O(scripts * depth) reads, filter writes
- **Plugin**: Receive `syncScriptsOnly` flag, skip non-script instances in `ChangeBatcher`
- **Tests**: Fixtures verifying pruning in read, WebSocket, write, and plugin paths

---

## Part 1: Server-Side Script Index

### 1a. Add script index to `RojoTree` ([src/snapshot/tree.rs](src/snapshot/tree.rs))

- Add `script_refs: HashSet<Ref>` field to `RojoTree` (alongside `path_to_ids`, `specified_id_to_refs`)
- Initialize in `RojoTree::new()`
- `**insert_instance()`**: after inserting, if `is_script_class(class_name)` then `self.script_refs.insert(referent)` (recursive children handled by the recursive call)
- `**remove()`**: `self.script_refs.remove(&id)` in the existing descendant cleanup loop
- Add `pub fn script_refs(&self) -> &HashSet<Ref>` accessor
- Add `pub(crate) fn update_script_tracking(&mut self, id: Ref, old_class: &str, new_class: &str)` for ClassName changes
- Add `pub(crate) fn is_script_class(class_name: &str) -> bool` free function (shared by `tree.rs` and `api.rs`)

### 1b. Handle ClassName changes ([src/snapshot/patch_apply.rs](src/snapshot/patch_apply.rs))

In `apply_update_child()` (line 270), when `changed_class_name` is present:

- Capture old class name before the mutable borrow begins
- After mutable borrow is dropped, call `tree.update_script_tracking(id, &old_class, new_class.as_str())`

### 1c. Optimize `handle_api_read` ([src/web/api.rs](src/web/api.rs) lines 3103-3138)

Replace the two-pass full-tree walk with script-index-based ancestor walk:

```rust
let all_scripts = tree.script_refs();
let mut included_ids: HashSet<Ref> = HashSet::new();
let requested_set: HashSet<Ref> = requested_ids.iter().copied().collect();

for &script_id in all_scripts {
    let mut chain = Vec::new();
    let mut current = script_id;
    let mut is_descendant = false;

    while let Some(inst) = tree.get_instance(current) {
        if included_ids.contains(&current) {
            is_descendant = true;
            break;
        }
        if requested_set.contains(&current) {
            is_descendant = true;
            chain.push(current);
            break;
        }
        chain.push(current);
        let parent = inst.parent();
        if parent.is_none() { break; }
        current = parent;
    }

    if is_descendant {
        included_ids.extend(chain);
    }
}

for &id in &included_ids {
    if let Some(instance) = tree.get_instance(id) {
        instances.insert(id, instance_for_scripts_only(instance, &included_ids));
    }
}
```

### 1d. Fix WebSocket ancestor injection ([src/web/api.rs](src/web/api.rs) `filter_subscribe_message_for_scripts`)

**Bug**: When a new script appears on disk inside a previously-pruned subtree, the WebSocket patch contains only the new script in `msg.added`. The function walks up the parent chain and adds ancestors to `included_ids`, but line 3574 only **retains** from `msg.added`. Ancestors that already exist in the tree (but are NOT in the plugin's InstanceMap because they were pruned) are never sent. The plugin receives an orphaned script with an unknown parent.

**Fix**: After building `included_ids`, inject missing ancestors from the tree into `msg.added`:

```rust
// After: msg.added.retain(|id, _| included_ids.contains(id));

// Inject ancestors that the plugin may not have (they exist in the tree
// but weren't part of this patch). Without this, a new script in a
// previously-pruned subtree arrives with an unknown parent.
for &id in &included_ids {
    if !msg.added.contains_key(&id) {
        if let Some(tree_inst) = tree.get_instance(id) {
            let inst = instance_for_scripts_only(tree_inst, &included_ids);
            msg.added.insert(id, inst);
        }
    }
}
```

The Reconciler handles redundant ancestors gracefully (matches them to existing Studio instances via the matching algorithm), so injecting an ancestor the plugin already has is harmless.

### 1e. Filter writes in `handle_api_write` ([src/web/api.rs](src/web/api.rs) line 447)

When `sync_scripts_only()` is true, filter incoming `WriteRequest`:

- `**updated**`: Retain only updates where `tree.get_instance(id)` is a script class
- `**added**`: Allow all (scripts + children of scripts are valid additions)
- `**removed**`: Allow all (plugin should only send script removals, but let server handle gracefully)

Defense-in-depth -- the plugin should already filter, but the server shouldn't blindly trust client data.

---

## Part 2: Server-to-Plugin Communication

### 2a. Add `syncScriptsOnly` to ServerInfoResponse ([src/web/interface.rs](src/web/interface.rs) line 260)

Add new field after `sync_source_only`:

```rust
#[serde(default)]
pub sync_scripts_only: bool,
```

### 2b. Populate from project config ([src/web/api.rs](src/web/api.rs) line 395)

In `handle_api_rojo()`, add:

```rust
sync_scripts_only: self.serve_session.sync_scripts_only(),
```

The accessor already exists at `serve_session.rs` line 501.

---

## Part 3: Plugin-Side Filtering

### 3a. Store flag in ServeSession ([plugin/src/ServeSession.lua](plugin/src/ServeSession.lua))

In `__computeInitialPatch` where `serverInfo` is received (line 619):

```lua
self.__syncScriptsOnly = serverInfo.syncScriptsOnly or false
self.__changeBatcher:setSyncScriptsOnly(self.__syncScriptsOnly)
```

### 3b. Filter in ChangeBatcher ([plugin/src/ChangeBatcher/init.lua](plugin/src/ChangeBatcher/init.lua))

Add `__syncScriptsOnly` field. In `add()` (line 120), early return for non-script instances:

```lua
function ChangeBatcher:add(instance, propertyName)
    if self.__syncScriptsOnly and not instance:IsA("LuaSourceContainer") then
        return
    end
    -- ... existing code ...
end
```

Using `IsA("LuaSourceContainer")` is the idiomatic Roblox way to check Script/LocalScript/ModuleScript (it catches all three and any future subclasses).

Add `setSyncScriptsOnly()` method:

```lua
function ChangeBatcher:setSyncScriptsOnly(value)
    self.__syncScriptsOnly = value
end
```

### 3c. Detect new scripts created in Studio ([plugin/src/ServeSession.lua](plugin/src/ServeSession.lua))

**Problem**: The plugin only connects `Changed` signals on instances in its InstanceMap. If a user creates a new Script inside a container that was pruned (no scripts), the plugin has zero signals on that container and the new script goes undetected. The plugin doesn't use `DescendantAdded` anywhere today.

**Fix**: When `syncScriptsOnly` is true, connect `DescendantAdded` on each synced service root. When a new `LuaSourceContainer` appears outside the mapped tree, encode it and its unmapped ancestor chain as additions and send via the write API.

**Key insight**: `syncback_added_instance` ([src/web/api.rs](src/web/api.rs) line 1097) already has **upsert logic** -- it checks `find_child_by_name` and updates in place if the instance already exists in the tree (returns `Ok(true)` to skip PatchAdd). This means the plugin can send intermediate containers as "additions" and the server won't create duplicates.

**Plugin implementation** (in `ServeSession.lua`, after initial sync):

```lua
if self.__syncScriptsOnly then
    for id, instance in self.__instanceMap.fromIds do
        if instance.Parent == game then
            local conn = instance.DescendantAdded:Connect(function(descendant)
                if not self.__twoWaySync then return end
                if not descendant:IsA("LuaSourceContainer") then return end
                if self.__instanceMap.fromInstances[descendant] then return end

                local chain = {} -- unmapped ancestors, ordered parent-first
                local current = descendant.Parent
                while current and current ~= game do
                    if self.__instanceMap.fromInstances[current] then
                        break
                    end
                    table.insert(chain, 1, current)
                    current = current.Parent
                end

                local mappedParent = current
                local mappedParentId = self.__instanceMap.fromInstances[mappedParent]
                if not mappedParentId then return end

                local patch = PatchSet.newEmpty()
                local prevParentId = mappedParentId

                -- Intermediate containers: skeleton only (className, name, parent).
                -- No properties -- the server already has these instances on disk.
                -- Sending properties would trigger syncback_update_existing_instance
                -- which writes to disk, risking round-trip identity violations.
                for _, intermediate in chain do
                    local tempId = HttpService:GenerateGUID()
                    patch.added[tempId] = {
                        parent = prevParentId,
                        name = intermediate.Name,
                        className = intermediate.ClassName,
                        properties = {},
                        children = {},
                    }
                    self.__instanceMap:insert(tempId, intermediate)
                    prevParentId = tempId
                end

                -- The script itself gets full encoding (Source, etc.)
                local scriptTempId = HttpService:GenerateGUID()
                patch.added[scriptTempId] = encodeInstance(descendant, prevParentId)
                self.__instanceMap:insert(scriptTempId, descendant)

                self.__apiContext:write(patch)
            end)
            table.insert(self.__connections, conn)
        end
    end
end
```

**Round-trip identity**: Intermediate containers are sent as skeletons (className + name only, empty properties). The server's upsert logic (`syncback_added_instance` line 1097) matches them by name to existing tree instances and returns `Ok(true)` to skip file creation. For non-script types with empty properties, the upsert at line 1370 (`if !added.properties.is_empty()`) skips the meta-file write entirely. This ensures the filesystem is NOT modified for containers that already exist. Only the new script file gets written.

**Server-side handling**: No new server code needed. The existing upsert handles intermediate containers without creating duplicates or modifying existing files. The new script gets created as a new file. The VFS watcher picks up the new file, the tree updates, and subsequent WebSocket patches include the script normally.

**After the write completes**: The server broadcasts a patch that includes the new script. The plugin receives it and the InstanceMap gets updated with the real server-assigned Ref (the temp ID was for the write request only -- the server's applied patch uses its own Refs). The plugin's InstanceMap may briefly have stale temp IDs, but the next patch from the server will replace them via hydration.

**Note**: The server side already handles the reverse direction: when a script file is created on disk in a pruned subtree, the VFS watcher picks it up, adds it to the tree and `script_refs`, and the WebSocket path (with the 1d ancestor injection fix) correctly sends it to the plugin.

---

## Part 4: Test Fixtures

### 4a. New test fixture: `rojo-test/serve-tests/scripts_only_read_pruning/`

Project with scripts at various depths mixed with non-script instances. Tests that `GET /api/read` only returns scripts and their ancestors -- verifies the script index produces correct results.

```
default.project.json5  (syncScriptsOnly: true)
src/
  TopScript.server.luau
  DeepNest/
    Middle/
      DeepScript.luau
  PureParts/
    PartA.model.json5
    PartB.model.json5
  MixedFolder/
    SomeScript.luau
    SomeModel.model.json5
```

**Test assertions:**

- Read response includes `TopScript`, `DeepScript`, `SomeScript` and their ancestor containers
- Read response does NOT include `PartA`, `PartB`, `SomeModel`, or the `PureParts` folder
- All included non-script ancestors have `ignoreUnknownInstances: true` and empty properties
- Instance count matches expected (scripts + ancestors only)

### 4b. New tests in `tests/tests/serve.rs` (or `syncback_format_transitions.rs`)

- `**scripts_only_read_prunes_non_script_subtrees`**: Verify read response excludes entire `PureParts/` subtree
- `**scripts_only_read_includes_deep_ancestors`**: Verify deeply nested scripts pull in full ancestor chain
- `**scripts_only_websocket_injects_ancestors_for_new_script`**: Create a new script file on disk inside a previously-pruned subtree (`PureParts/NewScript.luau`), verify the WebSocket message includes both the new script AND the `PureParts` ancestor container
- `**scripts_only_websocket_prunes_non_script_patches`**: Add a non-script file on disk, verify WebSocket message filters it out
- `**scripts_only_write_rejects_non_script_updates`**: POST an update for a non-script instance, verify it's filtered/ignored
- `**scripts_only_serverinfo_flag**`: Verify `GET /api/rojo` includes `syncScriptsOnly: true`

### 4c. Existing fixture enhancement

The existing `rojo-test/serve-tests/syncback_scripts_only/` fixture already has `NonScriptModel.model.json5`. Existing tests focus on syncback write behavior. Add a new test using this fixture that verifies `NonScriptModel` is absent from the read response.

---

## Mode Transitions (Stop Server -> Toggle syncScriptsOnly -> Restart)

This works with zero plugin-side mode-switching logic. The transitions are inherently safe because:

1. `App:endSession()` calls `serveSession:stop()` -> `instanceMap:stop()` which **removes ALL mappings**
2. `App:startSession()` creates a **brand new** `ServeSession` with a fresh `InstanceMap` and `ChangeBatcher`
3. The fresh session calls `start()` -> `GET /api/rojo` (receives `syncScriptsOnly` flag) -> `GET /api/read` (receives pruned or full tree depending on mode) -> `hydrate()` + `diff()`

**Normal -> Scripts-only**: Server returns pruned tree. Fresh InstanceMap gets only scripts + ancestors. Studio's non-script instances are untouched (all nodes have `ignoreUnknownInstances: true`). ChangeBatcher receives `syncScriptsOnly = true` and filters non-script changes.

**Scripts-only -> Normal**: Server returns full tree. Fresh InstanceMap gets everything. `hydrate()` matches server instances to existing Studio instances by Name+ClassName (Studio already has them -- they were never deleted). ChangeBatcher receives `syncScriptsOnly = false` and sends all changes.

No special handling needed. The plugin doesn't need to know it was previously in a different mode.

## What stays the same

- `**instance_for_scripts_only`**: Still used to build filtered Instance objects. No logic change.
- `**InstanceMap.__connectSignals()`**: Signals still connected for all mapped instances. The ChangeBatcher filtering (3b) is upstream and simpler than skipping signal connections, which could miss edge cases with Parent change detection.

## atlas.mdc Conformance

**Round-trip identity**: Preserved. Scripts-only pruning only affects what is SENT over the wire, not what is stored on disk. The filesystem always contains the full tree. Building an rbxl from the filesystem produces the same result regardless of scripts-only mode. The skeleton encoding for intermediate containers (step 3c) ensures no filesystem writes for existing non-script instances.

**Code quality / DRY**: `is_script_class()` is extracted as a shared `pub(crate)` free function in `tree.rs`, replacing the private duplicate in `api.rs`. The `collect_scripts_and_ancestors` helper is removed (replaced by the script index). No new duplication introduced.

**Two-way sync parity with CLI syncback**: In scripts-only mode, two-way sync only handles script instances. This is a different operating mode, not a divergence from syncback behavior. When `syncScriptsOnly` is false, all paths are identical to before.

**Error handling**: All new Rust code uses `anyhow::Context` and `?` propagation per atlas.mdc conventions. No `unwrap()` or `expect()` in library code. Plugin code guards against nil InstanceMap lookups and destroyed instances.

**Protocol compatibility**: The new `syncScriptsOnly` field on `ServerInfoResponse` uses `#[serde(default)]`, making it backward-compatible. Old plugins ignore the field (defaults to `false`). No protocol version bump needed.

**No unnecessary changes**: The plan does not modify globIgnorePaths handling (already correct at the snapshot layer), does not rename variables, and does not refactor unrelated code.

## Complexity comparison

- **Initial read** (100K instances, 200 scripts, depth 20): ~200K iterations (before) vs ~4,000 (after)
- **Two-way sync** (plugin): Non-script changes silently dropped in `ChangeBatcher.add()` instead of round-tripping to server
- **Server writes**: Non-script updates filtered before filesystem I/O
- **Tree mutation overhead**: O(1) per insert/remove (HashSet add/remove)
- **Memory**: O(scripts) HashSet entries (negligible)

