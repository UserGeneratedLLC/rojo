---
name: Rojo Syncback Architecture Fix
overview: Document Rojo's two-way sync architecture and fix the plugin API syncback to properly consult the tree/VFS instead of relying solely on plugin data, preventing duplicate file creation and format mismatches.
todos:
  - id: tree-lookup
    content: Add tree-based instance lookup in syncback_added_instance() to find existing instances by name+parent before deciding file format
    status: completed
  - id: use-instigating-source
    content: When existing instance found in tree, write to its instigating_source.path instead of creating new file
    status: completed
  - id: verify-plugin-children-tracking
    content: Verify plugin encodeInstance correctly includes/excludes children and test children deletion scenarios
    status: completed
  - id: test-plugin-cli-parity
    content: Create test verifying plugin syncback produces same result as CLI syncback for identical Studio state
    status: completed
isProject: false
---

# Rojo Syncback Architecture Fix

## Goal

Enable perfect two-way synchronization between Roblox Studio and the filesystem via the Rojo plugin, without requiring constant `rojo serve` restarts and manual `rojo syncback` operations. The plugin should accurately keep the project up to date as if running `rojo syncback` from scratch.

This builds on the [Pull Instances to Rojo Feature](../.cursor/plans/pull_removed_scripts_feature_dd790311.plan.md) which added support for pulling ALL instance types from Studio to Rojo.

---

## Background: How Two-Way Sync Works

### Original Design (commit a398338)

The original two-way sync only handled **property updates**:

```
Plugin ──[WriteRequest.updated]──> API Server
                                      │
                                      ▼
                              tree_mutation_sender
                                      │
                                      ▼
                              ChangeProcessor.handle_tree_event()
                                      │
                                      ▼ (for each update)
                              fs::write(instigating_source.path, new_value)
                                      │
                                      ▼
                              VFS detects file change
                                      │
                                      ▼
                              Tree updated from VFS snapshot
```

Key insight: **The tree knows the correct file path via `instigating_source**`. Updates write to that path, preserving the existing file structure.

The original `/api/write` only handled `updated` instances (property changes). It did NOT handle `added` or `removed` instances - those were delegated to CLI syncback.

### Extended Design (Pull Instances Feature)

We extended `/api/write` to handle three types of operations:


| Operation              | Plugin sends             | Server action                                          |
| ---------------------- | ------------------------ | ------------------------------------------------------ |
| **Update**             | `WriteRequest.updated[]` | Write to `instigating_source.path` via ChangeProcessor |
| **Pull (create file)** | `WriteRequest.added{}`   | Create new files via `syncback_added_instance()`       |
| **Pull (delete file)** | `WriteRequest.removed[]` | Delete files via `syncback_removed_instance()`         |


---

## How the Plugin Encodes Instances

### encodeInstance.lua

When the plugin sends an instance via "Pull", it uses `encodeInstance()` which:

1. **Recursively includes ALL children** (lines 247-268)
2. **Filters duplicate-named siblings** (can't reliably sync)
3. **Encodes all properties** (Attributes, Tags, etc.)

```lua
-- From plugin/src/ChangeBatcher/encodeInstance.lua
return {
    parent = parentId,
    name = instance.Name,
    className = instance.ClassName,
    properties = properties,
    children = children, -- ALL children recursively encoded
}
```

### When Plugin Sends Data


| Scenario                   | Plugin action    | Data sent                                   |
| -------------------------- | ---------------- | ------------------------------------------- |
| Property edit (Source)     | `createPatchSet` | `updated[]` with changed properties only    |
| User "Pulls" removed item  | `encodeInstance` | `added{}` with FULL instance + all children |
| Instance deleted in Studio | `createPatchSet` | `removed[]` with instance ID                |


---

## The Core Problems

### Problem 1: Server ignores existing tree state for "added" instances

When plugin sends an "added" instance, the server decides file format based on `has_children` from the plugin data, NOT the tree state:

```
Existing tree:     EventService -> instigating_source: "EventService/init.luau"
                   
Plugin sends:      AddedInstance { name: "EventService", children: [...] }
                   (user pulled it because it appeared as "to delete")

Current behavior:  Creates NEW files at EventService/init.luau
                   (even though files already exist!)
```

### Problem 2: Why items appear as "to delete" when files exist

If duplicate files exist (e.g., `BombardilloEvent.luau` AND `BombardilloEvent/`):

1. Rojo can't determine which representation is correct
2. Skips loading these instances ("duplicate-named siblings")
3. Instance exists in Studio but NOT in Rojo tree
4. Appears as "to be deleted" in sync diff
5. User pulls it, creating MORE files
6. Cycle continues

### Problem 3: Format transitions (children added/removed)


| Situation                     | Expected file format | Current plugin behavior                    |
| ----------------------------- | -------------------- | ------------------------------------------ |
| ModuleScript with children    | `Name/init.luau`     | encodeInstance includes children correctly |
| ModuleScript children deleted | `Name.luau`          | encodeInstance sends with `children: []`   |
| User edits Source only        | Keep existing format | Sent as UPDATE, not ADD - works correctly  |


The UPDATE path (via ChangeProcessor) is correct. The ADD path needs to check tree state first.

---

## The Duplicate Creation Chain

```
1. Initial state: EventService/init.luau exists, works fine

2. Bug triggers: Something creates EventService.luau (duplicate)
   - Could be: plugin syncback bug, manual creation, etc.

3. Rojo startup: Sees duplicate, can't load either
   - Warning: "Skipped 2 location(s) with duplicate-named siblings"
   
4. Sync diff: EventService appears as "to be deleted"
   - (Exists in Studio, not in Rojo tree due to duplicate skip)

5. User pulls: Plugin sends encodeInstance(EventService)
   - Server creates MORE files at EventService/...

6. Result: Even more duplicates, cycle continues
```

---

## Architectural Principle: Tree is Source of Truth

For plugin syncback, the **tree/VFS** should be the source of truth for existing file structure, not:

- The raw filesystem (our current fix checks this, but it's indirect)
- The plugin data (incomplete/partial)

### Correct Flow for "Added" Instances

```
Plugin sends AddedInstance for "EventService"
                │
                ▼
    Does instance already exist in tree at this path?
                │
        ┌───────┴───────┐
        ▼               ▼
       YES              NO
        │               │
        ▼               ▼
   Use tree's        Use has_children
   instigating_      to decide format
   source.path       (truly new instance)
        │               │
        ▼               ▼
   UPDATE existing   CREATE new files
   file in place     with format based
                     on children
```

---

## Files and Their Roles

### Server-Side (Rust)


| File                                               | Role                            | Status                        |
| -------------------------------------------------- | ------------------------------- | ----------------------------- |
| [src/web/api.rs](src/web/api.rs)                   | Plugin API `/api/write` handler | NEEDS FIX: tree lookup        |
| [src/change_processor.rs](src/change_processor.rs) | Two-way sync UPDATE handler     | OK: uses `instigating_source` |
| [src/syncback/mod.rs](src/syncback/mod.rs)         | CLI syncback clean mode         | FIXED: filesystem scan        |


### Plugin-Side (Lua)


| File                                                                                       | Role                         | Status                    |
| ------------------------------------------------------------------------------------------ | ---------------------------- | ------------------------- |
| [plugin/src/ServeSession.lua](plugin/src/ServeSession.lua)                                 | Orchestrates sync operations | OK: correct flow          |
| [plugin/src/ChangeBatcher/encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua) | Encodes instances for pull   | OK: includes children     |
| [plugin/src/ChangeBatcher/createPatchSet.lua](plugin/src/ChangeBatcher/createPatchSet.lua) | Creates patches for updates  | OK: property changes only |


---

## Detailed Implementation

### 1. [src/web/api.rs](src/web/api.rs) - Tree Lookup for Added Instances

**Current approach**: `detect_existing_script_format()` checks raw filesystem

**Better approach**: Check the tree FIRST

```rust
fn syncback_added_instance(
    &self,
    added: &AddedInstance,
    tree: &RojoTree,
    // ...
) -> anyhow::Result<()> {
    let parent_ref = added.parent.context("must have parent")?;
    
    // NEW: Check if instance already exists in tree
    if let Some(existing_ref) = find_child_by_name(tree, parent_ref, &added.name) {
        if let Some(existing) = tree.get_instance(existing_ref) {
            if let Some(source) = &existing.metadata().instigating_source {
                // Instance exists - update in place instead of creating new
                return self.syncback_update_existing(added, source.path());
            }
        }
    }
    
    // Instance doesn't exist - create new (existing code)
    self.syncback_instance_to_path_with_stats(added, &parent_dir, stats)
}

fn find_child_by_name(tree: &RojoTree, parent: Ref, name: &str) -> Option<Ref> {
    let parent_inst = tree.get_instance(parent)?;
    parent_inst.children().iter()
        .find(|&&child_ref| {
            tree.get_instance(child_ref)
                .map(|c| c.name() == name)
                .unwrap_or(false)
        })
        .copied()
}
```

### 2. [src/syncback/mod.rs](src/syncback/mod.rs) - Already Fixed

The clean mode fix (scanning actual filesystem for `existing_paths`) is correct:

```rust
// In clean mode, scan the actual filesystem to find ALL existing files
let existing_paths: HashSet<PathBuf> = if !incremental {
    let mut paths = HashSet::new();
    // Recursively scan source directories
    scan_directory(dir, &mut paths, &ignore_patterns, project_path);
    paths
} else {
    HashSet::new()
};
```

### 3. [src/change_processor.rs](src/change_processor.rs) - No Changes Needed

The UPDATE path correctly uses `instigating_source`:

```rust
for (key, changed_value) in &update.changed_properties {
    if key == "Source" {
        if let Some(InstigatingSource::Path(path)) = &instance.metadata().instigating_source {
            fs::write(path, value)?; // Writes to correct existing file
        }
    }
}
```

---

## Implementation Tasks

### Task 1: Tree-Based Instance Lookup (Priority: HIGH)

**File**: [src/web/api.rs](src/web/api.rs)

In `syncback_added_instance()`, add tree lookup BEFORE format detection:

```rust
// Add helper function
fn find_child_by_name(tree: &RojoTree, parent_ref: Ref, name: &str) -> Option<Ref> {
    let parent = tree.get_instance(parent_ref)?;
    parent.children().iter()
        .find(|&&child_ref| {
            tree.get_instance(child_ref)
                .map(|c| c.name() == name)
                .unwrap_or(false)
        })
        .copied()
}

// In syncback_added_instance(), before filesystem operations:
if let Some(existing_ref) = find_child_by_name(tree, parent_ref, &added.name) {
    if let Some(existing) = tree.get_instance(existing_ref) {
        if let Some(source) = &existing.metadata().instigating_source {
            // Found existing instance - write to its path, don't create new
            log::info!("Syncback: Updating existing instance at {}", source.path().display());
            return self.syncback_update_existing_instance(added, existing, source.path());
        }
    }
}
// Not found - continue with current new-instance creation logic
```

### Task 2: Handle Format Transitions (Priority: MEDIUM)

When instance has children in tree but plugin sends with empty children:

**Option A (Conservative - Recommended)**: Preserve existing format

- If tree has `Name/init.luau`, always update `Name/init.luau`
- User must use CLI syncback for format transitions

**Option B (Future)**: Add explicit flag

```rust
pub struct AddedInstance {
    // ... existing fields ...
    #[serde(default)]
    pub force_format_change: bool,  // If true, allow format transition
}
```

### Task 3: Verify Plugin Children Tracking (Priority: HIGH)

**File**: [plugin/src/ChangeBatcher/encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua)

Create test scenarios:

1. **ModuleScript with children** - verify `children` array populated
2. **ModuleScript with children deleted** - verify `children: []`
3. **Nested children** - verify recursive encoding
4. **Duplicate siblings** - verify correctly skipped with warning

Add test file: `plugin/src/ChangeBatcher/encodeInstance.spec.lua`

### Task 4: Integration Test (Priority: MEDIUM)

Create end-to-end test:

```
1. Setup:
   - Create EventService/init.luau with children
   - Start rojo serve
   
2. Simulate plugin syncback:
   - Send AddedInstance for EventService with children
   
3. Verify:
   - No duplicate files created
   - EventService/init.luau updated (not EventService.luau created)
   
4. Compare:
   - Plugin syncback result == CLI syncback result
```

---

## Summary of Changes

### Already Fixed


| Issue                         | Fix                                    | File                                                     |
| ----------------------------- | -------------------------------------- | -------------------------------------------------------- |
| reify.lua crash               | Gracefully handle missing children IDs | `plugin/src/Reconciler/reify.lua`                        |
| CLI clean mode misses orphans | Scan actual filesystem                 | `src/syncback/mod.rs`                                    |
| Plugin creates wrong format   | Check filesystem for existing format   | `src/web/api.rs`                                         |
| Hidden services marked delete | Expose ignoreHiddenServices to plugin  | `src/web/interface.rs`, `plugin/src/Reconciler/diff.lua` |


### Still Needed


| Issue                    | Fix                                    | File             |
| ------------------------ | -------------------------------------- | ---------------- |
| Ignores tree state       | Add tree lookup before format decision | `src/web/api.rs` |
| Duplicate creation cycle | Update existing instead of create new  | `src/web/api.rs` |
| Test coverage            | Add integration tests                  | `tests/`         |


---

## Design Principles

1. **Tree is source of truth** for existing file structure
2. **instigating_source.path** is THE path for an instance's file
3. **New instances** use `has_children` to decide format
4. **Existing instances** always update in place, never create duplicates
5. **Format transitions** only via CLI syncback (safe, explicit)
6. **Clean mode** scans filesystem to remove orphans tree doesn't know about

---

## File Format Reference

This applies to BOTH plugin syncback AND CLI syncback:


| Instance Type     | No Children        | With Children           |
| ----------------- | ------------------ | ----------------------- |
| ModuleScript      | `Name.luau`        | `Name/init.luau`        |
| Script (Server)   | `Name.server.luau` | `Name/init.server.luau` |
| Script (Client)   | `Name.client.luau` | `Name/init.client.luau` |
| LocalScript       | `Name.client.luau` | `Name/init.client.luau` |
| Folder            | directory          | directory               |
| StringValue       | `Name.txt`         | `Name/init.meta.json5`  |
| LocalizationTable | `Name.csv`         | `Name/init.csv`         |
| Other             | `Name.model.json5` | `Name/init.meta.json5`  |


**Note**: `init.meta.json5` only created if instance has properties (Attributes, etc.)