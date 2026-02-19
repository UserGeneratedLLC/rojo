# Rojo Changelog

<!-- DIRECTIONS
Thanks for contributing to Rojo! Please add your change to the "Unreleased" section below.

Use the following format:

```
## Unreleased

* Made a change to Rojo ([#0000])

[#0000]:  https://github.com/rojo-rbx/rojo/pull/0000
```

When adding your link definition, please keep them in ascending order (lowest number first).

Making a new release? Simply add the new header with the version and date underneath "Unreleased" and add the version link, like so:

```
## Unreleased

## [0.0.0] (January 1st, 1970)

* ... the changes here

[0.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.0.0
[#0000]: ... the other links go after the release link
```
-->

## Unreleased

## [8.4.1] (February 19th, 2026)

* Fix spurious diffs during VFS events by removing redundant Rojo reference attributes from snapshot comparisons.
* Fix `Rojo_Id` attribute handling in patch computation to improve reference resolution for instances.
* Ensure deterministic ordering for interned instance additions by sorting by redacted parent and name.
* Reduce log verbosity by changing prefetch file statistics logging from info to debug.
* Update README with Atlas branding, installation instructions, and CLI commands.
* Update vscode-rojo submodule to latest commit.

## [8.4.0] (February 19th, 2026)

* Implement ambiguous path handling in the Reconciler with a 3-pass matching algorithm (exact match, property-weighted scoring, recursive change-count) that correctly reconciles instances when multiple children share the same name.
* Add default property caching and enhance property comparison logic for more accurate instance matching during reconciliation.
* Implement recursive change-count matching as a tiebreaker when property-weighted scoring produces ambiguous results.
* Refactor Reconciler equality checks and path handling for robustness with duplicate-named siblings.
* Refactor descendant search functions for `WeakDom` and `RojoTree` to support the new matching algorithm.
* Expose `snapshot`, `syncback`, and `variant_eq` modules publicly and add integration tests for the matching algorithm.
* Update ChangeBatcher tests to reflect encoding behavior for duplicate-named children.
* Enhance deduplication processes and refactor code for improved readability and consistency.
* Update rbx-dom subproject and reflection database.
* Implement live syncback via the Studio plugin with service properties encoding, allowing Roblox→filesystem sync during `atlas serve` sessions without the CLI command.
* Add `SyncbackRequest` structure and API protocol validation for plugin-initiated syncback.
* Add `SyncbackConfirm` UI component for reviewing and confirming live syncback operations in the plugin.
* Implement git-based sync direction defaults: use `git status` metadata to auto-select sync direction for changed files, with `stage_paths` support in `PatchSet` for automatic git staging.
* Enhance matching module with reference identity checks, matching session caching, valid children tracking, and `@native` codegen annotations.
* Sort syncback matched indices by new-child index for stable output ordering.
* Update deduplication suffix numbering to start at `~2` instead of `~1`.
* Refactor `RefPathIndex` initialization and attribute indexing; add regression tests for unique placeholder handling in reference paths.
* Update syncback rules and property filter defaults.
* Compact JSON5 array formatting for property values.
* Update color handling and enhance session management in the Reconciler.
* Refactor animation goal settings to use instant transitions in the plugin UI.
* Refactor version rejection logic in `ApiContext`.
* Add VFS prefetch cache to optimize file reads during project loading, reducing I/O overhead for large projects.
* Implement parallel I/O in the VFS backend for improved serve startup and file processing performance.
* Replace `float-cmp` dependency with custom float comparison and formatting using `lexical-write-float` for precise, round-trip-safe disk representation of float properties.
* Fix NaN equality in `trueEquals` for compound types (e.g., `CFrame`, `Vector3` with NaN components).
* Fix patch application logic to ensure correct resolution order for legacy `Rojo_Target_*` and path-based `Rojo_Ref_*` references.
* Fix multiple live syncback issues including reference handling and float precision.
* Add yield intervals for plugin diffing and hydrating processes to prevent UI freezes during large syncs.
* Add automatic dedup suffix stripping during syncback for cleaner output when suffixes are no longer needed.
* Add server-side and test-side port retry mechanisms to handle port race conditions.
* Refactor font comparison logic in `variant_eq` and `hash_variant` for correct Font property equality.
* Refactor syncback error handling and improve concurrency management.
* Refactor ChangeBatcher encoding logic for improved readability and duplicate handling.
* Consolidate filesystem name retrieval in `ref_target_path_from_tree`.
* Enhance file and directory removal logic in VFS.
* Enhance `PatchVisualizer` with change statistics in the syncback confirmation dialog.
* Add new Git configuration options in `InitCommand` for improved project initialization usability.
* Increase server initialization backoff duration and maximum retry attempts.
* Refactor logging levels to debug for improved performance.
* Update rbx-dom submodule and reflection database.
* Add plugin upload workflow using Lune and OpenCloud for automated Roblox marketplace publishing.
* Add `publish-plugin` scripts (PowerShell and Bash) and integrate plugin upload step into the GitHub Actions release workflow.

<details>
<summary>Full commit log</summary>

- `ba117f3a` Enhance Reconciler robustness and review dialogue compliance
- `9d57c8d6` Add support for Ambiguous Path Handling
- `cc77a2cd` Add comprehensive plan for Ambiguous Path Handling
- `d6162ac2` Complete implementation of the 3-pass matching algorithm for ambiguous path handling
- `e313f615` Refactor ChangeBatcher tests to remove skipped count checks
- `4cb45cd6` Add audit plan for Ambiguous Path Handling feature
- `1a1d9ce8` Implement new recursive change-count matching algorithm for instance reconciliation
- `3ad3c21c` Add ref path and dedup integration plan
- `bb9d2412` Refactor Reconciler tests and update path handling logic
- `aabbfb23` Update ChangeBatcher tests to reflect encoding behavior for duplicate-named children
- `e207a398` Refactor Reconciler equality checks and enhance path handling
- `f2cdb059` Add audit plans for ambiguous paths fixes and follow-up actions
- `ff48e059` Enhance audit plan for ambiguous paths follow-up actions
- `ed1f61c0` Complete audit plan updates and enhance deduplication processes
- `31350cd7` Consolidate glob patterns in MDC files for improved clarity
- `cc2c8287` update rules
- `85787f34` Refactor code for improved readability and consistency
- `0afe6be4` Update reflection database and enhance deduplication processes
- `e54f5ef4` Update rbx-dom subproject to latest commit for improved functionality
- `9c6966ac` Add default properties caching and update matching logic
- `d45b1c5b` Enhance property comparison and caching mechanisms
- `b6e34b9e` Expose snapshot, syncback, and variant_eq modules publicly; add integration tests for matching algorithm
- `21b3ee45` Refactor descendant search functions for WeakDom and RojoTree
- `6ef81cf8` Refactor matching logic for improved readability and consistency
- `5c4fd801` Enhance Reconciler with ambiguous path handling and improved matching logic
- `dea60eb4` Add formatting and static analysis scripts with usage instructions
- `df225001` Refactor code for improved clarity and maintainability
- `7ba04f7f` Update dependencies and implement git-based sync direction defaults
- `6c0098d8` Add git metadata handling and integration tests for sync direction defaults
- `dcd92537` Refactor version rejection logic in ApiContext
- `91c599ec` Enhance documentation and refactor syncback logic
- `6dec0686` Refactor code for improved readability and maintainability
- `bb4d40c4` Enhance git metadata handling in compute_git_metadata function
- `6de088be` Add batch script files and enhance git staging logic
- `a59d9d49` Add stage_paths field to PatchSet struct in tests
- `d836ebdd` Update deduplication logic and documentation for suffix handling
- `7c3cb8ab` Add deduplication plan to start suffix numbering at ~2
- `a17852f9` Update syncback rules and property filter defaults
- `fe5852f9` Add compact JSON5 array formatting plan and update JSON serialization logic
- `201b9711` Refactor animation goal settings to use instant transitions
- `c5a5e973` Refactor RefPathIndex initialization and attribute indexing
- `a741f573` Add regression tests for unique placeholder handling in reference paths
- `d6169c6d` Refactor test assertions and improve code formatting
- `b8c3520d` Add Luau-style ref paths migration plan and update related tests
- `6f8e5a5c` Refactor code for improved readability and consistency
- `307ac81a` Implement live syncback feature and enhance API for service data transfer
- `b39c01e5` Add matching session caching and native codegen improvements
- `fa2b86c3` Refactor SyncbackConfirm component and enhance UI responsiveness
- `372a422b` Update SyncbackRequest structure and API protocol validation
- `93ea0bde` Refactor syncback process and improve code readability
- `bf4aef6e` Update sourcemap generator command in VSCode settings
- `b687c886` Enhance matching module with reference identity and caching improvements
- `ddd47b9a` Enhance matching module with valid children tracking and code readability improvements
- `30e032b2` Update color handling and enhance session management in the Reconciler
- `72ad84b5` Implement live syncback service properties encoding and enhance syncback functionality
- `c3cb9b4b` Enhance syncback functionality and improve matching process
- `8070129b` Refactor service encoding to conditionally include properties, attributes, tags, and references
- `e0fd9077` Refactor service encoding to streamline properties handling
- `a823db02` Refactor encoding logic in ChangeBatcher and improve matching depth handling
- `a75a1443` Enhance syncback process by integrating serialization of service children
- `7dc4f527` Implement live syncback fixes addressing multiple issues
- `39c8bd41` Enhance live syncback process with improved reference handling and float precision
- `3c026aea` Add float formatting functions and update serializer for improved precision
- `0450dabf` Refactor encoding logic in ChangeBatcher for improved readability and consistency
- `07e0100c` Add lexical-write-float and update float formatting in JSON serialization
- `ea902572` Update float cutoff values and adjust test cases for improved precision
- `b8a1dc26` Remove zero cutoff checks in float formatting functions
- `f5939258` Remove max significant digits and zero cutoff checks in float formatting functions
- `7f4b5c49` Refactor float formatting tests for improved precision and consistency
- `b47eef11` Add live syncback functionality and optimize service encoding
- `e6c06276` Add luau-optimize cursor command for performance optimization
- `24dec407` Enhance luau-optimize documentation with detailed type safety guidelines
- `9d8762bb` Refactor encoding logic in ChangeBatcher and enhance duplicate handling
- `9bb237f4` Add ChangeBatcher Encoding Audit Plan
- `d3a9155e` Update patch application logic to ensure correct resolution order for legacy and path-based references
- `b0f9b06c` Refactor filesystem name retrieval in `ref_target_path_from_tree`
- `23874d8a` Consolidate filesystem name retrieval in `ref_target_path_from_tree`
- `b1180d73` Enhance Luau documentation with additional context and examples
- `a283fd1d` Add Git Sync Defaults Audit Plan
- `7e0538b9` Remove `float-cmp` dependency and implement custom float comparison logic
- `69f0b9fd` Add Audit Plan for Ambiguous Path Handling
- `5b38648e` Update dependencies and improve syncback handling
- `49b4d858` Add Live Syncback Audit Fixes Plan
- `532425df` Update audit documentation and rules for live syncback enhancements
- `17d52e3e` Enhance float formatting and comparison functions for disk representation
- `7d0232c4` Add automatic dedup suffix stripping to enhance syncback functionality
- `bf56dc47` Refactor syncback error handling and improve concurrency management
- `d9fa62c8` Add server-side and test-side port retry mechanisms to handle race conditions
- `b85c5f5e` Refactor font comparison logic in variant_eq and hash_variant functions
- `354d4c8b` Add yield intervals for diffing and hydrating processes
- `b105d865` Fix NaN equality for trueEquals for compound types
- `1ffed121` Add prefetch cache for VFS to optimize file reads
- `9d3d02cd` Add new Git configuration options in InitCommand
- `0462b91a` Enhance PrefetchCache functionality and improve project file prefetching
- `5a943c33` Refactor prefetch_project_files function for improved readability
- `3ccaaa63` Enhance PrefetchCache and improve directory handling in VFS
- `5fab0d57` Refactor ServeCommand and enhance placeholder replacement in syncback
- `56e1ab4a` Remove unused methods from ServeSession and clean up syncback logic
- `1c25b928` Refactor logging levels to debug for improved performance and clarity
- `43834f35` Update submodule and implement parallel I/O audit fixes
- `a071d1f0` Update submodule and enhance PatchVisualizer with change statistics
- `8d17b1de` Enhance InitCommand Git configuration for improved usability
- `57112a9f` Update submodule and enhance syncback confirmation dialog
- `4f0afe87` Update submodule: rbx_binary Font snapshots remove cachedFaceId
- `d9459638` Update submodule: rbx-dom to latest commit b75dd401
- `d360ebe6` Enhance file and directory removal logic in VFS
- `d252b138` Increase backoff duration and maximum retry attempts in server initialization
- `8cf140af` Add enhancements and fixes to VFS and syncback processes
- `4918fb80` Add plugin upload workflow and related enhancements

</details>

## [8.3.0] (February 15th, 2026)

* Add two-way sync support for Ref properties via `Rojo_Ref_*` attributes. The plugin now detects Ref property changes in Studio and encodes them as path-based attributes; the server resolves paths back to instance Refs. ([#2])
* Implement `RefPathIndex` for efficient resolution of `Rojo_Ref_*` attributes during forward sync, including ambiguous-path detection and graceful fallback for nonexistent targets.
* Extend `ChangeProcessor` to handle `Rojo_Ref_*` attributes in filesystem writes during live two-way sync sessions.
* Extend `ChangeBatcher` and `encodePatchUpdate` in the plugin to encode, filter, and batch Ref property changes for the server.
* Add `InstanceMap:getIdByInstance` to the plugin for reverse instance-to-ID lookups needed by Ref encoding.
* Add comprehensive integration tests for Ref forward sync, two-way sync, and ambiguous-path edge cases.
* Optimize CI scripts for parallel Rust task execution.

<details>
<summary>Full commit log</summary>

- `bf65d8bf` initial work
- `bbd9c0e0` test fixtures
- `c1811724` Refactor test cases and improve snapshot handling
- `4070b483` Add audit documentation and enhance two-way sync handling
- `8f0df692` Enhance InstanceMap and ChangeBatcher for unresolved Ref properties
- `ca2a9389` Enhance audit documentation and address critical Ref property issues
- `faa12dc5` Implement RefPathIndex for efficient handling of Rojo_Ref_* attributes
- `075cca4c` Add audit plan for Ref property fixes and enhance CI test execution
- `c3efb0e5` Optimize CI scripts for Rust tasks by adding parallel execution
- `e617e8fc` Enhance audit documentation and improve Ref property handling
- `012c20b1` Add audit plan for Fix Ref Audit Round 3 addressing correctness and code quality issues
- `699fe60b` Add integration tests for Ref properties in standalone file formats
- `f28ee843` Update audit plans and documentation for Rojo_Ref_* attributes
- `6da94696` Update Luau LSP sourcemap setting to disable autogeneration

</details>

[8.4.1]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.4.1
[8.4.0]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.4.0
[8.3.0]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.3.0
[#2]: https://github.com/UserGeneratedLLC/rojo/pull/2

## [8.2.0] (February 12th, 2026)

* Fix tree drift caused by echo suppression eating VFS events for newly added instances. New-instance file writes no longer suppress watcher events, allowing the tree to pick them up correctly.
* Add periodic tree reconciliation in the ChangeProcessor to recover from lost OS file notification events (Windows `ReadDirectoryChangesW` buffer overflow). Runs automatically after event bursts settle.
* Add `GET /api/validate-tree` endpoint for test infrastructure to verify tree-filesystem consistency.
* Expose `WatcherCriticalError` channel from VFS to ChangeProcessor for `RescanRequired` event handling.
* Add VFS staleness tests, chaos fuzzer stress test, and tree freshness assertions across the connected mode test suite.
* Tighten echo suppression test assertions from lenient bounds to precise expected values.
* Update protocol version to 6.

<details>
<summary>Full commit log</summary>

- `d84b6a35` Merge pull request #1 from UserGeneratedLLC/vfs-patches
- `2e49763b` Update snapshot files and enhance VFS event handling
- `b1789a37` Refactor VFS event handling in ChangeProcessor
- `7f89d4ce` Enhance tree synchronization and testing capabilities
- `8462c06b` Enhance VFS initialization and error handling
- `ec7f9101` Refactor API file handling and improve echo suppression documentation
- `e2f515d8` Update dependencies and enhance VFS staleness testing
- `200c5ca2` Update protocol version to 6 and enhance API documentation
- `f798f3f5` Enhance connected mode test suite and introduce VFS staleness fixtures
- `316d7ea7` Update logging levels in Rojo plugin and enhance test plans
- `2cc17269` Add git configuration and update gitignore for project template
- `77b9c0e5` Update Luau LSP sourcemap file in VS Code settings
- `11d0e9d8` Update VS Code configuration and dependencies

</details>

[8.2.0]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.2.0

## [8.1.0] (February 11th, 2026)

* Overhaul filename handling: replace path encoding system with a slugify approach for robust round-trip fidelity between Roblox instances and the filesystem.
* Refactor syncback instance processing, metadata name handling, and deduplication logic for correctness and clarity.
* Add `bare_slug_from_filename` method for improved filename processing in snapshot middleware.
* Fix stem-level deduplication edge cases and improve error handling in metadata file operations.
* Streamline slugification logic in ApiService for two-way sync.
* Add option to forget prior info for a place in the reminder notification.
* Improve sourcemap path handling with `pathdiff` for correct relative paths.
* Refactor property comparison in `compute_property_patches`.
* Enhance session management in `App:startSession`.
* Refactor upload command to use `rbx_cookie` for Roblox auth cookie retrieval.
* Add build steps for VS Code extension in CI and release workflows; enable submodule checkout.
* Update rbx-dom and vscode-rojo submodule references.

<details>
<summary>Full commit log</summary>

- `f037ec3b` Refactor filename handling in ApiService to streamline slugification logic
- `72b1d8d4` Merge branch 'slugify' into master
- `bb266789` Add audit plan for slugify branch pass 3 and implement critical fixes
- `2a917bf8` Update rbx-dom subproject reference to latest commit 2533e16e
- `e01a143a` Update rbx-dom subproject reference to latest commit 5c8c108d
- `8dfca6b1` Update rbx-dom subproject reference and adjust CI script paths for Lua and Rust formatting tools
- `2ac0f117` Add option to forget prior info for place in reminder notif (#1215)
- `434a3c45` Refactor CI documentation and improve change processing logic
- `095e78df` Add option to forget prior info for place in reminder notif (#1215)
- `2ff5c393` Implement fixes and enhancements from audit pass 2
- `baf05dcb` Update slugify branch audit plan and implement fixes for identified issues
- `d8175d3b` Add audit plan for slugify branch to ensure round-trip fidelity
- `917949ed` Add bare_slug_from_filename method for improved filename processing
- `085e1ab3` Refactor change processing and metadata handling for improved clarity
- `7d47fe73` Enhance slug deduplication and metadata handling in syncback process
- `f38c1f13` Refactor metadata name handling for improved clarity
- `5a5e1a0c` Enhance error handling in metadata file operations
- `f496bee3` Refactor CI pipeline scripts and documentation
- `9688224c` Enhance slugification process and deduplication logic
- `41264a5a` Implement removal of `name` field from `.meta.json5` files
- `cc2eb2b6` Refactor syncback instance processing for improved efficiency
- `7a090b10` Refactor syncback instance handling for improved readability
- `560fce13` Implement fix for stem-level deduplication in file handling
- `126c8e66` Update vscode-rojo submodule reference and complete migration plan for path encoding
- `0955e38b` Add migration plan to replace path encoding system with slugify approach
- `f0932600` Refactor instance path retrieval in snapshot.rs
- `14e88157` Update rbx-dom submodule reference to the latest commit
- `64d21dc6` Update submodule reference and refactor match statements for improved clarity
- `181a003a` Improves sourcemap path handling with pathdiff
- `80b83310` Refactor property comparison in compute_property_patches
- `6f4a2792` Enhance session management in App:startSession
- `d43b9bc0` Enable submodule checkout in CI and release workflows
- `332566be` Add build steps for VS Code extension in CI and release workflows
- `255b3464` Update vscode-rojo submodule to latest commit
- `431a8b30` Update vscode-rojo submodule and add new rokit.toml configuration
- `2803c2d7` Add .env to .gitignore and update dependencies
- `7b737201` update docs
- `0b1e8c89` Refactor upload command to use rbx_cookie for Roblox auth cookie retrieval

</details>

[8.1.0]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.1.0

## [8.0.3] (February 9th, 2026)

* Bump toolchain dependencies in rokit.toml: selene 0.30.0, stylua 2.3.1.
* Add luau and luau-lsp to rokit.toml toolchain.

<details>
<summary>Full commit log</summary>

- `63447c7b` Enable luau and luau-lsp dependencies in rokit.toml
- `93c360f5` Bump dependencies in rokit.toml

</details>

[8.0.3]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.0.3

## [8.0.2] (February 9th, 2026)

* Update vscode-rojo extension submodule.
* Bump dependencies: fs-err 3.3.0, json5 1.3.1, ryu 1.0.23, unicode-ident 1.0.23, zmij 1.0.20.
* Remove self-referencing atlas entry from rokit.toml.

<details>
<summary>Full commit log</summary>

- `d210dca9` Update subproject commit reference for vscode-rojo
- `164d82a9` Update subproject commit reference for vscode-rojo
- `5bd098f0` Bump dependencies in Cargo.lock and Cargo.toml
- `d3c684e6` Bump version to X.Y.Z
- `61c21bc1` Remove atlas reference from rokit.toml
- `82e48071` Update CHANGELOG for version 8.0.1 and add release command documentation

</details>

[8.0.2]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.0.2

## [8.0.1] (February 8th, 2026)

* Root install scripts now also build and install the VS Code/Cursor extension.
* Syncback now manages `.gitkeep` files to preserve empty directories during filesystem writes.
* Added Git-style change metadata and diff utilities to the plugin for richer patch visualization.
* Refactored subtree highlight handling in PatchVisualizer and DomLabel components.
* Improved two-way sync test reliability with polling helpers and macOS compatibility checks.
* Fixed internal reference in ServeSession to reflect Atlas sync operation.
* Standardized messageCursor redaction in snapshot test files.

[8.0.1]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.0.1

## [8.0.0] (February 7th, 2026)

* Improved syncback performance by parallelizing filesystem writes using rayon. File writes and removals now run concurrently, while directory operations remain sequential to preserve ordering constraints.
* Fixed the "Always" confirmation behavior setting to prompt for confirmation on every sync patch, not just the initial sync (which is handled by "Initial", the default confirmation behavior setting). Changes that arrive during confirmation are merged into the pending patch and the UI updates in real-time. ([#1216])
* Fixed a bug caused by having reference properties (such as `ObjectValue.Value`) that point to an Instance not included in syncback. ([#1179])
* Fixed instance replacement fallback failing when too many instances needed to be replaced. ([#1192])
* Fixed a bug where MacOS paths weren't being handled correctly. ([#1201])
* Fixed a bug where the notification timeout thread would fail to cancel on unmount ([#1211])
* Added a "Forget" option to the sync reminder notification to avoid being reminded for that place in the future ([#1215])

[#1179]: https://github.com/rojo-rbx/rojo/pull/1179
[#1192]: https://github.com/rojo-rbx/rojo/pull/1192
[#1201]: https://github.com/rojo-rbx/rojo/pull/1201
[#1211]: https://github.com/rojo-rbx/rojo/pull/1211
[#1215]: https://github.com/rojo-rbx/rojo/pull/1215
[#1216]: https://github.com/rojo-rbx/rojo/pull/1216
[8.0.0]: https://github.com/UserGeneratedLLC/rojo/releases/tag/v8.0.0

## [7.7.0-rc.1] (November 27th, 2025)

* Fixed a bug where passing `--skip-git` to `rojo init` would still create a file named `gitignore.txt` ([#1172])
* A new command `rojo syncback` has been added. It can be used as `rojo syncback [path to project] --input [path to file]`. ([#937])
 	This command takes a Roblox file and pulls Instances out of it and places them in the correct position in the provided project.
    Syncback is primarily controlled by the project file. Any Instances who are either referenced in the project file or a descendant
    of one that is will be placed in an appropriate location.

    In addition, a new field has been added to project files, `syncbackRules` to control how it behaves:

    ```json
    {
        "syncbackRules": {
            "ignoreTrees": [
                "ServerStorage/ImportantSecrets",
            ],
            "ignorePaths": [
                "src/ServerStorage/Secrets/*"
            ],
            "ignoreProperties": {
                "BasePart": ["Color"]
            },
            "syncCurrentCamera": false,
            "syncUnscriptable": true,
        }
    }
    ```

    A brief explanation of each field:

    - `ignoreTrees` is a list of paths in the **roblox file** that should be ignored
    - `ignorePaths` is a list of paths in the **file system** that should be ignored
    - `ignoreProperties` is a list of properties that won't be synced back
    - `syncCurrentCamera` is a toggle for whether to sync back the Workspace's CurrentCamera. Defaults to `false`.
    - `syncUnscriptable` is a toggle for whether to sync back properties that cannot be set by the Roblox Studio plugin. Defaults to `true`.

* Fixed bugs and improved performance & UX for the script diff viewer ([#994])
* Rebuilt the internal communication between the server and plugin to use [websockets](https://devforum.roblox.com/t/websockets-support-in-studio-is-now-available/4021932/1) instead of [long polling](https://en.wikipedia.org/wiki/Push_technology#Long_polling) ([#1142])
* Added support for `.jsonc` files for all JSON-related files (e.g. `.project.jsonc` and `.meta.jsonc`) to accompany JSONC support ([#1159])

[7.7.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.7.0-rc.1
[#937]: https://github.com/rojo-rbx/rojo/pull/937
[#994]: https://github.com/rojo-rbx/rojo/pull/994
[#1142]: https://github.com/rojo-rbx/rojo/pull/1142
[#1159]: https://github.com/rojo-rbx/rojo/pull/1159
[#1172]: https://github.com/rojo-rbx/rojo/pull/1172

## [7.6.1] (November 6th, 2025)

* Fixed a bug where the last sync timestamp was not updating correctly in the plugin ([#1132])
* Improved the reliability of sync replacements by adding better error handling and recovery ([#1135])
* Small improvements to stability when syncing massive projects ([#1140])
* Added support for JSON comments and trailing commas in project, meta, and model json files ([#1144])
* Added `sourcemap.json` into the default `.gitignore` files ([#1145])

[7.6.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.6.1
[#1132]: https://github.com/rojo-rbx/rojo/pull/1132
[#1135]: https://github.com/rojo-rbx/rojo/pull/1135
[#1140]: https://github.com/rojo-rbx/rojo/pull/1140
[#1144]: https://github.com/rojo-rbx/rojo/pull/1144
[#1145]: https://github.com/rojo-rbx/rojo/pull/1145

## [7.6.0] (October 10th, 2025)

* Added flag to `rojo init` to skip initializing a git repository ([#1122])
* Added fallback method for when an Instance can't be synced through normal means ([#1030])

    This should make it possible to sync `MeshParts` and `Unions`!

    The fallback involves deleting and recreating Instances. This will break properties that reference them that Rojo does not know about, so be weary.

* Add auto-reconnect and improve UX for sync reminders ([#1096])
* Add support for syncing `yml` and `yaml` files (behaves similar to JSON and TOML) ([#1093])
* Fixed colors of Table diff ([#1084])
* Fixed `sourcemap` command outputting paths with OS-specific path separators ([#1085])
* Fixed nil -> nil properties showing up as failing to sync in plugin's patch visualizer ([#1081])
* Changed the background of the server's in-browser UI to be gray instead of white ([#1080])
* Fixed `Auto Connect Playtest Server` no longer functioning due to Roblox change ([#1066])
* Added an update indicator to the version header when a new version of the plugin is available. ([#1069])
* Added `--absolute` flag to the sourcemap subcommand, which will emit absolute paths instead of relative paths. ([#1092])
* Fixed applying `gameId` and `placeId` before initial sync was accepted ([#1104])

[7.6.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.6.0
[#1030]: https://github.com/rojo-rbx/rojo/pull/1030
[#1066]: https://github.com/rojo-rbx/rojo/pull/1066
[#1069]: https://github.com/rojo-rbx/rojo/pull/1069
[#1080]: https://github.com/rojo-rbx/rojo/pull/1080
[#1081]: https://github.com/rojo-rbx/rojo/pull/1081
[#1084]: https://github.com/rojo-rbx/rojo/pull/1084
[#1085]: https://github.com/rojo-rbx/rojo/pull/1085
[#1092]: https://github.com/rojo-rbx/rojo/pull/1092
[#1093]: https://github.com/rojo-rbx/rojo/pull/1093
[#1096]: https://github.com/rojo-rbx/rojo/pull/1096
[#1104]: https://github.com/rojo-rbx/rojo/pull/1104
[#1122]: https://github.com/rojo-rbx/rojo/pull/1122

## [7.5.1] (April 25th, 2025)

* Fixed output spam related to `Instance.Capabilities` in the plugin

[7.5.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.5.1

## [7.5.0] (April 25th, 2025)

* Fixed an edge case that caused model pivots to not be built correctly in some cases ([#1027])
* Add `blockedPlaceIds` project config field to allow blocking place ids from being live synced ([#1021])
* Adds support for `.plugin.lua(u)` files - this applies the `Plugin` RunContext. ([#1008])
* Added support for Roblox's `Content` type. This replaces the old `Content` type with `ContentId` to reflect Roblox's change.

    If you were previously using the fully-qualified syntax for `Content` you will need to switch it to `ContentId`.
* Added support for `Enum` attributes
* Significantly improved performance of `.rbxm` parsing
* Support for a `$schema` field in all special JSON files (`.project.json`, `.model.json`, and `.meta.json`) ([#974])
* Projects may now manually link `Ref` properties together using `Attributes`. ([#843])

    This has two parts: using `id` or `$id` in JSON files or a `Rojo_Target` attribute, an Instance
    is given an ID. Then, that ID may be used elsewhere in the project to point to an Instance
    using an attribute named `Rojo_Target_PROP_NAME`, where `PROP_NAME` is the name of a property.

    As an example, here is a `model.json` for an ObjectValue that refers to itself:

    ```json
    {
        "id": "arbitrary string",
        "attributes": {
            "Rojo_Target_Value": "arbitrary string"
        }
    }
    ```

    This is a very rough implementation and the usage will become more ergonomic
    over time.

* Updated Undo/Redo history to be more robust ([#915])
* Added popout diff visualizer for table properties like Attributes and Tags ([#834])
* Updated Theme to use Studio colors ([#838])
* Improved patch visualizer UX ([#883])
* Added update notifications for newer compatible versions in the Studio plugin. ([#832])
* Added experimental setting for Auto Connect in playtests ([#840])
* Improved settings UI ([#886])
* `Open Scripts Externally` option can now be changed while syncing ([#911])
* The sync reminder notification will now tell you what was last synced and when ([#987])
* Fixed notification and tooltip text sometimes getting cut off ([#988])
* Projects may now specify rules for syncing files as if they had a different file extension. ([#813])

    This is specified via a new field on project files, `syncRules`:

    ```json
    {
      "syncRules": [
        {
          "pattern": "*.foo",
          "use": "text",
          "exclude": "*.exclude.foo",
        },
        {
          "pattern": "*.bar.baz",
          "use": "json",
          "suffix": ".bar.baz",
        },
      ],
      "name": "SyncRulesAreCool",
      "tree": {
        "$path": "src"
      }
    }
    ```

    The `pattern` field is a glob used to match the sync rule to files. If present, the `suffix` field allows you to specify parts of a file's name get cut off by Rojo to name the Instance, including the file extension. If it isn't specified, Rojo will only cut off the first part of the file extension, up to the first dot.

    Additionally, the `exclude` field allows files to be excluded from the sync rule if they match a pattern specified by it. If it's not present, all files that match `pattern` will be modified using the sync rule.

    The `use` field corresponds to one of the potential file type that Rojo will currently include in a project. Files that match the provided pattern will be treated as if they had the file extension for that file type.

    | `use` value    | file extension  |
    |:---------------|:----------------|
    | `serverScript` | `.server.lua`   |
    | `clientScript` | `.client.lua`   |
    | `moduleScript` | `.lua`          |
    | `json`         | `.json`         |
    | `toml`         | `.toml`         |
    | `csv`          | `.csv`          |
    | `text`         | `.txt`          |
    | `jsonModel`    | `.model.json`   |
    | `rbxm`         | `.rbxm`         |
    | `rbxmx`        | `.rbxmx`        |
    | `project`      | `.project.json` |
    | `ignore`       | None!           |

    Additionally, there are `use` values for specific script types ([#909]):

    | `use` value              | script type                            |
    |:-------------------------|:---------------------------------------|
    | `legacyServerScript`     | `Script` with `Enum.RunContext.Legacy` |
    | `legacyClientScript`     | `LocalScript`                          |
    | `runContextServerScript` | `Script` with `Enum.RunContext.Server` |
    | `runContextClientScript` | `Script` with `Enum.RunContext.Client` |
    | `pluginScript`           | `Script` with `Enum.RunContext.Plugin` |

    **All** sync rules are reset between project files, so they must be specified in each one when nesting them. This is to ensure that nothing can break other projects by changing how files are synced!

[7.5.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.5.0
[#813]: https://github.com/rojo-rbx/rojo/pull/813
[#832]: https://github.com/rojo-rbx/rojo/pull/832
[#834]: https://github.com/rojo-rbx/rojo/pull/834
[#838]: https://github.com/rojo-rbx/rojo/pull/838
[#840]: https://github.com/rojo-rbx/rojo/pull/840
[#843]: https://github.com/rojo-rbx/rojo/pull/843
[#883]: https://github.com/rojo-rbx/rojo/pull/883
[#886]: https://github.com/rojo-rbx/rojo/pull/886
[#909]: https://github.com/rojo-rbx/rojo/pull/909
[#911]: https://github.com/rojo-rbx/rojo/pull/911
[#915]: https://github.com/rojo-rbx/rojo/pull/915
[#974]: https://github.com/rojo-rbx/rojo/pull/974
[#987]: https://github.com/rojo-rbx/rojo/pull/987
[#988]: https://github.com/rojo-rbx/rojo/pull/988
[#1008]: https://github.com/rojo-rbx/rojo/pull/1008
[#1021]: https://github.com/rojo-rbx/rojo/pull/1021
[#1027]: https://github.com/rojo-rbx/rojo/pull/1027

## [7.4.4] (August 22nd, 2024)

* Fixed issue with reading attributes from `Lighting` in new place files
* `Instance.Archivable` will now default to `true` when building a project into a binary (`rbxm`/`rbxl`) file rather than `false`.

[7.4.4]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.4

## [7.4.3] (August 6th, 2024)

* Fixed issue with building binary files introduced in 7.4.2
* Fixed `value of type nil cannot be converted to number` warning spam in output. [#955]

[7.4.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.3
[#955]: https://github.com/rojo-rbx/rojo/pull/955

## [7.4.2] (July 23, 2024)

* Added Never option to Confirmation ([#893])
* Fixed removing trailing newlines ([#903])
* Updated the internal property database, correcting an issue with `SurfaceAppearance.Color` that was reported [here][Surface_Appearance_Color_1] and [here][Surface_Appearance_Color_2] ([#948])

[7.4.2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.2
[#893]: https://github.com/rojo-rbx/rojo/pull/893
[#903]: https://github.com/rojo-rbx/rojo/pull/903
[#948]: https://github.com/rojo-rbx/rojo/pull/948
[Surface_Appearance_Color_1]: https://devforum.roblox.com/t/jailbreak-custom-character-turned-shiny-black-no-texture/3075563
[Surface_Appearance_Color_2]: https://devforum.roblox.com/t/surfaceappearance-not-displaying-correctly/3075588

## [7.4.1] (February 20, 2024)

* Made the `name` field optional on project files ([#870])

    Files named `default.project.json` inherit the name of the folder they're in and all other projects
    are named as expect (e.g. `foo.project.json` becomes an Instance named `foo`)

    There is no change in behavior if `name` is set.
* Fixed incorrect results when building model pivots ([#865])
* Fixed incorrect results when serving model pivots ([#868])
* Rojo now converts any line endings to LF, preventing spurious diffs when syncing Lua files on Windows ([#854])
* Fixed Rojo plugin failing to connect when project contains certain unreadable properties ([#848])
* Fixed various cases where patch visualizer would not display sync failures ([#845], [#844])
* Fixed http error handling so Rojo can be used in Github Codespaces ([#847])

[7.4.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.1
[#844]: https://github.com/rojo-rbx/rojo/pull/844
[#845]: https://github.com/rojo-rbx/rojo/pull/845
[#847]: https://github.com/rojo-rbx/rojo/pull/847
[#848]: https://github.com/rojo-rbx/rojo/pull/848
[#854]: https://github.com/rojo-rbx/rojo/pull/854
[#865]: https://github.com/rojo-rbx/rojo/pull/865
[#868]: https://github.com/rojo-rbx/rojo/pull/868
[#870]: https://github.com/rojo-rbx/rojo/pull/870

## [7.4.0] (January 16, 2024)

* Improved the visualization for array properties like Tags ([#829])
* Significantly improved performance of `rojo serve`, `rojo build --watch`, and `rojo sourcemap --watch` on macOS. ([#830])
* Changed `*.lua` files that init command generates to `*.luau` ([#831])
* Does not remind users to sync if the sync lock is claimed already ([#833])

[7.4.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0
[#829]: https://github.com/rojo-rbx/rojo/pull/829
[#830]: https://github.com/rojo-rbx/rojo/pull/830
[#831]: https://github.com/rojo-rbx/rojo/pull/831
[#833]: https://github.com/rojo-rbx/rojo/pull/833

## [7.4.0-rc3] (October 25, 2023)

* Changed `sourcemap --watch` to only generate the sourcemap when it's necessary ([#800])
* Switched script source property getter and setter to `ScriptEditorService` methods ([#801])

    This ensures that the script editor reflects any changes Rojo makes to a script while it is open in the script editor.

* Fixed issues when handling `SecurityCapabilities` values ([#803], [#807])
* Fixed Rojo plugin erroring out when attempting to sync attributes with invalid names ([#809])

[7.4.0-rc3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc3
[#800]: https://github.com/rojo-rbx/rojo/pull/800
[#801]: https://github.com/rojo-rbx/rojo/pull/801
[#803]: https://github.com/rojo-rbx/rojo/pull/803
[#807]: https://github.com/rojo-rbx/rojo/pull/807
[#809]: https://github.com/rojo-rbx/rojo/pull/809

## [7.4.0-rc2] (October 3, 2023)

* Fixed bug with parsing version for plugin validation ([#797])

[7.4.0-rc2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc2
[#797]: https://github.com/rojo-rbx/rojo/pull/797

## [7.4.0-rc1] (October 3, 2023)

### Additions

#### Project format

* Added support for `.toml` files to `$path` ([#633])
* Added support for `Font` and `CFrame` attributes ([rbx-dom#299], [rbx-dom#296])
* Added the `emitLegacyScripts` field to the project format ([#765]). The behavior is outlined below:

    | `emitLegacyScripts` Value | Action Taken by Rojo                                                                                             |
    |---------------------------|------------------------------------------------------------------------------------------------------------------|
    | false                     | Rojo emits Scripts with the appropriate `RunContext` for `*.client.lua` and `*.server.lua` files in the project. |
    | true   (default)          | Rojo emits LocalScripts and Scripts with legacy `RunContext` (same behavior as previously).                      |

    It can be used like this:

    ```json
    {
      "emitLegacyScripts": false,
      "name": "MyCoolRunContextProject",
      "tree": {
        "$path": "src"
      }
    }
    ```

* Added `Terrain` classname inference, similar to services ([#771])

    `Terrain` may now be defined in projects without using `$className`:

    ```json
    "Workspace": {
      "Terrain": {
        "$path": "path/to/terrain.rbxm"
      }
    }
    ```

* Added support for `Terrain.MaterialColors` ([#770])

    `Terrain.MaterialColors` is now represented in projects in a human readable format:

    ```json
    "Workspace": {
      "Terrain": {
        "$path": "path/to/terrain.rbxm"
        "$properties": {
          "MaterialColors": {
            "Grass": [10, 20, 30],
            "Asphalt": [40, 50, 60],
            "LeafyGrass": [255, 155, 55]
          }
        }
      }
    }
    ```

* Added better support for `Font` properties ([#731])

    `FontFace` properties may now be defined using implicit property syntax:

    ```json
    "TextBox": {
      "$className": "TextBox",
      "$properties": {
        "FontFace": {
          "family": "rbxasset://fonts/families/RobotoMono.json",
          "weight": "Thin",
          "style": "Normal"
        }
      }
    }
    ```

#### Patch visualizer and notifications

* Added a setting to control patch confirmation behavior ([#774])

    This is a new setting for controlling when the Rojo plugin prompts for confirmation before syncing. It has four options:
  * Initial (default): prompts only once for a project in a given Studio session
  * Always: always prompts for confirmation
  * Large Changes: only prompts when there are more than X changed instances. The number of instances is configurable - an additional setting for the number of instances becomes available when this option is chosen
  * Unlisted PlaceId: only prompts if the place ID is not present in servePlaceIds

* Added the ability to select Instances in patch visualizer ([#709])

    Double-clicking an instance in the patch visualizer sets Roblox Studio's selection to the instance.

* Added a sync reminder notification. ([#689])

    Rojo detects if you have previously synced to a place, and displays a notification reminding you to sync again:

    ![Rojo reminds you to sync a place that you've synced previously](https://user-images.githubusercontent.com/40185666/242397435-ccdfddf2-a63f-420c-bc18-a6e3d6455bba.png)

* Added rich Source diffs in patch visualizer ([#748])

    A "View Diff" button for script sources is now present in the patch visualizer. Clicking it displays a side-by-side diff of the script changes:

    ![The patch visualizer contains a "view diff" button](https://user-images.githubusercontent.com/40185666/256065992-3f03558f-84b0-45a1-80eb-901f348cf067.png)

    ![The "View Diff" button opens a widget that displays a diff](https://user-images.githubusercontent.com/40185666/256066084-1d9d8fe8-7dad-4ee7-a542-b4aee35a5644.png)

* Patch visualizer now indicates what changes failed to apply. ([#717])

    A clickable warning label is displayed when the Rojo plugin is unable to apply changes. Clicking the label displays precise information about which changes failed:

    ![Patch visualizer displays a clickable warning label when changes fail to apply](https://user-images.githubusercontent.com/40185666/252063660-f08399ef-1e16-4f1c-bed8-552821f98cef.png)

#### Miscellaneous

* Added `plugin` flag to the `build` command that outputs to the local plugins folder ([#735])

    This is a flag that builds a Rojo project into Roblox Studio's plugins directory. This allows you to build a Rojo project and load it into Studio as a plugin without having to type the full path to the plugins directory. It can be used like this: `rojo build <PATH-TO-PROJECT> --plugin <FILE-NAME>`

* Added new plugin template to the `init` command ([#738])

    This is a new template geared towards plugins. It is similar to the model template, but creates a `Script` instead of a `ModuleScript` in the `src` directory. It can be used like this: `rojo init --kind plugin`

* Added protection against syncing non-place projects as a place. ([#691])
* Add buttons for navigation on the Connected page ([#722])

### Fixes

* Significantly improved performance of `rojo sourcemap` ([#668])
* Fixed the diff visualizer of connected sessions. ([#674])
* Fixed disconnected session activity. ([#675])
* Skip confirming patches that contain only a datamodel name change. ([#688])
* Fix Rojo breaking when users undo/redo in Studio ([#708])
* Improve tooltip behavior ([#723])
* Better settings controls ([#725])
* Rework patch visualizer with many fixes and improvements ([#713], [#726], [#755])

[7.4.0-rc1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.4.0-rc1
[#633]: https://github.com/rojo-rbx/rojo/pull/633
[#668]: https://github.com/rojo-rbx/rojo/pull/668
[#674]: https://github.com/rojo-rbx/rojo/pull/674
[#675]: https://github.com/rojo-rbx/rojo/pull/675
[#688]: https://github.com/rojo-rbx/rojo/pull/688
[#689]: https://github.com/rojo-rbx/rojo/pull/689
[#691]: https://github.com/rojo-rbx/rojo/pull/691
[#708]: https://github.com/rojo-rbx/rojo/pull/708
[#709]: https://github.com/rojo-rbx/rojo/pull/709
[#713]: https://github.com/rojo-rbx/rojo/pull/713
[#717]: https://github.com/rojo-rbx/rojo/pull/717
[#722]: https://github.com/rojo-rbx/rojo/pull/722
[#723]: https://github.com/rojo-rbx/rojo/pull/723
[#725]: https://github.com/rojo-rbx/rojo/pull/725
[#726]: https://github.com/rojo-rbx/rojo/pull/726
[#731]: https://github.com/rojo-rbx/rojo/pull/731
[#735]: https://github.com/rojo-rbx/rojo/pull/735
[#738]: https://github.com/rojo-rbx/rojo/pull/738
[#748]: https://github.com/rojo-rbx/rojo/pull/748
[#755]: https://github.com/rojo-rbx/rojo/pull/755
[#765]: https://github.com/rojo-rbx/rojo/pull/765
[#770]: https://github.com/rojo-rbx/rojo/pull/770
[#771]: https://github.com/rojo-rbx/rojo/pull/771
[#774]: https://github.com/rojo-rbx/rojo/pull/774
[rbx-dom#296]: https://github.com/rojo-rbx/rbx-dom/pull/296
[rbx-dom#299]: https://github.com/rojo-rbx/rbx-dom/pull/299

## [7.3.0] (April 22, 2023)

* Added `$attributes` to project format. ([#574])
* Added `--watch` flag to `rojo sourcemap`. ([#602])
* Added support for `init.csv` files. ([#594])
* Added real-time sync status to the Studio plugin. ([#569])
* Added support for copying error messages to the clipboard. ([#614])
* Added sync locking for Team Create. ([#590])
* Added support for specifying HTTP or HTTPS protocol in plugin. ([#642])
* Added tooltips to buttons in the Studio plugin. ([#637])
* Added visual diffs when connecting from the Studio plugin. ([#603])
* Host and port are now saved in the Studio plugin. ([#613])
* Improved padding on notifications in Studio plugin. ([#589])
* Renamed `Common` to `Shared` in the default Rojo project. ([#611])
* Reduced the minimum size of the Studio plugin widget. ([#606])
* Fixed current directory in `rojo fmt-project`. ([#581])
* Fixed errors after a session has already ended. ([#587])
* Fixed an uncommon security permission error ([#619])

[7.3.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.3.0
[#569]: https://github.com/rojo-rbx/rojo/pull/569
[#574]: https://github.com/rojo-rbx/rojo/pull/574
[#581]: https://github.com/rojo-rbx/rojo/pull/581
[#587]: https://github.com/rojo-rbx/rojo/pull/587
[#589]: https://github.com/rojo-rbx/rojo/pull/589
[#590]: https://github.com/rojo-rbx/rojo/pull/590
[#594]: https://github.com/rojo-rbx/rojo/pull/594
[#602]: https://github.com/rojo-rbx/rojo/pull/602
[#603]: https://github.com/rojo-rbx/rojo/pull/603
[#606]: https://github.com/rojo-rbx/rojo/pull/606
[#611]: https://github.com/rojo-rbx/rojo/pull/611
[#613]: https://github.com/rojo-rbx/rojo/pull/613
[#614]: https://github.com/rojo-rbx/rojo/pull/614
[#619]: https://github.com/rojo-rbx/rojo/pull/619
[#637]: https://github.com/rojo-rbx/rojo/pull/637
[#642]: https://github.com/rojo-rbx/rojo/pull/642

## [7.2.1] (July 8, 2022)

* Fixed notification sound by changing it to a generic sound. ([#566])
* Added setting to turn off sound effects. ([#568])

[7.2.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.2.1
[#566]: https://github.com/rojo-rbx/rojo/pull/566
[#568]: https://github.com/rojo-rbx/rojo/pull/568

## [7.2.0] (June 29, 2022)

* Added support for `.luau` files. ([#552])
* Added support for live syncing Attributes and Tags. ([#553])
* Added notification popups in the Roblox Studio plugin. ([#540])
* Fixed `init.meta.json` when used with `init.lua` and related files. ([#549])
* Fixed incorrect output when serving from a non-default address or port ([#556])
* Fixed Linux binaries not running on systems with older glibc. ([#561])
* Added `camelCase` casing for JSON models, deprecating `PascalCase` names. ([#563])
* Switched from structopt to clap for command line argument parsing.
* Significantly improved performance of building and serving. ([#548])
* Increased minimum supported Rust version to 1.57.0. ([#564])

[7.2.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.2.0
[#540]: https://github.com/rojo-rbx/rojo/pull/540
[#548]: https://github.com/rojo-rbx/rojo/pull/548
[#549]: https://github.com/rojo-rbx/rojo/pull/549
[#552]: https://github.com/rojo-rbx/rojo/pull/552
[#553]: https://github.com/rojo-rbx/rojo/pull/553
[#556]: https://github.com/rojo-rbx/rojo/pull/556
[#561]: https://github.com/rojo-rbx/rojo/pull/561
[#563]: https://github.com/rojo-rbx/rojo/pull/563
[#564]: https://github.com/rojo-rbx/rojo/pull/564

## [7.1.1] (May 26, 2022)

* Fixed sourcemap command not stripping paths correctly ([#544])
* Fixed Studio plugin settings not saving correctly.

[7.1.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.1.1
[#544]: https://github.com/rojo-rbx/rojo/pull/544

## [7.1.0] (May 22, 2022)

* Added support for specifying an address to be used by default in project files. ([#507])
* Added support for optional paths in project files. ([#472])
* Added support for the new Open Cloud API when uploading. ([#504])
* Added `sourcemap` command for generating sourcemaps to feed into other tools. ([#530])
* Added PluginActions for connecting/disconnecting a session ([#537])
* Added changing toolbar icon to indicate state ([#538])

[7.1.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.1.0
[#472]: https://github.com/rojo-rbx/rojo/pull/472
[#504]: https://github.com/rojo-rbx/rojo/pull/504
[#507]: https://github.com/rojo-rbx/rojo/pull/507
[#530]: https://github.com/rojo-rbx/rojo/pull/530
[#537]: https://github.com/rojo-rbx/rojo/pull/537
[#538]: https://github.com/rojo-rbx/rojo/pull/538

## [7.0.0] (December 10, 2021)

* Fixed Rojo's interactions with properties enabled by FFlags that are not yet enabled. ([#493])
* Improved output in Roblox Studio plugin when bad property data is encountered.
* Reintroduced support for CFrame shorthand syntax in Rojo project and `.meta.json` files, matching Rojo 6. ([#430])
* Connection settings are now remembered when reconnecting in Roblox Studio. ([#500])
* Updated reflection database to Roblox v503.

[7.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0
[#430]: https://github.com/rojo-rbx/rojo/issues/430
[#493]: https://github.com/rojo-rbx/rojo/pull/493
[#500]: https://github.com/rojo-rbx/rojo/pull/500

## [7.0.0-rc.3] (October 19, 2021)

This is the last release candidate for Rojo 7. In an effort to get Rojo 7 out the door, we'll be freezing features from here on out, something we should've done a couple months ago.

Expect to see Rojo 7 stable soon!

* Added support for writing `Tags` in project files, model files, and meta files. ([#484])
* Adjusted Studio plugin colors to match Roblox Studio palette. ([#482])
* Improved experimental two-way sync feature by batching changes. ([#478])

[7.0.0-rc.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-rc.3
[#478]: https://github.com/rojo-rbx/rojo/pull/478
[#482]: https://github.com/rojo-rbx/rojo/pull/482
[#484]: https://github.com/rojo-rbx/rojo/pull/484

## 7.0.0-rc.2 (October 19, 2021)

(Botched release due to Git mishap, oops!)

## [7.0.0-rc.1] (August 23, 2021)

In Rojo 6 and previous Rojo 7 alphas, an explicit Vector3 property would be written like this:

```json
{
    "className": "Part",
    "properties": {
        "Position": {
            "Type": "Vector3",
            "Value": [1, 2, 3]
        }
    }
}
```

For Rojo 7, this will need to be changed to:

```json
{
    "className": "Part",
    "properties": {
        "Position": {
            "Vector3": [1, 2, 3]
        }
    }
}
```

The shorthand property format that most users use is not impacted. For reference, it looks like this:

```json
{
    "className": "Part",
    "properties": {
        "Position": [1, 2, 3]
    }
}
```

* Major breaking change: changed property syntax for project files; shorthand syntax is unchanged.
* Added the `fmt-project` subcommand for formatting Rojo project files.
* Improved error output for many subcommands.
* Updated to stable versions of rbx-dom libraries.
* Updated async infrastructure, which should fix a handful of bugs. ([#459])
* Fixed syncing refs in the Roblox Studio plugin ([#462], [#466])
* Added support for long paths on Windows. ([#464])

[7.0.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-rc.1
[#459]: https://github.com/rojo-rbx/rojo/pull/459
[#462]: https://github.com/rojo-rbx/rojo/pull/462
[#464]: https://github.com/rojo-rbx/rojo/pull/464
[#466]: https://github.com/rojo-rbx/rojo/pull/466

## [7.0.0-alpha.4] (May 5, 2021)

* Added the `gameId` and `placeId` optional properties to project files.

  * When connecting from the Rojo Roblox Studio plugin, Rojo will set the game and place ID of the current place to these values, if set.
  * This is equivalent to running `game:SetUniverseId(...)` and `game:SetPlaceId(...)` from the command bar in Studio.
* Added "EXPERIMENTAL!" label to two-way sync toggle in Rojo's Roblox Studio plugin.
* Fixed `Name` and `Parent` properties being allowed in Rojo projects. ([#413])
* Fixed "Open Scripts Externally" feature crashing Studio. ([#369])
* Empty `.model.json` files will no longer cause errors. ([#420])
* When specifying `$path` on a service, Rojo now keeps the correct class name. ([#331])
* Improved error messages for misconfigured projects.

[7.0.0-alpha.4]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.4
[#331]: https://github.com/rojo-rbx/rojo/issues/331
[#369]: https://github.com/rojo-rbx/rojo/issues/369
[#413]: https://github.com/rojo-rbx/rojo/pull/413
[#420]: https://github.com/rojo-rbx/rojo/pull/420

## [7.0.0-alpha.3] (February 19, 2021)

* Updated dependencies, fixing `OptionalCoordinateFrame`-related issues.
* Added `--address` flag to `rojo serve` to allow for external connections. ([#403])

[7.0.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.3
[#403]: https://github.com/rojo-rbx/rojo/pull/403

## [7.0.0-alpha.2] (February 19, 2021)

* Fixed incorrect protocol version between the client and server.

[7.0.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.2

## [7.0.0-alpha.1] (February 18, 2021)

This release includes a brand new implementation of the Roblox DOM. It brings performance improvements, much better support for `rbxl` and `rbxm` files, and a better internal API.

* Added support for all remaining property types.
* Added support for the entire Roblox binary model format.
* Changed `rojo upload` to upload binary places and models instead of XML.
  * This should make using `rojo upload` much more feasible for large places.
* **Breaking**: Changed format of some types of values in `project.json`, `model.json`, and `meta.json` files.
  * This should impact few projects. See [this file][allValues.json] for new examples of each property type.

Formatting of types will change more before the stable release of Rojo 7. We're hoping to use this opportunity to normalize some of the case inconsistency introduced in Rojo 0.5.

[7.0.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v7.0.0-alpha.1
[allValues.json]: https://github.com/rojo-rbx/rojo/blob/f4a790eb50b74e482000bad1dcfe22533992fb20/plugin/rbx_dom_lua/src/allValues.json

## [6.0.2] (February 9, 2021)

* Fixed `rojo upload` to handle CSRF challenges.

[6.0.2]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.2

## [6.0.1] (January 22, 2021)

* Fixed `rojo upload` requests being rejected by Roblox

[6.0.1]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.1

## [6.0.0] (January 16, 2021)

* Improved server error messages
  * The server will now keep running in more error cases
* Fixed Rojo being unable to diff ClassName changes

[6.0.0]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0

## [6.0.0-rc.4] (December 14, 2020)

* Added brand new Rojo UI ([#367])
* Added `projectName` to `/api/rojo` output.

[6.0.0-rc.4]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.4
[#367]: https://github.com/rojo-rbx/rojo/pull/367

## [6.0.0-rc.3] (November 19, 2020)

* Fixed the Rojo plugin attempting to write the non-scriptable properties `Instance.SourceAssetId` and `HttpServer.HttpEnabled`.
* Fixed the Rojo plugin's handling of null referents.

[6.0.0-rc.3]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.3

## [6.0.0-rc.2] (November 19, 2020)

* Fixed crash when malformed CSV files are put into a project. ([#310])
* Fixed incorrect string escaping when producing Lua code from JSON files. ([#314])
* Fixed performance issues introduced in Rojo 6.0.0-rc.1. ([#317])
* Fixed `rojo plugin install` subcommand failing for everyone except Rojo developers. ([#320])
* Updated default place template to take advantage of [#210].
* Enabled glob ignore patterns by default and removed the `unstable_glob_ignore` feature.
  * `globIgnorePaths` can be set on a project to a list of globs to ignore.
* The Rojo plugin now completes as much as it can from a patch without disconnecting. Warnings are shown in the console.
* Fixed 6.0.0-rc.1 regression causing instances that changed ClassName to instead... not change ClassName.

[6.0.0-rc.2]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.2
[#210]: https://github.com/rojo-rbx/rojo/pull/210
[#310]: https://github.com/rojo-rbx/rojo/issues/310
[#314]: https://github.com/rojo-rbx/rojo/issues/314
[#317]: https://github.com/rojo-rbx/rojo/issues/317
[#320]: https://github.com/rojo-rbx/rojo/issues/320

## [6.0.0-rc.1] (March 29, 2020)

This release jumped from 0.6.0 to 6.0.0. Rojo has been in use in production for many users for quite a long times, and so 6.0 is a more accurate reflection of Rojo's version than a pre-1.0 version.

* Added basic settings panel to plugin, with two settings:
  * "Open Scripts Externally": When enabled, opening a script in Studio will instead open it in your default text editor.
  * "Two-Way Sync": When enabled, Rojo will attempt to save changes to your place back to the filesystem. **Very early feature, very broken, beware!**
* Added `--color` option to force-enable or force-disable color in Rojo's output.
* Added support for turning `.json` files into `ModuleScript` instances ([#308])
* Added `rojo plugin install` and `rojo plugin uninstall` to allow Rojo to manage its Roblox Studio plugin. ([#304])
* Class names no longer need to be specified for Roblox services in Rojo projects. ([#210])
* The server half of **experimental** two-way sync is now enabled by default.
* Increased default logging verbosity in commands like `rojo build`.
* Rojo now requires a project file again, just like 0.5.4.

[6.0.0-rc.1]: https://github.com/rojo-rbx/rojo/releases/tag/v6.0.0-rc.1
[#304]: https://github.com/rojo-rbx/rojo/pull/304
[#308]: https://github.com/rojo-rbx/rojo/pull/308

## [0.6.0-alpha.3] (March 13, 2020)

* Added `--watch` argument to `rojo build`. ([#284])
* Added dark theme support to plugin. ([#241])
* Added a revamped `rojo init` command, which will now create more complete projects.
* Added the `rojo doc` command, which opens Rojo's documentation in your browser.
* Fixed many crashes from malformed projects and filesystem edge cases in `rojo serve`.
* Simplified filesystem access code dramatically.
* Improved error reporting and logging across the board.
  * Log messages have a less noisy prefix.
  * Any thread panicking now causes Rojo to abort instead of existing as a zombie.
  * Errors now have a list of causes, helping make many errors more clear.

[0.6.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.3
[#241]: https://github.com/rojo-rbx/rojo/issues/241
[#284]: https://github.com/rojo-rbx/rojo/pull/284

## [0.6.0-alpha.2] (March 6, 2020)

* Fixed `rojo upload` command always uploading models.
* Removed `--kind` parameter to `rojo upload`; Rojo now automatically uploads the correct kind of asset based on your project file.

[0.6.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.2

## [0.5.4] (February 26, 2020)

This is a general maintenance release for the Rojo 0.5.x release series.

* Updated reflection database and other dependencies.
* First stable release with binaries for macOS and Linux.

[0.5.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.4

## [0.6.0-alpha.1] (January 22, 2020)

### General

* Added support for nested project files. ([#95])
* Added project file hot-reloading. ([#10])
* Fixed Rojo dropping Ref properties ([#142])
  * This means that properties like `PrimaryPart` now work!
* Improved live sync protocol to reduce round-trips and improve syncing consistency.
* Improved support for binary model files and places.

### Command Line

* Added `--verbose`/`-v` flag, which can be specified multiple times to increase verbosity.
* Added support for automatically finding Roblox Studio's auth cookie for `rojo upload` on Windows.
* Added support for building, serving and uploading sources that aren't Rojo projects.
* Improved feedback from `rojo serve`.
* Removed support for legacy `roblox-project.json` projects, deprecated in an early Rojo 0.5.0 alpha.
* Rojo no longer traverses directories upwards looking for project files.
  * Though undocumented, Rojo 0.5.x will search for a project file contained in any ancestor folders. This feature was removed to better support other 0.6.x features.

### Roblox Studio Plugin

* Added "connecting" state to improve experience when live syncing.
* Added "error" state to show errors in a place that isn't the output panel.
* Improved diagnostics for when the Rojo plugin cannot create an instance.

[0.6.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.6.0-alpha.1
[#10]: https://github.com/rojo-rbx/rojo/issues/10
[#95]: https://github.com/rojo-rbx/rojo/issues/95
[#142]: https://github.com/rojo-rbx/rojo/issues/142

## [0.5.3] (October 15, 2019)

* Fixed an issue where Rojo would throw an error when encountering recently-added instance classes.

[0.5.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.3

## [0.5.2] (October 14, 2019)

* Fixed an issue where `LocalizationTable` instances would have their column order randomized. ([#173])

[0.5.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.2
[#173]: https://github.com/rojo-rbx/rojo/issues/173

## [0.5.1] (October 4, 2019)

* Fixed an issue where Rojo would drop changes if they happened too quickly ([#252])
* Improved diagnostics for when the Rojo plugin cannot create an instance.
* Updated dependencies
  * This brings Rojo's reflection database from client release 395 to client release 404.

[0.5.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.1
[#252]: https://github.com/rojo-rbx/rojo/issues/252

## [0.5.0] (August 27, 2019)

* Changed `.model.json` naming, which may require projects to migrate ambiguous cases:
  * The file name now takes precedence over the `Name` field in the model, like Rojo 0.4.x.
  * The `Name` field of the top-level instance is now optional. It's recommended that you remove it from your models.
  * Rojo will emit a warning when `Name` is specified and does not match the name from the file.
* Fixed `Rect` values being set to `0, 0, 0, 0` when synced with the Rojo plugin. ([#201])
* Fixed live-syncing of `PhysicalProperties`, `NumberSequence`, and `ColorSequence` values

[0.5.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0
[#201]: https://github.com/rojo-rbx/rojo/issues/201

## [0.5.0-alpha.13] (August 2, 2019)

* Bumped minimum Rust version to 1.34.0.
* Fixed default port documentation in `rojo serve --help` ([#219])
* Fixed BrickColor support by upgrading Roblox-related dependencies

[0.5.0-alpha.13]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.13
[#219]: https://github.com/rojo-rbx/rojo/issues/219

## [0.5.0-alpha.12] (July 2, 2019)

* Added `.meta.json` files
  * `init.meta.json` files replace `init.model.json` files from Rojo 0.4.x ([#183])
  * Other `.meta.json` files allow attaching extra data to other files ([#189])
* Added support for infinite and NaN values in types like `Vector2` when building models and places.
  * These types aren't supported for live-syncing yet due to limitations around JSON encoding.
* Added support for using `SharedString` values when building XML models and places.
* Added support for live-syncing `CollectionService` tags.
* Added a warning when building binary place files, since they're still experimental and have bugs.
* Added a warning when trying to use Rojo 0.5.x with a Rojo 0.4.x-only project.
* Added a warning when a Rojo project contains keys that start with `$`, which are reserved names. ([#191])
* Rojo now throws an error if unknown keys are found most files.
* Added an icon to the plugin's toolbar button
* Changed the plugin to use a docking widget for all UI.
* Changed the plugin to ignore unknown properties when live-syncing.
  * Rojo's approach to this problem might change later, like with a strict model mode ([#190]) or another approach.
* Upgraded to reflection database from client release 388.
* Updated Rojo's branding to shift the color palette to make it work better on dark backgrounds

[0.5.0-alpha.12]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.12
[#183]: https://github.com/rojo-rbx/rojo/pull/183
[#189]: https://github.com/rojo-rbx/rojo/pull/189
[#190]: https://github.com/rojo-rbx/rojo/issues/190
[#191]: https://github.com/rojo-rbx/rojo/issues/191

## [0.5.0-alpha.11] (May 29, 2019)

* Added support for implicit property values in JSON model files ([#154])
* `Content` properties can now be specified in projects and model files as regular string literals.
* Added support for `BrickColor` properties.
* Added support for properties added in client release 384, like `Lighting.Technology` being set to `"ShadowMap"`.
* Improved performance when working with XML models and places
* Fixed serializing empty `Content` properties as XML
* Fixed serializing infinite and NaN floating point properties in XML
* Improved compatibility with XML models
* Plugin should now be able to live-sync more properties, and ignore ones it can't, like `Lighting.Technology`.

[0.5.0-alpha.11]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.11
[#154]: https://github.com/rojo-rbx/rojo/pull/154

## 0.5.0-alpha.10

* This release was a dud due to [#176] and was rolled back.

[#176]: https://github.com/rojo-rbx/rojo/issues/176

## [0.5.0-alpha.9] (April 4, 2019)

* Changed `rojo build` to use buffered I/O, which can make it up to 2x faster in some cases.
  * Building [*Road Not Taken*](https://github.com/LPGhatguy/roads) to an `rbxlx` file dropped from 150ms to 70ms on my machine
* Fixed `LocalizationTable` instances being made from `csv` files incorrectly interpreting empty rows and columns. ([#149])
* Fixed CSV files with entries that parse as numbers causing Rojo to panic. ([#152])
* Improved error messages when malformed CSV files are found in a Rojo project.

[0.5.0-alpha.9]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.9
[#149]: https://github.com/rojo-rbx/rojo/pull/149
[#152]: https://github.com/rojo-rbx/rojo/pull/152

## [0.5.0-alpha.8] (March 29, 2019)

* Added support for a bunch of new types when dealing with XML model/place files:
  * `ColorSequence`
  * `Float64`
  * `Int64`
  * `NumberRange`
  * `NumberSequence`
  * `PhysicalProperties`
  * `Ray`
  * `Rect`
  * `Ref`
* Improved server instance ordering behavior when files are added during a live session ([#135])
* Fixed error being thrown when trying to unload the Rojo plugin.
* Added partial fix for [#141] for `Lighting.Technology`, which should restore live sync functionality for the default project file.

[0.5.0-alpha.8]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.8
[#135]: https://github.com/rojo-rbx/rojo/pull/135
[#141]: https://github.com/rojo-rbx/rojo/issues/141

## [0.5.0-alpha.6] (March 19, 2019)

* Fixed `rojo init` giving unexpected results by upgrading to `rbx_dom_weak` 1.1.0
* Fixed live server not responding when the Rojo plugin is connected ([#133])
* Updated default place file:
  * Improved default properties to be closer to Studio's built-in 'Baseplate' template
  * Added a baseplate to the project file (Thanks, [@AmaranthineCodices](https://github.com/AmaranthineCodices/)!)
* Added more type support to Rojo plugin
* Fixed some cases where the Rojo plugin would leave around objects that it knows should be deleted
* Updated plugin to correctly listen to `Plugin.Unloading` when installing or uninstalling new plugins

[0.5.0-alpha.6]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.6
[#133]: https://github.com/rojo-rbx/rojo/issues/133

## [0.5.0-alpha.5] (March 1, 2019)

* Upgraded core dependencies, which improves compatibility for lots of instance types
  * Upgraded from `rbx_tree` 0.2.0 to `rbx_dom_weak` 1.0.0
  * Upgraded from `rbx_xml` 0.2.0 to `rbx_xml` 0.4.0
  * Upgraded from `rbx_binary` 0.2.0 to `rbx_binary` 0.4.0
* Added support for non-primitive types in the Rojo plugin.
  * Types like `Color3` and `CFrame` can now be updated live!
* Fixed plugin assets flashing in on first load ([#121])
* Changed Rojo's HTTP server from Rouille to Hyper, which reduced the release size by around a megabyte.
* Added property type inference to projects, which makes specifying services a lot easier ([#130])
* Made error messages from invalid and missing files more user-friendly

[0.5.0-alpha.5]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.5
[#121]: https://github.com/rojo-rbx/rojo/issues/121
[#130]: https://github.com/rojo-rbx/rojo/pull/130

## [0.5.0-alpha.4] (February 8, 2019)

* Added support for nested partitions ([#102])
* Added support for 'transmuting' partitions ([#112])
* Added support for aliasing filesystem paths ([#105])
* Changed Windows builds to statically link the CRT ([#89])

[0.5.0-alpha.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.4
[#89]: https://github.com/rojo-rbx/rojo/issues/89
[#102]: https://github.com/rojo-rbx/rojo/issues/102
[#105]: https://github.com/rojo-rbx/rojo/issues/105
[#112]: https://github.com/rojo-rbx/rojo/issues/112

## [0.5.0-alpha.3] (February 1, 2019)

* Changed default project file name from `roblox-project.json` to `default.project.json` ([#120])
  * The old file name will still be supported until 0.5.0 is fully released.
* Added warning when loading project files that don't end in `.project.json`
  * This new extension enables Rojo to distinguish project files from random JSON files, which is necessary to support nested projects.
* Added new (empty) diagnostic page served from the server
* Added better error messages for when a file is missing that's referenced by a Rojo project
* Added support for visualization endpoints returning GraphViz source when Dot is not available
* Fixed an in-memory filesystem regression introduced recently ([#119])

[0.5.0-alpha.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.3
[#119]: https://github.com/rojo-rbx/rojo/pull/119
[#120]: https://github.com/rojo-rbx/rojo/pull/120

## [0.5.0-alpha.2] (January 28, 2019)

* Added support for `.model.json` files, compatible with 0.4.x
* Fixed in-memory filesystem not handling out-of-order filesystem change events
* Fixed long-polling error caused by a promise mixup ([#110])

[0.5.0-alpha.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.2
[#110]: https://github.com/rojo-rbx/rojo/issues/110

## [0.5.0-alpha.1] (January 25, 2019)

* Changed plugin UI to be way prettier
  * Thanks to [Reselim](https://github.com/Reselim) for the design!
* Changed plugin error messages to be a little more useful
* Removed unused 'Config' button in plugin UI
* Fixed bug where bad server responses could cause the plugin to be in a bad state
* Upgraded to rbx\_tree, rbx\_xml, and rbx\_binary 0.2.0, which dramatically expands the kinds of properties that Rojo can handle, especially in XML.

[0.5.0-alpha.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.1

## [0.5.0-alpha.0] (January 14, 2019)

* "Epiphany" rewrite, in progress since the beginning of time
* New live sync protocol
  * Uses HTTP long polling to reduce request count and improve responsiveness
* New project format
  * Hierarchical, preventing overlapping partitions
* Added `rojo build` command
  * Generates `rbxm`, `rbxmx`, `rbxl`, or `rbxlx` files out of your project
  * Usage: `rojo build <PROJECT> --output <OUTPUT>.rbxm`
* Added `rojo upload` command
  * Generates and uploads a place or model to roblox.com out of your project
  * Usage: `rojo upload <PROJECT> --cookie "<ROBLOSECURITY>" --asset_id <PLACE_ID>`
* New plugin
  * Only one button now, "Connect"
  * New UI to pick server address and port
  * Better error reporting
* Added support for `.csv` files turning into `LocalizationTable` instances
* Added support for `.txt` files turning into `StringValue` instances
* Added debug visualization code to diagnose problems
  * `/visualize/rbx` and `/visualize/imfs` show instance and file state respectively; they require GraphViz to be installed on your machine.
* Added optional place ID restrictions to project files
  * This helps prevent syncing in content to the wrong place
  * Multiple places can be specified, like when building a multi-place game
* Added support for specifying properties on services in project files

[0.5.0-alpha.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.5.0-alpha.0

## [0.4.13] (November 12, 2018)

* When `rojo.json` points to a file or directory that does not exist, Rojo now issues a warning instead of throwing an error and exiting

[0.4.13]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.13

## [0.4.12] (June 21, 2018)

* Fixed obscure assertion failure when renaming or deleting files ([#78])
* Added a `PluginAction` for the sync in command, which should help with some automation scripts ([#80])

[0.4.12]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.12
[#78]: https://github.com/rojo-rbx/rojo/issues/78
[#80]: https://github.com/rojo-rbx/rojo/pull/80

## [0.4.11] (June 10, 2018)

* Defensively insert existing instances into RouteMap; should fix most duplication cases when syncing into existing trees.
* Fixed incorrect synchronization from `Plugin:_pull` that would cause polling to create issues
* Fixed incorrect file routes being assigned to `init.lua` and `init.model.json` files
* Untangled route handling-internals slightly

[0.4.11]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.11

## [0.4.10] (June 2, 2018)

* Added support for `init.model.json` files, which enable versioning `Tool` instances (among other things) with Rojo. ([#66])
* Fixed obscure error when syncing into an invalid service.
* Fixed multiple sync processes occurring when a server ID mismatch is detected.

[0.4.10]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.10
[#66]: https://github.com/rojo-rbx/rojo/issues/66

## [0.4.9] (May 26, 2018)

* Fixed warning when renaming or removing files that would sometimes corrupt the instance cache ([#72])
* JSON models are no longer as strict -- `Children` and `Properties` are now optional.

[0.4.9]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.9
[#72]: https://github.com/rojo-rbx/rojo/pull/72

## [0.4.8] (May 26, 2018)

* Hotfix to prevent errors from being thrown when objects managed by Rojo are deleted

[0.4.8]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.8

## [0.4.7] (May 25, 2018)

* Added icons to the Rojo plugin, made by [@Vorlias](https://github.com/Vorlias)! ([#70])
* Server will now issue a warning if no partitions are specified in `rojo serve` ([#40])

[0.4.7]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.7
[#40]: https://github.com/rojo-rbx/rojo/issues/40
[#70]: https://github.com/rojo-rbx/rojo/pull/70

## [0.4.6] (May 21, 2018)

* Rojo handles being restarted by Roblox Studio more gracefully ([#67])
* Folders should no longer get collapsed when syncing occurs.
* **Significant** robustness improvements with regards to caching.
  * **This should catch all existing script duplication bugs.**
  * If there are any bugs with script duplication or caching in the future, restarting the Rojo server process will fix them for that session.
* Fixed message in plugin not being prefixed with `Rojo:`.

[0.4.6]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.6
[#67]: https://github.com/rojo-rbx/rojo/issues/67

## [0.4.5] (May 1, 2018)

* Rojo messages are now prefixed with `Rojo:` to make them stand out in the output more.
* Fixed server to notice file changes *much* more quickly. (200ms vs 1000ms)
* Server now lists name of project when starting up.
* Rojo now throws an error if no project file is found. ([#63])
* Fixed multiple sync operations occuring at the same time. ([#61])
* Partitions targeting files directly now work as expected. ([#57])

[0.4.5]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.5
[#57]: https://github.com/rojo-rbx/rojo/issues/57
[#61]: https://github.com/rojo-rbx/rojo/issues/61
[#63]: https://github.com/rojo-rbx/rojo/issues/63

## [0.4.4] (April 7, 2018)

* Fix small regression introduced in 0.4.3

[0.4.4]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.4

## [0.4.3] (April 7, 2018)

* Plugin now automatically selects `HttpService` if it determines that HTTP isn't enabled ([#58])
* Plugin now has much more robust handling and will wipe all state when the server changes.
  * This should fix issues that would otherwise be solved by restarting Roblox Studio.

[0.4.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.3
[#58]: https://github.com/rojo-rbx/rojo/pull/58

## [0.4.2] (April 4, 2018)

* Fixed final case of duplicated instance insertion, caused by reconciled instances not being inserted into `RouteMap`.
  * The reconciler is still not a perfect solution, especially if script instances get moved around without being destroyed. I don't think this can be fixed before a big refactor.

[0.4.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.2

## [0.4.1] (April 1, 2018)

* Merged plugin repository into main Rojo repository for easier tracking.
* Improved `RouteMap` object tracking; this should fix some cases of duplicated instances being synced into the tree.

[0.4.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.1

## [0.4.0] (March 27, 2018)

* Protocol version 1, which shifts more responsibility onto the server
  * This is a **major breaking** change!
  * The server now has a content of 'filter plugins', which transform data at various stages in the pipeline
  * The server now exposes Roblox instance objects instead of file contents, which lines up with how `rojo pack` will work, and paves the way for more robust syncing.
* Added `*.model.json` files, which let you embed small Roblox objects into your Rojo tree.
* Improved error messages in some cases ([#46])

[0.4.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.4.0
[#46]: https://github.com/rojo-rbx/rojo/issues/46

## [0.3.2] (December 20, 2017)

* Fixed `rojo serve` failing to correctly construct an absolute root path when passed as an argument
* Fixed intense CPU usage when running `rojo serve`

[0.3.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.2

## [0.3.1] (December 14, 2017)

* Improved error reporting when invalid JSON is found in a `rojo.json` project
  * These messages are passed on from Serde

[0.3.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.1

## [0.3.0] (December 12, 2017)

* Factored out the plugin into a separate repository
* Fixed server when using a file as a partition
  * Previously, trailing slashes were put on the end of a partition even if the read request was an empty string. This broke file reading on Windows when a partition pointed to a file instead of a directory!
* Started running automatic tests on Travis CI ([#9])

[0.3.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.3.0
[#9]: https://github.com/rojo-rbx/rojo/pull/9

## [0.2.3] (December 4, 2017)

* Plugin only release
* Tightened `init` file rules to only match script files
  * Previously, Rojo would sometimes pick up the wrong file when syncing

[0.2.3]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.3

## [0.2.2] (December 1, 2017)

* Plugin only release
* Fixed broken reconciliation behavior with `init` files

[0.2.2]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.2

## [0.2.1] (December 1, 2017)

* Plugin only release
* Changes default port to 8000

[0.2.1]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.1

## [0.2.0] (December 1, 2017)

* Support for `init.lua` like rbxfs and rbxpacker
* More robust syncing with a new reconciler

[0.2.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.2.0

## [0.1.0] (November 29, 2017)

* Initial release, functionally very similar to [rbxfs](https://github.com/LPGhatguy/rbxfs)

[0.1.0]: https://github.com/rojo-rbx/rojo/releases/tag/v0.1.0
