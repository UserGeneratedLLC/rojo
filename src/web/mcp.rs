use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, header::CONTENT_TYPE, Method, Request, Response, StatusCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::interface::SERVER_VERSION;

/// Plugin config received via the MCP stream WebSocket greeting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginConfig {
    #[serde(default)]
    pub two_way_sync: bool,
    #[serde(default)]
    pub one_shot_sync: bool,
    #[serde(default)]
    pub confirmation_behavior: String,
    #[serde(default)]
    pub place_id: Option<f64>,
}

/// Agent-specified auto-accept directive for a single instance change.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyncOverride {
    pub id: String,
    pub direction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_properties: Option<Value>,
}

/// Command sent from the MCP handler to the plugin via the MCP stream WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSyncCommand {
    #[serde(rename = "type")]
    pub command_type: String,
    pub request_id: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub overrides: Vec<SyncOverride>,
}

/// A single change entry in the sync result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChange {
    pub path: String,
    pub direction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_selection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Value>,
}

/// Result sent from the plugin back through the MCP stream WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSyncResult {
    pub request_id: String,
    pub status: String,
    #[serde(default)]
    pub changes: Vec<SyncChange>,
    #[serde(default)]
    pub message: Option<String>,
}

/// Result for passthrough plugin tools (run_code, insert_model, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginToolResult {
    pub request_id: String,
    pub status: String,
    #[serde(default)]
    pub response: Option<String>,
}

// ---------------------------------------------------------------------------
// Tool argument schemas (schemars-derived, used only for inputSchema generation)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(JsonSchema)]
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

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum SyncMode {
    Standard,
    Manual,
    Fastfail,
    Dryrun,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(JsonSchema)]
struct RunCodeArgs {
    /// Luau code to execute in Roblox Studio.
    command: String,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct InsertModelArgs {
    /// Search query for the model on the Roblox Creator Store.
    query: String,
}

#[derive(JsonSchema)]
struct NoArgs {}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum StartStopPlayMode {
    StartPlay,
    RunServer,
    Stop,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct StartStopPlayArgs {
    /// Play mode action.
    mode: StartStopPlayMode,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum PlayTestMode {
    StartPlay,
    RunServer,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
struct RunScriptInPlayModeArgs {
    /// Luau code to run inside a play session.
    code: String,
    /// Timeout in seconds. Defaults to 100.
    timeout: Option<u32>,
    /// Play mode to start.
    mode: PlayTestMode,
}

/// Result for get_script, deserialized from the plugin's Value response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetScriptResult {
    pub request_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub studio_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_draft: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Shared state for MCP coordination between the agent-facing `/mcp`
/// endpoint and the plugin-facing `/api/mcp/stream` WebSocket.
///
/// Channels carry `serde_json::Value` so any command/result type can flow
/// through (sync, getScript, future endpoints).
pub struct McpState {
    pub command_in_progress: AtomicBool,
    pub command_tx: tokio::sync::watch::Sender<Option<Value>>,
    pub command_rx: tokio::sync::watch::Receiver<Option<Value>>,
    pub result_tx: Mutex<Option<tokio::sync::oneshot::Sender<Value>>>,
    pub plugin_stream_connected: AtomicBool,
    pub plugin_config: Mutex<Option<PluginConfig>>,
}

impl McpState {
    pub fn new() -> Self {
        let (command_tx, command_rx) = tokio::sync::watch::channel(None);
        Self {
            command_in_progress: AtomicBool::new(false),
            command_tx,
            command_rx,
            result_tx: Mutex::new(None),
            plugin_stream_connected: AtomicBool::new(false),
            plugin_config: Mutex::new(None),
        }
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC types (minimal subset for MCP Streamable HTTP)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

fn json_response(body: &JsonRpcResponse, status: StatusCode) -> Response<Full<Bytes>> {
    let serialized = serde_json::to_string(body).unwrap_or_default();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(serialized)))
        .unwrap()
}

// ---------------------------------------------------------------------------
// MCP HTTP endpoint: POST /mcp
// ---------------------------------------------------------------------------

pub async fn call(
    request: Request<Incoming>,
    mcp_state: Arc<McpState>,
    active_api_connections: Arc<std::sync::atomic::AtomicUsize>,
) -> Response<Full<Bytes>> {
    if request.method() != Method::POST {
        let resp = JsonRpcResponse::error(None, -32600, "MCP endpoint only accepts POST requests");
        return json_response(&resp, StatusCode::METHOD_NOT_ALLOWED);
    }

    let body = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            let resp =
                JsonRpcResponse::error(None, -32700, format!("Failed to read request body: {err}"));
            return json_response(&resp, StatusCode::BAD_REQUEST);
        }
    };

    let rpc_request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(req) => req,
        Err(err) => {
            let resp = JsonRpcResponse::error(None, -32700, format!("Parse error: {err}"));
            return json_response(&resp, StatusCode::BAD_REQUEST);
        }
    };

    match rpc_request.method.as_str() {
        "initialize" => {
            log::info!("MCP agent connected");
            handle_initialize(rpc_request.id)
        }
        "notifications/initialized" => Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(Full::new(Bytes::new()))
            .unwrap(),
        "tools/list" => handle_tools_list(rpc_request.id),
        "tools/call" => {
            handle_tools_call(
                rpc_request.id,
                rpc_request.params,
                mcp_state,
                active_api_connections,
            )
            .await
        }
        _ => {
            let resp = JsonRpcResponse::error(
                rpc_request.id,
                -32601,
                format!("Method not found: {}", rpc_request.method),
            );
            json_response(&resp, StatusCode::OK)
        }
    }
}

// ---------------------------------------------------------------------------
// MCP method handlers
// ---------------------------------------------------------------------------

fn handle_initialize(id: Option<Value>) -> Response<Full<Bytes>> {
    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "atlas",
            "version": SERVER_VERSION
        }
    });
    let resp = JsonRpcResponse::success(id, result);
    json_response(&resp, StatusCode::OK)
}

fn tool_def<T: JsonSchema>(name: &str, description: &str) -> Value {
    let settings = schemars::generate::SchemaSettings::draft07().with(|s| {
        s.meta_schema = None;
        s.inline_subschemas = true;
    });
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut schema_value = serde_json::to_value(schema).unwrap_or_default();
    if let Some(obj) = schema_value.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
        obj.remove("definitions");
    }
    serde_json::json!({
        "name": name,
        "description": description,
        "inputSchema": schema_value,
    })
}

fn handle_tools_list(id: Option<Value>) -> Response<Full<Bytes>> {
    let tools = vec![
        tool_def::<AtlasSyncArgs>("atlas_sync", include_str!("mcp_docs/atlas_sync.md")),
        tool_def::<GetScriptArgs>("get_script", include_str!("mcp_docs/get_script.md")),
        tool_def::<RunCodeArgs>("run_code", include_str!("mcp_docs/run_code.md")),
        tool_def::<InsertModelArgs>("insert_model", include_str!("mcp_docs/insert_model.md")),
        tool_def::<NoArgs>(
            "get_console_output",
            include_str!("mcp_docs/get_console_output.md"),
        ),
        tool_def::<NoArgs>(
            "get_studio_mode",
            include_str!("mcp_docs/get_studio_mode.md"),
        ),
        tool_def::<StartStopPlayArgs>(
            "start_stop_play",
            include_str!("mcp_docs/start_stop_play.md"),
        ),
        tool_def::<RunScriptInPlayModeArgs>(
            "run_script_in_play_mode",
            include_str!("mcp_docs/run_script_in_play_mode.md"),
        ),
    ];

    let result = serde_json::json!({ "tools": tools });
    let resp = JsonRpcResponse::success(id, result);
    json_response(&resp, StatusCode::OK)
}

async fn handle_tools_call(
    id: Option<Value>,
    params: Option<Value>,
    mcp_state: Arc<McpState>,
    active_api_connections: Arc<std::sync::atomic::AtomicUsize>,
) -> Response<Full<Bytes>> {
    let tool_name = params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let arguments = params
        .as_ref()
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    log::info!(
        "MCP call: {} {}",
        tool_name,
        serde_json::to_string(&arguments).unwrap_or_default()
    );

    match tool_name {
        "atlas_sync" => handle_atlas_sync(id, arguments, mcp_state, active_api_connections).await,
        "get_script" => handle_get_script(id, arguments, mcp_state).await,
        "run_code"
        | "insert_model"
        | "get_console_output"
        | "get_studio_mode"
        | "start_stop_play"
        | "run_script_in_play_mode" => {
            dispatch_to_plugin(id, tool_name, arguments, mcp_state).await
        }
        _ => {
            let resp = JsonRpcResponse::error(id, -32602, format!("Unknown tool: {tool_name}"));
            json_response(&resp, StatusCode::OK)
        }
    }
}

async fn handle_atlas_sync(
    id: Option<Value>,
    arguments: Value,
    mcp_state: Arc<McpState>,
    active_api_connections: Arc<std::sync::atomic::AtomicUsize>,
) -> Response<Full<Bytes>> {
    let mode = arguments
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("standard")
        .to_string();

    let overrides: Vec<SyncOverride> = arguments
        .get("overrides")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if active_api_connections.load(Ordering::Relaxed) > 0 {
        let config_info = {
            let config = mcp_state
                .plugin_config
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match config.as_ref() {
                Some(cfg) => format!(
                    " Plugin config: twoWaySync={}, oneShotSync={}, confirmationBehavior={}.",
                    cfg.two_way_sync, cfg.one_shot_sync, cfg.confirmation_behavior
                ),
                None => String::new(),
            }
        };

        return tool_response(
            id,
            true,
            &format!(
                "Atlas is already connected to Studio in live sync mode. \
                 All changes are syncing automatically. No manual sync needed.{config_info}"
            ),
        );
    }

    if mcp_state
        .command_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return tool_response(
            id,
            true,
            "An MCP command is already in progress. Please wait for it to complete.",
        );
    }

    if !mcp_state.plugin_stream_connected.load(Ordering::Relaxed) {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(
            id,
            true,
            "No Roblox Studio plugin is connected to the MCP stream. \
             Make sure Studio is open with the Atlas plugin installed.",
        );
    }

    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    {
        let mut slot = mcp_state
            .result_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = Some(result_tx);
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let command = McpSyncCommand {
        command_type: "sync".to_string(),
        request_id: request_id.clone(),
        mode,
        overrides,
    };

    let command_value = serde_json::to_value(&command).unwrap_or(Value::Null);
    if mcp_state.command_tx.send(Some(command_value)).is_err() {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(id, true, "Failed to send sync command to the plugin.");
    }

    let result_value = match result_rx.await {
        Ok(v) => v,
        Err(_) => {
            mcp_state.command_in_progress.store(false, Ordering::SeqCst);
            return tool_response(
                id,
                true,
                "Plugin disconnected before completing the sync operation.",
            );
        }
    };

    let result: McpSyncResult = match serde_json::from_value(result_value) {
        Ok(r) => r,
        Err(e) => {
            mcp_state.command_in_progress.store(false, Ordering::SeqCst);
            return tool_response(
                id,
                true,
                &format!("Failed to parse sync result from plugin: {e}"),
            );
        }
    };

    mcp_state.command_in_progress.store(false, Ordering::SeqCst);

    let _ = mcp_state.command_tx.send(None);

    log::info!(
        "MCP result: atlas_sync status={} changes={}",
        result.status,
        result.changes.len()
    );

    let is_error = !matches!(result.status.as_str(), "success" | "empty" | "dryrun");

    let mut text = match result.status.as_str() {
        "success" => "Sync completed successfully.".to_string(),
        "empty" => "No changes to sync. Studio is already up to date.".to_string(),
        "rejected" => "Sync was rejected by the user.".to_string(),
        "dryrun" => "Dry run complete. No changes were applied.".to_string(),
        "fastfail_unresolved" => {
            "Sync failed: unresolved changes exist that require manual review or overrides."
                .to_string()
        }
        "already_connected" => "Atlas is already connected to Studio in live sync mode. \
             All changes are syncing automatically."
            .to_string(),
        "sync_in_progress" => "A sync operation is already in progress.".to_string(),
        other => format!("Sync finished with status: {other}"),
    };

    if let Some(msg) = &result.message {
        text.push_str(&format!("\n\n{msg}"));
    }

    if !result.changes.is_empty() {
        let label = if is_error {
            "Presented changes"
        } else {
            "Changes"
        };
        text.push_str(&format!("\n\n{label}:"));
        for change in &result.changes {
            let fs_info = change
                .fs_path
                .as_deref()
                .map(|p| format!(", fs={p}"))
                .unwrap_or_default();
            let type_info = match (change.patch_type.as_deref(), change.class_name.as_deref()) {
                (Some(pt), Some(cn)) => format!(" ({pt}, {cn}{fs_info})"),
                (Some(pt), None) => format!(" ({pt}{fs_info})"),
                _ if !fs_info.is_empty() => format!(" ({fs_info})"),
                _ => String::new(),
            };
            text.push_str(&format!(
                "\n- [{}] {}{}",
                change.direction, change.path, type_info
            ));
        }

        let json_block = serde_json::json!({
            "status": result.status,
            "changes": result.changes,
        });
        text.push_str(&format!(
            "\n\n<json>\n{}\n</json>",
            serde_json::to_string(&json_block).unwrap_or_default()
        ));
    }

    tool_response(id, is_error, &text)
}

async fn handle_get_script(
    id: Option<Value>,
    arguments: Value,
    mcp_state: Arc<McpState>,
) -> Response<Full<Bytes>> {
    if mcp_state
        .command_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return tool_response(
            id,
            true,
            "An MCP command is already in progress. Please wait for it to complete.",
        );
    }

    if !mcp_state.plugin_stream_connected.load(Ordering::Relaxed) {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(
            id,
            true,
            "No Roblox Studio plugin is connected to the MCP stream. \
             Make sure Studio is open with the Atlas plugin installed.",
        );
    }

    let has_id = arguments.get("id").and_then(|v| v.as_str()).is_some();
    let has_fs_path = arguments.get("fsPath").and_then(|v| v.as_str()).is_some();
    if !has_id && !has_fs_path {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(
            id,
            true,
            "Either 'id' or 'fsPath' must be provided. Both come from a previous atlas_sync response.",
        );
    }

    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    {
        let mut slot = mcp_state
            .result_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = Some(result_tx);
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let from_draft = arguments
        .get("fromDraft")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let command = serde_json::json!({
        "type": "getScript",
        "requestId": request_id,
        "id": arguments.get("id"),
        "fsPath": arguments.get("fsPath"),
        "fromDraft": from_draft,
    });

    if mcp_state.command_tx.send(Some(command)).is_err() {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(id, true, "Failed to send get_script command to the plugin.");
    }

    let result_value = match result_rx.await {
        Ok(v) => v,
        Err(_) => {
            mcp_state.command_in_progress.store(false, Ordering::SeqCst);
            return tool_response(
                id,
                true,
                "Plugin disconnected before completing the get_script operation.",
            );
        }
    };

    mcp_state.command_in_progress.store(false, Ordering::SeqCst);
    let _ = mcp_state.command_tx.send(None);

    let result: GetScriptResult = match serde_json::from_value(result_value) {
        Ok(r) => r,
        Err(e) => {
            return tool_response(
                id,
                true,
                &format!("Failed to parse get_script result from plugin: {e}"),
            );
        }
    };

    let is_error = result.status != "success";

    log::info!(
        "MCP result: get_script status={} class={}",
        result.status,
        result.class_name.as_deref().unwrap_or("?")
    );

    if is_error {
        let msg = result.message.as_deref().unwrap_or("Unknown error");
        return tool_response(id, true, msg);
    }

    let class_name = result.class_name.as_deref().unwrap_or("Script");
    let instance_path = result.instance_path.as_deref().unwrap_or("Unknown");
    let studio_hash = result.studio_hash.as_deref().unwrap_or("");

    let mut text =
        format!("Script source for {instance_path} ({class_name}):\nHash: {studio_hash}");

    let json_block = serde_json::to_value(&result).unwrap_or(Value::Null);
    text.push_str(&format!(
        "\n\n<json>\n{}\n</json>",
        serde_json::to_string(&json_block).unwrap_or_default()
    ));

    tool_response(id, false, &text)
}

async fn dispatch_to_plugin(
    id: Option<Value>,
    tool_name: &str,
    arguments: Value,
    mcp_state: Arc<McpState>,
) -> Response<Full<Bytes>> {
    if mcp_state
        .command_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return tool_response(
            id,
            true,
            "An MCP command is already in progress. Please wait for it to complete.",
        );
    }

    if !mcp_state.plugin_stream_connected.load(Ordering::Relaxed) {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(
            id,
            true,
            "No Roblox Studio plugin is connected to the MCP stream. \
             Make sure Studio is open with the Atlas plugin installed.",
        );
    }

    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    {
        let mut slot = mcp_state
            .result_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = Some(result_tx);
    }

    let request_id = uuid::Uuid::new_v4().to_string();
    let command = serde_json::json!({
        "type": tool_name,
        "requestId": request_id,
        "args": arguments,
    });

    if mcp_state.command_tx.send(Some(command)).is_err() {
        mcp_state.command_in_progress.store(false, Ordering::SeqCst);
        return tool_response(
            id,
            true,
            &format!("Failed to send {tool_name} command to the plugin."),
        );
    }

    let result_value = match result_rx.await {
        Ok(v) => v,
        Err(_) => {
            mcp_state.command_in_progress.store(false, Ordering::SeqCst);
            return tool_response(
                id,
                true,
                &format!("Plugin disconnected before completing the {tool_name} operation."),
            );
        }
    };

    mcp_state.command_in_progress.store(false, Ordering::SeqCst);
    let _ = mcp_state.command_tx.send(None);

    let result: PluginToolResult = match serde_json::from_value(result_value) {
        Ok(r) => r,
        Err(e) => {
            return tool_response(
                id,
                true,
                &format!("Failed to parse {tool_name} result from plugin: {e}"),
            );
        }
    };

    let is_error = result.status != "success";
    let text = result
        .response
        .as_deref()
        .unwrap_or(if is_error { "Unknown error" } else { "" });

    log::info!("MCP result: {} {}", tool_name, &text[..text.len().min(200)]);

    tool_response(id, is_error, text)
}

fn tool_response(id: Option<Value>, is_error: bool, text: &str) -> Response<Full<Bytes>> {
    let result = serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    });
    let resp = JsonRpcResponse::success(id, result);
    json_response(&resp, StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // -- JSON-RPC type tests --------------------------------------------------

    mod jsonrpc_types {
        use super::*;

        #[test]
        fn success_response_serializes_correctly() {
            let resp =
                JsonRpcResponse::success(Some(Value::from(1)), serde_json::json!({"ok": true}));
            let json = serde_json::to_value(&resp).unwrap();
            assert_eq!(json["jsonrpc"], "2.0");
            assert_eq!(json["id"], 1);
            assert!(json["result"]["ok"].as_bool().unwrap());
            assert!(json.get("error").is_none());
        }

        #[test]
        fn error_response_serializes_correctly() {
            let resp = JsonRpcResponse::error(Some(Value::from(2)), -32600, "bad request");
            let json = serde_json::to_value(&resp).unwrap();
            assert_eq!(json["jsonrpc"], "2.0");
            assert_eq!(json["id"], 2);
            assert!(json.get("result").is_none());
            assert_eq!(json["error"]["code"], -32600);
            assert_eq!(json["error"]["message"], "bad request");
        }

        #[test]
        fn null_id_omitted_in_success() {
            let resp = JsonRpcResponse::success(None, serde_json::json!({}));
            let json = serde_json::to_value(&resp).unwrap();
            assert!(json.get("id").is_none());
        }
    }

    // -- Wire type serde tests ------------------------------------------------

    mod wire_types {
        use super::*;

        #[test]
        fn mcp_sync_command_serializes_camel_case() {
            let cmd = McpSyncCommand {
                command_type: "sync".to_string(),
                request_id: "abc-123".to_string(),
                mode: "standard".to_string(),
                overrides: vec![],
            };
            let json = serde_json::to_value(&cmd).unwrap();
            assert_eq!(json["type"], "sync");
            assert_eq!(json["requestId"], "abc-123");
            assert_eq!(json["mode"], "standard");
        }

        #[test]
        fn mcp_sync_result_deserializes_camel_case() {
            let json = serde_json::json!({
                "requestId": "abc-123",
                "status": "success",
                "changes": [
                    { "path": "ReplicatedStorage/Module.luau", "direction": "push" }
                ],
                "message": "done"
            });
            let result: McpSyncResult = serde_json::from_value(json).unwrap();
            assert_eq!(result.request_id, "abc-123");
            assert_eq!(result.status, "success");
            assert_eq!(result.changes.len(), 1);
            assert_eq!(result.changes[0].path, "ReplicatedStorage/Module.luau");
            assert_eq!(result.changes[0].direction, "push");
            assert_eq!(result.message.as_deref(), Some("done"));
        }

        #[test]
        fn mcp_sync_result_defaults_for_missing_optional_fields() {
            let json = serde_json::json!({
                "requestId": "x",
                "status": "empty"
            });
            let result: McpSyncResult = serde_json::from_value(json).unwrap();
            assert!(result.changes.is_empty());
            assert!(result.message.is_none());
        }

        #[test]
        fn plugin_config_deserializes_with_defaults() {
            let json = serde_json::json!({});
            let cfg: PluginConfig = serde_json::from_value(json).unwrap();
            assert!(!cfg.two_way_sync);
            assert!(!cfg.one_shot_sync);
            assert_eq!(cfg.confirmation_behavior, "");
        }

        #[test]
        fn plugin_config_roundtrip() {
            let cfg = PluginConfig {
                two_way_sync: true,
                one_shot_sync: false,
                confirmation_behavior: "Always".to_string(),
                place_id: None,
            };
            let json = serde_json::to_value(&cfg).unwrap();
            assert_eq!(json["twoWaySync"], true);
            assert_eq!(json["oneShotSync"], false);
            assert_eq!(json["confirmationBehavior"], "Always");
        }

        #[test]
        fn plugin_tool_result_deserializes() {
            let json = serde_json::json!({
                "requestId": "r1",
                "status": "success",
                "response": "hello world"
            });
            let result: PluginToolResult = serde_json::from_value(json).unwrap();
            assert_eq!(result.request_id, "r1");
            assert_eq!(result.status, "success");
            assert_eq!(result.response.as_deref(), Some("hello world"));
        }

        #[test]
        fn plugin_tool_result_defaults_for_missing_response() {
            let json = serde_json::json!({
                "requestId": "r2",
                "status": "error"
            });
            let result: PluginToolResult = serde_json::from_value(json).unwrap();
            assert_eq!(result.status, "error");
            assert!(result.response.is_none());
        }

        #[test]
        fn plugin_tool_result_serializes_camel_case() {
            let result = PluginToolResult {
                request_id: "r3".to_string(),
                status: "success".to_string(),
                response: Some("output".to_string()),
            };
            let json = serde_json::to_value(&result).unwrap();
            assert_eq!(json["requestId"], "r3");
            assert_eq!(json["status"], "success");
            assert_eq!(json["response"], "output");
        }
    }

    // -- McpState tests -------------------------------------------------------

    mod state_tests {
        use super::*;

        #[test]
        fn new_state_starts_clean() {
            let state = McpState::new();
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
            assert!(!state.plugin_stream_connected.load(Ordering::Relaxed));
            assert!(state.plugin_config.lock().unwrap().is_none());
            assert!(state.result_tx.lock().unwrap().is_none());
            assert!(state.command_rx.borrow().is_none());
        }

        #[test]
        fn command_mutex_prevents_double_acquire() {
            let state = McpState::new();
            let first = state.command_in_progress.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            assert!(first.is_ok());
            let second = state.command_in_progress.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            assert!(second.is_err());
        }

        #[test]
        fn command_channel_delivers_value() {
            let state = McpState::new();
            let mut rx = state.command_rx.clone();

            let cmd = serde_json::json!({"type": "sync", "requestId": "test-1"});
            state.command_tx.send(Some(cmd)).unwrap();

            assert!(rx.has_changed().unwrap());
            let received = rx.borrow_and_update().clone().unwrap();
            assert_eq!(received["requestId"], "test-1");
        }

        #[test]
        fn result_oneshot_delivers_value() {
            let state = McpState::new();
            let (tx, rx) = tokio::sync::oneshot::channel();
            *state.result_tx.lock().unwrap() = Some(tx);

            let result = serde_json::json!({"status": "success", "requestId": "r1"});
            let sender = state.result_tx.lock().unwrap().take().unwrap();
            sender.send(result).unwrap();

            let received = rx.blocking_recv().unwrap();
            assert_eq!(received["status"], "success");
        }

        #[test]
        fn plugin_config_cache() {
            let state = McpState::new();
            let cfg = PluginConfig {
                two_way_sync: true,
                one_shot_sync: false,
                confirmation_behavior: "Initial".to_string(),
                place_id: None,
            };
            *state.plugin_config.lock().unwrap() = Some(cfg);

            let cached = state.plugin_config.lock().unwrap();
            assert!(cached.as_ref().unwrap().two_way_sync);
            assert_eq!(cached.as_ref().unwrap().confirmation_behavior, "Initial");
        }
    }

    // -- Handler response tests (pure functions) ------------------------------

    mod handler_tests {
        use super::*;

        #[test]
        fn initialize_returns_capabilities() {
            let resp = handle_initialize(Some(Value::from(1)));
            assert_eq!(resp.status(), StatusCode::OK);
            let body = resp.into_body();
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { body.collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["jsonrpc"], "2.0");
            assert_eq!(json["id"], 1);
            assert_eq!(json["result"]["serverInfo"]["name"], "atlas");
            assert!(json["result"]["capabilities"]["tools"].is_object());
        }

        #[test]
        fn tools_list_returns_all_tools() {
            let resp = handle_tools_list(Some(Value::from(2)));
            assert_eq!(resp.status(), StatusCode::OK);
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            let tools = json["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 8);
            let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(
                names,
                vec![
                    "atlas_sync",
                    "get_script",
                    "run_code",
                    "insert_model",
                    "get_console_output",
                    "get_studio_mode",
                    "start_stop_play",
                    "run_script_in_play_mode",
                ]
            );
            for tool in tools {
                assert!(tool["inputSchema"].is_object());
                assert!(tool["description"].as_str().unwrap().len() > 10);
            }
        }

        #[test]
        fn tool_response_formats_correctly() {
            let resp = tool_response(Some(Value::from(3)), false, "All good");
            assert_eq!(resp.status(), StatusCode::OK);
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["result"]["isError"], false);
            let content = json["result"]["content"].as_array().unwrap();
            assert_eq!(content[0]["type"], "text");
            assert_eq!(content[0]["text"], "All good");
        }

        #[test]
        fn tool_response_error_flag() {
            let resp = tool_response(Some(Value::from(4)), true, "Oops");
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["result"]["isError"], true);
        }
    }

    // -- atlas_sync guard tests (via handle_atlas_sync) -----------------------

    mod atlas_sync_guards {
        use super::*;

        fn empty_args() -> Value {
            serde_json::json!({})
        }

        #[tokio::test]
        async fn rejects_when_api_connected() {
            let state = Arc::new(McpState::new());
            let conns = Arc::new(AtomicUsize::new(1));

            let resp = handle_atlas_sync(Some(Value::from(1)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("already connected"));
        }

        #[tokio::test]
        async fn rejects_when_api_connected_includes_config() {
            let state = Arc::new(McpState::new());
            *state.plugin_config.lock().unwrap() = Some(PluginConfig {
                two_way_sync: true,
                one_shot_sync: false,
                confirmation_behavior: "Always".to_string(),
                place_id: None,
            });
            let conns = Arc::new(AtomicUsize::new(1));

            let resp = handle_atlas_sync(Some(Value::from(1)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("twoWaySync=true"));
            assert!(text.contains("confirmationBehavior=Always"));
        }

        #[tokio::test]
        async fn rejects_when_command_in_progress() {
            let state = Arc::new(McpState::new());
            state.command_in_progress.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let resp = handle_atlas_sync(Some(Value::from(2)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("already in progress"));
        }

        #[tokio::test]
        async fn rejects_when_no_plugin_connected() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(false, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let resp =
                handle_atlas_sync(Some(Value::from(3)), empty_args(), state.clone(), conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("No Roblox Studio plugin"));
            // Mutex should be released after rejection.
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn sends_command_and_returns_success_result() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "changes": [{"path": "ServerScriptService/Main.server.luau", "direction": "push"}],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp =
                handle_atlas_sync(Some(Value::from(5)), empty_args(), state.clone(), conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Sync completed successfully"));
            assert!(text.contains("[push] ServerScriptService/Main.server.luau"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn returns_rejection_with_presented_changes() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "rejected",
                    "changes": [
                        {"path": "ReplicatedStorage/Foo.luau", "direction": "push"},
                        {"path": "Workspace/Bar.model.json5", "direction": "pull"},
                    ],
                    "message": "User rejected the sync changes.",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = handle_atlas_sync(Some(Value::from(6)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("rejected"));
            assert!(text.contains("Presented changes"));
            assert!(text.contains("[push] ReplicatedStorage/Foo.luau"));
            assert!(text.contains("[pull] Workspace/Bar.model.json5"));
        }

        #[tokio::test]
        async fn handles_plugin_disconnect_during_sync() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                // Drop the sender without sending a result (simulates disconnect).
                let _tx = state2.result_tx.lock().unwrap().take();
            });

            let resp =
                handle_atlas_sync(Some(Value::from(7)), empty_args(), state.clone(), conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("disconnected"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn empty_status_is_not_an_error() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "empty",
                    "changes": [],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = handle_atlas_sync(Some(Value::from(8)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("No changes to sync"));
        }

        #[tokio::test]
        async fn dryrun_status_is_not_an_error() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "dryrun",
                    "changes": [{
                        "path": "Workspace/Part",
                        "direction": "unresolved",
                        "id": "aabb",
                        "className": "Part",
                        "patchType": "Edit",
                    }],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let args = serde_json::json!({"mode": "dryrun"});
            let resp = handle_atlas_sync(Some(Value::from(9)), args, state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Dry run complete"));
            assert!(text.contains("<json>"));
        }

        #[tokio::test]
        async fn fastfail_unresolved_is_error() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "fastfail_unresolved",
                    "changes": [{
                        "path": "ServerScriptService/Script",
                        "direction": "unresolved",
                    }],
                    "message": "Unresolved changes exist.",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let args = serde_json::json!({"mode": "fastfail"});
            let resp = handle_atlas_sync(Some(Value::from(10)), args, state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("unresolved"));
        }

        #[tokio::test]
        async fn mode_and_overrides_forwarded_to_command() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                assert_eq!(cmd["mode"], "fastfail");
                assert_eq!(cmd["overrides"].as_array().unwrap().len(), 1);
                assert_eq!(cmd["overrides"][0]["id"], "aabbccdd");
                assert_eq!(cmd["overrides"][0]["direction"], "push");
                assert_eq!(cmd["overrides"][0]["studioHash"], "abc123");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "changes": [],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let args = serde_json::json!({
                "mode": "fastfail",
                "overrides": [{
                    "id": "aabbccdd",
                    "direction": "push",
                    "studioHash": "abc123"
                }]
            });
            let resp = handle_atlas_sync(Some(Value::from(11)), args, state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["result"]["isError"], false);
        }

        #[tokio::test]
        async fn mode_defaults_to_standard() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                assert_eq!(cmd["mode"], "standard");
                assert!(cmd["overrides"].as_array().unwrap().is_empty());

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "empty",
                    "changes": [],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = handle_atlas_sync(Some(Value::from(12)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["result"]["isError"], false);
        }

        #[tokio::test]
        async fn enriched_changes_include_json_block() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "changes": [{
                        "path": "Workspace/Part",
                        "direction": "push",
                        "id": "00aabb",
                        "className": "Part",
                        "patchType": "Edit",
                        "defaultSelection": "push",
                        "fsPath": "src/workspace/Part.model.json5",
                        "properties": {
                            "Anchored": {
                                "current": false,
                                "incoming": true
                            }
                        }
                    }],
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = handle_atlas_sync(Some(Value::from(13)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("<json>"));
            assert!(text.contains("</json>"));
            assert!(text.contains("Edit"));
            assert!(text.contains("Part"));
            assert!(text.contains("fs=src/workspace/Part.model.json5"));

            let json_start = text.find("<json>\n").unwrap() + 7;
            let json_end = text.find("\n</json>").unwrap();
            let inner: Value = serde_json::from_str(&text[json_start..json_end]).unwrap();
            assert_eq!(inner["status"], "success");
            assert_eq!(inner["changes"][0]["id"], "00aabb");
            assert_eq!(inner["changes"][0]["className"], "Part");
            assert_eq!(
                inner["changes"][0]["fsPath"],
                "src/workspace/Part.model.json5"
            );
            assert_eq!(
                inner["changes"][0]["properties"]["Anchored"]["current"],
                false
            );
            assert_eq!(
                inner["changes"][0]["properties"]["Anchored"]["incoming"],
                true
            );
        }
    }

    // -- New wire type tests for added structs --------------------------------

    mod new_wire_types {
        use super::*;

        #[test]
        fn sync_override_serializes_camel_case() {
            let ov = SyncOverride {
                id: "aabbccdd".to_string(),
                direction: "push".to_string(),
                studio_hash: Some("abc123".to_string()),
                expected_properties: Some(serde_json::json!({"Anchored": true})),
            };
            let json = serde_json::to_value(&ov).unwrap();
            assert_eq!(json["id"], "aabbccdd");
            assert_eq!(json["direction"], "push");
            assert_eq!(json["studioHash"], "abc123");
            assert_eq!(json["expectedProperties"]["Anchored"], true);
        }

        #[test]
        fn sync_override_defaults_optional_fields() {
            let json = serde_json::json!({
                "id": "aabb",
                "direction": "pull"
            });
            let ov: SyncOverride = serde_json::from_value(json).unwrap();
            assert_eq!(ov.id, "aabb");
            assert_eq!(ov.direction, "pull");
            assert!(ov.studio_hash.is_none());
            assert!(ov.expected_properties.is_none());
        }

        #[test]
        fn sync_override_omits_none_on_serialize() {
            let ov = SyncOverride {
                id: "aabb".to_string(),
                direction: "push".to_string(),
                studio_hash: None,
                expected_properties: None,
            };
            let json = serde_json::to_value(&ov).unwrap();
            assert!(json.get("studioHash").is_none());
            assert!(json.get("expectedProperties").is_none());
        }

        #[test]
        fn mcp_sync_command_includes_mode_and_overrides() {
            let cmd = McpSyncCommand {
                command_type: "sync".to_string(),
                request_id: "r1".to_string(),
                mode: "dryrun".to_string(),
                overrides: vec![SyncOverride {
                    id: "x".to_string(),
                    direction: "pull".to_string(),
                    studio_hash: None,
                    expected_properties: None,
                }],
            };
            let json = serde_json::to_value(&cmd).unwrap();
            assert_eq!(json["mode"], "dryrun");
            assert_eq!(json["overrides"][0]["id"], "x");
        }

        #[test]
        fn mcp_sync_command_defaults_mode_and_overrides() {
            let json = serde_json::json!({
                "type": "sync",
                "requestId": "r1"
            });
            let cmd: McpSyncCommand = serde_json::from_value(json).unwrap();
            assert_eq!(cmd.mode, "");
            assert!(cmd.overrides.is_empty());
        }

        #[test]
        fn sync_change_serializes_all_fields() {
            let change = SyncChange {
                path: "Workspace/Part".to_string(),
                direction: "push".to_string(),
                id: Some("aabb".to_string()),
                class_name: Some("Part".to_string()),
                patch_type: Some("Edit".to_string()),
                studio_hash: Some("hash123".to_string()),
                default_selection: Some("push".to_string()),
                fs_path: Some("src/workspace/Part.model.json5".to_string()),
                properties: Some(
                    serde_json::json!({"Anchored": {"current": false, "incoming": true}}),
                ),
            };
            let json = serde_json::to_value(&change).unwrap();
            assert_eq!(json["id"], "aabb");
            assert_eq!(json["className"], "Part");
            assert_eq!(json["patchType"], "Edit");
            assert_eq!(json["studioHash"], "hash123");
            assert_eq!(json["defaultSelection"], "push");
            assert_eq!(json["fsPath"], "src/workspace/Part.model.json5");
            assert!(json["properties"]["Anchored"].is_object());
        }

        #[test]
        fn sync_change_omits_none_fields() {
            let change = SyncChange {
                path: "Test".to_string(),
                direction: "push".to_string(),
                ..Default::default()
            };
            let json = serde_json::to_value(&change).unwrap();
            assert!(json.get("id").is_none());
            assert!(json.get("className").is_none());
            assert!(json.get("patchType").is_none());
            assert!(json.get("studioHash").is_none());
            assert!(json.get("defaultSelection").is_none());
            assert!(json.get("fsPath").is_none());
            assert!(json.get("properties").is_none());
        }

        #[test]
        fn enriched_sync_result_roundtrip() {
            let result = McpSyncResult {
                request_id: "r1".to_string(),
                status: "dryrun".to_string(),
                changes: vec![
                    SyncChange {
                        path: "ServerScriptService/Main".to_string(),
                        direction: "push".to_string(),
                        id: Some("00aa".to_string()),
                        class_name: Some("Script".to_string()),
                        patch_type: Some("Edit".to_string()),
                        studio_hash: Some("hashval".to_string()),
                        default_selection: Some("push".to_string()),
                        fs_path: None,
                        properties: None,
                    },
                    SyncChange {
                        path: "Workspace/Part".to_string(),
                        direction: "unresolved".to_string(),
                        id: Some("00bb".to_string()),
                        class_name: Some("Part".to_string()),
                        patch_type: Some("Edit".to_string()),
                        studio_hash: None,
                        default_selection: None,
                        fs_path: Some("src/workspace/Part.model.json5".to_string()),
                        properties: Some(
                            serde_json::json!({"Position": {"current": [0,0,0], "incoming": [1,2,3]}}),
                        ),
                    },
                ],
                message: None,
            };
            let serialized = serde_json::to_string(&result).unwrap();
            let deserialized: McpSyncResult = serde_json::from_str(&serialized).unwrap();
            assert_eq!(deserialized.changes.len(), 2);
            assert_eq!(
                deserialized.changes[0].studio_hash.as_deref(),
                Some("hashval")
            );
            assert_eq!(
                deserialized.changes[1].fs_path.as_deref(),
                Some("src/workspace/Part.model.json5")
            );
            assert!(deserialized.changes[1].properties.is_some());
        }
    }

    // -- Input schema tests ---------------------------------------------------

    mod input_schema_tests {
        use super::*;

        #[test]
        fn tools_list_has_mode_and_overrides_in_schema() {
            let resp = handle_tools_list(Some(Value::from(1)));
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            let schema = &json["result"]["tools"][0]["inputSchema"];
            assert!(schema["properties"]["mode"].is_object());
            assert!(schema["properties"]["overrides"].is_object());

            let mode_enum = schema["properties"]["mode"]["enum"].as_array().unwrap();
            let mode_strs: Vec<&str> = mode_enum.iter().filter_map(|v| v.as_str()).collect();
            assert!(mode_strs.contains(&"standard"));
            assert!(mode_strs.contains(&"manual"));
            assert!(mode_strs.contains(&"fastfail"));
            assert!(mode_strs.contains(&"dryrun"));

            let override_items = &schema["properties"]["overrides"]["items"];
            assert_eq!(override_items["properties"]["id"]["type"], "string");
            assert!(override_items["required"]
                .as_array()
                .unwrap()
                .contains(&Value::from("id")));
            assert!(override_items["required"]
                .as_array()
                .unwrap()
                .contains(&Value::from("direction")));
        }
    }

    // -- get_script tests -----------------------------------------------------

    mod get_script_tests {
        use super::*;

        #[test]
        fn get_script_result_serde_roundtrip() {
            let result = GetScriptResult {
                request_id: "r1".to_string(),
                status: "success".to_string(),
                source: Some("print('hi')".to_string()),
                studio_hash: Some("abc123".to_string()),
                class_name: Some("Script".to_string()),
                instance_path: Some("ServerScriptService/Main".to_string()),
                id: Some("00aabb".to_string()),
                fs_path: Some("src/server/Main.server.luau".to_string()),
                is_draft: Some(false),
                message: None,
            };
            let json = serde_json::to_value(&result).unwrap();
            assert_eq!(json["requestId"], "r1");
            assert_eq!(json["source"], "print('hi')");
            assert_eq!(json["studioHash"], "abc123");
            assert_eq!(json["className"], "Script");
            assert_eq!(json["instancePath"], "ServerScriptService/Main");
            assert_eq!(json["isDraft"], false);
            assert!(json.get("message").is_none());

            let deserialized: GetScriptResult = serde_json::from_value(json).unwrap();
            assert_eq!(deserialized.source.as_deref(), Some("print('hi')"));
            assert_eq!(
                deserialized.fs_path.as_deref(),
                Some("src/server/Main.server.luau")
            );
        }

        #[test]
        fn get_script_result_omits_none_fields() {
            let result = GetScriptResult {
                request_id: "r1".to_string(),
                status: "error".to_string(),
                source: None,
                studio_hash: None,
                class_name: None,
                instance_path: None,
                id: None,
                fs_path: None,
                is_draft: None,
                message: Some("Not found".to_string()),
            };
            let json = serde_json::to_value(&result).unwrap();
            assert!(json.get("source").is_none());
            assert!(json.get("studioHash").is_none());
            assert!(json.get("className").is_none());
            assert!(json.get("isDraft").is_none());
            assert_eq!(json["message"], "Not found");
        }

        #[test]
        fn tools_list_includes_get_script() {
            let resp = handle_tools_list(Some(Value::from(1)));
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            let tools = json["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 8);

            let get_script = tools.iter().find(|t| t["name"] == "get_script").unwrap();
            assert!(get_script["description"]
                .as_str()
                .unwrap()
                .contains("Conflict resolution"));
            assert!(get_script["inputSchema"]["properties"]["id"].is_object());
            assert!(get_script["inputSchema"]["properties"]["fsPath"].is_object());
            assert!(get_script["inputSchema"]["properties"]["fromDraft"].is_object());
        }

        #[tokio::test]
        async fn get_script_rejects_when_no_plugin() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(false, Ordering::SeqCst);

            let args = serde_json::json!({"id": "aabb"});
            let resp = handle_get_script(Some(Value::from(1)), args, state).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("No Roblox Studio plugin"));
        }

        #[tokio::test]
        async fn get_script_rejects_when_command_in_progress() {
            let state = Arc::new(McpState::new());
            state.command_in_progress.store(true, Ordering::SeqCst);
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let args = serde_json::json!({"id": "aabb"});
            let resp = handle_get_script(Some(Value::from(1)), args, state).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("already in progress"));
        }

        #[tokio::test]
        async fn get_script_requires_id_or_fspath() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let args = serde_json::json!({"fromDraft": true});
            let resp = handle_get_script(Some(Value::from(1)), args, state).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("id") && text.contains("fsPath"));
        }

        #[tokio::test]
        async fn get_script_success_includes_json_block() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "source": "print('hello')",
                    "studioHash": "abc123def456",
                    "className": "Script",
                    "instancePath": "ServerScriptService/Main",
                    "isDraft": false,
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let args = serde_json::json!({"id": "00aabb"});
            let resp = handle_get_script(Some(Value::from(1)), args, state).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("ServerScriptService/Main"));
            assert!(text.contains("abc123def456"));
            assert!(text.contains("<json>"));
            assert!(text.contains("</json>"));

            let json_start = text.find("<json>\n").unwrap() + 7;
            let json_end = text.find("\n</json>").unwrap();
            let inner: Value = serde_json::from_str(&text[json_start..json_end]).unwrap();
            assert_eq!(inner["status"], "success");
            assert_eq!(inner["source"], "print('hello')");
        }

        #[tokio::test]
        async fn get_script_error_from_plugin() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "error",
                    "message": "Instance not found by id.",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let args = serde_json::json!({"id": "deadbeef"});
            let resp = handle_get_script(Some(Value::from(1)), args, state).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Instance not found by id"));
        }
    }

    // -- dispatch_to_plugin guard tests ----------------------------------------

    mod dispatch_guards {
        use super::*;

        #[tokio::test]
        async fn rejects_when_command_in_progress() {
            let state = Arc::new(McpState::new());
            state.command_in_progress.store(true, Ordering::SeqCst);

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "run_code",
                serde_json::json!({"command": "print(1)"}),
                state,
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("already in progress"));
        }

        #[tokio::test]
        async fn rejects_when_no_plugin_connected() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(false, Ordering::SeqCst);

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "get_studio_mode",
                serde_json::json!({}),
                state.clone(),
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("No Roblox Studio plugin"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn sends_command_and_returns_success() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");
                assert_eq!(cmd["type"], "run_code");
                assert_eq!(cmd["args"]["command"], "print(42)");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "response": "[OUTPUT] 42\n",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "run_code",
                serde_json::json!({"command": "print(42)"}),
                state.clone(),
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("[OUTPUT] 42"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn returns_error_status_from_plugin() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "error",
                    "response": "Missing command in run_code",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "run_code",
                serde_json::json!({}),
                state,
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Missing command"));
        }

        #[tokio::test]
        async fn handles_plugin_disconnect() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                // Drop the result_tx without sending to simulate disconnect.
                let _ = state2.result_tx.lock().unwrap().take();
            });

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "get_console_output",
                serde_json::json!({}),
                state.clone(),
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("disconnected"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn empty_response_returns_empty_text() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "get_console_output",
                serde_json::json!({}),
                state,
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], false);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert_eq!(text, "");
        }

        #[tokio::test]
        async fn malformed_result_returns_parse_error() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();

                let result = serde_json::json!("just a string, not an object");

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let resp = dispatch_to_plugin(
                Some(Value::from(1)),
                "run_code",
                serde_json::json!({"command": "x"}),
                state.clone(),
            )
            .await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();

            assert_eq!(json["result"]["isError"], true);
            let text = json["result"]["content"][0]["text"].as_str().unwrap();
            assert!(text.contains("Failed to parse"));
            assert!(text.contains("run_code"));
            assert!(!state.command_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn command_channel_cleared_after_completion() {
            let state = Arc::new(McpState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();
                let req_id = cmd["requestId"].as_str().unwrap_or("");

                let result = serde_json::json!({
                    "requestId": req_id,
                    "status": "success",
                    "response": "ok",
                });

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(result).unwrap();
            });

            let _ = dispatch_to_plugin(
                Some(Value::from(1)),
                "get_studio_mode",
                serde_json::json!({}),
                state.clone(),
            )
            .await;

            assert!(
                state.command_rx.borrow().is_none(),
                "command_tx should be reset to None after dispatch"
            );
        }

        #[tokio::test]
        async fn tool_type_forwarded_in_command() {
            for tool_name in &[
                "insert_model",
                "get_console_output",
                "get_studio_mode",
                "start_stop_play",
                "run_script_in_play_mode",
            ] {
                let state = Arc::new(McpState::new());
                state.plugin_stream_connected.store(true, Ordering::SeqCst);

                let expected_type = tool_name.to_string();
                let state2 = Arc::clone(&state);
                tokio::spawn(async move {
                    let mut rx = state2.command_rx.clone();
                    rx.changed().await.unwrap();
                    let cmd = rx.borrow().clone().unwrap();
                    assert_eq!(
                        cmd["type"].as_str().unwrap(),
                        expected_type,
                        "type field should match tool_name for {expected_type}"
                    );
                    let req_id = cmd["requestId"].as_str().unwrap_or("");

                    let result = serde_json::json!({
                        "requestId": req_id,
                        "status": "success",
                        "response": "ok",
                    });

                    let tx = state2.result_tx.lock().unwrap().take().unwrap();
                    tx.send(result).unwrap();
                });

                let _ = dispatch_to_plugin(
                    Some(Value::from(1)),
                    tool_name,
                    serde_json::json!({}),
                    state,
                )
                .await;
            }
        }
    }

    // -- Tool schema validation ------------------------------------------------

    mod tool_schema_tests {
        use super::*;

        fn get_tools() -> Vec<Value> {
            let resp = handle_tools_list(Some(Value::from(1)));
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            json["result"]["tools"].as_array().unwrap().clone()
        }

        fn find_tool<'a>(tools: &'a [Value], name: &str) -> &'a Value {
            tools.iter().find(|t| t["name"] == name).unwrap()
        }

        #[test]
        fn run_code_requires_command() {
            let tools = get_tools();
            let tool = find_tool(&tools, "run_code");
            let required = tool["inputSchema"]["required"].as_array().unwrap();
            assert!(required.iter().any(|v| v == "command"));
            assert!(tool["inputSchema"]["properties"]["command"]["type"] == "string");
        }

        #[test]
        fn insert_model_requires_query() {
            let tools = get_tools();
            let tool = find_tool(&tools, "insert_model");
            let required = tool["inputSchema"]["required"].as_array().unwrap();
            assert!(required.iter().any(|v| v == "query"));
            assert!(tool["inputSchema"]["properties"]["query"]["type"] == "string");
        }

        #[test]
        fn get_console_output_has_no_required() {
            let tools = get_tools();
            let tool = find_tool(&tools, "get_console_output");
            assert!(tool["inputSchema"]["required"].is_null());
        }

        #[test]
        fn get_studio_mode_has_no_required() {
            let tools = get_tools();
            let tool = find_tool(&tools, "get_studio_mode");
            assert!(tool["inputSchema"]["required"].is_null());
        }

        #[test]
        fn start_stop_play_requires_mode_with_enum() {
            let tools = get_tools();
            let tool = find_tool(&tools, "start_stop_play");
            let required = tool["inputSchema"]["required"].as_array().unwrap();
            assert!(required.iter().any(|v| v == "mode"));
            let mode_enum = tool["inputSchema"]["properties"]["mode"]["enum"]
                .as_array()
                .unwrap();
            let values: Vec<&str> = mode_enum.iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(values, vec!["start_play", "run_server", "stop"]);
        }

        #[test]
        fn run_script_in_play_mode_requires_code_and_mode() {
            let tools = get_tools();
            let tool = find_tool(&tools, "run_script_in_play_mode");
            let required = tool["inputSchema"]["required"].as_array().unwrap();
            let req_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
            assert!(req_strs.contains(&"code"));
            assert!(req_strs.contains(&"mode"));
            assert!(!req_strs.contains(&"timeout"));
            let mode_enum = tool["inputSchema"]["properties"]["mode"]["enum"]
                .as_array()
                .unwrap();
            let values: Vec<&str> = mode_enum.iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(values, vec!["start_play", "run_server"]);
            let timeout = &tool["inputSchema"]["properties"]["timeout"];
            let has_integer = timeout["type"] == "integer"
                || timeout["type"]
                    .as_array()
                    .map_or(false, |arr| arr.iter().any(|v| v == "integer"));
            assert!(has_integer, "timeout should accept integer: {timeout}");
        }
    }
}
