---
name: VFS staleness test fixtures
overview: Create test fixtures that replicate VFS tree staleness, add a tree-freshness assertion helper, tighten lenient existing tests, and expose a test-only API for tree validation -- all to surface the root causes of missed VFS events so they can be fixed properly.
todos:
  - id: api-endpoint
    content: Add GET /api/validate-tree endpoint that re-snapshots and returns drift counts without correcting the tree
    status: completed
  - id: session-method
    content: Extract check_tree_freshness() in ServeSession (no age gate, no apply, returns counts) and set VALIDATE_TREE_ON_CONNECT = false
    status: completed
  - id: test-helper
    content: Add assert_tree_fresh() helper and ValidateTreeResponse type to tests/rojo_test/serve_util.rs
    status: completed
  - id: fixture
    content: Create rojo-test/serve-tests/stale_tree/ fixture with project file and multiple scripts
    status: completed
  - id: staleness-tests
    content: "Write 4 new staleness tests: bulk changes, post-API external edit, rapid delete/recreate, directory restructure"
    status: completed
  - id: append-freshness
    content: Append assert_tree_fresh() to all watcher_* and echo_suppression_* tests in two_way_sync.rs
    status: completed
  - id: tighten-echo
    content: Tighten echo suppression cursor_delta assertion from < 10 to <= 3
    status: completed
  - id: chaos-fuzzer
    content: Write chaos fuzzer test that slams the filesystem with random operations for 10 seconds, then asserts tree freshness
    status: completed
isProject: false
---

# VFS Staleness Test Fixtures and Test Tightening

## Context

The `validate_tree` bandaid is being disabled. We need test fixtures that REPRODUCE VFS staleness so we can fix the underlying issues. We also need to tighten existing tests that are too lenient per the quality standard: "Any deviation is a bug."

## Leniency Issues Found in Existing Tests

### 1. Echo suppression uses `cursor_delta < 10` (critical)

In `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)` line 725:

```rust
assert!(cursor_delta < 10, ...);
```

Allowing up to 9 extra messages is absurdly lenient. For a single add operation, the expected delta is 0-2 (the suppressed echo plus maybe one metadata update). This means echo suppression could be 80% broken and tests would still pass.

### 2. No tree-vs-filesystem validation (critical)

No test ever asserts that the in-memory `RojoTree` matches what a fresh snapshot from disk would produce. Tests only check individual instances/properties. A watcher test could pass while the tree has phantom instances, missing children, or stale properties on unrelated instances.

### 3. `send_update` uses fixed sleep instead of event verification

In `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)` line 107:

```rust
thread::sleep(Duration::from_millis(300));
```

This is a guess, not a guarantee. If the ChangeProcessor is slow (CI load), the tree may not be updated yet. If the operation is fast, we're wasting 300ms per test.

### 4. No completeness checks after operations

Watcher tests (e.g., `watcher_multi_file_simultaneous_edits`) verify that expected changes arrived but never verify that ONLY expected changes arrived and that no unrelated instances were corrupted.

## Plan

### Step 1: Add `GET /api/validate-tree` test-only endpoint

Add a new API endpoint to `[src/web/api.rs](src/web/api.rs)` that calls `validate_tree` logic WITHOUT the 5-second age gate (the age gate exists to avoid racing with the VFS in production, but tests need immediate validation). Returns a structured response:

```rust
#[derive(Serialize)]
struct ValidateTreeResponse {
    is_fresh: bool,       // true if tree matches filesystem
    added: usize,         // instances that should exist but don't
    removed: usize,       // instances in tree but not on disk
    updated: usize,       // instances with different properties
    elapsed_ms: f64,      // how long the validation took
}
```

Route: `GET /api/validate-tree` in the match block around line 127 of `[src/web/api.rs](src/web/api.rs)`.

The handler re-snapshots from VFS, diffs, and returns the result WITHOUT applying corrections (read-only check). This way the test sees the drift but the tree isn't silently fixed.

### Step 2: Add `assert_tree_fresh()` helper to test infrastructure

In `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)`, add:

```rust
impl TestServeSession {
    /// Assert the in-memory tree exactly matches the filesystem.
    /// Panics with a descriptive message if there is any drift.
    pub fn assert_tree_fresh(&self) {
        let url = format!("http://localhost:{}/api/validate-tree", self.port);
        let body = reqwest::blocking::get(url).unwrap().bytes().unwrap();
        let response: ValidateTreeResponse = deserialize_msgpack(&body).unwrap();
        
        assert!(
            response.is_fresh,
            "Tree does not match filesystem: {} added, {} removed, {} updated \
             (validated in {:.1}ms)",
            response.added, response.removed, response.updated, response.elapsed_ms
        );
    }
}
```

### Step 3: Create VFS staleness test fixture and tests

New fixture: `rojo-test/serve-tests/stale_tree/` with a simple project containing multiple scripts under `src/` (enough files to stress the watcher).

**Test A: `stale_tree_bulk_filesystem_changes**`
Simulates a git checkout -- change 20+ files simultaneously. Verifies every single change is reflected in the tree:

1. Start serve, wait for stability (1s)
2. Write 20 files simultaneously in a loop (no sleep between writes)
3. Wait for tree to settle (poll a canary file)
4. Call `assert_tree_fresh()` -- MUST pass

**Test B: `stale_tree_after_api_write_then_external_edit**`
Tests that echo suppression does NOT eat external edits:

1. Start serve
2. API write creates a file (triggers suppression)
3. Immediately overwrite the same file from the filesystem with different content
4. Wait for tree to settle
5. Verify tree has the EXTERNAL content (not the API-written content)
6. Call `assert_tree_fresh()`

**Test C: `stale_tree_rapid_delete_recreate_different_names**`
Tests the debouncer coalescing edge case:

1. Start serve with 5 files
2. Delete all 5, immediately create 5 new files with different names
3. Wait for tree to settle
4. Verify old names are gone, new names exist
5. Call `assert_tree_fresh()`

**Test D: `stale_tree_directory_restructure**`
Tests bulk restructuring (the git checkout pattern):

1. Start serve with a directory structure: `src/a.luau`, `src/b.luau`, `src/sub/c.luau`
2. Delete `src/sub/`, rename `src/a.luau` to `src/x.luau`, change content of `src/b.luau`
3. Wait for tree to settle
4. `assert_tree_fresh()`

### Step 4: Chaos fuzzer -- `fuzz_filesystem_chaos`

A single long-running test that slams the filesystem with random operations for 10 seconds straight, then waits for the tree to settle and asserts freshness. The goal is to reproduce the exact conditions that cause VFS drift. If this test ever fails, the failure message tells us exactly what the tree got wrong.

**Operations the fuzzer randomly picks from each iteration:**

- Create a new `.luau` file with random content
- Delete an existing file
- Overwrite an existing file with new content
- Rename a file to a new name
- Create a subdirectory and add a file inside it
- Delete a subdirectory (recursive)
- Rename a directory
- Rapid delete+recreate of the same file (the known edge case)
- Write to a file, then immediately overwrite it again (double-write)

**Structure:**

```rust
#[test]
fn fuzz_filesystem_chaos() {
    run_serve_test("stale_tree", |session, _redactions| {
        let src = session.path().join("src");
        let mut rng = rand::thread_rng();
        
        let start = Instant::now();
        let duration = Duration::from_secs(10);
        let mut op_count = 0u64;
        
        // Track what files/dirs we've created so we can pick valid targets
        let mut known_files: Vec<PathBuf> = vec![/* initial fixture files */];
        let mut known_dirs: Vec<PathBuf> = vec![src.clone()];
        
        while start.elapsed() < duration {
            match rng.gen_range(0..9) {
                0 => { /* create new file */ }
                1 => { /* delete random existing file */ }
                2 => { /* overwrite random existing file */ }
                3 => { /* rename random file */ }
                4 => { /* create subdirectory + file */ }
                5 => { /* delete random subdirectory */ }
                6 => { /* rename random directory */ }
                7 => { /* rapid delete+recreate same file */ }
                8 => { /* double-write same file */ }
                _ => unreachable!(),
            }
            op_count += 1;
            
            // Tiny random delay between ops (0-10ms) to vary timing
            thread::sleep(Duration::from_millis(rng.gen_range(0..10)));
        }
        
        eprintln!("Chaos fuzzer completed {} operations in {:?}", op_count, start.elapsed());
        
        // Wait for VFS watcher + ChangeProcessor to fully settle
        // Use a generous timeout since the watcher may be processing a backlog
        thread::sleep(Duration::from_secs(3));
        
        // THE CRITICAL ASSERTION: tree must match filesystem after all that chaos
        session.assert_tree_fresh();
    });
}
```

**Key design decisions:**

- Uses `rand` crate (add as a dev-dependency)
- Tracks its own inventory of created files/dirs so it picks valid targets for delete/rename/overwrite
- Handles errors gracefully (file might already be deleted by a previous op) -- just skips and continues
- Tiny random delay (0-10ms) between ops to vary the timing pressure on the debouncer
- 3-second cooldown after the chaos period to let the watcher fully drain
- Single `assert_tree_fresh()` at the end -- if it fails, the drift counts tell us exactly what went wrong
- Logs total operation count for debugging

**Why this catches the bug:** The original issue was a developer making filesystem changes and then finding the tree stale on reconnect. The fuzzer recreates this by doing hundreds of rapid operations with varied timing, which is exactly the kind of workload that overwhelms the notify debouncer and triggers `RescanRequired` events. If the tree stays fresh after 10 seconds of abuse, the watcher is solid. If it drifts, we have a reproduction.

### Step 5: Add `assert_tree_fresh()` to existing watcher tests (unchanged from above)

Append `assert_tree_fresh()` at the end of every `watcher_*` test in `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)`. This is the key force multiplier -- every existing watcher test becomes a staleness detector. If any of the ~25 watcher tests produce tree drift that the VFS watcher didn't resolve, it will now fail.

The following tests get the assertion appended (after their existing verifications, before the closure ends):

- All `watcher_*` tests (25+ tests)
- All `echo_suppression_*` tests (3 tests)
- `delete_and_recreate_via_filesystem_recovery`

### Step 6: Tighten echo suppression assertion

In `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)`, replace the lenient `cursor_delta < 10` with a tighter bound. For a single add with proper suppression, the expected delta is 0-2. Use:

```rust
assert!(
    cursor_delta <= 3,
    "Echo suppression failure: cursor advanced by {} (expected <= 3). \
     VFS echo events are leaking through suppression.",
    cursor_delta
);
```

### Step 7: Disable `VALIDATE_TREE_ON_CONNECT`

In `[src/serve_session.rs](src/serve_session.rs)`, set the constant to `false`:

```rust
const VALIDATE_TREE_ON_CONNECT: bool = false;
```

This removes the bandaid so the real tests surface real failures.

## Expected outcomes

- If VFS watcher events are reliably delivered: all tests pass with no drift detected
- If VFS events are missed: `assert_tree_fresh()` fails with exact counts of added/removed/updated drift, giving us a precise reproduction to debug
- The echo suppression tightening will reveal if suppression is leaky
- The bulk-change tests will reveal if the notify debouncer's `RescanRequired` causes tree drift

## Files to modify

- `[src/web/api.rs](src/web/api.rs)` -- new `GET /api/validate-tree` endpoint
- `[src/serve_session.rs](src/serve_session.rs)` -- extract snapshot+diff logic into a reusable `check_tree_freshness()` method, set constant to false
- `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)` -- `assert_tree_fresh()` helper and response type
- `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)` -- append `assert_tree_fresh()` to watcher tests, tighten echo suppression, add chaos fuzzer
- `rojo-test/serve-tests/stale_tree/` -- new test fixture (project file + src scripts)
- `Cargo.toml` -- add `rand` as a dev-dependency for the fuzzer

