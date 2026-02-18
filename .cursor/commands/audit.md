# /audit - Production-Grade Sync Feature Audit

**IMMEDIATE ACTION:** Before reading any files, running any commands, or performing any analysis, you MUST call `SwitchMode` with `target_mode_id: "plan"`. Do not proceed with any other step until you are in Plan mode. This is non-negotiable.

**Mode:** This command runs in **Plan mode** (read-only). It does NOT directly apply code changes. The deliverable is a `.cursor/plans/*.plan.md` file containing every approved fix, ready to be executed in a subsequent Agent-mode session.

**Workflow:** Switch to Plan mode -> Analyze -> Report -> Quiz user on each fix -> Write plan file.

## Prerequisites

Before beginning the audit, read `.cursor/rules/atlas.mdc` for the project's quality standards, architecture overview, and code conventions. The two standards below are extracted from that file and govern every audit finding:

### The Invariant (from `atlas.mdc` §Quality Standard)

> **Round-trip identity:** Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a bit-identical instance tree -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

A "works most of the time" finding is NOT acceptable. If there is ANY code path where an instance name, property, or hierarchy relationship can be lost, mangled, or silently altered through a syncback/two-way-sync -> rebuild cycle, flag it as **critical**.

### CLI Syncback Parity (from `atlas.mdc` §Two-Way Sync Strategy)

> Plugin-based sync must produce **exactly the same filesystem output** that `atlas syncback` would give for the same input. Byte-for-byte identical files, identical directory structures, identical naming.

Any divergence between the two-way sync path and the CLI syncback path is a bug until proven otherwise. The CLI syncback is the ground truth.

### Code Quality (from `atlas.mdc` §Code Quality Standard)

> Watch for duplicated logic -- the same slugify/dedup/meta-update pattern copy-pasted across `change_processor.rs`, `api.rs`, `dir.rs`, `project.rs`, etc. The goal is clean, DRY code that is easy to reason about and hard to get wrong.

- **Small refactor** (helper function, 2-3 call sites): flag it and include it in the fix plan.
- **Major rewrite** (10+ call sites, pipeline redesign): flag it in a "Deferred Refactors" section of the plan.

---

## Sync System Layout

The auditor MUST read the relevant files from each pipeline when tracing round-trip identity. This section maps every file involved in both sync systems. Read these files -- do not skip any that are in the scope of the feature being audited.

### CLI Syncback Pipeline (`atlas syncback`)

One-shot command: reads a Roblox binary/XML file, diffs it against the existing project tree, and writes the result to the filesystem.

```
CLI Entry
└── src/cli/syncback.rs                    Entry point. Loads project, reads .rbxl/.rbxm,
                                            creates ServeSession for existing tree, calls
                                            syncback_loop(), writes FsSnapshot to disk.

Syncback Core
├── src/syncback/mod.rs                    Main orchestration. syncback_loop() runs the full
│                                           pipeline: compile ignore patterns, prune unknown
│                                           children, filter hidden services, collect/link
│                                           referents, hash trees, process snapshots recursively.
├── src/syncback/snapshot.rs               SyncbackSnapshot struct. Carries instance + path +
│                                           middleware context through the pipeline. Methods:
│                                           with_joined_path(), get_path_filtered_properties().
├── src/syncback/file_names.rs             Filename generation. slugify_name() replaces forbidden
│                                           chars with '_'. deduplicate_name() appends ~1, ~2 on
│                                           collision. name_for_inst() is the main entry point.
│                                           adjacent_meta_path() computes .meta.json5 paths for
│                                           scripts (strips .server/.client suffixes from stem).
├── src/syncback/property_filter.rs        Property filtering. filter_properties() removes Name,
│                                           Parent, Source (scripts), Value (StringValue), Contents
│                                           (LocalizationTable), Ref/UniqueId, unscriptable props.
├── src/syncback/ref_properties.rs         Ref property resolution. collect_referents() finds all
│                                           Ref properties. link_referents() writes Rojo_Ref_*
│                                           (path-based) and Rojo_Id attributes to instances.
│                                           Rojo_Target_* is legacy (read-only during forward sync).
├── src/syncback/hash/mod.rs               Content hashing. hash_tree() hashes entire tree bottom-
│   └── src/syncback/hash/variant.rs       up with blake3. hash_instance() hashes class + filtered
│                                           properties + children hashes. Ref properties hashed as
│                                           paths, not raw Refs.
├── src/syncback/fs_snapshot.rs            Filesystem operation buffer. FsSnapshot collects added
│                                           files/dirs and removed files/dirs. write_to_vfs_parallel()
│                                           creates dirs (sequential), writes files (parallel with
│                                           retry on Windows), removes files (parallel), removes
│                                           dirs (sequential).
├── src/syncback/meta.rs                   Meta/model file helpers. upsert_meta_name() / remove_
│                                           meta_name() for .meta.json5. upsert_model_name() /
│                                           remove_model_name() for .model.json5. update_ref_paths_
│                                           in_file() updates Rojo_Ref_* attribute paths.
└── src/syncback/stats.rs                  Statistics tracking for syncback operations.

Snapshot Middleware (syncback direction: instance → filesystem)
├── src/snapshot_middleware/mod.rs          Dispatcher. Middleware enum routes to handler.
│                                           Each middleware has syncback() producing SyncbackReturn
│                                           { fs_snapshot, children }. get_best_middleware() lives
│                                           in src/syncback/mod.rs (selects middleware for an instance).
├── src/snapshot_middleware/dir.rs          Directory handler. syncback_dir() processes children,
│                                           creates init.meta.json5, handles .gitkeep for empty dirs.
│                                           syncback_dir_no_meta() handles child dedup and recursion.
├── src/snapshot_middleware/lua.rs          Script handler. syncback_lua() for file-based scripts,
│                                           syncback_lua_init() for directory-based (init.luau).
│                                           Extracts Source property into file body, remaining props
│                                           go into adjacent .meta.json5.
├── src/snapshot_middleware/json_model.rs   Model handler. syncback_json_model() recursively converts
│                                           instance tree into .model.json5. Name stored inline in
│                                           the model file, NOT adjacent meta.
├── src/snapshot_middleware/meta_file.rs    Meta file types. AdjacentMetadata (file.meta.json5) and
│                                           DirectoryMetadata (init.meta.json5). Handles property/
│                                           attribute overlay, $id, name override.
├── src/snapshot_middleware/project.rs      Project handler. syncback_project() updates .project.json5
│                                           with instance properties. Handles nested projects.
├── src/snapshot_middleware/csv.rs          CSV handler. Extracts Contents → .csv file.
├── src/snapshot_middleware/txt.rs          Text handler. Extracts Value → .txt file.
├── src/snapshot_middleware/rbxm.rs         Binary model. Serializes instance tree → .rbxm.
├── src/snapshot_middleware/rbxmx.rs        XML model. Serializes instance tree → .rbxmx.
├── src/snapshot_middleware/json.rs         JSON data → ModuleScript.
├── src/snapshot_middleware/toml.rs         TOML data → ModuleScript.
├── src/snapshot_middleware/yaml.rs         YAML data → ModuleScript.
└── src/snapshot_middleware/util.rs         Shared utilities (PathExt).

Binary/XML Parsing (rbx-dom submodule)
├── rbx-dom/rbx_binary/                    rbx_binary::from_reader() parses .rbxl/.rbxm → WeakDom.
│                                           rbx_binary::to_writer() serializes WeakDom → binary.
└── rbx-dom/rbx_xml/                       rbx_xml::from_reader() / to_writer() for .rbxlx/.rbxmx.
```

### Two-Way Sync Pipeline (`atlas serve` + Studio Plugin)

Live bidirectional sync. The plugin detects Studio changes and sends them to the server (Plugin → Server), and the server detects filesystem changes and pushes them to the plugin (Server → Plugin).

```
PLUGIN → SERVER (Studio changes written to filesystem)
======================================================

Change Detection (Plugin)
├── plugin/src/InstanceMap.lua             Tracks Studio instances. __connectSignals() connects
│                                           Changed events (or GetPropertyChangedSignal for
│                                           ValueBase). Fires onInstanceChanged callback.
│                                           Guards: skips if paused, if RunService:IsRunning().
└── plugin/src/Settings.lua                User settings. oneShotSync blocks outgoing writes.
                                            twoWaySync enables/disables the feature entirely.

Change Batching (Plugin)
├── plugin/src/ChangeBatcher/init.lua      Batches changes on 200ms interval. add() records
│                                           property change. __cycle() runs on RenderStepped,
│                                           __flush() converts pending → PatchSet. pause()/resume()
│                                           prevents feedback loops during reconciliation.
├── plugin/src/ChangeBatcher/createPatchSet.lua
│                                           Converts pending property changes → PatchSet. Parent
│                                           changed to nil → removed. Otherwise → encodePatchUpdate.
│                                           syncSourceOnly mode filters to Source property only.
├── plugin/src/ChangeBatcher/encodePatchUpdate.lua
│                                           Encodes property updates for existing instances. Handles
│                                           Name changes, Ref properties (with deferral for unresolved
│                                           targets), other properties via encodeProperty.
├── plugin/src/ChangeBatcher/encodeProperty.lua
│                                           Encodes a single property value using RbxDom.EncodedValue.
│                                           encode(). Maps Roblox values to wire format.
├── plugin/src/ChangeBatcher/encodeInstance.lua
│                                           Full instance encoding for additions. All children are
│                                           encoded including those with duplicate names -- the server
│                                           handles dedup. Skips Ref properties (target has no server
│                                           ID during addition). Attributes/Tags support.
└── plugin/src/ChangeBatcher/propertyFilter.lua
                                            Filters which properties are synced. Controls which data
                                            types and property names are included/excluded.

Session & Transport (Plugin)
├── plugin/src/ServeSession.lua            Orchestrates the session. onInstanceChanged → guards
│                                           __twoWaySync, calls ChangeBatcher:add(). onChangesFlushed
│                                           → guards oneShotSync, calls ApiContext:write(). Also
│                                           handles __confirmAndApplyInitialPatch (confirmation
│                                           dialog flow -- a SECOND entry point for encodePatchUpdate).
├── plugin/src/ApiContext.lua              HTTP client. write(patch) encodes PatchSet as MessagePack
│                                           via Http.msgpackEncode(), POSTs to /api/write.
├── plugin/src/PatchSet.lua                PatchSet data structure: { removed, added, updated }.
│                                           Utility functions for combining and inspecting patches.
└── plugin/src/PatchTree.lua               Tree structure for patch visualization in the UI.

Server Write Processing (receives plugin writes)
├── src/web/api.rs                         /api/write endpoint. handle_api_write() deserializes
│                                           MessagePack → WriteRequest. Processes in order:
│                                           (1) removals → syncback_removed_instance() deletes files.
│                                           (2) additions → syncback_added_instance() creates files.
│                                           (3) updates → syncback_updated_properties() writes meta/
│                                           model files. Sends PatchSet to tree_mutation_sender.
│                                           CRITICAL: file writes and PatchSet must stay consistent.
├── src/change_processor.rs                handle_tree_event() receives PatchSet from api.rs.
│                                           Processes removals (tree update), dedup cleanup (with
│                                           removed_set to exclude co-removed siblings), updates
│                                           (Source writes, renames with slugify/dedup/meta lifecycle,
│                                           ClassName transitions, ref path updates via RefPathIndex),
│                                           additions (tree insert). Applies patch via apply_patch_set().
│                                           Broadcasts via message_queue.
├── src/git.rs                             Git integration. compute_git_metadata() identifies changed
│                                           files relative to HEAD. compute_blob_sha1() hashes content
│                                           using git blob format. git_add() stages files after writes.
│                                           Used by api.rs for stageIds and by serve_session for
│                                           GitMetadata in server info response.
├── src/rojo_ref.rs                        Ref path system. ref_target_path_from_tree() builds
│                                           filesystem-name paths for Rojo_Ref_* attributes. RefPathIndex
│                                           tracks which files contain which ref paths for efficient
│                                           updates on rename. Constants: REF_PATH_ATTRIBUTE_PREFIX,
│                                           REF_POINTER_ATTRIBUTE_PREFIX, REF_ID_ATTRIBUTE_NAME.
├── src/variant_eq.rs                      Property value comparison. variant_eq() compares Variant
│                                           values with fuzzy float matching (approx_eq! with epsilon).
│                                           Used by matching algorithms for property diff scoring.
└── src/serve_session.rs                   Server-side session. Owns RojoTree, ChangeProcessor,
                                            MessageQueue, RefPathIndex, git_repo_root. Coordinates
                                            serve lifecycle.

SERVER → PLUGIN (Filesystem changes pushed to Studio)
=====================================================

Filesystem Watching (Server)
├── crates/memofs/src/lib.rs               VFS abstraction. Vfs struct with file watching,
│                                           caching, event_receiver() for change events.
├── crates/memofs/src/std_backend.rs       Real filesystem backend using notify crate. Debounced
│                                           file watching, cross-platform support.
└── src/change_processor.rs                handle_vfs_event() processes VFS events. apply_patches()
                                            finds affected instance IDs, re-snapshots, computes
                                            diff via compute_patch_set(). reconcile_tree() does
                                            full re-snapshot 200ms after events to correct drift.
                                            suppress_path() prevents echo from API writes.

Snapshot Generation (Server, forward-sync direction: filesystem → instance)
├── src/snapshot_middleware/mod.rs          snapshot_from_vfs() reads filesystem → InstanceSnapshot.
│                                           Detects directories (checks for init.* files), applies
│                                           user sync rules + default rules, dispatches to handler.
├── src/snapshot_middleware/dir.rs          snapshot_dir() reads directory → Folder snapshot.
│                                           Recurses into children, applies init.meta.json5.
├── src/snapshot_middleware/lua.rs          snapshot_lua() reads .luau → Script snapshot. Reads
│                                           adjacent .meta.json5 for properties/attributes.
│                                           snapshot_lua_init() for init scripts (usurps parent dir).
├── src/snapshot_middleware/json_model.rs   snapshot_json_model() reads .model.json5 → instance tree.
├── src/snapshot_middleware/meta_file.rs    Parses .meta.json5 files. AdjacentMetadata and
│                                           DirectoryMetadata types used as overlays.
├── src/snapshot_middleware/project.rs      snapshot_project() reads .project.json5. Resolves nodes,
│                                           infers class names for services, merges $properties.
├── (other middleware: csv.rs, txt.rs, json.rs, toml.rs, yaml.rs, rbxm.rs, rbxmx.rs)
│
├── src/snapshot/matching.rs               Forward sync matching algorithm. match_forward() pairs
│                                           InstanceSnapshots to existing RojoTree children using
│                                           recursive change-count scoring + greedy assignment.
│                                           Constants: UNMATCHED_PENALTY=10000, MAX_SCORING_DEPTH=3.

Patch & Delivery (Server)
├── src/snapshot/patch_compute.rs           compute_patch_set() diffs old snapshot vs tree →
│                                           PatchSet. Matches children by name+class. Handles
│                                           ref property rewriting (snapshot IDs → instance IDs).
├── src/snapshot/patch_apply.rs            apply_patch_set() applies PatchSet to RojoTree →
│                                           AppliedPatchSet. Deferred ref resolution (path-based
│                                           and ID-based). Cleans Rojo_Ref_*/Rojo_Target_*/Rojo_Id
│                                           from Attributes before sending to plugin.
├── src/message_queue.rs                   MessageQueue batches and queues AppliedPatchSets.
│                                           subscribe(cursor) returns receiver. Cursor system
│                                           allows reconnection recovery.
└── src/web/api.rs                         WebSocket endpoint. handle_websocket_subscription()
                                            sends AppliedPatchSets as MessagePack SocketPackets.
                                            Validates tree on connect, sends corrections.

Patch Application (Plugin)
├── plugin/src/ApiContext.lua              WebSocket client. connectWebSocket() connects to
│                                           /api/socket/:cursor. Decodes MessagePack, validates
│                                           session ID, routes to packet handlers.
├── plugin/src/ServeSession.lua            __onWebSocketMessage() receives patches. Combines
│                                           multiple messages into single patch. Shows confirmation
│                                           UI if needed, pauses ChangeBatcher, applies patch.
├── plugin/src/Reconciler/applyPatch.lua   applyPatch() applies PatchSet to Studio DOM.
│                                           Removals → instanceMap:destroyId(). Additions →
│                                           reifyInstance(). Updates → hydrate(). Deferred Refs
│                                           applied last. Pauses instances during update.
├── plugin/src/Reconciler/reify.lua        reifyInstance() creates Studio instances from virtual
│                                           data. Recursive. Defers Ref properties. Sets Parent last.
├── plugin/src/Reconciler/hydrate.lua      hydrate() matches existing Studio instances to server
│                                           IDs by Name+ClassName. Recursive child matching.
├── plugin/src/Reconciler/decodeValue.lua  decodeValue() converts encoded values → Roblox types.
│                                           Handles Ref resolution (maps server ID → Studio instance
│                                           via instanceMap).
├── plugin/src/Reconciler/diff.lua         diff() compares virtual instance data against Studio.
│                                           Used for confirmation dialog to show what will change.
├── plugin/src/Reconciler/matching.lua     Recursive change-count scoring algorithm. matchChildren()
│                                           pairs virtual instances with Studio instances during
│                                           hydration. Signature: (virtualChildren, studioChildren,
│                                           virtualInstances). One of 3 parallel matching impls that
│                                           must produce identical pairings (see also src/snapshot/
│                                           matching.rs and src/syncback/matching.rs).
├── plugin/src/Reconciler/trueEquals.lua   Shared value equality. Fuzzy floats (epsilon 0.0001),
│                                           Color3 via RGB ints, CFrame/Vector3 component-wise, NaN
│                                           handling, nil/null-ref equivalence. Used by matching.lua
│                                           and diff.lua.
├── plugin/src/Reconciler/setProperty.lua  setProperty() applies a single property to an instance.
│                                           Handles special cases and error recovery.
├── plugin/src/Reconciler/getProperty.lua  getProperty() reads a property from an instance.
│                                           Handles special cases for property access.
└── plugin/src/Reconciler/Error.lua        Error types for reconciliation failures.
```

### Shared Infrastructure (used by both pipelines)

```
Snapshot System
├── src/snapshot/mod.rs                    System overview and public exports.
├── src/snapshot/instance_snapshot.rs      InstanceSnapshot struct: snapshot_id, metadata, name,
│                                           class_name, properties, children.
├── src/snapshot/metadata.rs               InstanceMetadata, InstanceContext (ignore/sync rules),
│                                           InstigatingSource (Path vs ProjectNode), SyncRule.
├── src/snapshot/tree.rs                   RojoTree: enhanced WeakDom with metadata. path_to_ids
│                                           mapping, specified_id_to_refs tracking, get_instance_
│                                           by_path() for path-based lookups, filesystem_name_for()
│                                           for ref path resolution.
├── src/snapshot/matching.rs               Forward sync matching: match_forward() pairs snapshot
│                                           children with existing tree children. Uses recursive
│                                           change-count scoring (same algorithm as syncback and
│                                           plugin matching). UNMATCHED_PENALTY=10000.
├── src/snapshot/patch.rs                  PatchSet, AppliedPatchSet, PatchAdd, PatchUpdate,
│                                           AppliedPatchUpdate data structures.
├── src/snapshot/patch_compute.rs          compute_patch_set(): diffs snapshot vs tree. Uses
│                                           match_forward() for child pairing.
└── src/snapshot/patch_apply.rs            apply_patch_set(): applies patch to tree. Deferred ref
                                            resolution (path-based Rojo_Ref_* and ID-based
                                            Rojo_Target_*). Cleans internal attributes before
                                            sending to plugin.

Project System
└── src/project.rs                         Project loading. Project, ProjectNode, PathNode types.
                                            load_fuzzy() / load_exact() for discovery. Parses
                                            sync rules, syncback rules, glob ignore patterns.

VFS (Virtual Filesystem)
├── crates/memofs/src/lib.rs               Vfs interface. File watching, caching, event delivery.
├── crates/memofs/src/std_backend.rs       Real filesystem backend (notify crate).
├── crates/memofs/src/in_memory_fs.rs      In-memory backend (used in tests).
├── crates/memofs/src/noop_backend.rs      No-op backend.
└── crates/memofs/src/snapshot.rs          VFS snapshot types.

rbx-dom (Roblox format libraries, git submodule)
├── rbx-dom/rbx_dom_weak/                  WeakDom: in-memory instance tree representation.
├── rbx-dom/rbx_binary/                    Binary format (.rbxl/.rbxm) read/write.
├── rbx-dom/rbx_xml/                       XML format (.rbxlx/.rbxmx) read/write.
├── rbx-dom/rbx_reflection/                Property metadata and type information.
└── rbx-dom/rbx_reflection_database/       Reflection database (class/property/enum definitions).

Plugin Shared Modules
├── plugin/src/InstanceMap.lua             Instance ID ↔ Studio instance mapping. Signal management.
│                                           Also has onInstanceInserted callback for deferred Ref
│                                           resolution in ChangeBatcher.
├── plugin/src/PatchSet.lua                PatchSet data structure and utilities.
├── plugin/src/PatchTree.lua               Builds tree for patch visualization. Accepts optional
│                                           gitMetadata to compute smart default selections (push/
│                                           pull/nil) per instance based on git change status and
│                                           committed script hashes.
├── plugin/src/Config.lua                  Protocol version (6), server version compatibility,
│                                           defaultPort ("34873").
├── plugin/src/Settings.lua                Persistent settings (prefixed Atlas_ in Studio store).
│                                           Key defaults: oneShotSync=true, twoWaySync=true.
├── plugin/src/Types.lua                   Shared type definitions.
├── plugin/src/SHA1.luau                   SHA1 hashing. Computes git blob format hash ("blob
│                                           <size>\0<content>") for comparing script Source against
│                                           committed versions. Used by PatchTree for defaults.
├── plugin/src/XXH32.luau                  XXH32 hash function for plugin use.
├── plugin/src/ChangeMetadata.lua          Change metadata tracking for sync operations.
├── plugin/src/strict.lua                  Module export wrapper (strict mode enforcement).
└── plugin/src/DiffUtil.lua                Diff utilities for comparing instance data.
```

### Audit Reading Order

When auditing a feature, read files in this order to build understanding before tracing bugs:

1. **Shared infrastructure first:** `src/snapshot/mod.rs`, `src/snapshot/metadata.rs`, `src/snapshot/tree.rs` -- understand the data model
2. **Forward-sync path:** The relevant `src/snapshot_middleware/*.rs` file for the file type involved -- understand how filesystem becomes instances
3. **Syncback path:** `src/syncback/mod.rs`, then the relevant middleware's `syncback()` function -- understand how instances become filesystem
4. **Two-way sync server:** `src/web/api.rs` (`handle_api_write`, `syncback_*` functions), then `src/change_processor.rs` (`handle_tree_event`) -- understand how plugin writes are processed
5. **Two-way sync plugin:** `plugin/src/ChangeBatcher/` (encoding path), then `plugin/src/Reconciler/` (decoding path) -- understand both directions in the plugin
6. **File naming:** `src/syncback/file_names.rs` -- understand slugification, dedup, meta path computation (`adjacent_meta_path` strips script suffixes: `Foo.server.luau` pairs with `Foo.meta.json5`)
7. **Ref properties:** `src/rojo_ref.rs` (constants, `RefPathIndex`, `ref_target_path_from_tree`), `src/syncback/ref_properties.rs` (syncback ref linking), `src/snapshot/patch_apply.rs` (deferred ref resolution) -- understand cross-instance references
8. **Matching algorithms:** `src/snapshot/matching.rs` (forward sync), `src/syncback/matching.rs` (CLI syncback), `plugin/src/Reconciler/matching.lua` (plugin) -- three parallel implementations that must produce identical pairings. Also `src/variant_eq.rs` (Rust) and `plugin/src/Reconciler/trueEquals.lua` (Lua) for property comparison
9. **Git integration:** `src/git.rs` (server-side git metadata, blob hashing, auto-staging), `plugin/src/PatchTree.lua` (default selection logic using gitMetadata), `plugin/src/SHA1.luau` (plugin-side hash computation)

---

## Instructions

### 0. Read Project Standards

Read `.cursor/rules/atlas.mdc` in full. This file defines architecture, conventions, and quality bars that inform every audit step. Do not proceed without reading it.

### 1. Identify the Feature Scope

**Default behavior:** Audit all changes in the current workspace against `origin/master`. This captures everything: committed branch changes, unpushed commits, staged changes, and unstaged changes. Run:

```bash
git log origin/master..HEAD --oneline
git diff origin/master --stat
git diff origin/master
```

Read the commit log (for intent) and the full diff (for code). If the user provides plan files (via `@` reference), read those for additional context. Do NOT search for or read plan files on your own -- plans may be stale and describe outdated designs or already-fixed issues.

**If the user specifies a feature or branch:** Scope the audit to that feature's changes only.

From the diff, identify:

- Which files were modified (plugin Lua, server Rust, test fixtures)
- What the feature does (new property type support, naming change, sync flow change, etc.)
- What the correct round-trip behavior should be
- Which sync paths are affected (forward sync, reverse sync, two-way sync, all three)

### 2. Trace Round-Trip Identity (MOST CRITICAL)

For each operation the feature introduces, trace the **complete lifecycle** through every path:

#### 2a. Two-Way Sync -> Forward Sync Round-Trip

Pick a concrete example. Trace:

1. **Plugin encodes** -- How does the plugin read the property/change from Studio? What format does it send to the server? What encoding is used (msgpack, JSON, etc.)?
2. **Server receives** -- How does `handle_api_write` deserialize the data? Any type conversions?
3. **Server writes to disk** -- Which function writes to the filesystem? What file format (meta.json5, model.json5, .luau, etc.)? What path computation is used?
4. **Server applies to tree** -- How is the PatchSet built? Does `apply_patch_set` handle this data type correctly? Any special-case skips (like `Ref::none()` skip in `apply_update_child`)?
5. **VFS echo prevention** -- Is `suppress_path()` called before `fs::write()` for updates? Is suppression correctly NOT used for additions?
6. **Forward sync reads** -- When the file is read back by the snapshot system, does it produce the exact same property value? Any lossy conversion?
7. **Plugin receives** -- Does `decodeValue.lua` correctly decode the value back to a Studio-compatible type?

**Verify:** After the full round-trip, is the value identical? If you killed the server and ran `atlas build`, would the property have the correct value?

#### 2b. Nil/Removal Round-Trip

Trace what happens when the property is set to nil or removed:

- Plugin encoding of nil values
- Server handling (property removal vs. nil sentinel)
- Filesystem representation (attribute removed? file deleted? empty value?)
- Forward sync behavior (absent property = default value?)

#### 2c. Change/Update Round-Trip

Trace what happens when the value changes from A to B:

- Does the old value get properly overwritten?
- Any merge conflicts with existing file content?
- Ordering issues (removal before addition? addition before removal?)

#### 2d. Rename/Move Interactions

Trace what happens when an instance involved in the feature is renamed or moved:

- Do stored paths/references become stale?
- Does the ChangeProcessor update dependent data?
- Is there a window of inconsistency during the rename?

### 3. Audit Plugin-Side Encoding

Every code path in the plugin that encodes the feature's data must be checked:

- **Property filter** (`propertyFilter.lua`): Is the data type correctly included/excluded?
- **Patch update encoding** (`encodePatchUpdate.lua`): Does the encoding handle all cases (nil, valid, edge)?
- **Instance encoding** (`encodeInstance.lua`): For newly added instances, is the data type handled correctly? Any fragile error paths (pcall catching errors instead of explicit checks)?
- **Patch creation** (`createPatchSet.lua`): Are all required parameters passed through?
- **All callers**: Search for ALL callers of modified functions. Any call site that was NOT updated when a function signature changed? (Use grep to find every caller.)

**CRITICAL -- Lua silent argument mismatch:** In Lua/Luau, calling `foo(a, b, c)` when the function signature is `function(a, b, c, d)` does NOT produce any error -- `d` is simply `nil`. This means adding a new parameter to a function signature produces **zero errors** at callers that weren't updated. The missing argument silently becomes `nil`, and the function may take a fallback path that drops data without warning. **Grep for every caller of any modified function and verify the argument count matches.** This is the #1 source of silent data loss bugs in the plugin.

**Multiple plugin entry points:** Property changes flow through the plugin via TWO separate paths:
1. **ChangeBatcher flow** (live two-way sync): `Instance.Changed` -> `InstanceMap.__connectSignals` -> `ChangeBatcher:add()` -> `createPatchSet()` -> `encodePatchUpdate()` -> `ApiContext:write()`
2. **Confirmation dialog flow** (initial sync pull): `ServeSession.__confirmAndApplyInitialPatch` -> `encodePatchUpdate()` -> `ApiContext:write()`

Both paths must be audited. A function modified in path 1 may not have been updated in path 2.

**Change detection signals:** `Instance.Changed` fires for all scriptable properties on regular instances, but ValueBase instances (ObjectValue, IntValue, etc.) override Changed to only fire for `Value`. The InstanceMap connects `GetPropertyChangedSignal` for `Name`, `Value`, and `Parent` on ValueBase instances. Verify the feature's data type is detected by the correct signal.

### 4. Audit Server-Side Processing

Every code path in `api.rs` and `change_processor.rs`:

- **Data extraction**: Is the feature's data extracted correctly from the write request?
- **Path/value computation**: Are computed values correct? **Verify inverse consistency:** if you compute a value from data (e.g., `full_path_of(ref)` -> path), then resolve it back (e.g., `get_instance_by_path(path)` -> ref), do you get the same original data? Any case where the round-trip fails (e.g., instance names containing the path separator `/`)?
- **Meta/model file writing**: Correct file format? Correct section (properties vs. attributes)? Merge with existing content? Creation from scratch?
- **PatchSet construction**: Data included correctly? Any type conversions that lose information?
- **Filter functions**: Any filter that should now pass the data type but was not updated?
- **Dual write path consistency**: `handle_api_write` does TWO things: (1) writes files to disk via `syncback_updated_properties`, and (2) sends a PatchSet to the ChangeProcessor to update the in-memory tree. These MUST stay consistent. If the file write succeeds but the PatchSet doesn't include the same change (or vice versa), the tree and filesystem diverge silently.
- **Processing order**: `handle_api_write` processes removals, then additions, then updates. Property writes (`syncback_updated_properties`) happen BEFORE the PatchSet is sent to the ChangeProcessor. This ordering matters: if a property change references an instance that was renamed in the same batch, the path is computed from the PRE-rename tree state.
- **Sentinel values in patch system**: Check `apply_update_child` and `apply_add_child` for any data type that is silently skipped or treated as a no-op. Example: `Variant::Ref(Ref::none())` is skipped with `continue` at line 281-282 of `patch_apply.rs`. If the feature's nil/empty representation matches a sentinel, the patch system may silently ignore it instead of applying it.

### 5. Audit Forward-Sync Resolution

The filesystem -> server -> plugin direction:

- **Snapshot middleware**: Does the middleware that reads the file format correctly parse the feature's data?
- **Patch computation**: Does `compute_patch_set` handle the data type? Any special-case resolution (like `compute_ref_properties` for Rojo_Ref_* attributes)?
- **Patch application**: Does `apply_patch_set` correctly apply the data? Any sentinel values that are skipped?
- **Attribute cleanup**: Are internal attributes (Rojo_Ref_*, Rojo_Target_*, etc.) cleaned from the Attributes property before being sent to Studio?
- **Plugin decoding**: Does `decodeValue.lua` handle the type?

### 6. Audit Interactions with Existing Systems

- **Legacy systems**: Does the new feature coexist with legacy systems? Priority conflicts? Duplicate data?
- **Other callers**: Are there callers of modified functions from OUTSIDE the feature's scope? Were they updated?
- **Shared filters/helpers**: Were shared data structures (like `UNENCODABLE_DATA_TYPES`) modified? Does this affect other code paths that import them?

### 7. Audit Meta File Lifecycle

- **Creation**: New file with just the feature's data works?
- **Merge**: Adding to existing file with other content preserves everything?
- **Update**: Changing the value overwrites correctly?
- **Deletion**: Removing the value cleans up correctly? Empty sections removed?
- **Orphaned data**: Instance deleted -> file deleted -> no orphaned data?

### 8. Audit VFS Echo Prevention

The suppression system uses per-path counters. Verify:

- **Updates to existing files**: MUST call `suppress_path()` before `fs::write()`
- **New instance additions**: Must NOT suppress (VFS watcher needs to pick up new files)
- **All write branches**: Every branch that writes a file must be checked

### 9. Audit CLI Syncback Parity

Compare two-way sync output with CLI syncback output:

- Same data format on disk?
- Same file placement (attributes section vs. properties)?
- Same nil handling?
- Shared helpers used consistently?
- Any divergence is a bug per `atlas.mdc`

### 10. Audit Wire Format

- **Msgpack serialization**: Does the data type survive the plugin -> server msgpack round-trip?
- **Type parsing**: Does the server's deserializer correctly parse the wire format?
- **Edge values**: Does nil/zero/empty round-trip correctly?

### 11. Check Sync-Specific Patterns

These are patterns specific to Atlas's sync system that every audit must verify:

#### 11a. The 4 File Format Branches

`syncback_updated_properties` has 4 branches for writing property changes. EVERY audit must verify the feature works in ALL 4:

1. **Directory format**: `init.meta.json5` inside the instance's directory
2. **Standalone script**: adjacent `ScriptName.meta.json5` (strips `.server`/`.client`/`.plugin`/`.local`/`.legacy` suffix from stem)
3. **Model file**: inline in `.model.json5` (writes directly into the file, NOT adjacent meta)
4. **Other file types** (`.txt`, `.csv`, `.toml`): adjacent `FileName.meta.json5`

Common bug: treating model files like scripts (writing adjacent meta instead of inline).

#### 11b. Compound File Extensions

`.server.luau`, `.client.luau`, `.model.json5`, `.meta.json5`, `.project.json5` are multi-part extensions that need special handling:

- **Rename**: Must preserve the compound extension. The suffix list in `change_processor.rs` must include all compound parts. Missing `.model` from the suffix list caused a critical bug where `.model.json5` became `.json5` on rename.
- **Meta pairing**: `Foo.server.luau` pairs with `Foo.meta.json5` (NOT `Foo.server.meta.json5`). Script suffixes are stripped from the stem before computing the meta path.
- **Model files store `name` inline**: `.model.json5` has `"name"` in the root JSON object. `.meta.json5` has `"name"` as a top-level field. These are different locations.

#### 11c. InstigatingSource

Every instance in the RojoTree tracks where it came from:
- `InstigatingSource::Path(PathBuf)` -- created from a file, can be written back
- `InstigatingSource::ProjectNode { path, name, node, parent_class }` -- defined in project file, CANNOT be modified via two-way sync

**Verify:** Does the feature correctly guard against writing to ProjectNode instances? Is `InstigatingSource` set correctly for new instances created by the feature?

#### 11d. Pre-seeding Dedup Sets

When adding instances, `sibling_slugs` (or `taken_names`) must be seeded from existing tree children via their `instigating_source` paths (not instance names). This prevents slug collisions with existing files on disk.

**Verify:** Does the feature seed the dedup set correctly? Does it use `strip_middleware_extension` to derive the dedup key from filesystem paths?

#### 11e. Init File Detection

`init.luau`, `init.server.luau`, etc. inside directories make the directory represent the script instance. When auditing:

- **Renames**: Renaming a directory instance must rename the directory AND its init file path. The meta file is `init.meta.json5` inside the directory.
- **Deletions**: Deleting a directory instance deletes the entire directory. Also clean up adjacent `DirName.meta.json5` at the grandparent level.
- **Property writes**: Properties for directory instances go into `init.meta.json5` inside the directory, NOT an adjacent file.

#### 11f. File-to-Folder and Folder-to-File Transitions

Instances can flip between single-file representation (e.g., `Script.server.luau`) and directory representation (e.g., `Script/init.server.luau` + children). This transition is one of the most fragile areas of the sync system and a persistent source of bugs. Every audit must verify:

**When a file becomes a folder (child added to a file-represented instance):**

- The original file is removed and replaced with a directory of the same stem
- An `init.*` file is created inside the new directory with the original file's content
- The meta file transitions correctly: `Script.meta.json5` (adjacent) must become `Script/init.meta.json5` (inside directory). The old adjacent meta must be removed. No duplication.
- Existing properties, name overrides, and attributes from the old meta file are preserved in the new `init.meta.json5`
- The directory is not duplicated (e.g., creating `Script/` when `Script/` already exists from a prior partial operation)

**When a folder becomes a file (all children removed from a directory-represented instance):**

- The directory is removed and replaced with a single file
- `init.meta.json5` content migrates to an adjacent `Script.meta.json5` (or is dropped if empty). No orphaned `init.meta.json5` left behind.
- Children are not silently lost during the collapse -- verify the child list is genuinely empty before collapsing
- The init file's content becomes the new single file's content

**Path indexing during transitions:**

- `InstigatingSource` paths are updated to reflect the new location (file path vs. directory path)
- Any in-memory path lookups (ChangeProcessor path maps, VFS entries, suppression counters) are invalidated/updated for both the old and new paths
- Meta file path resolution (`get_meta_path`, `meta_path_of`, or equivalent) returns the correct path for the current representation -- not a stale path from the prior state
- Model files (`.model.json5`) follow the same transition rules: inline `name` field, not adjacent meta

**Round-trip identity through transitions:**

- Adding a child to a file-represented instance, then removing that child, must produce a filesystem state identical to the original (same file, same meta, same content)
- The transition must be idempotent: if the VFS watcher fires mid-transition, the snapshot system must not produce a corrupt intermediate state (e.g., both the old file and new directory existing simultaneously)

This area has historically produced bugs including: incorrect meta file indexing after transitions, folder duplication when the old path isn't cleaned up, child loss when directory contents aren't migrated, and stale path references in the in-memory tree. Treat any code touching file-to-folder or folder-to-file transitions as high-risk.

#### 11g. Atomicity and File Watcher Races

When an operation involves multiple filesystem changes (delete old file + create new file, or rename + meta update), there is a window where the VFS watcher could fire between operations and produce incorrect intermediate state.

**Verify:** Are multi-step operations ordered to minimize race windows? Does `suppress_path` cover all intermediate states? Could the ChangeProcessor snapshot an inconsistent state?

#### 11h. Determinism

Given the same instance tree, does the feature ALWAYS produce the same filesystem output?

- Is child ordering stable (sorted, not random)?
- Are generated identifiers (dedup suffixes, IDs) deterministic?
- Non-determinism = different builds from the same source = git churn = invariant violation.

#### 11i. Syncback Idempotency

Running the operation twice should produce zero changes on the second run. If re-running writes the same files again (even with identical content), something is unstable.

#### 11j. Review Dialogue Invariant

**No two-way sync change may bypass the review dialogue.** In Always and Initial sync modes, every incoming change must be cataloged in the confirmation UI. The user must explicitly choose a resolution (Atlas / Skip / Studio) for each entry. There is no "fast path" or silent application.

**Verify:**
- Are there any code paths in `ServeSession`, `ApiContext`, or the Reconciler that apply patches without routing through the confirmation UI?
- Does the ChangeBatcher or any batch-processing logic silently apply changes that should appear in the review dialogue?
- When multiple changes arrive in a single WebSocket message or batch, does every individual change appear as a separate reviewable entry in the UI?
- Are there error/fallback paths that skip the dialogue and apply changes directly (e.g., "if dialogue fails, apply anyway")?
- Does the plugin correctly block patch application until the user has resolved every entry in the dialogue?

Any code path that writes to Studio instances without the user having explicitly approved it through the review UI is a **critical** bug. This applies to property updates, additions, removals, renames, and any other mutation — nothing is exempt.

#### 11k. Edge Cases

- **ProjectNode instances**: Can the feature interact with instances defined in project files? Is there a guard?
- **Nested projects**: Does data span project boundaries correctly?
- **Concurrent changes**: Multiple changes in one batch handled correctly?
- **Non-existent targets**: Graceful handling when referenced data doesn't exist?
- **Name conflicts**: Instance names with special characters (`/`, `:`, `?`, `*`, `<`, `>`, `|`, `"`, `\`) that affect the feature?
- **Script type transitions**: ClassName changes (ModuleScript -> Script) cause file extension changes. Does the feature handle this?
- **One-shot mode**: Does `Settings:get("oneShotSync")` correctly block the feature's outgoing writes?

#### 11l. Matching Algorithm Parity

The matching algorithm exists in **3 parallel implementations** that must produce identical pairings for the same input:

1. **Rust syncback** -- `src/syncback/matching.rs` (`match_children`, operates on `WeakDom` instances)
2. **Rust forward sync** -- `src/snapshot/matching.rs` (`match_forward`, operates on `InstanceSnapshot` vs `RojoTree`)
3. **Lua plugin** -- `plugin/src/Reconciler/matching.lua` (`Matching.matchChildren`, operates on virtual vs Studio instances)

**Verify:**
- If the feature modifies matching logic in one implementation, was the same change applied to all three?
- Do all three use the same constants? (`UNMATCHED_PENALTY=10000`, `MAX_SCORING_DEPTH=3`)
- Do all three use the same grouping strategy? (fast-path by `(Name, ClassName)`, then recursive scoring for ambiguous groups)
- Is sort stability preserved? (Rust `sort_by` is stable; Lua `table.sort` is NOT -- ties must be broken by insertion index)
- Are the property comparison functions consistent? (`variant_eq` in Rust, `trueEquals.lua` in Lua -- both use fuzzy float equality, both treat nil and null-ref as equal)

#### 11m. Git-Based Sync Defaults and Auto-Staging

The confirmation dialog uses git metadata for smart default selections (`PatchTree.lua` + `src/git.rs`). Verify:

- **Server-side `compute_git_metadata()`**: Does it correctly identify changed files vs HEAD? Does the two-phase tree lock (snapshot paths briefly, release before git subprocesses) prevent deadlocks?
- **Hash computation parity**: Server uses `compute_blob_sha1()` in `git.rs` and plugin uses `SHA1.luau` -- both must produce identical hashes for the same content using `SHA1("blob <byte_len>\0<content>")` git blob format.
- **Default selection logic** (`PatchTree.lua`): File has no git changes → default "pull". File has git changes AND is script AND Studio Source matches committed hash → default "push". Otherwise → `nil` (user must decide). Verify these rules are applied correctly.
- **Auto-staging (`stageIds`)**: After confirmation, `api.rs` receives `stageIds` and calls `git_add`. Push-accepted items are always staged. Pull-accepted items are only staged if auto-selected (`defaultSelection ~= nil`). Manually-chosen pulls are left unstaged. Is this flow correct?
- **Staging split**: `api.rs` stages additions/removals/push files directly. `change_processor` stages Source writes after they complete (via `stage_ids` on PatchSet). Verify both paths handle the `stage_ids` list correctly.
- **No git repo**: When `git_repo_root` is `None`, all defaults should be `nil` and no staging should be attempted.

### 12. Run Static Analysis

Run `cargo clippy` on the modified code to catch Rust issues the audit might miss:

```bash
cargo clippy --all-targets 2>&1
```

Focus on warnings in files modified by the feature. Clippy catches: unused variables, dead code, redundant clones, suspicious patterns, missing error handling, and type conversion issues that could silently lose data.

Also run `selene plugin/src` for Lua linting if plugin files were modified.

### 13. Produce the Report

Structured report with:

- **Critical issues** -- data loss, incorrect values, silent corruption on round-trip
- **Correctness concerns** -- edge cases that might not manifest immediately
- **Missing test coverage** -- specific test cases needed, prioritized by risk
- **Known limitations** -- for each limitation: what it is, the concrete scenario, severity, and what fixing it would require
- **Challenged limitations** -- for each limitation: what it is, user's decision (accept/fix), and if accepted, confirmation that warnings are logged and no silent corruption occurs beyond the accepted scope
- **Code quality items** -- DRY violations, dead code, fragile error paths
- **Deferred refactors** -- major structural improvements too large to do inline
- For each issue: **file path, line numbers, description, and suggested fix**

### 14. Challenge Known Limitations

Do NOT blindly accept "known limitations" from implementation plans. For every limitation that violates round-trip identity:

1. **Present each limitation to the user** using the AskQuestion tool. State:
   - What the limitation is
   - The concrete scenario where data is lost or corrupted
   - The severity (how likely is this to hit users?)
   - What fixing it would require
2. **Ask the user to explicitly confirm** it's acceptable for this release, or whether it should be fixed now.
3. **Never silently skip a round-trip violation** just because a plan document says "known limitation." Plans are written before implementation -- the user may have changed their mind, or the severity may be worse than anticipated.

### 15. Quiz User on Each Planned Fix

**No code changes may be applied until the user explicitly approves each one.** After producing the report and challenging limitations, present EVERY planned fix to the user for approval before writing any code.

For each fix identified in the report (bug fixes, refactors, test additions, etc.), use the **AskQuestion tool** to present:

1. **What the fix changes** -- which file(s), what code, what behavior changes
2. **Why it's needed** -- which audit finding it addresses (reference the report section)
3. **Risk assessment** -- could this fix break anything else? What's the blast radius?
4. **Options:**
   - **Apply** -- proceed with this fix
   - **Skip** -- do not apply this fix (document the reason)
   - **Modify** -- the user wants a different approach (wait for their input before proceeding)

**Rules:**
- Present fixes **one at a time** so the user can evaluate each independently
- Group closely related fixes (e.g., a bug fix + its regression test) into a single question, but never bundle unrelated fixes
- If the user selects "Modify," wait for their revised instructions before continuing to the next fix
- After all fixes are quizzed, summarize which were approved, skipped, and modified
- Only proceed to step 16 with the approved and modified fixes

### 16. Create the Fix Plan

After all fixes have been quizzed and the user has made their decisions, write a plan file to `.cursor/plans/`. This plan is the deliverable of the audit -- do NOT directly apply any code changes.

**Filename:** `.cursor/plans/<descriptive_slug>.plan.md` (e.g., `fix_ref_round_trip_issues.plan.md`)

**Plan structure:**

```markdown
# <Title describing the audit scope>

> This plan was generated by `/audit` from a Plan-mode session.
> Refer to `.cursor/rules/atlas.mdc` for full project standards.

## Standards

These standards govern every fix in this plan. An implementer MUST read `.cursor/rules/atlas.mdc` before starting.

### Round-Trip Identity (from `atlas.mdc` §Quality Standard)

Syncback (or two-way sync) writes a directory tree. Building an rbxl from that directory tree and forward-syncing it back must produce a **bit-identical instance tree** -- same names, same classes, same properties, same hierarchy, same ref targets. Any deviation is a bug.

### CLI Syncback Parity (from `atlas.mdc` §Two-Way Sync Strategy)

Plugin-based sync must produce **exactly the same filesystem output** that `atlas syncback` would give for the same input. Byte-for-byte identical files, identical directory structures, identical naming. Any divergence is a bug until proven otherwise.

### Code Quality (from `atlas.mdc` §Code Quality Standard)

DRY code that is easy to reason about and hard to get wrong. The same slugify/dedup/meta-update pattern should not be copy-pasted across files. Small refactors (2-3 call sites) are included as fixes. Major rewrites (10+ call sites) are deferred.

## Context

<Brief summary of what was audited, which branch/feature, and the audit findings.>

## Fixes

Each fix below was approved by the user during the audit quiz (step 15). Implement them in order. Each fix includes its test requirements -- a fix is not complete until its tests pass.

### Fix N: <Short title>

- **Status:** Approved | Modified
- **Finding:** <Which audit section identified this (e.g., "Step 2a: Round-trip identity")>
- **Files:** <List of files to modify>
- **Problem:** <What's wrong, with file paths and line numbers>
- **Solution:** <Exact description of what to change -- specific enough to implement without re-reading the audit. For modified fixes, this reflects the user's revised approach.>
- **Risk:** <Blast radius, what else could break>
- **Verify round-trip:** <How to confirm this fix preserves round-trip identity -- specific test scenario>
- **Verify syncback parity:** <How to confirm two-way sync output matches CLI syncback -- or "N/A" if this fix doesn't touch sync paths>
- **Tests required:**
  - <Specific test description, which test layer (unit/integration/spec/snapshot), expected behavior>

## Skipped Fixes

Documented so they aren't lost -- they may be revisited later.

### Skipped: <Short title>

- **Finding:** <Audit section>
- **Problem:** <What's wrong>
- **Reason skipped:** <User's stated reason>

## Accepted Limitations

For each limitation the user accepted in step 14:

- **Limitation:** <What it is>
- **Scenario:** <When data is lost/corrupted>
- **User decision:** Accepted for this release
- **Mitigation:** <Any warnings/logging that should be in place>

## Deferred Refactors

Major structural improvements flagged during the audit that are too large to include. These should be evaluated after the fixes above are complete.

- <Refactor description, affected files, estimated scope>

## Test Plan

Tests are not optional. Every approved fix must have test coverage. A fix is not complete until its tests pass (or intentionally fail, for known limitations).

Summary of ALL tests to be written across every fix, organized by layer. Each entry references back to the fix that requires it.

### Rust Unit Tests (`#[cfg(test)]` blocks)
For isolated logic: helpers, path computation, patch compute/apply.
- <test description> (Fix N)

### Rust Integration Tests (`tests/tests/`)
For end-to-end server behavior: serve tests, two-way sync via `/api/write`, connected mode, syncback.
- <test description> (Fix N)

### Lua Spec Tests (`.spec.lua` files)
For plugin-side encoding, decoding, batching, reconciliation.
- <test description> (Fix N)

### Snapshot Tests (`insta` crate)
For forward-sync patch output verification.
- <test description> (Fix N)

### Failing Tests for Known Limitations
- <test description -- these assert correct behavior and are expected to fail> (Accepted Limitation)

## Test Rules

The implementer MUST follow these rules when writing tests for this plan:

### For bugs found and fixed

Every bug fix MUST have a test that:
1. Would have **FAILED** before the fix
2. **PASSES** after the fix
3. Prevents the bug from regressing

### For missing coverage identified

For each "missing test coverage" gap:
1. Write the test
2. Verify it passes against the current implementation
3. If it fails, investigate -- it may have found another bug

### For known limitations (Accepted Limitations section)

For each known limitation that violates round-trip identity:
1. Write a test that asserts the **correct** behavior
2. The test SHOULD FAIL against the current implementation
3. **Leave the test failing** -- do NOT mark it `#[ignore]` or fix the implementation
4. If a test unexpectedly PASSES, analyze why. If correct, keep it as free coverage
5. These tests serve as acceptance criteria for future fixes

### Scope: test every affected layer

For each finding, determine which test layers are affected and write tests in ALL of them:
- Does the bug manifest at the plugin level? -> Lua spec test
- Does it manifest at the server level? -> Rust unit test or integration test
- Does it affect the round-trip? -> Integration test with `/api/write` + filesystem verification
- Does it affect forward-sync? -> Serve test with `recv_socket_packet`
- Could it regress from a different code path? -> Test that code path too

## Final Step: Run CI

After ALL fixes and tests are implemented, run the `/ci` command (see `.cursor/commands/ci.md`) to execute the full CI pipeline. Every fix must pass CI before the plan is considered complete. Do not skip this step.
```

### 17. Run CI After Plan Execution

After the plan has been fully executed in Agent mode (all approved fixes applied, all tests written), run the `/ci` command (`.cursor/commands/ci.md`) as the final step. The plan is not complete until CI passes clean.

**Rules for the plan file:**

- The plan must be **self-contained**: an implementer should be able to execute it in Agent mode by reading only the plan file and `atlas.mdc`, without re-reading the audit chat
- The Standards section is mandatory -- it anchors every fix to the project's quality bars
- Every fix must include `Verify round-trip` and `Verify syncback parity` fields so the implementer knows how to validate correctness
- Include exact file paths and line numbers (as of the current commit)
- For each fix, list the tests that must accompany it -- a fix without a test entry is incomplete
- Tests for known limitations must assert the **correct** behavior (they are expected to fail against the current implementation and should be left failing)
- Skipped fixes are documented with their problem description so future audits don't rediscover them
- The plan file is the single source of truth for post-audit work; do not leave information only in the chat
