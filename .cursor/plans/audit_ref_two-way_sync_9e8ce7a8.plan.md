---
name: Audit Ref Two-Way Sync
overview: "Production-grade audit of the two-way sync Ref property feature. The invariant: for every Ref property change (set, change target, set to nil), the filesystem must represent the Ref as a Rojo_Ref_* path-based attribute such that building an rbxl from the filesystem and forward-syncing it back produces a bit-identical Ref target. Every audit item is evaluated against this round-trip fidelity standard."
todos:
  - id: round-trip-identity
    content: "Verify round-trip identity for all Ref lifecycle paths: set, nil, change target, ObjectValue.Value, script ref, stale path after rename"
    status: completed
  - id: plugin-encoding
    content: "Audit plugin-side encoding: propertyFilter, encodePatchUpdate, createPatchSet, encodeInstance, ChangeBatcher for Ref correctness"
    status: completed
  - id: server-resolution
    content: "Audit server-side path resolution: syncback_updated_properties, filter_properties_for_meta, merge_or_build_meta, handle_api_write nil Ref conversion"
    status: completed
  - id: forward-sync
    content: "Audit forward-sync: compute_ref_properties, patch_apply defer/finalize, decodeValue.lua for Rojo_Ref_* attribute resolution"
    status: completed
  - id: existing-ref-systems
    content: "Audit interaction with legacy Rojo_Target_*/Rojo_Id system: coexistence, priority, DRY concerns"
    status: completed
  - id: meta-lifecycle
    content: "Audit meta file lifecycle with Refs: creation, merge, update, deletion, orphaned attributes"
    status: completed
  - id: path-correctness
    content: "Audit path computation: full_path_of/get_instance_by_path inverse consistency, instance names containing /, deep nesting, root target"
    status: completed
  - id: encode-instance-leak
    content: "Audit encodeInstance.lua Ref leak: Ref no longer in UNENCODABLE_DATA_TYPES means Refs reach encodeProperty and fail via pcall instead of explicit filter"
    status: completed
  - id: vfs-echo
    content: Verify suppress_path() is called on all meta/model file writes with Rojo_Ref_* attributes (all 4 format branches)
    status: completed
  - id: syncback-parity
    content: "Verify two-way sync Ref output matches CLI syncback output: same attribute format, same path format, same nil handling, shared helpers"
    status: completed
  - id: msgpack-wire
    content: "Verify Variant::Ref msgpack round-trip: plugin { Ref = hexid } deserializes to correct Ref, Ref::none() round-trips as Ref(None)"
    status: completed
  - id: edge-cases
    content: "Audit edge cases: ProjectNode targets, nested projects, concurrent changes, non-existent property names, multiple instances"
    status: completed
  - id: known-limitations
    content: "Verify known limitations are documented and handled gracefully: ambiguous paths, stale paths, no immediate ID for new instances"
    status: completed
  - id: prove-stale-path
    content: "Write ignored regression tests proving stale-path-after-rename bug: set Ref, rename target, assert stored path updated (should fail)"
    status: completed
  - id: prove-untracked-ref
    content: "Write ignored regression tests proving untracked-instance-ref-dropped bug: add instance + set Ref to it in same batch, assert Ref persisted (should fail)"
    status: completed
  - id: prove-missing-instancemap
    content: "Write ignored Lua spec test proving ServeSession.lua:738 missing instanceMap arg: call encodePatchUpdate without 4th arg, assert Ref encoded (should fail)"
    status: completed
  - id: test-coverage
    content: Review test coverage against audit areas, flag gaps with specific test descriptions
    status: completed
  - id: write-report
    content: "Produce final audit report: critical issues, correctness concerns, missing coverage, known limitations, code quality items"
    status: completed
isProject: false
---

# Audit: Two-Way Sync Ref Properties -- Round-Trip Fidelity

## Quality Standard

> **Round-trip identity**: Two-way sync writes a `Rojo_Ref_`* attribute to a meta/model file. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical Ref target** -- same property name, same target instance. Any deviation is a bug.

Every audit item below must be evaluated against this standard. If there is ANY code path where a Ref property can be lost, point to the wrong target, or silently become nil through a two-way-sync + rebuild cycle, flag it as **critical**.

## Code Quality Standard

Watch for duplicated logic between [ref_properties.rs](src/syncback/ref_properties.rs) (CLI syncback bulk mode) and [api.rs](src/web/api.rs) (two-way sync per-instance mode). Both compute paths and format `Rojo_Ref_`* attributes. The shared helpers in [rojo_ref.rs](src/rojo_ref.rs) should be the single source of truth.

---

## 1. Round-Trip Identity Verification

Trace the **complete lifecycle** of a Ref property through every path.

### 1a. Set PrimaryPart Round-Trip

Concrete example: `Model.PrimaryPart = Part1` (Part1 is at `Workspace/TestModel/Part1`).

- **Plugin encodes**: reads `instance.PrimaryPart` -> Instance -> `instanceMap.fromInstances[Part1]` -> server Ref ID -> `{ Ref = "hexid" }`
- **Server receives**: `Variant::Ref(part1_ref)` in `changed_properties`
- **Server writes**: `syncback_updated_properties` extracts Ref, computes path via `full_path_of`, writes `Rojo_Ref_PrimaryPart = "Workspace/TestModel/Part1"` to `init.meta.json5`
- **Server applies**: PatchSet includes `PrimaryPart = Variant::Ref(part1_ref)` to in-memory tree
- **Forward sync reads**: `compute_ref_properties` finds `Rojo_Ref_PrimaryPart` attribute, calls `get_instance_by_path("Workspace/TestModel/Part1")`, resolves to `Variant::Ref`
- **Verify**: Is the resolved Ref the same target instance? Does `full_path_of` produce a path that `get_instance_by_path` can resolve back? Any off-by-one in root handling?

### 1b. Nil PrimaryPart Round-Trip

`Model.PrimaryPart = nil`:

- **Plugin encodes**: reads nil -> `{ Ref = "000...0" }`
- **Server receives**: `Variant::Ref(Ref::none())` 
- **handle_api_write converts**: nil Ref -> `None` (property removal) in PatchSet
- **Server writes**: `remove_attributes` includes `"Rojo_Ref_PrimaryPart"`
- **Server applies**: property removed from in-memory tree
- **Forward sync reads**: no `Rojo_Ref_PrimaryPart` attribute -> no PrimaryPart property
- **Verify**: Is the nil Ref -> `None` conversion in `handle_api_write` correct? Does `merge_or_build_meta` actually remove the attribute? Does `remove_attributes` run BEFORE `new_attributes` merge (ordering matters if both lists have the same key)?

### 1c. Change Target Round-Trip

`Model.PrimaryPart = Part1` then `Model.PrimaryPart = Part2`:

- First write: `Rojo_Ref_PrimaryPart = "Workspace/TestModel/Part1"` 
- Second write: `merge_or_build_meta` overwrites attribute to `"Workspace/TestModel/Part2"`
- **Verify**: Does the merge correctly overwrite? Or does the old value persist alongside the new? Is the attribute key case-sensitive?

### 1d. ObjectValue.Value Round-Trip

`ObjectValue.Value = SomePart` where ObjectValue is defined in `.model.json5`:

- **Server writes**: attribute goes into the `.model.json5` file directly (is_model_file = true)
- **Verify**: Does `merge_or_build_meta` correctly add `Rojo_Ref_Value` to a model file's attributes? Does the snapshot middleware read attributes from `.model.json5` the same way as from `.meta.json5`?

### 1e. Ref on Script Round-Trip

Script has a custom Ref property set via two-way sync:

- **Server writes**: adjacent `ScriptName.meta.json5` gets the `Rojo_Ref_`* attribute
- **Verify**: Is the meta file path computed correctly (strip `.server`/`.client` suffix from stem)? Does the adjacent meta correctly pair with the script file on read?

### 1f. Stale Path After Rename

Set `Model.PrimaryPart = Part1`, then rename Model from `TestModel` to `RenamedModel`:

- Meta file written with `Rojo_Ref_PrimaryPart = "Workspace/TestModel/Part1"`
- ChangeProcessor renames directory to `RenamedModel`
- Path in attribute is now stale
- **Verify**: What happens on rebuild? `get_instance_by_path("Workspace/TestModel/Part1")` returns None because `TestModel` no longer exists. PrimaryPart silently becomes nil. Is this documented as a known limitation? Is a warning logged?

---

## 2. Plugin-Side Encoding Completeness

Every code path in the plugin that encodes property changes must correctly handle Ref properties.

- [propertyFilter.lua](plugin/src/ChangeBatcher/propertyFilter.lua): `Ref` removed from `UNENCODABLE_DATA_TYPES`? Only `UniqueId` remains?
- [encodePatchUpdate.lua](plugin/src/ChangeBatcher/encodePatchUpdate.lua): 
  - `descriptor.dataType == "Ref"` branch handles nil, valid, and unresolvable cases?
  - `descriptor:read(instance)` correctly reads Ref properties on all instance types (Model.PrimaryPart, ObjectValue.Value, Weld.Part0)?
  - Does the `instanceMap.fromInstances[readResult]` lookup use the Instance as key (not a string)?
  - When `instanceMap` is nil (should never happen but defensively), does the code log and skip without crashing?
- [createPatchSet.lua](plugin/src/ChangeBatcher/createPatchSet.lua): `instanceMap` passed to `encodePatchUpdate` in both normal and `syncSourceOnly` paths?
- [encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua): Still filters out Ref properties during instance addition encoding? (Refs on newly added instances are NOT encoded -- they'd have no target ID yet)
- **Edge cases**:
  - Instance's `Changed` event fires for Ref property but the target was just destroyed -> `readResult` is nil -> correctly encoded as null ref?
  - ValueBase instances (ObjectValue) -> `GetPropertyChangedSignal("Value")` fires, `descriptor:read` returns the Instance reference?

---

## 3. Server-Side Path Resolution Completeness

Every code path in [api.rs](src/web/api.rs) that handles Ref properties must correctly compute paths.

- `syncback_updated_properties`: 
  - Ref extraction happens BEFORE `filter_properties_for_meta`?
  - `crate::ref_attribute_name(key)` produces correct attribute name?
  - `crate::ref_target_path(tree.inner(), *target_ref)` produces correct path?
  - `tree.get_instance(*target_ref).is_some()` correctly checks target existence?
  - Nil Ref -> `remove_attributes` list, not `ref_attributes` map?
  - Target not in tree -> warning logged, not in either list?
  - Ref entries removed from `props` before `filter_properties_for_meta`? (verify Refs don't fall through to `variant_to_json` which returns None)
- `filter_properties_for_meta`:
  - `Variant::Ref` is no longer skipped? Only `Variant::UniqueId` is skipped?
  - If a `Variant::Ref` somehow reaches `variant_to_json`, it returns `None` (safe fallback)?
- `merge_or_build_meta`:
  - `remove_attributes` processed BEFORE `new_attributes` merge? (important: if the same key is in both, the add should win)
  - Removal works on existing files (parses JSON5, modifies object, writes back)?
  - Removal is no-op on non-existent files?
  - Removal is no-op when the attribute doesn't exist in the file?
- `handle_api_write`:
  - Nil Refs (`Ref::none()`) converted to `None` (property removal) in the PatchSet? (the fix we applied)
  - Conversion happens for ALL updated instances, not just some?
  - Non-nil Refs pass through unchanged?

---

## 4. Forward-Sync Resolution Completeness

The forward-sync direction (filesystem -> server -> plugin) must correctly resolve `Rojo_Ref_`* attributes.

- [patch_compute.rs](src/snapshot/patch_compute.rs) `compute_ref_properties`:
  - Correctly strips `Rojo_Ref`_ prefix to get property name?
  - Handles both `Variant::String` and `Variant::BinaryString` attribute values?
  - Logs warning for non-string attribute types?
  - `tree.get_instance_by_path(path)` returns `None` for invalid paths without crashing?
  - `None` return for unresolvable paths -- does this cause property removal on the tree instance? (could silently clear a Ref that was set during the session but hasn't been written to disk yet)
- [patch_apply.rs](src/snapshot/patch_apply.rs):
  - `defer_ref_properties` correctly collects `Rojo_Ref_*` attributes for later resolution?
  - `finalize_patch_application` resolves path refs via `get_instance_by_path`?
  - Resolved refs are set as `Variant::Ref` on the instance?
  - `Rojo_Ref_*` attributes are cleaned from the Attributes property after resolution? (they should NOT appear as instance attributes in Studio)
- [decodeValue.lua](plugin/src/Reconciler/decodeValue.lua):
  - `Ref` type correctly handled: resolves via `instanceMap.fromIds[value]`?
  - Null ref (`"000...0"`) returns nil?
  - Unknown ID returns error?

---

## 5. Interaction with Existing Ref Systems

The new `Rojo_Ref_*` system coexists with the legacy `Rojo_Target_*` + `Rojo_Id` system.

- [ref_properties.rs](src/syncback/ref_properties.rs): CLI syncback still uses both path-based (`Rojo_Ref_*`) and ID-based (`Rojo_Target_*` + `Rojo_Id`) systems?
  - The shared `ref_attribute_name` helper is used for path-based refs?
  - The `Rojo_Target_*` formatting still uses inline `format!` (not shared)?
  - Is this inconsistency a DRY concern?
- `link_referents`: Old attributes (`Rojo_Ref_*` and `Rojo_Target_*`) are filtered out before re-inserting? No duplicate attributes?
- `compute_ref_properties` in patch_compute.rs: handles BOTH `Rojo_Ref_*` and `Rojo_Target_*`? Both resolve correctly? Priority if both exist for the same property?
- Two-way sync: `filter_properties_for_meta` no longer skips `Variant::Ref` -- does this mean `Variant::Ref` values could reach `variant_to_json` if a Ref property change comes from a path OTHER than `syncback_updated_properties`? Are there other callers?

---

## 6. Meta File Lifecycle with Refs

Ref attributes add a new dimension to meta file management.

- **Creation**: When the first Ref is set on an instance with no meta file -> meta file created with just `attributes: { Rojo_Ref_PropertyName: "path" }`? Does `build_meta_object` handle this (called when file doesn't exist)?
- **Merge**: When Ref attribute is added to an existing meta file with `className`, `properties`, other `attributes` -> existing content preserved? `Rojo_Ref`_* merged into attributes alongside user attributes?
- **Update**: When Ref target changes -> attribute value overwritten? Old value not left alongside new?
- **Deletion**: When Ref set to nil -> attribute removed from file? If no other content remains in `attributes`, is the empty `attributes: {}` section left or cleaned up? If meta file has no remaining content at all (no className, no properties, no attributes), should the file be deleted?
- **Orphaned Rojo_Ref_***: If an instance is deleted, its meta file is deleted too (existing behavior). No orphaned `Rojo_Ref_`* attributes should remain. Verify this path.

---

## 7. Path Computation Correctness

The path computation is the heart of the system. Any bug here silently corrupts Ref targets.

- `ref_target_path` calls `dom.full_path_of(target_ref, "/")`:
  - Root instance -> empty string ""? Does `get_instance_by_path("")` correctly resolve to root?
  - Single depth -> `"Workspace"`? Resolves back?
  - Deep nesting -> `"Workspace/Model/SubFolder/Part"`? Resolves back?
  - Instance name contains `/` -> path becomes ambiguous! `full_path_of` uses `/` as separator but instance names can contain `/` (slugified in filesystem but NOT in the tree). **This could be a critical issue.** A Part named `"A/B"` at `Workspace` would produce path `"Workspace/A/B"` which resolves to `Workspace > A > B` (3 levels) instead of `Workspace > "A/B"` (2 levels).
  - Instance name contains special chars -> path may not match filesystem slug. But paths are computed from tree names, not filesystem names, so this should be OK for resolution within the same session. Cross-session, paths are stored on disk and resolved from the tree -- tree names should be consistent.

---

## 7b. Known Potential Issue: Missing instanceMap in ServeSession Pull Path

**File:** [ServeSession.lua](plugin/src/ServeSession.lua) line 738

`encodePatchUpdate` was updated to accept an `instanceMap` parameter (4th arg) for Ref encoding. `createPatchSet.lua` passes it correctly. However, the call in `ServeSession.lua` line 738 was NOT updated:

```lua
local update = encodePatchUpdate(instance, change.id, propertiesToSync)
```

This is the "pull" path in `__confirmAndApplyInitialPatch` -- when users select "pull" in the initial patch confirmation dialog to sync Studio property changes back to Atlas. The missing 4th argument means `instanceMap` is `nil` inside `encodePatchUpdate`, causing all Ref property changes (PrimaryPart, ObjectValue.Value, constraint refs, etc.) to be silently dropped with a warning instead of encoded. This is a round-trip identity violation for the initial sync pull path.

**Expected fix:** Pass `self.__instanceMap` as the 4th argument:

```lua
local update = encodePatchUpdate(instance, change.id, propertiesToSync, self.__instanceMap)
```

**Audit should verify:** Are there any OTHER callers of `encodePatchUpdate` beyond `createPatchSet.lua` and `ServeSession.lua:738` that are also missing the `instanceMap` argument?

---

## 7c. encodeInstance.lua Ref Leak

**File:** [encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua)

`encodeInstance.lua` imports `UNENCODABLE_DATA_TYPES` from `propertyFilter.lua` and uses it to filter properties before calling `encodeProperty`. We removed `Ref = true` from that filter. Now Ref properties on newly added instances pass through to `encodeProperty`, which calls `RbxDom.EncodedValue.encode(readResult, dataType)`. For Ref properties, `readResult` is a Roblox Instance, and `RbxDom.EncodedValue.encode` likely throws (Instance is not an encodable value). The `pcall` wrapper catches the error and the property is silently dropped.

**The behavior is correct** (you can't encode a Ref without the InstanceMap during instance addition), **but the code path is fragile**. It relies on `pcall` catching an error thrown by `RbxDom.EncodedValue.encode` rather than an explicit check. If `RbxDom.EncodedValue.encode` is updated to handle Refs in the future, the behavior would change silently.

**Audit should verify:** Does `encodeInstance.lua` need its own Ref-specific filter (like `encodePatchUpdate.lua` has), or is the pcall fallback acceptable?

---

## 7d. VFS Echo Prevention for Ref Meta Writes

**File:** [api.rs](src/web/api.rs) `syncback_updated_properties()`

The suppression system uses per-path counters: `suppress_path()` increments a create/write counter, `suppress_path_remove()` increments a remove counter. The change processor checks and decrements these when VFS events arrive.

**Modern pattern:**

- **Updates to existing files** (meta writes, model updates, source writes): MUST suppress to prevent feedback loops
- **New instance additions** (`syncback_added_instance`): do NOT suppress -- the VFS watcher must pick up new files to add them to the tree
- **Folder creation**: suppression varies by context (conversion vs. new creation)

**Verify for Ref changes specifically:** All four file format branches in `syncback_updated_properties` call `suppress_path()` before `fs::write()`:

1. Directory init.meta.json5 (line ~2630)
2. Adjacent script .meta.json5 (line ~2663)
3. Inline .model.json5 (line ~2690)
4. Adjacent non-script .meta.json5 (line ~2713)

These are all updates to existing instances (not additions), so suppression is correct. Verify no branch was missed and that the `suppress_path` calls use the correct path (the path being written, not the instance path).

---

## 7e. CLI Syncback Parity

**From atlas.mdc:** "Plugin-based sync must produce exactly the same filesystem output that atlas syncback would give for the same input. Byte-for-byte identical files."

Verify that the two-way sync Ref output matches CLI syncback (`ref_properties.rs`) output for equivalent scenarios:

- Same `Rojo_Ref_`* attribute name format
- Same path format (slash-separated, root excluded)
- Same JSON5 attribute placement (in `attributes` section, not `properties`)
- Same handling of nil Refs (attribute absent, not present with empty value)
- Both use the shared `ref_attribute_name()` helper from [rojo_ref.rs](src/rojo_ref.rs)

Any divergence between the two paths is a bug per atlas.mdc.

---

## 7f. Msgpack Wire Format for Refs

The plugin sends `{ Ref = "hexid" }` via msgpack to the server. The server deserializes this as `Variant::Ref(Ref)`.

**Verify:** Does the msgpack deserialization in `handle_api_write` correctly round-trip `Variant::Ref` values? Check that `Ref::from_str` parses the hex string from the plugin and produces the correct `NonZeroU128`. Check that `Ref::none()` (hex `"000...0"`) deserializes as `Ref(None)`.

---

## 8. Edge Cases and Error Handling

- **Ref to ProjectNode instance**: Can a Ref property target an instance defined in a project file? Path resolution should work since ProjectNode instances ARE in the tree. But if the target is removed from the project file, the path becomes invalid.
- **Ref to instance in nested project**: Path computed by `full_path_of` spans the entire tree. But nested projects have their own sync boundaries. Is the path still valid after a project reload?
- **Concurrent Ref changes**: Two Ref properties changed in the same batch -> both written to meta file? Does `merge_or_build_meta` handle multiple attribute additions in one call?
- **Ref property that doesn't exist in reflection**: If the plugin sends a Ref for a property name that doesn't exist in rbx_reflection -> attribute is written but never resolved on forward-sync?
- **Multiple instances with same Ref property change**: Different instances, each with PrimaryPart changes -> each gets their own meta file written correctly?

---

## 9. Known Limitations (Document, Don't Fix)

These were explicitly accepted in the implementation plan. Confirm they are documented, log appropriate warnings, and do NOT produce silent data corruption.

- **Ambiguous paths**: Duplicate-named siblings make paths non-unique. Path-based refs may resolve to wrong sibling. No crash, but round-trip violation.
- **Stale paths after rename**: Stored path becomes invalid when target is renamed/moved.
- **No immediate ID for new instances**: Pulled instances don't get IDs until VFS watcher processes files. Ref properties targeting new instances are dropped with warning.

---

## 10. Prove Known Limitations (Regression Tests Left Failing)

Write tests that **assert the correct round-trip behavior** for the known limitations. These tests SHOULD FAIL against the current implementation, proving the bugs are real. **Leave them failing -- do NOT mark them `#[ignore]` or `SKIP` or try to fix the implementation to make them pass.** They document real violations of the round-trip invariant and will serve as acceptance criteria when fixes are implemented later.

If a test unexpectedly PASSES, analyze why. If the behavior is genuinely correct, leave the test -- it's free coverage.

### 10a. Stale Path After Target Rename (Rust integration)

In `tests/tests/two_way_sync.rs`:

1. Set `Model.PrimaryPart = Part1` via `/api/write`
2. Wait for `Rojo_Ref_PrimaryPart = "Workspace/TestModel/Part1"` in meta file
3. Rename `Part1` to `RenamedPart` via `/api/write` (changed_name)
4. Wait for rename to process
5. Read meta file -- assert `Rojo_Ref_PrimaryPart` is now `"Workspace/TestModel/RenamedPart"`
6. EXPECTED: FAIL (stored path is stale)

Add variations:

- Rename the MODEL (parent) instead of the target Part
- Rename an ancestor higher in the tree

### 10b. Ref to Instance Added in Same Request (Rust integration)

In `tests/tests/two_way_sync.rs`:

1. Add a new Part via `/api/write` (added instances) with a temp GUID key
2. In the SAME write request, set `Model.PrimaryPart = newPartRef` (using the temp GUID)
3. Assert that `Rojo_Ref_PrimaryPart` appears in the meta file with the correct path to the new Part
4. EXPECTED: FAIL (target not in tree when `syncback_updated_properties` runs)

### 10c. Ref to Untracked Instance Dropped (Lua spec)

In `plugin/src/ChangeBatcher/encodePatchUpdate.spec.lua`:

1. Create a Model and a Part in Studio
2. Set `Model.PrimaryPart = Part`
3. Insert Model into InstanceMap but NOT Part
4. Call `encodePatchUpdate` with `PrimaryPart` change and the instanceMap
5. Assert that PrimaryPart IS encoded (with Part's server ID)
6. EXPECTED: FAIL (Part not in InstanceMap, property dropped with warning)

### 10d. ServeSession Pull Path Missing instanceMap (Lua spec)

In `plugin/src/ChangeBatcher/encodePatchUpdate.spec.lua`:

1. Create a Model with PrimaryPart set to a tracked Part (both in InstanceMap)
2. Call `encodePatchUpdate(model, "MODEL_ID", { PrimaryPart = true })` -- WITHOUT the 4th instanceMap arg
3. Assert that PrimaryPart IS encoded as `{ Ref = serverId }`
4. EXPECTED: FAIL (instanceMap is nil, triggers warning path, property dropped)

---

## 11. Test Coverage Review

Evaluate existing tests against the audit areas above. For each gap, flag as missing coverage with specific test description.

- **Round-trip tests**: Do integration tests verify that setting a Ref via `/api/write`, then building from filesystem, produces the same Ref?
- **Nil Ref cleanup**: Is the nil Ref -> attribute removal -> property removal chain tested end-to-end?
- **Forward-sync resolution**: Are there serve tests that verify `Rojo_Ref`_* attributes resolve to `Variant::Ref` in the tree?
- **Path edge cases**: Instance names with `/` in them? Deep nesting? Root-level Ref targets?
- **Concurrent changes**: Multiple Ref changes in one batch?
- **Error paths**: Ref to non-existent instance? Ref on ProjectNode instance?
- **CLI syncback parity**: Does the test suite verify that two-way sync and CLI syncback produce identical `Rojo_Ref_`* output for the same input?

---

## 12. Deliverables

Produce a structured report with:

- **Critical issues** -- bugs that would cause incorrect Ref targets, lost Refs, or silent data corruption on round-trip
- **Correctness concerns** -- logic that might not manifest immediately but could cause problems (e.g., the `/` in instance names issue in section 7)
- **Missing test coverage** -- specific test cases needed, prioritized by risk
- **Known limitations** -- confirmed as documented and handled gracefully (warning logged, no crash, no silent corruption beyond the documented scope)
- **Code quality items** -- DRY violations, dead code, stale comments
- For each issue: file path, line numbers, description, and suggested fix

