---
name: Atlas Fork Isolation
overview: Change default port, add a server fork identifier, rename settings prefix, rename plugin rbxm, rename binary to atlas, update VS Code extension binary references, and rename internal Studio markers so Atlas and stock Rojo can coexist without conflicts.
todos:
  - id: port
    content: Change default port from 34872 to 34873 in serve.rs and Config.lua
    status: completed
  - id: version
    content: Bump version in Cargo.toml and plugin/Version.txt
    status: completed
  - id: protocol
    content: Bump protocol version 5->6 in interface.rs and Config.lua
    status: completed
  - id: fork-id
    content: Add serverFork field to ServerInfoResponse, populate with 'atlas', validate in plugin ApiContext.lua and Types.lua
    status: completed
  - id: settings
    content: Change settings prefix from Rojo_ to Atlas_ in Settings.lua
    status: completed
  - id: session-markers
    content: Rename __Rojo_SessionLock and __Rojo_ConnectionUrl to __Atlas_ in App/init.lua
    status: completed
  - id: widget-ids
    content: Rename Rojo_ prefixed widget IDs to Atlas_ in Confirming.lua and Connected.lua
    status: completed
  - id: rbxm-managed
    content: Rename RojoManagedPlugin.rbxm to AtlasManagedPlugin.rbxm in plugin.rs
    status: completed
  - id: rbxm-build
    content: Rename Rojo.rbxm to Atlas.rbxm in build scripts, install scripts, and release workflow
    status: completed
  - id: binary-rename
    content: "Rename binary: Cargo.toml name rojo->atlas, CARGO_BIN_EXE in tests, clap name, release workflow BIN, install script binary refs"
    status: completed
  - id: vscode-binary-refs
    content: "Change all rojo binary invocations to atlas in vscode-rojo: getRojoInstall, installPlugin, serveProject, buildProject, createProjectFile, openMenu"
    status: completed
  - id: bugfix-waypoint
    content: "Fix broken undo detection: App/init.lua checks '^Rojo: Patch' but ServeSession already records as 'Atlas: Patch'"
    status: completed
  - id: templates
    content: Update project template READMEs and init.rs CLI output to say atlas instead of rojo
    status: completed
isProject: false
---

# Atlas Fork Isolation from Stock Rojo

## 1. Default Port: 34872 -> 34873

**Server:**

- `[src/cli/serve.rs](src/cli/serve.rs)` line 15: `DEFAULT_PORT: u16 = 34872` -> `34873`
- Same file line 28-29: update help text from `34872` to `34873`

**Plugin:**

- `[plugin/src/Config.lua](plugin/src/Config.lua)` line 32: `defaultPort = "34872"` -> `"34873"`

## 2. Version Bump + Unique Server Fork Identifier

**Version bump** (both must match per `build.rs` assertion):

- `[Cargo.toml](Cargo.toml)` line 3: bump `version` (e.g. `"7.7.0-rc.1"` -> `"7.7.0-rc.2"` or whatever the next version should be)
- `[plugin/Version.txt](plugin/Version.txt)`: match the same value

**Protocol version bump:**

- `[src/web/interface.rs](src/web/interface.rs)` line 28: `PROTOCOL_VERSION: u64 = 5` -> `6`
- `[plugin/src/Config.lua](plugin/src/Config.lua)` line 30: `protocolVersion = 5` -> `6`

**Add `serverFork` field to `ServerInfoResponse`** so the plugin can reject non-Atlas servers even if protocol versions coincidentally match in the future:

- `[src/web/interface.rs](src/web/interface.rs)`: add `pub server_fork: String` to `ServerInfoResponse` (serializes as `"serverFork"`)
- `[src/web/api.rs](src/web/api.rs)` ~line 272: include `server_fork: "atlas".to_owned()` when constructing the response
- `[plugin/src/Types.lua](plugin/src/Types.lua)` ~line 38: add `serverFork = t.string` to `ApiInfoResponse`
- `[plugin/src/ApiContext.lua](plugin/src/ApiContext.lua)` ~line 27: add a `rejectWrongFork` check that validates `infoResponseBody.serverFork == "atlas"`, rejecting with a clear message if it's a stock Rojo server

## 3. Plugin Settings Prefix: `Rojo_` -> `Atlas_`

- `[plugin/src/Settings.lua](plugin/src/Settings.lua)`: change `"Rojo_"` to `"Atlas_"` on lines 41, 45, 70 (3 occurrences, all share the same pattern)

## 4. Session Lock + Connection Attribute Naming

These are stored on real instances in Studio and would conflict if both plugins are loaded:

- `[plugin/src/App/init.lua](plugin/src/App/init.lua)`: rename `"__Rojo_SessionLock"` -> `"__Atlas_SessionLock"` (lines 321, 348, 351, 365)
- Same file: rename `"__Rojo_ConnectionUrl"` -> `"__Atlas_ConnectionUrl"` (lines 534, 547, 557, 561)

## 5. Ephemeral Widget IDs: `Rojo_` -> `Atlas_`

These are `CreateDockWidgetPluginGui` IDs that persist across sessions and would clash:

- `[plugin/src/App/StatusPages/Confirming.lua](plugin/src/App/StatusPages/Confirming.lua)`: 4 IDs (`Rojo_ConfirmingStringDiff`, `Rojo_ConfirmingTableDiff`, `Rojo_AcceptConfirm`, `Rojo_DiffSync`)
- `[plugin/src/App/StatusPages/Connected.lua](plugin/src/App/StatusPages/Connected.lua)`: 3 IDs (`Rojo_ChangesViewer`, `Rojo_ConnectedStringDiff`, `Rojo_ConnectedTableDiff`)

## 6. Plugin RBXM Naming: `Rojo.rbxm` -> `Atlas.rbxm`

`**rojo plugin install` command:**

- `[src/cli/plugin.rs](src/cli/plugin.rs)` line 13: `PLUGIN_FILE_NAME = "RojoManagedPlugin.rbxm"` -> `"AtlasManagedPlugin.rbxm"`

**Build scripts:**

- `[scripts/watch-build-plugin.sh](scripts/watch-build-plugin.sh)`: `Rojo.rbxm` -> `Atlas.rbxm`, `rojo build` -> `atlas build`
- `[scripts/watch-build-plugin.ps1](scripts/watch-build-plugin.ps1)`: same
- `[scripts/unit-test-plugin.sh](scripts/unit-test-plugin.sh)`: `rojo build` -> `atlas build`
- `[scripts/unit-test-plugin.ps1](scripts/unit-test-plugin.ps1)`: `rojo build` -> `atlas build`
- `[install.sh](install.sh)`: `Rojo.rbxm` -> `Atlas.rbxm`, `./target/$MODE/rojo` -> `./target/$MODE/atlas`, `pkill -f rojo` -> `pkill -f atlas`
- `[install.ps1](install.ps1)`: `Rojo.rbxm` -> `Atlas.rbxm`, `rojo.exe` -> `atlas.exe`, `rojo.pdb` -> `atlas.pdb`, `C:\Program Files\Rojo` -> `C:\Program Files\Atlas`, `Stop-Process -Name "rojo"` -> `Stop-Process -Name "atlas"`

**GitHub Actions:**

- `[.github/workflows/release.yml](.github/workflows/release.yml)`:
  - Line 34: `rojo build` -> `atlas build` (but this uses rokit's stock rojo -- see note below)
  - Line 88: `BIN: rojo` -> `BIN: atlas`
  - Lines 34, 40, 45, 46: `Rojo.rbxm` -> `Atlas.rbxm`

**Note on CI plugin build:** Release workflow line 34 uses rokit-installed `rojo` to build the plugin rbxm artifact. After binary rename, the locally compiled binary is `atlas`, but rokit installs stock `rojo`. Two options:

- Option A: Change CI to compile atlas first, then use `atlas build` for the plugin (requires reordering jobs or combining them)
- Option B: Keep using rokit's stock `rojo build` for the plugin artifact (the `build` command is format-compatible)
- **Recommend Option A** for correctness -- the build-plugin step should use the atlas binary from the build step.

## 7. Binary Rename: `rojo` -> `atlas`

**Cargo.toml:**

- `[Cargo.toml](Cargo.toml)` line 2: `name = "rojo"` -> `name = "atlas"` (changes binary output from `rojo`/`rojo.exe` to `atlas`/`atlas.exe`)
- `[lib] name = "librojo"` stays unchanged (line 41) -- this keeps all `use librojo::` imports working without cascading changes to ~20 test/bench files

**CLI display name:**

- `[src/cli/mod.rs](src/cli/mod.rs)` line 36: `#[clap(name = "Rojo"...)]` -> `#[clap(name = "Atlas"...)]`

**Test binary path resolution:**

- `[tests/rojo_test/io_util.rs](tests/rojo_test/io_util.rs)` line 10: `env!("CARGO_BIN_EXE_rojo")` -> `env!("CARGO_BIN_EXE_atlas")` (Cargo auto-generates this env var from the binary name)

## 8. VS Code Extension: Binary Invocations `rojo` -> `atlas`

The extension package is already rebranded (`vscode-atlas`, commands `vscode-atlas.`*, config `atlas.*`). But source files still invoke the `rojo` binary:

- `[vscode-rojo/src/getRojoInstall.ts](vscode-rojo/src/getRojoInstall.ts)`:
  - Line 46: `which("rojo")` -> `which("atlas")`
  - Line 56: `exec("rojo --version", ...)` -> `exec("atlas --version", ...)`
- `[vscode-rojo/src/installPlugin.ts](vscode-rojo/src/installPlugin.ts)`:
  - Line 13: `exec("rojo plugin install", ...)` -> `exec("atlas plugin install", ...)`
- `[vscode-rojo/src/serveProject.ts](vscode-rojo/src/serveProject.ts)`:
  - Line 30: `terminal.sendText('rojo serve ...')` -> `terminal.sendText('atlas serve ...')`
- `[vscode-rojo/src/buildProject.ts](vscode-rojo/src/buildProject.ts)`:
  - Line 24: `exec('rojo build ...')` -> `exec('atlas build ...')`
- `[vscode-rojo/src/createProjectFile.ts](vscode-rojo/src/createProjectFile.ts)`:
  - Line 8: `exec("rojo init", ...)` -> `exec("atlas init", ...)`
- `[vscode-rojo/src/commands/openMenu.ts](vscode-rojo/src/commands/openMenu.ts)`:
  - Line 164: `which("rojo")` -> `which("atlas")`
  - Line 571: `terminal.sendText('rojo studio ...')` -> `terminal.sendText('atlas studio ...')`
  - Line 602: `terminal.sendText('rojo syncback ...')` -> `terminal.sendText('atlas syncback ...')`

**Persisted state key:**

- `[vscode-rojo/src/serveProject.ts](vscode-rojo/src/serveProject.ts)` line 16 and `[vscode-rojo/src/commands/serveRecent.ts](vscode-rojo/src/commands/serveRecent.ts)` line 10: `"rojoLastPath"` -> `"atlasLastPath"`

**Aftman install references** (in `installRojo.ts` lines 183, 201, 210): These reference `rojo-rbx/rojo` for auto-installing. Left as-is for now since our fork may not be published to aftman yet. Will need updating when distribution is set up.

## 9. Bugfix: Undo Waypoint Detection

`ServeSession.lua:529` already records as `"Atlas: Patch ..."` but `App/init.lua:66` still checks for `"^Rojo: Patch"`. This means undo detection is currently broken.

- `[plugin/src/App/init.lua](plugin/src/App/init.lua)` line 66: `"^Rojo: Patch"` -> `"^Atlas: Patch"`

## 10. Project Templates and CLI Output: `rojo` -> `atlas`

**Template READMEs:**

- `[assets/project-templates/place/README.md](assets/project-templates/place/README.md)`: `Generated by [Rojo](...)` -> `Generated by [Atlas](...)`, `rojo serve` -> `atlas serve`, `rojo syncback` -> `atlas syncback`
- `[assets/project-templates/model/README.md](assets/project-templates/model/README.md)`: same pattern, `rojo build` -> `atlas build`
- `[assets/project-templates/plugin/README.md](assets/project-templates/plugin/README.md)`: same pattern, `rojo build` -> `atlas build`

**CLI print:**

- `[src/cli/init.rs](src/cli/init.rs)` line 206: `"Run 'rojo syncback' to sync your place."` -> `"Run 'atlas syncback' to sync your place."`

---

## What We Are NOT Changing (and why)


| Item                                                            | Reason                                                                                                                        |
| --------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `plugin.project.json` name `"Rojo"`                             | Would require updating `FindFirstAncestor("Rojo")` in 50+ Lua files; each plugin loads its own tree, no cross-plugin conflict |
| `[lib] name = "librojo"`                                        | Keeps all `use librojo::` imports working in ~20 test/bench files                                                             |
| `/api/rojo` endpoint path                                       | Internal protocol between our server and our plugin; not user-facing                                                          |
| `handle_api_rojo` / `get_api_rojo` method names                 | Internal Rust/test method names; cosmetic                                                                                     |
| `Rojo_Id`, `Rojo_Target_`, `Rojo_Ref_` attribute names          | Data format embedded in existing Roblox files; changing breaks backward compatibility                                         |
| `ROJO_DEV_BUILD` instance name                                  | Internal dev flag within plugin tree; no cross-plugin conflict                                                                |
| `ROJO_SYNCBACK_DEBUG` env var                                   | Internal debug var; cosmetic                                                                                                  |
| `rojo-test/` directory, `mod rojo_test`, `rojo-insta-ext` crate | Internal test infrastructure; not derived from package name, still works                                                      |
| `rokit.toml` `rojo = "rojo-rbx/rojo@7.5.1"`                     | Dev tool dependency for stock rojo; doesn't affect our binary                                                                 |
| `build/windows/rojo-manifest.rc` and `rojo.manifest`            | Windows manifest files; paths are hardcoded in build.rs, not derived from package name                                        |
| `RojoTree`, `RojoRef`, etc. type names                          | Internal Rust types; cosmetic                                                                                                 |
| Comments, doc strings, error messages (~150 occurrences)        | Cosmetic only; no functional conflict                                                                                         |
| `plugin/Packages/` vendored dependencies                        | Third-party code; must not touch                                                                                              |
| `vscode-rojo/src/installRojo.ts` aftman refs                    | Distribution not set up yet; update later                                                                                     |


