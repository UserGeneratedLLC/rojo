<div align="center">
    <a href="https://github.com/UserGeneratedLLC/rojo"><img src="assets/brand_images/logo-512.png" alt="Atlas" height="217" /></a>
</div>

<div>&nbsp;</div>

<div align="center">
    <a href="https://github.com/UserGeneratedLLC/rojo/actions"><img src="https://github.com/UserGeneratedLLC/rojo/workflows/CI/badge.svg" alt="CI status" /></a>
    <a href="https://marketplace.visualstudio.com/items?itemName=UserGeneratedLLC.vscode-roblox-atlas"><img src="https://img.shields.io/badge/VS%20Code-Marketplace-blue" alt="VS Code Marketplace" /></a>
    <a href="https://open-vsx.org/extension/UserGeneratedLLC/vscode-roblox-atlas"><img src="https://img.shields.io/badge/Open%20VSX-Registry-purple" alt="Open VSX Registry" /></a>
</div>

<hr />

**Atlas** bridges Roblox Studio with **Cursor/Claude**, so large teams can collaborate on complex games with full project context.

## Features

- **Live Sync** — Work on scripts and models from the filesystem with real-time sync to Roblox Studio
- **Two-Way Sync** — Push changes from the filesystem to Studio, or pull changes from Studio back to the filesystem
- **Build & Upload** — Package and deploy `.rbxl`, `.rbxm`, `.rbxlx`, and `.rbxmx` files from the command line
- **Syncback** — Pull an entire Roblox place into a local project with one command
- **Clone** — Create a full local project from any existing Roblox place: `atlas clone PLACEID`
- **Cursor / VS Code Extension** — JSON5 schema support, automatic CLI installation, bundled Luau tooling

## Installation

The fastest way to get started is to install the Cursor or VS Code extension, which handles CLI installation automatically.

### 1. Install the Extension

- **Cursor / Other**: [Open VSX Registry](https://open-vsx.org/extension/UserGeneratedLLC/vscode-roblox-atlas)
- **VS Code**: [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=UserGeneratedLLC.vscode-roblox-atlas)

The extension will prompt to install the Atlas CLI via [Rokit](https://github.com/rojo-rbx/rokit) and the Roblox Studio plugin if they aren't already on your system.

### 2. Alternative: Manual CLI Install

If you prefer not to use the extension, install the CLI directly:

**Via Rokit (recommended):**
```bash
rokit add --global UserGeneratedLLC/rojo atlas
```

**Build from source:**
```bash
git clone --recursive https://github.com/UserGeneratedLLC/rojo
cd rojo
cargo build --release
```

The binary is output to `target/release/atlas` (or `atlas.exe` on Windows).

## Getting Started

### Clone an Existing PlaceId

```bash
atlas clone 123456789              # Download and set up the project locally
atlas serve                        # Start live sync server for Roblox Studio
```

Then just:

```bash
atlas studio                         # Open the project in Roblox Studio
atlas cursor                         # Open the project in Cursor IDE
```

## New Project

```bash
atlas init --kind place
atlas serve
```

Then in Roblox Studio, open the Atlas plugin widget and click **Connect**.

## CLI Commands

### Core

```bash
atlas serve [project]                # Start live sync server (default port: 34873)
atlas serve --port 8080              # Use a custom port
atlas build [project] -o out.rbxl    # Build to .rbxl, .rbxm, .rbxlx, or .rbxmx
atlas build --watch -o out.rbxl      # Rebuild automatically on file changes
atlas build --plugin Plugin.rbxm     # Build directly to Studio's plugins folder
atlas upload [project] --asset_id ID # Upload to Roblox (cookie or API key auth)
```

### Project Management

```bash
atlas init                           # Initialize a new place project
atlas init --kind model              # Initialize a model project
atlas init --kind plugin             # Initialize a plugin project
atlas clone PLACEID                  # Clone a Roblox place into a new local project
atlas fmt-project [project]          # Reformat project file with sorted keys
```

### Syncback

```bash
atlas syncback [project]             # Sync from a local .rbxl to the filesystem
atlas syncback -d PLACEID            # Download the place from Roblox first
atlas syncback --incremental         # Preserve existing file structure
atlas syncback --list                # Preview what will change (no writes)
atlas syncback --dry-run             # Simulate without writing files
atlas syncback --sourcemap           # Generate sourcemap.json after sync
```

`atlas pull` is an alias for `atlas syncback`.

### Tooling

```bash
atlas sourcemap [project]            # Generate sourcemap.json for Luau LSP
atlas sourcemap --watch              # Regenerate on file changes
atlas plugin install                 # Install the Studio plugin
atlas plugin uninstall               # Remove the Studio plugin
atlas studio [project]               # Open the project in Roblox Studio
atlas cursor [path]                  # Open the project in Cursor IDE
atlas doc                            # Open documentation in the browser
```

### Global Options

```bash
atlas --verbose                      # Enable debug logging (-v, -vv, -vvv)
atlas --color never                  # Disable colored output (auto | always | never)
```

## Workflows

### Cloning from an Existing Place

```bash
atlas clone 123456789
```

What happens:
1. Fetches the experience name from the Roblox API
2. Creates a project folder with a sanitized name
3. Initializes the project with `atlas init`
4. Downloads the place and runs syncback to write all instances to the filesystem
5. Commits the result if git was initialized

### Updating from Roblox

If the place has been edited in Studio and you want to pull those changes back:

```bash
atlas syncback -d PLACEID
```

Or if you already have a local `.rbxl` file:

```bash
atlas syncback
```

Use `--incremental` to preserve your existing file layout, or omit it for a clean rebuild. Use `--list` or `--dry-run` to preview before committing to changes.

### Shortcuts

| Command | What it does |
|---------|-------------|
| `atlas studio` | Opens the project's place in Roblox Studio (reads `servePlaceIds` from the project file) |
| `atlas cursor` | Opens the project directory in Cursor IDE |

## One-Shot Mode

One-Shot Sync is a plugin setting (enabled by default) that performs a single sync and then disconnects. When enabled:

- The project tree syncs once, then the plugin automatically disconnects
- All automatic outgoing writes are blocked — only changes you explicitly confirm are applied
- The sync lock is skipped, so any number of people can connect and work together simultaneously
- A 30-second safety timeout ensures the connection is cleaned up

To use persistent (continuous) sync instead, disable "One-Shot Sync" in the plugin settings.

## Working with Teams

Atlas includes a sync lock to prevent conflicts when multiple people work on the same place in Team Create.

- A session lock (`__Atlas_SessionLock` in `ServerStorage`) is acquired when someone connects in persistent mode
- Only one user can actively sync at a time — others see a message indicating who holds the lock
- **One-shot mode bypasses the lock**, letting the whole team preview the synced state without conflicts

**Recommended team workflow:** One designated person syncs in persistent mode (one-shot disabled). Everyone else uses one-shot mode for quick, conflict-free previews.

## MCP Integration (AI Agents)

`atlas serve` automatically hosts an MCP (Model Context Protocol) server on the same port as the API. This lets AI agents like Cursor and Claude Code trigger syncs to Roblox Studio programmatically.

### Available Tools

| Tool | Description |
|------|-------------|
| `atlas_sync` | Sync filesystem changes to Studio. Auto-accepts if all changes are pre-selected by git. Blocks until the user accepts or rejects if there are unresolved changes. |

### Adding to Cursor

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "atlas": {
      "url": "http://localhost:34873/mcp"
    }
  }
}
```

If your project uses a custom port (via `servePort` in the project file), update the URL accordingly.

### Adding to Claude Code

```bash
claude mcp add atlas --transport http http://localhost:34873/mcp
```

Or add it to your project-level config at `.claude/settings.json`:

```json
{
  "mcpServers": {
    "atlas": {
      "type": "url",
      "url": "http://localhost:34873/mcp"
    }
  }
}
```

### How It Works

1. Start `atlas serve` — the MCP server is available immediately
2. The Studio plugin connects to the MCP stream in the background (retries every 5s)
3. When an agent calls `atlas_sync`:
   - If all changes are pre-selected by the git system, they are **auto-accepted** (fast-forward)
   - If there are unresolved changes, the **Confirming page** appears in Studio for the user to review
   - The agent blocks until the sync completes and receives a list of synced file paths with directions (`push`/`pull`)
4. If the plugin is already connected in live sync mode, the tool returns immediately with a notice that sync is automatic

### Requirements

- `atlas serve` must be running
- Roblox Studio must be open with the Atlas plugin installed
- The plugin does **not** need to be manually connected — the MCP stream connects automatically

## File Formats

### Scripts

| Extension | Result | RunContext |
|-----------|--------|-----------|
| `*.luau` | ModuleScript | — |
| `*.server.luau` | Script | Server |
| `*.client.luau` | Script | Client |
| `*.plugin.luau` | Script | Plugin |
| `*.local.luau` | LocalScript | — |
| `*.legacy.luau` | Script | Legacy |

### Data & Models

| Extension | Result |
|-----------|--------|
| `*.json5` | ModuleScript (Lua table) |
| `*.toml` | ModuleScript (Lua table) |
| `*.yaml` / `*.yml` | ModuleScript (Lua table) |
| `*.csv` | LocalizationTable |
| `*.txt` | StringValue |
| `*.rbxm` | Binary model |
| `*.rbxmx` | XML model |
| `*.model.json5` | JSON model |

### Metadata & Projects

| Extension | Purpose |
|-----------|---------|
| `*.meta.json5` | Add properties/attributes to a sibling instance |
| `*.project.json5` | Project file (or nested project) |

Legacy `.lua`, `.json`, `.meta.json`, `.model.json`, and `.project.json` extensions are still supported.

## Project Structure

A typical Atlas place project:

```
MyGame/
├── default.project.json5    # Project configuration
├── src/                     # Instance tree (synced to DataModel)
│   ├── ReplicatedStorage/
│   ├── ServerScriptService/
│   ├── ServerStorage/
│   ├── StarterGui/
│   ├── StarterPlayer/
│   └── Workspace/
├── rokit.toml               # Pins Atlas version for the project
└── .luaurc                  # Luau LSP configuration
```

Example `default.project.json5`:

```json5
{
  name: "MyGame",
  servePlaceIds: [123456789],
  tree: {
    $className: "DataModel",
    $path: "src"
  }
}
```

## Troubleshooting

### Plugin not connecting
- Ensure `atlas serve` is running
- Check that the port matches (default: `34873`, or whatever `servePort` is set to in your project file)
- Verify the Atlas CLI version matches the plugin version (major.minor must agree)

### Syncback produces unexpected results
- Use `--list` or `--dry-run` to preview changes before writing
- Try `--incremental` to preserve your existing file structure
- Check `syncbackRules` in your project file for ignored paths or classes

### Build errors
- Run `atlas fmt-project` to fix JSON formatting issues
- Ensure all `$path` entries in the project file point to existing files/directories

### Plugin shows "another user is syncing"
- Someone else holds the sync lock in Team Create
- Wait for them to disconnect, or use one-shot mode to preview without locking
