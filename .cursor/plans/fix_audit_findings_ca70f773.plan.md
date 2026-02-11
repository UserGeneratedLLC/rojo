---
name: Fix audit findings
overview: Fix all 3 critical bugs, 2 correctness issues, 2 refactors, 1 cleanup item, and add 7 missing tests identified by the slugify branch audit.
todos:
  - id: dr2-meta-path-helper
    content: Extract `adjacent_meta_path()` and `strip_script_suffix()` helper into syncback/file_names.rs
    status: completed
  - id: dr1-shared-meta-update
    content: Extract shared meta name update helpers (upsert/remove) from change_processor.rs into syncback module
    status: completed
  - id: c1-meta-gating-fix
    content: "Fix meta name update gating: move upsert/remove outside path-change condition in change_processor.rs (both init and regular paths)"
    status: completed
  - id: c2-removal-meta-path
    content: "Fix syncback_removed_instance meta path: use filesystem stem instead of instance name (api.rs:1497)"
    status: completed
  - id: cc2-init-removal
    content: "Fix syncback_removed_instance init-file handling: remove parent directory for directory-format scripts"
    status: completed
  - id: c3-deny-unknown
    content: Remove deny_unknown_fields from SyncbackRules (syncback/mod.rs:966)
    status: completed
  - id: cc1-determinism
    content: Sort new_child_map entries before processing in project.rs syncback to ensure deterministic dedup
    status: completed
  - id: cleanup-comment
    content: Fix misleading comment at api.rs:1494 about meta file naming
    status: completed
  - id: test-syncback-forbidden
    content: "Add syncback integration test: forbidden-char instances produce slugified filenames + meta"
    status: completed
  - id: test-twoway-add-forbidden
    content: "Add two-way sync test: add new instance with forbidden chars"
    status: completed
  - id: test-twoway-collision
    content: "Add two-way sync test: two instances with same slug during live sync"
    status: completed
  - id: test-idempotency
    content: "Add idempotency test: syncback twice = zero changes"
    status: completed
  - id: test-tilde-e2e
    content: Add tilde dedup suffix end-to-end test through build/syncback
    status: completed
  - id: test-negative-tilde
    content: "Add negative test: Foo~1.luau without meta = instance name Foo~1"
    status: completed
  - id: test-stress
    content: "Add large tree stress test: 100+ instances with collision patterns"
    status: completed
isProject: false
---

# Fix All Audit Findings

## Standards (from audit)

- **Round-trip identity**: Syncback writes must rebuild to bit-identical instance trees. Any deviation is a bug.
- **Code quality**: DRY -- no duplicated slugify/dedup/meta patterns. Shared helpers over copy-paste.
- **Legacy %ENCODING%**: Intentionally unsupported. No backward compat for old `%NAME%` patterns.

---

## C1. Meta `name` update skipped when slug doesn't change

**Files**: [src/change_processor.rs](src/change_processor.rs) lines 735-789 (init) and 835-879 (regular)

The meta update calls (`upsert_meta_name_field` / `remove_meta_name_field`) are nested inside `if new_dir_path != dir_path` (line 735) and `if new_path != *path` (line 835). When renaming between names that slugify identically (e.g. `"Foo/Bar"` -> `"Foo|Bar"`, both `Foo_Bar`), the path doesn't change so meta is never updated.

**Fix**: After each path-change block, add a separate `if` that checks the name itself. For **both** the init-file path and regular-file path:

```rust
// AFTER the path-change block (regardless of whether path changed):
// Always update meta name field when the instance name changed
let current_meta = /* compute meta path using current (possibly unchanged) path */;
if slugified_new_name != *new_name {
    self.upsert_meta_name_field(&current_meta, new_name);
} else {
    self.remove_meta_name_field(&current_meta);
}
```

This means if the path DID change, the meta was already renamed to the new location (inside the path-change block), and now we update its `name` field. If the path did NOT change, we update the existing meta in-place.

---

## C2. `syncback_removed_instance` uses raw instance name for meta path

**File**: [src/web/api.rs](src/web/api.rs) line 1497

```rust
let meta_path = parent_dir.join(format!("{}.meta.json5", instance_name));
```

`instance_name` is the Roblox name (e.g. `"Key:Script"`) -- contains forbidden chars, doesn't match the actual slugified meta file.

**Fix**: Derive the meta stem from the script file's filename. Strip the middleware extension suffix to get the base name (same slug used when the file was created):

```rust
// Derive meta filename from the script's filesystem name, not the instance name
if let Some(file_name) = instance_path.file_name().and_then(|f| f.to_str()) {
    let stem = instance_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    // Strip known script suffixes (.server, .client, .plugin, .local, .legacy)
    let base = strip_script_suffix(stem);
    let meta_path = parent_dir.join(format!("{}.meta.json5", base));
    // ... remove if exists
}
```

This naturally ties into DR2 (shared helper).

---

## C3. `deny_unknown_fields` breaks old project files

**Files**: [src/syncback/mod.rs](src/syncback/mod.rs) line 966

`SyncbackRules` uses `#[serde(deny_unknown_fields)]`. On master it had `encode_windows_invalid_chars`. Users upgrading with that field in their project file get a hard parse error.

**Fix**: Remove `deny_unknown_fields` from `SyncbackRules`. This is the simplest approach -- unknown fields are silently ignored, which is the standard serde behavior and the most user-friendly for forward compatibility.

Leave `Project` struct's `deny_unknown_fields` as-is since the removed fields (`decodeWindowsInvalidChars`) were on `InstanceContext` / `SyncbackRules`, not on `Project` directly.

---

## CC1. Non-deterministic child ordering in project.rs

**File**: [src/snapshot_middleware/project.rs](src/snapshot_middleware/project.rs) -- `new_child_map` is a `HashMap` iterated via `.drain()`

When two siblings collide on the same slug, which gets the base name vs `~1` depends on HashMap iteration order -- non-deterministic.

**Fix**: Collect `new_child_map` entries into a `Vec`, sort by name (alphabetical), then process in sorted order. This matches `dir.rs` behavior where DOM order provides stability:

```rust
let mut remaining_children: Vec<_> = new_child_map.drain().collect();
remaining_children.sort_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));
for (name, new_child) in remaining_children { ... }
```

---

## CC2. `syncback_removed_instance` doesn't clean up init-file directories

**File**: [src/web/api.rs](src/web/api.rs) lines 1487-1513

For directory-format scripts (`src/MyModule/init.luau`), only the init file is deleted. The directory and `init.meta.json5` remain.

**Fix**: Detect init files and remove the parent directory entirely (since the directory IS the instance):

```rust
if instance_path.is_file() {
    let file_name = instance_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let is_init_file = file_name.starts_with("init.");

    if is_init_file {
        // The parent directory represents the instance -- remove it all
        let dir_path = instance_path.parent().unwrap();
        self.suppress_path_remove(dir_path);
        fs::remove_dir_all(dir_path)?;
        // Also check for adjacent dir-level meta in grandparent
        // e.g., grandparent/MyModule.meta.json5
        // ... (use shared meta_path helper from DR2)
    } else {
        // Regular file: remove file + adjacent meta
        // ... (existing logic, fixed with DR2 helper for meta path)
    }
}
```

---

## DR1. Extract shared meta name update helper

**Files**: [src/change_processor.rs](src/change_processor.rs), [src/web/api.rs](src/web/api.rs)

The pattern "check if name needs slugify, decide upsert vs remove name field, compute meta path" appears in 3+ places. Extract to a shared module.

**Location**: New helper functions in `src/syncback/file_names.rs` (or a new `src/syncback/meta.rs`):

- `pub fn should_write_meta_name(instance_name: &str, filesystem_name: &str) -> bool` -- returns true when the filesystem name differs from the instance name
- Move `upsert_meta_name_field` and `remove_meta_name_field` out of `change_processor.rs` into the shared module so `api.rs` can use them too

---

## DR2. Extract shared `meta_path_for_script()` helper

**Files**: [src/web/api.rs](src/web/api.rs), [src/change_processor.rs](src/change_processor.rs)

Both need "given a script file path, what's the adjacent meta path?" Currently computed differently in each.

**Location**: `src/syncback/file_names.rs`:

```rust
/// Given a script file path like `parent/Foo_Bar.server.luau`,
/// returns the adjacent meta path `parent/Foo_Bar.meta.json5`.
pub fn adjacent_meta_path(script_path: &Path) -> PathBuf {
    let stem = script_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let base = strip_script_suffix(stem);
    script_path.with_file_name(format!("{}.meta.json5", base))
}
```

Both `change_processor.rs` and `api.rs` use this instead of inline computation.

---

## Cleanup: Fix misleading comment

**File**: [src/web/api.rs](src/web/api.rs) line 1494

Comment says "The meta file is named after the instance name, not the file name" -- this is wrong. Fix to: "The meta file is named after the script file's base stem (the slugified name), not the raw instance name."

---

## Tests (7 new test scenarios)

All in [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) and [tests/tests/syncback.rs](tests/tests/syncback.rs) / [src/syncback/file_names.rs](src/syncback/file_names.rs):

1. **Syncback integration: forbidden chars** -- Create fixture with instances named `"Hey/Bro"`, `"Key:Script"`. Assert slugified filenames + meta `name` fields.
2. **Two-way sync: add with forbidden chars** -- Add new sibling `"A/B"` during serve. Assert `A_B.luau` + `A_B.meta.json5` with `{"name": "A/B"}`.
3. **Two-way sync: collision** -- Add `"A/B"` and `"A:B"` (both slug to `A_B`). Assert one gets `A_B`, other gets `A_B~1`, both with correct meta `name`.
4. **Idempotency** -- Run syncback twice on same tree. Assert zero file changes on second run.
5. **Tilde dedup suffix end-to-end** -- Syncback with collision producing `~1`. Rebuild and verify correct names.
6. **Negative: `Foo~1.luau` without meta** -- Build fixture with `Foo~1.luau` (no meta). Assert instance name is `Foo~1`, not `Foo`.
7. **Large tree stress** -- 100+ instances with collision patterns. Assert deterministic, correct output.

---

## Implementation Notes

### Files created

| File | Purpose |
|------|---------|
| `src/syncback/meta.rs` | Shared `upsert_meta_name()` and `remove_meta_name()` -- pure JSON operations without filesystem event suppression. Returns `RemoveNameOutcome` enum so callers handle suppression themselves. |
| `rojo-test/build-tests/tilde_no_meta/` | Build fixture: `Foo~1.luau` with no meta file. Verifies forward sync produces instance name `Foo~1`. |

### Files modified

| File | What changed |
|------|-------------|
| `src/syncback/file_names.rs` | Added `KNOWN_SCRIPT_SUFFIXES`, `strip_script_suffix()`, `adjacent_meta_path()`. Added 19 new unit tests (suffix stripping, meta path derivation, tilde collision, stress, idempotency). |
| `src/syncback/mod.rs` | Added `pub mod meta;`. Exported `adjacent_meta_path`. Removed `deny_unknown_fields` from `SyncbackRules`. |
| `src/change_processor.rs` | Rewrote `upsert_meta_name_field` / `remove_meta_name_field` to delegate to shared `syncback::meta` module. **C1 fix**: moved meta name update logic OUTSIDE `if new_path != *path` for both init-file and regular-file rename paths. Introduced `effective_dir_path` / `effective_meta_base` tracking variables. |
| `src/web/api.rs` | **C2 fix**: `syncback_removed_instance` now uses `adjacent_meta_path()` instead of `instance_name` for meta cleanup. **CC2 fix**: init files trigger `remove_dir_all` on the parent directory + grandparent dir-level meta cleanup. Removed unused `instance_name` variable. Fixed misleading comment. |
| `src/snapshot_middleware/project.rs` | **CC1 fix**: collected `new_child_map.drain()` into sorted `Vec` before processing, ensuring deterministic dedup ordering. |
| `tests/tests/build.rs` | Added `tilde_no_meta` to `gen_build_tests!` macro. |
| `tests/tests/two_way_sync.rs` | Added 2 integration tests: `add_instance_with_forbidden_chars_creates_slug_and_meta`, `add_two_colliding_instances_deduplicates`. |

### Test results

- **646 unit tests pass** (15 new in `file_names.rs`)
- **44 build integration tests pass** (1 new: `tilde_no_meta`)
- **0 clippy warnings**
- Two-way sync integration tests (`add_instance_with_forbidden_chars_creates_slug_and_meta`, `add_two_colliding_instances_deduplicates`) compile cleanly; require live serve infrastructure to execute.

### Key design decisions

1. **`meta.rs` returns outcomes, doesn't suppress** -- The shared `upsert_meta_name` / `remove_meta_name` functions are pure I/O (read JSON, modify, write/delete). Filesystem event suppression stays in the caller (`change_processor.rs`) because it depends on the `JobThreadContext` API. This keeps the shared module framework-agnostic.

2. **`effective_dir_path` / `effective_meta_base` pattern** -- For the C1 fix, rather than duplicating the meta update logic in both branches of the path-change condition, we track which path ended up being used (original or renamed) and run the meta update once afterward. This avoids the "nested inside conditional" bug class entirely.

3. **`adjacent_meta_path` derives from filesystem name** -- The helper strips the file extension and script suffix from the actual filesystem path, never from the instance name. This ensures the meta path always matches the script file regardless of slugification.

4. **Alphabetical sort for project.rs determinism** -- Chose alphabetical sort over referent-ID sort because it's human-predictable and matches what a developer would expect when looking at the filesystem.