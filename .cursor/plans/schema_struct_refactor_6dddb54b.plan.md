---
name: Schema struct refactor
overview: Replace the ~150-line hand-written JSON Schema blob in `handle_tools_list` with `schemars::JsonSchema`-derived arg structs and a `tool_def<T>()` helper, matching the studio-rust-mcp-server pattern.
todos:
  - id: add-schemars
    content: Add schemars = "1" to Cargo.toml dependencies
    status: completed
  - id: define-arg-structs
    content: Define JsonSchema arg structs/enums for all 8 tools in mcp.rs, add JsonSchema to SyncOverride
    status: completed
  - id: tool-def-helper
    content: "Create tool_def<T: JsonSchema>() helper function"
    status: completed
  - id: replace-tools-list
    content: Replace handle_tools_list JSON blob with tool_def calls
    status: completed
  - id: update-tests
    content: Update tool_schema_tests and handler_tests for schemars-generated schemas
    status: completed
isProject: false
---

# Refactor `handle_tools_list` to Use `schemars` Struct-Derived Schemas

## Problem

`handle_tools_list` in [src/web/mcp.rs](src/web/mcp.rs) (lines 297-448) is a ~150-line hand-written `serde_json::json!` blob with manually maintained `inputSchema` objects for all 8 tools. Adding or modifying tool parameters requires editing nested JSON, which is error-prone and hard to review.

The studio-rust-mcp-server solves this with `#[derive(schemars::JsonSchema)]` on arg structs -- the struct definition IS the schema. That's what we want here.

## Approach

Add `schemars` (v1, matching studio-rust-mcp-server) and define arg structs per tool. A `tool_def<T: JsonSchema>()` helper generates the full tool entry from (name, description, schema). The existing `SyncOverride` struct gets `JsonSchema` added to its derives.

### Before (current)

```rust
fn handle_tools_list(id: Option<Value>) -> Response<Full<Bytes>> {
    let result = serde_json::json!({
        "tools": [
            {
                "name": "run_code",
                "description": include_str!("mcp_docs/run_code.md"),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Luau code to execute..."
                        }
                    },
                    "required": ["command"]
                }
            },
            // ... 7 more tools, ~20 lines each ...
        ]
    });
}
```

### After (target)

```rust
#[derive(JsonSchema)]
struct RunCodeArgs {
    /// Luau code to execute in Roblox Studio.
    command: String,
}

fn handle_tools_list(id: Option<Value>) -> Response<Full<Bytes>> {
    let tools = vec![
        tool_def::<RunCodeArgs>("run_code", include_str!("mcp_docs/run_code.md")),
        // ... 7 more, one line each ...
    ];
    // ...
}
```

## Changes

### 1. Add `schemars` dependency

In [Cargo.toml](Cargo.toml), add after the `serde_json` line (line 93):

```toml
schemars = "1"
```

### 2. Define arg structs and enums

In [src/web/mcp.rs](src/web/mcp.rs), add after the existing `PluginToolResult` struct. These are schema-only structs -- they define the shape of each tool's arguments. `schemars` reads `#[doc = "..."]` comments as JSON Schema `description` fields.

```rust
use schemars::JsonSchema;

#[derive(JsonSchema)]
#[schemars(rename_all = "camelCase")]
struct AtlasSyncArgs {
    /// Sync mode. 'standard' (default): auto-accept if all changes are
    /// git-resolved, otherwise show Studio UI. 'manual': always show
    /// Studio UI. 'fastfail': fail immediately if any changes are
    /// unresolved. 'dryrun': return what would change without applying.
    mode: Option<SyncMode>,
    /// Auto-accept directives for specific instance changes, matched
    /// by id from a previous sync response.
    overrides: Option<Vec<SyncOverride>>,
}

#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum SyncMode {
    Standard,
    Manual,
    Fastfail,
    Dryrun,
}

#[derive(JsonSchema)]
#[schemars(rename_all = "camelCase")]
struct GetScriptArgs {
    /// Server Ref (32-char hex) from a previous atlas_sync response.
    /// Preferred over fsPath.
    id: Option<String>,
    /// Atlas filesystem path relative to project root
    /// (e.g. 'src/server/MyScript.server.luau'). Resolved server-side
    /// to an instance id.
    fs_path: Option<String>,
    /// If true, read unsaved editor content instead of the saved Source
    /// property. Default: false.
    from_draft: Option<bool>,
}

#[derive(JsonSchema)]
struct RunCodeArgs {
    /// Luau code to execute in Roblox Studio.
    command: String,
}

#[derive(JsonSchema)]
struct InsertModelArgs {
    /// Search query for the model on the Roblox Creator Store.
    query: String,
}

#[derive(JsonSchema)]
struct NoArgs {}

#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum StartStopPlayMode {
    StartPlay,
    RunServer,
    Stop,
}

#[derive(JsonSchema)]
struct StartStopPlayArgs {
    /// Play mode action.
    mode: StartStopPlayMode,
}

#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum PlayTestMode {
    StartPlay,
    RunServer,
}

#[derive(JsonSchema)]
struct RunScriptInPlayModeArgs {
    /// Luau code to run inside a play session.
    code: String,
    /// Timeout in seconds. Defaults to 100.
    timeout: Option<u32>,
    /// Play mode to start.
    mode: PlayTestMode,
}
```

Also add `JsonSchema` to the existing `SyncOverride` struct (line ~29). It already has `Serialize, Deserialize` with `#[serde(rename_all = "camelCase")]` which schemars will pick up:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyncOverride { ... }
```

### 3. Create `tool_def` helper

```rust
fn tool_def<T: JsonSchema>(name: &str, description: &str) -> Value {
    let schema = schemars::schema_for!(T);
    let mut schema_value = serde_json::to_value(schema).unwrap_or_default();
    if let Some(obj) = schema_value.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": schema_value,
    })
}
```

Note: `schema_for!` is a macro so it works with concrete types. We call `tool_def::<RunCodeArgs>(...)` etc. If schemars 1.x doesn't support `schema_for!` in this context, use `SchemaGenerator::default().into_root_schema_for::<T>()` instead.

### 4. Replace `handle_tools_list` body

The 150-line JSON blob becomes:

```rust
fn handle_tools_list(id: Option<Value>) -> Response<Full<Bytes>> {
    let tools = vec![
        tool_def::<AtlasSyncArgs>("atlas_sync", include_str!("mcp_docs/atlas_sync.md")),
        tool_def::<GetScriptArgs>("get_script", include_str!("mcp_docs/get_script.md")),
        tool_def::<RunCodeArgs>("run_code", include_str!("mcp_docs/run_code.md")),
        tool_def::<InsertModelArgs>("insert_model", include_str!("mcp_docs/insert_model.md")),
        tool_def::<NoArgs>("get_console_output", include_str!("mcp_docs/get_console_output.md")),
        tool_def::<NoArgs>("get_studio_mode", include_str!("mcp_docs/get_studio_mode.md")),
        tool_def::<StartStopPlayArgs>("start_stop_play", include_str!("mcp_docs/start_stop_play.md")),
        tool_def::<RunScriptInPlayModeArgs>("run_script_in_play_mode", include_str!("mcp_docs/run_script_in_play_mode.md")),
    ];

    let result = serde_json::json!({ "tools": tools });
    let resp = JsonRpcResponse::success(id, result);
    json_response(&resp, StatusCode::OK)
}
```

### 5. Update tests

The `tool_schema_tests` module tests check for specific schema structures (e.g. `["inputSchema"]["required"]`, `["inputSchema"]["properties"]["mode"]["enum"]`). The schemars-generated schemas should produce the same logical structure:

- Required fields (non-`Option`) -> appears in `required` array
- `Option<T>` fields -> absent from `required`
- Enums with `rename_all = "snake_case"` -> `{"enum": ["start_play", ...]}`
- String fields -> `{"type": "string"}`
- `u32` -> `{"type": "integer"}` (or `{"type": "integer", "minimum": 0}`)

The tests may need minor adjustments depending on exact schemars output:

- schemars may add `"format": "uint32"` to the `timeout` field
- schemars may produce `"required"` as absent (not `null`) for empty-args tools -- current test checks `is_null()`, may need to check `is_null() || required.is_empty()`
- Enum schemas might be nested under `$defs` with `$ref` -- if so, configure `schemars` to inline subschemas

The `handler_tests::tools_list_returns_all_tools` and `get_script_tests::tools_list_includes_get_script` tests check tool count and presence -- those should pass unchanged.

## What stays the same

### `.md` tool descriptions (our improvement)

The studio-rust-mcp-server uses inline `#[doc = include_str!("run_code.md")]` on the tool method. We use `include_str!("mcp_docs/<tool>.md")` in the `tool_def` call. Same `include_str!` pattern, but our `.md` files are more comprehensive (workflow guidance, response formats, conflict resolution steps). This is our improvement over their version and stays as-is.

### Error handling (already equivalent)

Our error handling already mirrors the studio-rust-mcp-server's rmcp pattern:


| Our pattern                               | Their rmcp equivalent                                | When used                                                |
| ----------------------------------------- | ---------------------------------------------------- | -------------------------------------------------------- |
| `tool_response(id, false, text)`          | `CallToolResult::success(vec![Content::text(text)])` | Tool completed successfully                              |
| `tool_response(id, true, msg)`            | `CallToolResult::error(vec![Content::text(msg)])`    | Recoverable tool error (tells the agent what went wrong) |
| `JsonRpcResponse::error(id, -32602, msg)` | `ErrorData::invalid_params(msg, None)`               | Protocol-level error (bad params, unknown tool)          |


No changes needed here. Their `error.rs` is a `color_eyre`/axum wrapper that doesn't apply to our hyper-based architecture.

### No file separation

All tool definitions, arg structs, and the `tool_def` helper stay in [src/web/mcp.rs](src/web/mcp.rs). We do not split into one-file-per-tool like the studio-rust-mcp-server does -- we only have 8 tools and the overhead of separate files isn't justified.

## Files changed

- [Cargo.toml](Cargo.toml) -- add `schemars = "1"`
- [src/web/mcp.rs](src/web/mcp.rs) -- add `JsonSchema` import, arg structs, `tool_def` helper, replace `handle_tools_list` body, add `JsonSchema` derive to `SyncOverride`, update tests

