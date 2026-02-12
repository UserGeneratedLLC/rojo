---
name: Connected Mode Test Expansion
overview: Expand the serve test suite to cover multi-step connected mode scenarios -- format transitions, init file operations, directory conversions, slugify/dedup forward-sync, meta name field lifecycle, and sequential change sequences -- with full WebSocket patch verification AND round-trip identity verification. 53 tests total across 8 phases.
todos:
  - id: fixtures
    content: "Create 4 new test fixtures in rojo-test/serve-tests/: connected_scripts, connected_init, connected_models, connected_slugify"
    status: completed
  - id: harness
    content: "Add harness helpers to tests/rojo_test/serve_util.rs: get_message_cursor for chaining multi-step tests, fresh_rebuild_read for round-trip verification"
    status: completed
  - id: phase1-format-transitions
    content: "Implement Phase 1: Format transition tests (standalone<->directory, init type changes) with round-trip verification -- 8 tests"
    status: completed
  - id: phase2-multi-step
    content: "Implement Phase 2: Multi-step connected sequence tests (create/edit/rename/delete lifecycle) with round-trip verification -- 5 tests"
    status: completed
  - id: phase3-init
    content: "Implement Phase 3: Init file operation tests during connected mode with round-trip verification -- 4 tests"
    status: completed
  - id: phase4-meta
    content: "Implement Phase 4: Property/meta file change tests -- 4 tests"
    status: completed
  - id: phase5-models
    content: "Implement Phase 5: Model JSON file change tests -- 5 tests"
    status: completed
  - id: phase6-cursor
    content: "Implement Phase 6: Cursor tracking and reconnection tests -- 3 tests"
    status: completed
  - id: phase7-mixed
    content: "Implement Phase 7: Mixed API + filesystem verification tests -- 2 tests"
    status: completed
  - id: phase8-slugify-fixture
    content: Create connected_slugify fixture with slugified files, dedup ~1 files, clean files, and directory-format slugified instances
    status: completed
  - id: phase8-slugify-forward
    content: "Implement Phase 8a: Slugify forward-sync tests -- editing/creating/deleting slugified files and verifying patches show real names from meta -- 6 tests"
    status: completed
  - id: phase8-dedup-forward
    content: "Implement Phase 8b: Dedup (~N suffix) forward-sync tests -- collision handling, ~1 with and without meta, add/remove colliding files -- 5 tests"
    status: completed
  - id: phase8-slugdir
    content: "Implement Phase 8c: Slugified directory format tests -- init type changes, child adds inside slugified dirs -- 3 tests"
    status: completed
  - id: phase8-meta-name-lifecycle
    content: "Implement Phase 8d: Meta name field lifecycle tests -- create/update/delete meta name, model.json5 name field -- 5 tests"
    status: completed
  - id: phase8-slug-multistep
    content: "Implement Phase 8e: Multi-step slugify scenarios with round-trip verification -- slug rename chains, collision evolution -- 3 tests"
    status: completed
  - id: run-and-review
    content: Run cargo test, cargo insta review, accept snapshots, verify clean pass
    status: completed
isProject: false
---

# Connected Mode Test Expansion

## Quality Standard

This test expansion is held to the same standard as the [slugify branch audit](audit_slugify_branch_e913b7dc.plan.md):

> **Round-trip identity**: Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree** -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

Every test in this plan must verify not just "did the WebSocket patch look right" but also "if I killed the server and rebuilt from the filesystem right now, would I get the same tree?" This is verified by starting a fresh `rojo serve` on the same project directory after changes and comparing the resulting tree.

---

## Problem

The current test suite has a major asymmetry:

- `**two_way_sync.rs**` (plugin -> filesystem via API): 119 tests, covering stress, format transitions, encoded names, concurrent ops, failure recovery
- `**serve.rs**` (filesystem -> plugin via WebSocket patches): 22 tests, each making only ONE change. No multi-step sequences, no format transitions, no directory conversions, no round-trip verification

Connected mode means the plugin is connected and receiving patches over time as the filesystem evolves. The `serve.rs` tests only verify a single atomic change, never simulating a session where the tree evolves through multiple states. They also never verify the round-trip invariant -- that the filesystem state after changes would produce an identical tree if rebuilt from scratch.

---

## Architecture of a Connected Mode Test

Each test follows this pattern in [tests/tests/serve.rs](tests/tests/serve.rs):

```rust
run_serve_test("fixture_name", |session, mut redactions| {
    // 1. Snapshot initial state
    let info = session.get_api_rojo().unwrap();
    let root_id = info.root_instance_id;
    let read = session.get_api_read(root_id).unwrap();
    assert_yaml_snapshot!("test_initial", read.intern_and_redact(&mut redactions, root_id));

    // 2. Change 1 -> verify patch + state
    let packet = session.recv_socket_packet(SocketPacketType::Messages, 0, || {
        fs::write(&path, "new content").unwrap();
    }).unwrap();
    assert_yaml_snapshot!("test_patch1", packet.intern_and_redact(&mut redactions, ()));
    let read = session.get_api_read(root_id).unwrap();
    assert_yaml_snapshot!("test_state1", read.intern_and_redact(&mut redactions, root_id));

    // 3. Change 2 -> verify patch + state (using cursor from patch 1)
    let cursor = get_message_cursor(&packet);
    let packet = session.recv_socket_packet(SocketPacketType::Messages, cursor, || {
        fs::rename(&old, &new).unwrap();
    }).unwrap();
    assert_yaml_snapshot!("test_patch2", packet.intern_and_redact(&mut redactions, ()));

    // 4. Round-trip verification: fresh rebuild must match live tree
    let final_read = session.get_api_read(root_id).unwrap();
    let fresh_read = fresh_rebuild_read(session.path());
    assert_trees_match(&final_read, &fresh_read);
});
```

The critical addition over existing serve.rs tests:

- **Multi-step sequences** with cursor chaining
- **Round-trip verification** at the end -- a fresh `rojo serve` on the same filesystem must produce the same instance tree

Key files involved:

- [tests/tests/serve.rs](tests/tests/serve.rs) -- where new tests go
- [tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs) -- test harness
- [rojo-test/serve-tests/](rojo-test/serve-tests/) -- test fixtures
- [rojo-test/serve-test-snapshots/](rojo-test/serve-test-snapshots/) -- insta snapshots

---

## Harness Changes

### `get_message_cursor` -- cursor extraction for chaining

[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs) needs a helper to extract `messageCursor` from a `SocketPacket` for chaining multi-step tests:

```rust
/// Extract the message cursor from a SocketPacket for use in subsequent subscriptions.
pub fn get_message_cursor(packet: &SocketPacket) -> u32 {
    match &packet.body {
        SocketPacketBody::Messages(msg) => msg.message_cursor,
        _ => panic!("Expected Messages packet"),
    }
}
```

### `fresh_rebuild_read` -- round-trip identity verification

This is the core addition for enforcing the round-trip standard. After all changes, start a **fresh** `rojo serve` on the same project directory and read the full tree. If the live DOM and the fresh rebuild disagree, the filesystem state is inconsistent.

```rust
/// Start a fresh rojo serve on the given project path, read the full tree,
/// and return a normalized representation for comparison.
///
/// This verifies the round-trip invariant: the filesystem state written by
/// the live session, when read from scratch, must produce the same tree.
pub fn fresh_rebuild_read(project_path: &Path) -> NormalizedTree {
    // Find the project file in the directory
    let project_file = find_project_file(project_path);

    let port = get_port_number();
    let port_string = port.to_string();
    let working_dir = get_working_dir_path();

    let mut process = Command::new(ROJO_PATH)
        .args(["serve", project_file.to_str().unwrap(), "--port", &port_string])
        .current_dir(working_dir)
        .stderr(Stdio::piped())
        .spawn()
        .expect("Couldn't start fresh Rojo for round-trip check");

    let kill_guard = KillOnDrop(process);

    // Wait for fresh server to come online (same backoff as TestServeSession)
    let info = wait_for_server(port);
    let read = get_read(port, info.root_instance_id);

    // Normalize: strip Refs (non-deterministic), sort children, extract
    // (name, class, properties, hierarchy) tuples for structural comparison
    normalize_tree(&read)
}
```

### `NormalizedTree` -- structural comparison ignoring Refs

Since two separate `rojo serve` sessions produce different `Ref` values, we need structural comparison. A `NormalizedTree` is a recursive structure of `(name, class_name, sorted_properties, sorted_children)` that can be compared with `==` or snapshotted.

```rust
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct NormalizedInstance {
    pub name: String,
    pub class_name: String,
    pub properties: BTreeMap<String, String>,  // property name -> debug representation
    pub children: Vec<NormalizedInstance>,       // sorted by (name, class_name)
}
```

### `assert_round_trip` -- convenience assertion

```rust
/// Assert that the live session tree matches a fresh rebuild from the filesystem.
pub fn assert_round_trip(session: &TestServeSession, root_id: Ref) {
    let live_read = session.get_api_read(root_id).unwrap();
    let live_tree = normalize_read_response(&live_read, root_id);

    let fresh_tree = fresh_rebuild_read(session.path());

    assert_eq!(
        live_tree, fresh_tree,
        "Round-trip identity violation: live tree and fresh rebuild differ. \
         The filesystem state does not faithfully represent the instance tree."
    );
}
```

---

## New Test Fixtures

### `rojo-test/serve-tests/connected_scripts/`

Fixture with both standalone and directory format scripts, covering all three script types:

```
connected_scripts/
  default.project.json5    # name: "connected_scripts", tree: { ReplicatedStorage: { $path: "src" } }
  src/
    standalone.luau         # ModuleScript (standalone format)
    server.server.luau      # Script (standalone format)
    local.client.luau       # LocalScript (standalone format)
    DirModule/
      init.luau             # ModuleScript (directory format)
      ChildA.luau           # child ModuleScript
      ChildB.luau           # child ModuleScript
    DirScript/
      init.server.luau      # Script (directory format)
      Handler.luau          # child ModuleScript
    DirLocal/
      init.local.luau       # LocalScript (directory format)
      Helper.luau           # child ModuleScript
```

### `rojo-test/serve-tests/connected_init/`

Focused init file fixture for init-specific tests:

```
connected_init/
  default.project.json5    # name: "connected_init", tree: { ReplicatedStorage: { $path: "src" } }
  src/
    init.luau               # Root is a ModuleScript
    Child.luau              # A child module
```

### `rojo-test/serve-tests/connected_models/`

Model file fixture:

```
connected_models/
  default.project.json5    # name: "connected_models", tree: { ReplicatedStorage: { $path: "src" } }
  src/
    SimpleModel.model.json5 # { "className": "Configuration", "properties": { "Name": "SimpleModel" } }
    DirModel/
      init.meta.json5       # { "className": "Folder" }
      Child.luau            # child ModuleScript
```

---

## New Tests -- Organized by Category

### Phase 1: Format Transitions (filesystem-driven, patch + round-trip verified)

Highest priority. Tests what the plugin receives when files transform between standalone and directory formats, AND verifies the filesystem is consistent after each transformation.


| #   | Test Name                                 | What It Tests                                                                                                                                                                                                          | Round-Trip |
| --- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 1   | `standalone_to_directory_add_child`       | Create `ChildNew.luau` next to `standalone.luau`. The snapshot system should convert `standalone.luau` -> `standalone/init.luau` + `standalone/ChildNew.luau`. Verify the patch shows the correct tree transformation. | Yes        |
| 2   | `directory_to_standalone_remove_children` | Remove `ChildA.luau` and `ChildB.luau` from `DirModule/`. With no children left, verify whether `DirModule/init.luau` collapses to `DirModule.luau`. Verify patch.                                                     | Yes        |
| 3   | `standalone_to_directory_and_back`        | Multi-step: (a) add child next to standalone script (standalone->dir), (b) remove child (dir->standalone). Two patches verified, plus round-trip after each step.                                                      | Yes        |
| 4   | `init_type_change_module_to_server`       | Rename `DirModule/init.luau` -> `DirModule/init.server.luau`. Verify patch contains ClassName change from ModuleScript to Script.                                                                                      | Yes        |
| 5   | `init_type_change_server_to_local`        | Rename `DirScript/init.server.luau` -> `DirScript/init.local.luau`. Verify ClassName change from Script to LocalScript.                                                                                                | Yes        |
| 6   | `init_type_change_cycle`                  | Multi-step: `init.luau` -> `init.server.luau` -> `init.local.luau` -> `init.luau`. Three patches, three state snapshots, round-trip at each step.                                                                      | Yes        |
| 7   | `init_delete_children_survive`            | Delete `DirModule/init.luau` while `ChildA.luau` and `ChildB.luau` remain. Verify children survive and parent becomes Folder (or whatever the correct behavior is).                                                    | Yes        |
| 8   | `init_create_in_existing_folder`          | Given a folder with only children (no init file), create `init.luau`. Verify the Folder instance becomes a ModuleScript.                                                                                               | Yes        |


**Important for tests 1-2**: The snapshot middleware may or may not automatically convert between standalone and directory format when children are added/removed via the filesystem. If it doesn't (i.e., the conversion only happens through the API/change processor), the test should document that and verify the actual behavior.

### Phase 2: Multi-Step Connected Sequences

Core connected mode pattern: the tree evolves through multiple states. Each intermediate state is verified.


| #   | Test Name                            | What It Tests                                                                                                                                                                         | Round-Trip   |
| --- | ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------ |
| 9   | `create_edit_rename_delete_sequence` | Create `src/NewScript.luau` -> edit its contents -> rename to `Renamed.luau` -> delete it. Four patches, four state snapshots. Verify each patch contains exactly the expected delta. | Yes (at end) |
| 10  | `directory_lifecycle`                | Create `src/NewDir/` -> add `init.luau` + `Child.luau` -> edit `init.luau` -> remove `Child.luau` -> remove directory. Full directory lifecycle with 5 patches.                       | Yes (at end) |
| 11  | `multi_file_sequential_edits`        | Edit `standalone.luau`, then `server.server.luau`, then `local.client.luau`. Verify each patch only contains the changed file (no cross-contamination).                               | No           |
| 12  | `add_multiple_files_sequentially`    | Add 3 new `.luau` files one at a time. Verify each patch adds exactly one instance with correct name and class.                                                                       | Yes (at end) |
| 13  | `remove_multiple_files_sequentially` | Remove 3 files one at a time. Verify each patch removes exactly one instance.                                                                                                         | No           |


### Phase 3: Init File Operations (connected mode)


| #   | Test Name                    | What It Tests                                                                                                                                         | Round-Trip |
| --- | ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 14  | `edit_init_then_edit_child`  | Edit `DirModule/init.luau`, then edit `DirModule/ChildA.luau`. Two separate patches, each containing only the changed instance's Source property.     | No         |
| 15  | `replace_init_file_type`     | Delete `DirModule/init.luau`, then create `DirModule/init.server.luau`. Verify the ClassName changes from ModuleScript to Script, children preserved. | Yes        |
| 16  | `add_init_to_plain_folder`   | Start with a directory containing only children (create by removing init from fixture). Add `init.luau`. Verify Folder -> ModuleScript.               | Yes        |
| 17  | `delete_init_from_directory` | Delete init file from directory with children. Verify parent becomes Folder, children survive.                                                        | Yes        |


### Phase 4: Property/Meta Changes (connected mode)


| #   | Test Name                | What It Tests                                                                                                                             | Round-Trip |
| --- | ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 18  | `adjacent_meta_creation` | Create `standalone.meta.json5` with `{ "properties": { "Disabled": true } }` next to `standalone.luau`. Verify property appears in patch. | Yes        |
| 19  | `adjacent_meta_edit`     | Edit existing `.meta.json5` to change a property value. Verify update patch.                                                              | No         |
| 20  | `adjacent_meta_delete`   | Delete `.meta.json5`. Verify properties revert to defaults.                                                                               | Yes        |
| 21  | `init_meta_creation`     | Create `DirModule/init.meta.json5` with properties. Verify directory instance gains properties.                                           | Yes        |


### Phase 5: Model File Changes (connected mode)


| #   | Test Name                    | What It Tests                                                                              | Round-Trip |
| --- | ---------------------------- | ------------------------------------------------------------------------------------------ | ---------- |
| 22  | `model_json_edit_properties` | Edit `SimpleModel.model.json5` to change/add properties. Verify property changes in patch. | Yes        |
| 23  | `model_json_add_children`    | Edit model JSON to add child instances. Verify additions in patch.                         | Yes        |
| 24  | `model_json_remove_children` | Edit model JSON to remove child instances. Verify removals in patch.                       | No         |
| 25  | `model_json_create`          | Create new `NewModel.model.json5`. Verify instance addition patch.                         | Yes        |
| 26  | `model_json_delete`          | Delete `SimpleModel.model.json5`. Verify instance removal patch.                           | No         |


### Phase 6: Cursor Tracking and Reconnection


| #   | Test Name                       | What It Tests                                                                                                              | Round-Trip |
| --- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 27  | `cursor_advances_correctly`     | Make 3 filesystem changes. After each, verify the cursor returned in the WebSocket packet increments monotonically.        | No         |
| 28  | `reconnect_with_old_cursor`     | Make 2 changes with cursor advancement. Reconnect with cursor 0. Verify ALL patches are received in the catch-up response. | No         |
| 29  | `reconnect_with_partial_cursor` | Make 3 changes. Reconnect with cursor from after change 1. Verify only patches 2+3 are received.                           | No         |


### Phase 7: Mixed API + Filesystem Verification


| #   | Test Name                          | What It Tests                                                                                                                                                                              | Round-Trip |
| --- | ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------- |
| 30  | `api_write_then_filesystem_change` | POST `/api/write` to update Source, then edit a different file on disk. Verify only the filesystem change produces a WebSocket patch (echo suppression working).                           | Yes        |
| 31  | `filesystem_change_then_api_write` | Edit file on disk, receive patch. Then POST `/api/write` on a different instance. Verify the API write produces the correct filesystem change and does NOT echo back as a duplicate patch. | Yes        |


### Phase 8: Slugify / Dedup / Meta Name (connected mode, round-trip critical)

This is the most critical phase for round-trip identity. The slugify system replaces forbidden filesystem chars with `_`, deduplicates collisions with `~1`/`~2` suffixes, and stores the real instance name in `.meta.json5` / `.model.json5` `name` fields. If ANY part of this chain is broken in the forward-sync direction (filesystem -> plugin patches), instance names are silently corrupted.

The existing `two_way_sync.rs` has 17+ tests for slugify/dedup via the API write path (plugin -> filesystem). **None** of those verify the forward-sync direction: what WebSocket patch does the plugin receive when slugified files are changed on the filesystem? This phase fills that gap completely.

#### New Fixture: `rojo-test/serve-tests/connected_slugify/`

Richer than `syncback_encoded_names` -- includes dedup collisions and directory-format slugified instances:

```
connected_slugify/
  default.project.json5    # name: "connected_slugify", tree: { ReplicatedStorage: { $path: "src" } }
  src/
    # Clean-named files (no meta needed)
    Normal.luau                     # ModuleScript "Normal"
    CleanScript.server.luau         # Script "CleanScript"

    # Slugified files (meta has real name)
    What_Module.luau                # ModuleScript, meta: "What?Module"
    What_Module.meta.json5          # { "name": "What?Module" }
    Key_Script.server.luau          # Script, meta: "Key:Script"
    Key_Script.meta.json5           # { "name": "Key:Script" }

    # Dedup collision: two instances whose slugs both = "Hey_Bro"
    Hey_Bro.luau                    # ModuleScript "Hey_Bro" (natural name, no meta)
    Hey_Bro~1.luau                  # ModuleScript, meta: "Hey/Bro"
    Hey_Bro~1.meta.json5            # { "name": "Hey/Bro" }

    # Model JSON with name field
    What_Model.model.json5          # { "name": "What?Model", "className": "Configuration" }

    # Slugified directory format
    Slug_Dir/                       # Directory, meta: "Slug:Dir"
      init.luau                     # ModuleScript init
      init.meta.json5               # { "name": "Slug:Dir" }
      DirChild.luau                 # child ModuleScript
```

#### Phase 8a: Slugify Forward-Sync (filesystem -> plugin patches)

Tests that editing/creating/deleting slugified files produces patches with correct **real** instance names (from meta), not filesystem names (slugs).


| #   | Test Name                             | What It Tests                                                                                                                                                                                                                            | Round-Trip                                         |
| --- | ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------- |
| 32  | `edit_slugified_script_source`        | Edit `What_Module.luau` content. Verify WebSocket patch contains `changedProperties.Source` for instance named `What?Module` (NOT `What_Module`). The meta `name` field must be respected during forward-sync.                           | Yes                                                |
| 33  | `edit_slugified_server_script_source` | Edit `Key_Script.server.luau` content. Verify patch contains Source change for instance named `Key:Script`. Same principle, different script type.                                                                                       | No                                                 |
| 34  | `create_new_slugified_file_with_meta` | Create `New_Slug.luau` + `New_Slug.meta.json5` with `{"name": "New/Slug"}`. Verify the patch adds an instance named `New/Slug` (not `New_Slug`).                                                                                         | Yes                                                |
| 35  | `delete_slugified_file_and_meta`      | Delete both `What_Module.luau` and `What_Module.meta.json5`. Verify the patch removes exactly one instance (the one named `What?Module`).                                                                                                | No                                                 |
| 36  | `edit_only_meta_name_field`           | Edit `What_Module.meta.json5` to change `{"name": "What?Module"}` -> `{"name": "What*Module"}`. Verify the patch contains a name change (or re-snapshot). The slug stays the same but the real name changes. This is the "Foo/Bar -> Foo | Bar, both slugify to Foo_Bar" case from the audit. |
| 37  | `delete_meta_name_reverts_to_stem`    | Delete `What_Module.meta.json5` (but leave `What_Module.luau`). Verify the patch shows the instance name changing from `What?Module` to `What_Module` (the filename stem becomes the name when no meta exists).                          | Yes                                                |


#### Phase 8b: Dedup (~N Suffix) Forward-Sync

Tests that `~N` suffixed files are handled correctly: the suffix is just part of the filename stem, the real name comes from meta. Without meta, the full stem (including `~N`) IS the instance name.


| #   | Test Name                           | What It Tests                                                                                                                                                                                                              | Round-Trip |
| --- | ----------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 38  | `edit_dedup_file_with_meta`         | Edit `Hey_Bro~1.luau` content. Verify patch shows Source change for instance named `Hey/Bro` (from meta), not `Hey_Bro~1`.                                                                                                 | Yes        |
| 39  | `dedup_file_without_meta_uses_stem` | Create `Foo~1.luau` with NO meta file. Verify the instance name in the patch is literally `Foo~1` -- the tilde is NOT parsed as a dedup marker during forward-sync. This is the backward compatibility guarantee.          | Yes        |
| 40  | `add_second_colliding_file`         | Start with `Hey_Bro.luau` (name: `Hey_Bro`). Create `Hey_Bro~1.luau` + `Hey_Bro~1.meta.json5` with `{"name": "Hey:Bro"}`. Verify the patch adds a NEW instance named `Hey:Bro` as a sibling, without disturbing `Hey_Bro`. | Yes        |
| 41  | `remove_one_of_two_colliding_files` | Delete `Hey_Bro~1.luau` + `Hey_Bro~1.meta.json5`. Verify the patch removes exactly the instance named `Hey/Bro`, leaving `Hey_Bro` untouched.                                                                              | Yes        |
| 42  | `add_third_collision`               | With `Hey_Bro.luau` and `Hey_Bro~1.luau` already present, create `Hey_Bro~2.luau` + `Hey_Bro~2.meta.json5` with `{"name": "Hey*Bro"}`. Verify patch adds instance named `Hey*Bro`. All three coexist.                      | Yes        |


#### Phase 8c: Slugified Directory Format

Tests that directory-format instances with slugified names work correctly during connected mode. The real name is in `init.meta.json5`.


| #   | Test Name                                 | What It Tests                                                                                                                                                       | Round-Trip |
| --- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| 43  | `edit_init_in_slugified_directory`        | Edit `Slug_Dir/init.luau` content. Verify patch shows Source change for instance named `Slug:Dir` (from `init.meta.json5`), not `Slug_Dir`.                         | Yes        |
| 44  | `init_type_change_in_slugified_directory` | Rename `Slug_Dir/init.luau` -> `Slug_Dir/init.server.luau`. Verify ClassName changes from ModuleScript to Script. Instance name must remain `Slug:Dir` (from meta). | Yes        |
| 45  | `add_child_to_slugified_directory`        | Create `Slug_Dir/NewChild.luau`. Verify patch adds a new ModuleScript child under the instance named `Slug:Dir`.                                                    | Yes        |


#### Phase 8d: Meta Name Field Lifecycle (connected mode)

Tests the full lifecycle of the meta `name` field -- the bridge between filesystem names and real instance names. Any bug here silently corrupts names.


| #   | Test Name                         | What It Tests                                                                                                                                                                                          | Round-Trip |
| --- | --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------- |
| 46  | `add_meta_name_to_clean_file`     | `Normal.luau` has no meta (name = `Normal`). Create `Normal.meta.json5` with `{"name": "Nor/mal"}`. Verify the patch shows the instance name changing from `Normal` to `Nor/mal`.                      | Yes        |
| 47  | `update_meta_name_field`          | `What_Module.meta.json5` has `{"name": "What?Module"}`. Change to `{"name": "What:Module"}`. Verify the patch shows name change. The slug (`What_Module`) doesn't change, only the real name does.     | Yes        |
| 48  | `meta_name_with_other_properties` | Create meta file with BOTH `name` AND `$properties`: `{"name": "Real/Name", "$properties": {"Disabled": true}}`. Verify both the name override AND the property appear in the patch.                   | Yes        |
| 49  | `model_json_name_field_edit`      | Edit `What_Model.model.json5` to change `"name": "What?Model"` -> `"name": "What:Model"`. Verify name change in patch. The `name` field in `.model.json5` serves the same purpose as in `.meta.json5`. | Yes        |
| 50  | `model_json_remove_name_field`    | Edit `What_Model.model.json5` to remove the `"name"` field entirely. Verify instance name reverts from `What?Model` to `What_Model` (derived from filename stem).                                      | Yes        |


#### Phase 8e: Multi-Step Slugify Scenarios (round-trip proofs)

Multi-step sequences that exercise the slugify system through state transitions, with round-trip verification at each step. These are the highest-value tests for catching subtle bugs.


| #   | Test Name                               | What It Tests                                                                                                                                                                                                                                                                                                                                                       | Round-Trip                                                                              |
| --- | --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| 51  | `slugify_rename_chain`                  | Multi-step: (1) Edit `What_Module.luau` content, (2) Update meta name `What?Module` -> `What*Module`, (3) Delete meta (name reverts to `What_Module`), (4) Re-create meta with `{"name": "What/Module"}`. Four patches, four state snapshots. Round-trip verified at each step.                                                                                     | Yes (each step)                                                                         |
| 52  | `collision_evolution`                   | Multi-step: (1) Add `Hey_Bro~2.luau` + meta `{"name": "Hey*Bro"}` (third collision), (2) Delete `Hey_Bro~1.luau` + meta (remove `Hey/Bro`), (3) Add `Hey_Bro~1.luau` + meta with different name `{"name": "Hey                                                                                                                                                      | Bro"}`(reuse the`~1` slot). Three patches. Verify correct instances exist at each step. |
| 53  | `clean_to_slug_to_clean_via_filesystem` | Multi-step: (1) `Normal.luau` exists (clean name). Add `Normal.meta.json5` with `{"name": "Nor:mal"}` -- name changes to `Nor:mal`. (2) Delete the meta -- name reverts to `Normal`. (3) Rename `Normal.luau` -> `Has_Slash.luau` + create `Has_Slash.meta.json5` with `{"name": "Has/Slash"}` -- completely different slug+name. Round-trip verified at each step. | Yes (each step)                                                                         |


---

## Execution Order

1. **Fixtures**: Create 4 new fixture directories: `connected_scripts`, `connected_init`, `connected_models`, `connected_slugify`
2. **Harness**: Add `get_message_cursor`, `NormalizedTree`/`NormalizedInstance`, `fresh_rebuild_read`, `assert_round_trip` to [tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)
3. **Phase 1** (format transitions): Highest value, most likely to catch bugs in the snapshot pipeline
4. **Phase 2** (multi-step sequences): Core connected mode pattern, validates cursor chaining
5. **Phase 3** (init file ops): Critical for directory handling correctness
6. **Phases 4-7**: Property/meta, model JSON, cursor tracking, mixed API+filesystem
7. **Phase 8a-e** (slugify/dedup): 22 tests covering the entire forward-sync direction of the name system, with round-trip proofs
8. **Run**: `cargo test`, then `cargo insta review` to accept snapshots
9. **Verify**: All tests pass on a clean run, no snapshot drift

---

## Notes on Test Design

### Filesystem events are OS-dependent

Some operations (like rename) may fire different events on macOS (kqueue/FSEvents) vs Windows (ReadDirectoryChangesW) vs Linux (inotify). Tests that depend on exact patch structure may need `#[cfg_attr(target_os = "macos", ignore)]` annotations if event ordering differs. The existing `ref_properties_remove` test already does this.

### The snapshot system may not auto-convert formats

When adding a child file next to a standalone script via the filesystem, the snapshot system may NOT automatically convert to directory format (that conversion might only happen through the change processor during API writes). Tests should verify and document the actual behavior rather than assuming conversion happens.

### Echo suppression is critical for mixed tests

Phase 7 tests depend on echo suppression working correctly. The API write path suppresses VFS events for paths it wrote, preventing feedback loops. If echo suppression is broken, these tests will see duplicate patches.