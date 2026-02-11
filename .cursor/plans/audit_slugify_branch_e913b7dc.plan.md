---
name: Audit slugify branch
overview: "Production-grade audit of the `slugify` branch. The invariant: filesystem must be a perfect 1:1 representation of the Studio place such that syncback → rebuild from filesystem produces a byte-identical place file. Every audit item is evaluated against this round-trip fidelity standard."
todos:
  - id: round-trip-invariant
    content: "Verify the core invariant: for every instance in a place, syncback writes files that forward-sync rebuilds into a bit-identical instance tree (name, class, properties, hierarchy)"
    status: pending
  - id: forward-sync
    content: "Audit forward sync: all middleware correctly resolves instance names from meta name field or filename stem, no lingering decode_path_name"
    status: pending
  - id: reverse-sync
    content: "Audit reverse sync: slugify + dedup + meta emission correct in file_names.rs, snapshot.rs, dir.rs, project.rs"
    status: pending
  - id: two-way-sync
    content: "Audit two-way sync: change_processor.rs renames and api.rs adds/removes handle slugify, dedup, meta lifecycle, event suppression"
    status: pending
  - id: dedup-consistency
    content: "Verify dedup_key consistency: case sensitivity, cross-format collisions, old-inst vs new-inst paths, all insertion sites"
    status: pending
  - id: property-preservation
    content: Verify no property data is lost or mangled through name changes -- Name property, metadata fields, ref properties, attributes all survive round-trips
    status: pending
  - id: config-cleanup
    content: Grep for any lingering path_encoding, encode/decode references, %ENCODED% patterns, removed config fields
    status: pending
  - id: test-coverage
    content: Catalog existing tests vs missing tests -- especially round-trip tests that assert filesystem→rebuild identity
    status: pending
  - id: json-model-name
    content: "Audit json_model name field: read path, write path, legacy compat, json_model_legacy_name snapshot change correctness"
    status: pending
  - id: meta-lifecycle
    content: "Audit meta file lifecycle: creation, update, deletion, merge with existing fields, orphaned meta handling"
    status: pending
  - id: legacy-cleanup
    content: "Verify legacy %ENCODING% system is fully purged: no references in code/tests/fixtures, old config fields parse without crashing"
    status: pending
  - id: write-report
    content: "Produce final report: critical issues, correctness concerns, missing test coverage, cleanup items with file paths and suggested fixes"
    status: pending
isProject: false
---

# Audit: slugify Branch -- Production-Grade Round-Trip Fidelity

## Quality Standard

This is a **production-grade synchronization system**. The filesystem must represent a **1:1 copy** of the Studio place. The core invariant:

> **Round-trip identity**: Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree** -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

Every audit item below must be evaluated against this standard. A "works most of the time" finding is not acceptable. If there is ANY code path where an instance name, property, or hierarchy relationship can be lost, mangled, or silently altered through a syncback→rebuild cycle, flag it as **critical**.

## Code Quality Standard

We are building a **maintainable** codebase. During the audit, watch for duplicated logic -- the same slugify/dedup/meta-update pattern copy-pasted across `change_processor.rs`, `api.rs`, `dir.rs`, `project.rs`, etc. If you find duplicated code that could be consolidated into a shared helper:

- **Small refactor** (extracting a helper function, consolidating 2-3 call sites): flag it AND fix it as part of this audit.
- **Major system rewrite** (restructuring data flow, changing function signatures across 10+ call sites, redesigning the snapshot pipeline): flag it clearly in a **"Deferred Refactors"** section at the bottom of the report. We will evaluate whether it's worth doing after the audit is complete.

The goal is clean, DRY code that is easy to reason about and hard to get wrong. If the same "compute meta path, check if name needs slugify, upsert/remove name field" sequence appears in 3+ places, that's a refactor candidate.

## Context

This audit covers two plans that drove the branch:

- [nuke_path_encoding_migration](.cursor/plans/nuke_path_encoding_migration_7dfffc4d.plan.md) -- removed `src/path_encoding.rs`, replaced all encode/decode callsites with `slugify_name()` + `deduplicate_name()` + metadata `name` field
- [fix_stem-level_dedup](.cursor/plans/fix_stem-level_dedup_8fa9a015.plan.md) -- fixed dedup key propagation so `taken_names` contains bare slugs (not filenames with extensions)

Run `git diff main...HEAD` for the full diff. Read both plan files in full. Then audit every area below, citing file paths, line numbers, and code snippets. Flag anything missing, incorrect, inconsistent, or undertested.

---

## 1. Round-Trip Identity Verification

This is the single most important audit area. Trace the **complete lifecycle** of an instance with a name containing forbidden chars through every path:

### 1a. Syncback → Forward Sync Round-Trip

Pick a concrete example: instance named `"Key:Script"` (ServerScript class).

- **Syncback writes**: `Key_Script.server.luau` + `Key_Script.meta.json5` with `{"name": "Key:Script"}`
- **Forward sync reads**: `Key_Script.server.luau` → detects adjacent `Key_Script.meta.json5` → reads `name` field → sets `specified_name` on metadata → final instance name becomes `"Key:Script"`
- **Verify**: Trace this exact path through the code. Is there ANY point where the name could become `"Key_Script"` instead of `"Key:Script"`? Any middleware that reads the filename stem and ignores the meta name?

### 1b. Two-Way Sync → Forward Sync Round-Trip

Instance added in Studio with name `"What?Module"` during a live `rojo serve` session:

- **API handler writes**: `What_Module.luau` + `What_Module.meta.json5` with `{"name": "What?Module"}`
- **Change processor** picks up the new files → snapshots them → applies to DOM
- **Verify**: Does the DOM instance end up with name `"What?Module"` or `"What_Module"`? If the change processor snapshots the new file before the meta file is written, what happens?

### 1c. Rename Round-Trip

Instance renamed in Studio from `"Clean"` to `"Has/Slash"`:

- Old file `Clean.server.luau` is removed
- New file `Has_Slash.server.luau` + `Has_Slash.meta.json5` with `{"name": "Has/Slash"}` is created
- **Verify**: Is there a window where the old file is deleted but the new file hasn't been written yet? Could the file watcher fire in between and produce an incorrect intermediate state?

### 1d. Collision Round-Trip

Two sibling instances: `"A/B"` (ModuleScript) and `"A_B"` (ModuleScript). Both slugify to `A_B`.

- **Syncback**: First gets `A_B.luau`, second gets `A_B~1.luau` (with meta `{"name": "A_B"}` since slug+dedup differs from real name)
- **Forward sync rebuild**: `A_B.luau` → name `"A/B"` (from meta), `A_B~1.luau` → name `"A_B"` (from meta)
- **Verify**: Is the ordering deterministic? If processed in reverse order next time, do names get swapped? Is there a stable sort or tie-breaking rule?

### 1e. Directory Instance Round-Trip

Folder named `"My:Folder"`:

- **Syncback**: Directory `My_Folder/` + `My_Folder/init.meta.json5` with `{"name": "My:Folder"}`
- **Forward sync**: Reads directory → finds `init.meta.json5` → applies name override → instance named `"My:Folder"`
- **Verify**: Does `DirectoryMetadata.apply_name()` actually work? Is `init.meta.json5` read before or after the directory name is set?

### 1f. Model JSON Round-Trip

Configuration instance named `"What?Model"`:

- **Syncback**: `What_Model.model.json5` with `{"name": "What?Model", "className": "Configuration"}`
- **Forward sync**: Reads `.model.json5` → parses `name` field → uses it as instance name
- **Verify**: Trace through [json_model.rs](src/snapshot_middleware/json_model.rs). Does the `name` field actually override? Is there a priority conflict with `properties.Name`?

---

## 2. Forward Sync Completeness (filesystem → Roblox)

Every middleware that reads files and produces `InstanceSnapshot` must correctly resolve the instance name.

- [src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs): Is `decode_path_name` fully removed? Does every middleware path derive the instance name from the metadata `name` field (when present) or the filename stem (when absent)?
- [src/snapshot_middleware/meta_file.rs](src/snapshot_middleware/meta_file.rs): Do `AdjacentMetadata` and `DirectoryMetadata` both have `name: Option<String>`? Do `apply_all` / `apply_name` correctly set `specified_name` on `InstanceMetadata`? Is `is_empty()` updated for `name`?
- [src/snapshot_middleware/json_model.rs](src/snapshot_middleware/json_model.rs): Does `name` from `.model.json5` actually override the filename-derived name? Trace parse → `InstanceSnapshot` construction → final instance name. Any path where `name` is silently ignored?
- [src/snapshot_middleware/lua.rs](src/snapshot_middleware/lua.rs), [txt.rs](src/snapshot_middleware/txt.rs), [csv.rs](src/snapshot_middleware/csv.rs): Correct instance names? Any lingering `encode_path_name`/`decode_path_name` references?
- **Edge cases**:
  - `Foo~1.server.luau` with NO meta file -- must produce instance name `Foo~1` (backward compat, tilde is NOT a dedup marker during reads)
  - `Foo~1.server.luau` WITH meta `"name": "Foo/Bar"` -- meta name must win
  - `init.meta.json5` with `"name": "SomeOverride"` inside directory `MyFolder` -- must name the directory instance `SomeOverride`

---

## 3. Reverse Sync Completeness (Roblox → filesystem / syncback)

Every path that writes instances to the filesystem must use slugify + dedup + metadata correctly, and the written files must round-trip back to identical instances.

- [src/syncback/file_names.rs](src/syncback/file_names.rs): Audit `slugify_name()` for all 9 forbidden chars (`< > : " / \ | ? *`), Windows reserved names (CON, PRN, NUL, COM1-9, LPT1-9), trailing dots/spaces, empty strings, all-forbidden-chars strings. Does `deduplicate_name()` handle `~1` already taken (skip to `~2`)? Does `name_for_inst()` correctly return `(filename, needs_meta, dedup_key)` for both old-inst and new-inst paths?
- [src/syncback/snapshot.rs](src/syncback/snapshot.rs): Do `with_joined_path` and `with_base_path` propagate `dedup_key`? Are callers in dir.rs/project.rs using `dedup_key` (not `filename`) when inserting into `taken_names`?
- [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs): Is `taken_names` seeded correctly (tree children, not disk)? Is `dedup_key.to_lowercase()` inserted after each child? Any ordering issue where a child is processed before its collision partner is in the set?
- [src/snapshot_middleware/project.rs](src/snapshot_middleware/project.rs): Same checks. Verify `or_insert_with` closure seeds from tree children.
- **Meta file emission**: When `needs_meta` is true, is `name` actually written to `.meta.json5` or `.model.json5`? Trace from `name_for_inst` returning `needs_meta=true` → meta file on disk. Is there ANY code path where `needs_meta` is true but no meta file gets written? This would silently lose the real instance name on rebuild.
- **Determinism**: Given the same instance tree, does syncback ALWAYS produce the same filesystem output? Is child ordering stable? Is dedup suffix assignment deterministic? Non-determinism means different builds from the same source, which violates the invariant.
- **Edge cases**:
  - Instance name `Hey_Bro` (no forbidden chars, slug equals another instance's slug) -- no meta if first claim, `~1` + meta if second
  - Instance name exactly a Windows reserved name (e.g. `CON`) -- what does `slugify_name` produce? Can the result be read back?

---

## 4. Two-Way Sync Completeness (live serve sessions)

Highest-risk area. Every change made in Studio must produce filesystem state that, if the serve session were killed and the place rebuilt from scratch, would produce an identical tree.

- [src/change_processor.rs](src/change_processor.rs) -- instance **renamed** in Studio:
  - Slugifies new name?
  - Dedups against sibling files in same directory?
  - Creates/updates/removes meta `name` field as needed?
  - Renaming FROM slugified TO clean: removes stale `name` from meta? (check `remove_meta_name_field`)
  - Renaming FROM clean TO slugified: creates meta with `name`? (check `upsert_meta_name_field`)
  - Suppresses filesystem events correctly to avoid feedback loops?
  - **After the rename completes, if you killed the server and ran `rojo build`, would the instance have the correct name?**
- [src/web/api.rs](src/web/api.rs) -- instance **added**:
  - `syncback_added_instance` uses slugify + dedup correctly?
  - `taken_names` seeded from tree siblings (not disk)?
  - Writes meta file when slug differs from real name?
  - Meta file path matches script file path (e.g. `Foo_Bar.server.luau` pairs with `Foo_Bar.meta.json5`, not some other path)?
  - **After the add completes, does `rojo build` include this instance with the correct name, class, and source?**
- [src/web/api.rs](src/web/api.rs) -- instance **removed**:
  - Cleans up both script file AND associated meta file?
  - Handles missing meta file gracefully (instance had clean name)?
  - **After the remove, does `rojo build` correctly omit this instance?**
- **Atomicity**: When a rename involves deleting old files and creating new files, is there a window where the filesystem is in an inconsistent state that the file watcher could pick up? Could this cause a "ghost" instance or a lost instance in the DOM?
- **Race conditions**: When multiple renames happen in quick succession, is `taken_names` rebuilt each time or cached? Could stale data cause incorrect dedup?

---

## 5. Dedup Key Consistency

The dedup system must be watertight. Any inconsistency means two instances can silently overwrite each other's files.

- `name_for_inst` old-inst path: derives `dedup_key` by stripping middleware extension from existing filename? What if Dir middleware (no extension)? What if extension doesn't match `extension_for_middleware` (manually renamed file)?
- `name_for_inst` new-inst path: `dedup_key` is always the slug BEFORE extension is appended?
- **Case sensitivity**: Is `.to_lowercase()` applied consistently at ALL insertion sites in dir.rs, project.rs, api.rs, change_processor.rs? A single site that forgets lowercase would cause silent overwrites on case-insensitive filesystems (Windows/macOS).
- **Cross-format collision**: Instance `Helper` as ModuleScript (`Helper.luau`) and instance `Helper` as Folder (`Helper/`) -- both dedup keys should be `helper` (lowercase). Do they collide correctly, producing `Helper/` and `Helper~1.luau` (or vice versa)?
- **Stable ordering**: When two instances collide, which one gets the base name and which gets `~1`? Is this deterministic across runs? If not, consecutive syncbacks could swap filenames, causing unnecessary git churn.

---

## 6. Property and Data Preservation

The name system change must not affect ANY other instance data.

- **Name property vs instance name**: Roblox instances have both `Instance.Name` (the tree name) and a `Name` string property. Are these conflated anywhere? Does the new `name` field in meta/model files interact with the `Name` property in `properties`?
- **Ref properties**: Do ref properties (ObjectValue.Value, etc.) still resolve correctly after the naming change? Refs use instance IDs, not names, but verify no code path accidentally uses filenames for ref resolution.
- **Attributes**: Are instance attributes preserved through syncback? The naming change shouldn't affect them, but verify no metadata merge operation accidentally clobbers attributes stored in meta files.
- **Source scripts**: Does script source content survive the rename cycle? When a file is renamed (old deleted, new created), is the content preserved byte-for-byte?

---

## 7. Config/Schema Cleanup

- `decode_windows_invalid_chars` fully removed from `InstanceContext`, `InstanceMetadata`, all `.snap` files, `serve_session.rs`?
- `encode_windows_invalid_chars` fully removed from `SyncbackRules`, project schema, JSON schema?
- Grep for ANY lingering references to: `path_encoding`, `encode_path_name`, `decode_path_name`, `%SLASH%`, `%COLON%`, `%QUESTION%`, `%DOT%`, `%STAR%`, `%PIPE%`, `%QUOTE%`, `%LESSTHAN%`, `%GREATERTHAN%`
- Do old project files with these removed fields parse without crashing? (serde should ignore unknown fields, but verify)

---

## 8. Test Coverage Gaps

For each category, list what IS tested and what is MISSING. **Prioritize round-trip tests** -- these are the ultimate proof of correctness.

- **Round-trip tests**: Is there a test that does syncback → rebuild → compare instance trees? This is the gold standard. If it doesn't exist, flag as critical missing coverage.
- **Unit tests** ([src/syncback/file_names.rs](src/syncback/file_names.rs)): `slugify_name` edge cases, `deduplicate_name` edge cases, `name_for_inst` all paths, `dedup_key` correctness
- **Build integration** ([tests/tests/build.rs](tests/tests/build.rs)): `.model.json5` with `name` field? `init.meta.json5` with `name`? `~N` suffixed files read back correctly?
- **Syncback integration** ([tests/tests/syncback.rs](tests/tests/syncback.rs)): Forbidden-char instances produce slugified filenames + meta? Collision test (two instances whose slugs match)?
- **Two-way sync** ([tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)): All 28 old encoded-pattern references updated? Rename test (clean → slugified, slugified → clean)? Add test (new instance with forbidden chars)? Collision test during live sync?
- **Snapshot tests**: Any pending `cargo insta review` changes? All `.snap` files reflect new system?
- **Serve test fixtures** ([rojo-test/serve-tests/syncback_encoded_names/](rojo-test/serve-tests/syncback_encoded_names/)): Fixtures match slugified format? Meta files with `name` fields present?
- **Negative test**: File `Foo~1.luau` WITHOUT meta produces instance name `Foo~1` (tilde NOT parsed as dedup marker during forward sync)?
- **Idempotency test**: Run syncback twice on the same tree -- does the second run produce zero file changes? If not, something is unstable.
- **Large tree stress test**: 100+ instances with various collision patterns -- does dedup produce correct, deterministic results?

---

## 9. JSON Model `name` Field

- **Read path** ([json_model.rs](src/snapshot_middleware/json_model.rs)): `.model.json5` with `"name": "Real Name"` uses `Real Name`? `specified_name` set on metadata?
- **Write path** (syncback): `name` field emitted when slug differs? Omitted when slug matches?
- **Legacy compat**: Old `.model.json` with `"Name": "Something"` in `properties` still works? Any conflict between new top-level `name` and old `properties.Name`? What wins if both are present?
- `**json_model_legacy_name` build test**: Snapshot changed from `Expected Name` to `Overridden Name`. Is this correct? What was the test fixture's intent? Read the fixture files to understand the expected behavior.

---

## 10. Meta File Lifecycle

Meta files are the bridge between filesystem names and real instance names. Any bug here silently corrupts names.

- **Creation**: Only when `needs_meta` is true? If meta already exists with other fields (e.g. `ignoreUnknownInstances`), is the `name` field MERGED in (not replacing the whole file)? Verify `upsert_meta_name_field` in change_processor.rs does a proper merge.
- **Update**: When instance renamed and slug changes, is meta updated? When slug stays same but real name changes (e.g. `A/B` → `A:B`, both slugify to `A_B`) -- the meta `name` field must change from `"A/B"` to `"A:B"`. Is this handled?
- **Deletion**: Renaming from slugified to clean removes `name` from meta? If meta has no other fields after removal, is the file itself deleted? Verify `remove_meta_name_field` handles both cases.
- **Orphaned meta files**: Script deleted but meta remains -- does forward sync handle this gracefully (ignore the orphan) or does it create a phantom instance?
- **Model JSON meta vs adjacent meta**: For `.model.json5` files, the `name` is stored IN the model file (not in a separate `.meta.json5`). Is this consistent? Could there be a case where both a `.model.json5` `name` field and a `.meta.json5` `name` field exist for the same instance? What wins?

---

## 11. Legacy `%ENCODING%` System -- Intentionally Unsupported

The old `%NAME%` path encoding system (`%SLASH%`, `%COLON%`, `%QUESTION%`, etc.) is **intentionally removed with no backward compatibility**. This is a full refactor, not an incremental change. The old system was inferior -- it produced ugly filenames, required special decoding on read, and was fragile. The new slugify + metadata `name` approach is strictly better.

**Do NOT flag the removal of `%ENCODING%` support as a regression.** It is by design. However, DO verify:

- `src/path_encoding.rs` is fully deleted and no references remain anywhere in the codebase
- No `%SLASH%`, `%COLON%`, `%QUESTION%`, `%DOT%`, `%STAR%`, `%PIPE%`, `%QUOTE%`, `%LESSTHAN%`, `%GREATERTHAN%` patterns exist in source code, test fixtures, or snapshots
- Projects with the old `encodeWindowsInvalidChars`/`decodeWindowsInvalidChars` fields in `.project.json` parse without crashing (serde should ignore unknown fields -- verify this)
- The `syncback_encoded_names` test fixture has been fully migrated from `%ENCODED%` filenames to slugified filenames + meta files, with no old-format files remaining

---

## 12. Known Issues (Already Identified)

These have already been found. Include them in the report as confirmed, and check whether the suggested fixes are correct and complete.

### 12a. CRITICAL: Meta `name` field update skipped when slug doesn't change

**Files**: [src/change_processor.rs:834-879](src/change_processor.rs), [src/change_processor.rs:734-789](src/change_processor.rs)

The meta file `name` field update logic (`upsert_meta_name_field` / `remove_meta_name_field`) is nested inside the `if new_path != *path` condition. When renaming an instance where both old and new names slugify to the same filename (e.g. `"Foo/Bar"` to `"Foo|Bar"` -- both become `"Foo_Bar"`), the path doesn't change, so the entire block is skipped -- including the meta file updates.

**Impact**: The instance rename is silently lost. The meta file retains the old `name` value, so subsequent forward syncs restore the old name instead of the new one. This is a direct round-trip fidelity violation.

**Affected paths**: Both regular file renames (line ~835) and init file/directory renames (line ~735).

**Suggested fix**: Move the meta `name` field update logic OUTSIDE the `if new_path != *path` block. The meta name must be updated whenever the instance name changes, regardless of whether the filesystem path changes. The condition for updating the meta file is `old_name != new_name`, not `old_path != new_path`.

---

## 13. Deliverables

Produce a structured report with:

- **Critical issues** -- bugs that would cause data loss, incorrect instance names, file overwrites, or round-trip identity failures
- **Correctness concerns** -- logic that looks wrong or has edge cases that might not manifest immediately but could cause silent data corruption
- **Missing test coverage** -- specific test cases that should exist but don't, prioritized by risk (round-trip tests > unit tests)
- **Determinism concerns** -- any non-deterministic behavior that could cause different builds from the same source
- **Cleanup items** -- leftover references, dead code, stale comments
- **Deferred refactors** -- major structural improvements that would reduce duplication or improve maintainability but are too large to do inline. Describe the problem, the affected files, and the rough scope. These will be evaluated separately after the audit.
- For each issue: file path, line numbers, description, and suggested fix

