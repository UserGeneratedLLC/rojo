---
name: Two-way sync integration tests
overview: "Add integration tests covering the critical untested two-way sync operations: instance rename, ClassName transitions, non-Source property persistence, removal with meta cleanup, and end-to-end bidirectional sync workflows. Tests validate that the plugin sync pipeline can eventually replace `rojo syncback` for keeping the filesystem in sync with Studio."
todos:
  - id: create-test-file
    content: Create tests/tests/two_way_sync.rs with test module structure, imports, and helper functions
    status: pending
  - id: register-module
    content: Register two_way_sync module in tests/end_to_end.rs
    status: pending
  - id: basic-source-write
    content: "Test: update_source_writes_to_disk — verify Source property write to disk via InstanceUpdate"
    status: pending
  - id: property-persistence
    content: "Tests: update_non_source_property_creates_meta_file + update_properties_for_directory_script_writes_init_meta"
    status: pending
  - id: rename-tests
    content: "Tests: rename_standalone_script + rename_preserves_adjacent_meta_file"
    status: pending
  - id: classname-tests
    content: "Tests: classname_change_standalone_module_to_script + classname_change_directory_script_renames_init_file"
    status: pending
  - id: removal-tests
    content: "Tests: remove_instance_deletes_file_and_meta + remove_directory_instance"
    status: pending
  - id: round-trip
    content: "Test: round_trip_add_modify_rename_remove — end-to-end lifecycle"
    status: pending
  - id: edge-cases
    content: "Tests: property_update_skips_project_node_instances + echo_suppression_prevents_redundant_patches"
    status: pending
  - id: ci-verify
    content: Run /ci to verify all new tests pass alongside existing 606+397 tests
    status: pending
isProject: false
---

# Two-Way Sync Integration Tests

## Intent

The plugin's two-way sync (`POST /api/write`) should eventually supersede the `rojo syncback` command entirely. The goal is continuous bidirectional sync: connect to Studio, accept changes, and work in an external IDE while others work in Studio. These tests validate the core primitives that make this possible.

## Coverage Gaps Addressed

The existing tests cover format transitions (standalone/directory) and stress testing but have **zero coverage** for:

1. **Instance rename** (changed_name) — file/directory rename on disk, adjacent meta file rename
2. **ClassName transitions** (changed_class_name) — script type changes, init file detection in directories
3. **Non-Source property persistence** (changed_properties except Source) — writing to meta files
4. **Instance removal with meta file cleanup** — deleting files plus their adjacent `.meta.json5`
5. **End-to-end round-trip** — add instance, modify it, rename it, verify filesystem at each step

## Test Fixture

Use existing `syncback_write` fixture (`rojo-test/serve-tests/syncback_write/`). It has a minimal project: `ReplicatedStorage` mapped to `src/` with one existing `existing.luau` (ModuleScript). This is ideal for testing write operations without interference from complex file structures.

For rename tests, we also use `syncback_format_transitions` which has directory-format scripts with init files.

## New Test File

Add tests to `tests/tests/two_way_sync.rs` (new file). Register it via `tests/end_to_end.rs`.

All tests follow the established pattern:

- `run_serve_test(fixture_name, |session, _redactions| { ... })`
- Find parent ID via `get_api_rojo()` + `get_api_read()`
- Construct `WriteRequest` with `added`, `updated`, or `removed`
- Call `session.post_api_write()`
- Assert filesystem state

## Tests

### 1. `update_source_writes_to_disk` — Verify the basic Source write path

Send an `InstanceUpdate` with `changed_properties: { Source: "-- new content" }` for the existing `existing.luau`. Verify `fs::read_to_string` returns the new content. This is the most fundamental two-way sync operation.

### 2. `update_non_source_property_creates_meta_file` — Issue 2 coverage

Send an `InstanceUpdate` with a non-Source property (Attributes) for `existing.luau`. Verify:

- `existing.meta.json5` is created adjacent to `existing.luau`
- The meta file contains the attribute value
- `existing.luau` itself is unchanged

### 3. `update_properties_for_directory_script_writes_init_meta` — Issue 2 directory path

Use `syncback_format_transitions` fixture. Send property update for a directory-format script (`DirModuleWithChildren`). Verify `init.meta.json5` is created inside the directory.

### 4. `rename_standalone_script` — Issue 8 rename coverage

Add a new ModuleScript `OldName` via `added`, wait for it to appear on disk. Then send an `InstanceUpdate` with `changed_name: Some("NewName")`. Verify:

- `OldName.luau` is gone
- `NewName.luau` exists with same content

### 5. `rename_preserves_adjacent_meta_file` — Issue 8 meta rename

Add a ModuleScript, send a property update to create its meta file, then rename it. Verify both the `.luau` and `.meta.json5` files are renamed together.

### 6. `classname_change_standalone_module_to_script` — Issue 8 ClassName coverage

Add a ModuleScript `MyModule`, then send `changed_class_name: Some("Script")`. Verify:

- `MyModule.luau` is gone
- `MyModule.server.luau` exists

### 7. `classname_change_directory_script_renames_init_file` — Issue 8 directory ClassName

Use `syncback_format_transitions`. Send `changed_class_name` for `DirModuleWithChildren` (ModuleScript -> Script). Verify:

- Directory still exists
- `init.luau` is gone
- `init.server.luau` exists
- Children still exist

### 8. `remove_instance_deletes_file_and_meta` — Issue 4 removal cleanup

Add a ModuleScript, create its meta file via property update, then remove it via `removed`. Verify both the `.luau` file and `.meta.json5` are deleted.

### 9. `remove_directory_instance` — Issue 4 directory removal

Use `syncback_format_transitions`. Remove a directory-format instance. Verify the entire directory is deleted recursively.

### 10. `round_trip_add_modify_rename_remove` — End-to-end workflow

This test validates the complete lifecycle that makes two-way sync viable as a `syncback` replacement:

1. **Add** a new Script `PlayerHandler` with children
2. Verify directory created on disk
3. **Modify** its Source property
4. Verify file content updated
5. **Rename** it to `GameHandler`
6. Verify directory renamed
7. **Remove** it
8. Verify directory deleted

### 11. `property_update_skips_project_node_instances` — Post-fix coverage

Send property update for a Service (ReplicatedStorage) which has `InstigatingSource::ProjectNode`. Verify it does NOT corrupt the project file. The project file content should be unchanged.

### 12. `echo_suppression_prevents_redundant_patches` — Issue 7 coverage

Add a new instance, then immediately read the message queue. Count the WebSocket messages — there should be exactly one patch notification, not two (which would indicate echo from the VFS picking up the file write).

## Key Implementation Details

- `InstanceUpdate` requires `UstrMap<Option<Variant>>` for `changed_properties`. Import `rbx_dom_weak::{ustr, UstrMap}`.
- Finding existing instance IDs: read tree via `get_api_read(root_id)`, traverse `instances` to find by class/name.
- After `post_api_write`, sleep 200-500ms for change processor to handle VFS events.
- Use `fs::read_to_string` to verify file content, `fs::metadata` to verify existence.
- For rename/ClassName tests, the instance ID in `InstanceUpdate` must match the ID the server assigned (found via `get_api_read` after the initial add).

## File Registration

Add `mod two_way_sync;` to `tests/tests/mod.rs` (or the test harness entry point `tests/end_to_end.rs`).