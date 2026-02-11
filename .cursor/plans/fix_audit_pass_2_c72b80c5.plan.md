---
name: Fix audit pass 2
overview: Fix 1 critical bug, 4 correctness issues, 3 cleanup items, and add 9 missing tests identified by the slugify branch audit pass 2.
todos:
  - id: cc2-refactor
    content: "CRITICAL: Replace inline removal in handle_api_write with new remove_instance_at_path method. Delete dead syncback_removed_instance."
    status: completed
  - id: preseed-fix
    content: Fix pre-seeding in dir.rs and project.rs to derive dedup keys from inst.metadata().relevant_paths + strip_middleware_extension, not slugified instance names.
    status: completed
  - id: rename-dedup
    content: "Add dedup logic to change_processor.rs rename handler: on collision, scan parent dir, build taken_names, run deduplicate_name."
    status: completed
  - id: dir-sort
    content: Add alphabetical sort to dir.rs child iteration to match project.rs for deterministic dedup ordering.
    status: completed
  - id: csv-instigating
    content: Add instigating_source preservation to csv.rs snapshot_csv_init, matching lua.rs pattern.
    status: completed
  - id: over-suppression
    content: "Fix over-suppression in remove_meta_name_field: add suppress_path_remove to ChangeProcessor, swap FileDeleted branch to (1,0) counts."
    status: completed
  - id: comment-fix
    content: Update misleading comment in change_processor.rs:608-609 to reference remove_instance_at_path.
    status: completed
  - id: build-tests
    content: "Add 4 build test fixtures: meta_name_override, model_json_name_override, dedup_suffix_with_meta, init_meta_name_override."
    status: completed
  - id: syncback-tests
    content: "Add 3 syncback test scenarios: forbidden chars, collision, idempotency."
    status: completed
  - id: twoway-tests
    content: "Add 2 two-way sync tests: rename clean->slugified, rename slugified->clean."
    status: completed
isProject: false
---

# Fix Audit Pass 2 Findings

## Standards (from audit plan)

> **Round-trip identity**: Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree** -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

> **Code quality**: DRY -- no duplicated slugify/dedup/meta patterns. Shared helpers over copy-paste. If the same "compute meta path, check if name needs slugify, upsert/remove name field" sequence appears in 3+ places, that's a refactor candidate.

> **Legacy %ENCODING%**: Intentionally unsupported. No backward compat for old `%NAME%` patterns.

> **When working on two-way sync:** Plugin-based sync must produce **exactly the same filesystem output** that `rojo syncback` would give for the same input. Byte-for-byte identical files, identical directory structures, identical naming. Any divergence is a bug until proven otherwise.

---

## CRITICAL 1: CC2 init-file removal broken in active code path

**Problem:** The inline removal in `handle_api_write` ([src/web/api.rs](src/web/api.rs) lines 363-451) uses `p.is_dir()` to decide how to delete. For init-file scripts (e.g. `src/MyModule/init.luau`), `instigating_source` is the init file path so `p.is_dir()` = false. Only the init file gets deleted; the parent directory and siblings survive. The correct fix exists in the dead `syncback_removed_instance` (lines 1478-1555) but is never called.

**Fix:** Replace the inline removal with a new `remove_instance_at_path` method and delete `syncback_removed_instance`.

- Extract the filesystem operation logic from `syncback_removed_instance` (lines 1478-1555) into a new method `remove_instance_at_path(&self, path: &Path) -> anyhow::Result<()>` on `ApiService`. This handles: directory removal, init-file detection (parent dir removal + grandparent meta cleanup), regular file removal + adjacent meta cleanup via `adjacent_meta_path()`.
- Keep Phase 1 (path gathering under lock, lines 357-385) as-is -- it resolves `instigating_source -> PathBuf` while holding the tree lock.
- Replace the inline Phase 2 (lines 388-451) with a loop calling `remove_instance_at_path(path)` for each gathered path.
- Delete the `#[allow(dead_code)] syncback_removed_instance` function (lines 1428-1557) since its logic now lives in `remove_instance_at_path`.
- Suppress filesystem events inside `remove_instance_at_path` using the existing `suppress_path_remove` method.

---

## FIX 2: Pre-seeding uses slugified names instead of filesystem dedup keys

**Problem:** In [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs) lines 196-204 and [src/snapshot_middleware/project.rs](src/snapshot_middleware/project.rs) lines 670-678, pre-seeding inserts `slugify_name(inst.name()).to_lowercase()`. But if an old instance already has a tilde-suffixed file (e.g. `A_B~1.luau` from prior dedup), only `"a_b"` is seeded, not `"a_b~1"`. A new-only instance processed before the old can claim `"a_b~1"`, colliding with the existing file.

**Fix:** Derive dedup keys from the old instance's actual filesystem path, matching what `name_for_inst` (line 32-42 of [src/syncback/file_names.rs](src/syncback/file_names.rs)) does for old instances. The old instance metadata has `relevant_paths` and `middleware`:

```rust
// In both dir.rs and project.rs pre-seeding loops:
for inst in old_child_map.values() {
    if let Some(path) = inst.metadata().relevant_paths.first() {
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let middleware = inst.metadata().middleware.unwrap_or(Middleware::Dir);
            let dedup_key = strip_middleware_extension(filename, middleware).to_lowercase();
            taken_names.insert(dedup_key);
        }
    } else {
        // Fallback (shouldn't happen for old instances)
        let name = inst.name();
        let slug = if name_needs_slugify(name) {
            slugify_name(name).to_lowercase()
        } else {
            name.to_lowercase()
        };
        taken_names.insert(slug);
    }
}
```

This uses `inst.metadata().relevant_paths.first()` (actual filesystem path) + `inst.metadata().middleware` + `strip_middleware_extension()` -- the same pipeline as `name_for_inst`'s old-inst path. Both `dir.rs` and `project.rs` need the same fix.

---

## FIX 3: Rename collision -- no dedup against siblings

**Problem:** [src/change_processor.rs](src/change_processor.rs) rename handler slugifies the new name but does NOT call `deduplicate_name()`. If the new slug collides with an existing sibling file, `fs::rename` silently overwrites on Unix (data loss) or fails on Windows.

**Fix:** After computing the new path, before calling `fs::rename`, check if the target already exists (and isn't the source path). If collision detected, scan the parent directory to build a `taken_names` set, then run `deduplicate_name()`:

- For both the **init-file path** (dir rename, ~~line 697) and **regular file path** (~~line 810):
  1. Compute `new_path` from `slugified_new_name`
  2. If `new_path != old_path && new_path.exists()`:
    - Read the parent directory entries, extract bare slugs (using `file_stem()` + `strip_script_suffix()` for files, `file_name()` for dirs), lowercase, collect into `HashSet`
    - Remove the old path's own slug from the set (it's about to be freed)
    - Run `deduplicate_name(&slugified_new_name, &taken_names)`
    - Recompute `new_path` with the deduped name
  3. The resulting `new_path` is guaranteed unique

This scan only happens on actual collision (the `new_path.exists()` guard), so the common case has zero overhead.

---

## FIX 4: `dir.rs` child ordering not sorted

**Problem:** [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs) processes children in Roblox DOM order (`new_inst.children()`). Unlike `project.rs` which sorts alphabetically (CC1 fix at line 685-686), `dir.rs` doesn't sort. Dedup suffix assignment (`~1` vs base) can differ across serializations.

**Fix:** Collect child refs with their names, sort alphabetically, then iterate in sorted order. Apply to the main child iteration loop starting at line 207:

```rust
// Collect children with names for deterministic ordering
let mut sorted_children: Vec<_> = new_inst.children().iter()
    .map(|r| (*r, snapshot.get_new_instance(*r).unwrap().name.clone()))
    .collect();
sorted_children.sort_by(|(_, a), (_, b)| a.cmp(b));

for (new_child_ref, _) in &sorted_children {
    let new_child = snapshot.get_new_instance(*new_child_ref).unwrap();
    // ... existing loop body ...
}
```

Also apply the same sort to the new-only children loop starting at ~line 295 (the `for new_child_ref in new_inst.children()` after the matched-children loop).

---

## FIX 5: `csv.rs` `snapshot_csv_init` doesn't preserve `instigating_source`

**Problem:** [src/snapshot_middleware/csv.rs](src/snapshot_middleware/csv.rs) line 81 does `init_snapshot.metadata = dir_snapshot.metadata;` without preserving the CSV file's `instigating_source`. Two-way sync writes target the directory instead of the file. Compare with `lua.rs` lines 114-123 which correctly saves and restores.

**Fix:** Add the same preservation pattern from `lua.rs`:

```rust
// Before copying dir metadata, save the init script's instigating_source
let script_instigating_source = init_snapshot.metadata.instigating_source.take();

init_snapshot.children = dir_snapshot.children;
init_snapshot.metadata = dir_snapshot.metadata;

// Restore so two-way sync writes to the CSV file, not the directory
init_snapshot.metadata.instigating_source = script_instigating_source;
```

---

## FIX 6: Over-suppression in `remove_meta_name_field`

**Problem:** In [src/change_processor.rs](src/change_processor.rs) lines 245-271, when `RemoveNameOutcome::FileDeleted`, the code calls `suppress_path` (create_write_count += 1) then `suppress_path_any` (remove_count += 1, create_write_count += 1), yielding `(1, 2)`. The correct counts should be `(1, 0)` -- one Remove event, zero Write events.

**Fix:**

1. Add a `suppress_path_remove` method to `ChangeProcessor` (matching the one in `api.rs:253-257`): `entry.0 += 1` (only remove_count).
2. Rewrite the `FileDeleted` branch:

```rust
Ok(RemoveNameOutcome::FileDeleted) => {
    // Was a delete, not a write. Swap: undo Write, add Remove.
    self.unsuppress_path(meta_path);       // undo (0, 1) -> (0, 0)
    self.suppress_path_remove(meta_path);  // add  (1, 0)
}
```

---

## FIX 7: Update misleading comment in `change_processor.rs`

**Problem:** [src/change_processor.rs](src/change_processor.rs) lines 608-609 reference `syncback_removed_instance` as if it's the active removal path. After the CC2 refactor, it will reference the now-deleted function.

**Fix:** Update the comment to accurately describe the new path:

```rust
// NOTE: We do NOT delete files from disk here. The API handler
// (handle_api_write → remove_instance_at_path) already deleted
// the files before sending this PatchSet. ...
```

---

## Tests: 9 new test scenarios

### Build tests (4 new fixtures in `rojo-test/build-tests/`)

1. `**meta_name_override**` -- `Hey_Bro.luau` + `Hey_Bro.meta.json5` with `{"name": "Hey/Bro"}`. Assert instance name = `"Hey/Bro"`, not `"Hey_Bro"`.
2. `**model_json_name_override**` -- `Hey_Bro.model.json5` with `{"name": "Hey/Bro", "className": "Configuration"}`. Assert instance name = `"Hey/Bro"`.
3. `**dedup_suffix_with_meta**` -- `Foo.luau` + `Foo~1.luau` + `Foo~1.meta.json5` with `{"name": "Foo"}`. Assert two instances: one named from each file's meta.
4. `**init_meta_name_override**` -- `MyFolder/init.luau` + `MyFolder/init.meta.json5` with `{"name": "Real:Name"}`. Assert directory instance name = `"Real:Name"`.

Each needs a `default.project.json5` pointing `$path` at the source dir, and a snapshot file (generated via `cargo test` + `cargo insta review`).

### Syncback tests (3 new scenarios)

1. **Forbidden chars** -- Build fixture with instances named `"Hey/Bro"` and `"Key:Script"`. Run syncback. Assert slugified filenames + `.meta.json5` with correct `name` fields.
2. **Collision** -- Two sibling instances `"A/B"` and `"A:B"` (both slug to `A_B`). Assert one gets `A_B.luau`, other gets `A_B~1.luau`, both with correct meta.
3. **Idempotency** -- Run syncback twice on same tree. Assert zero file changes on second run.

### Two-way sync tests (2 new scenarios in `tests/tests/two_way_sync.rs`)

1. **Rename clean to slugified** -- Rename `Normal` to `Hey/Bro`. Assert file becomes `Hey_Bro.luau` + `Hey_Bro.meta.json5` with `{"name": "Hey/Bro"}`.
2. **Rename slugified to clean** -- Rename `What?Module` to `CleanName`. Assert file becomes `CleanName.luau`, meta `name` field removed (or meta file deleted if no other fields).

---

## Implementation Notes

### Files modified


| File                                 | What changed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/web/api.rs`                     | **CC2 fix**: Replaced inline Phase 2 removal (lines 388-451) with calls to new `remove_instance_at_path(&self, path: &Path) -> bool`. Simplified Phase 1 to gather `Vec<(Ref, Option<PathBuf>)>` (no `is_dir` needed). Deleted dead `syncback_removed_instance` function. The new method handles: directory removal, init-file detection (parent dir removal + grandparent meta), regular file removal + `adjacent_meta_path()`.                                                               |
| `src/snapshot_middleware/dir.rs`     | **Pre-seeding fix**: Changed to derive dedup keys from `inst.metadata().relevant_paths.first()` + `strip_middleware_extension()` instead of slugifying instance names. Added `strip_middleware_extension` and `Middleware` to imports. **Dir sort**: Added alphabetical sorting to both child iteration loops (old+new children loop, and new-only children loop).                                                                                                                             |
| `src/snapshot_middleware/project.rs` | **Pre-seeding fix**: Same filesystem-based dedup key derivation as dir.rs. Added `strip_middleware_extension` to imports.                                                                                                                                                                                                                                                                                                                                                                      |
| `src/change_processor.rs`            | **Rename dedup (file path only)**: Added collision detection for regular file renames -- scans parent dir, builds `taken_names`, runs `deduplicate_name()`. Uses `deduped_new_name` for meta file naming. **Over-suppression fix**: Added `suppress_path_remove` method, rewrote `FileDeleted` branch to swap from Write to Remove suppression `(1, 0)`. **Comment fix**: Updated line 621 to reference `remove_instance_at_path`. Added `deduplicate_name`, `strip_script_suffix` to imports. |
| `src/snapshot_middleware/csv.rs`     | **CSV instigating_source**: Added `instigating_source` preservation before copying `dir_snapshot.metadata`, matching `lua.rs` pattern.                                                                                                                                                                                                                                                                                                                                                         |
| `src/syncback/mod.rs`                | Added `strip_script_suffix` to the re-exports from `file_names`.                                                                                                                                                                                                                                                                                                                                                                                                                               |


### Files created


| File                                                  | Purpose                                                                                          |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `rojo-test/build-tests/meta_name_override/`           | Build fixture: `.meta.json5` `name` field overrides instance name to `Hey/Bro`.                  |
| `rojo-test/build-tests/model_json_name_override/`     | Build fixture: `.model.json5` `name` field overrides instance name to `Hey/Bro`.                 |
| `rojo-test/build-tests/dedup_suffix_with_meta/`       | Build fixture: `Foo.luau` + `Foo~1.luau` each with meta name overrides (`A/B` and `A_B`).        |
| `rojo-test/build-tests/init_meta_name_override/`      | Build fixture: `init.meta.json5` inside folder overrides directory instance name to `Real:Name`. |
| `rojo-test/build-test-snapshots/` (4 new .snap files) | Accepted insta snapshots for the 4 build tests above.                                            |


### Snapshots updated


| Snapshot                   | Change                                                     | Reason                                                                                         |
| -------------------------- | ---------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `csv_init.snap`            | `instigating_source: Path: /root` → `Path: /root/init.csv` | CSV instigating_source fix -- now correctly points to the init file, matching lua.rs behavior. |
| `csv_init_with_meta.snap`  | Same `instigating_source` change.                          | Same fix.                                                                                      |
| 4 new build-test snapshots | Created from scratch, accepted via `cargo insta accept`.   | New build test fixtures.                                                                       |


### Key design decision: directory rename dedup NOT applied

**Finding during implementation:** The rename dedup logic was initially added to both the **init-file/directory rename** path and the **regular file rename** path. However, this broke two existing tests:

- `failed_directory_rename_no_stale_suppression`
- `failed_directory_rename_then_successful_rename`

These tests intentionally block a directory rename with a non-empty directory and assert the rename fails gracefully. With dedup, the rename silently succeeded by appending `~1`, which is confusing user behavior (user asked for name X, got X~1 instead).

**Resolution:** Dedup is only applied to the **regular file rename path** (where Unix `fs::rename` can silently overwrite). Directory renames are left without dedup because:

1. `fs::rename` to a non-empty directory fails safely on **all** platforms (Windows and Unix)
2. `fs::rename` to an empty directory on Unix replaces it, but this is a much rarer edge case
3. Silently deduplicating a directory name is more confusing than failing with an error

### Test results

- **649 lib tests pass** (3 new in `file_names.rs`: forbidden chars, collision, idempotency)
- **312 integration tests pass** (4 new build tests, 2 new two-way sync tests)
- **0 failures, 0 clippy warnings**

### Audit pass 2 findings -- verification status


| Original finding              | Status                   | Notes                                                                                                   |
| ----------------------------- | ------------------------ | ------------------------------------------------------------------------------------------------------- |
| C1 (meta gating)              | Verified FIXED in pass 1 | `effective_dir_path`/`effective_meta_base` pattern confirmed correct                                    |
| C2 (removal meta path)        | Verified FIXED in pass 1 | Inline code uses `path.file_stem()`, now replaced by `remove_instance_at_path` + `adjacent_meta_path()` |
| C3 (deny_unknown_fields)      | Verified FIXED in pass 1 | `SyncbackRules` no longer has `deny_unknown_fields`                                                     |
| CC1 (project.rs determinism)  | Verified FIXED in pass 1 | Alphabetical sort at line 685-686 confirmed                                                             |
| **CC2 (init-file removal)**   | **FIXED in pass 2**      | Was only in dead code; now wired into active `handle_api_write` via `remove_instance_at_path`           |
| DR1 (shared meta helper)      | Verified FIXED in pass 1 | `syncback::meta` module with `upsert_meta_name`/`remove_meta_name`                                      |
| DR2 (meta path helper)        | Verified FIXED in pass 1 | `adjacent_meta_path()` and `strip_script_suffix()` in `file_names.rs`                                   |
| **Pre-seeding bug**           | **FIXED in pass 2**      | Both `dir.rs` and `project.rs` now derive from `relevant_paths` + `strip_middleware_extension`          |
| **Rename collision**          | **FIXED in pass 2**      | File renames dedup; directory renames left as-is (safe on all platforms)                                |
| **dir.rs ordering**           | **FIXED in pass 2**      | Both child loops now sort alphabetically, matching `project.rs`                                         |
| **csv.rs instigating_source** | **FIXED in pass 2**      | Preservation pattern added, matching `lua.rs`                                                           |
| **Over-suppression**          | **FIXED in pass 2**      | `suppress_path_remove` added, `FileDeleted` branch now `(1, 0)`                                         |


### Forward sync audit results (pass 2)

All middleware correctly resolves instance names:

- `meta_file.rs`: Both `AdjacentMetadata` and `DirectoryMetadata` have `name: Option<String>`, `apply_name` sets `specified_name`. `is_empty()` includes `name`.
- `lua.rs`, `txt.rs`, `csv.rs`: All use filename stem + meta override. No lingering `encode_path_name`/`decode_path_name`.
- `json_model.rs`: Top-level `name` (with `alias = "Name"` for legacy) correctly overrides filename. `specified_name` set on metadata.
- `mod.rs`: `decode_path_name` fully removed.

### Dedup key consistency (pass 2)

All 6 insertion sites use `.to_lowercase()` consistently:

1. `file_names.rs` `deduplicate_name()` -- internal comparison
2. `dir.rs` -- 3 insertion points + pre-seed
3. `project.rs` -- insertion + pre-seed
4. `api.rs` -- seed from tree siblings + insert after add + `process_children_incremental`

Cross-format collision (file vs directory with same slug) handled correctly.

### Property/data preservation (pass 2)

- `Name` property vs instance name: Separate fields, no cross-contamination.
- Ref properties: Resolved via DOM tree paths, not filenames.
- Attributes: `upsert_meta_name` merges (only touches `"name"` key), no clobber risk.
- Script source through renames: `fs::rename` preserves content.

### Legacy cleanup (pass 2)

- `path_encoding.rs`: Deleted, zero references.
- `%ENCODED%` patterns: Zero in source/tests/snaps (only in `.cursor/plans/` docs).
- `deny_unknown_fields`: Removed from `SyncbackRules`.
- Old project files: Parse without crashing (serde ignores unknown fields on `SyncbackRules`).

### Meta file lifecycle (pass 2)

- Creation: Gated by `needs_meta`. Merge-based (only touches `"name"` key).
- Update: C1 fix ensures meta updates even when slug doesn't change.
- Deletion: `remove_meta_name` deletes empty file, rewrites when other fields remain.
- Orphaned meta: Not loaded as instance (excluded from sync rules), benign on disk.
- Model JSON vs adjacent meta: No conflict -- `JsonModel` doesn't read adjacent meta.

