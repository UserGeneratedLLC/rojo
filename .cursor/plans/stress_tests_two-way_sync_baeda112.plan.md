---
name: Stress Tests Two-Way Sync
overview: Nuclear-grade stress tests, randomized fuzzing, and file watcher stress tests. Deterministic tests at 10+ cycles. Randomized fuzzer generates hundreds of random operation sequences. File watcher tests hammer the VFS pipeline with direct filesystem operations, init file shenanigans, singular/directory format flips, and editor save pattern simulations.
todos:
  - id: fixture
    content: Create syncback_stress fixture (default.project.json5 + 5 .luau files)
    status: completed
  - id: helpers
    content: Add timing helpers (send_update_fast, send_update_no_wait, wait_for_settle), lookup helpers (get_stress_instances), and filesystem polling helpers (poll_tree_source, poll_tree_has_instance, poll_tree_no_instance)
    status: completed
  - id: prng
    content: Add XorShift64 PRNG, FuzzOp enum, FuzzState tracker, and verify_instance_file helper
    status: completed
  - id: rapid-source
    content: "Tests 26-27: rapid_source_writes_10x, rapid_source_writes_no_wait_10x"
    status: completed
  - id: rapid-rename
    content: "Tests 28-29: rapid_rename_chain_10x, rapid_rename_chain_directory_10x"
    status: completed
  - id: rapid-classname
    content: "Tests 30-31: rapid_classname_cycle_10x, rapid_classname_cycle_directory_init_10x"
    status: completed
  - id: combined-blitz
    content: "Tests 32-34: combined_rename_classname_source_blitz_10x, combined_rename_and_source_rapid_10x, combined_classname_and_source_rapid_10x"
    status: completed
  - id: multi-instance
    content: "Tests 35-36: multi_instance_source_update_single_request, multi_instance_rename_single_request"
    status: completed
  - id: delete-recreate
    content: "Tests 37-38: delete_and_recreate_via_filesystem_recovery, rapid_delete_recreate_cycle_5x"
    status: completed
  - id: echo-stress
    content: "Tests 39-40: echo_suppression_rapid_adds_10x, echo_suppression_mixed_operations"
    status: completed
  - id: encoded-stress
    content: "Test 41: encoded_name_rapid_rename_chain"
    status: completed
  - id: fuzz-source-rename
    content: "Test 42: fuzz_source_and_rename_200_iterations"
    status: completed
  - id: fuzz-classname
    content: "Test 43: fuzz_classname_cycling_200_iterations"
    status: completed
  - id: fuzz-combined
    content: "Test 44: fuzz_combined_operations_200_iterations"
    status: completed
  - id: fuzz-multi-instance
    content: "Test 45: fuzz_multi_instance_100_iterations"
    status: completed
  - id: fuzz-directory
    content: "Test 46: fuzz_directory_format_operations_100_iterations"
    status: completed
  - id: watcher-rapid-edits
    content: "Tests 47-48: watcher_rapid_source_edits_on_disk_10x, watcher_burst_writes_100x"
    status: completed
  - id: watcher-rename
    content: "Tests 49-50: watcher_filesystem_rename_chain_10x, watcher_rename_with_content_change"
    status: completed
  - id: watcher-delete-recreate
    content: "Tests 51-53: watcher_delete_recreate_immediate, watcher_delete_recreate_cycle_5x, watcher_delete_recreate_different_content"
    status: completed
  - id: watcher-init
    content: "Tests 54-57: watcher_edit_init_file, watcher_init_type_cycling_10x, watcher_delete_init_file, watcher_replace_init_file_type"
    status: completed
  - id: watcher-format-flip
    content: "Tests 58-60: watcher_standalone_to_directory_conversion, watcher_directory_to_standalone_conversion, watcher_format_flip_flop_5x"
    status: completed
  - id: watcher-editor-patterns
    content: "Tests 61-62: watcher_atomic_save_pattern, watcher_backup_rename_write_pattern"
    status: completed
  - id: watcher-parent-dir
    content: "Tests 63-64: watcher_parent_directory_rename, watcher_parent_directory_delete_all"
    status: completed
  - id: watcher-concurrent
    content: "Tests 65-66: watcher_filesystem_and_api_concurrent, watcher_multi_file_simultaneous_edits"
    status: completed
isProject: false
---

# Nuclear-Grade Stress Tests + Fuzzing + File Watcher Stress

## Context

16 commits ahead of origin. Major changes to:

- [src/change_processor.rs](src/change_processor.rs) -- event suppression (canonical+raw matching), pending recovery, rename/ClassName/Source handling, `overridden_source_path`, canonicalize retry, init file detection
- [src/web/api.rs](src/web/api.rs) -- syncback deletion (two-phase, `actually_removed` tracking), `suppress_path` (single canonical entry), standalone-to-directory conversion, encoded name handling
- [crates/memofs/src/lib.rs](crates/memofs/src/lib.rs) -- watches NOT removed on delete (for rapid recreate)

Existing tests: 25 in `two_way_sync.rs` (API-driven), ~10 in `serve.rs` (filesystem-driven). No tests exercise:

- Rapid multi-cycle operations
- Recovery mechanism under stress
- Init file type cycling on disk
- Singular/directory format conversion via filesystem
- Editor save patterns (atomic write, backup-rename-write)
- Concurrent filesystem + API operations

## Files Changed

- [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) -- ALL new tests go here
- [rojo-test/serve-tests/syncback_stress/](rojo-test/serve-tests/syncback_stress/) -- new fixture (5 ModuleScripts)

## New Fixture

`**rojo-test/serve-tests/syncback_stress/**`

- `default.project.json5` (DataModel with ReplicatedStorage -> `src/`)
- `src/Alpha.luau`, `src/Bravo.luau`, `src/Charlie.luau`, `src/Delta.luau`, `src/Echo.luau`

Each file: `-- {Name} module\nreturn {}`

---

## New Infrastructure

### Timing Helpers

```rust
fn send_update_fast(session, session_id, update)  // 50ms sleep
fn send_update_no_wait(session, session_id, update) // 0ms sleep
fn send_removal_fast(session, session_id, ids)     // 50ms sleep
fn wait_for_settle()                                // 800ms sleep
```

### Tree Polling Helpers (for file-watcher tests)

```rust
/// Poll tree until an instance with `name` under `parent_id` has Source containing `expected`.
/// Panics after `timeout_ms` if not found.
fn poll_tree_source(
    session: &TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    expected_content: &str,
    timeout_ms: u64,
)

/// Poll tree until an instance with `name` exists under `parent_id`.
fn poll_tree_has_instance(
    session: &TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    timeout_ms: u64,
) -> Ref

/// Poll tree until NO instance with `name` exists under `parent_id`.
fn poll_tree_no_instance(
    session: &TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    timeout_ms: u64,
)

/// Poll tree until instance `name` has the expected ClassName.
fn poll_tree_class(
    session: &TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    expected_class: &str,
    timeout_ms: u64,
)
```

These all use a 50ms polling interval with `get_api_read()`. The typical timeout for file watcher propagation is 2000ms (50ms debounce + processing time + safety margin).

### Deterministic PRNG

```rust
struct XorShift64(u64);
impl XorShift64 {
    fn new(seed: u64) -> Self { ... }
    fn next(&mut self) -> u64 { ... }
    fn range(&mut self, min: u64, max: u64) -> u64 { ... }
    fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T { ... }
}
```

### FuzzOp / FuzzState / verify_instance_file

(Same as previous plan revision -- FuzzOp enum with 7 variants, FuzzState tracking current_name/class/source, verify_instance_file checking file existence + extension + content + no stale files)

---

## PART 1: API-Driven Stress Tests (Tests 26-41)

All tests use `send_update_fast` (50ms) or `send_update_no_wait` (0ms) and verify final state.

### A. Rapid Source (26-27)

- **26: `rapid_source_writes_10x**` -- 10 Source writes at 50ms gaps. Fixture: `syncback_write`
- **27: `rapid_source_writes_no_wait_10x**` -- 10 Source writes with 0ms gaps. Fixture: `syncback_write`

### B. Rapid Rename (28-29)

- **28: `rapid_rename_chain_10x**` -- 10 sequential renames (re-read tree each time). Fixture: `syncback_write`
- **29: `rapid_rename_chain_directory_10x**` -- 10 renames of directory-format script, verify children intact. Fixture: `syncback_format_transitions`

### C. Rapid ClassName (30-31)

- **30: `rapid_classname_cycle_10x**` -- 10 class transitions (Module->Script->Local->...). Fixture: `syncback_write`
- **31: `rapid_classname_cycle_directory_init_10x**` -- 10 class transitions on directory init, verify children survive. Fixture: `syncback_format_transitions`

### D. Combined Blitz (32-34)

- **32: `combined_rename_classname_source_blitz_10x**` -- All 3 in one update, 10 rounds cycling classes. Fixture: `syncback_write`
- **33: `combined_rename_and_source_rapid_10x**` -- Rename + Source, 10 rounds. Fixture: `syncback_write`
- **34: `combined_classname_and_source_rapid_10x**` -- ClassName + Source, 10 rounds. Fixture: `syncback_write`

### E. Multi-Instance (35-36)

- **35: `multi_instance_source_update_single_request**` -- Single WriteRequest updating 5 instances. Fixture: `syncback_stress`
- **36: `multi_instance_rename_single_request**` -- Single WriteRequest renaming 5 instances. Fixture: `syncback_stress`

### F. Delete + Recreate Race (37-38)

- **37: `delete_and_recreate_via_filesystem_recovery**` -- API delete, then `fs::write` file back, wait for recovery. Fixture: `syncback_write`
- **38: `rapid_delete_recreate_cycle_5x**` -- 5 cycles of API delete + filesystem recreate. Fixture: `syncback_write`

### G. Echo Suppression (39-40)

- **39: `echo_suppression_rapid_adds_10x**` -- 10 rapid instance additions, verify cursor doesn't explode. Fixture: `syncback_write`
- **40: `echo_suppression_mixed_operations**` -- 2 adds + 2 updates + 1 removal in one request. Fixture: `syncback_stress`

### H. Encoded Names (41)

- **41: `encoded_name_rapid_rename_chain**` -- Rename `What?Module` through 5 special-char names. Fixture: `syncback_encoded_names`

---

## PART 2: Randomized Fuzzing (Tests 42-46)

Seeded `XorShift64` PRNG. On failure, prints seed + iteration for exact replay.

- **42: `fuzz_source_and_rename_200_iterations**` -- Random Source writes and renames, 3-8 ops per iteration. Fixture: `syncback_write`
- **43: `fuzz_classname_cycling_200_iterations**` -- Random ClassName transitions, 3-8 ops per iteration. Fixture: `syncback_write`
- **44: `fuzz_combined_operations_200_iterations**` -- ALL 7 FuzzOp variants random, 3-8 ops per iteration. Fixture: `syncback_write`
- **45: `fuzz_multi_instance_100_iterations**` -- Random ops across 1-5 instances per WriteRequest. Fixture: `syncback_stress`
- **46: `fuzz_directory_format_operations_100_iterations**` -- Random rename/class on directory-format script. Fixture: `syncback_format_transitions`

---

## PART 3: File Watcher Stress Tests (Tests 47-66)

These exercise the **filesystem -> VFS -> ChangeProcessor -> Tree** pipeline by directly manipulating files with `fs::write`, `fs::rename`, `fs::remove_file`, `fs::create_dir`, etc. Verification uses the `poll_tree_*` helpers to wait for the tree to update via `get_api_read()`.

Pattern for all watcher tests:

1. Get the ReplicatedStorage ID via API
2. Directly manipulate files in `session.path().join("src")`
3. Poll the tree until it reflects the change (2000ms timeout)
4. Assert final state

### I. Rapid Source Edits on Disk (47-48)

**Test 47: `watcher_rapid_source_edits_on_disk_10x**`

- Write `existing.luau` directly 10 times via `fs::write` with 30ms gaps
- Each write has distinct content: `"-- disk v1"` through `"-- disk v10"`
- Poll tree until Source contains `"-- disk v10"`
- Exercises: debouncer coalescing, snapshot regeneration, no lost updates
- Fixture: `syncback_write`

**Test 48: `watcher_burst_writes_100x**`

- Write `existing.luau` 100 times with ZERO delay (as fast as possible)
- Final content: `"-- burst final"`
- Poll tree until Source contains `"-- burst final"`
- Exercises: debouncer under extreme load, final state correctness
- Fixture: `syncback_write`

### J. Filesystem Rename (49-50)

**Test 49: `watcher_filesystem_rename_chain_10x**`

- Rename `existing.luau` on disk through 10 names: `R1.luau` -> `R2.luau` -> ... -> `R10.luau`
- 30ms between renames
- Poll tree until instance named `R10` exists
- Verify: no instances with intermediate names, content preserved
- Exercises: VFS Remove+Create events from renames, tree path tracking
- Fixture: `syncback_write`

**Test 50: `watcher_rename_with_content_change**`

- Write new content to a temp file, then `fs::rename` temp -> `existing.luau` (atomic overwrite)
- Poll tree until Source reflects new content
- Exercises: editor-style atomic writes via rename
- Fixture: `syncback_write`

### K. Delete + Recreate via Filesystem (51-53)

**Test 51: `watcher_delete_recreate_immediate**`

- `fs::remove_file("existing.luau")`, immediately `fs::write("existing.luau", new_content)`
- Poll tree until Source contains new content
- Exercises: `pending_recovery` mechanism -- VFS Remove may arrive but file already exists on disk
- Fixture: `syncback_write`

**Test 52: `watcher_delete_recreate_cycle_5x**`

- 5 cycles of: `fs::remove_file` -> 50ms delay -> `fs::write` with new content
- Each cycle uses distinct content
- After each cycle, poll tree for the new content
- Exercises: repeated recovery under stress (the recovery delay is 200ms, sweep is 500ms)
- Fixture: `syncback_write`

**Test 53: `watcher_delete_recreate_different_content**`

- Delete `existing.luau`, wait 300ms (let removal propagate), recreate with completely different content
- Poll tree until Source matches the new content AND instance still exists
- Exercises: instance removal + recreation via recovery mechanism
- Fixture: `syncback_write`

### L. Init File Shenanigans (54-57)

All use `syncback_format_transitions` fixture which has `DirModuleWithChildren/init.luau` + `ChildA.luau` + `ChildB.luau`.

**Test 54: `watcher_edit_init_file**`

- `fs::write("DirModuleWithChildren/init.luau", "-- edited init")`
- Poll tree until `DirModuleWithChildren` instance Source contains `"-- edited init"`
- Verify children still exist
- Exercises: init file instigating_source -> parent directory snapshot

**Test 55: `watcher_init_type_cycling_10x**`

- 10 cycles of renaming the init file: `init.luau` -> `init.server.luau` -> `init.local.luau` -> `init.luau` -> ...
- After each rename, poll tree until ClassName matches expected (ModuleScript/Script/LocalScript)
- Verify children (ChildA, ChildB) survive all 10 transitions
- Exercises: init file detection (`find_init_file`), snapshot_path resolution, tree ClassName updates

**Test 56: `watcher_delete_init_file**`

- Delete `DirModuleWithChildren/init.luau`
- Poll tree until `DirModuleWithChildren` ClassName changes (no longer a ModuleScript since init file is gone -- becomes a Folder)
- Verify children still exist (directory still exists on disk)
- Exercises: init file removal edge case

**Test 57: `watcher_replace_init_file_type**`

- Delete `init.luau`, immediately create `init.server.luau` with same content
- Poll tree until ClassName is "Script"
- Verify children unaffected
- Exercises: atomic init file type swap

### M. Singular <-> Directory Format Conversion (58-60)

**Test 58: `watcher_standalone_to_directory_conversion**`

- Start with `StandaloneModule.luau` (standalone ModuleScript)
- On disk: `fs::remove_file("StandaloneModule.luau")`, `fs::create_dir("StandaloneModule")`, `fs::write("StandaloneModule/init.luau", original_content)`, `fs::write("StandaloneModule/ChildNew.luau", child_content)`
- Poll tree until `StandaloneModule` instance has a child named `ChildNew`
- Verify `StandaloneModule` Source matches original content
- Exercises: complete format transition via filesystem, VFS picks up directory + init + children
- Fixture: `syncback_format_transitions`

**Test 59: `watcher_directory_to_standalone_conversion**`

- Start with `DirModuleWithChildren/` (directory with init.luau + children)
- Read original init.luau content
- On disk: `fs::remove_dir_all("DirModuleWithChildren")`, `fs::write("DirModuleWithChildren.luau", original_content)`
- Poll tree until `DirModuleWithChildren` has NO children (standalone script)
- Verify Source matches
- Exercises: directory removal + standalone file creation detected by watcher
- Fixture: `syncback_format_transitions`

**Test 60: `watcher_format_flip_flop_5x**`

- 5 cycles of: standalone -> directory -> standalone -> directory -> standalone
- Start with `StandaloneModule.luau`
- Each "to directory" step: remove file, create dir + init.luau + child
- Each "to standalone" step: remove dir, create .luau file
- Verify final state correct after all 5 cycles
- Exercises: the ultimate format conversion stress test
- Fixture: `syncback_format_transitions`

### N. Editor Save Patterns (61-62)

**Test 61: `watcher_atomic_save_pattern**`

- Simulate VSCode/Vim atomic save: write to `existing.luau.tmp`, then `fs::rename("existing.luau.tmp", "existing.luau")`
- Poll tree until Source reflects new content
- Do this 5 times with different content
- Exercises: the most common real-world save pattern
- Fixture: `syncback_write`

**Test 62: `watcher_backup_rename_write_pattern**`

- Simulate editor backup-rename-write: `fs::rename("existing.luau", "existing.luau.bak")`, then `fs::write("existing.luau", new_content)`
- Poll tree until Source reflects new content
- Clean up .bak file
- Exercises: rename-away + create-new pattern (generates Remove + Create events)
- Fixture: `syncback_write`

### O. Parent Directory Operations (63-64)

**Test 63: `watcher_parent_directory_rename**`

- Rename `DirModuleWithChildren` directory to `RenamedDir` on disk
- Poll tree until instance `RenamedDir` appears with children intact
- Verify `DirModuleWithChildren` instance is gone
- Exercises: directory rename generates Remove(old/init.luau) + Create(new/init.luau), tree rebuilt
- Fixture: `syncback_format_transitions`

**Test 64: `watcher_parent_directory_delete_all**`

- `fs::remove_dir_all("DirModuleWithChildren")`
- Poll tree until `DirModuleWithChildren` instance is gone
- Verify no orphaned children remain
- Exercises: recursive delete generates Remove events for all children + init file
- Fixture: `syncback_format_transitions`

### P. Concurrent Filesystem + API (65-66)

**Test 65: `watcher_filesystem_and_api_concurrent**`

- Simultaneously: `fs::write` to `StandaloneModule.luau` on disk AND send API Source update to `StandaloneScript`
- Both operations target different instances to avoid data races
- Poll tree until BOTH instances reflect their respective changes
- Exercises: VFS events and tree mutation channel both active at same time
- Fixture: `syncback_format_transitions`

**Test 66: `watcher_multi_file_simultaneous_edits**`

- Write to 5 different files on disk simultaneously (Alpha through Echo) with distinct content
- Poll tree until ALL 5 instances reflect new Source
- Exercises: multiple VFS events in quick succession, all processed correctly
- Fixture: `syncback_stress`

---

## Implementation Details

### Polling Timeout Strategy

File watcher tests use 2000ms timeout (generous for: 50ms debounce + processing + recovery + Windows slowness). Polling interval is 50ms.

### Test Isolation

Each test uses `run_serve_test` which copies the fixture to a temp directory and spawns a fresh `rojo serve` process. Tests can freely mutate files without affecting other tests.

### PRNG Reproducibility

Fuzz tests print `"Fuzz test failed at seed {seed}. Replay with this seed to reproduce."` on failure.

### State Reset Between Fuzz Iterations

After each fuzz iteration: rename instance back to original name, restore original class, write original source, wait 300ms. Avoids restarting the serve session (which would be slow).

### File Watcher Test Timing

- After `fs::write`: poll immediately (debouncer needs ~50-100ms)
- After `fs::rename`: poll immediately (rename events come fast)
- After `fs::remove_file` + `fs::write` (delete+recreate): poll with 800ms initial wait (recovery delay 200ms + sweep 500ms)
- After directory operations: poll with extra buffer (multiple events to process)

### Total Test Count

- Part 1 (API stress): 16 tests (26-41)
- Part 2 (Fuzzing): 5 tests (42-46) with 900 total iterations
- Part 3 (File watcher): 20 tests (47-66)
- **Total: 41 new tests**

