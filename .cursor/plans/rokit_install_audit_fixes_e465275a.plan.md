---
name: Rokit Install Audit Fixes
overview: Fix the EPERM crash when deleting a globally installed atlas.exe, and harden the entire rokit installation pipeline in the VS Code extension for reliability across Windows, macOS, and Linux.
todos:
  - id: bug1-elevated-delete
    content: Add elevatedDelete() helper and fix the 'Switch to Rokit' flow in openMenu.ts to handle EPERM/EACCES/EBUSY with platform-specific elevation
    status: completed
  - id: bug2-progress
    content: Wrap installRojo() in vscode.window.withProgress() with stage reporting
    status: completed
  - id: bug3-github-api
    content: Replace eryn.io proxy URL with official GitHub API URL in installRojo.ts
    status: completed
  - id: bug4-temp-cleanup
    content: Delete temp rokit binary after successful self-install
    status: completed
  - id: bug5-path-dedup
    content: Deduplicate PATH entry and verify rokit accessibility before rokit add
    status: completed
  - id: bug6-stream
    content: Replace manual stream piping with stream.promises.pipeline()
    status: completed
  - id: bug7-rokit-add-existing
    content: Use 'rokit add --global --force' + 'rokit trust' + verify atlas --version + non-fatal plugin install
    status: completed
  - id: feat-check-updates
    content: Add 'Check for Updates' menu item in openMenu.ts for rokit-managed installs, handle project-level + global rokit update, reinstall Studio plugin after update
    status: completed
  - id: feat-auto-update-check
    content: Add background update check on activation using rokit update --check (once per day, project + global), prompt user with Update button that triggers rokit update + plugin reinstall
    status: completed
isProject: false
---

# Rokit Installation Audit and Fixes

## Audit Findings

### How Rokit Installation Works (No Elevation Needed)

Rokit installs entirely to `~/.rokit/` (user-level directory). `rokit self-install` modifies user-level PATH environment variables (HKCU on Windows, shell rc files on Unix). **Neither rokit installation nor `rokit add` requires elevation.**

The only scenario requiring elevation is the "Switch to Rokit" migration flow, which tries to delete an existing globally installed `atlas.exe` from a protected directory like `C:\Program Files\Atlas` (written by [install.ps1](install.ps1)) or `/usr/local/bin/atlas` (written by [install.sh](install.sh)).

### Issues Found (by severity)

---

**BUG 1 -- CRITICAL: `fs.unlink` fails without elevation, crashes the entire flow**

In [vscode-rojo/src/commands/openMenu.ts](vscode-rojo/src/commands/openMenu.ts) line 133:

```typescript
return fs.unlink(install.resolvedPath)
```

When atlas.exe is in `C:\Program Files\Atlas` or `/usr/local/bin/`, this throws EPERM/EACCES. The `.then()` error handler on line 139 catches it but just shows a generic message and the flow dies. Additionally, on Windows the exe may be **locked by a running process** (atlas serve), causing EBUSY even with elevation.

**Fix:** Add an `elevatedDelete()` helper that:

1. Tries `fs.unlink()` first
2. On EPERM/EACCES, uses platform-specific elevated delete:
  - **Windows:** `powershell -Command "Start-Process powershell -ArgumentList '-Command Remove-Item \"path\"' -Verb RunAs -Wait"` (triggers UAC prompt)
  - **macOS:** `osascript -e 'do shell script "rm path" with administrator privileges'` (triggers password dialog)
  - **Linux:** `pkexec rm path` (triggers PolicyKit dialog)
3. On EBUSY (Windows exe locked), shows a specific error message telling the user to stop atlas serve first

---

**BUG 2 -- HIGH: No progress indicator during installation**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) `installRojo()`:

The entire flow (download rokit zip ~3.5MB, extract, self-install, `rokit add` atlas) runs silently. Users see zero feedback and may think the extension froze.

**Fix:** Wrap the body of `installRojo()` in `vscode.window.withProgress()` with `ProgressLocation.Notification` and `cancellable: false`. Report progress at each stage:

- "Downloading Rokit..."
- "Installing Rokit..."
- "Installing Atlas via Rokit..."

---

**BUG 3 -- HIGH: Third-party release URL is a single point of failure**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) line 113:

```typescript
const latestReleaseResponse = await fetch(
  "https://latest-github-release.eryn.io/rojo-rbx/rokit",
)
```

This third-party proxy (`eryn.io`) is not under our control. If it goes down, installation breaks. The official rokit install scripts use the GitHub API directly.

**Fix:** Replace with the official GitHub API URL: `https://api.github.com/repos/rojo-rbx/rokit/releases/latest`. Verified: response schema is identical, both return the same `GitHubRelease` shape. Add an `Accept: application/vnd.github+json` header per GitHub API best practice.

---

**BUG 4 -- MEDIUM: Temp files never cleaned up**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) line 143-170:

The rokit binary is downloaded to `os.tmpdir()` but never deleted after `rokit self-install` succeeds.

**Fix:** Add `fs.promises.unlink(tempPath).catch(() => {})` after the `exec(self-install)` call succeeds.

---

**BUG 5 -- MEDIUM: PATH update is fragile**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) lines 177-181:

```typescript
if ("PATH" in process.env) {
  const envPath = process.env.PATH!.split(path.delimiter)
  envPath.push(path.join(os.homedir(), ".rokit", "bin"))
  process.env.PATH = envPath.join(path.delimiter)
}
```

This only updates the extension's own process environment. But the subsequent `rokit add --global` on line 184 runs via `exec()` which inherits this env, so it should work. The concern is:

- It doesn't check if the path is already present (could add duplicates on repeated attempts)
- The rokit binary path might differ on some systems

**Fix:** Check for duplicates before appending. Also verify the rokit binary is actually accessible after the PATH update by doing a quick `which("rokit")` check before proceeding to `rokit add`.

---

**BUG 6 -- LOW: Stream error handling is fragile**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) lines 51-60, 149-163:

`promisifyStream` resolves on whichever of "close"/"finish"/"end" fires first. The pipeline/pipe chain has multiple potential failure modes that aren't properly propagated.

**Fix:** Use `stream.promises.pipeline()` (available in Node 15+, which VS Code ships with) instead of manually piping and tracking events. This gives proper error propagation and backpressure handling. Alternatively, simplify to `await promisify(stream.pipeline)(download.body!, unzipper.ParseOne(), fs.createWriteStream(tempPath))`.

---

**BUG 7 -- HIGH: `rokit add --global` crashes if atlas is already configured**

In [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) line 184:

```typescript
await exec("rokit add --global UserGeneratedLLC/rojo atlas")
```

This is the **only** action after rokit is confirmed installed. It fails if atlas is already in the project's `rokit.toml` or already added globally. The scenario:

1. Project has `rokit.toml` with atlas listed but `rokit install` hasn't been run yet
2. `which("atlas")` finds the rokit shim at `~/.rokit/bin/atlas`
3. `atlas --version` fails (rokit says tool not installed), stderr contains "rokit" -> `getRojoInstall` returns null
4. User sees "Install Atlas now", clicks it
5. `isRokitInstalled()` returns true (skips rokit download)
6. `rokit add --global` fails because atlas is already configured -> **onboarding dies**

**Fix:** Use `--force` to handle "already exists", trust the tool, then verify and install the plugin:

```typescript
await exec("rokit add --global --force UserGeneratedLLC/rojo atlas")
await exec("rokit trust UserGeneratedLLC/rojo")
await exec("atlas --version")

// Install the Studio plugin (non-fatal -- don't crash onboarding if Studio isn't installed)
try {
  await exec("atlas plugin install")
} catch (e: any) {
  vscode.window.showWarningMessage(
    `Atlas installed successfully, but the Studio plugin could not be installed: ${e.stderr || e}. ` +
    `You can install it later from the Atlas menu or by running "atlas plugin install".`
  )
}
```

- `--force` on `rokit add` handles the "already exists" case cleanly -- no try/catch needed, it re-adds/updates regardless
- `rokit trust` marks `UserGeneratedLLC/rojo` as trusted so rokit won't prompt the user for confirmation when running atlas (without trust, `rokit install` and first-run shims can block waiting for user input)
- `atlas --version` verifies the tool is actually usable after add
- `atlas plugin install` writes the embedded Studio plugin to disk (non-fatal if Studio isn't installed yet)

---

---

**FEATURE: "Check for Updates" menu item for Rokit-managed installs**

In [vscode-rojo/src/commands/openMenu.ts](vscode-rojo/src/commands/openMenu.ts), the `generateProjectMenu()` return array (line 315) has action items like Studio, Syncback, Sourcemap, Install Plugin. There is no way to update Atlas from the extension.

`generateProjectMenu()` already tracks `installType` (line 168) and whether it's `InstallType.rokit`. We use this to conditionally show the item.

**Implementation:**

1. Add a menu item in `generateProjectMenu()` return array, conditionally included when `installType === InstallType.rokit`:

```typescript
...(installType === InstallType.rokit
  ? [
      {
        label: "$(sync) Check for Updates",
        description: `Atlas v${allRojoVersions[0]}`,
        action: "checkUpdates",
        projectFile: projectFiles[0],
      },
    ]
  : []),
```

1. Add `"checkUpdates"` case in the `onDidAccept` handler (after the `"install"` case). The handler needs to support both project-level `rokit.toml` and global installations:

```typescript
case "checkUpdates": {
  if (!selectedItem.projectFile) return
  const updateFolder = path.dirname(selectedItem.projectFile.path.fsPath)

  input.hide()

  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "Checking for Atlas updates..." },
    async (progress) => {
      // Try project-level update first, fall back to global
      let updateOutput: string
      try {
        const result = await exec("rokit update atlas", { cwd: updateFolder })
        updateOutput = result.stdout || result.stderr
      } catch {
        try {
          const result = await exec("rokit update --global atlas")
          updateOutput = result.stdout || result.stderr
        } catch (e: any) {
          vscode.window.showErrorMessage(`Could not update Atlas: ${e.stderr || e}`)
          return
        }
      }

      // Re-install the Studio plugin so it matches the new binary.
      // The plugin is embedded in the atlas binary at compile time
      // (include_bytes! in src/cli/plugin.rs), so updating the binary
      // without re-installing leaves a stale AtlasManagedPlugin.rbxm
      // in Studio's plugins folder. Mismatched versions break live sync
      // due to protocol version enforcement.
      progress.report({ message: "Reinstalling Studio plugin..." })
      try {
        await exec("atlas plugin install")
      } catch (e: any) {
        vscode.window.showWarningMessage(
          `Atlas updated, but plugin reinstall failed: ${e.stderr || e}. ` +
          `Run "atlas plugin install" manually to avoid version mismatch.`
        )
      }

      vscode.window.showInformationMessage(updateOutput.trim() || "Atlas is up to date.")
    },
  )
  break
}
```

The strategy: try `rokit update atlas` in the project directory first. If atlas isn't in the project's `rokit.toml`, this fails and we fall back to `rokit update --global atlas`. Rokit's output tells the user whether an update was applied or they're already current.

After a successful update, immediately runs `atlas plugin install` to overwrite the stale `AtlasManagedPlugin.rbxm` in Studio's plugins folder. The plugin is baked into the atlas binary via `include_bytes!` at compile time (`src/cli/plugin.rs:12`), so the `.rbxm` in Studio never auto-updates -- it must be explicitly reinstalled. If the plugin reinstall fails (e.g. Studio not installed), a warning is shown with manual instructions rather than failing the whole update.

---

**FEATURE: Background update check on extension activation**

In [vscode-rojo/src/extension.ts](vscode-rojo/src/extension.ts) `activate()`:

Rokit has `rokit update atlas --check` which checks for updates without installing. Supports both project-level (`rokit update atlas --check` in the project dir, reads `rokit.toml`) and global (`rokit update --global atlas --check`).

**Implementation:** Add a new file `vscode-rojo/src/checkForUpdates.ts` with a `checkForAtlasUpdates(context)` function, called fire-and-forget from `activate()`:

```typescript
export async function checkForAtlasUpdates(context: vscode.ExtensionContext) {
  // Throttle: once per day
  const lastCheck = context.globalState.get<number>("atlas::lastUpdateCheck")
  const ONE_DAY = 24 * 60 * 60 * 1000
  if (lastCheck && Date.now() - lastCheck < ONE_DAY) return

  // Check if rokit is available
  const rokitPath = await which("rokit").catch(() => null)
  if (!rokitPath) return

  const workspaceFolder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath

  // rokit update --check prints update info if available, or "up to date" if current.
  // Check both project-level and global.
  let checkOutput: string | null = null
  let updateScope: "project" | "global" = "global"

  // Try project-level first (if workspace is open)
  if (workspaceFolder) {
    try {
      const result = await exec("rokit update atlas --check", { cwd: workspaceFolder })
      const output = (result.stdout + result.stderr).trim()
      if (output && !output.toLowerCase().includes("up to date")) {
        checkOutput = output
        updateScope = "project"
      }
    } catch {
      // atlas not in project rokit.toml -- fall through
    }
  }

  // Try global if no project-level update found
  if (!checkOutput) {
    try {
      const result = await exec("rokit update --global atlas --check")
      const output = (result.stdout + result.stderr).trim()
      if (output && !output.toLowerCase().includes("up to date")) {
        checkOutput = output
        updateScope = "global"
      }
    } catch {
      // atlas not installed globally via rokit
    }
  }

  context.globalState.update("atlas::lastUpdateCheck", Date.now())

  if (!checkOutput) return

  const choice = await vscode.window.showInformationMessage(
    `An Atlas update is available. ${checkOutput}`,
    "Update",
    "Dismiss",
  )

  if (choice !== "Update") return

  await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "Updating Atlas..." },
    async (progress) => {
      try {
        if (updateScope === "project" && workspaceFolder) {
          await exec("rokit update atlas", { cwd: workspaceFolder })
        } else {
          await exec("rokit update --global atlas")
        }
      } catch (e: any) {
        vscode.window.showErrorMessage(`Could not update Atlas: ${e.stderr || e}`)
        return
      }

      progress.report({ message: "Reinstalling Studio plugin..." })
      try {
        await exec("atlas plugin install")
      } catch {
        vscode.window.showWarningMessage(
          "Atlas updated, but Studio plugin reinstall failed. Run 'atlas plugin install' manually.",
        )
      }

      vscode.window.showInformationMessage("Atlas updated successfully.")
    },
  )
}
```

Called from `activate()` as fire-and-forget (no `await`) so it never blocks extension startup:

```typescript
// In activate(), after all synchronous setup is done:
checkForAtlasUpdates(context).catch(() => {})
```

Key design decisions:

- **Uses `rokit update --check`**: Lets rokit handle version resolution natively -- no GitHub API calls, no manual semver parsing
- **Project-level first, then global**: Checks the project's `rokit.toml` first (if workspace is open), falls back to global. When updating, uses the same scope that found the update
- **Throttle**: Once per day via `globalState`, so frequent restarts don't spam the user
- **Silent failures**: Every step is wrapped in try/catch that returns early. Missing rokit, no atlas configured, network errors -- all silently ignored
- **Non-blocking**: Called without `await` so extension activation is never delayed

---

### Summary of What's Working Correctly

- Asset name regex (`-(?<platform>\w+)-(?<arch>\w+)\.zip$`) correctly matches all 6 rokit 1.2.0 release assets (linux/macos/windows x x86_64/aarch64, all `.zip`)
- Platform/arch mapping covers all common Node.js platforms (win32, darwin, linux) and architectures (x64, arm64)
- `rokit self-install` correctly sets up `~/.rokit/bin` in user PATH on all platforms
- `chmod 0o755` is correctly applied on Unix after extraction
- `getRojoInstall` correctly detects rokit-managed vs globally installed atlas

## Files to Change

- [vscode-rojo/src/installRojo.ts](vscode-rojo/src/installRojo.ts) -- Bugs 2, 3, 4, 5, 6, 7
- [vscode-rojo/src/commands/openMenu.ts](vscode-rojo/src/commands/openMenu.ts) -- Bug 1, Check for Updates menu item
- [vscode-rojo/src/checkForUpdates.ts](vscode-rojo/src/checkForUpdates.ts) -- NEW: background update check logic
- [vscode-rojo/src/extension.ts](vscode-rojo/src/extension.ts) -- Call checkForAtlasUpdates() on activation

