---
name: Parallel filesystem IO
overview: Add parallel file reading via VFS prefetch cache, parallel snapshot building via rayon at directory level, and parallel RefPathIndex population to speed up all project loading paths.
todos:
  - id: vfs-prefetch-cache
    content: Add PrefetchCache struct (files + canonical maps), prefetch_cache field on VfsInner, read_raw() helper, wire into read()/read_to_string()/canonicalize()
    status: completed
  - id: vfs-prefetch-api
    content: Add Vfs::set_prefetch_cache() and Vfs::clear_prefetch_cache() public methods
    status: completed
  - id: vfs-prefetch-tests
    content: Add unit tests for prefetch cache (file hit/miss, read_to_string, canonicalize hit/miss, watch registration, clear)
    status: completed
  - id: prefetch-builder
    content: Write prefetch_project_files() using walkdir + rayon to parallel-read all files under root
    status: completed
  - id: parallel-snapshot-build
    content: Add rayon par_iter to snapshot_dir_no_meta in dir.rs for parallel child processing
    status: completed
  - id: env-var-fallback
    content: Add ATLAS_SEQUENTIAL env var to disable all parallelization for debugging
    status: completed
  - id: integrate-serve-session
    content: Call prefetch in ServeSession::init_tree() before snapshot_from_vfs()
    status: completed
  - id: parallel-refpathindex
    content: Parallelize RefPathIndex::populate_from_dir() using walkdir + rayon for file reads/parsing
    status: completed
  - id: test-determinism
    content: Add parallel snapshot determinism test - run snapshot_from_vfs N times, verify identical output
    status: completed
  - id: test-and-verify
    content: Run full cargo test, verify no regressions, add timing logs to confirm speedup
    status: completed
isProject: false
---

# Parallel Filesystem I/O for Project Loading

## Problem

Every file read goes through `Vfs` -> `Mutex<VfsInner>` -> `StdBackend::read()` -> `fs_err::read()`. This serializes all I/O. For a 3500-file project, startup is bottlenecked by ~3500 sequential syscalls, plus sequential CPU-bound parsing of each file.

**Affected commands:** `serve`, `build`, `upload`, `syncback`, `sourcemap` -- all call `snapshot_from_vfs()` which recursively reads every file through the VFS one at a time.

## Current Sequential Hotspots

1. `**snapshot_from_vfs()` initial tree build** ([src/serve_session.rs](src/serve_session.rs) line 141) -- The biggest bottleneck. Recursive directory traversal in [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs) lines 54-63, each child read goes through VFS mutex.
2. `**RefPathIndex::populate_from_dir()`** ([src/rojo_ref.rs](src/rojo_ref.rs) lines 372-431) -- Sequential walk + `std::fs::read()` for every `.meta.json5`/`.model.json5` file. Runs after tree build on `serve`.
3. **Two-way sync writes in `api.rs`** -- Sequential `fs::write()` calls (low priority, small batches).

## Strategy: Full Pipeline

Three layers, all using `rayon` (already a dependency):

1. **VFS Prefetch Cache** -- parallel I/O: read all files upfront
2. **Parallel Snapshot Build** -- parallel CPU: parse files concurrently at directory level
3. **Parallel RefPathIndex** -- parallel I/O+CPU: read and parse meta/model files concurrently

Combined timeline (vs current sequential):

```
Current:   [read→parse→read→parse→read→parse→read→parse→...]  (serial everything)

Pipelined: [=====parallel read all files=====]                 (Phase 1: I/O)
           ......[==parallel parse+build snapshots==]          (Phase 2: CPU)
```

### Part 1: VFS Prefetch Cache

**Concept:** Before `snapshot_from_vfs()` runs, walk the directory tree and read all file contents in parallel using `std::fs` + rayon (bypassing the VFS lock). Store results in a HashMap on VfsInner. The subsequent parallel snapshot build hits the cache (HashMap lookup, no I/O) instead of disk.

**Changes to [crates/memofs/src/lib.rs](crates/memofs/src/lib.rs):**

- Add `PrefetchCache` struct with `files: HashMap<PathBuf, Vec<u8>>` and `canonical: HashMap<PathBuf, PathBuf>`
- Add `prefetch_cache: Option<PrefetchCache>` field to `VfsInner`
- Extract a shared `read_raw(&mut self, path) -> io::Result<Vec<u8>>` helper that checks `prefetch_cache.files` first (using `HashMap::remove` to free memory on hit), falls back to backend
- Wire BOTH `VfsInner::read()` AND `VfsInner::read_to_string()` through `read_raw()`
- Add cache check to `VfsInner::canonicalize()`: check `prefetch_cache.canonical` first (using `HashMap::remove`), fall back to backend
- Add `Vfs::set_prefetch_cache(cache)` and `Vfs::clear_prefetch_cache()`

```rust
pub struct PrefetchCache {
    pub files: HashMap<PathBuf, Vec<u8>>,
    pub canonical: HashMap<PathBuf, PathBuf>,
}
```

**Critical: `read_to_string_lf_normalized` coverage.** The `Vfs::read_to_string_lf_normalized()` method (used by ALL `.luau` files via `lua.rs:55`) calls `VfsInner::read_to_string()` internally. Since we wire `VfsInner::read_to_string()` through `read_raw()`, LF-normalized reads hit the cache automatically. The flow: `Vfs::read_to_string_lf_normalized` -> lock -> `VfsInner::read_to_string` -> `read_raw` (cache hit) -> UTF-8 convert -> unlock -> CRLF->LF normalize.

**Critical: `canonicalize` coverage.** Every middleware calls `vfs.canonicalize(path)` once, storing the result in `InstanceMetadata::relevant_paths` for file-change-to-instance mapping. During init, these are parallelized in the prefetch phase. After init, the cache is cleared and `change_processor.rs` calls `canonicalize()` through the backend for fresh results.

**What is NOT cached** (and why that's acceptable):

- `read_dir()`: directory listing syscall, ~10us each, ~500 dirs = ~5ms total
- `metadata()`: stat syscall, ~~1-10us each. `get_dir_middleware` probes up to 13 init files per directory (~~6500 calls for 500 dirs = ~6.5ms)
- `exists()`: only used by `Project::load_initial_project` (1-2 calls before prefetch starts)

**No stale canonicalize entries:** The cache exists only during `init_tree()`. During this window, the filesystem is static (no watcher running, no mutations). After `clear_prefetch_cache()`, all canonicalize calls go through the backend -- the `ChangeProcessor` always gets fresh results for rename/move detection. **No memory leak:** `clear_prefetch_cache()` drops the entire `PrefetchCache` struct.

**New helper (in `src/` or inline in `serve_session.rs`):**

```rust
fn prefetch_project_files(root: &Path) -> io::Result<PrefetchCache> {
    use memofs::PrefetchCache;
    use rayon::prelude::*;
    use walkdir::WalkDir;

    let entries: Vec<walkdir::DirEntry> = WalkDir::new(root)
        .follow_links(true)   // Match VFS behavior: StdBackend::read() follows symlinks
        .into_iter()
        .filter_map(|e| e.ok())
        .collect();

    // Parallel file reads (files only)
    let file_data: Vec<_> = entries
        .par_iter()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            std::fs::read(&path).ok().map(|c| (path, c))
        })
        .collect();

    // Parallel canonicalize (files AND directories)
    let canonical_data: Vec<_> = entries
        .par_iter()
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            std::fs::canonicalize(&path).ok().map(|c| (path, c))
        })
        .collect();

    Ok(PrefetchCache {
        files: file_data.into_iter().collect(),
        canonical: canonical_data.into_iter().collect(),
    })
}
```

`walkdir` (already a dependency) handles the directory walk. `.follow_links(true)` ensures symlinked files are prefetched (walkdir handles symlink loop detection). `rayon::par_iter` parallelizes both file reads AND canonicalize syscalls. The canonicalize cache covers both files and directories since `dir.rs:66` canonicalizes directory paths.

**Integration in [src/serve_session.rs](src/serve_session.rs) `init_tree()`:**

```rust
fn init_tree(vfs: &Vfs, start_path: &Path) -> Result<...> {
    let root_project = Project::load_initial_project(vfs, start_path)?;

    if std::env::var("ATLAS_SEQUENTIAL").is_err() {
        let cache = prefetch_project_files(root_project.folder_location())?;
        vfs.set_prefetch_cache(cache);
    }

    let snapshot = snapshot_from_vfs(&instance_context, vfs, start_path)?;

    vfs.clear_prefetch_cache();
    // ... rest unchanged
}
```

**Note:** The serve command calls `init_tree()` twice -- once via `new_oneshot()` for config (port/address), once via `new()` for the real session. Both get prefetch. The oneshot is immediately dropped so its prefetch is "wasted", but the overhead is small (~50ms) and the code stays simple.

**Why this is safe:** The prefetch reads files with `std::fs` (no VFS lock). The cache is loaded into VfsInner behind the mutex. The snapshot build then acquires the mutex per-read as before, but each read is a HashMap remove (nanoseconds) instead of a disk syscall. Existing behavior is preserved for cache misses (falls through to backend). Watch registration still happens normally (gated by `watch_enabled`).

`**StdBackend::read_dir()` is eager:** It collects and sorts all directory entries before returning the `ReadDir` iterator (`std_backend.rs:283-296`). The iterator does not hold any reference into the backend. This means `read_dir` results are safe to use from rayon threads after the VFS lock is released.

### Part 2: Parallel Snapshot Build at Directory Level

**Concept:** In `snapshot_dir_no_meta`, process each directory's children in parallel using rayon. With the prefetch cache warm, each rayon thread acquires the VFS mutex briefly for a cache hit (~1us), then parses the file content lock-free (the expensive part). Mutex contention is negligible since critical sections are just HashMap removes.

**Changes to [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs) `snapshot_dir_no_meta()`:**

```rust
// Current (sequential):
for entry in vfs.read_dir(path)? {
    let entry = entry?;
    if !passes_filter_rules(&entry) { continue; }
    if let Some(child) = snapshot_from_vfs(context, vfs, entry.path())? {
        snapshot_children.push(child);
    }
}

// Parallel (with order preservation):
use rayon::prelude::*;

let entries: Vec<_> = vfs.read_dir(path)?
    .filter_map(|e| e.ok())
    .collect();

let results: Vec<anyhow::Result<Option<InstanceSnapshot>>> = entries
    .par_iter()
    .map(|entry| {
        if !passes_filter_rules(entry) {
            return Ok(None);
        }
        snapshot_from_vfs(context, vfs, entry.path())
    })
    .collect();

let mut snapshot_children = Vec::new();
for result in results {
    if let Some(snapshot) = result? {
        snapshot_children.push(snapshot);
    }
}
```

**Why `map` instead of `filter_map`:** rayon's `par_iter().map().collect()` preserves original order (map is an indexed operation). `filter_map` converts to an unindexed iterator where order is implementation-dependent. Using `map` + sequential flatten ensures children appear in the same order as `read_dir` returns them.

**Why this is safe for concurrent VFS access:** Each VFS call (metadata, read, read_dir) independently acquires/releases the Mutex -- no nested locking, so no deadlock risk. `&InstanceContext` is `Sync` (contains `Arc<Vec<PathIgnoreRule>>` + `Vec<SyncRule>`). `&Vfs` is `Sync` (wraps `Mutex<VfsInner>`). `anyhow::Error` is `Send` (required for rayon's `collect::<Vec<Result<_>>>()`).

**Multiple VFS calls per middleware:** Each middleware makes several VFS calls during snapshot building. For example, `lua.rs` calls `read_to_string_lf_normalized` (cache hit), `canonicalize` (backend), and reads companion `.meta.json5` (cache hit). With parallel builds, the cached reads are fast (~1us lock + HashMap remove), while `metadata` and `canonicalize` still serialize on the mutex with real syscalls. This limits maximum parallelism but the file-read savings dominate.

**Recursion:** Since `snapshot_from_vfs` is recursive, a directory with 10 subdirectories kicks off 10 parallel subtrees, each of which parallelizes their own children. rayon's work-stealing keeps all cores busy across the entire tree.

**Rayon thread pool:** The `sourcemap` command configures a global rayon pool with `max(num_cpus, 6)` threads at `src/cli/sourcemap.rs:93`, but this happens AFTER `init_tree()` completes. Our parallel snapshot build runs before that configuration, using rayon's default pool (CPU count). No conflict.

**Environment variable gate:**

```rust
if std::env::var("ATLAS_SEQUENTIAL").is_ok() {
    // Sequential fallback (existing code)
} else {
    // Parallel path
}
```

### Part 3: Parallel RefPathIndex Population

**Changes to [src/rojo_ref.rs](src/rojo_ref.rs) `populate_from_dir()`:**

Current: sequential stack-based walk + `std::fs::read()` per file.

New approach:

1. Walk tree with `walkdir`, collect all `.meta.json5`/`.model.json5` paths (sequential, fast -- just filename checks)
2. Read + parse + extract `Rojo_Ref_*` entries in parallel with rayon. `&RojoTree` is read-only and safe to share across threads (`get_ids_at_path()` and `ref_target_path_from_tree()` are in-memory lookups).
3. Collect `Vec<(String, PathBuf)>` results, then insert into index (sequential)

## Race Condition Analysis

### No risk (by design)

- **VFS data races:** Impossible. `Mutex<VfsInner>` serializes all access. Multiple rayon threads contend briefly on cache lookups (~1us critical sections), which is safe.
- **Deadlock from re-entrant locking:** Impossible. Each VFS call (metadata, read, read_dir) acquires and releases the lock independently. No VFS call internally calls another VFS method. `ReadDir` iterator is fully materialized before lock release.
- **InMemoryFs thread safety:** `InMemoryFs` wraps `Arc<Mutex<InMemoryFsInner>>`, additionally protected by VFS's outer Mutex. Unit tests using InMemoryFs are safe with parallel snapshot building.

### Mitigated risks

- **Child ordering non-determinism:** `par_iter().filter_map().collect()` does NOT guarantee order (filter_map makes the iterator unindexed). We use `par_iter().map().collect()` instead, which preserves input order because `map` on an indexed iterator stays indexed. The sequential flatten step then filters Nones. A determinism test (run N times, compare output) validates this.
- **Prefetch cache stale reads:** If a file is modified between prefetch and snapshot_from_vfs, the cached content is stale. In practice this cannot happen during the few milliseconds of startup. Same time-of-check-time-of-use window exists with sequential reads. No additional risk.
- **Error propagation in parallel:** `par_iter().map().collect::<Vec<Result<_>>>()` collects all results. Errors are propagated sequentially in the flatten step via `?`. First error wins, consistent with sequential behavior.

### Covered by environment variable fallback

`ATLAS_SEQUENTIAL=1` disables both prefetch cache and parallel snapshot building, reverting to the exact current sequential code path. Useful for:

- Debugging if parallelization causes platform-specific issues
- A/B performance comparison
- CI environments requiring deterministic sequential execution

## Testing Strategy

### Existing tests that automatically exercise the new system

All integration tests spawn the CLI binary, which goes through `init_tree()` -> prefetch + parallel snapshot:

- **Build tests** ([tests/tests/build.rs](tests/tests/build.rs)) -- Run `rojo build` on real filesystem fixtures. Exercise full prefetch + parallel snapshot pipeline.
- **Serve tests** ([tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs) `TestServeSession`) -- Spawn `atlas serve` on temp dirs. Exercise prefetch + parallel snapshot + RefPathIndex.
- **Syncback/roundtrip tests** ([tests/tests/syncback.rs](tests/tests/syncback.rs), [tests/tests/syncback_roundtrip.rs](tests/tests/syncback_roundtrip.rs)) -- Full CLI syncback pipeline.
- **Two-way sync tests** ([tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)) -- Serve session + write API.
- **Matching fixture tests** ([tests/tests/matching_fixtures.rs](tests/tests/matching_fixtures.rs)) -- Call `snapshot_from_vfs()` directly with `Vfs::new_default()`. Exercise parallel snapshot building in dir.rs but NOT prefetch (no init_tree call).
- **Unit tests in dir.rs** -- Use `InMemoryFs`. Exercise parallel snapshot building but not prefetch. This is correct -- InMemoryFs files don't exist on the real filesystem.

### New tests to add

1. **VFS prefetch cache unit tests** (in [crates/memofs/src/lib.rs](crates/memofs/src/lib.rs) test module):
  - Cache hit: `set_prefetch_cache` with known path -> `read()` returns cached data
  - Cache depletion: second `read()` of same path falls through to backend (entry was removed on first read)
  - Cache miss: `read()` of path not in cache falls through to backend
  - `read_to_string` uses cache: verify UTF-8 conversion from cached bytes
  - Watch registration: if `watch_enabled`, watches are registered even on cache hits
  - `clear_prefetch_cache`: after clear, all reads go through backend
  - Empty/None cache behaves identically to no cache
2. **Parallel snapshot determinism test** (in [tests/tests/build.rs](tests/tests/build.rs) or new file):
  - Run `snapshot_from_vfs` on a non-trivial fixture 5 times
  - Assert all 5 snapshots are identical (catches ordering bugs from rayon)
  - Use a fixture with multiple children per directory to stress the parallel path
3. **Prefetch + parallel integration sanity** (implicit):
  - All existing insta snapshot tests serve as regression tests. If prefetch returns wrong data or parallel build reorders children, snapshot comparisons fail immediately.

### Tests we do NOT need

- **VFS mutex stress test:** The Mutex is a well-tested primitive. Contention with ~1us critical sections and rayon's thread pool is a solved problem.
- **Prefetch TOCTOU test:** The time-of-check-time-of-use window between prefetch and read is identical to the existing sequential window. No additional test.
- **InMemoryFs concurrency test:** InMemoryFs is protected by both its own Mutex and the VFS's outer Mutex. Double-locked by design.

## What About the File Watcher?

The ChangeProcessor handles individual file events one at a time. Single-file reads don't benefit from parallelization. Two scenarios where deeper parallelism could help:

- **Batch events after `git checkout`**: Many files change at once, processed sequentially. Could batch and parallelize, but adds complexity. Deferred.
- **Rescan after watcher critical error**: Re-snapshots the full tree. Would benefit from prefetch, but rescan is rare. Can be added later.

## Other Sequential I/O (Lower Priority, Not in This Plan)

- **Two-way sync writes** (`api.rs`): Small batches (1-10 files). Not worth the complexity.
- **Build output encoding**: Single file write. Cannot parallelize.
- **Upload encoding**: Single buffer + HTTP upload. Network-bound.
- `**validate_tree()` / `check_tree_freshness()`**: Re-snapshots full tree. Could use prefetch. Secondary benefit, can add later.

## Files Changed


| File                                                             | Change                                                                              |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| [crates/memofs/src/lib.rs](crates/memofs/src/lib.rs)             | Add prefetch cache field, `read_raw()` helper, `set/clear_prefetch_cache()` methods |
| [src/snapshot_middleware/dir.rs](src/snapshot_middleware/dir.rs) | Add rayon `par_iter` for parallel child snapshot building                           |
| [src/serve_session.rs](src/serve_session.rs)                     | Call prefetch before snapshot build in `init_tree()`                                |
| [src/rojo_ref.rs](src/rojo_ref.rs)                               | Parallelize `populate_from_dir()` with rayon                                        |


No changes to VFS backend trait. No new dependencies (rayon, walkdir already present).

## Stack Compatibility Checklist

Verified across the full stack:

- `Vfs` is `Sync` (wraps `Mutex<VfsInner>`, `VfsBackend: Send`)
- `InstanceContext` is `Sync` (contains `Arc<Vec<...>>` + `Vec<SyncRule>`)
- `anyhow::Error` is `Send` (required for rayon error collection)
- `StdBackend::read_dir()` eagerly collects entries before returning (`std_backend.rs:283-296`) -- no lazy iteration holding the lock
- No `vfs.lock()` calls in any snapshot middleware -- only syncback writes use `VfsLock`
- `InMemoryFs` is thread-safe (`Arc<Mutex<...>>`) -- unit tests with parallel dir.rs are safe
- `read_to_string_lf_normalized` routes through `VfsInner::read_to_string` -> `read_raw` -- cache hit for all `.luau` files
- `VfsInner::canonicalize` checks `prefetch_cache.canonical` -- eliminates ~400ms Windows serialization point
- Canonicalize cache safe: only alive during `init_tree()` (no mutations), `ChangeProcessor` always gets fresh backend results after cache clear
- `walkdir` with `.follow_links(true)` matches `StdBackend::read` symlink behavior; handles loop detection
- `Project::load_initial_project` reads via VFS BEFORE prefetch starts -- correct, only reads 1 project file
- Rayon global pool not configured until after `init_tree` completes (sourcemap configures at `sourcemap.rs:93`)
- `ReadDir` iterator fully materialized -- safe to use from rayon threads after lock release
- No stateful side-effects in any snapshot middleware -- all are pure functions of (context, vfs, path)

