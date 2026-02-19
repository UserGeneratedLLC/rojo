---
name: Luau-style ref paths
overview: "Migrate Rojo_Ref_* attribute paths from bare absolute paths to Luau require-by-string style: @self/, ./, ../, and @game/. Prefer relative paths for maximum copy/paste portability. Update all syncback, forward sync, and two-way sync codepaths, plus comprehensive tests."
todos:
  - id: core-compute
    content: Add compute_relative_ref_path() to rojo_ref.rs + unit tests for all prefix selection scenarios
    status: completed
  - id: resolve-fn
    content: Add resolve_ref_path() to tree.rs that handles @game/, @self/, ./, ../ prefixes and .. mid-path
    status: completed
  - id: forward-sync
    content: Update patch_compute.rs and patch_apply.rs to pass source_ref when resolving Rojo_Ref_* paths
    status: completed
  - id: syncback-placeholders
    content: Redesign placeholder system in ref_properties.rs to encode (source, target) pairs, compute relative paths in post-processing
    status: completed
  - id: syncback-final
    content: Update fs_snapshot.rs fix_ref_paths and syncback/mod.rs post-processing to produce relative substitutions
    status: completed
  - id: two-way-write
    content: Update api.rs syncback_updated_properties and added_paths to write relative ref paths
    status: completed
  - id: change-processor
    content: Rewrite update_ref_paths_after_rename in change_processor.rs to resolve-then-recompute instead of string prefix replacement
    status: completed
  - id: ref-path-index
    content: Change RefPathIndex.populate_from_dir to populate_from_tree, store resolved absolute paths, update serve_session.rs call site
    status: completed
  - id: update-existing-tests
    content: Fix all existing test fixtures and assertions to use new prefix format
    status: completed
  - id: new-test-compute
    content: Add comprehensive compute_relative_ref_path unit tests (all scenarios from plan)
    status: completed
  - id: new-test-resolve
    content: Add comprehensive resolve_ref_path unit tests (all prefix types, .., edge cases)
    status: completed
  - id: new-test-roundtrip
    content: Add syncback round-trip integration tests with relative refs (@self, ./, ../, @game)
    status: completed
  - id: new-test-twoway
    content: Add two-way sync integration tests (ref writes, renames, dedup cleanup with relative paths)
    status: completed
  - id: new-test-edge
    content: Add edge case tests (cross-service, ancestor refs, slugified names, dedup suffixes, same-batch add+ref)
    status: completed
  - id: cleanup
    content: Remove dead escape/unescape functions, update documentation comments throughout
    status: completed
isProject: false
---

# Luau-Style Ref Paths

## Current State

`Rojo_Ref_*` attributes store bare absolute paths from DataModel's children:

```json5
{ "Rojo_Ref_PrimaryPart": "Workspace/TestModel/Part1.model.json5" }
```

## Target State

Use Luau require-by-string semantics with four prefix types:

```json5
// Descendant of source instance
{ "Rojo_Ref_PrimaryPart": "@self/Part1.model.json5" }

// Sibling (child of parent)
{ "Rojo_Ref_Value": "./Sibling.model.json5" }

// Navigate up then down (within same service)
{ "Rojo_Ref_Attachment0": "../Attachments/Att1.model.json5" }

// Cross-service or target is ancestor of source
{ "Rojo_Ref_Value": "@game/ReplicatedStorage/Shared/Module.luau" }
```

## Prefix Selection Algorithm

Given source and target absolute ref paths:

1. **target == source** --> `@self` (self-reference, rare)
2. **target is descendant of source** --> `@self/<relative_path>`
3. **LCA is DataModel root** (cross-service, `common_prefix_len == 0`) --> `@game/<target_abs>`
4. **target is ancestor of source** (`remaining` is empty after LCA) --> `@game/<target_abs>`
5. **Otherwise** (same service, different branches):
  - `ups == 1` (LCA is parent) --> `./<remaining>`
  - `ups >= 2` --> `"../" * (ups - 1)` + `<remaining>`

This maximizes portability: subtrees can be copied/moved within a service without breaking refs.

**Worked examples:**


| Source                 | Target                          | LCA           | Result                                |
| ---------------------- | ------------------------------- | ------------- | ------------------------------------- |
| `Workspace/A/Script`   | `Workspace/A/Script/Child`      | (descendant)  | `@self/Child`                         |
| `Workspace/A/Script`   | `Workspace/A/Part.model.json5`  | `Workspace/A` | `./Part.model.json5`                  |
| `Workspace/A/Script`   | `Workspace/B/Part.model.json5`  | `Workspace`   | `../B/Part.model.json5`               |
| `Workspace/A/B/Script` | `Workspace/C/Part.model.json5`  | `Workspace`   | `../../C/Part.model.json5`            |
| `Workspace/A/Script`   | `ReplicatedStorage/Module.luau` | root          | `@game/ReplicatedStorage/Module.luau` |
| `Workspace/A/B/Script` | `Workspace/A`                   | (ancestor)    | `@game/Workspace/A`                   |


## Resolution Algorithm

Per the [Roblox require-by-string docs](https://create.roblox.com/docs/reference/engine/globals/LuaGlobals#require):

```
resolve_ref_path(path, source_ref, tree):
    @game/rest   --> resolve segments from root (game)
    @self        --> return source_ref (script itself)
    @self/rest   --> resolve segments from source_ref (script)
    ./rest       --> resolve segments from parent(source_ref) (script.Parent)
    ../rest      --> resolve segments from parent(parent(source_ref)) (script.Parent.Parent)
    bare_path    --> resolve segments from root (backward compat fallback)

    During segment walk:
      ".."  --> go to parent of previous component
      other --> find child by filesystem name (case-insensitive)
```

**How `../` chaining works** (from Roblox docs):

- `../X` = script.Parent.Parent.X (2 ups total)
- `../../X` = script.Parent.Parent.Parent.X (3 ups -- `../` prefix gives 2, then `..` component gives 1 more)
- `../../../X` = 4 ups (prefix gives 2, two `..` components give 2 more)

**Generation formula** (ups = levels from source to LCA):

- ups=1: `"./" + remaining`
- ups>=2: `"../" * (ups-1) + remaining`

Verification: ups=3 --> `"../" * 2` = `"../../"` + remaining. Parse: `../` prefix = 2 ups, one `..` component = 1 more = 3. Correct.

Bare paths (no prefix) are treated as `@game/` for backward compatibility with old-format files. New writes never produce bare paths.

---

## Phase 1: Core Path Functions

**File: [src/rojo_ref.rs](src/rojo_ref.rs)**

- Add `compute_relative_ref_path(source_abs: &str, target_abs: &str) -> String` -- pure string function implementing the prefix selection algorithm above. Both inputs are absolute ref paths (slash-separated filesystem names from root children). Output is the prefixed relative/absolute path.
- Keep `ref_target_path_from_tree()` and `ref_target_path()` for computing absolute paths (they become internal building blocks).
- The escape/unescape functions (`escape_ref_path_segment`, `split_ref_path`, etc.) become dead code since filesystem names can't contain `/`. Remove or deprecate them.

## Phase 2: Path Resolution (Forward Sync)

**File: [src/snapshot/tree.rs](src/snapshot/tree.rs)**

- Add `resolve_ref_path(&self, path: &str, source_ref: Ref) -> Option<Ref>` -- new public function. Parses prefix, sets starting Ref, then walks segments. Handles `..` as "go to parent" at any position.
- `get_instance_by_path()` remains as internal helper for absolute segment walking (used by `resolve_ref_path` for the `@game/` and bare-path cases).

**File: [src/snapshot/patch_compute.rs](src/snapshot/patch_compute.rs)**

- `compute_ref_properties()` currently calls `tree.get_instance_by_path(path)`. Change to call `tree.resolve_ref_path(path, source_ref)` where `source_ref` is the instance that owns the attributes.

**File: [src/snapshot/patch_apply.rs](src/snapshot/patch_apply.rs)**

- `finalize_patch_application()` resolves deferred refs in `path_refs_to_rewrite`. Currently uses `tree.get_instance_by_path()`. Change to `tree.resolve_ref_path(path, source_ref)` where source_ref comes from the `MultiMap` key.

## Phase 3: CLI Syncback (Writing Relative Paths)

**File: [src/syncback/ref_properties.rs](src/syncback/ref_properties.rs)**

- **Placeholder redesign:** Placeholders must be unique per (source, target) pair since the same target gets different relative paths from different sources. Change from `__ROJO_REF_<target>`__ to `__ROJO_REF_<source>_TO_<target>`__.
- Change `placeholder_to_target: HashMap<String, Ref>` to `placeholder_to_source_and_target: HashMap<String, (Ref, Ref)>`.
- When `final_paths` IS available, call `compute_relative_ref_path(source_path, target_path)` directly instead of storing the absolute target path.
- `collect_all_paths()` is unchanged (still collects absolute paths for all instances).

**File: [src/syncback/fs_snapshot.rs](src/syncback/fs_snapshot.rs)**

- `fix_ref_paths()` substitution map entries are now `(placeholder, relative_path)`. The post-processing step in [src/syncback/mod.rs](src/syncback/mod.rs) must look up BOTH source and target in `ref_path_map`, call `compute_relative_ref_path()`, and produce the substitution.

**File: [src/syncback/snapshot.rs](src/syncback/snapshot.rs)**

- `record_ref_path()` is unchanged -- it still records absolute paths in `ref_path_map`. These are used as inputs to `compute_relative_ref_path`.

## Phase 4: Two-Way Sync (Writing Relative Paths)

**File: [src/web/api.rs](src/web/api.rs)**

- `syncback_updated_properties()`: Currently computes target absolute path via `ref_target_path_from_tree()` and stores it directly. Change to also compute source absolute path, then call `compute_relative_ref_path(source_abs, target_abs)` for the attribute value.
- `added_paths` pre-computation: Same change -- compute relative paths from source to the newly-added target.

**File: [src/change_processor.rs](src/change_processor.rs)**

- `update_ref_paths_after_rename()`: Currently does string prefix replacement on absolute paths. With relative paths on disk, the approach changes:
  1. Find affected files via RefPathIndex (still indexed by absolute target path)
  2. For each file, parse JSON, resolve each `Rojo_Ref`_* value to absolute (using source instance's absolute path)
  3. If resolved absolute matches old prefix, compute new absolute, then recompute relative path from source
  4. Write updated relative path back
- This requires knowing each file's source instance absolute path. Add a helper that derives this from the file's filesystem path + tree metadata (instigating source lookup).

## Phase 5: RefPathIndex Updates

**File: [src/rojo_ref.rs*](src/rojo_ref.rs)* (RefPathIndex)

The index stores **resolved absolute paths** as keys (not the on-disk relative strings). This preserves the efficient prefix-based lookup.

- `populate_from_dir()` --> Change to `populate_from_tree(tree: &RojoTree)`. Walk instances in the tree, read `Rojo_Ref`_* attributes, resolve relative paths to absolute using the source instance's position, index the absolute path. Called after initial tree build in [src/serve_session.rs](src/serve_session.rs) line 177-181.
- `add()` / `remove()`: Callers pass resolved absolute paths (same as before).
- `find_by_prefix()` / `update_prefix()`: Unchanged (operate on absolute keys).
- `update_ref_paths_in_file()` in [src/syncback/meta.rs](src/syncback/meta.rs): Rewrite to parse + resolve + recompute instead of string prefix replacement.

## Phase 6: Tests

### Update Existing Tests

All test fixtures and assertions with bare absolute ref paths need updating to use the new prefix format.

**Key files:**

- [rojo-test/serve-tests/ref_two_way_sync/](rojo-test/serve-tests/ref_two_way_sync/) -- fixture meta files
- [rojo-test/serve-tests/ref_forward_sync/](rojo-test/serve-tests/ref_forward_sync/) -- fixture meta files
- [tests/tests/two_way_sync.rs](tests/tests/two_way_sync.rs) -- ref assertions
- [tests/tests/serve.rs](tests/tests/serve.rs) -- ref assertions
- [src/rojo_ref.rs](src/rojo_ref.rs) -- unit tests
- [src/syncback/ref_properties.rs](src/syncback/ref_properties.rs) -- unit tests
- [src/syncback/fs_snapshot.rs](src/syncback/fs_snapshot.rs) -- fix_ref_paths tests
- [src/snapshot/tests/](src/snapshot/tests/) -- patch compute/apply tests

---

### Test Specification: `compute_relative_ref_path(source_abs, target_abs)`

Every test below shows the two absolute path inputs and the exact expected output string.

#### `@self` -- Target is the source itself or a descendant


| #   | Source            | Target                                       | Expected                           | Why                                      |
| --- | ----------------- | -------------------------------------------- | ---------------------------------- | ---------------------------------------- |
| 1   | `Workspace/Model` | `Workspace/Model`                            | `@self`                            | Self-reference                           |
| 2   | `Workspace/Model` | `Workspace/Model/Handle.model.json5`         | `@self/Handle.model.json5`         | Direct child                             |
| 3   | `Workspace/Model` | `Workspace/Model/SubFolder/Part.model.json5` | `@self/SubFolder/Part.model.json5` | Nested descendant                        |
| 4   | `Workspace/Model` | `Workspace/Model/A/B/C/D.luau`               | `@self/A/B/C/D.luau`               | Deeply nested descendant (4 levels)      |
| 5   | `Workspace/Tool`  | `Workspace/Tool/Handle.model.json5`          | `@self/Handle.model.json5`         | Tool with Handle (common Roblox pattern) |
| 6   | `Workspace/Model` | `Workspace/Model/Init`                       | `@self/Init`                       | Child that is a directory (init-style)   |
| 7   | `Workspace/Gun`   | `Workspace/Gun/Sounds/Fire.model.json5`      | `@self/Sounds/Fire.model.json5`    | Tool referencing nested sound            |
| 8   | `Workspace/Model` | `Workspace/Model/Hey_Bro.server.luau`        | `@self/Hey_Bro.server.luau`        | Slugified child name                     |
| 9   | `Workspace/Model` | `Workspace/Model/Data~2`                     | `@self/Data~2`                     | Dedup'd child                            |


#### `./` -- Target is a sibling (shares same parent, 1 level up)


| #   | Source                                     | Target                                 | Expected                | Why                                    |
| --- | ------------------------------------------ | -------------------------------------- | ----------------------- | -------------------------------------- |
| 10  | `Workspace/Folder/ScriptA.server.luau`     | `Workspace/Folder/ScriptB.server.luau` | `./ScriptB.server.luau` | Sibling script                         |
| 11  | `Workspace/Folder/ScriptA.server.luau`     | `Workspace/Folder/Config.luau`         | `./Config.luau`         | Sibling module                         |
| 12  | `Workspace/Folder/ScriptA.server.luau`     | `Workspace/Folder/SubFolder`           | `./SubFolder`           | Sibling directory                      |
| 13  | `Workspace/Folder/ScriptA.server.luau`     | `Workspace/Folder/SubFolder/Deep.luau` | `./SubFolder/Deep.luau` | Sibling then descend                   |
| 14  | `Workspace/Folder/ObjectValue.model.json5` | `Workspace/Folder/Target.model.json5`  | `./Target.model.json5`  | ObjectValue.Value pointing to sibling  |
| 15  | `Workspace/Beams/Beam.model.json5`         | `Workspace/Beams/Att1.model.json5`     | `./Att1.model.json5`    | Beam.Attachment0 to sibling attachment |
| 16  | `Workspace/Beams/Beam.model.json5`         | `Workspace/Beams/Att2.model.json5`     | `./Att2.model.json5`    | Beam.Attachment1 to another sibling    |
| 17  | `Workspace/Folder/A.luau`                  | `Workspace/Folder/Hey_Bro.server.luau` | `./Hey_Bro.server.luau` | Sibling with slugified name            |
| 18  | `Workspace/Folder/A.luau`                  | `Workspace/Folder/Data~2`              | `./Data~2`              | Sibling with dedup suffix              |
| 19  | `Workspace/Folder/A.luau`                  | `Workspace/Folder/Data~3.model.json5`  | `./Data~3.model.json5`  | Sibling model with dedup suffix        |


#### `../` -- Navigate up 2+ levels then down (within same service)


| #   | Source                                          | Target                                         | Expected                         | Why                                         |
| --- | ----------------------------------------------- | ---------------------------------------------- | -------------------------------- | ------------------------------------------- |
| 20  | `Workspace/A/Script.server.luau`                | `Workspace/B/Part.model.json5`                 | `../B/Part.model.json5`          | Parent's sibling's child (2 ups)            |
| 21  | `Workspace/A/Script.server.luau`                | `Workspace/B`                                  | `../B`                           | Parent's sibling directory (2 ups)          |
| 22  | `Workspace/A/B/Script.server.luau`              | `Workspace/C/Part.model.json5`                 | `../../C/Part.model.json5`       | Grandparent's sibling's child (3 ups)       |
| 23  | `Workspace/A/B/C/Script.luau`                   | `Workspace/D/E/Part.model.json5`               | `../../../D/E/Part.model.json5`  | 4 ups within same service                   |
| 24  | `Workspace/A/B/C/D/Script.luau`                 | `Workspace/E/Part.model.json5`                 | `../../../../E/Part.model.json5` | 5 ups within same service                   |
| 25  | `Workspace/Models/Car/Body.model.json5`         | `Workspace/Models/Truck/Body.model.json5`      | `../Truck/Body.model.json5`      | Peer model in sibling folder                |
| 26  | `Workspace/A/Deep/Script.luau`                  | `Workspace/A/Other.luau`                       | `../Other.luau`                  | Up from deep, target is cousin (2 ups)      |
| 27  | `Workspace/A/B/Beam.model.json5`                | `Workspace/A/C/Att.model.json5`                | `../C/Att.model.json5`           | Beam referencing attachment in uncle folder |
| 28  | `Workspace/Systems/Combat/Hitbox.server.luau`   | `Workspace/Systems/Audio/HitSound.model.json5` | `../Audio/HitSound.model.json5`  | Cross-system ref within same parent         |
| 29  | `Workspace/Map/Zone1/Spawns/Spawn1.model.json5` | `Workspace/Map/Zone1/Props/Tree.model.json5`   | `../Props/Tree.model.json5`      | Same zone, different category               |
| 30  | `Workspace/Map/Zone1/Spawns/Spawn1.model.json5` | `Workspace/Map/Zone2/Flag.model.json5`         | `../../Zone2/Flag.model.json5`   | Cross-zone ref                              |


#### `@game/` -- Cross-service or target is ancestor


| #   | Source                                    | Target                                       | Expected                                           | Why                                   |
| --- | ----------------------------------------- | -------------------------------------------- | -------------------------------------------------- | ------------------------------------- |
| 31  | `Workspace/Script.server.luau`            | `ReplicatedStorage/Module.luau`              | `@game/ReplicatedStorage/Module.luau`              | Classic cross-service require pattern |
| 32  | `Workspace/Script.server.luau`            | `ServerStorage/Data.luau`                    | `@game/ServerStorage/Data.luau`                    | Server script to ServerStorage        |
| 33  | `ServerScriptService/Main.server.luau`    | `ReplicatedStorage/Shared/Utils.luau`        | `@game/ReplicatedStorage/Shared/Utils.luau`        | SSS to RS (common pattern)            |
| 34  | `StarterGui/ScreenGui/Button.client.luau` | `ReplicatedStorage/UI/Theme.luau`            | `@game/ReplicatedStorage/UI/Theme.luau`            | Client UI to shared module            |
| 35  | `Workspace/A/B/C/Script.luau`             | `Lighting`                                   | `@game/Lighting`                                   | Ref to a service itself               |
| 36  | `Workspace/A/B/C/Script.luau`             | `Workspace`                                  | `@game/Workspace`                                  | Ref to own service (ancestor)         |
| 37  | `Workspace/A/B/Script.luau`               | `Workspace/A`                                | `@game/Workspace/A`                                | Ref to own grandparent (ancestor)     |
| 38  | `Workspace/A/B/C/Script.luau`             | `Workspace/A/B`                              | `@game/Workspace/A/B`                              | Ref to own great-grandparent          |
| 39  | `Workspace/Model`                         | `ReplicatedStorage/Assets/Model.model.json5` | `@game/ReplicatedStorage/Assets/Model.model.json5` | Cross-service model ref               |
| 40  | `Workspace/Beam.model.json5`              | `Lighting/Atmosphere.model.json5`            | `@game/Lighting/Atmosphere.model.json5`            | Workspace to Lighting                 |
| 41  | `ReplicatedStorage/A.luau`                | `ReplicatedStorage`                          | `@game/ReplicatedStorage`                          | Ref to own service (ancestor)         |
| 42  | `Workspace/Script.luau`                   | `SoundService/BGM.model.json5`               | `@game/SoundService/BGM.model.json5`               | To SoundService                       |


#### Special names: slugified, dedup'd, init-style


| #   | Source                    | Target                                 | Expected                | Why                                             |
| --- | ------------------------- | -------------------------------------- | ----------------------- | ----------------------------------------------- |
| 43  | `Workspace/Folder/A.luau` | `Workspace/Folder/Hey_Bro.server.luau` | `./Hey_Bro.server.luau` | Sibling with slug (real name "Hey/Bro")         |
| 44  | `Workspace/Folder/A.luau` | `Workspace/Other/CON_.luau`            | `../Other/CON_.luau`    | Windows reserved name (slugified)               |
| 45  | `Workspace/A.luau`        | `Workspace/Data~2`                     | `./Data~2`              | Dedup'd sibling (Folder)                        |
| 46  | `Workspace/A.luau`        | `Workspace/Data~3.model.json5`         | `./Data~3.model.json5`  | Dedup'd model sibling                           |
| 47  | `Workspace/A.luau`        | `Workspace/Data~2/Child.luau`          | `./Data~2/Child.luau`   | Into dedup'd folder                             |
| 48  | `Workspace/Model`         | `Workspace/Model/Scripts`              | `@self/Scripts`         | Child is init-style directory                   |
| 49  | `Workspace/Scripts`       | `Workspace/Scripts/Foo`                | `@self/Foo`             | Source is init-style, child is too              |
| 50  | `Workspace/MyFolder`_     | `Workspace/MyFolder_/Child.luau`       | `@self/Child.luau`      | Source name has trailing underscore (from slug) |


#### Tricky edge cases


| #   | Source                              | Target                              | Expected                              | Why                                                          |
| --- | ----------------------------------- | ----------------------------------- | ------------------------------------- | ------------------------------------------------------------ |
| 51  | `Workspace`                         | `Workspace/Model.model.json5`       | `@self/Model.model.json5`             | Service referencing own child                                |
| 52  | `Workspace`                         | `ReplicatedStorage/Module.luau`     | `@game/ReplicatedStorage/Module.luau` | Service to service (cross-service)                           |
| 53  | `Workspace`                         | `Workspace`                         | `@self`                               | Service self-ref                                             |
| 54  | `Workspace`                         | `Lighting`                          | `@game/Lighting`                      | Service to different service                                 |
| 55  | `Workspace/A`                       | `Workspace/AB`                      | `./AB`                                | Prefix ambiguity: "A" is NOT prefix of "AB" in segment terms |
| 56  | `Workspace/Foo.server.luau`         | `Workspace/Foo.luau`                | `./Foo.luau`                          | Same stem different extension = different instances          |
| 57  | `Workspace/A/B/C/D/E/F/Script.luau` | `Workspace/A/B/C/D/E/G/Target.luau` | `../G/Target.luau`                    | Deep nesting, close cousins                                  |
| 58  | `Workspace/A/B/C/D/E/F/Script.luau` | `Workspace/X/Target.luau`           | `../../../../../X/Target.luau`        | Deep nesting, far apart but same service                     |


---

### Test Specification: `resolve_ref_path(path, source_ref, tree)`

Build a RojoTree for each test. The tree structure is shown in comments. Each test verifies that the path resolves to the correct Ref (or None).

#### Prefix resolution

Tree for tests 59-78 (instance hierarchy with filesystem names in parens):

```
DataModel (root)
  Workspace
    ModelA (Model, fs: "ModelA")
      Script (Script server, fs: "Script.server.luau")
      Handle (Part, fs: "Handle.model.json5")
      SubFolder (Folder, fs: "SubFolder")
        Deep (Part, fs: "Deep.model.json5")
    ModelB (Folder, fs: "ModelB")
      Part (Part, fs: "Part.model.json5")
  ReplicatedStorage
    Module (ModuleScript, fs: "Module.luau")
    Shared (Folder, fs: "Shared")
      Utils (ModuleScript, fs: "Utils.luau")
  Lighting
```

Key relationships: Script and Handle are **siblings** under ModelA. ModelA **has children** (Script, Handle, SubFolder). ModelA and ModelB are **siblings** under Workspace.


| #   | Path                                        | Source | Expected | Why                                                                                                                                                                                         |
| --- | ------------------------------------------- | ------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 59  | `@self/Handle.model.json5`                  | ModelA | Handle   | `@self/` starts at ModelA, child Handle                                                                                                                                                     |
| 60  | `@self/SubFolder/Deep.model.json5`          | ModelA | Deep     | `@self/` nested descent from ModelA                                                                                                                                                         |
| 61  | `@self`                                     | Script | Script   | Self-reference (no path after prefix)                                                                                                                                                       |
| 62  | `./Handle.model.json5`                      | Script | Handle   | `./` starts at Script.Parent = ModelA, child Handle                                                                                                                                         |
| 63  | `./SubFolder/Deep.model.json5`              | Script | Deep     | `./` starts at ModelA, descend through SubFolder                                                                                                                                            |
| 64  | `./Script.server.luau`                      | Handle | Script   | `./` from Handle.Parent = ModelA, child Script                                                                                                                                              |
| 65  | `../ModelB/Part.model.json5`                | Script | Part     | `../` starts at Script.Parent.Parent = Workspace, then ModelB/Part                                                                                                                          |
| 66  | `../../ReplicatedStorage/Module.luau`       | Script | Module   | `../` starts at Workspace; remaining `../ReplicatedStorage/Module.luau` goes `..` up to DataModel root, then descends. Valid path (just not one we'd generate -- we'd use `@game/` instead) |
| 67  | `@game/ReplicatedStorage/Module.luau`       | Script | Module   | `@game/` absolute from root                                                                                                                                                                 |
| 68  | `@game/Workspace/ModelA/Handle.model.json5` | Script | Handle   | `@game/` works even when target is nearby                                                                                                                                                   |
| 69  | `@game/Lighting`                            | Script | Lighting | `@game/` to a service                                                                                                                                                                       |
| 70  | `Workspace/ModelA/Handle.model.json5`       | Script | Handle   | Bare path (no prefix) = backward compat, resolves like `@game/`                                                                                                                             |
| 71  | `@game/Workspace/ModelA/NonExistent`        | Script | None     | Target doesn't exist                                                                                                                                                                        |
| 72  | `./NonExistent.luau`                        | Script | None     | Sibling doesn't exist                                                                                                                                                                       |


#### `..` mid-path


| #   | Path                                        | Source | Expected | Why                                                                                  |
| --- | ------------------------------------------- | ------ | -------- | ------------------------------------------------------------------------------------ |
| 73  | `@self/SubFolder/../Handle.model.json5`     | ModelA | Handle   | Start at ModelA, into SubFolder, `..` back to ModelA, child Handle                   |
| 74  | `./SubFolder/../Script.server.luau`         | Handle | Script   | `./` at ModelA, into SubFolder, `..` back to ModelA, child Script                    |
| 75  | `./SubFolder/../../ModelB/Part.model.json5` | Script | Part     | `./` at ModelA, into SubFolder, `..` to ModelA, `..` to Workspace, child ModelB/Part |


#### Case-insensitive matching


| #   | Path                                        | Source | Expected | Why                                      |
| --- | ------------------------------------------- | ------ | -------- | ---------------------------------------- |
| 76  | `@self/handle.model.json5`                  | ModelA | Handle   | Lowercase "handle" matches "Handle"      |
| 77  | `./SCRIPT.SERVER.LUAU`                      | Handle | Script   | All-caps matches filesystem name         |
| 78  | `@game/workspace/modela/Handle.model.json5` | Script | Handle   | Lowercase service and folder names match |


#### Filesystem name vs instance name fallback

Tree with metadata:

```
DataModel
  Workspace
    MyFolder (fs_name: "MyFolder", instance_name: "MyFolder")
      Slugged (fs_name: "Hey_Bro.server.luau", instance_name: "Hey/Bro")
      Normal.luau (fs_name: "Normal.luau", instance_name: "Normal")
```


| #   | Path                                           | Source | Expected | Why                        |
| --- | ---------------------------------------------- | ------ | -------- | -------------------------- |
| 79  | `./Hey_Bro.server.luau`                        | Normal | Slugged  | Matches by filesystem name |
| 80  | `@game/Workspace/MyFolder/Hey_Bro.server.luau` | Normal | Slugged  | Absolute with fs name      |


---

### Test Specification: Syncback Round-Trip

Each test: build a WeakDom with specific instances and Ref properties, run syncback, verify the on-disk `Rojo_Ref_*` attribute values, then rebuild from disk and verify the Ref properties resolve to the same targets.

#### RT-1: Model with PrimaryPart (`@self/`)

```
Workspace
  TestModel (Model)
    PrimaryPart → Handle
    Handle (Part)
    Other (Part)
```

**Expected on disk** (`Workspace/TestModel/init.meta.json5`):

```json5
{ "className": "Model", "attributes": { "Rojo_Ref_PrimaryPart": "@self/Handle.model.json5" } }
```

Rebuild: `TestModel.PrimaryPart` == Handle. Round-trip identical.

#### RT-2: Cross-service ObjectValue (`@game/`)

```
Workspace
  OV (ObjectValue)
    Value → Target
ReplicatedStorage
  Target (ModuleScript, source="return {}")
```

**Expected on disk** (`Workspace/OV.model.json5`):

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "@game/ReplicatedStorage/Target.luau" } }
```

#### RT-3: Beam with attachments in same parent (`./`)

```
Workspace
  Beams (Folder)
    Beam (Beam)
      Attachment0 → Att1
      Attachment1 → Att2
    Att1 (Attachment)
    Att2 (Attachment)
```

**Expected on disk** (`Workspace/Beams/Beam.model.json5`):

```json5
{
  "className": "Beam",
  "attributes": {
    "Rojo_Ref_Attachment0": "./Att1.model.json5",
    "Rojo_Ref_Attachment1": "./Att2.model.json5"
  }
}
```

#### RT-4: Beam with attachments in sibling subfolder (`./`)

```
Workspace
  Beams (Folder)
    Beam (Beam)
      Attachment0 → Att1
    Attachments (Folder)
      Att1 (Attachment)
```

Beam and Attachments are siblings under Beams. Source = `Workspace/Beams/Beam.model.json5`. Target = `Workspace/Beams/Attachments/Att1.model.json5`. LCA = `Workspace/Beams`. ups = 1 --> `./`.

**Expected on disk** (`Workspace/Beams/Beam.model.json5`):

```json5
{
  "className": "Beam",
  "attributes": { "Rojo_Ref_Attachment0": "./Attachments/Att1.model.json5" }
}
```

#### RT-5: Beam with attachments across branches (`../`)

```
Workspace
  GroupA (Folder)
    Beam (Beam)
      Attachment0 → Att1
  GroupB (Folder)
    Att1 (Attachment)
```

**Expected on disk** (`Workspace/GroupA/Beam.model.json5`):

```json5
{ "className": "Beam", "attributes": { "Rojo_Ref_Attachment0": "../GroupB/Att1.model.json5" } }
```

Source = `Workspace/GroupA/Beam.model.json5`. Target = `Workspace/GroupB/Att1.model.json5`. LCA = `Workspace`. ups = 2. `../GroupB/Att1.model.json5`.

#### RT-6: Mixed prefix types across multiple meta files

```
Workspace
  Container (Folder)
    Model (Model)
      Handle (Part)         <-- child of Model
      RefToSibling (ObjectValue, Value → Sibling)  <-- child of Model
    Sibling (Part)           <-- child of Container (sibling of Model)
ReplicatedStorage
  SharedModule (ModuleScript)
```

Two refs produce different prefix types:

1. **Model.PrimaryPart → Handle**: Source = Model (`Workspace/Container/Model`). Handle is a descendant of Model --> `@self/Handle.model.json5`. Written in `Model/init.meta.json5`.
2. **RefToSibling.Value → Sibling**: Source = RefToSibling (`Workspace/Container/Model/RefToSibling.model.json5`). Target = Sibling (`Workspace/Container/Sibling.model.json5`). LCA = Container. ups = 2 --> `../Sibling.model.json5`. Written in `RefToSibling.model.json5`.

**Expected** (`Workspace/Container/Model/init.meta.json5`):

```json5
{ "className": "Model", "attributes": { "Rojo_Ref_PrimaryPart": "@self/Handle.model.json5" } }
```

**Expected** (`Workspace/Container/Model/RefToSibling.model.json5`):

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "../Sibling.model.json5" } }
```

Verifies two different ref prefix types coexist in the same directory tree.

#### RT-7: Refs through dedup'd siblings

```
Workspace
  Container (Folder)
    Data (Folder, child1)
    Data (Folder, child2)  --> gets dedup'd to Data~2
    Pointer (ObjectValue)
      Value → Data(child2)
```

**Expected** (`Workspace/Container/Pointer.model.json5`):

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "./Data~2" } }
```

#### RT-8: Ref to init-style script

```
Workspace
  Folder
    Pointer (ObjectValue, Value → BigScript)
    BigScript (ModuleScript with children)
      Helper (ModuleScript)
```

BigScript has children, so it becomes a directory: `Folder/BigScript/init.luau`. Filesystem name = `BigScript`.

**Expected** (`Workspace/Folder/Pointer.model.json5`):

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "./BigScript" } }
```

#### RT-9: Ref to instance with slugified name

```
Workspace
  Folder
    Pointer (ObjectValue, Value → Target)
    Target (Script, name="Hey:There", server)
```

"Hey:There" slugifies to "Hey_There". File = `Hey_There.server.luau`.

**Expected** (`Workspace/Folder/Pointer.model.json5`):

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "./Hey_There.server.luau" } }
```

#### RT-10: Deep nesting within same service

```
Workspace
  A (Folder)
    B (Folder)
      C (Folder)
        D (Folder)
          Source (ObjectValue, Value → Target)
    X (Folder)
      Target (Part)
```

Source = `Workspace/A/B/C/D/Source.model.json5`. Target = `Workspace/A/X/Target.model.json5`. LCA = `Workspace/A`. ups = 4. --> `../../../X/Target.model.json5`.

**Expected**:

```json5
{ "className": "ObjectValue", "attributes": { "Rojo_Ref_Value": "../../../X/Target.model.json5" } }
```

#### RT-11: Multiple instances referencing same target from different depths

```
Workspace
  Target (Part)
  Shallow (ObjectValue, Value → Target)
  Deep (Folder)
    Mid (Folder)
      DeepOV (ObjectValue, Value → Target)
```

- Shallow: Source = `Workspace/Shallow.model.json5`, Target = `Workspace/Target.model.json5`. ups = 1. --> `./Target.model.json5`
- DeepOV: Source = `Workspace/Deep/Mid/DeepOV.model.json5`, Target = `Workspace/Target.model.json5`. LCA = `Workspace`. ups = 3. --> `../../Target.model.json5`

Verifies same target gets different paths from different sources.

#### RT-12: Duplicate-named attachments (dedup + placeholder system)

```
Workspace
  Beams (Folder)
    Att (Attachment, child1)
    Att (Attachment, child2) --> Att~2
    Att (Attachment, child3) --> Att~3
    Beam (Beam)
      Attachment0 → Att(child2)
      Attachment1 → Att(child3)
```

**Expected** (`Workspace/Beams/Beam.model.json5`):

```json5
{
  "className": "Beam",
  "attributes": {
    "Rojo_Ref_Attachment0": "./Att~2.model.json5",
    "Rojo_Ref_Attachment1": "./Att~3.model.json5"
  }
}
```

Verifies source-aware placeholders produce correct per-source relative paths even with dedup.

---

### Test Specification: Two-Way Sync

#### TW-1: Ref property change writes relative path

Setup: Serve session with tree:

```
Workspace
  Model (Model)
    Handle (Part)
```

Plugin sends: `Model.PrimaryPart = Handle` (Ref property change).

**Verify** `init.meta.json5` on disk contains:

```json5
{ "attributes": { "Rojo_Ref_PrimaryPart": "@self/Handle.model.json5" } }
```

#### TW-2: Cross-service ref writes `@game/`

Setup:

```
Workspace
  OV (ObjectValue)
ReplicatedStorage
  Target (ModuleScript)
```

Plugin sends: `OV.Value = Target`.

**Verify** `OV.model.json5` contains:

```json5
{ "attributes": { "Rojo_Ref_Value": "@game/ReplicatedStorage/Target.luau" } }
```

#### TW-3: Nil ref removes attribute

Setup: Model already has `Rojo_Ref_PrimaryPart: "@self/Handle.model.json5"` on disk.

Plugin sends: `Model.PrimaryPart = nil`.

**Verify**: `Rojo_Ref_PrimaryPart` is removed from the meta file.

#### TW-4: Same-batch add + ref

Plugin sends in one write request:

- Add: new Part "NewPart" under Model
- Update: `Model.PrimaryPart = NewPart`

**Verify**: meta file contains `Rojo_Ref_PrimaryPart` with relative path to the newly added instance (should be `@self/NewPart.model.json5`).

#### TW-5: Rename target updates refs

Setup:

```
Workspace
  Folder
    Pointer (ObjectValue, Value → OldName)
    OldName (Part)
```

On disk: `Pointer.model.json5` has `"Rojo_Ref_Value": "./OldName.model.json5"`.

Plugin renames `OldName` to `NewName`.

**Verify**: `Pointer.model.json5` updated to `"Rojo_Ref_Value": "./NewName.model.json5"`.

#### TW-6: Rename source does NOT change its own outgoing refs

Setup:

```
Workspace
  Folder
    SourceOV (ObjectValue, Value → Target)
    Target (Part)
```

On disk: `SourceOV.model.json5` has `"Rojo_Ref_Value": "./Target.model.json5"`.

Plugin renames `SourceOV` to `RenamedOV`.

**Verify**: `RenamedOV.model.json5` still has `"Rojo_Ref_Value": "./Target.model.json5"` (unchanged -- the relative relationship didn't change).

#### TW-7: Dedup cleanup rename updates refs

Setup:

```
Workspace
  Folder
    Data (Folder)
    Data~2 (Folder)
    Pointer (ObjectValue, Value → Data~2)
```

On disk: `Pointer.model.json5` has `"Rojo_Ref_Value": "./Data~2"`.

Delete `Data` (the base). Dedup cleanup promotes `Data~2` --> `Data`.

**Verify**: `Pointer.model.json5` updated to `"Rojo_Ref_Value": "./Data"`.

#### TW-8: Base-name promotion with cross-branch ref

Setup:

```
Workspace
  GroupA
    Pointer (ObjectValue, Value → Target)
  GroupB
    Target (Part)
    Target~2 (Part)
```

On disk: `Pointer.model.json5` has `"Rojo_Ref_Value": "../GroupB/Target~2"`.

Delete `Target` (base). Cleanup: `Target~2` --> `Target`.

**Verify**: `Pointer.model.json5` updated to `"Rojo_Ref_Value": "../GroupB/Target"`.

#### TW-9: Multiple referencing files with different relative paths

Setup:

```
Workspace
  Target (Part)
  ShallowOV (ObjectValue, Value → Target)
  Deep
    DeepOV (ObjectValue, Value → Target)
```

Rename `Target` to `Renamed`.

**Verify**:

- `ShallowOV.model.json5`: `"Rojo_Ref_Value": "./Renamed.model.json5"` (was `./Target.model.json5`)
- `Deep/DeepOV.model.json5`: `"Rojo_Ref_Value":` ../Renamed.model.json5"`(was`../Target.model.json5`)

Both files updated with correct relative paths from their respective positions.

#### TW-10: Rename intermediate ancestor

Setup:

```
Workspace
  Parent
    Child
      Target (Part)
  Pointer (ObjectValue, Value → Target)
```

On disk: `Pointer.model.json5` has `"Rojo_Ref_Value": "./Parent/Child/Target.model.json5"`.

Rename `Parent` to `RenamedParent`.

**Verify**: `Pointer.model.json5` updated to `"Rojo_Ref_Value": "./RenamedParent/Child/Target.model.json5"`.

---

### Test Specification: Placeholder System (Syncback)

#### PH-1: Source-aware placeholders are unique

Build DOM with two ObjectValues pointing to the same target:

```
Workspace
  Target (Part)
  OV_A (ObjectValue, Value → Target)
  OV_B (ObjectValue, Value → Target)
```

Call `collect_referents` without `final_paths`.

**Verify**:

- OV_A gets placeholder `__ROJO_REF_<OV_A>_TO_<Target>`__
- OV_B gets placeholder `__ROJO_REF_<OV_B>_TO_<Target>`__
- Placeholders are different strings
- `placeholder_to_source_and_target` maps each to `(source_ref, target_ref)`

#### PH-2: Post-processing produces different relative paths for same target

After walk, `ref_path_map` contains:

- OV_A --> `Workspace/OV_A.model.json5`
- OV_B --> `Workspace/Deep/OV_B.model.json5`
- Target --> `Workspace/Target.model.json5`

**Verify substitutions**:

- OV_A's placeholder --> `./Target.model.json5` (sibling)
- OV_B's placeholder --> `../Target.model.json5` (2 ups)

#### PH-3: Duplicate-named targets with source-aware placeholders

```
Workspace
  Beams
    Att (Attachment, child1)
    Att (Attachment, child2)
    Beam (Beam, Attachment0 → child1, Attachment1 → child2)
```

Both attachments are named "Att" so they get different Refs but tentatively the same path. Placeholders encode both source (Beam) and target (child1, child2) so they're unique.

After walk with dedup: child1 = `Att.model.json5`, child2 = `Att~2.model.json5`.

**Verify**:

- Attachment0: `./Att.model.json5`
- Attachment1: `./Att~2.model.json5`
- No chaining corruption

#### PH-4: With `final_paths` available, computes relative directly

Call `collect_referents` WITH `final_paths`:

```
OV_A source path: "Workspace/OV_A.model.json5"
Target final path: "Workspace/Target.model.json5"
```

**Verify**: No placeholders generated. Path stored directly as `./Target.model.json5`.

---

### Test Specification: RefPathIndex with Relative Paths

#### RI-1: populate_from_tree indexes resolved absolute paths

Tree has:

```
Workspace/Model/init.meta.json5 with Rojo_Ref_PrimaryPart: "@self/Handle.model.json5"
```

Source instance absolute path: `Workspace/Model`. Resolve `@self/Handle.model.json5` from Model = `Workspace/Model/Handle.model.json5`.

**Verify**: Index contains key `Workspace/Model/Handle.model.json5` mapping to the meta file.

#### RI-2: find_by_prefix still works with absolute keys

After RI-1, call `find_by_prefix("Workspace/Model/Handle.model.json5")`.

**Verify**: Returns the meta file path.

#### RI-3: Relative refs from different files pointing to same target

Two files:

- `Workspace/A/Pointer.model.json5` with `"Rojo_Ref_Value": "./Target.model.json5"` (resolves to `Workspace/A/Target.model.json5`)
- `Workspace/B/Pointer.model.json5` with `"Rojo_Ref_Value": "../A/Target.model.json5"` (resolves to `Workspace/A/Target.model.json5`)

**Verify**: Both files indexed under key `Workspace/A/Target.model.json5`.

#### RI-4: Rename updates files with recomputed relative paths

After RI-3, rename `Workspace/A/Target.model.json5` to `Workspace/A/Renamed.model.json5`.

**Verify**:

- `Workspace/A/Pointer.model.json5` updated to `"Rojo_Ref_Value": "./Renamed.model.json5"`
- `Workspace/B/Pointer.model.json5` updated to `"Rojo_Ref_Value": "../A/Renamed.model.json5"`
- Index key updated from `Workspace/A/Target.model.json5` to `Workspace/A/Renamed.model.json5`

---

### Test Specification: `fix_ref_paths` (fs_snapshot.rs)

#### FP-1: Source-aware substitution with different relative paths

Snapshot has two meta files:

```
/test/ShallowOV.model.json5:
  { "attributes": { "Rojo_Ref_Value": "__ROJO_REF_shallow_TO_target__" } }

/test/Deep/DeepOV.model.json5:
  { "attributes": { "Rojo_Ref_Value": "__ROJO_REF_deep_TO_target__" } }
```

Substitutions:

- `__ROJO_REF_shallow_TO_target__` --> `./Target.model.json5`
- `__ROJO_REF_deep_TO_target__` --> `../Target.model.json5`

**Verify**: Each file gets its own correct relative path.

#### FP-2: Non-ref lines are never touched

Meta file has a `properties.Description` containing "Rojo_Ref_" as a string value.

**Verify**: Only lines with actual `Rojo_Ref`_ attribute keys are substituted.

#### FP-3: Non-meta files are never touched

A `.luau` file happens to contain a placeholder string in a comment.

**Verify**: File is not modified.

#### FP-4: Multiple refs in same file with mixed prefixes

```json5
{
  "attributes": {
    "Rojo_Ref_PrimaryPart": "__ROJO_REF_model_TO_handle__",
    "Rojo_Ref_Value": "__ROJO_REF_model_TO_target__"
  }
}
```

Substitutions produce `@self/Handle.model.json5` and `@game/ReplicatedStorage/Target.luau`.

**Verify**: Both substituted correctly in same file.

---

### Test Specification: Backward Compatibility

#### BC-1: Old bare paths resolve correctly

Meta file on disk: `{ "attributes": { "Rojo_Ref_PrimaryPart": "Workspace/Model/Handle.model.json5" } }`.

**Verify**: Forward sync resolves this to the correct Handle instance (bare path treated as `@game/`).

#### BC-2: Old bare paths still work during transition

Build from a project that has old-format `Rojo_Ref_`* values. Refs should resolve correctly. No errors or warnings that break the build.

#### BC-3: Syncback overwrites old format with new

Project has old bare paths on disk. Run syncback. **Verify** all `Rojo_Ref_`* values are rewritten with the correct prefix (`@self/`, `./`, `../`, or `@game/`).

---

### Test Specification: Format Transition (syncback_format_transitions)

#### FT-1: Old absolute --> new relative on re-syncback

Project has `"Rojo_Ref_PrimaryPart": "Workspace/Model/Handle.model.json5"` (old format).

Run syncback with same rbxl.

**Verify**: File updated to `"Rojo_Ref_PrimaryPart": "@self/Handle.model.json5"`.

#### FT-2: Mixed old and new formats in same project

Some files have old bare paths, others have already been updated to new format. Run syncback.

**Verify**: ALL files now use new format. No old bare paths remain.