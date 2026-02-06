---
name: Stress Tests Two-Way Sync
overview: Nuclear-grade stress tests and randomized fuzzing for two_way_sync.rs. Deterministic tests cranked to 10+ cycles. Randomized fuzzer generates hundreds of random operation sequences per test with seeded PRNG for reproducibility.
todos:
  - id: fixture
    content: Create syncback_stress fixture (default.project.json5 + 5 .luau files)
    status: pending
  - id: helpers
    content: Add timing helpers (send_update_fast, send_update_no_wait, wait_for_settle) and lookup helpers (get_stress_instances)
    status: pending
  - id: prng
    content: Add XorShift64 PRNG and FuzzOp enum for randomized test generation
    status: pending
  - id: rapid-source
    content: "Tests 26-27: rapid_source_writes_10x, rapid_source_writes_no_wait_10x"
    status: pending
  - id: rapid-rename
    content: "Tests 28-29: rapid_rename_chain_10x, rapid_rename_chain_directory_10x"
    status: pending
  - id: rapid-classname
    content: "Tests 30-31: rapid_classname_cycle_10x, rapid_classname_cycle_directory_init_10x"
    status: pending
  - id: combined-blitz
    content: "Tests 32-34: combined_rename_classname_source_blitz_10x, combined_rename_and_source_rapid_10x, combined_classname_and_source_rapid_10x"
    status: pending
  - id: multi-instance
    content: "Tests 35-36: multi_instance_source_update_single_request, multi_instance_rename_single_request"
    status: pending
  - id: delete-recreate
    content: "Tests 37-38: delete_and_recreate_via_filesystem_recovery, rapid_delete_recreate_cycle_5x"
    status: pending
  - id: echo-stress
    content: "Tests 39-40: echo_suppression_rapid_adds_10x, echo_suppression_mixed_operations"
    status: pending
  - id: encoded-stress
    content: "Test 41: encoded_name_rapid_rename_chain"
    status: pending
  - id: fuzz-source-rename
    content: "Test 42: fuzz_source_and_rename_200_iterations - randomized Source writes and renames"
    status: pending
  - id: fuzz-classname
    content: "Test 43: fuzz_classname_cycling_200_iterations - randomized ClassName transitions"
    status: pending
  - id: fuzz-combined
    content: "Test 44: fuzz_combined_operations_200_iterations - randomized mix of ALL operation types"
    status: pending
  - id: fuzz-multi-instance
    content: "Test 45: fuzz_multi_instance_100_iterations - randomized operations across 5 instances simultaneously"
    status: pending
  - id: fuzz-directory
    content: "Test 46: fuzz_directory_format_operations_100_iterations - randomized rename/class changes on directory-format scripts"
    status: pending
isProject: false
---

# Nuclear-Grade Stress Tests + Randomized Fuzzing for Two-Way Sync

## Context

14 commits ahead of origin introduced major changes to:

- [src/change_processor.rs](src/change_processor.rs) (+780 lines) -- event suppression, pending recovery, rename/ClassName/Source handling, `overridden_source_path` chaining, `ComputeResult` with recovery tracking
- [src/web/api.rs](src/web/api.rs) (+1284 lines) -- syncback deletion, `suppress_path`, standalone-to-directory conversion, encoded name handling

Existing 25 tests cover each operation once. No tests exercise rapid multi-cycle operations, race conditions between VFS events and API calls, or the recovery mechanism under stress.

## New Fixture

`**rojo-test/serve-tests/syncback_stress/**` -- 5 standalone ModuleScripts:

- `default.project.json5` (DataModel with ReplicatedStorage -> `src/`)
- `src/Alpha.luau`, `src/Bravo.luau`, `src/Charlie.luau`, `src/Delta.luau`, `src/Echo.luau`

## New Infrastructure in `two_way_sync.rs`

### Timing Helpers

- `send_update_fast(session, session_id, update)` -- 50ms sleep (races the 200ms recovery delay)
- `send_update_no_wait(session, session_id, update)` -- 0ms sleep (fire and forget)
- `send_removal_fast(session, session_id, ids)` -- 50ms sleep
- `wait_for_settle()` -- 800ms sleep (covers 200ms recovery + 500ms periodic sweep + buffer)
- `get_stress_instances(session)` -- Returns `(SessionId, Vec<(Ref, String)>)` for Alpha-Echo

### Deterministic PRNG (no external dependency)

```rust
struct XorShift64(u64);
impl XorShift64 {
    fn new(seed: u64) -> Self { Self(seed) }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, min: u64, max: u64) -> u64 {
        min + (self.next() % (max - min))
    }
    fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }
}
```

### FuzzOp Enum

```rust
enum FuzzOp {
    WriteSource(String),
    Rename(String),
    ChangeClass(&'static str),
    RenameAndSource(String, String),
    ClassAndSource(&'static str, String),
    RenameAndClass(String, &'static str),
    RenameClassAndSource(String, &'static str, String),
}
```

### FuzzState Tracker

Tracks expected state after each operation for invariant checking:

```rust
struct FuzzState {
    current_name: String,
    current_class: &'static str,
    current_source: String,
}
```

After `wait_for_settle()`, verify:

1. Exactly one file matching `{current_name}{suffix}.luau` exists (where suffix depends on class)
2. No stale files from previous names/extensions
3. File content matches `current_source`

### `run_fuzz_iteration` Helper

Takes a `FuzzState`, generates a random `FuzzOp`, executes it, updates state, and returns the new state. This is the core building block for all fuzz tests.

---

## PART 1: Deterministic Stress Tests (10+ cycles)

All deterministic tests cranked from 5 to 10+ cycles with 50ms timing.

### A. Rapid Source Flip-Flop (Tests 26-27)

**Test 26: `rapid_source_writes_10x**`

- Write Source to `existing.luau` 10 times with `send_update_fast` (50ms gaps)
- Content: `"-- v1"` through `"-- v10"`
- Verify final content is `"-- v10"`
- Fixture: `syncback_write`

**Test 27: `rapid_source_writes_no_wait_10x**`

- Fire 10 Source writes back-to-back with ZERO delay
- Wait 800ms at end
- Verify file contains last write
- Fixture: `syncback_write`

### B. Rapid Rename Chain (Tests 28-29)

**Test 28: `rapid_rename_chain_10x**`

- Rename through 10 names: `existing` -> `R1` -> `R2` -> ... -> `R10`
- Re-read tree between each rename, 50ms delay
- Verify only `R10.luau` exists; ALL intermediates gone
- Verify content preserved through all 10 renames
- Fixture: `syncback_write`

**Test 29: `rapid_rename_chain_directory_10x**`

- Rename `DirModuleWithChildren` through 10 names
- Verify final directory has `init.luau` + `ChildA.luau` + `ChildB.luau` intact
- Verify ALL 9 intermediate directory names are gone
- Fixture: `syncback_format_transitions`

### C. Rapid ClassName Cycling (Tests 30-31)

**Test 30: `rapid_classname_cycle_10x**`

- Cycle `existing` through 10 transitions: Module -> Script -> Local -> Module -> Script -> Local -> Module -> Script -> Local -> Module
- Re-read tree between each
- Verify final file is `.luau` (ModuleScript); no stale `.server.luau` or `.local.luau`
- Fixture: `syncback_write`

**Test 31: `rapid_classname_cycle_directory_init_10x**`

- Cycle `DirModuleWithChildren` init through 10 transitions
- Verify children (ChildA.luau, ChildB.luau) survive ALL transitions
- Fixture: `syncback_format_transitions`

### D. Combined Operations Blitz (Tests 32-34)

**Test 32: `combined_rename_classname_source_blitz_10x**`

- 10 rounds, each sending rename + ClassName + Source in a single update
- Cycles through all 3 class types with different names and sources each round
- Verify final file matches expected name, extension, and content
- Verify ZERO intermediate files remain
- Fixture: `syncback_write`

**Test 33: `combined_rename_and_source_rapid_10x**`

- 10 rounds of rename + Source update (no class change)
- Verify Source always in the renamed file, never in stale location
- Fixture: `syncback_write`

**Test 34: `combined_classname_and_source_rapid_10x**`

- 10 rounds of ClassName cycling + Source update (no rename)
- Verify Source always in the file with correct extension
- Fixture: `syncback_write`

### E. Multi-Instance Concurrent (Tests 35-36)

**Test 35: `multi_instance_source_update_single_request**`

- Single WriteRequest updating Source for all 5 instances (Alpha-Echo)
- Verify all 5 files updated correctly
- Fixture: `syncback_stress`

**Test 36: `multi_instance_rename_single_request**`

- Single WriteRequest renaming all 5 instances at once
- Verify all 5 renames succeed, no collisions
- Fixture: `syncback_stress`

### F. Delete + Recreate Race (Tests 37-38)

**Test 37: `delete_and_recreate_via_filesystem_recovery**`

- Delete instance via API
- Immediately `fs::write` file back (simulating editor undo)
- Wait 800ms for `process_pending_recoveries`
- Verify tree recovers (instance in read API)
- Exercises `pending_recovery` + `ComputeResult::removed_path`
- Fixture: `syncback_write`

**Test 38: `rapid_delete_recreate_cycle_5x**`

- 5 cycles of: delete via API -> `fs::write` file back -> wait for recovery -> verify tree
- Exercises recovery system under repeated hammering
- Fixture: `syncback_write`

### G. Echo Suppression Under Load (Tests 39-40)

**Test 39: `echo_suppression_rapid_adds_10x**`

- Add 10 new instances in rapid succession (10 separate write requests, no delay)
- Verify all 10 files exist on disk
- Verify message cursor hasn't exploded
- Fixture: `syncback_write`

**Test 40: `echo_suppression_mixed_operations**`

- Single write request: 2 adds + 2 updates + 1 removal
- Verify all operations complete correctly
- Verify server responsive
- Fixture: `syncback_stress`

### H. Encoded Name Stress (Test 41)

**Test 41: `encoded_name_rapid_rename_chain**`

- Rename `What?Module` through 5 names containing special characters
- Example names: `What?V2`, `Key:V3`, `Slash/V4`, `Dot.V5`, `Star*V6`
- Verify all renames use `%QUESTION%`, `%COLON%`, etc. encoded filesystem names
- Fixture: `syncback_encoded_names`

---

## PART 2: Randomized Fuzzing (hundreds of iterations)

Each fuzz test uses the `XorShift64` PRNG with a fixed seed for reproducibility. On failure, the seed and iteration number are printed so the exact failing sequence can be replayed.

### Test 42: `fuzz_source_and_rename_200_iterations`

- **Fixture:** `syncback_write`
- **Per iteration:** Generate a random sequence of 3-8 operations, each being either `WriteSource` or `Rename`
- **200 iterations** with seeds 1..200
- **Timing:** `send_update_fast` (50ms) between ops
- **Invariants after each iteration:**
  1. Exactly one `.luau` file exists in `src/` with the current name
  2. File content matches the last Source write (if any)
  3. No stale files from previous names
- **Between iterations:** clean up by renaming back to `existing` and restoring original source

### Test 43: `fuzz_classname_cycling_200_iterations`

- **Fixture:** `syncback_write`
- **Per iteration:** Generate a random sequence of 3-8 `ChangeClass` operations (randomly picking ModuleScript/Script/LocalScript each time)
- **200 iterations** with seeds 1..200
- **Invariants:** file extension matches last class, no stale extensions

### Test 44: `fuzz_combined_operations_200_iterations`

- **Fixture:** `syncback_write`
- **Per iteration:** Generate a random sequence of 3-8 operations of ANY type (`WriteSource`, `Rename`, `ChangeClass`, `RenameAndSource`, `ClassAndSource`, `RenameAndClass`, `RenameClassAndSource`)
- **200 iterations** with seeds 1..200
- **This is the nuclear option** -- every possible combination of operations in random order with random timing
- **Invariants:** correct file name, extension, content after each iteration

### Test 45: `fuzz_multi_instance_100_iterations`

- **Fixture:** `syncback_stress`
- **Per iteration:** Randomly select 1-5 of the Alpha-Echo instances, generate a random operation for each, send as a SINGLE WriteRequest with multiple updates
- **100 iterations** with seeds 1..100
- **Invariants:** all 5 instances are in a consistent state after each iteration (correct files, no stale artifacts)

### Test 46: `fuzz_directory_format_operations_100_iterations`

- **Fixture:** `syncback_format_transitions`
- **Per iteration:** Randomly choose between rename and ClassName change for `DirModuleWithChildren`
- **100 iterations** with seeds 1..100
- **Invariants:** directory exists with correct name, init file has correct extension, children (ChildA.luau, ChildB.luau) present

---

## Implementation Details

### PRNG Reproducibility

Every fuzz test prints the seed on failure:

```rust
for seed in 1..=200 {
    let result = std::panic::catch_unwind(|| {
        run_fuzz_iteration(seed, ...);
    });
    if result.is_err() {
        panic!("Fuzz test failed at seed {}. Replay with this seed to reproduce.", seed);
    }
}
```

### State Reset Between Iterations

For the fuzz tests that run hundreds of iterations on the same fixture, we need to reset state between iterations. The approach:

1. After each iteration, rename the instance back to its original name
2. Change ClassName back to original
3. Write original Source back
4. Wait for settle

This avoids needing to restart the serve session between iterations (which would be slow).

### Timing Strategy

- `send_update_fast`: 50ms -- tight enough to race the 200ms recovery delay
- `send_update_no_wait`: 0ms -- fires before VFS events from previous op are processed
- `wait_for_settle`: 800ms -- covers recovery delay (200ms) + periodic sweep (500ms) + buffer (100ms)
- Between fuzz iterations: 300ms settle time (lighter than full settle since we just need API to finish)

### File Verification Helper

```rust
fn verify_instance_file(
    src_dir: &Path,
    expected_name: &str,
    expected_class: &str,
    expected_source: Option<&str>,
) {
    let suffix = match expected_class {
        "Script" => ".server",
        "LocalScript" => ".local",
        _ => "",
    };
    let expected_file = src_dir.join(format!("{}{}.luau", expected_name, suffix));
    assert!(expected_file.is_file(), "Expected file: {}", expected_file.display());
    if let Some(source) = expected_source {
        let content = fs::read_to_string(&expected_file).unwrap();
        assert!(content.contains(source), "Expected source '{}' in {}", source, content);
    }
    // Verify no stale files with the same name but wrong extension
    for stale_suffix in &[".server", ".local", ".client", ".plugin", ""] {
        if *stale_suffix == suffix { continue; }
        let stale = src_dir.join(format!("{}{}.luau", expected_name, stale_suffix));
        assert!(!stale.exists(), "Stale file should not exist: {}", stale.display());
    }
}
```

