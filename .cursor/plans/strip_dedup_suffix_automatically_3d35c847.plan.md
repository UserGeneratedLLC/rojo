---
name: Strip dedup suffix automatically
overview: Eliminate unnecessary `.meta.json5` files for dedup'd instances by having forward sync automatically strip `~N` dedup suffixes from filenames when deriving instance names, and having syncback skip writing meta `name` fields for dedup-only cases.
todos:
  - id: strip-fn
    content: Add `strip_dedup_suffix()` to `dedup_suffix.rs` and ensure it is importable from snapshot code
    status: completed
  - id: forward-files
    content: Modify `file_name_for_path()` in `metadata.rs` to strip `~N` dedup suffixes
    status: completed
  - id: forward-dirs
    content: Modify `get_dir_middleware()` in `snapshot_middleware/mod.rs` to strip `~N` from directory names
    status: completed
  - id: syncback-meta
    content: Change `name_for_inst()` needs_meta condition to `needs_slugify` only (remove dedup check)
    status: completed
  - id: change-processor
    content: Update file rename handler in `change_processor.rs` to use `slugified_new_name` for meta name conditions
    status: completed
  - id: update-tests
    content: Update existing tests and fixtures for new dedup-only behavior (no meta name)
    status: completed
  - id: new-tests
    content: "Add new tests: forward sync stripping, edge cases, syncback dedup-only, roundtrip without meta"
    status: completed
isProject: false
---

# Auto-Strip Dedup Suffixes to Eliminate Redundant Meta Files

## Problem

When syncback encounters duplicate instance names (e.g., two instances named "LocalScript"), it creates:

- `LocalScript.local.luau` (first)
- `LocalScript~2.local.luau` (second, with dedup suffix)
- `LocalScript~2.meta.json5` with `name: "LocalScript"` (meta file to recover real name)

The meta file is redundant because `~N` is a well-defined system convention -- the `~` character is forbidden in instance names (always slugified to `_`), so `~N` can only come from the dedup system. Forward sync can strip it automatically.

## Safety Argument

- The `~` character is in `SLUGIFY_CHARS` (`[file_names.rs](src/syncback/file_names.rs)`), so any instance literally named `Foo~2` would be slugified to `Foo_2` during syncback. Natural tilde names cannot produce the `~N` pattern.
- Existing projects with meta files still work: meta `name` field takes priority over filename-derived name (`[meta_file.rs](src/snapshot_middleware/meta_file.rs)` `apply_name`).
- Non-dedup patterns like `~0`, `~abc`, `~` are NOT stripped (only `~N` where N > 0).

## Changes

### 1. Add `strip_dedup_suffix()` utility

Add to `[src/syncback/dedup_suffix.rs](src/syncback/dedup_suffix.rs)` alongside `parse_dedup_suffix`:

```rust
pub fn strip_dedup_suffix(name: &str) -> &str {
    parse_dedup_suffix(name).map_or(name, |(base, _)| base)
}
```

### 2. Forward sync: strip dedup from file names

In `[src/snapshot/metadata.rs](src/snapshot/metadata.rs)`, `SyncRule::file_name_for_path()` (line 330) -- after stripping the middleware extension, also strip `~N`:

```rust
pub fn file_name_for_path<'a>(&self, path: &'a Path) -> anyhow::Result<&'a str> {
    let name = /* existing logic to strip middleware extension */;
    Ok(strip_dedup_suffix(name))
}
```

The return type stays `&'a str` because stripping `~N` returns a sub-slice of the same string.

### 3. Forward sync: strip dedup from directory names

In `[src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs)`, `get_dir_middleware()` (line 116) -- strip `~N` from `dir_name` before returning:

```rust
let dir_name = strip_dedup_suffix(dir_name);
```

Return type stays `&'path str` (sub-slice of the original path).

### 4. Syncback: don't require meta name for dedup-only

In `[src/syncback/file_names.rs](src/syncback/file_names.rs)`, `name_for_inst()` line 79 -- change:

```rust
// Before:
let needs_meta = needs_slugify || deduped_slug != base;
// After:
let needs_meta = needs_slugify;
```

When only dedup happened (no slugification), forward sync will recover the correct name by stripping `~N`.

### 5. Two-way sync: update meta name conditions in change_processor

In `[src/change_processor.rs](src/change_processor.rs)`, the file rename handler (lines 1569, 1581) currently uses `deduped_new_name != *new_name` to decide whether meta name is needed. Change both to `slugified_new_name != *new_name`, matching the directory handler (line 1401) which already does this correctly.

### 6. Module visibility

Ensure `strip_dedup_suffix` is importable from snapshot code. May need to adjust `pub` visibility on the `syncback::dedup_suffix` module, or alternatively place the function in a shared location if cross-module import is problematic.

### 7. Update existing tests and fixtures

- `**src/syncback/file_names.rs` tests:** Update tests like `name_for_inst_dedup_collision` (line 826) and `nfi_clean_name_dedup_still_needs_meta_dir` (line 1175) -- these should now assert `needs_meta = false` for dedup-only cases.
- `**rojo-test/build-tests/dedup_suffix_with_meta/`:** Update fixture to remove now-unnecessary meta files for dedup-only cases.
- `**tests/tests/syncback_roundtrip.rs`:** The `dedup_suffix_with_meta` roundtrip test (line 299) and idempotency test (line 362) may need updated expected output.

### 8. Add new tests

- **Forward sync unit test:** File `Foo~2.luau` without meta -> instance name is `"Foo"`
- **Forward sync unit test:** Directory `Foo~2/` without meta -> instance name is `"Foo"`
- **Forward sync edge cases:** `Foo~0.luau` -> `"Foo~0"`, `Foo~abc.luau` -> `"Foo~abc"` (not valid dedup, kept as-is)
- **Syncback test:** Dedup-only case produces no meta `name` field
- **Syncback test:** Slugify + dedup still produces meta `name` field (e.g., `"Hey/Bro"` -> `Hey_Bro~2.luau` with meta name `"Hey/Bro"`)
- **Roundtrip test:** Dedup'd names without meta files survive build -> syncback -> rebuild

