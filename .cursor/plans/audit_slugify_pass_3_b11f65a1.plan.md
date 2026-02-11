---
name: Audit slugify pass 3
overview: Holistic audit of the entire `slugify` branch diff against master (110 files, +6315/-1188 lines, 21 commits). Found 1 critical bug, 2 correctness issues, and 8 test gaps. All prior plan fixes verified in code. Legacy cleanup confirmed complete.
todos:
  - id: c1-model-rename
    content: "CRITICAL: Fix model JSON rename in change_processor.rs -- add .model to suffix list, update name inside model file instead of adjacent meta"
    status: completed
  - id: cc1-model-update-name
    content: Fix syncback_update_existing_instance (api.rs:1093) dropping name field when rewriting .model.json5 files, and write_init_meta_json (api.rs:1075) dropping name field for directories
    status: completed
  - id: cc2-dir-removal-meta
    content: Fix remove_instance_at_path directory case to clean up grandparent-level DirName.meta.json5
    status: completed
  - id: test-roundtrip-slugified
    content: "Add roundtrip test: instances with forbidden chars through syncback -> rebuild -> compare"
    status: completed
  - id: test-model-rename
    content: "Add two-way sync test: rename a .model.json5 instance, verify compound extension preserved and name updated"
    status: completed
  - id: test-idempotency
    content: "Add syncback idempotency integration test: syncback twice = zero changes"
    status: completed
  - id: test-3-collision
    content: Add 3+ instance collision test (X/Y, X:Y, X|Y all slug to X_Y)
    status: completed
  - id: test-reserved-names
    content: Add Windows reserved name integration test (CON, PRN round-trip)
    status: completed
  - id: test-dir-rename
    content: "Add two-way sync test: directory/init-file rename with forbidden chars"
    status: completed
  - id: test-case-collision
    content: Add case-insensitive collision integration test (Foo vs foo)
    status: completed
  - id: test-meta-empty-deletion
    content: Strengthen rename_slugified_to_clean test to verify meta file is deleted (not just name removed) when it becomes empty
    status: completed
isProject: false
---

# Audit: slugify Branch -- Pass 3 (Holistic)

## Scope

Full diff against `master`: 110 files, +6,315 / -1,188 lines across 21 commits. Every changed file reviewed. All 4 prior plan files cross-referenced against codebase.

## Quality Standard

> **Round-trip identity**: Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree**. Any deviation is a bug.

---

## Prior Plan Verification

All claims from prior plans verified in code -- no discrepancies:

- **fix_audit_findings**: C1 (meta gating), C2 (removal meta path), C3 (deny_unknown_fields), CC1 (determinism), CC2 (init removal), DR1 (shared meta helper), DR2 (meta path helper) -- all confirmed in code.
- **fix_audit_pass_2**: CC2 refactor (`remove_instance_at_path`), preseed fix, rename dedup, dir sort, csv instigating_source, over-suppression, comment fix -- all confirmed.
- **fix_stem-level_dedup**: `return-dedup-key`, `fix-dir`, `fix-project`, `fix-api` -- all confirmed. Note: `fix-tests` is still marked `in_progress` in the plan but tests do exist; the plan status is stale.

---

## Areas Confirmed CLEAN

Thoroughly re-audited via the full diff and found correct:

- **Forward sync** (all middleware): `decode_path_name` fully removed. All middleware correctly resolves names from metadata `name` field or filename stem. `specified_name` set on `InstanceMetadata` in every middleware. `is_empty()` updated for `name`. `instigating_source` preserved in `csv.rs`.
- **Reverse sync / syncback**: `slugify_name()`, `deduplicate_name()`, `name_for_inst()` all correct. Pre-seeding uses `relevant_paths` + `strip_middleware_extension()`. Child ordering sorted alphabetically in both `dir.rs` and `project.rs`. `dedup_key.to_lowercase()` applied at ALL insertion sites.
- **Two-way sync adds**: `syncback_added_instance` groups adds by parent, seeds `sibling_slugs` from tree children via `instigating_source` paths, uses `strip_middleware_extension`. Meta files written when slug differs. Meta path matches script file path via `adjacent_meta_path()`.
- **Two-way sync removals**: `remove_instance_at_path` handles directories, init files (parent dir removal + grandparent meta), and regular files (adjacent meta). Old dead `syncback_removed_instance` deleted.
- **Meta lifecycle**: C1 fix in place -- meta updates OUTSIDE `if new_path != *path` for both init and regular paths. Shared module (`syncback::meta`) used by `change_processor.rs`. `suppress_path_remove` exists. `RemoveNameOutcome` handled correctly with `(1, 0)` counts for `FileDeleted`.
- **Legacy cleanup**: Zero references to old encoding system in source, tests, plugin, schema, or snapshots. `src/path_encoding.rs` deleted. `pub mod path_encoding` removed from `lib.rs`. `decode_windows_invalid_chars` removed from `InstanceContext` and `serve_session.rs`.
- **Dedup key consistency**: All insertion sites use `.to_lowercase()`. Cross-format collision handled.
- **SyncbackRules**: No `deny_unknown_fields`.
- **JSON schema**: `encodeWindowsInvalidChars` removed from `vscode-rojo/schemas/`.
- **Snapshot files**: All `.snap` files updated. No stale `decode_windows_invalid_chars` or `%ENCODED%` patterns.
- `**json_model_legacy_name` snapshot change**: Instance name changed from `Expected Name` to `Overridden Name` -- correct, this is the intended behavior change (the `Name`/`name` field is now respected instead of ignored).
- **Test fixture migration**: All `%ENCODED%` filenames in `syncback_encoded_names/` correctly replaced with slugified names + meta files.
- **Existing test updates**: All ~12 two-way sync tests with encoded patterns correctly updated to slugified equivalents.

---

## CRITICAL: Model JSON rename loses `.model` compound extension

**File**: [src/change_processor.rs](src/change_processor.rs) lines 786-868

The rename handler's suffix list:

```786:789:src/change_processor.rs
let known_suffixes = [
    ".server", ".client", ".plugin", ".local",
    ".legacy",
];
```

Does NOT include `.model`. For `What_Model.model.json5` renamed to `Why?Model`:

- `stem` = `"What_Model.model"`, `extension` = `"json5"`, `script_suffix` = `""` (no match)
- `old_base` = `"What_Model.model"` (includes `.model`)
- New filename = `"Why_Model.json5"` -- `.model` LOST

**Impact**: File type changes from `.model.json5` (JSON instance) to `.json5` (data module). Instance class, properties, hierarchy silently lost. The meta update then writes to `Why_Model.meta.json5` (wrong location for model files). Direct round-trip violation.

**Fix**: Add `.model` to the suffix list. Then detect `.model.json5`/`.model.json` files and update the `name` field INSIDE the model file (like `syncback::meta::upsert_meta_name` but targeting the JSON root's `name` key) instead of writing adjacent `.meta.json5`.

---

## CORRECTNESS: `syncback_update_existing_instance` drops `name` field

**File**: [src/web/api.rs](src/web/api.rs) lines 1091-1098

When updating an existing `.model.json5` via two-way sync (plugin sends updated properties), the code rewrites the entire model file:

```1093:1093:src/web/api.rs
let content = self.serialize_instance_to_model_json(added, None)?;
```

`None` for `instance_name` means the `name` field is NOT written. If the model file previously had `"name": "Hey/Bro"`, the name is silently lost on rewrite. Same issue exists at line 1075 for directory updates via `write_init_meta_json(existing_path, added, None)`.

**Fix**: Compute `meta_name_field` by comparing the slug of the existing filesystem name against `added.name`. If they differ, pass `Some(&added.name)`. This matches the logic in `syncback_instance_to_path_with_stats`.

---

## CORRECTNESS: `remove_instance_at_path` directory case skips grandparent meta

**File**: [src/web/api.rs](src/web/api.rs) lines 1389-1396

When `path.is_dir()` (plain Folder), `remove_dir_all` is called but no adjacent `DirName.meta.json5` at the parent level is cleaned up. The init-file case (lines 1423-1438) correctly handles this.

**Severity**: LOW. Grandparent-level `DirName.meta.json5` only exists from manual creation or the change_processor rename path (which renames it). Standard syncback uses `init.meta.json5` inside the directory.

**Fix**: After `remove_dir_all`, add grandparent meta cleanup matching the init-file path.

---

## TEST COVERAGE GAPS

Prioritized by risk:

1. **Roundtrip test with slugified names** -- `syncback_roundtrip.rs` exists but no fixtures exercise slugified names. None of `meta_name_override`, `model_json_name_override`, `dedup_suffix_with_meta` are in the roundtrip test list.
2. **Model file rename two-way sync** -- No test for renaming a `.model.json5` instance. Would catch the critical bug above.
3. **Syncback idempotency integration** -- Run syncback twice, assert zero changes on second run. Unit tests exist but no full-pipeline test.
4. **3+ instance collision** -- Only 2-instance collision tested (`add_two_colliding_instances_deduplicates`). Need `X/Y`, `X:Y`, `X|Y` -> `X_Y`, `X_Y~1`, `X_Y~2`.
5. **Windows reserved name integration** -- Unit tests for `CON`/`PRN` slugification exist. No build or syncback integration test.
6. **Directory rename with forbidden chars** -- Two-way sync tests cover file renames but not init-file/directory renames.
7. **Case-insensitive collision** -- Code handles it (`.to_lowercase()` everywhere). No integration test.
8. **Meta file empty deletion** -- `rename_slugified_to_clean` test checks meta doesn't have `name` field but doesn't verify the meta file itself is deleted when it becomes empty (the `RemoveNameOutcome::FileDeleted` path).

---

## Summary


| Category      | Count | Details                                                                                                                                              |
| ------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Critical bugs | 1     | Model JSON rename loses `.model` extension                                                                                                           |
| Correctness   | 2     | `syncback_update_existing_instance` drops `name` field; directory removal skips grandparent meta                                                     |
| Missing tests | 8     | Roundtrip, model rename, idempotency, 3+ collision, reserved names, dir rename, case collision, meta empty deletion                                  |
| Prior plans   | OK    | All claims verified in code, no discrepancies                                                                                                        |
| Clean areas   | 10    | Forward sync, reverse sync, two-way adds/removals, meta lifecycle, legacy cleanup, dedup consistency, schema, snapshots, test fixtures, test updates |


