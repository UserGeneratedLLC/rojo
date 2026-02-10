---
name: Nuke path_encoding migration
overview: "Remove the `%NAME%` path encoding system entirely and replace it with PR #1187's `name` property approach: slugify filenames (replace forbidden chars with `_`), deduplicate with `~N` suffixes on collision, and store the real instance name in `.meta.json`/`.model.json` `name` fields."
todos:
  - id: phase1-infrastructure
    content: "Manual port of PR #1187 infrastructure: specified_name on InstanceMetadata, slugify_name(), deduplicate_name(), name field on meta file structs, apply_name() methods, json_model name field support"
    status: pending
  - id: phase2-replace-encode-decode
    content: Replace all encode_path_name/decode_path_name callsites with slugify + dedup + name property approach in snapshot middleware, syncback/file_names.rs, syncback/snapshot.rs
    status: pending
  - id: phase3-two-way-sync
    content: "Replace encoding in two-way sync: change_processor.rs renames and web/api.rs added instances, with dedup checks"
    status: pending
  - id: phase4-remove-config
    content: Remove decode_windows_invalid_chars from InstanceContext, encode_windows_invalid_chars from SyncbackRules, serve_session propagation, JSON schema
    status: pending
  - id: phase5-delete-module
    content: Delete src/path_encoding.rs and remove pub mod declaration from lib.rs
    status: pending
  - id: phase6-hardcore-tests
    content: "Hardcore test suite: unit tests for slugify/dedup, integration tests for build/syncback/two-way-sync roundtrips, collision tests, edge case tests; update existing encoded pattern tests; regenerate snapshots"
    status: pending
  - id: phase7-update-docs
    content: Update cursor rules (.cursor/rules/rojo.mdc, src.mdc) to reflect new approach
    status: pending
isProject: false
---

# Nuke path_encoding.rs and Migrate to PR #1187's `name` Property

## Approach Summary

**Current system (path_encoding.rs):** Encodes special chars as `%NAME%` patterns in filenames (`/` -> `%SLASH%`, `.` -> `%DOT%`, etc.), decodes them back during snapshot. Round-trips perfectly but creates ugly filenames like `What%QUESTION%Module.server.luau`.

**New system (slugify + dedup + metadata name):** Two layers working together:

1. `**slugify_name()**` -- pure, stateless. Replaces forbidden chars with `_`. Identical inputs produce identical outputs.
2. `**deduplicate_name()**` -- stateful, called at write sites. Takes a slug and a set of already-claimed names, appends `~1`, `~2`, etc. on collision. The filesystem name is just an opaque identifier.
3. **Metadata `name` field** -- the authoritative instance name, stored in `.meta.json` / `.model.json`. Always written when the slug differs from the real name (including when `~N` suffix is appended).

```
Instance "Hey_Bro"  -> Hey_Bro.luau         (no meta needed, name == slug)
Instance "Hey/Bro"  -> Hey_Bro~1.luau       (meta: {"name": "Hey/Bro"})
Instance "Hey:Bro"  -> Hey_Bro~2.luau       (meta: {"name": "Hey:Bro"})

Future duplicate trees:
Instance "Folder"   -> Folder/              (first claim, no meta needed)
Instance "Folder"   -> Folder~1/            (meta: {"name": "Folder"})
```

**Why `~`?** Valid on all OS, rarely in instance names, visually distinct, established convention (Windows short names, git ancestry), sorts next to base name.

**Forward sync (reading):** `~N` is just part of the filename stem. The real name comes from metadata `name` field. If no metadata, the full stem (including `~N`) IS the instance name -- backwards-compatible for files without metadata.

**Breaking change:** Any existing files with `%SLASH%`, `%DOT%`, `%QUESTION%` etc. in their filenames will no longer be decoded. This is intentional and clean.

## Merge Strategy

**Manual port, NOT git merge.** Our fork has diverged too far from upstream for a classical merge of PR #1187. Instead, we read their diff and manually implement the same concepts into our codebase, adapting to our existing code structure. Key differences from a raw merge:

- PR #1187 doesn't have `deduplicate_name()` -- we add that on top
- PR #1187 doesn't have our two-way sync system, `change_processor.rs`, or many of our custom middleware tweaks -- we handle those ourselves
- PR #1187's test fixtures target upstream's test structure -- we write our own tests tailored to our fork
- We do NOT cherry-pick or apply their commits. We read their approach and reimplement it.

## Complete File Catalogue

### Touched Files by Category

Every file that references `path_encoding`, `encode_path_name`, `decode_path_name`, `decode_windows_invalid_chars`, or `encode_windows_invalid_chars`:

**Delete:**

- [src/path_encoding.rs](src/path_encoding.rs) -- entire module

**Core metadata (3 files):**

- [src/lib.rs](src/lib.rs) -- remove `pub mod path_encoding;`
- [src/snapshot/metadata.rs](src/snapshot/metadata.rs) -- add `specified_name`, remove `decode_windows_invalid_chars`
- [src/serve_session.rs](src/serve_session.rs) -- remove `decode_windows_invalid_chars` setup

**Forward sync: filesystem -> Roblox (4 files):**

- [src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs) -- remove `decode_path_name` usage, name comes from meta files now
- [src/snapshot_middleware/meta_file.rs](src/snapshot_middleware/meta_file.rs) -- add `name` field to both `AdjacentMetadata` and `DirectoryMetadata`, add `apply_name()` method
- [src/snapshot_middleware/json_model.rs](src/snapshot_middleware/json_model.rs) -- respect `name` field instead of ignoring it
- [src/snapshot_middleware/lua.rs](src/snapshot_middleware/lua.rs) -- replace `encode_path_name` with `slugify_name` for meta file naming

**Reverse sync: Roblox -> filesystem (5 files):**

- [src/syncback/file_names.rs](src/syncback/file_names.rs) -- add `slugify_name()` + `deduplicate_name()`, remove `encode_path_name`, remove `encode_invalid_chars` param, add `taken_names` param
- [src/syncback/mod.rs](src/syncback/mod.rs) -- export `slugify_name`, `deduplicate_name`, remove `encode_windows_invalid_chars` from `SyncbackRules`
- [src/syncback/snapshot.rs](src/syncback/snapshot.rs) -- remove `encode_windows_invalid_chars()` method, track claimed names per directory, pass `taken_names` to `name_for_inst`
- [src/snapshot_middleware/txt.rs](src/snapshot_middleware/txt.rs) -- replace `encode_path_name` with `slugify_name`
- [src/snapshot_middleware/csv.rs](src/snapshot_middleware/csv.rs) -- replace `encode_path_name` with `slugify_name`

**Two-way sync (2 files):**

- [src/change_processor.rs](src/change_processor.rs) -- replace `encode_path_name` with `slugify_name` + dedup against directory contents + meta file handling
- [src/web/api.rs](src/web/api.rs) -- replace `encode_path_name` with `slugify_name` + dedup against directory contents + meta file handling

**Tests (~28 encoded patterns in two_way_sync):**

- [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) -- update all `%QUESTION%`, `%COLON%`, `%DOT%` patterns to slugified names + meta files
- [tests/tests/build.rs](tests/tests/build.rs) and [tests/tests/syncback.rs](tests/tests/syncback.rs) -- add new tests

**Config/Schema (1 file):**

- [vscode-rojo/schemas/project.template.schema.json](vscode-rojo/schemas/project.template.schema.json) -- remove `encodeWindowsInvalidChars`

**Snapshot test files (~20+ .snap files):**

- All `.snap` files under `src/snapshot/tests/snapshots/` contain `decode_windows_invalid_chars: true` -- will need regeneration

**Cursor rules (2 files):**

- [.cursor/rules/rojo.mdc](.cursor/rules/rojo.mdc) -- update `InstanceContext` docs
- [.cursor/rules/src.mdc](.cursor/rules/src.mdc) -- update middleware docs

---

## Key New Functions

### `slugify_name()` in `src/syncback/file_names.rs`

Pure, stateless. Ported from PR #1187 with no changes needed:

```rust
pub fn slugify_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for ch in name.chars() {
        if FORBIDDEN_CHARS.contains(&ch) {
            result.push('_');
        } else {
            result.push(ch);
        }
    }
    // Handle Windows reserved names, trailing dot/space, empty result
    // ... (same as PR #1187)
    result
}
```

### `deduplicate_name()` in `src/syncback/file_names.rs`

Stateful collision resolver. Takes a base name (slug or natural) and a set of already-claimed names in that directory:

```rust
/// Appends ~1, ~2, etc. to avoid collisions. Returns the name as-is if unclaimed.
/// `taken_names` contains names already used in this directory (both from existing
/// files on disk and from siblings processed earlier in the current batch).
pub fn deduplicate_name(base: &str, taken_names: &HashSet<String>) -> String {
    if !taken_names.contains(base) {
        return base.to_string();
    }
    for i in 1.. {
        let candidate = format!("{base}~{i}");
        if !taken_names.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}
```

### Updated `name_for_inst()` signature

```rust
pub fn name_for_inst<'a>(
    middleware: Middleware,
    new_inst: &'a Instance,
    old_inst: Option<InstanceWithMeta<'a>>,
    taken_names: &HashSet<String>,  // replaces encode_invalid_chars: bool
) -> anyhow::Result<(Cow<'a, str>, bool)>
//                    ^filename      ^needs_meta_name (true if slug != real name)
```

The return type gains a `bool` indicating whether metadata needs a `name` field (because the filesystem name differs from the instance name). Callers use this to know when to emit/update meta files.

---

## Implementation Phases

### Phase 1: Add the new infrastructure (manual port, additive only)

Read PR #1187's diff and manually implement the same concepts into our fork. Add dedup on top. No git merge/cherry-pick. Nothing is removed yet -- only new code added alongside existing code:

- `**src/snapshot/metadata.rs**`: Add `specified_name: Option<String>` to `InstanceMetadata` with builder method
- `**src/syncback/file_names.rs**`: Add `slugify_name()` (from PR #1187) and `deduplicate_name()` (new)
- `**src/syncback/mod.rs**`: Export `slugify_name`, `deduplicate_name`
- `**src/snapshot_middleware/meta_file.rs**`: Add `name: Option<String>` to both `AdjacentMetadata` and `DirectoryMetadata`, add `apply_name()` methods, update `is_empty()`, update syncback to emit `name` when slug differs from real name
- `**src/snapshot_middleware/json_model.rs**`: Respect `name` field from JSON (stop ignoring it), pass `specified_name` through metadata

### Phase 2: Replace encode/decode with slugify + dedup

Swap every callsite:

- `**src/snapshot_middleware/mod.rs**`: Remove `decode_path_name` import and conditionals -- filenames used as-is, real names from meta `name` field
- `**src/snapshot_middleware/lua.rs**`: Replace `encode_path_name` with `slugify_name` for meta file naming
- `**src/snapshot_middleware/txt.rs**`: Same
- `**src/snapshot_middleware/csv.rs**`: Same
- `**src/syncback/file_names.rs**`: Remove `encode_path_name` import, replace `encode_invalid_chars` param with `taken_names: &HashSet<String>`, use `slugify_name` + `deduplicate_name` + `validate_file_name`
- `**src/syncback/snapshot.rs**`: Remove `encode_windows_invalid_chars()` method, accumulate `taken_names` per directory, pass to `name_for_inst()`

### Phase 3: Replace encoding in two-way sync (with dedup)

- `**src/change_processor.rs**`: Replace `encode_path_name` renames with `slugify_name` + dedup against directory listing + meta file creation/update
- `**src/web/api.rs**`: Replace `encode_path_name` in `syncback_added_instance` with `slugify_name` + dedup against directory listing + meta file handling

### Phase 4: Remove config plumbing

- `**src/snapshot/metadata.rs**`: Remove `decode_windows_invalid_chars` field, setter, default fn from `InstanceContext`
- `**src/serve_session.rs**`: Remove `decode_windows_invalid_chars` propagation
- `**src/syncback/mod.rs**`: Remove `encode_windows_invalid_chars` field and getter from `SyncbackRules`
- `**vscode-rojo/schemas/project.template.schema.json**`: Remove `encodeWindowsInvalidChars`

### Phase 5: Delete path_encoding.rs

- `**src/path_encoding.rs**`: Delete file
- `**src/lib.rs**`: Remove `pub mod path_encoding;`

### Phase 6: Hardcore test suite

Tests are the safety net for this entire migration. Every angle must be covered.

**Unit tests in `src/syncback/file_names.rs`:**

- `slugify_name()`: all 9 forbidden chars individually, combinations, Windows reserved names (CON, PRN, etc.), trailing dot/space, empty string, all-forbidden-chars string, string that's already clean, string with `~` in it (edge case)
- `deduplicate_name()`: no collision (returns base), single collision (`~1`), multiple collisions (`~1`, `~2`, `~3`), collision where `~1` is also taken (skips to `~2`), natural name collision with slug (`Hey_Bro` vs slugified `Hey/Bro`), empty taken set
- `validate_file_name()`: existing tests still pass, slugified names always pass validation
- `name_for_inst()`: old inst preserves path, new inst with clean name, new inst with forbidden chars (slugifies), new inst with collision (deduplicates), directory middleware vs file middleware

**Integration tests -- build roundtrips (`tests/tests/build.rs` + `rojo-test/build-tests/`):**

- `slugified_name_roundtrip`: instance with `/` in name via `init.meta.json` `name` field builds correctly
- `model_json_name_input`: `.model.json` with `name` field overrides filename-derived name
- `dedup_collision_build`: two siblings where one natural name collides with another's slug -- both build with correct names

**Integration tests -- syncback roundtrips (`tests/tests/syncback.rs` + `rojo-test/syncback-tests/`):**

- `slugified_name`: instance with forbidden chars writes slugified filename + meta with `name`
- `model_json_name`: `.model.json` preserves `name` field through syncback
- `dedup_collision_syncback`: two instances whose slugs collide -- second gets `~1` suffix, both get correct meta `name` fields
- `natural_vs_slug_collision`: instance `Hey_Bro` and instance `Hey/Bro` in same directory -- no overwrite, `~1` suffix on the slug

**Two-way sync tests (`tests/tests/two_way_sync.rs`):**

- Convert all 28 existing `%QUESTION%`, `%COLON%`, `%DOT%` encoded pattern references to use slugified names + meta file assertions
- Add rename test: instance renamed to name with forbidden chars -> file renamed to slug, meta file created/updated
- Add add test: new instance with forbidden chars added via two-way sync -> slug + meta written, no collision with existing files
- Add collision test: add instance whose slug collides with existing file -> `~1` suffix applied

**Snapshot regeneration:**

- `cargo insta review` after all changes to accept updated `.snap` files
- All `decode_windows_invalid_chars: true` entries disappear from snapshots after Phase 4

### Phase 7: Update documentation

- Update `.cursor/rules/rojo.mdc` and `.cursor/rules/src.mdc` to reflect slugify + dedup + name property approach

---

## FUTURE: Phase 2 Plan Preview -- Duplicate Name Resolution

**Goal:** Remove all code that skips/prevents syncing of duplicate-named instances. Replace with an N-to-M property-based matcher in the plugin that resolves ambiguous instances by best-fit matching.

**Prerequisite:** This plan (path_encoding nuke + `~N` dedup) must be complete first. The `~N` suffix + metadata `name` field provides the filesystem foundation for hosting duplicate-named instances.

### Duplicate Prevention Code to Remove (14+ files)

**Rust server -- skip/filter logic:**

- `src/web/api.rs` -- `compute_tree_refs_with_duplicate_siblings()`, `is_tree_path_unique_with_cache()`, `filter_duplicate_children()`, parent path uniqueness bail (~lines 463-696, 947-977, 1353-1513)
- `src/syncback/stats.rs` -- `duplicate_name_count`, `record_duplicate_name()`, `record_duplicate_names_batch()`, log messages
- `src/syncback/ref_properties.rs` -- `compute_refs_with_duplicate_siblings()`, `is_path_unique_with_cache()`, ID fallback logic (~lines 54-180)
- `src/syncback/snapshot.rs` -- `warn_duplicate_names()` config getter
- `src/syncback/mod.rs` -- `warn_duplicate_names` field in `SyncbackRules`
- `src/snapshot_middleware/dir.rs` -- duplicate detection/skip in syncback (~lines 142-252)
- `src/snapshot_middleware/project.rs` -- duplicate child error messages (~lines 589-602)

**Lua plugin -- skip/filter logic:**

- `plugin/src/Reconciler/diff.lua` -- `findDuplicateNames()`, `ambiguousIds`, `scanForDuplicates()`, all skip logic (~lines 19-466)
- `plugin/src/ChangeBatcher/encodeInstance.lua` -- `findDuplicateNames()`, `hasDuplicateSiblings()`, `isPathUnique()`, skip logic (~lines 12-300)
- `plugin/src/Reconciler/reify.lua` -- ambiguous path skip (~line 52-57)

**Tests to remove/replace:**

- `plugin/src/Reconciler/diff.spec.lua` -- duplicate-named siblings test block
- `plugin/src/ChangeBatcher/encodeInstance.stress.spec.lua` -- duplicate subtree test
- `src/web/api.rs` -- ~~20 duplicate path detection tests (~~lines 4542-4898)

**Config to remove:**

- `warn_duplicate_names` from `SyncbackRules` and project schema

### New System: N-to-M Property-Based Matcher (Plugin Side)

**Concept:** When the plugin encounters N instances in Studio with the same name and M incoming sync entries that could match them, it runs a best-fit matching algorithm:

1. **Collect candidates:** N Studio instances with the same name under the same parent
2. **Collect targets:** M sync entries (from server patch) that map to those candidates
3. **Score each (candidate, target) pair** by property similarity (ClassName, Source, properties, attributes) with weighted scoring
4. **Greedy assignment:** Pick the highest-scoring pair, assign it, remove both from pools. Repeat until one pool is exhausted.
5. **Leftovers:** Unmatched Studio instances are "extra" (potentially removed). Unmatched targets are "new" (need creation with `~N` dedup).

**Where it lives:** `plugin/src/Reconciler/matchAmbiguous.lua`, called from `diff.lua` when duplicate names are detected instead of skipping them.

**Server changes:** Remove all skip/filter/bail logic. Let duplicate-named instances flow through. The `~N` dedup from this plan handles filesystem collisions. The plugin matcher handles Studio-side resolution.