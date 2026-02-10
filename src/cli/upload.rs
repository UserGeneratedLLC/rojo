use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::Parser;
use memofs::Vfs;
use reqwest::{
    blocking::multipart,
    header::{ACCEPT, CONTENT_TYPE, COOKIE, USER_AGENT},
    StatusCode,
};
use serde_json::Value;

use crate::serve_session::ServeSession;

use super::resolve_path;

const ASSETS_API_BASE: &str = "https://apis.roblox.com/assets/v1";
const MAX_OPERATION_RETRIES: u32 = 10;

/// Builds the project and uploads it to Roblox.
///
/// Supports three upload modes:
///   - Legacy cookie auth (default): uses Data/Upload.ashx
///   - Open Cloud Assets API (--api_key only): for Creator Store assets (models, plugins)
///   - Open Cloud Places API (--api_key + --universe_id): for places
#[derive(Debug, Parser)]
pub struct UploadCommand {
    /// Path to the project to upload. Defaults to the current directory.
    #[clap(default_value = "")]
    pub project: PathBuf,

    /// Authentication cookie to use. If not specified, Rojo will attempt to find one from the system automatically.
    #[clap(long)]
    pub cookie: Option<String>,

    /// API key obtained from create.roblox.com/credentials.
    /// Without --universe_id, uses the Open Cloud Assets API (models, plugins).
    /// With --universe_id, uses the Open Cloud Places API.
    #[clap(long = "api_key")]
    pub api_key: Option<String>,

    /// The Universe ID of the given place. When provided with --api_key,
    /// uses the Open Cloud Places API instead of the Assets API.
    #[clap(long = "universe_id")]
    pub universe_id: Option<u64>,

    /// Asset ID to upload to.
    #[clap(long = "asset_id")]
    pub asset_id: u64,
}

impl UploadCommand {
    pub fn run(self) -> Result<(), anyhow::Error> {
        let project_path = resolve_path(&self.project);

        let vfs = Vfs::new_default();

        let session = ServeSession::new(vfs, project_path)?;

        let tree = session.tree();
        let inner_tree = tree.inner();
        let root = inner_tree.root();

        let encode_ids = match root.class.as_str() {
            "DataModel" => root.children().to_vec(),
            _ => vec![root.referent()],
        };

        let mut buffer = Vec::new();

        log::trace!("Encoding binary model");
        rbx_binary::to_writer(&mut buffer, tree.inner(), &encode_ids)?;

        match (self.cookie, self.api_key, self.universe_id) {
            (cookie, None, universe) => {
                // Legacy cookie auth
                if universe.is_some() {
                    log::warn!(
                        "--universe_id was provided but is ignored when using legacy upload"
                    );
                }

                let cookie = cookie
                    .or_else(rbx_cookie::get_value)
                    .context(
                        "Rojo could not find your Roblox auth cookie. Please log into Roblox Studio or pass one via --cookie.",
                    )?;
                do_upload_legacy(buffer, self.asset_id, &cookie)
            }

            (cookie, Some(api_key), None) => {
                // Open Cloud Assets API (models, plugins, Creator Store assets)
                if cookie.is_some() {
                    log::warn!("--cookie was provided but is ignored when using Open Cloud API");
                }

                do_upload_asset(buffer, self.asset_id, &api_key)
            }

            (cookie, Some(api_key), Some(universe_id)) => {
                // Open Cloud Places API
                if cookie.is_some() {
                    log::warn!("--cookie was provided but is ignored when using Open Cloud API");
                }

                do_upload_place(buffer, universe_id, self.asset_id, &api_key)
            }
        }
    }
}

/// Legacy upload via Data/Upload.ashx with cookie auth.
fn do_upload_legacy(buffer: Vec<u8>, asset_id: u64, cookie: &str) -> anyhow::Result<()> {
    let url = format!(
        "https://data.roblox.com/Data/Upload.ashx?assetid={}",
        asset_id
    );

    let client = reqwest::blocking::Client::new();

    let build_request = move || {
        client
            .post(&url)
            .header(COOKIE, format!(".ROBLOSECURITY={}", cookie))
            .header(USER_AGENT, "Roblox/WinInet")
            .header(CONTENT_TYPE, "application/xml")
            .header(ACCEPT, "application/json")
            .body(buffer.clone())
    };

    log::debug!("Uploading to Roblox (legacy)...");
    let mut response = build_request().send()?;

    // Starting in Feburary, 2021, the upload endpoint performs CSRF challenges.
    // If we receive an HTTP 403 with a X-CSRF-Token reply, we should retry the
    // request, echoing the value of that header.
    if response.status() == StatusCode::FORBIDDEN {
        if let Some(csrf_token) = response.headers().get("X-CSRF-Token") {
            log::debug!("Received CSRF challenge, retrying with token...");
            response = build_request().header("X-CSRF-Token", csrf_token).send()?;
        }
    }

    let status = response.status();
    if !status.is_success() {
        let body = response.text()?;
        bail!(
            "The Roblox API returned HTTP {}: {}",
            status,
            if body.is_empty() {
                "(empty response)".to_owned()
            } else {
                body
            }
        );
    }

    Ok(())
}

/// Upload via the Open Cloud Assets API (PATCH /assets/v1/assets/{assetId}).
/// Works for Creator Store assets: models, plugins, etc.
///
/// Follows the same flow as Roblox's rbxasset tool:
///   1. PATCH the asset with multipart form (request JSON + file content)
///   2. Poll the returned operation until completion
///
/// See https://create.roblox.com/docs/reference/cloud/assets/v1
fn do_upload_asset(buffer: Vec<u8>, asset_id: u64, api_key: &str) -> anyhow::Result<()> {
    let client = reqwest::blocking::Client::new();

    // Step 1: PATCH the asset with new content
    let url = format!("{}/assets/{}", ASSETS_API_BASE, asset_id);

    let request_json = serde_json::json!({
        "assetId": asset_id,
    })
    .to_string();

    let form = multipart::Form::new()
        .part(
            "request",
            multipart::Part::text(request_json)
                .mime_str("application/json")?,
        )
        .part(
            "fileContent",
            multipart::Part::bytes(buffer)
                .file_name("asset.rbxm")
                .mime_str("model/x-rbxm")?,
        );

    log::info!("Uploading to Roblox (Open Cloud Assets API)...");
    let response = client
        .patch(&url)
        .header("x-api-key", api_key)
        .multipart(form)
        .send()?;

    let status = response.status();
    let body: Value = response
        .json()
        .context("Failed to parse upload response as JSON")?;

    if !status.is_success() {
        let message = body["message"].as_str().unwrap_or("unknown error");
        let code = body["code"].as_str().unwrap_or("");
        bail!(
            "The Roblox API returned HTTP {} ({}): {}",
            status,
            code,
            message
        );
    }

    // Step 2: Poll the operation until it completes
    // The PATCH response returns { "path": "operations/{operationId}", "done": bool, ... }
    if let Some(true) = body["done"].as_bool() {
        // Operation completed immediately
        log_asset_result(&body);
        return Ok(());
    }

    let operation_path = body["path"]
        .as_str()
        .context("Upload response missing 'path' field for operation polling")?;

    // Extract operationId from "operations/{operationId}"
    let operation_id = operation_path
        .strip_prefix("operations/")
        .unwrap_or(operation_path);

    log::info!("Upload accepted, waiting for processing (operation: {})...", operation_id);

    let mut retry_delay = std::time::Duration::from_secs(1);

    for attempt in 1..=MAX_OPERATION_RETRIES {
        std::thread::sleep(retry_delay);

        let op_url = format!("{}/operations/{}", ASSETS_API_BASE, operation_id);
        let op_response = client
            .get(&op_url)
            .header("x-api-key", api_key)
            .send()?;

        let op_status = op_response.status();
        let op_body: Value = op_response
            .json()
            .context("Failed to parse operation response as JSON")?;

        if !op_status.is_success() {
            log::warn!(
                "Operation poll attempt {}/{} returned HTTP {}: {}",
                attempt,
                MAX_OPERATION_RETRIES,
                op_status,
                op_body
            );
            retry_delay *= 2;
            continue;
        }

        if let Some(true) = op_body["done"].as_bool() {
            // Check for error in the operation result
            if op_body.get("error").is_some() {
                let err_message = op_body["error"]["message"]
                    .as_str()
                    .unwrap_or("unknown error");
                bail!("Asset processing failed: {}", err_message);
            }

            log_asset_result(&op_body);
            return Ok(());
        }

        log::debug!(
            "Operation not yet complete (attempt {}/{}), retrying in {}s...",
            attempt,
            MAX_OPERATION_RETRIES,
            retry_delay.as_secs()
        );

        retry_delay *= 2;
    }

    bail!(
        "Operation {} did not complete within {} attempts",
        operation_id,
        MAX_OPERATION_RETRIES
    );
}

fn log_asset_result(body: &Value) {
    if let Some(response) = body.get("response") {
        let asset_id = response
            .get("assetId")
            .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")));
        let revision = response.get("revisionId").and_then(|v| v.as_str());

        match (asset_id, revision) {
            (Some(_), Some(rev)) => {
                log::info!("Upload complete! Revision: {}", rev);
            }
            _ => {
                log::info!("Upload complete!");
            }
        }

        if let Some(moderation) = response.get("moderationResult") {
            if let Some(state) = moderation.get("moderationState").and_then(|v| v.as_str()) {
                log::info!("Moderation state: {}", state);
            }
        }
    } else {
        log::info!("Upload complete!");
    }
}

/// Upload via the Open Cloud Places API.
/// See https://create.roblox.com/docs/cloud/guides/usage-place-publishing
fn do_upload_place(
    buffer: Vec<u8>,
    universe_id: u64,
    asset_id: u64,
    api_key: &str,
) -> anyhow::Result<()> {
    let url = format!(
        "https://apis.roblox.com/universes/v1/{}/places/{}/versions?versionType=Published",
        universe_id, asset_id
    );

    let client = reqwest::blocking::Client::new();

    log::debug!("Uploading to Roblox (Open Cloud Places API)...");
    let response = client
        .post(url)
        .header("x-api-key", api_key)
        .header(CONTENT_TYPE, "application/octet-stream")
        .header(ACCEPT, "application/json")
        .body(buffer)
        .send()?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text()?;
        bail!(
            "The Roblox API returned HTTP {}: {}",
            status,
            if body.is_empty() {
                "(empty response)".to_owned()
            } else {
                body
            }
        );
    }

    Ok(())
}
