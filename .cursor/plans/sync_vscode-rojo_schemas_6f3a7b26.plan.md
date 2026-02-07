---
name: Sync vscode-rojo schemas
overview: Update the vscode-rojo extension's schemas, file associations, TypeScript code, and branding to match this Rojo fork -- rebrand as "Atlas", add JSON5 support, new project fields, new file schemas, new middleware types, and syncback rules.
todos:
  - id: update-project-schema
    content: "Overhaul project.template.schema.json: make name optional, add all new fields with descriptions from Rust doc comments. Every field gets a concise description for VS Code tooltip intellisense."
    status: completed
  - id: create-meta-schema
    content: Create meta.template.schema.json with documented fields. Descriptions sourced from meta_file.rs doc comments.
    status: completed
  - id: create-model-schema
    content: Create model.template.schema.json with documented fields. Descriptions sourced from json_model.rs doc comments.
    status: completed
  - id: update-package-json
    content: Rebrand package.json to Atlas (name, displayName, publisher, commands, config keys, activation events), add json5 file associations, new schema entries for meta/model, update GitHub URL
    status: completed
  - id: update-generate-plugin
    content: Update GenerateSchemaPlugin.js to also generate meta.schema.json and model.schema.json, inject class names into model schema
    status: completed
  - id: rebrand-typescript
    content: Rebrand all user-facing strings from 'Rojo' to 'Atlas' in TS files, update command IDs from vscode-rojo.* to vscode-atlas.*, update GitHub/docs links, add .project.json5 support to findProjectFiles and openMenu
    status: completed
  - id: update-readme
    content: Rebrand README.md from Rojo to Atlas, update GitHub links, mention json5 support
    status: completed
isProject: false
---

# Sync vscode-rojo Extension with Rojo Fork + Atlas Rebrand

## Three Concerns

1. **Schema sync** -- bring schemas up to date with all Rojo fork changes
2. **Schema documentation** -- every field gets a concise `description` from the Rust doc comments, so VS Code shows useful tooltips
3. **Atlas rebrand** -- rename all user-facing text, VS Code registration IDs, and links

**NOT in scope:** Renaming actual file paths, folders, TypeScript variable/type names, or dependencies. Only front-end text, VS Code command/config IDs, and links change.

---

## Schema Changes

### 1. Update `project.template.schema.json` -- Major overhaul

Source of truth: [src/project.rs](src/project.rs) lines 61-149 (Project), 372-435 (ProjectNode)

**Top-level changes:**

- `name` -- change from **required** to **optional**
- Add `$schema` (string), `blockedPlaceIds` (integer[]), `syncbackRules` (object), `syncRules` (SyncRule[]), `syncScriptsOnly` (boolean), `ignoreHiddenServices` (boolean)
- Fix `servePlaceIds` items to `integer`
- Remove `"required": ["name", "tree"]` -- only `tree` required

**Tree node additions:** `$id` (string), `$attributes` (object)

**SyncbackRules sub-schema** (from [src/syncback/mod.rs](src/syncback/mod.rs) lines 959-1015):

- `ignoreTrees`, `ignorePaths`, `ignoreClasses`: string[]
- `ignoreProperties`: { [className]: string[] }
- `syncCurrentCamera`, `syncUnscriptable`, `ignoreReferents`, `createIgnoreDirPaths`, `encodeWindowsInvalidChars`, `ignoreHiddenServices`, `warnDuplicateNames`: boolean

**SyncRule sub-schema** ([src/snapshot/metadata.rs](src/snapshot/metadata.rs) lines 294-313):

- `pattern` (string), `exclude` (string, optional), `use` (Middleware enum), `suffix` (string, optional)

**Middleware enum** (camelCase): `csv`, `jsonModel`, `json`, `serverScript`, `clientScript`, `moduleScript`, `pluginScript`, `localScript`, `legacyScript`, `project`, `rbxm`, `rbxmx`, `toml`, `text`, `yaml`, `ignore`

### 2. Create `schemas/meta.template.schema.json`

Source: [src/snapshot_middleware/meta_file.rs](src/snapshot_middleware/meta_file.rs). Combined schema for `*.meta.json5` and `init.meta.json5`.

All fields optional: `$schema`, `id`, `ignoreUnknownInstances`, `properties`, `attributes`, `className`

### 3. Create `schemas/model.template.schema.json`

Source: [src/snapshot_middleware/json_model.rs](src/snapshot_middleware/json_model.rs) lines 171-202.

`className` required. Optional: `$schema`, `name`, `id`, `children` (recursive), `properties`, `attributes`

### 4. Update `GenerateSchemaPlugin.js`

- Also read and process `meta.template.schema.json` and `model.template.schema.json`
- Inject class names enum into model schema's `className` field
- Write all three to `dist/`

---

## Atlas Rebrand

### 5. `package.json` changes


| Field            | Old                                | New                                                                                                                                             |
| ---------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`           | `vscode-rojo`                      | `vscode-atlas`                                                                                                                                  |
| `displayName`    | `Rojo - Roblox Studio Sync`        | `Atlas - Roblox Studio Sync`                                                                                                                    |
| `description`    | `Rojo for VS Code`                 | `Atlas for VS Code`                                                                                                                             |
| `repository`     | `rojo-rbx/vscode-rojo`             | `UserGeneratedLLC/vscode-rojo`                                                                                                                  |
| `publisher`      | `evaera`                           | TBD (keep or change)                                                                                                                            |
| command ID       | `vscode-rojo.openMenu`             | `vscode-atlas.openMenu`                                                                                                                         |
| command category | `Rojo`                             | `Atlas`                                                                                                                                         |
| config title     | `Rojo`                             | `Atlas`                                                                                                                                         |
| config keys      | `rojo.*`                           | `atlas.*`                                                                                                                                       |
| activation       | `onCommand:vscode-rojo.openMenu`   | `onCommand:vscode-atlas.openMenu`                                                                                                               |
| activation       | `workspaceContains:*.project.json` | also add `*.project.json5`                                                                                                                      |
| jsonValidation   | only `*.project.json/jsonc`        | Replace with 3 `.json` entries only: `*.project.json`, `*.meta.json`, `*.model.json` (drop stale `.jsonc`; `.json5` handled by `json5.schemas`) |


**JSON5 schema registration via Better JSON5.** VS Code's built-in `jsonValidation` only works for `json`/`jsonc` language modes. Since we target the [Better JSON5](https://github.com/BlueGlassBlock/better-json5) extension (which has its own `json5.schemas` setting with full IntelliSense/validation), we register `.json5` schemas via `configurationDefaults` instead:

```json
"configurationDefaults": {
  "json5.schemas": [
    { "fileMatch": ["*.project.json5"], "url": "./dist/project.schema.json" },
    { "fileMatch": ["*.meta.json5"], "url": "./dist/meta.schema.json" },
    { "fileMatch": ["*.model.json5"], "url": "./dist/model.schema.json" }
  ]
}
```

The `jsonValidation` entries only cover the `.json` variants (which use VS Code's native JSON language service). No `jsonc` file association overrides needed.

**Bundle Better JSON5.** Add `extensionPack` to `package.json` so it installs alongside Atlas (users can disable independently):

```json
"extensionPack": [
  "BlueGlassBlock.better-json5",
  "JohnnyMorganz.luau-lsp",
  "JohnnyMorganz.stylua"
]
```

**Conflict check with original Rojo extension.** VS Code has no manifest-level incompatibility flag, so add a runtime check in `extension.ts` on activation: if `vscode.extensions.getExtension("evaera.vscode-rojo")` returns non-null, show a warning message asking the user to disable it (both extensions register the same commands/schemas and will conflict).

### 6. TypeScript string replacements (user-facing text only, not variable names)

**Command IDs** (in all TS files): `"vscode-rojo.*"` -> `"vscode-atlas.*"`

Affected files:

- `extension.ts` -- button command, console log, status messages
- `updateButton.ts` -- button commands
- `commands/openMenu.ts` -- registerCommand, executeCommand
- `commands/serveRecent.ts` -- registerCommand, executeCommand
- `commands/stopAll.ts` -- registerCommand
- `configuration.ts` -- config section name `"rojo"` -> `"atlas"`

**User-facing "Rojo" -> "Atlas"** in string literals:

- `extension.ts`: button text, info messages
- `openMenu.ts`: menu labels, error messages, info messages
- `serveProject.ts`: terminal title
- `buildProject.ts`: error/info messages
- `installPlugin.ts`: error/info messages
- `installRojo.ts`: success/error messages (note: the actual `rojo` CLI binary name stays the same -- only display text changes)

**Links:**

- `rojo.space/docs/v7/` -> keep as-is (still valid Rojo docs) OR update if there's a new docs URL
- `discord.gg/wH5ncNS` -> keep as-is (same community)
- `extension/evaera.vscode-rojo` -> update extension ID reference

`**default.project.json**` references -> `default.project.json5` (openMenu.ts lines 401-404)

`**findProjectFiles.ts**` line 77: also match `.project.json5`

### 7. README.md

Rebrand all "Rojo" display text to "Atlas", update GitHub link to `UserGeneratedLLC/vscode-rojo`, mention `.project.json5` support.