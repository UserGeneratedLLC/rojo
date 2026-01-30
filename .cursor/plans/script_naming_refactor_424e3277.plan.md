---
name: Script Naming Refactor
overview: Remove the `emitLegacyScripts` option entirely and implement a new, deterministic script naming convention where each script type has exactly one file extension. Add period encoding to the Windows encoding system and default that setting to true.
todos:
  - id: path-encoding
    content: Add %DOT% to CHAR_ENCODINGS, add %% escape for literal %, default encoding to true
    status: completed
  - id: middleware-enum
    content: Add LocalScript, LegacyScript, LocalScriptDir, LegacyScriptDir middleware variants
    status: completed
  - id: script-type
    content: Simplify ScriptType enum and update snapshot_lua logic
    status: completed
  - id: sync-rules
    content: Add .local.lua/.luau and .legacy.lua/.luau sync rules and init paths
    status: completed
  - id: remove-emit-legacy
    content: Remove emitLegacyScripts from Project, InstanceContext, and all usages
    status: completed
  - id: syncback-runcontext
    content: Update get_best_middleware to check RunContext for Script class
    status: completed
  - id: file-extensions
    content: Update extension_for_middleware with new script types
    status: completed
  - id: meta-suffixes
    content: Add .local and .legacy to meta file suffix stripping
    status: completed
  - id: web-api
    content: Update web/api.rs for new script naming conventions
    status: completed
  - id: update-tests
    content: Update snapshot files and test cases
    status: completed
isProject: false
---

# Script Naming Convention Refactor

## New File Extension Mapping


| Script Type                | File Extension |
| -------------------------- | -------------- |
| Script (RunContext.Server) | `.server.luau` |
| Script (RunContext.Client) | `.client.luau` |
| Script (RunContext.Plugin) | `.plugin.luau` |
| Script (RunContext.Legacy) | `.legacy.luau` |
| LocalScript                | `.local.luau`  |
| ModuleScript               | `.luau`        |


## Key Changes

### 1. Add Period Encoding, Percent Escaping, and Default to True

Update `[src/path_encoding.rs](src/path_encoding.rs)`:

1. Add `(".", "%DOT%")` to `CHAR_ENCODINGS`
2. Add `%%` as escape sequence for literal `%` (encode first, decode last)

```rust
const CHAR_ENCODINGS: &[(&str, &str)] = &[
    (".", "%DOT%"),  // NEW
    ("<", "%LT%"),
    // ... rest unchanged
];

pub fn encode_path_name(name: &str) -> String {
    // ... handle leading/trailing spaces ...
    
    // FIRST: Escape existing % as %% (before adding new % signs)
    let mut encoded_middle = middle.replace("%", "%%");
    
    // THEN: Encode special characters (which add new % signs)
    for (char, encoded) in CHAR_ENCODINGS {
        encoded_middle = encoded_middle.replace(char, encoded);
    }
    // ...
}

pub fn decode_path_name(name: &str) -> String {
    // ...
    // FIRST: Decode %NAME% patterns
    for (char, encoded) in CHAR_ENCODINGS {
        result = result.replace(encoded, char);
    }
    
    // LAST: Unescape %% back to %
    result = result.replace("%%", "%");
    // ...
}
```

This ensures round-tripping works correctly:

- `My%DOT%Thing` encodes to `My%%DOT%%Thing`, decodes back to `My%DOT%Thing`
- `My.Thing` encodes to `My%DOT%Thing`, decodes back to `My.Thing`

Change the default for `encode_windows_invalid_chars` to `true` in `[src/syncback/mod.rs](src/syncback/mod.rs)` `SyncbackRules::encode_windows_invalid_chars()`:

```rust
pub fn encode_windows_invalid_chars(&self) -> bool {
    self.encode_windows_invalid_chars.unwrap_or(true)  // Changed from false to true
}
```

Also update the `decode_windows_invalid_chars` logic in `[src/serve_session.rs](src/serve_session.rs)` to default to `true`.

### 2. Add New Middleware Variants

In `[src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs)`, add new variants:

- `LocalScript` - for `.local.lua/.luau` files (LocalScript class)
- `LegacyScript` - for `.legacy.lua/.luau` files (Script with RunContext.Legacy)
- `LocalScriptDir` - for `init.local.lua/.luau` directories
- `LegacyScriptDir` - for `init.legacy.lua/.luau` directories

### 3. Update ScriptType Enum

In `[src/snapshot_middleware/lua.rs](src/snapshot_middleware/lua.rs)`, simplify to:

```rust
pub enum ScriptType {
    Server,   // Script + RunContext.Server
    Client,   // Script + RunContext.Client  
    Plugin,   // Script + RunContext.Plugin
    Legacy,   // Script + RunContext.Legacy
    Local,    // LocalScript
    Module,   // ModuleScript
}
```

Remove `LegacyServer`, `LegacyClient`, `RunContextServer`, `RunContextClient` - these were for the dual-mode system.

### 4. Update Default Sync Rules

In `[src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs)` `default_sync_rules()`:

```rust
vec![
    sync_rule!("*.server.lua", ServerScript, ".server.lua"),
    sync_rule!("*.server.luau", ServerScript, ".server.luau"),
    sync_rule!("*.client.lua", ClientScript, ".client.lua"),
    sync_rule!("*.client.luau", ClientScript, ".client.luau"),
    sync_rule!("*.plugin.lua", PluginScript, ".plugin.lua"),
    sync_rule!("*.plugin.luau", PluginScript, ".plugin.luau"),
    sync_rule!("*.legacy.lua", LegacyScript, ".legacy.lua"),     // NEW
    sync_rule!("*.legacy.luau", LegacyScript, ".legacy.luau"),   // NEW
    sync_rule!("*.local.lua", LocalScript, ".local.lua"),        // NEW
    sync_rule!("*.local.luau", LocalScript, ".local.luau"),      // NEW
    sync_rule!("*.{lua,luau}", ModuleScript),  // Unchanged - catches remaining
    // ... rest unchanged
]
```

### 5. Update Init Path Handling

In `[src/snapshot_middleware/mod.rs](src/snapshot_middleware/mod.rs)` `get_dir_middleware()`:

```rust
vec![
    (Middleware::ModuleScriptDir, "init.luau"),
    (Middleware::ModuleScriptDir, "init.lua"),
    (Middleware::ServerScriptDir, "init.server.luau"),
    (Middleware::ServerScriptDir, "init.server.lua"),
    (Middleware::ClientScriptDir, "init.client.luau"),
    (Middleware::ClientScriptDir, "init.client.lua"),
    (Middleware::LocalScriptDir, "init.local.luau"),     // NEW
    (Middleware::LocalScriptDir, "init.local.lua"),       // NEW
    (Middleware::LegacyScriptDir, "init.legacy.luau"),   // NEW
    (Middleware::LegacyScriptDir, "init.legacy.lua"),     // NEW
    (Middleware::CsvDir, "init.csv"),
]
```

### 6. Remove emitLegacyScripts Entirely

Files to modify:

- `[src/project.rs](src/project.rs)` - Remove `emit_legacy_scripts` field from `Project`
- `[src/snapshot/metadata.rs](src/snapshot/metadata.rs)` - Remove from `InstanceContext`, remove `with_emit_legacy_scripts()`
- `[src/snapshot_middleware/util.rs](src/snapshot_middleware/util.rs)` - Remove `emit_legacy_scripts_default()`
- `[src/snapshot_middleware/project.rs](src/snapshot_middleware/project.rs)` - Remove `set_emit_legacy_scripts()` call
- `[src/serve_session.rs](src/serve_session.rs)` - Remove emit_legacy_scripts usage

### 7. Update Syncback Middleware Selection

In `[src/syncback/mod.rs](src/syncback/mod.rs)` `get_best_middleware()`:

```rust
middleware = match inst.class.as_str() {
    "Script" => {
        // Check RunContext to determine which middleware
        match inst.properties.get(&ustr("RunContext")) {
            Some(Variant::Enum(e)) => match e.to_u32() {
                // Legacy = 0, Server = 1, Client = 2, Plugin = 3
                0 => Middleware::LegacyScript,
                1 => Middleware::ServerScript,
                2 => Middleware::ClientScript,
                3 => Middleware::PluginScript,
                _ => Middleware::ServerScript, // fallback
            },
            _ => Middleware::LegacyScript, // No RunContext = Legacy behavior
        }
    }
    "LocalScript" => Middleware::LocalScript,
    "ModuleScript" => Middleware::ModuleScript,
    // ... rest unchanged
}
```

### 8. Update File Name Extensions

In `[src/syncback/file_names.rs](src/syncback/file_names.rs)` `extension_for_middleware()`:

```rust
Middleware::ServerScript => "server.luau",
Middleware::ClientScript => "client.luau",
Middleware::PluginScript => "plugin.luau",
Middleware::LegacyScript => "legacy.luau",   // NEW
Middleware::LocalScript => "local.luau",      // NEW
Middleware::ModuleScript => "luau",
```

### 9. Update Meta File Suffix Stripping

In `[src/snapshot_middleware/lua.rs](src/snapshot_middleware/lua.rs)` `syncback_lua()` and `[src/snapshot_middleware/meta_file.rs](src/snapshot_middleware/meta_file.rs)`:

Add `.local` and `.legacy` to the suffix stripping logic.

### 10. Update Web API (Two-Way Sync)

In `[src/web/api.rs](src/web/api.rs)`:

- Update script handling to use new naming conventions
- Remove all `emit_legacy_scripts` references and conditional logic

### 11. Update Test Files

- Delete test projects in `rojo-test/build-tests/nested_runcontext/` that test emitLegacyScripts
- Update all snapshot files (`.snap`) that contain `emit_legacy_scripts: true/false`
- Update test cases in `src/snapshot_middleware/lua.rs` to use the new single behavior

### 12. Clean Up Comments

Remove all legacy references and comments about the old dual-mode system throughout the codebase.

## Files to Modify (Summary)

Core changes:

- `src/path_encoding.rs` - Add period encoding (`%DOT%`) and `%%` escape for literal `%`
- `src/syncback/mod.rs` - Default encode_windows_invalid_chars to true, update middleware selection
- `src/serve_session.rs` - Default decode_windows_invalid_chars to true
- `src/snapshot_middleware/mod.rs` - Add middleware variants, update sync rules
- `src/snapshot_middleware/lua.rs` - Simplify ScriptType, update logic
- `src/snapshot/metadata.rs` - Remove emit_legacy_scripts
- `src/project.rs` - Remove emit_legacy_scripts field
- `src/syncback/mod.rs` - Update middleware selection with RunContext check
- `src/syncback/file_names.rs` - Add new extensions
- `src/snapshot_middleware/util.rs` - Remove emit_legacy_scripts_default
- `src/snapshot_middleware/project.rs` - Remove emit_legacy_scripts propagation
- `src/serve_session.rs` - Remove emit_legacy_scripts usage
- `src/web/api.rs` - Update for new conventions
- `src/snapshot_middleware/meta_file.rs` - Add new suffix handling

Test updates:

- ~40 snapshot files need `emit_legacy_scripts` line removed and `decode_windows_invalid_chars` changed to `true`
- Test functions need updating to not pass emit_legacy_scripts parameter

