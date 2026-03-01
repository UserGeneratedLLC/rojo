use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, header::CONTENT_TYPE, Method, Request, Response, StatusCode};
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Shared state for MCP sync coordination between the agent-facing `/mcp`
/// endpoint and the plugin-facing `/api/mcp/stream` WebSocket.
pub struct McpSyncState {
    pub sync_in_progress: AtomicBool,
    pub command_tx: tokio::sync::watch::Sender<Option<McpSyncCommand>>,
    pub command_rx: tokio::sync::watch::Receiver<Option<McpSyncCommand>>,
    pub result_tx: Mutex<Option<tokio::sync::oneshot::Sender<McpSyncResult>>>,
    pub plugin_stream_connected: AtomicBool,
    pub plugin_config: Mutex<Option<PluginConfig>>,
}

impl McpSyncState {
    pub fn new() -> Self {
        let (command_tx, command_rx) = tokio::sync::watch::channel(None);
        Self {
            sync_in_progress: AtomicBool::new(false),
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
    mcp_state: Arc<McpSyncState>,
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
        "initialize" => handle_initialize(rpc_request.id),
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

fn handle_tools_list(id: Option<Value>) -> Response<Full<Bytes>> {
    let result = serde_json::json!({
        "tools": [
            {
                "name": "atlas_sync",
                "description": "Sync filesystem changes to Roblox Studio. Supports modes: 'standard' (default, auto-accepts if all git-resolved, else shows UI), 'manual' (always shows UI), 'fastfail' (fails immediately if unresolved changes exist), 'dryrun' (returns what would change without applying). Supports overrides to auto-accept specific changes by instance id with hash/property verification.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode": {
                            "type": "string",
                            "enum": ["standard", "manual", "fastfail", "dryrun"],
                            "description": "Sync mode. 'standard' (default): auto-accept if all changes are git-resolved, otherwise show Studio UI. 'manual': always show Studio UI. 'fastfail': fail immediately if any changes are unresolved. 'dryrun': return what would change without applying."
                        },
                        "overrides": {
                            "type": "array",
                            "description": "Auto-accept directives for specific instance changes, matched by id from a previous sync response.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": {
                                        "type": "string",
                                        "description": "Server Ref (32-char hex) from a previous sync response."
                                    },
                                    "direction": {
                                        "type": "string",
                                        "enum": ["push", "pull"],
                                        "description": "Direction to accept: 'push' (Atlas to Studio) or 'pull' (Studio to Atlas)."
                                    },
                                    "studioHash": {
                                        "type": "string",
                                        "description": "For scripts: SHA1 of git blob of current Studio Source. Override is rejected if hash doesn't match (concurrent edit detected)."
                                    },
                                    "expectedProperties": {
                                        "type": "object",
                                        "description": "Map of property name to expected current Studio value (RbxDom encoded). Override is rejected if any value doesn't match."
                                    }
                                },
                                "required": ["id", "direction"]
                            }
                        }
                    }
                }
            }
        ]
    });
    let resp = JsonRpcResponse::success(id, result);
    json_response(&resp, StatusCode::OK)
}

async fn handle_tools_call(
    id: Option<Value>,
    params: Option<Value>,
    mcp_state: Arc<McpSyncState>,
    active_api_connections: Arc<std::sync::atomic::AtomicUsize>,
) -> Response<Full<Bytes>> {
    let tool_name = params
        .as_ref()
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    if tool_name != "atlas_sync" {
        let resp = JsonRpcResponse::error(id, -32602, format!("Unknown tool: {tool_name}"));
        return json_response(&resp, StatusCode::OK);
    }

    let arguments = params
        .as_ref()
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    handle_atlas_sync(id, arguments, mcp_state, active_api_connections).await
}

async fn handle_atlas_sync(
    id: Option<Value>,
    arguments: Value,
    mcp_state: Arc<McpSyncState>,
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
        .sync_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return tool_response(
            id,
            true,
            "An atlas_sync operation is already in progress. Please wait for it to complete.",
        );
    }

    if !mcp_state.plugin_stream_connected.load(Ordering::Relaxed) {
        mcp_state.sync_in_progress.store(false, Ordering::SeqCst);
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

    if mcp_state.command_tx.send(Some(command)).is_err() {
        mcp_state.sync_in_progress.store(false, Ordering::SeqCst);
        return tool_response(id, true, "Failed to send sync command to the plugin.");
    }

    let result = match result_rx.await {
        Ok(result) => result,
        Err(_) => {
            mcp_state.sync_in_progress.store(false, Ordering::SeqCst);
            return tool_response(
                id,
                true,
                "Plugin disconnected before completing the sync operation.",
            );
        }
    };

    mcp_state.sync_in_progress.store(false, Ordering::SeqCst);

    let _ = mcp_state.command_tx.send(None);

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
    }

    // -- McpSyncState tests ---------------------------------------------------

    mod state_tests {
        use super::*;

        #[test]
        fn new_state_starts_clean() {
            let state = McpSyncState::new();
            assert!(!state.sync_in_progress.load(Ordering::Relaxed));
            assert!(!state.plugin_stream_connected.load(Ordering::Relaxed));
            assert!(state.plugin_config.lock().unwrap().is_none());
            assert!(state.result_tx.lock().unwrap().is_none());
            assert!(state.command_rx.borrow().is_none());
        }

        #[test]
        fn sync_mutex_prevents_double_acquire() {
            let state = McpSyncState::new();
            let first = state.sync_in_progress.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            assert!(first.is_ok());
            let second = state.sync_in_progress.compare_exchange(
                false,
                true,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            assert!(second.is_err());
        }

        #[test]
        fn command_channel_delivers() {
            let state = McpSyncState::new();
            let mut rx = state.command_rx.clone();

            let cmd = McpSyncCommand {
                command_type: "sync".to_string(),
                request_id: "test-1".to_string(),
                mode: "standard".to_string(),
                overrides: vec![],
            };
            state.command_tx.send(Some(cmd)).unwrap();

            assert!(rx.has_changed().unwrap());
            let received = rx.borrow_and_update().clone().unwrap();
            assert_eq!(received.request_id, "test-1");
        }

        #[test]
        fn result_oneshot_delivers() {
            let state = McpSyncState::new();
            let (tx, rx) = tokio::sync::oneshot::channel();
            *state.result_tx.lock().unwrap() = Some(tx);

            let result = McpSyncResult {
                request_id: "r1".to_string(),
                status: "success".to_string(),
                changes: vec![],
                message: None,
            };
            let sender = state.result_tx.lock().unwrap().take().unwrap();
            sender.send(result).unwrap();

            let received = rx.blocking_recv().unwrap();
            assert_eq!(received.status, "success");
        }

        #[test]
        fn plugin_config_cache() {
            let state = McpSyncState::new();
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
        fn tools_list_returns_atlas_sync() {
            let resp = handle_tools_list(Some(Value::from(2)));
            assert_eq!(resp.status(), StatusCode::OK);
            let rt = tokio::runtime::Runtime::new().unwrap();
            let bytes = rt.block_on(async { resp.into_body().collect().await.unwrap().to_bytes() });
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            let tools = json["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), 1);
            assert_eq!(tools[0]["name"], "atlas_sync");
            assert!(tools[0]["inputSchema"].is_object());
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
            let state = Arc::new(McpSyncState::new());
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
            let state = Arc::new(McpSyncState::new());
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
        async fn rejects_when_sync_in_progress() {
            let state = Arc::new(McpSyncState::new());
            state.sync_in_progress.store(true, Ordering::SeqCst);
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
            let state = Arc::new(McpSyncState::new());
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
            assert!(!state.sync_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn sends_command_and_returns_success_result() {
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            // Spawn a task that simulates the plugin responding.
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let result = McpSyncResult {
                    request_id: cmd.request_id,
                    status: "success".to_string(),
                    changes: vec![SyncChange {
                        path: "ServerScriptService/Main.server.luau".to_string(),
                        direction: "push".to_string(),
                        ..Default::default()
                    }],
                    message: None,
                };

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
            assert!(!state.sync_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn returns_rejection_with_presented_changes() {
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let result = McpSyncResult {
                    request_id: cmd.request_id,
                    status: "rejected".to_string(),
                    changes: vec![
                        SyncChange {
                            path: "ReplicatedStorage/Foo.luau".to_string(),
                            direction: "push".to_string(),
                            ..Default::default()
                        },
                        SyncChange {
                            path: "Workspace/Bar.model.json5".to_string(),
                            direction: "pull".to_string(),
                            ..Default::default()
                        },
                    ],
                    message: Some("User rejected the sync changes.".to_string()),
                };

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
            let state = Arc::new(McpSyncState::new());
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
            assert!(!state.sync_in_progress.load(Ordering::Relaxed));
        }

        #[tokio::test]
        async fn empty_status_is_not_an_error() {
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "empty".to_string(),
                    changes: vec![],
                    message: None,
                })
                .unwrap();
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
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "dryrun".to_string(),
                    changes: vec![SyncChange {
                        path: "Workspace/Part".to_string(),
                        direction: "unresolved".to_string(),
                        id: Some("aabb".to_string()),
                        class_name: Some("Part".to_string()),
                        patch_type: Some("Edit".to_string()),
                        ..Default::default()
                    }],
                    message: None,
                })
                .unwrap();
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
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "fastfail_unresolved".to_string(),
                    changes: vec![SyncChange {
                        path: "ServerScriptService/Script".to_string(),
                        direction: "unresolved".to_string(),
                        ..Default::default()
                    }],
                    message: Some("Unresolved changes exist.".to_string()),
                })
                .unwrap();
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
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                assert_eq!(cmd.mode, "fastfail");
                assert_eq!(cmd.overrides.len(), 1);
                assert_eq!(cmd.overrides[0].id, "aabbccdd");
                assert_eq!(cmd.overrides[0].direction, "push");
                assert_eq!(cmd.overrides[0].studio_hash.as_deref(), Some("abc123"));

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "success".to_string(),
                    changes: vec![],
                    message: None,
                })
                .unwrap();
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
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                assert_eq!(cmd.mode, "standard");
                assert!(cmd.overrides.is_empty());

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "empty".to_string(),
                    changes: vec![],
                    message: None,
                })
                .unwrap();
            });

            let resp = handle_atlas_sync(Some(Value::from(12)), empty_args(), state, conns).await;
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(json["result"]["isError"], false);
        }

        #[tokio::test]
        async fn enriched_changes_include_json_block() {
            let state = Arc::new(McpSyncState::new());
            state.plugin_stream_connected.store(true, Ordering::SeqCst);
            let conns = Arc::new(AtomicUsize::new(0));

            let state2 = Arc::clone(&state);
            tokio::spawn(async move {
                let mut rx = state2.command_rx.clone();
                rx.changed().await.unwrap();
                let cmd = rx.borrow().clone().unwrap();

                let tx = state2.result_tx.lock().unwrap().take().unwrap();
                tx.send(McpSyncResult {
                    request_id: cmd.request_id,
                    status: "success".to_string(),
                    changes: vec![SyncChange {
                        path: "Workspace/Part".to_string(),
                        direction: "push".to_string(),
                        id: Some("00aabb".to_string()),
                        class_name: Some("Part".to_string()),
                        patch_type: Some("Edit".to_string()),
                        default_selection: Some("push".to_string()),
                        fs_path: Some("src/workspace/Part.model.json5".to_string()),
                        studio_hash: None,
                        properties: Some(serde_json::json!({
                            "Anchored": {
                                "current": false,
                                "incoming": true
                            }
                        })),
                    }],
                    message: None,
                })
                .unwrap();
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
}
