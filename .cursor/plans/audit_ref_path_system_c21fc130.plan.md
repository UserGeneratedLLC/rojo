---
name: Audit Ref Path System
overview: Audit the Luau-style ref path system (rojo_ref.rs, tree.rs, patch_apply.rs, change_processor.rs, ref_properties.rs, api.rs, meta.rs) for round-trip correctness, resolution consistency, and edge-case safety.
todos:
  - id: roundtrip
    content: Verify compute_relative_ref_path <-> resolve_ref_path_to_absolute round-trip for all prefix classes
    status: completed
  - id: dual-resolution
    content: Verify resolve_ref_path (tree.rs) and resolve_ref_path_to_absolute (rojo_ref.rs) produce identical results
    status: completed
  - id: priority
    content: Audit Rojo_Ref_ vs Rojo_Target_ priority handling in patch_apply.rs
    status: completed
  - id: cleanup
    content: Verify attribute cleanup completeness after ref resolution
    status: completed
  - id: fs-name-consistency
    content: Audit filesystem_name_for vs ref_target_path_from_tree consistency
    status: completed
  - id: index-correctness
    content: Audit RefPathIndex operations (populate, find_by_prefix, update_prefix, rename_file)
    status: completed
  - id: rename-fixup
    content: Audit update_ref_paths_after_rename path reconstruction logic
    status: completed
  - id: placeholder
    content: Audit placeholder system in CLI syncback (uniqueness, substitution order, fallback)
    status: completed
  - id: added-paths
    content: Audit added_paths same-batch ref handling in api.rs (extensions, dedup suffixes)
    status: completed
  - id: test-gaps
    content: Identify test coverage gaps and flag missing scenarios
    status: completed
isProject: false
---

# Audit: Ref Path System (Luau-Style Ref Paths)

## Scope

The ref path system converts Roblox `Ref` properties (like `Model.PrimaryPart`) into filesystem-path-based `Rojo_Ref_*` attributes using Luau require-by-string syntax (`@self`, `./`, `../`, `@game/`). This audit covers:

- Path generation: `[src/rojo_ref.rs](src/rojo_ref.rs)` (`compute_relative_ref_path`, `ref_target_path_from_tree`)
- Path resolution: `[src/rojo_ref.rs](src/rojo_ref.rs)` (`resolve_ref_path_to_absolute`), `[src/snapshot/tree.rs](src/snapshot/tree.rs)` (`resolve_ref_path`, `walk_segments`, `get_instance_by_path`, `filesystem_name_for`)
- Forward sync application: `[src/snapshot/patch_apply.rs](src/snapshot/patch_apply.rs)` (`finalize_patch_application`, `defer_ref_properties`)
- Syncback generation: `[src/syncback/ref_properties.rs](src/syncback/ref_properties.rs)` (`collect_referents`, `tentative_fs_path`)
- Two-way sync: `[src/web/api.rs](src/web/api.rs)` (`syncback_updated_properties`, `added_paths`)
- Rename updates: `[src/change_processor.rs](src/change_processor.rs)` (`update_ref_paths_after_rename`), `[src/syncback/meta.rs](src/syncback/meta.rs)` (`update_ref_paths_in_file`)
- Index: `[src/rojo_ref.rs](src/rojo_ref.rs)` (`RefPathIndex`)

## Audit Methodology

Evaluate each area against the **round-trip identity invariant**: syncback writes `Rojo_Ref_`* paths, forward sync resolves them back to the same `Ref` targets. Any code path where a ref target can be lost, silently changed, or fail to resolve is a bug.

## Audit Areas

### 1. Round-Trip: `compute_relative_ref_path` <-> `resolve_ref_path_to_absolute`

**Files:** `src/rojo_ref.rs` lines 205-289

Verify the compute/resolve pair is bijective for all prefix classes:

- `@self` / `@self/...` -- descendant paths
- `./...` -- sibling paths (1 up)
- `../...` chains -- 2+ levels up within same service
- `@game/...` -- cross-service / ancestor refs
- Bare paths (no prefix) -- legacy fallback

**Specific checks:**

- `compute_relative_ref_path` uses **case-sensitive** common-prefix detection (line 222: `a == b`), but `resolve_ref_path` / `walk_segments` uses **case-insensitive** matching (line 365: `eq_ignore_ascii_case`). This asymmetry is safe only if path generation always uses canonical filesystem case. Verify this invariant holds across all callers of `ref_target_path_from_tree`.
- When `remaining_parts.is_empty()` (target is ancestor of source), falls through to `@game/` (line 231-233). Verify `resolve_ref_path_to_absolute` handles this correctly.
- Empty `source_abs` or `target_abs` edge cases.

### 2. Dual Resolution: `resolve_ref_path` (tree.rs) vs `resolve_ref_path_to_absolute` (rojo_ref.rs)

**Files:** `src/snapshot/tree.rs` lines 300-389, `src/rojo_ref.rs` lines 254-289

Two independent implementations resolve relative paths:

- `resolve_ref_path` (tree.rs) -- walks the live `RojoTree`, used during forward sync
- `resolve_ref_path_to_absolute` (rojo_ref.rs) -- pure string manipulation, used by `RefPathIndex` and `update_ref_paths_in_file`

Verify they handle **all prefix variants identically**:

- `../` prefix: tree.rs pops to grandparent THEN walks rest. rojo_ref.rs pops source + parent from parts THEN processes rest. Both handle subsequent `..` in rest via loop.
- Edge case: `../` on a root-level instance (1 segment in `source_abs`). `resolve_ref_path_to_absolute` does `p.pop(); p.pop()?` -- the second pop returns `None`. `resolve_ref_path` checks `grandparent.is_none()`. Both return None. Consistent.
- Empty rest after prefix strip: e.g., `"@self/"` → empty rest. tree.rs: `walk_segments(source, "")` returns `Some(source)`. rojo_ref.rs: no segments to iterate, returns `source_abs`. Consistent (equivalent to `@self`).

### 3. Priority: `Rojo_Ref_`* vs `Rojo_Target_`* in Forward Sync

**File:** `src/snapshot/patch_apply.rs` lines 110-205, 321-360

In `defer_ref_properties`: both `Rojo_Ref_`* and `Rojo_Target_`* are collected into **separate** maps without deduplication. In `finalize_patch_application`: `attribute_refs_to_rewrite` (Target) runs first (line 131), then `path_refs_to_rewrite` (Ref) runs second (line 144), **overwriting** the Target result.

**Finding:** Priority is correct (`Rojo_Ref_`* wins) but implemented via **execution order**, not explicit logic. If the blocks are reordered, priority silently flips. Consider whether this warrants a code comment or explicit dedup in `defer_ref_properties`.

### 4. Attribute Cleanup Completeness

**File:** `src/snapshot/patch_apply.rs` lines 171-202

After resolution, `instances_needing_attr_cleanup` triggers removal of all `Rojo_Id`, `Rojo_Target_`*, and `Rojo_Ref_`* attributes. Verify:

- Both `defer_ref_properties` code paths (Rojo_Target_ and Rojo_Ref_) add to `instances_needing_attr_cleanup`.
- Rojo_Id is also added (line 357).
- Cleanup iterates ALL attributes and removes matches by prefix -- catches attributes even if they weren't in the rewrite maps (e.g., orphaned attributes).
- Check: does an update patch also trigger defer_ref_properties? Yes (line 310).

### 5. `filesystem_name_for` Consistency

**File:** `src/snapshot/tree.rs` lines 394-418

This function is the **ground truth** for path segment names during forward sync. Verify:

- `InstigatingSource::Path` -- returns filename from disk. Correct: includes extension, dedup suffix, slugified name.
- `InstigatingSource::ProjectNode` -- returns instance name. Correct: project nodes are services or named containers, not files.
- No `instigating_source` -- falls back to instance name. This happens for newly-added instances (two-way sync). Verify this fallback doesn't break ref path resolution for refs pointing at just-added instances.
- `ref_target_path_from_tree` (rojo_ref.rs lines 140-191) independently reimplements the same logic. Check for consistency: both use `meta.instigating_source`, both have the same ProjectNode vs Path branching.

### 6. `RefPathIndex` Correctness

**File:** `src/rojo_ref.rs` lines 339-511

- `populate_from_dir` (line 400): scans filesystem directly (bypassing VFS), reads JSON5 files, resolves relative paths to absolute. The `source_abs` comes from `tree.get_ids_at_path`. **Edge case:** orphaned meta file (no tree entry) gets `source_abs = ""`, which corrupts relative path resolution (e.g., `"./Sibling"` resolves to `"Sibling"` instead of `"Service/Parent/Sibling"`). Low severity since orphaned meta files shouldn't exist during normal serve sessions.
- `find_by_prefix` (line 464): checks exact match OR prefix-with-slash. Correct. No false positives from segment-prefix overlap (e.g., `"Workspace/Foo"` won't match `"Workspace/FooBar"`).
- `update_prefix` (line 491): merges file sets when new key already exists. Correct for convergent renames.
- `rename_file` (line 481): updates filesystem paths in all entries. O(n) scan -- acceptable since called rarely.

### 7. `update_ref_paths_after_rename` Path Fixup

**File:** `src/change_processor.rs` lines 429-540

When the indexed file path no longer exists (directory was renamed), this code tries to reconstruct the new path by replacing the old directory segment with the new one in the PathBuf components (lines 462-497).

**Checks:**

- Only replaces the **first** occurrence. If the old segment appears in multiple path components (e.g., `src/Foo/Foo/file.meta.json5`), only the first gets replaced. This is correct for single-level renames but could fail for deeply nested same-named directories. **Low severity** since dedup suffixes prevent truly identical sibling names.
- Handles both plain name and slugified name variants (lines 448-484).
- After updating files, updates the index with both `update_prefix` and `rename_file` (lines 532-540). The zip iteration assumes `original_paths` and `files_to_check` have the same length and order -- they do since `files_to_check` is a 1:1 map of the originals.

### 8. Placeholder System (CLI Syncback)

**Files:** `src/syncback/ref_properties.rs`, `src/syncback/mod.rs`, `src/syncback/fs_snapshot.rs`

Placeholders (`__ROJO_REF_<source>_TO_<target>`__) are written during the syncback walk before dedup suffixes are known, then substituted with final relative paths.

**Checks:**

- Placeholder uniqueness: encodes both source and target Refs. Unique per (source, target) pair.
- Substitution in `fix_ref_paths`: verify it handles placeholders appearing in both meta and model files.
- Length-descending sort prevents partial prefix matches (e.g., `__ROJO_REF_A_TO_B`__ won't partially match `__ROJO_REF_A_TO_BC_`_ if the longer one is processed first).
- Fallback: if final path not in `ref_path_map`, falls back to `tentative_fs_path_public()`. Verify this fallback produces a valid path.

### 9. Two-Way Sync: `added_paths` for Same-Batch Refs

**File:** `src/web/api.rs`

When the plugin adds instances and sets Ref properties targeting them in the same `/api/write` request, `added_paths` pre-computes filesystem paths. Verify:

- `added_instance_fs_segment` mirrors `tentative_fs_name` logic from ref_properties.rs.
- Paths include correct extensions (`.server.luau`, `.luau`, `.model.json5`, no extension for folders).
- Dedup suffixes are NOT included in `added_paths` (the instance hasn't been written yet), which could cause the ref path to mismatch if the actual write adds a dedup suffix.

### 10. Test Coverage Assessment

**Files:** `src/rojo_ref.rs` (tests), `tests/tests/two_way_sync.rs`, `tests/tests/syncback.rs`

Existing tests cover: basic ref write, nil ref, target change, rename update, same-batch add, dedup cleanup, RefPathIndex population, relative path round-trips (100+ cases).

**Gaps to check:**

- Multi-level ancestor renames (rename grandparent, verify ref 3 levels down updates)
- Cross-nested-project refs
- Refs where source and target are in the same dedup group
- `../` chains exceeding 2 levels
- Unicode instance names in ref paths

## Severity Scale

- **Critical** -- Round-trip identity violated; ref targets silently lost or changed
- **High** -- Refs fail to resolve in specific but reproducible scenarios  
- **Medium** -- Code fragility or missing error handling that could cause future regressions
- **Low** -- Minor edge cases, code quality, or documentation gaps

