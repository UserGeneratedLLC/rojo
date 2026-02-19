---
name: Merge plugin upload workflow
overview: Manually apply the upstream commit "Upload plugin as part of release workflow" to the Atlas fork, adapting all file references and metadata to match the fork's diverged codebase.
todos:
  - id: new-files
    content: Create .lune/.config.luau, .lune/scripts/plugin-upload.luau, .lune/upload-plugin.luau (adapted for atlas), plugin/UploadDetails.json (with Atlas values)
    status: completed
  - id: submodule
    content: Add opencloud-execute submodule at .lune/opencloud-execute
    status: completed
  - id: build-rs
    content: Add UploadDetails.json to plugin snapshot in build.rs
    status: completed
  - id: project-json
    content: Add UploadDetails node to plugin.project.json
    status: completed
  - id: rokit
    content: Add lune tool to rokit.toml
    status: completed
  - id: release-yml
    content: Add Roblox upload step to release.yml build-plugin job
    status: completed
  - id: plugin-rs
    content: Apply trivial test change in src/cli/plugin.rs
    status: completed
  - id: local-scripts
    content: Replace scripts/publish-plugin.ps1 and add scripts/publish-plugin.sh to use lune-based upload
    status: completed
isProject: false
---

# Merge Plugin Upload Release Workflow

Cherry-picking will produce conflicts in 6+ files due to heavy divergence. Manual application is the cleanest path.

## Upstream bugs to fix

**Swapped env vars in `release.yml`:** The upstream commit has `RBX_UNIVERSE_ID` mapped to `vars.PLUGIN_CI_PLACE_ID` and `RBX_PLACE_ID` mapped to `vars.PLUGIN_CI_UNIVERSE_ID` -- clearly crossed. We will use the correct mapping.

## Local scripts replacement

`scripts/publish-plugin.ps1` currently uses `cargo run -- upload`. Replace it (and add missing `.sh`) to use the same Lune-based upload pipeline locally. The `.lune/upload-plugin.luau` script already handles the local case (builds the plugin via `atlas build` when `CI != "true"`), so the wrapper scripts just load `.env` and run `lune run upload-plugin Atlas.rbxm`.

`.env` uses the same names as GitHub Actions secrets/vars:

```
PLUGIN_UPLOAD_TOKEN=your-opencloud-api-key
PLUGIN_CI_UNIVERSE_ID=your-universe-id
PLUGIN_CI_PLACE_ID=your-place-id
```

The local scripts map these to `RBX_API_KEY`/`RBX_UNIVERSE_ID`/`RBX_PLACE_ID` before calling lune, matching what the CI workflow `env:` block does.

## What the commit adds

Automated plugin upload to Roblox as part of the GitHub release workflow, using [Lune](https://lune-org.github.io/docs) + OpenCloud Luau Execute to publish the built `.rbxm` to Roblox's plugin marketplace.

## New files (create as-is, with Atlas adaptations)

### `.lune/.config.luau`

Lune config with strict mode and typedefs alias. Copy verbatim from upstream.

### `.lune/scripts/plugin-upload.luau`

Roblox engine script that deserializes the plugin `.rbxm`, reads `UploadDetails`, and calls `AssetService:CreateAssetVersionAsync`. Copy verbatim -- it's generic (reads values from UploadDetails at runtime).

### `.lune/upload-plugin.luau`

Lune entry point that reads the `.rbxm`, sends it to OpenCloud for execution. One adaptation needed: change `process.exec("rojo", ...)` to `process.exec("atlas", ...)` on the local-testing path (line 25 in upstream).

### `plugin/UploadDetails.json`

Atlas-specific values per user input:

```json
{
  "assetId": 111151139098227,
  "name": "Atlas",
  "description": "The plugin portion of Atlas, a tool to enable professional tooling for Roblox developers.",
  "creatorId": 422368266,
  "creatorType": "Group"
}
```

## Submodule addition

### `.lune/opencloud-execute`

Add git submodule: `https://github.com/Dekkonot/opencloud-luau-execute-lune.git` at `.lune/opencloud-execute`. This also adds an entry to `[.gitmodules](.gitmodules)`.

## Existing file modifications

### `[.github/workflows/release.yml](.github/workflows/release.yml)`

Two additions to the `build-plugin` job:

1. **Add Rokit install step** (after "Install Rust", before "Build Atlas") so `lune` is on PATH:

```yaml
- name: Install Rokit
  uses: CompeyDev/setup-rokit@v0.1.2
```

1. **Add upload step** (after "Upload Plugin to Artifacts" at line 47) with **corrected** env var mapping (upstream had these swapped):

```yaml
- name: Upload Plugin to Roblox
  env:
    RBX_API_KEY: ${{ secrets.PLUGIN_UPLOAD_TOKEN }}
    RBX_UNIVERSE_ID: ${{ vars.PLUGIN_CI_UNIVERSE_ID }}
    RBX_PLACE_ID: ${{ vars.PLUGIN_CI_PLACE_ID }}
  run: lune run upload-plugin Atlas.rbxm
```

### `[build.rs](build.rs)`

Add `"UploadDetails.json"` entry to the plugin snapshot hashmap, after `"Version.txt"` on line 98:

```rust
"UploadDetails.json" => snapshot_from_fs_path(&plugin_dir.join("UploadDetails.json"))?,
```

### `[plugin.project.json](plugin.project.json)`

Add `UploadDetails` child node after `Version` (line 25):

```json
"UploadDetails": {
  "$path": "plugin/UploadDetails.json"
}
```

### `[rokit.toml](rokit.toml)`

Add lune tool (latest version) after the existing entries:

```toml
lune = "lune-org/lune@0.10.4"
```

### `[src/cli/plugin.rs](src/cli/plugin.rs)`

Trivial test change on line 101: `assert!(initialize_plugin().is_ok())` becomes `let _ = initialize_plugin().unwrap();`

### `[scripts/publish-plugin.ps1](scripts/publish-plugin.ps1)`

Replace entirely. New version: load `.env`, export `RBX_API_KEY`/`RBX_UNIVERSE_ID`/`RBX_PLACE_ID`, run `lune run upload-plugin Atlas.rbxm`.

### `scripts/publish-plugin.sh` (new)

Bash equivalent: source `.env`, export vars, run `lune run upload-plugin Atlas.rbxm`.

## Required GitHub secrets/vars

After merging, the repo needs these configured in GitHub settings:

- **Secret:** `PLUGIN_UPLOAD_TOKEN` -- Roblox OpenCloud API key
- **Variable:** `PLUGIN_CI_UNIVERSE_ID` -- Universe ID for the place used by OpenCloud Execute
- **Variable:** `PLUGIN_CI_PLACE_ID` -- Place ID within that universe

