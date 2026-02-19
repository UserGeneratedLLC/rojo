---
name: Ref path and dedup integration
overview: "Two deferred items from the ambiguous path handling feature: (1) migrate collect_referents() to build filesystem-name-compatible paths so ref round-trips work for slugified/dedup'd instances, and (2) integrate dedup_suffix.rs cleanup rules into the change_processor deletion handler.\""
todos:
  - id: ref-path-tentative-fs
    content: Create tentative_fs_path() function and replace inst_path() calls in collect_referents/collect_all_paths
    status: completed
  - id: dedup-cleanup-change-processor
    content: Integrate dedup_suffix cleanup rules into change_processor handle_tree_event removal handling
    status: completed
isProject: false
---

# Syncback Ref Path Migration and Dedup Cleanup Integration

## Problem 1: Ref paths break for slugified/dedup'd instance names

### Root cause

`collect_referents()` in [src/syncback/ref_properties.rs](src/syncback/ref_properties.rs) calls `inst_path()` which delegates to the legacy `ref_target_path()`. This produces escaped instance-name paths like `"Workspace/Hey\/Bro"`. But `get_instance_by_path()` in [src/snapshot/tree.rs](src/snapshot/tree.rs) now uses simple `path.split('/')` without escape handling. An instance named `"Hey/Bro"` gets path `"Workspace/Hey\/Bro"`, which splits into `["Workspace", "Hey\\/Bro"]` -- the escaped slash is NOT unescaped, so resolution fails.

The fallback in `get_instance_by_path()` tries case-insensitive instance name matching, but the segment `"Hey\\/Bro"` does not match instance name `"Hey/Bro"`.

### Fix: Build filesystem-compatible paths in collect_referents

Create a `tentative_fs_path()` function that builds paths compatible with the new `get_instance_by_path()`:

- Walk ancestors from target to root (same as `ref_target_path`)
- For each instance, compute a **tentative filesystem name**: `slugify(name)` + extension based on class/children
- Join with `/` (no escaping needed since slugified names can't contain `/`)

The extension mapping uses the same class-to-middleware logic as `get_best_middleware()`:

- `Script` -> check RunContext: `.server.luau` / `.client.luau` / `.plugin.luau` / `.legacy.luau` / `.local.luau`; if has children, directory (no ext)
- `LocalScript` -> `.local.luau` or directory
- `ModuleScript` -> `.luau` or directory
- `Folder`, `Configuration`, `Tool`, GUI classes -> directory (no ext)
- `StringValue` -> `.txt`
- `LocalizationTable` -> `.csv`
- Everything else -> `.model.json5` or directory if has children

**Files to change:**

- [src/syncback/ref_properties.rs](src/syncback/ref_properties.rs): Replace `inst_path()` calls with new `tentative_fs_path()`. Update `collect_all_paths()` similarly.
- [src/syncback/snapshot.rs](src/syncback/snapshot.rs): Update `inst_path()` to use the new path builder (or deprecate it for ref contexts).

### How resolution works after this fix

A ref path like `"Workspace/Hey_Bro.server.luau"`:

1. Primary lookup: `filesystem_name_for()` returns `"Hey_Bro.server.luau"` (from instigating_source) -- **match**
2. If no instigating_source: fallback matches instance name `"Hey/Bro"` against segment `"Hey_Bro.server.luau"` -- no match, but this is expected for file-backed instances

For directory instances like services: `"Workspace"` matches both filesystem name and instance name -- works as before.

---

## Problem 2: Dedup cleanup not triggered on deletion in change_processor

### Current state

[src/syncback/dedup_suffix.rs](src/syncback/dedup_suffix.rs) provides `compute_cleanup_action()` with gap-tolerant, base-name-promotion, and group-to-1 rules. But `handle_tree_event()` in [src/change_processor.rs](src/change_processor.rs) (line 1043) only logs removals -- it doesn't check if a deletion leaves a dedup group needing suffix cleanup.

### Fix: Add dedup cleanup after removals

In `handle_tree_event()`, after the removal loop (line ~1070), for each removed instance:

1. Find the parent instance (before the removal is applied to the tree)
2. Enumerate the parent's remaining children (excluding the removed one)
3. Group remaining children by dedup base name (parse `~N` suffixes from filesystem names via `parse_dedup_suffix()`)
4. For each dedup group affected by the removal, call `compute_cleanup_action()`
5. If the action is `RemoveSuffix` or `PromoteLowest`:
  a. Call `suppress_path()` for both old and new paths
   b. Rename the file on disk
   c. Update meta file `name` field if needed
   d. Update `RefPathIndex` via `update_ref_paths_after_rename()`

**Files to change:**

- [src/change_processor.rs](src/change_processor.rs): Add dedup cleanup logic after the removal loop in `handle_tree_event()`. Import `dedup_suffix::{parse_dedup_suffix, compute_cleanup_action}`.

### Edge cases

- The removed instance's parent must still exist in the tree when we enumerate siblings
- ProjectNode instances are excluded (already guarded)
- The cleanup rename must happen BEFORE `apply_patch_set()` removes the instance from the tree (since we need the parent relationship)
- VFS suppression must cover both the old path (being renamed from) and the new path (being renamed to)

