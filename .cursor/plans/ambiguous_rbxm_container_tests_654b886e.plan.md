---
name: Ambiguous rbxm Container Tests
overview: Create a comprehensive test suite (~30+ tests) for the ambiguous rbxm container system covering syncback (clean+incremental), two-way sync transitions, forward sync, refs, and edge cases. Tests are designed to be hard to pass, verifying round-trip identity and preventing regressions.
todos:
  - id: fixture-gen
    content: Create fixture generator helper (fixture_gen.rs) for programmatically building rbxm input files with duplicate-named children
    status: completed
  - id: syncback-clean-fixtures
    content: Create 8 syncback clean-mode test fixtures (ambiguous_basic through ambiguous_deep_nesting) with project files and input.rbxm binaries
    status: completed
  - id: syncback-clean-tests
    content: Add 8 syncback clean-mode test entries to syncback_tests! macro and generate initial snapshots
    status: completed
  - id: syncback-incremental-fixtures
    content: Create 4 syncback incremental test fixtures (expansion_resolved, still_ambiguous, user_rbxm_not_expanded, expansion_script_to_init)
    status: completed
  - id: syncback-incremental-tests
    content: Add 4 syncback incremental test entries to syncback_tests_incremental! macro
    status: completed
  - id: serve-fixture
    content: Create serve-test fixture ambiguous_container/ for two-way sync tests
    status: completed
  - id: twoway-sync-tests
    content: Write 12 two-way sync test functions in two_way_sync.rs covering transitions, property changes, refs, batch operations
    status: completed
  - id: build-fixtures-tests
    content: Create 2 build-test fixtures and add to gen_build_tests! macro
    status: completed
  - id: roundtrip-test
    content: Add 1 syncback roundtrip test for ambiguous containers
    status: completed
  - id: edge-case-tests
    content: Write 5 edge case tests (windows chars, idempotent, project node parent, case insensitive, cross-boundary refs)
    status: completed
  - id: run-verify-fix
    content: Run full test suite, verify tests exercise real code paths, fix any implementation bugs found
    status: completed
isProject: false
---

# Ambiguous rbxm Container Test Suite

## Standards (from audit.md)

**Round-trip identity:** Syncback writes a directory tree. Building from that tree and forward-syncing back must produce a bit-identical instance tree. Any deviation is a bug.

**CLI Syncback Parity:** Plugin-based sync must produce the same filesystem output as `atlas syncback`. Any divergence is a bug.

## Current State

The implementation is already in place across all files (metadata, syncback fallback, forward sync, change_processor, plugin). Currently there are ~12 unit tests for infrastructure (has_duplicate_children, find_rbxm_container, is_dir_middleware, AdjacentMetadata serde) and 1 existing syncback test (`rbxm_fallback`). We need ~30+ additional integration tests.

## Test Categories and Counts

- Syncback clean mode: 8 tests
- Syncback incremental (expansion): 4 tests  
- Two-way sync: 12 tests
- Build roundtrip: 3 tests
- Edge cases: 5 tests
- **Total: 32 new tests**

---

## Part A: Syncback Clean-Mode Tests

**Fixture approach:** Create new syncback test fixtures using programmatically-generated `.rbxm` input files. Each fixture needs:

- `rojo-test/syncback-tests/{name}/input-project/default.project.json5`
- `rojo-test/syncback-tests/{name}/input-project/src/` (empty or with starter files)
- `rojo-test/syncback-tests/{name}/input.rbxm` (binary, built programmatically)

**New file:** `tests/tests/syncback_ambiguous.rs` -- a dedicated submodule to keep these organized, included from the existing test runner.

Actually, since the test macro is in [tests/tests/syncback.rs](tests/tests/syncback.rs), add the new entries to `syncback_tests!` and `syncback_tests_incremental!` macros there. The fixtures go in `rojo-test/syncback-tests/`.

**To create input .rbxm files:** Write a one-time Rust binary/script that uses `rbx_dom_weak::InstanceBuilder` + `rbx_binary::to_writer` to produce each fixture. OR, add a build-step test helper function. The simplest approach: create a `tests/create_fixtures.rs` helper that generates the binary files, then commit them.

### Tests

1. `**ambiguous_basic`** -- Two Folder children both named "Child" under a Folder. Verifies parent becomes `.rbxm`, adjacent `.meta.json5` has `ambiguousContainer: true`, both children are inside the rbxm.
  - Check files: `["src/Parent.rbxm", "src/Parent.meta.json5"]`
2. `**ambiguous_deepest_level`** -- Outer/Inner structure where only Inner has duplicate children. Verifies Inner becomes rbxm, Outer stays a directory.
  - Check: `["src/Outer/Inner.rbxm", "src/Outer/Inner.meta.json5"]`
3. `**ambiguous_multiple_groups`** -- Parent has A(x2) and B(x2). All four plus any unique siblings get captured in one rbxm.
  - Check: `["src/Parent.rbxm", "src/Parent.meta.json5"]`
4. `**ambiguous_with_unique_siblings`** -- Parent has duplicate "Dup"(x2) + unique "Solo". All three end up in the rbxm. Verify Solo is NOT a separate file.
  - Check: `["src/Parent.rbxm"]` and assert `src/Solo.luau` does NOT exist
5. `**ambiguous_script_container**` -- A ModuleScript with Source="return {}" has two children both named "Child". Entire script becomes rbxm. Verifies Source property is preserved.
  - Check: `["src/MyModule.rbxm"]`
6. `**ambiguous_slugified_name**` -- Container instance named "What?Folder" (contains `?`). rbxm should be `What_Folder.rbxm`, meta should have `name: "What?Folder"`.
  - Check: `["src/What_Folder.rbxm", "src/What_Folder.meta.json5"]`
7. `**ambiguous_dedup_collision**` -- Two sibling folders: one named "Test" (unique children, stays dir) and one named "Te/st" which slugifies to "Test" (has duplicate children, becomes rbxm). The rbxm should dedup to `Test~1.rbxm`.
  - Check: `["src/Test/init.meta.json5", "src/Test~1.rbxm", "src/Test~1.meta.json5"]`
8. `**ambiguous_deep_nesting**` -- Container with 5 levels of nesting inside. All levels preserved in rbxm.
  - Check: `["src/Deep.rbxm"]` -- snapshot the rbxm content to verify all 5 levels

---

## Part B: Syncback Incremental (Expansion) Tests

These test the expansion path: an existing rbxm container whose duplicates are resolved.

### Fixtures

Each needs an `input-project/` with an existing `.rbxm` + `.meta.json5` (from a previous syncback), plus an `input.rbxm` where the duplicates are resolved (one renamed).

1. `**ambiguous_expansion_resolved**` -- Was rbxm (had 2x "Child"), now one is renamed to "ChildB". Verifies rbxm is removed and directory is created with individual files.
  - Check: `["src/Parent/Child/...", "src/Parent/ChildB/..."]` -- assert `src/Parent.rbxm` does NOT exist
2. `**ambiguous_expansion_still_ambiguous**` -- Was rbxm (had 3x "Child"), now one is renamed but still 2x "Child" remain. Verifies rbxm stays.
  - Check: `["src/Parent.rbxm", "src/Parent.meta.json5"]`
3. `**ambiguous_user_rbxm_not_expanded**` -- A user-created `.rbxm` (no `ambiguousContainer` flag in meta) with unique children. Verifies it stays as rbxm and is NOT expanded.
  - Check: `["src/UserModel.rbxm"]`
4. `**ambiguous_expansion_script_to_init**` -- Was rbxm containing a Script with Source + children. On expansion, becomes `Script/init.server.luau` with children as siblings.
  - Check files for init script format

---

## Part C: Two-Way Sync Tests

**Fixture:** Create `rojo-test/serve-tests/ambiguous_container/` with:

- `default.project.json5` pointing to `src/`
- `src/` containing a few normal scripts/folders to work with

All tests use `run_serve_test("ambiguous_container", ...)`.

**File:** Add tests to [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs).

### Tests

1. `**twoway_add_duplicate_creates_rbxm`** -- Add instance "Dup" under RS. Add another "Dup" under RS. Verify parent (RS's `$path` dir) transitions to rbxm. Assert `.rbxm` file exists, directory files are cleaned up.
2. `**twoway_rename_creates_rbxm`** -- RS has "ScriptA" and "ScriptB". Rename "ScriptB" to "ScriptA". Verify container transitions to rbxm.
3. `**twoway_remove_resolves_rbxm`** -- After creating an rbxm container (via duplicate add), remove one of the duplicates. Verify rbxm expands back to directory with individual files.
4. `**twoway_rename_resolves_rbxm`** -- After creating rbxm, rename one duplicate to a unique name. Verify expansion.
5. `**twoway_property_change_inside_rbxm**` -- Add duplicates to create rbxm. Then send property change (Disabled=true) for one child inside the container. Verify rbxm is re-serialized (build from rbxm and check property).
6. `**twoway_source_change_inside_rbxm**` -- Create rbxm container with a Script inside. Send Source change. Verify rbxm is re-written with new Source.
7. `**twoway_add_child_inside_rbxm**` -- Create rbxm container, then add a new child to an instance inside it. Verify rbxm is re-serialized with the new child.
8. `**twoway_remove_child_from_rbxm**` -- Remove a child from inside an rbxm container. Verify rbxm is re-serialized without that child.
9. `**twoway_ref_inside_rbxm**` -- Create rbxm container with an ObjectValue inside. Set its Value ref to point to another instance (outside the container). Verify Rojo_Ref_* attribute is written into the rbxm.
10. `**twoway_batch_mixed_operations`** -- In a single write request, mix: property change on normal instance, property change on instance inside rbxm, addition of a new instance. Verify all three are handled correctly.
11. `**twoway_rapid_create_then_resolve`** -- Add duplicate (triggers rbxm), then immediately rename it away (triggers expansion), all in rapid succession. Verify no orphaned files.
12. `**twoway_rbxm_container_tree_consistency`** -- After all operations on an rbxm container, call `session.assert_tree_fresh()` and `assert_round_trip()` to verify the tree matches a fresh rebuild from disk.

---

## Part D: Build/Forward-Sync Roundtrip Tests

**Fixture:** Create `rojo-test/build-tests/ambiguous_container/` with a project that has a `.rbxm` file + `.meta.json5` with `ambiguousContainer: true`. The rbxm contains duplicate-named children.

1. `**build_ambiguous_container`** -- Build the project to rbxmx. Verify the output contains the correct hierarchy with duplicate-named children, correct properties, correct names.
  - Add to `gen_build_tests!` macro in [tests/tests/build.rs](tests/tests/build.rs)
2. `**build_rbxm_cross_refs`** -- Project has an rbxm container with internal refs (Rojo_Ref_*) and a sibling instance that references something inside the container. Build and verify all refs resolve.
  - Add to `gen_build_tests!`
3. `**syncback_roundtrip_ambiguous`** -- Full roundtrip: build project to rbxm, syncback from that rbxm, rebuild from syncback output, compare trees. This is the ultimate round-trip identity test.
  - Add as a function test in [tests/tests/syncback_roundtrip.rs](tests/tests/syncback_roundtrip.rs) using `run_roundtrip_test()`

---

## Part E: Edge Case and Regression Tests

1. `**ambiguous_windows_invalid_chars`** -- Container instance named with every forbidden Windows char (`<>:"/\|?`*). Verify slugification produces valid filename, dedup works, meta has correct name, round-trips correctly.
  - Syncback test
2. `**ambiguous_idempotent`** -- Run syncback twice on the same input with duplicates. Second run should produce zero filesystem changes. Verify via hash comparison or file modification times.
  - Syncback test (incremental mode, compare before/after)
3. `**ambiguous_project_node_parent`** -- Duplicates directly under a ProjectNode-defined service (e.g., ReplicatedStorage from project file). Verify the system does NOT try to convert the service to rbxm (it can't -- it's a ProjectNode). Should warn and handle gracefully.
  - Syncback test
4. `**ambiguous_case_insensitive**` -- Two children named "child" and "Child" (differ only in case). On Windows/macOS this is a collision. Verify they're detected as duplicates and parent becomes rbxm.
  - Syncback test
5. `**ambiguous_refs_cross_boundary**` -- Instance A inside rbxm has Ref pointing to instance B outside rbxm, AND instance C outside has Ref pointing to instance D inside rbxm. Both directions should work through Rojo_Ref_* attributes.
  - Syncback test, verify meta/model files have correct ref attributes

---

## Implementation Approach

### Phase 1: Create Fixture Generator

Create a helper in [tests/rojo_test/fixture_gen.rs](tests/rojo_test/fixture_gen.rs) (new file) that uses `rbx_dom_weak::InstanceBuilder` and `rbx_binary::to_writer` to generate `.rbxm` input files programmatically. Key functions:

```rust
pub fn write_rbxm(path: &Path, root: InstanceBuilder) { ... }
pub fn folder(name: &str) -> InstanceBuilder { ... }
pub fn script(name: &str, source: &str) -> InstanceBuilder { ... }
pub fn module_script(name: &str, source: &str) -> InstanceBuilder { ... }
```

### Phase 2: Create Syncback Fixtures

For each syncback test, create the fixture directory under `rojo-test/syncback-tests/`:

- `default.project.json5` (minimal, pointing to `src`)
- `src/` (starter files if needed for incremental tests)
- `input.rbxm` (generated by fixture helper, then committed)

### Phase 3: Create Serve-Test Fixtures

For two-way sync tests, create `rojo-test/serve-tests/ambiguous_container/`:

- `default.project.json5`
- `src/existing.luau` (starter ModuleScript)

### Phase 4: Create Build-Test Fixtures

For build tests, create `rojo-test/build-tests/ambiguous_container/`:

- `default.project.json5`
- `src/Container.rbxm` + `src/Container.meta.json5`

### Phase 5: Write Test Code

Add test functions to:

- [tests/tests/syncback.rs](tests/tests/syncback.rs) -- syncback_tests! entries
- [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) -- new functions  
- [tests/tests/build.rs](tests/tests/build.rs) -- gen_build_tests! entries
- [tests/tests/syncback_roundtrip.rs](tests/tests/syncback_roundtrip.rs) -- roundtrip test

### Phase 6: Run and Verify

Run each test, verify it exercises the actual implementation, review snapshot outputs. If a test passes too easily, make it harder. If a test reveals a bug, document it and fix the implementation.

---

## Key Files to Create

- `tests/rojo_test/fixture_gen.rs` -- rbxm fixture generator
- `rojo-test/syncback-tests/ambiguous_basic/` (+ 7 more syncback fixtures)
- `rojo-test/syncback-tests/ambiguous_expansion_resolved/` (+ 3 more incremental fixtures)
- `rojo-test/serve-tests/ambiguous_container/` -- two-way sync fixture
- `rojo-test/build-tests/ambiguous_container/` (+ 1 more build fixture)

## Key Files to Modify

- [tests/tests/syncback.rs](tests/tests/syncback.rs) -- add ~12 entries to macros
- [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) -- add ~12 test functions
- [tests/tests/build.rs](tests/tests/build.rs) -- add 2 entries to macro
- [tests/tests/syncback_roundtrip.rs](tests/tests/syncback_roundtrip.rs) -- add 1 roundtrip
- [tests/rojo_test/mod.rs](tests/rojo_test/mod.rs) -- include fixture_gen module

