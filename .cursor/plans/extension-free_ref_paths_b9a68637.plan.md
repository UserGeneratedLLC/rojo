---
name: extension-free ref paths
overview: Remove file extensions from ref path segments and enforce dedup suffixes for ALL instances sharing an Instance.Name, regardless of middleware/file type. Ref paths change from `Workspace/Foo.server.luau` to `Workspace/Foo`, and siblings like `LocalScript.local.luau` + `LocalScript.client.luau` now become `LocalScript.local.luau` + `LocalScript~2.client.luau`.
todos:
  - id: strip-ext
    content: Add `strip_known_extensions()` utility to `file_names.rs` with unit tests
    status: pending
  - id: dedup-bare-slug
    content: Change `deduplicate_name_with_ext()` and `name_for_inst()` to use bare slug for collision detection; update dedup_key semantics
    status: pending
  - id: taken-names-sites
    content: Update ALL taken_names seeding sites (dir.rs, project.rs, api.rs) to use bare slugs via strip_known_extensions
    status: pending
  - id: ref-segment
    content: Add `ref_segment_for()` to tree.rs; update `walk_segments()` and `get_instance_by_path()` to match bare stems
    status: pending
  - id: ref-construction
    content: Update `ref_target_path_from_tree()`, `tentative_fs_name()`, `added_instance_fs_segment()`, `record_ref_path()` to use bare stems
    status: pending
  - id: cleanup-grouping
    content: Update change_processor.rs cleanup to group by bare stem (remove extension filter), refactor compute_cleanup_action for mixed-extension groups
    status: pending
  - id: unit-tests-dedup
    content: Update all file_names.rs unit tests for new bare-slug taken_names semantics; add same-name-different-type tests
    status: pending
  - id: unit-tests-ref
    content: Update rojo_ref.rs unit tests for extension-free ref path format
    status: pending
  - id: forward-sync-constraint
    content: "Add forward sync handling for bare-stem uniqueness constraint: walk_segments backward compat fallback for old-format refs, and warning/auto-dedup for legacy same-bare-stem files"
    status: pending
  - id: integration-tests
    content: Add build test fixture for same-name-different-type dedup; add connected_mode cleanup test; update snapshot expectations with `cargo insta review`
    status: pending
isProject: false
---

# Extension-Free Ref Paths

## Core Semantic Change

**Old behavior:** Dedup key = full filesystem name (slug + extension). `Foo.server.luau` and `Foo.luau` do NOT collide. Ref paths include extensions: `Workspace/Foo.server.luau`.

**New behavior:** Dedup key = bare slug only (no extension). ALL instances with the same slug collide and get `~2`, `~3`, etc. Ref paths use bare stems: `Workspace/Foo`, `Workspace/Foo~2`.

`~1` is never produced (already the case, maintained).

## Hard Constraint: Bare Stem Uniqueness

**Files in the same directory can NEVER share a primary name (bare stem), even if they have different types/extensions.** This is the foundational invariant of this change. Previously, `Foo.server.luau` and `Foo.luau` could coexist because their full filenames differed. Now they CANNOT -- one must get a dedup suffix (`Foo~2`).

This constraint must be enforced at multiple levels:

1. **Writeout (syncback/two-way sync):** The dedup system uses bare slugs in `taken_names`, so same-named instances of different types always get `~2`, `~3`, etc. This is the primary enforcement mechanism.
2. **Forward sync (reading files):** When reading a directory, if two files share a bare stem (legacy state from before this change), forward sync needs to handle it. The `walk_segments` resolver must support a **backward compatibility fallback** that matches old-format refs (with extensions) so existing projects don't break. Additionally, we should log a warning when same-bare-stem siblings are detected, guiding users to re-run syncback to fix the state.
3. **Ref path resolution backward compat:** `walk_segments()` should have a 3-tier fallback:
  - **Primary:** Match against `ref_segment_for()` (bare stem) -- new format
  - **Fallback 1:** Match against `filesystem_name_for()` (full filename) -- old format backward compat
  - **Fallback 2:** Match against instance name -- existing fallback

This ensures old ref paths like `./Foo.server.luau` still resolve correctly until the project is re-synced.

## Phase 1: Core Dedup Change

### 1a. Add `strip_known_extensions()` to [file_names.rs](src/syncback/file_names.rs)

New utility function that strips known middleware extensions from a filename to recover the bare stem (slug + dedup suffix). Must check compound extensions before simple ones:

```rust
pub fn strip_known_extensions(filename: &str) -> &str {
    const COMPOUND: &[&str] = &[
        ".server.luau", ".client.luau", ".plugin.luau", ".local.luau", ".legacy.luau",
        ".server.lua", ".client.lua", ".plugin.lua",
        ".model.json5", ".model.json",
        ".project.json5", ".project.json",
        ".meta.json5", ".meta.json",
    ];
    const SIMPLE: &[&str] = &[
        ".luau", ".lua", ".json5", ".json", ".csv", ".txt",
        ".rbxm", ".rbxmx", ".toml", ".yml", ".yaml",
    ];
    for ext in COMPOUND { if let Some(s) = filename.strip_suffix(ext) { return s; } }
    for ext in SIMPLE { if let Some(s) = filename.strip_suffix(ext) { return s; } }
    filename
}
```

### 1b. Modify `deduplicate_name_with_ext()` in [file_names.rs](src/syncback/file_names.rs)

Change collision check from full filesystem name to bare slug:

```rust
// OLD: checks "foo.server.luau" against taken_names
// NEW: checks "foo" (bare slug) against taken_names
if !taken_names.contains(&base.to_lowercase()) { ... }
```

`taken_names` now stores bare slugs (lowercased, no extensions).

### 1c. Modify `name_for_inst()` in [file_names.rs](src/syncback/file_names.rs)

- **New instance branch:** `dedup_key` = bare slug (with dedup suffix if applied), NOT full filesystem name.
- **Old instance branch:** `dedup_key` = `strip_known_extensions(filename)` (bare stem from existing file).
- Doc comments updated to reflect new semantics.

### 1d. Update all `taken_names` seeding sites

These files seed `taken_names` from existing filesystem entries and must switch to bare slugs:

- [dir.rs](src/snapshot_middleware/dir.rs) lines 224-242: `taken_names.insert(filename.to_lowercase())` must become `taken_names.insert(strip_known_extensions(filename).to_lowercase())`
- [project.rs](src/snapshot_middleware/project.rs) lines 736-744: same change
- [api.rs](src/web/api.rs): any taken_names seeding from tree siblings

## Phase 2: Ref Path Segment Change

### 2a. Add `ref_segment_for()` to [tree.rs](src/snapshot/tree.rs)

New method on `RojoTree` that returns the bare stem for use in ref paths. Keep `filesystem_name_for()` unchanged for any non-ref-path callers (verify first).

```rust
pub fn ref_segment_for(&self, id: Ref) -> String {
    if let Some(meta) = self.metadata_map.get(&id) {
        if let Some(source) = &meta.instigating_source {
            match source {
                InstigatingSource::Path(_) => {
                    if let Some(name) = source.path().file_name().and_then(|f| f.to_str()) {
                        return strip_known_extensions(name).to_string();
                    }
                }
                InstigatingSource::ProjectNode { .. } => {
                    if let Some(inst) = self.inner.get_by_ref(id) {
                        return inst.name.clone();
                    }
                }
            }
        }
    }
    self.inner.get_by_ref(id).map(|i| i.name.clone()).unwrap_or_default()
}
```

For `Foo~2.server.luau` this returns `Foo~2`. For `Foo/` (dir) this returns `Foo`. For project nodes, returns instance name.

### 2b. Update `walk_segments()` and `get_instance_by_path()` in [tree.rs](src/snapshot/tree.rs)

Replace `filesystem_name_for()` with `ref_segment_for()` in the segment matching loop. The fallback to instance name matching remains.

### 2c. Update `ref_target_path_from_tree()` in [rojo_ref.rs](src/rojo_ref.rs)

Use `tree.ref_segment_for()` instead of `tree.filesystem_name_for()` when building path segments.

### 2d. Update `tentative_fs_name()` in [ref_properties.rs](src/syncback/ref_properties.rs)

Return just the bare slug (slugified instance name, no extension). This produces tentative ref-path segments like `Foo` instead of `Foo.server.luau`.

### 2e. Update `added_instance_fs_segment()` in [api.rs](src/web/api.rs)

Return bare slug (slugified instance name, no extension) for same-batch ref target paths.

### 2f. Update `record_ref_path()` in [snapshot.rs](src/syncback/snapshot.rs)

Currently passes the full filename (e.g., `"Foo~2.server.luau"`) as the child segment. Must pass the bare stem instead (e.g., `"Foo~2"`). The `name` variable from `name_for_inst()` is the full filename, so we need to derive the bare stem -- use `strip_known_extensions(&name)`.

## Phase 2.5: Forward Sync Constraint Enforcement

### 2.5a. Update `walk_segments()` with 3-tier fallback in [tree.rs](src/snapshot/tree.rs)

The current 2-tier fallback (filesystem_name_for -> instance name) becomes 3-tier:

```rust
// Tier 1: match ref segment (bare stem) -- new format
for &child_ref in children {
    let ref_seg = self.ref_segment_for(child_ref);
    if ref_seg.eq_ignore_ascii_case(segment) { ... break; }
}
// Tier 2: match full filesystem name -- backward compat for old refs
if !found {
    for &child_ref in children {
        let fs_name = self.filesystem_name_for(child_ref);
        if fs_name.eq_ignore_ascii_case(segment) { ... break; }
    }
}
// Tier 3: match instance name -- existing fallback
if !found {
    for &child_ref in children {
        if child.name.eq_ignore_ascii_case(segment) { ... break; }
    }
}
```

Same structure for `get_instance_by_path()`.

### 2.5b. Add bare-stem collision warning in forward sync

In [dir.rs](src/snapshot_middleware/dir.rs) `snapshot_dir_no_meta()`, after collecting all child snapshots, check for bare-stem collisions among siblings. If found, log a warning like:

```
Warning: Files "Foo.server.luau" and "Foo.luau" share bare stem "Foo" -- 
this is ambiguous for ref paths. Run syncback to fix.
```

This is a non-fatal warning. The forward sync still works (instances get their correct names/classes), but ref resolution may be ambiguous until syncback adds dedup suffixes.

## Phase 3: Dedup Cleanup Update

### 3a. Update cleanup grouping in [change_processor.rs](src/change_processor.rs) (lines 1090-1156)

Currently groups siblings by matching extension (`sibling_ext != removed_extension`). Remove this extension filter -- all siblings with the same bare stem should be in the same dedup group regardless of extension.

- `removed_stem` derivation: use `strip_known_extensions(removed_fs_name)` instead of splitting on first `.`
- `sibling_stem` derivation: use `strip_known_extensions(sibling_fs)` instead of splitting on first `.`
- Remove the `sibling_ext != removed_extension` guard entirely
- `compute_cleanup_action()` `extension` parameter: still needed for building filesystem paths. Pass `None` for the extension since cleanup now operates on bare stems. The renamed file keeps its original extension -- we just need to rename `Foo~2.server.luau` to `Foo.server.luau`. The extension is derived from the actual file's path, not from the dedup group.

Wait -- `compute_cleanup_action()` builds rename paths using `build_dedup_name(base_stem, suffix, extension)`. It needs the extension to know the full filename. Since siblings now have DIFFERENT extensions within the same group, the survivor's extension must be read from its actual filesystem name. This requires a small refactor of `compute_cleanup_action()` or the caller: instead of passing a single `extension`, the cleanup should track each remaining sibling's full filename so renames produce correct paths.

**Approach:** In the caller loop in `change_processor.rs`, track `remaining_entries: Vec<(stem, full_fs_name)>` instead of just `remaining_stems: Vec<String>`. The cleanup action can then derive the correct filesystem path from the survivor's actual full name.

### 3b. Update `compute_cleanup_action()` in [dedup_suffix.rs](src/syncback/dedup_suffix.rs)

Modify to accept `remaining_entries` with both stem and full filename info, so it can build correct rename paths for mixed-extension groups.

## Phase 4: Tests

### 4a. Update unit tests in [file_names.rs](src/syncback/file_names.rs)

**All tests using full filesystem names in `taken_names`** must switch to bare slugs:

- `nfi_dedup_collision`: `"foo.luau"` -> `"foo"`
- `nfi_slug_collision_file_middleware`: `"hey_bro.luau"` -> `"hey_bro"`  
- `disk_seed_file_middleware_*`: entries like `"mymodule.luau"` -> `"mymodule"`
- `nightmare_mixed_middleware_*`: These tests will fundamentally change. `Folder "Shared"` and `ModuleScript "Shared"` now DO collide (same bare slug "shared") where they previously didn't.

**New tests to add:**

- `same_name_different_type_dedup`: Two instances "Foo" (ModuleScript + ServerScript) get dedup: `Foo.luau`, `Foo~2.server.luau`
- `same_name_dir_and_file_dedup`: Folder "Foo" and Script "Foo" get dedup: `Foo/`, `Foo~2.server.luau`
- `three_way_mixed_type_dedup`: Three instances "X" (ModuleScript, ServerScript, Folder) all dedup
- `dedup_key_is_bare_slug`: Verify dedup_key returned by `name_for_inst()` is the bare slug

### 4b. Add `strip_known_extensions` unit tests in [file_names.rs](src/syncback/file_names.rs)

Test all compound and simple extension stripping, including edge cases (dots in instance names, no extension, etc.).

### 4c. Update ref path unit tests in [rojo_ref.rs](src/rojo_ref.rs)

Update expected ref path formats: `"Workspace/Foo.luau"` -> `"Workspace/Foo"`, etc.

### 4d. Add build test fixture

New test in [build-tests/](rojo-test/build-tests/) for same-name-different-type instances. Verify the build output snapshot shows correct dedup suffixes.

### 4e. Add connected_mode test

Test dedup cleanup when a mixed-type group member is deleted. Verify the survivor's suffix is cleaned correctly even though siblings have different extensions.

### 4f. Update ALL existing snapshot test expectations

Run `cargo test` and `cargo insta review` to update snapshot files affected by the new ref path format and dedup behavior.

## Key Files Summary


| File                                                                     | Change Type                                                               |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| [src/syncback/file_names.rs](src/syncback/file_names.rs)                 | Add `strip_known_extensions`, modify dedup logic, update tests            |
| [src/snapshot/tree.rs](src/snapshot/tree.rs)                             | Add `ref_segment_for`, update `walk_segments`, `get_instance_by_path`     |
| [src/rojo_ref.rs](src/rojo_ref.rs)                                       | Update `ref_target_path_from_tree`, update tests                          |
| [src/syncback/ref_properties.rs](src/syncback/ref_properties.rs)         | Update `tentative_fs_name` to return bare slug                            |
| [src/web/api.rs](src/web/api.rs)                                         | Update `added_instance_fs_segment`, taken_names seeding                   |
| [src/change_processor.rs](src/change_processor.rs)                       | Remove extension-based grouping, update ref paths                         |
| [src/syncback/dedup_suffix.rs](src/syncback/dedup_suffix.rs)             | Update `compute_cleanup_action` for mixed-extension groups                |
| [src/syncback/snapshot.rs](src/syncback/snapshot.rs)                     | Update `record_ref_path` to use bare stems                                |
| [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs)         | Update taken_names seeding to bare slugs; add bare-stem collision warning |
| [src/snapshot_middleware/project.rs](src/snapshot_middleware/project.rs) | Update taken_names seeding to bare slugs                                  |


## Risk Areas

1. `**compute_cleanup_action` refactor** -- The most complex change. Currently assumes all group members share an extension. Must now handle mixed extensions and still produce correct filesystem rename paths.
2. **Custom sync rules** -- `strip_known_extensions()` only handles built-in extensions. Custom `syncRules` patterns won't be stripped. Acceptable for initial implementation; the ref segment will include the custom extension, and resolution will match correctly since both sides use the same derivation.
3. **Snapshot test churn** -- Many snapshot tests will have new expected output. Use `cargo insta review` to batch-accept.
4. **Legacy project migration** -- Projects with same-bare-stem files (e.g., `Foo.server.luau` + `Foo.luau` in same dir) are now in an invalid state. The 3-tier `walk_segments` fallback ensures old ref paths still resolve, but the ambiguity warning encourages users to re-run syncback. The first syncback after this change will automatically fix the filesystem by applying dedup suffixes.

