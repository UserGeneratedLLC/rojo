---
name: Dedup suffix start at 2
overview: Change the dedup suffix numbering to start at `~2` instead of `~1`, so the sequence is `Something.luau`, `Something~2.luau`, `Something~3.luau` -- consistent with Windows duplicate naming conventions.
todos:
  - id: core-loops
    content: Change `for i in 1..` to `for i in 2..` in both `deduplicate_name_with_ext` and `deduplicate_name` in `src/syncback/file_names.rs`
    status: completed
  - id: unit-tests
    content: Update unit test expectations in `file_names.rs` for generated suffix assertions (~1 -> ~2, ~2 -> ~3)
    status: completed
  - id: integration-tests
    content: Update integration test assertions in `two_way_sync.rs` and `connected_mode.rs`
    status: completed
  - id: fixtures-rename
    content: "Rename test fixture files: connected_slugify, ref_ambiguous_path, UFOWave_matching expected outputs"
    status: completed
  - id: snapshot-tests
    content: Run cargo test + cargo insta review to update snapshot expectations
    status: completed
  - id: docs-update
    content: Update atlas-dedup.mdc examples to reflect ~2 starting suffix
    status: completed
isProject: false
---

# Dedup Suffix: Start at ~2

## Core Change

The only production logic change is in `[src/syncback/file_names.rs](src/syncback/file_names.rs)` -- two `for` loops that generate dedup suffixes:

`**deduplicate_name_with_ext` (line ~297):**

```rust
// Before:
for i in 1.. {
// After:
for i in 2.. {
```

`**deduplicate_name` (line ~315):**

```rust
// Before:
for i in 1.. {
// After:
for i in 2.. {
```

These are the only two places where dedup suffix numbers are generated. All other code (`parse_dedup_suffix`, `compute_cleanup_action`, `build_dedup_name`, cleanup logic in `change_processor.rs`, `api.rs`) operates on whatever numbers exist on disk and needs no changes.

## Backward Compatibility

- `**parse_dedup_suffix**` in `[src/syncback/dedup_suffix.rs](src/syncback/dedup_suffix.rs)` (line ~~53): keep `if n > 0` unchanged. Old filesystems may have `~~1` files; forward sync must still read them correctly.
- `**build_dedup_name**`: takes an explicit number, no change needed.
- **Cleanup rules**: operate on parsed suffix numbers dynamically, work with any starting value.

## Test Updates

### Unit Tests

- `[src/syncback/file_names.rs](src/syncback/file_names.rs)` test module (~~line 389+): update assertions that expect generated `~~1`to`~~2`,`~~ 2`to`~3`, etc. Key tests:` dedup_single_collision`,` dedup_multiple_collisions`,` dedup_skips_taken_suffix`,` dedup_gap_in_suffixes`.
- `[src/syncback/dedup_suffix.rs](src/syncback/dedup_suffix.rs)` test module: `parse_suffix_basic` still tests `~1` parsing (backward compat) -- no change. `build_name_file` references `~1` as explicit input to `build_dedup_name` -- no change (function takes arbitrary numbers). Cleanup tests (`cleanup_group_to_one`, `cleanup_base_deleted_promote_lowest`, etc.) use `~1`/`~2` as existing filesystem state -- no change needed since they test cleanup of pre-existing suffixes.

### Integration Tests

- `[tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs)`: update all generation assertions: `X_Y~1` to `X_Y~2`, `X_Y~2` to `X_Y~3`. Affected tests: `add_two_colliding_instances_deduplicates`, `add_three_colliding_instances_deduplicates`, `delete_deduped_instance_group_to_1_cleanup`, `delete_base_deduped_instance_promotes_lowest`, `delete_middle_deduped_instance_tolerates_gap`, `add_same_named_folders_deduplicates`, `ref_through_deduped_instance`, `ref_path_updated_after_dedup_cleanup`.
- `[tests/tests/connected_mode.rs](tests/tests/connected_mode.rs)`: update tests that interact with generated dedup files. `add_third_collision` expects `Hey_Bro~2` -- should become `Hey_Bro~3`. Tests that read existing fixture files (`edit_dedup_file_with_meta`, `remove_one_of_two_colliding_files`) need fixture updates (see below).

### Test Fixtures (rename files)

- `rojo-test/serve-tests/connected_slugify/src/Hey_Bro~1.luau` -> `Hey_Bro~2.luau`
- `rojo-test/serve-tests/connected_slugify/src/Hey_Bro~1.meta.json5` -> `Hey_Bro~2.meta.json5`
- `rojo-test/serve-tests/ref_ambiguous_path/src/Workspace/DupParent/Child~1.model.json5` -> `Child~2.model.json5` (and update any ref paths inside)
- `rojo-test/syncback-tests/UFOWave_matching/expected/TsunamiWave/`: rename all `~N` files to `~(N+1)` (e.g., `Texture~1` -> `Texture~2`, ..., `Texture~11` -> `Texture~12`)

**Keep unchanged (backward compat reading):**

- `rojo-test/build-tests/dedup_suffix_with_meta/src/Foo~1.`* -- tests that forward sync reads old `~1` files correctly
- `rojo-test/build-tests/tilde_no_meta/src/Foo~1.luau` -- tests literal tilde name (no meta)

### Snapshot Tests

- Run `cargo test` then `cargo insta review` to accept updated snapshot expectations. The `.snap` files in `rojo-test/build-test-snapshots/` and `rojo-test/serve-test-snapshots/` will auto-update.

## Documentation Update

- `[.cursor/rules/atlas-dedup.mdc](.cursor/rules/atlas-dedup.mdc)`: update examples that reference `~1` as generated output (e.g., `"Foo~1.server.luau"` -> `"Foo~2.server.luau"` where it represents generated names; keep `~1` where it represents parseable/legacy names).

