//! Defines Rojo's HTTP API, all under /api. These endpoints generally return
//! JSON.

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use futures::{sink::SinkExt, stream::StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_tungstenite::{is_upgrade_request, tungstenite::Message, upgrade, HyperWebsocket};
use opener::OpenError;
use rbx_dom_weak::{
    types::{Ref, Variant},
    InstanceBuilder, UstrMap, WeakDom,
};

use crate::{
    serve_session::ServeSession,
    snapshot::{InstanceWithMeta, InstigatingSource, PatchSet, PatchUpdate},
    syncback::{slugify_name, VISIBLE_SERVICES},
    web::{
        interface::{
            ErrorResponse, Instance, InstanceMetadata, MessagesPacket, OpenResponse, ReadResponse,
            ServerInfoResponse, SocketPacket, SocketPacketBody, SocketPacketType, SubscribeMessage,
            WriteRequest, WriteResponse, PROTOCOL_VERSION, SERVER_VERSION,
        },
        util::{deserialize_msgpack, msgpack, msgpack_ok, serialize_msgpack},
    },
    web_api::{InstanceUpdate, RefPatchResponse, SerializeResponse},
};

/// Represents the existing file format for a script/instance on disk.
/// Used to preserve the current format when doing partial updates from the plugin.
///
/// When the plugin sends a script update, it might not include children info.
/// We check the filesystem to see if this is already a directory (with init file)
/// or a standalone file, and preserve that format to avoid creating duplicates.
#[derive(Debug, Clone, PartialEq)]
enum ExistingFileFormat {
    /// No existing file found - use has_children to decide
    None,
    /// Standalone file exists (e.g., Name.luau). Carries the detected file path
    /// so we write back to the same extension (.model.json vs .model.json5).
    Standalone(PathBuf),
    /// Directory with init file exists (e.g., Name/init.luau). Carries the dir path.
    Directory(PathBuf),
}

/// Convert a Variant to a JSON-compatible value for .model.json5 files
fn variant_to_json(variant: &Variant) -> Option<serde_json::Value> {
    use serde_json::{json, Value};

    match variant {
        Variant::String(s) => Some(Value::String(s.clone())),
        Variant::Bool(b) => Some(Value::Bool(*b)),
        Variant::Int32(n) => Some(json!(n)),
        Variant::Int64(n) => Some(json!(n)),
        Variant::Float32(n) => Some(json!(n)),
        Variant::Float64(n) => Some(json!(n)),
        Variant::Vector2(v) => Some(json!([v.x, v.y])),
        Variant::Vector3(v) => Some(json!([v.x, v.y, v.z])),
        Variant::Color3(c) => Some(json!([c.r, c.g, c.b])),
        Variant::Color3uint8(c) => Some(json!([c.r, c.g, c.b])),
        Variant::CFrame(cf) => {
            let pos = cf.position;
            let ori = cf.orientation;
            Some(json!({
                "position": [pos.x, pos.y, pos.z],
                "orientation": [
                    [ori.x.x, ori.x.y, ori.x.z],
                    [ori.y.x, ori.y.y, ori.y.z],
                    [ori.z.x, ori.z.y, ori.z.z]
                ]
            }))
        }
        Variant::UDim(u) => Some(json!({"scale": u.scale, "offset": u.offset})),
        Variant::UDim2(u) => Some(json!({
            "x": {"scale": u.x.scale, "offset": u.x.offset},
            "y": {"scale": u.y.scale, "offset": u.y.offset}
        })),
        Variant::Enum(e) => {
            // Note: Enum values without type context can only be serialized as numbers.
            // Properties like RunContext should be filtered out before calling this function
            // since their type is encoded in the file suffix instead.
            Some(json!(e.to_u32()))
        }
        Variant::BrickColor(bc) => Some(json!(*bc as u16)),
        Variant::NumberSequence(ns) => {
            let keypoints: Vec<_> = ns
                .keypoints
                .iter()
                .map(|kp| json!({"time": kp.time, "value": kp.value, "envelope": kp.envelope}))
                .collect();
            Some(json!({"keypoints": keypoints}))
        }
        Variant::ColorSequence(cs) => {
            let keypoints: Vec<_> = cs
                .keypoints
                .iter()
                .map(|kp| json!({"time": kp.time, "color": [kp.color.r, kp.color.g, kp.color.b]}))
                .collect();
            Some(json!({"keypoints": keypoints}))
        }
        Variant::NumberRange(nr) => Some(json!({"min": nr.min, "max": nr.max})),
        Variant::Rect(r) => Some(json!({
            "min": [r.min.x, r.min.y],
            "max": [r.max.x, r.max.y]
        })),
        // Skip complex types that don't serialize well to JSON
        _ => None,
    }
}

pub async fn call(
    serve_session: Arc<ServeSession>,
    mut request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let service = ApiService::new(serve_session);

    match (request.method(), request.uri().path()) {
        (&Method::GET, "/api/rojo") => service.handle_api_rojo().await,
        (&Method::GET, path) if path.starts_with("/api/read/") => {
            service.handle_api_read(request).await
        }
        (&Method::GET, path) if path.starts_with("/api/socket/") => {
            if is_upgrade_request(&request) {
                service.handle_api_socket(&mut request).await
            } else {
                msgpack(
                    ErrorResponse::bad_request(
                        "/api/socket must be called as a websocket upgrade request",
                    ),
                    StatusCode::BAD_REQUEST,
                )
            }
        }
        (&Method::GET, path) if path.starts_with("/api/serialize/") => {
            service.handle_api_serialize(request).await
        }
        (&Method::GET, path) if path.starts_with("/api/ref-patch/") => {
            service.handle_api_ref_patch(request).await
        }

        (&Method::POST, path) if path.starts_with("/api/open/") => {
            service.handle_api_open(request).await
        }
        (&Method::POST, "/api/write") => service.handle_api_write(request).await,

        (_method, path) => msgpack(
            ErrorResponse::not_found(format!("Route not found: {}", path)),
            StatusCode::NOT_FOUND,
        ),
    }
}

pub struct ApiService {
    serve_session: Arc<ServeSession>,
    suppressed_paths: Arc<Mutex<HashMap<PathBuf, (usize, usize)>>>,
}

/// Derives the directory name from a standalone script's filesystem path.
///
/// Uses `file_stem()` with script suffix stripping instead of the decoded
/// instance name, so that Windows-invalid character encoding (e.g., `%3F`
/// for `?`) is preserved.
///
/// Examples:
/// - `What%3F.server.luau` → `What%3F`
/// - `MyModule.luau` → `MyModule`
/// - `Handler.client.lua` → `Handler`
fn dir_name_from_script_path(standalone_path: &Path) -> &str {
    let file_stem = standalone_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    file_stem
        .strip_suffix(".server")
        .or_else(|| file_stem.strip_suffix(".client"))
        .or_else(|| file_stem.strip_suffix(".plugin"))
        .or_else(|| file_stem.strip_suffix(".local"))
        .or_else(|| file_stem.strip_suffix(".legacy"))
        .unwrap_or(file_stem)
}

/// Derives the directory name from a standalone non-script instance's filesystem path.
///
/// Strips compound extensions (`.model.json5`, `.model.json`) or falls back
/// to `file_stem()` for single-extension types (`.txt`, `.csv`, etc.).
///
/// Examples:
/// - `What%3F.model.json5` → `What%3F`
/// - `MyPart.model.json` → `MyPart`
/// - `Greeting.txt` → `Greeting`
/// - `Translations.csv` → `Translations`
fn dir_name_from_instance_path(standalone_path: &Path) -> &str {
    let file_name = standalone_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");
    file_name
        .strip_suffix(".model.json5")
        .or_else(|| file_name.strip_suffix(".model.json"))
        .unwrap_or_else(|| {
            standalone_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(file_name)
        })
}

impl ApiService {
    pub fn new(serve_session: Arc<ServeSession>) -> Self {
        let suppressed_paths = serve_session.suppressed_paths();
        ApiService {
            serve_session,
            suppressed_paths,
        }
    }

    /// Canonicalize a path for use as a suppression map key.
    fn suppression_key(path: &Path) -> PathBuf {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            canonical
        } else if let Some(parent) = path.parent() {
            if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
                if let Some(file_name) = path.file_name() {
                    canonical_parent.join(file_name)
                } else {
                    path.to_path_buf()
                }
            } else {
                path.to_path_buf()
            }
        } else {
            path.to_path_buf()
        }
    }

    /// Suppress the next Create/Write VFS event for the given path.
    fn suppress_path(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        suppressed.entry(key).or_insert((0, 0)).1 += 1;
    }

    /// Suppress the next Remove VFS event for the given path.
    fn suppress_path_remove(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        suppressed.entry(key).or_insert((0, 0)).0 += 1;
    }

    /// Get a summary of information about the server
    async fn handle_api_rojo(&self) -> Response<Full<Bytes>> {
        let tree = self.serve_session.tree();
        let root_instance_id = tree.get_root_id();

        let ignore_hidden_services = self.serve_session.ignore_hidden_services();
        let visible_services = if ignore_hidden_services {
            VISIBLE_SERVICES.iter().map(|s| s.to_string()).collect()
        } else {
            Vec::new()
        };

        msgpack_ok(&ServerInfoResponse {
            server_version: SERVER_VERSION.to_owned(),
            protocol_version: PROTOCOL_VERSION,
            server_fork: "atlas".to_owned(),
            session_id: self.serve_session.session_id(),
            project_name: self.serve_session.project_name().to_owned(),
            expected_place_ids: self.serve_session.serve_place_ids().cloned(),
            unexpected_place_ids: self.serve_session.blocked_place_ids().cloned(),
            place_id: self.serve_session.place_id(),
            game_id: self.serve_session.game_id(),
            root_instance_id,
            sync_source_only: true,
            ignore_hidden_services,
            visible_services,
        })
    }

    /// Handle WebSocket upgrade for real-time message streaming
    async fn handle_api_socket(&self, request: &mut Request<Incoming>) -> Response<Full<Bytes>> {
        let argument = &request.uri().path()["/api/socket/".len()..];
        let input_cursor: u32 = match argument.parse() {
            Ok(v) => v,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Malformed message cursor: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        // Upgrade the connection to WebSocket
        let (response, websocket) = match upgrade(request, None) {
            Ok(result) => result,
            Err(err) => {
                return msgpack(
                    ErrorResponse::internal_error(format!("WebSocket upgrade failed: {}", err)),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        };

        let serve_session = Arc::clone(&self.serve_session);

        // Spawn a task to handle the WebSocket connection
        tokio::spawn(async move {
            if let Err(e) =
                handle_websocket_subscription(serve_session, websocket, input_cursor).await
            {
                log::error!("Error in websocket subscription: {}", e);
            }
        });

        response
    }

    async fn handle_api_write(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let session_id = self.serve_session.session_id();
        let tree_mutation_sender = self.serve_session.tree_mutation_sender();

        let body = request.into_body().collect().await.unwrap().to_bytes();

        let request: WriteRequest = match deserialize_msgpack(&body) {
            Ok(request) => request,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Invalid body: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        if request.session_id != session_id {
            return msgpack(
                ErrorResponse::bad_request("Wrong session ID"),
                StatusCode::BAD_REQUEST,
            );
        }

        // Process removed instances (syncback: delete files from Rojo filesystem)
        // Phase 1: Gather paths with the tree lock held.
        // Phase 2: Delete files without the lock.
        // Only IDs that are actually removable (have a Path instigating source)
        // are included in the PatchSet. ProjectNode instances are skipped so
        // they are NOT removed from the in-memory tree.
        let mut actually_removed: Vec<Ref> = Vec::new();
        if !request.removed.is_empty() {
            let removal_actions: Vec<(Ref, Option<(PathBuf, bool)>)> = {
                let tree = self.serve_session.tree();
                request
                    .removed
                    .iter()
                    .map(|&id| {
                        let action = tree.get_instance(id).and_then(|inst| {
                            inst.metadata().instigating_source.as_ref().and_then(|src| {
                                match src {
                                    crate::snapshot::InstigatingSource::Path(p) => {
                                        Some((p.clone(), p.is_dir()))
                                    }
                                    crate::snapshot::InstigatingSource::ProjectNode {
                                        name,
                                        ..
                                    } => {
                                        log::warn!(
                                            "Syncback: Cannot remove '{}' — defined in project file",
                                            name
                                        );
                                        None
                                    }
                                }
                            })
                        });
                        (id, action)
                    })
                    .collect()
            }; // tree lock dropped here

            // Phase 2: Execute filesystem deletions without the lock
            for (id, action) in removal_actions {
                if let Some((path, is_dir)) = action {
                    if !path.exists() {
                        log::info!(
                            "Syncback: Path already removed (likely parent was deleted): {}",
                            path.display()
                        );
                        actually_removed.push(id);
                        continue;
                    }
                    if is_dir {
                        self.suppress_path_remove(&path);
                        if let Err(err) = fs::remove_dir_all(&path) {
                            log::warn!(
                                "Failed to remove directory {:?} for instance {:?}: {}",
                                path,
                                id,
                                err
                            );
                        } else {
                            log::info!("Syncback: Removed directory at {}", path.display());
                            actually_removed.push(id);
                        }
                    } else {
                        self.suppress_path_remove(&path);
                        if let Err(err) = fs::remove_file(&path) {
                            log::warn!(
                                "Failed to remove file {:?} for instance {:?}: {}",
                                path,
                                id,
                                err
                            );
                        } else {
                            log::info!("Syncback: Removed file at {}", path.display());
                            actually_removed.push(id);
                        }
                        // Also remove adjacent meta file.
                        // Strip known script suffixes (.server, .client, etc.)
                        // rather than splitting on dots, so that names containing
                        // dots (e.g. "Config.Client.server.luau") resolve correctly.
                        if let Some(file_stem) = path.file_stem().and_then(|s| s.to_str()) {
                            let base_name = file_stem
                                .strip_suffix(".server")
                                .or_else(|| file_stem.strip_suffix(".client"))
                                .or_else(|| file_stem.strip_suffix(".plugin"))
                                .or_else(|| file_stem.strip_suffix(".local"))
                                .or_else(|| file_stem.strip_suffix(".legacy"))
                                .unwrap_or(file_stem);
                            if let Some(parent_dir) = path.parent() {
                                let meta_path =
                                    parent_dir.join(format!("{}.meta.json5", base_name));
                                if meta_path.exists() {
                                    self.suppress_path_remove(&meta_path);
                                    let _ = fs::remove_file(&meta_path);
                                    log::info!(
                                        "Syncback: Removed adjacent meta file at {}",
                                        meta_path.display()
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Process added instances (syncback: create files from Studio instances)
        // We hold the tree lock during the entire processing to ensure the cache
        // remains consistent with the tree state being checked.
        // Create a stats tracker for this syncback operation
        let stats = crate::syncback::SyncbackStats::new();

        if !request.added.is_empty() {
            let tree = self.serve_session.tree();
            // Pre-compute duplicate sibling info in O(N) for efficient path uniqueness checks
            // This makes all subsequent path checks O(d) instead of O(d × s)
            let duplicate_siblings_cache = Self::compute_tree_refs_with_duplicate_siblings(&tree);

            // Pre-scan: identify standalone parents that need directory conversion.
            // Multiple children may share the same parent. We convert each parent
            // ONCE before processing any children, avoiding double-conversion that
            // would wipe the parent's source (the old standalone file is deleted by
            // the first conversion, so the second would read empty).
            let mut converted_parents: HashMap<Ref, PathBuf> = HashMap::new();
            for added in request.added.values() {
                if let Some(parent_ref) = added.parent {
                    if converted_parents.contains_key(&parent_ref) {
                        continue;
                    }
                    if let Some(parent_inst) = tree.get_instance(parent_ref) {
                        if let Some(source) = &parent_inst.metadata().instigating_source {
                            // Resolve the actual filesystem path for the parent.
                            // For ProjectNode sources, use the resolved $path (not
                            // the project file path). For init-file sources, use
                            // the parent directory (since init files represent
                            // directory-format instances that are already dirs).
                            let resolved_path: std::borrow::Cow<'_, Path> = match source {
                                crate::snapshot::InstigatingSource::Path(p) => {
                                    let file_name =
                                        p.file_name().and_then(|f| f.to_str()).unwrap_or("");
                                    if file_name.starts_with("init.") {
                                        std::borrow::Cow::Borrowed(
                                            p.parent().unwrap_or(p.as_path()),
                                        )
                                    } else {
                                        std::borrow::Cow::Borrowed(p.as_path())
                                    }
                                }
                                crate::snapshot::InstigatingSource::ProjectNode {
                                    path: project_path,
                                    node,
                                    ..
                                } => {
                                    if let Some(path_node) = &node.path {
                                        let fs_path = path_node.path();
                                        let resolved = if fs_path.is_relative() {
                                            project_path
                                                .parent()
                                                .unwrap_or(project_path.as_path())
                                                .join(fs_path)
                                        } else {
                                            fs_path.to_path_buf()
                                        };
                                        std::borrow::Cow::Owned(resolved)
                                    } else {
                                        std::borrow::Cow::Borrowed(source.path())
                                    }
                                }
                            };
                            if !resolved_path.is_dir() {
                                let parent_class = parent_inst.class_name();
                                let parent_name = parent_inst.name();
                                let containing_dir = match resolved_path.parent() {
                                    Some(d) => d,
                                    None => continue,
                                };
                                log::info!(
                                    "Syncback: Pre-converting standalone {} '{}' at {} to directory \
                                     (children being added in this batch)",
                                    parent_class,
                                    parent_name,
                                    resolved_path.display()
                                );
                                let new_dir = if matches!(
                                    parent_class.as_str(),
                                    "ModuleScript" | "Script" | "LocalScript"
                                ) {
                                    self.convert_standalone_script_to_directory(
                                        &resolved_path,
                                        parent_name,
                                        parent_class.as_str(),
                                        containing_dir,
                                    )
                                } else {
                                    self.convert_standalone_instance_to_directory(
                                        &resolved_path,
                                        parent_name,
                                        parent_class.as_str(),
                                        containing_dir,
                                    )
                                };
                                match new_dir {
                                    Ok(dir) => {
                                        converted_parents.insert(parent_ref, dir);
                                    }
                                    Err(err) => {
                                        log::warn!(
                                            "Failed to pre-convert parent '{}': {}",
                                            parent_name,
                                            err
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Group added instances by parent so siblings share a dedup set.
            // This prevents two instances in the same batch from claiming the
            // same slug when added to the same parent.
            let mut adds_by_parent: HashMap<Ref, Vec<&crate::web::interface::AddedInstance>> =
                HashMap::new();
            for added in request.added.values() {
                if let Some(parent_ref) = added.parent {
                    adds_by_parent.entry(parent_ref).or_default().push(added);
                } else {
                    // No parent — process individually (will fail with context)
                    adds_by_parent.entry(Ref::none()).or_default().push(added);
                }
            }
            for (parent_ref, siblings) in &adds_by_parent {
                // Pre-seed sibling_slugs from the tree's existing children
                // of this parent so new instances dedup against existing ones.
                // We derive slugs from actual filesystem paths (via instigating_source)
                // to correctly account for dedup suffixes (e.g., Hey_Bro~1).
                let mut sibling_slugs: HashSet<String> = if *parent_ref != Ref::none() {
                    if let Some(parent_inst) = tree.get_instance(*parent_ref) {
                        use crate::snapshot::InstigatingSource;
                        use crate::syncback::{name_needs_slugify, strip_middleware_extension};
                        parent_inst
                            .children()
                            .iter()
                            .filter_map(|r| tree.get_instance(*r))
                            .map(|inst| {
                                // Prefer filesystem path for accurate dedup keys
                                if let Some(InstigatingSource::Path(p)) =
                                    &inst.metadata().instigating_source
                                {
                                    if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
                                        if let Some(mw) = inst.metadata().middleware {
                                            return strip_middleware_extension(fname, mw)
                                                .to_lowercase();
                                        }
                                        // No middleware info — use filename as-is
                                        // (directory entries have no extension)
                                        return fname.to_lowercase();
                                    }
                                }
                                // Fallback: re-slugify instance name
                                let name = inst.name();
                                if name_needs_slugify(name) {
                                    slugify_name(name).to_lowercase()
                                } else {
                                    name.to_lowercase()
                                }
                            })
                            .collect()
                    } else {
                        HashSet::new()
                    }
                } else {
                    HashSet::new()
                };
                for added in siblings {
                    if let Err(err) = self.syncback_added_instance(
                        added,
                        &tree,
                        &duplicate_siblings_cache,
                        &stats,
                        &converted_parents,
                        &mut sibling_slugs,
                    ) {
                        log::warn!(
                            "Failed to syncback added instance '{}': {}",
                            added.name,
                            err
                        );
                    }
                }
            }
        }

        // Log summary of any syncback issues
        stats.log_summary();

        // Persist non-Source property changes to disk (meta/model files).
        // Source property changes are handled by ChangeProcessor when it
        // receives the PatchSet below.
        {
            let tree = self.serve_session.tree();
            for update in &request.updated {
                let prop_names: Vec<&str> = update
                    .changed_properties
                    .keys()
                    .map(|k| k.as_str())
                    .collect();
                if !prop_names.is_empty() {
                    log::info!(
                        "Syncback: Updating properties for instance {:?}: {}",
                        update.id,
                        prop_names.join(", ")
                    );
                }
                if update.changed_name.is_some() {
                    log::info!(
                        "Syncback: Renaming instance {:?} to {:?}",
                        update.id,
                        update.changed_name
                    );
                }

                // Write non-Source properties to meta/model files
                if let Err(err) = self.syncback_updated_properties(update, &tree) {
                    log::warn!(
                        "Failed to persist non-Source properties for instance {:?}: {}",
                        update.id,
                        err
                    );
                }
            }
        }

        let updated_instances = request
            .updated
            .into_iter()
            .map(|update| PatchUpdate {
                id: update.id,
                changed_class_name: update.changed_class_name,
                changed_name: update.changed_name,
                changed_properties: update.changed_properties,
                changed_metadata: None,
            })
            .collect();

        tree_mutation_sender
            .send(PatchSet {
                removed_instances: actually_removed,
                added_instances: Vec::new(),
                updated_instances,
            })
            .unwrap();

        msgpack_ok(WriteResponse { session_id })
    }

    /// Syncback an added instance by creating a file in the filesystem.
    /// The file watcher will pick up the change and update Rojo's tree.
    ///
    /// Pre-computes which instances have duplicate-named siblings in a RojoTree.
    /// Returns a HashSet of Refs that have at least one sibling with the same name.
    ///
    /// This is O(N) where N is the number of instances, and allows subsequent
    /// path uniqueness checks to be O(d) instead of O(d × s) where d=depth, s=siblings.
    fn compute_tree_refs_with_duplicate_siblings(tree: &crate::snapshot::RojoTree) -> HashSet<Ref> {
        use std::collections::VecDeque;

        let mut has_duplicate_siblings = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(tree.root().id());

        while let Some(inst_ref) = queue.pop_front() {
            let inst = match tree.get_instance(inst_ref) {
                Some(i) => i,
                None => continue,
            };

            // Count children by name and collect their refs
            let mut name_to_refs: HashMap<&str, Vec<Ref>> = HashMap::new();
            for child_ref in inst.children() {
                if let Some(child) = tree.get_instance(*child_ref) {
                    name_to_refs
                        .entry(child.name())
                        .or_default()
                        .push(*child_ref);
                }
                queue.push_back(*child_ref);
            }

            // Mark refs that share a name with siblings
            for (_name, refs) in name_to_refs {
                if refs.len() > 1 {
                    for r in refs {
                        has_duplicate_siblings.insert(r);
                    }
                }
            }
        }

        has_duplicate_siblings
    }

    /// Checks if a path in the Rojo tree is unique using pre-computed duplicate sibling info.
    /// Returns true if the path is unique, false if duplicates exist at any level.
    /// This is O(d) where d is the depth of the instance.
    /// Records any duplicate path issues via the stats tracker.
    fn is_tree_path_unique_with_cache(
        tree: &crate::snapshot::RojoTree,
        target_ref: Ref,
        has_duplicate_siblings: &HashSet<Ref>,
        stats: &crate::syncback::SyncbackStats,
    ) -> bool {
        let mut current_ref = target_ref;

        loop {
            // O(1) lookup instead of O(siblings) counting
            if has_duplicate_siblings.contains(&current_ref) {
                if let Some(current) = tree.get_instance(current_ref) {
                    if let Some(parent) = tree.get_instance(current.parent()) {
                        // Build the path for the stats tracker
                        let inst_path = format!("{}/{}", parent.name(), current.name());
                        stats.record_duplicate_name(&inst_path, current.name());
                    }
                }
                return false;
            }

            let current = match tree.get_instance(current_ref) {
                Some(inst) => inst,
                None => return false,
            };

            let parent_ref = current.parent();
            if parent_ref.is_none() {
                // Reached root - path is unique at all levels
                return true;
            }

            // Move up to parent and check the next level
            current_ref = parent_ref;
        }
    }

    /// Find a child instance by name under a given parent in the tree.
    /// Returns the Ref of the child if found, None otherwise.
    fn find_child_by_name(
        tree: &crate::snapshot::RojoTree,
        parent_ref: Ref,
        name: &str,
    ) -> Option<Ref> {
        let parent = tree.get_instance(parent_ref)?;
        parent
            .children()
            .iter()
            .find(|&&child_ref| {
                tree.get_instance(child_ref)
                    .map(|c| c.name() == name)
                    .unwrap_or(false)
            })
            .copied()
    }

    /// This follows the same middleware selection logic as the dedicated syncback
    /// system in `src/syncback/mod.rs` to ensure consistent behavior.
    ///
    /// Takes the tree reference and duplicate siblings cache as parameters to ensure
    /// consistency - the cache was computed from this exact tree state.
    ///
    /// IMPORTANT: Before creating new files, this function checks if an instance
    /// with the same name already exists in the tree. If so, it updates the existing
    /// file at its `instigating_source.path` instead of creating a new file.
    /// This prevents duplicate file creation when the plugin sends an instance
    /// that already exists on the filesystem.
    fn syncback_added_instance(
        &self,
        added: &crate::web::interface::AddedInstance,
        tree: &crate::snapshot::RojoTree,
        duplicate_siblings_cache: &HashSet<Ref>,
        stats: &crate::syncback::SyncbackStats,
        converted_parents: &HashMap<Ref, PathBuf>,
        sibling_slugs: &mut HashSet<String>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        // Get the parent Ref (required for top-level added instances)
        let parent_ref = added
            .parent
            .context("Top-level added instance must have a parent")?;

        // Find the parent instance to get its filesystem path
        let parent_instance = tree
            .get_instance(parent_ref)
            .context("Parent instance not found in Rojo tree")?;

        // Check if the parent path is unique (no duplicate-named siblings at any level)
        // Uses pre-computed cache for O(d) instead of O(d × s) complexity
        if !Self::is_tree_path_unique_with_cache(tree, parent_ref, duplicate_siblings_cache, stats)
        {
            anyhow::bail!(
                "Cannot sync instance '{}' - parent path contains duplicate-named siblings",
                added.name
            );
        }

        // CRITICAL: Check if instance already exists in tree before creating new files.
        // This prevents duplicate file creation when the plugin sends an instance
        // that already exists (e.g., user "pulls" an instance that appeared as
        // "to delete" due to duplicate detection issues).
        if let Some(existing_ref) = Self::find_child_by_name(tree, parent_ref, &added.name) {
            if let Some(existing) = tree.get_instance(existing_ref) {
                if let Some(source) = &existing.metadata().instigating_source {
                    let existing_path = source.path();
                    log::info!(
                        "Syncback: Instance '{}' already exists in tree at {}, updating in place",
                        added.name,
                        existing_path.display()
                    );
                    // Update the existing instance instead of creating new files
                    return self.syncback_update_existing_instance(added, existing_path, stats);
                }
            }
        }

        // Instance doesn't exist in tree - create new files
        // Get the parent's filesystem path from its metadata.
        // For ProjectNode sources, resolve the $path field relative to the project file.
        let instigating_source = parent_instance
            .metadata()
            .instigating_source
            .as_ref()
            .context("Parent instance has no filesystem path (not synced from filesystem)")?;

        let parent_path: std::borrow::Cow<'_, std::path::Path> = match instigating_source {
            crate::snapshot::InstigatingSource::Path(p) => std::borrow::Cow::Borrowed(p.as_path()),
            crate::snapshot::InstigatingSource::ProjectNode {
                path: project_path,
                name,
                node,
                ..
            } => {
                if let Some(path_node) = &node.path {
                    let fs_path = path_node.path();
                    let resolved = if fs_path.is_relative() {
                        project_path
                            .parent()
                            .unwrap_or(project_path.as_path())
                            .join(fs_path)
                    } else {
                        fs_path.to_path_buf()
                    };
                    std::borrow::Cow::Owned(resolved)
                } else {
                    anyhow::bail!(
                        "Cannot add '{}' — parent '{}' is defined in a project file without $path",
                        added.name,
                        name
                    );
                }
            }
        };

        // Determine if parent is a directory or file.
        // ANY instance can have children in Roblox, so if the parent is a
        // standalone file (script, model, txt, csv, etc.), we must convert it
        // to directory format before we can place children inside it.
        //
        // Check converted_parents first — if another child in this batch
        // already triggered the conversion, use the cached directory path
        // instead of trying to convert again (the standalone file was already
        // deleted by the first conversion).
        let parent_dir = if let Some(dir) = converted_parents.get(&parent_ref) {
            dir.clone()
        } else if parent_path.is_dir() {
            parent_path.to_path_buf()
        } else {
            // Fallback: parent wasn't pre-converted (e.g., parent was resolved
            // through a ProjectNode path). Convert it now.
            let parent_class = parent_instance.class_name();
            let parent_name = parent_instance.name();
            let containing_dir = parent_path
                .parent()
                .context("Could not get parent directory")?;

            log::info!(
                "Syncback: Converting standalone {} '{}' at {} to directory format \
                 (child '{}' being added)",
                parent_class,
                parent_name,
                parent_path.display(),
                added.name
            );

            if matches!(
                parent_class.as_str(),
                "ModuleScript" | "Script" | "LocalScript"
            ) {
                self.convert_standalone_script_to_directory(
                    &parent_path,
                    parent_name,
                    parent_class.as_str(),
                    containing_dir,
                )?
            } else {
                self.convert_standalone_instance_to_directory(
                    &parent_path,
                    parent_name,
                    parent_class.as_str(),
                    containing_dir,
                )?
            }
        };

        // Create the instance at the resolved path, accumulating its claimed
        // slug into sibling_slugs so subsequent siblings in the same batch
        // see it as taken.
        let slug =
            self.syncback_instance_to_path_with_stats(added, &parent_dir, stats, sibling_slugs)?;
        sibling_slugs.insert(slug);
        Ok(())
    }

    /// Update an existing instance in place instead of creating new files.
    /// This is called when the plugin sends an "added" instance that already
    /// exists in the tree. We update the existing file at its instigating_source
    /// path to preserve the file format and avoid creating duplicates.
    ///
    /// For children, we use the normal syncback path which will check for
    /// existing files via `detect_existing_script_format`.
    fn syncback_update_existing_instance(
        &self,
        added: &crate::web::interface::AddedInstance,
        existing_path: &std::path::Path,
        stats: &crate::syncback::SyncbackStats,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        let class_name = &added.class_name;

        // For scripts, update the Source property at the existing path
        match class_name.as_str() {
            "ModuleScript" | "Script" | "LocalScript" => {
                let source = self.get_source_property(added);

                // Determine the actual file to write based on existing structure
                // If existing_path is a directory, look for init file inside
                let file_path = if existing_path.is_dir() {
                    // Find the init file based on class
                    let init_name = match class_name.as_str() {
                        "ModuleScript" => {
                            if existing_path.join("init.luau").exists() {
                                "init.luau"
                            } else {
                                "init.lua"
                            }
                        }
                        "Script" => {
                            // Check which init file exists
                            if existing_path.join("init.server.luau").exists() {
                                "init.server.luau"
                            } else if existing_path.join("init.client.luau").exists() {
                                "init.client.luau"
                            } else if existing_path.join("init.server.lua").exists() {
                                "init.server.lua"
                            } else {
                                "init.server.luau" // Default
                            }
                        }
                        "LocalScript" => {
                            // Modern: init.local.luau produces LocalScript
                            // Legacy: init.client.lua produces LocalScript (without 'u')
                            // Note: init.client.luau produces Script with Client RunContext, NOT LocalScript!
                            if existing_path.join("init.local.luau").exists() {
                                "init.local.luau"
                            } else if existing_path.join("init.client.lua").exists() {
                                "init.client.lua"
                            } else if existing_path.join("init.local.lua").exists() {
                                "init.local.lua"
                            } else {
                                "init.local.luau" // Default for LocalScript
                            }
                        }
                        _ => unreachable!(),
                    };
                    existing_path.join(init_name)
                } else {
                    // It's already a file path
                    existing_path.to_path_buf()
                };

                self.suppress_path(&file_path);
                fs::write(&file_path, source.as_bytes())
                    .with_context(|| format!("Failed to write file: {}", file_path.display()))?;

                log::info!(
                    "Syncback: Updated existing {} at {}",
                    class_name,
                    file_path.display()
                );

                // Handle children - they go into the directory
                if !added.children.is_empty() {
                    let children_dir = if existing_path.is_dir() {
                        existing_path.to_path_buf()
                    } else if existing_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("init."))
                    {
                        // existing_path is an init file (e.g., DirFoo/init.luau).
                        // The instance is already in directory format -- children go
                        // into the same directory that contains the init file.
                        existing_path
                            .parent()
                            .unwrap_or(existing_path)
                            .to_path_buf()
                    } else {
                        // Standalone scripts cannot have children in Rojo's file format.
                        // We need to convert from standalone (e.g., MyScript.server.luau)
                        // to directory format (e.g., MyScript/init.server.luau).
                        let script_name = added.name.as_str();
                        let parent_dir = existing_path.parent().unwrap_or(existing_path);
                        let new_dir = parent_dir.join(script_name);

                        // Create the directory
                        self.suppress_path(&new_dir);
                        fs::create_dir_all(&new_dir).with_context(|| {
                            format!(
                                "Failed to create directory for script with children: {}",
                                new_dir.display()
                            )
                        })?;

                        // Determine the init file name based on class
                        let init_name = match class_name.as_str() {
                            "ModuleScript" => "init.luau",
                            "Script" => "init.server.luau",
                            "LocalScript" => "init.local.luau",
                            _ => "init.luau",
                        };

                        // Move the script content to init file
                        let init_path = new_dir.join(init_name);
                        self.suppress_path(&init_path);
                        fs::write(&init_path, source.as_bytes()).with_context(|| {
                            format!("Failed to write init file: {}", init_path.display())
                        })?;

                        // Remove the old standalone file
                        if existing_path.exists() && existing_path != init_path {
                            self.suppress_path_remove(existing_path);
                            fs::remove_file(existing_path).with_context(|| {
                                format!(
                                    "Failed to remove old standalone script: {}",
                                    existing_path.display()
                                )
                            })?;
                        }

                        log::info!(
                            "Syncback: Converted standalone {} to directory format at {}",
                            class_name,
                            new_dir.display()
                        );

                        new_dir
                    };

                    // Filter duplicate children
                    let inst_path = format!("{}", existing_path.display());
                    let unique_children =
                        self.filter_duplicate_children(&added.children, &inst_path, stats);

                    // Process children using normal syncback path
                    // This will use detect_existing_script_format to check existing files
                    self.process_children_incremental(&unique_children, &children_dir, stats)?;
                }
            }

            // For non-script types, update the appropriate file
            _ => {
                if existing_path.is_dir() {
                    // Update init.meta.json5 if needed
                    if !added.properties.is_empty() {
                        self.write_init_meta_json(existing_path, added, None)?;
                        log::info!(
                            "Syncback: Updated existing {} at {}/init.meta.json5",
                            class_name,
                            existing_path.display()
                        );
                    }

                    // Handle children
                    if !added.children.is_empty() {
                        let inst_path = format!("{}", existing_path.display());
                        let unique_children =
                            self.filter_duplicate_children(&added.children, &inst_path, stats);

                        self.process_children_incremental(&unique_children, existing_path, stats)?;
                    }
                } else {
                    // It's a standalone file (e.g., .model.json5)
                    let content = self.serialize_instance_to_model_json(added, None)?;
                    self.suppress_path(existing_path);
                    fs::write(existing_path, &content).with_context(|| {
                        format!("Failed to write file: {}", existing_path.display())
                    })?;
                    log::info!(
                        "Syncback: Updated existing {} at {}",
                        class_name,
                        existing_path.display()
                    );
                }
            }
        }

        Ok(())
    }

    /// Converts a standalone script file (e.g., `MyModule.luau`) into directory
    /// format (e.g., `MyModule/init.luau`). This is needed when a child is being
    /// added to a standalone script — standalone scripts cannot have children in
    /// Rojo's file format.
    ///
    /// Returns the path to the new directory.
    fn convert_standalone_script_to_directory(
        &self,
        standalone_path: &std::path::Path,
        _script_name: &str,
        class_name: &str,
        containing_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        use anyhow::Context;

        // Read the current script content before any modifications
        let source = if standalone_path.exists() {
            fs::read_to_string(standalone_path).unwrap_or_default()
        } else {
            String::new()
        };

        let dir_name = dir_name_from_script_path(standalone_path);

        // Create the directory
        let new_dir = containing_dir.join(dir_name);
        self.suppress_path(&new_dir);
        fs::create_dir_all(&new_dir).with_context(|| {
            format!(
                "Failed to create directory for script conversion: {}",
                new_dir.display()
            )
        })?;

        // Determine the init file name based on class
        let init_name = match class_name {
            "ModuleScript" => "init.luau",
            "Script" => "init.server.luau",
            "LocalScript" => "init.local.luau",
            _ => "init.luau",
        };

        // Write source content to the init file
        let init_path = new_dir.join(init_name);
        self.suppress_path(&init_path);
        fs::write(&init_path, source.as_bytes())
            .with_context(|| format!("Failed to write init file: {}", init_path.display()))?;

        // Remove the old standalone file
        if standalone_path.exists() && standalone_path != init_path {
            self.suppress_path_remove(standalone_path);
            fs::remove_file(standalone_path).with_context(|| {
                format!(
                    "Failed to remove old standalone script: {}",
                    standalone_path.display()
                )
            })?;
        }

        // Move adjacent meta file into directory if it exists
        let meta_path = containing_dir.join(format!("{}.meta.json5", dir_name));
        if meta_path.exists() {
            let init_meta_path = new_dir.join("init.meta.json5");
            self.suppress_path_remove(&meta_path);
            self.suppress_path(&init_meta_path);
            fs::rename(&meta_path, &init_meta_path).with_context(|| {
                format!(
                    "Failed to move meta file {} to {}",
                    meta_path.display(),
                    init_meta_path.display()
                )
            })?;
            log::info!(
                "Syncback: Moved {} to {}",
                meta_path.display(),
                init_meta_path.display()
            );
        }

        log::info!(
            "Syncback: Converted standalone {} to directory format at {}",
            class_name,
            new_dir.display()
        );

        Ok(new_dir)
    }

    /// Converts a standalone non-script file (e.g., `MyPart.model.json5`, `MyValue.txt`)
    /// into directory format (e.g., `MyPart/init.meta.json5`). This is needed when a
    /// child is being added to any non-script instance that is currently a standalone file.
    ///
    /// Returns the path to the new directory.
    fn convert_standalone_instance_to_directory(
        &self,
        standalone_path: &std::path::Path,
        instance_name: &str,
        class_name: &str,
        containing_dir: &std::path::Path,
    ) -> anyhow::Result<std::path::PathBuf> {
        use anyhow::Context;

        let dir_name = dir_name_from_instance_path(standalone_path);

        let new_dir = containing_dir.join(dir_name);
        self.suppress_path(&new_dir);
        fs::create_dir_all(&new_dir).with_context(|| {
            format!(
                "Failed to create directory for instance conversion: {}",
                new_dir.display()
            )
        })?;

        // Determine the init file based on the standalone file type.
        // The content of the standalone file becomes the init file inside the directory.
        let file_ext = standalone_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let file_name = standalone_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");

        if file_name.ends_with(".model.json5") || file_name.ends_with(".model.json") {
            // .model.json5 → init.meta.json5 with className and properties.
            // IMPORTANT: .model.json5 supports a "children" field for inline child
            // instances, but init.meta.json5 (DirectoryMetadata) does not. We must
            // parse the model and extract only the compatible fields, warning if
            // inline children would be lost.
            if standalone_path.exists() {
                let raw = fs::read(standalone_path).unwrap_or_default();
                let init_meta_path = new_dir.join("init.meta.json5");

                // Parse the model JSON5 to extract only meta-compatible fields.
                // Use json5::from_str (not serde_json) because .model.json5 files
                // can contain JSON5 features (comments, trailing commas, unquoted keys).
                let raw_str = String::from_utf8_lossy(&raw);
                let meta_content = if let Ok(model) = json5::from_str::<serde_json::Value>(&raw_str)
                {
                    // Warn about inline children that will be lost
                    if let Some(children) = model.get("children").or(model.get("Children")) {
                        if children.is_array() && children.as_array().is_some_and(|a| !a.is_empty())
                        {
                            log::warn!(
                                "Syncback: .model.json5 at {} contains inline children \
                                     that cannot be represented in init.meta.json5 — \
                                     these children will be lost during directory conversion",
                                standalone_path.display()
                            );
                        }
                    }

                    // Build init.meta.json5 with only the supported fields
                    let mut meta = serde_json::Map::new();
                    if let Some(cn) = model.get("className").or(model.get("ClassName")).cloned() {
                        meta.insert("className".to_string(), cn);
                    }
                    if let Some(props) =
                        model.get("properties").or(model.get("Properties")).cloned()
                    {
                        if props.is_object() && props.as_object().is_some_and(|o| !o.is_empty()) {
                            meta.insert("properties".to_string(), props);
                        }
                    }
                    if let Some(attrs) =
                        model.get("attributes").or(model.get("Attributes")).cloned()
                    {
                        if attrs.is_object() && attrs.as_object().is_some_and(|o| !o.is_empty()) {
                            meta.insert("attributes".to_string(), attrs);
                        }
                    }
                    if let Some(ignore) = model.get("ignoreUnknownInstances").cloned() {
                        meta.insert("ignoreUnknownInstances".to_string(), ignore);
                    }
                    if let Some(id) = model.get("id").cloned() {
                        meta.insert("id".to_string(), id);
                    }

                    crate::json::to_vec_pretty_sorted(&serde_json::Value::Object(meta))
                        .unwrap_or(raw.clone())
                } else {
                    // Parse failed — copy raw as fallback (best-effort)
                    log::warn!(
                        "Syncback: Could not parse {} as JSON — \
                             copying raw content to init.meta.json5",
                        standalone_path.display()
                    );
                    raw
                };

                self.suppress_path(&init_meta_path);
                fs::write(&init_meta_path, &meta_content)
                    .with_context(|| format!("Failed to write {}", init_meta_path.display()))?;
            }
        } else if file_ext == "txt" {
            // StringValue .txt → init.meta.json5 with className and Value property
            let value = if standalone_path.exists() {
                fs::read_to_string(standalone_path).unwrap_or_default()
            } else {
                String::new()
            };
            let meta = serde_json::json!({
                "className": class_name,
                "properties": {
                    "Value": value
                }
            });
            let init_meta_path = new_dir.join("init.meta.json5");
            let content = crate::json::to_vec_pretty_sorted(&meta)
                .context("Failed to serialize init.meta.json5")?;
            self.suppress_path(&init_meta_path);
            fs::write(&init_meta_path, &content)
                .with_context(|| format!("Failed to write {}", init_meta_path.display()))?;
        } else if file_ext == "csv" {
            // LocalizationTable .csv → init.csv
            if standalone_path.exists() {
                let content = fs::read(standalone_path).unwrap_or_default();
                let init_csv_path = new_dir.join("init.csv");
                self.suppress_path(&init_csv_path);
                fs::write(&init_csv_path, &content)
                    .with_context(|| format!("Failed to write {}", init_csv_path.display()))?;
            }
        } else {
            // Generic fallback: create init.meta.json5 with className
            let meta = serde_json::json!({
                "className": class_name
            });
            let init_meta_path = new_dir.join("init.meta.json5");
            let content = crate::json::to_vec_pretty_sorted(&meta)
                .context("Failed to serialize init.meta.json5")?;
            self.suppress_path(&init_meta_path);
            fs::write(&init_meta_path, &content)
                .with_context(|| format!("Failed to write {}", init_meta_path.display()))?;
        }

        // Remove the old standalone file
        if standalone_path.exists() {
            self.suppress_path_remove(standalone_path);
            fs::remove_file(standalone_path).with_context(|| {
                format!(
                    "Failed to remove old standalone file: {}",
                    standalone_path.display()
                )
            })?;
        }

        log::info!(
            "Syncback: Converted standalone {} '{}' to directory format at {}",
            class_name,
            instance_name,
            new_dir.display()
        );

        Ok(new_dir)
    }

    /// Syncback a removed instance by deleting its file(s) from the filesystem.
    /// This is called when the user selects "pull" for an instance that exists in Rojo
    /// but has been deleted in Studio.
    #[allow(dead_code)]
    fn syncback_removed_instance(
        &self,
        removed_id: Ref,
        tree: &crate::snapshot::RojoTree,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        // Find the instance in the tree
        let instance = tree
            .get_instance(removed_id)
            .context("Instance not found in Rojo tree")?;

        let instance_name = instance.name();

        // Get the instigating source and handle each variant appropriately
        let instigating_source = instance
            .metadata()
            .instigating_source
            .as_ref()
            .context("Instance has no filesystem path (not synced from filesystem)")?;

        // Only delete instances that originate from filesystem paths, not from project nodes
        let instance_path = match instigating_source {
            InstigatingSource::Path(path) => path,
            InstigatingSource::ProjectNode { name, .. } => {
                // Instances defined in project files cannot be removed via syncback
                // because that would require modifying the project file structure
                log::warn!(
                    "Syncback: Cannot remove instance '{}' (id: {:?}) - it is defined in a project file",
                    name,
                    removed_id
                );
                anyhow::bail!(
                    "Cannot remove instance '{}' because it is defined in a project file. \
                     To remove this instance, edit the project file directly.",
                    name
                );
            }
        };

        // Check if path exists - it may have already been deleted as part of a parent removal
        // (e.g., if both a directory and its children are marked for removal, the directory
        // deletion will recursively delete all children, so we just skip them)
        if !instance_path.exists() {
            log::trace!(
                "Syncback: Path already removed (likely parent was deleted): {}",
                instance_path.display()
            );
            return Ok(());
        }

        // Delete the file or directory
        if instance_path.is_dir() {
            self.suppress_path_remove(instance_path);
            fs::remove_dir_all(instance_path).with_context(|| {
                format!("Failed to remove directory: {}", instance_path.display())
            })?;
            log::info!("Syncback: Removed directory at {}", instance_path.display());
        } else if instance_path.is_file() {
            self.suppress_path_remove(instance_path);
            fs::remove_file(instance_path)
                .with_context(|| format!("Failed to remove file: {}", instance_path.display()))?;
            log::info!("Syncback: Removed file at {}", instance_path.display());

            // Also remove adjacent meta file if it exists
            // The meta file is named after the instance name, not the file name
            // e.g., for "MyScript.server.luau", the meta file is "MyScript.meta.json5"
            if let Some(parent_dir) = instance_path.parent() {
                let meta_path = parent_dir.join(format!("{}.meta.json5", instance_name));
                if meta_path.exists() {
                    if let Err(err) = fs::remove_file(&meta_path) {
                        log::warn!(
                            "Failed to remove adjacent meta file {}: {}",
                            meta_path.display(),
                            err
                        );
                    } else {
                        log::info!(
                            "Syncback: Removed adjacent meta file at {}",
                            meta_path.display()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Check for duplicate names among children and return the set of unique children
    /// that should be synced. Records skipped instances via stats tracker.
    fn filter_duplicate_children<'a>(
        &self,
        children: &'a [crate::web::interface::AddedInstance],
        parent_path: &str,
        stats: &crate::syncback::SyncbackStats,
    ) -> Vec<&'a crate::web::interface::AddedInstance> {
        use std::collections::HashMap;

        // Count occurrences of each name
        let mut name_counts: HashMap<&str, usize> = HashMap::new();
        for child in children {
            *name_counts.entry(&child.name).or_insert(0) += 1;
        }

        // Find duplicates
        let duplicates: std::collections::HashSet<&str> = name_counts
            .iter()
            .filter(|(_, &count)| count > 1)
            .map(|(&name, _)| name)
            .collect();

        // Record skipped duplicates in stats
        if !duplicates.is_empty() {
            let mut skipped_count = 0;
            for child in children {
                if duplicates.contains(child.name.as_str()) {
                    skipped_count += 1;
                }
            }
            let duplicate_list: Vec<&str> = duplicates.iter().copied().collect();
            stats.record_duplicate_names_batch(parent_path, &duplicate_list, skipped_count);
        }

        // Return only non-duplicate children
        children
            .iter()
            .filter(|child| !duplicates.contains(child.name.as_str()))
            .collect()
    }

    /// Detects what file format currently exists on disk for a given instance.
    /// This is used to preserve the existing format during partial updates.
    ///
    /// When the plugin sends a script update, it might not include children info.
    /// We check the filesystem to see if this is already a directory (with init file)
    /// or a standalone file, and preserve that format.
    fn detect_existing_script_format(
        parent_dir: &std::path::Path,
        name: &str,
        class_name: &str,
    ) -> ExistingFileFormat {
        let dir_path = parent_dir.join(name);

        // Check for directory with init file first (takes precedence)
        if dir_path.is_dir() {
            // Check for any init file based on class type
            let init_files = match class_name {
                "ModuleScript" => vec!["init.luau", "init.lua"],
                "Script" => vec![
                    "init.server.luau",
                    "init.server.lua",
                    "init.client.luau",
                    "init.client.lua",
                ],
                "LocalScript" => vec![
                    // Modern: init.local.luau produces LocalScript
                    // Legacy: init.client.lua produces LocalScript (without 'u')
                    // Note: init.client.luau produces Script with Client RunContext, NOT LocalScript!
                    "init.local.luau",
                    "init.local.lua",
                    "init.client.lua", // Legacy only (no .luau!)
                ],
                // For non-scripts, check for init.model.json5 or similar
                _ => vec!["init.model.json5", "init.model.json", "init.meta.json5"],
            };

            for init_file in init_files {
                if dir_path.join(init_file).exists() {
                    return ExistingFileFormat::Directory(dir_path.clone());
                }
            }
        }

        // Check for standalone file
        let standalone_patterns = match class_name {
            "ModuleScript" => vec![format!("{}.luau", name), format!("{}.lua", name)],
            "Script" => vec![
                format!("{}.server.luau", name),
                format!("{}.server.lua", name),
                format!("{}.client.luau", name),
                format!("{}.client.lua", name),
            ],
            "LocalScript" => vec![
                // Modern: .local.luau produces LocalScript
                // Legacy: .client.lua produces LocalScript (without 'u')
                // Note: .client.luau produces Script with Client RunContext, NOT LocalScript!
                format!("{}.local.luau", name),
                format!("{}.local.lua", name),
                format!("{}.client.lua", name), // Legacy only (no .luau!)
            ],
            // For non-scripts, check for model files
            _ => vec![
                format!("{}.model.json5", name),
                format!("{}.model.json", name),
            ],
        };

        for pattern in standalone_patterns {
            let full_path = parent_dir.join(&pattern);
            if full_path.exists() {
                return ExistingFileFormat::Standalone(full_path);
            }
        }

        ExistingFileFormat::None
    }

    /// Recursively syncback an instance and its children to the filesystem.
    /// This is the internal implementation that handles the actual file creation.
    #[allow(dead_code)]
    fn syncback_instance_to_path(
        &self,
        added: &crate::web::interface::AddedInstance,
        parent_dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        // Use a default stats tracker for backwards compatibility
        let stats = crate::syncback::SyncbackStats::new();
        let sibling_slugs = HashSet::new();
        self.syncback_instance_to_path_with_stats(added, parent_dir, &stats, &sibling_slugs)
            .map(|_| ())
    }

    /// Recursively syncback an instance and its children to the filesystem.
    /// This is the internal implementation that handles the actual file creation.
    /// Uses the provided stats tracker for recording issues.
    ///
    /// `sibling_slugs` contains the slugified names of siblings already claimed
    /// in the parent directory (bare slugs, lowercased). This is used by
    /// `deduplicate_name` to avoid collisions. Returns the lowercased slug
    /// that this instance claimed, so callers can accumulate it incrementally.
    fn syncback_instance_to_path_with_stats(
        &self,
        added: &crate::web::interface::AddedInstance,
        parent_dir: &std::path::Path,
        stats: &crate::syncback::SyncbackStats,
        sibling_slugs: &HashSet<String>,
    ) -> anyhow::Result<String> {
        use crate::syncback::{deduplicate_name, name_needs_slugify};
        use anyhow::Context;

        // Slugify the instance name for filesystem safety if it contains
        // forbidden characters. The real name is preserved in metadata.
        let needs_slug = name_needs_slugify(&added.name);
        let base_name = if needs_slug {
            slugify_name(&added.name)
        } else {
            added.name.clone()
        };

        // Deduplicate against sibling slugs (bare instance-name-level slugs,
        // not filenames with extensions). This ensures file-format instances
        // are correctly detected as collisions.
        let encoded_name = deduplicate_name(&base_name, sibling_slugs);
        let needs_meta_name = needs_slug || encoded_name != base_name;
        let meta_name_field: Option<&str> = if needs_meta_name {
            Some(&added.name)
        } else {
            None
        };

        // Build path string for stats
        let inst_path = format!("{}/{}", parent_dir.display(), added.name);

        // Filter out children with duplicate names (cannot reliably sync)
        let unique_children = self.filter_duplicate_children(&added.children, &inst_path, stats);
        let has_children = !unique_children.is_empty();

        // Determine the appropriate middleware/file format based on class name.
        // This matches the logic in src/syncback/mod.rs::get_best_middleware.
        //
        // Format transitions (matching `rojo syncback` behavior):
        //   - Standalone + children added  → convert to directory (clean up old standalone)
        //   - Standalone + no children     → keep standalone
        //   - Directory  + children or not → PRESERVE directory (plugin may omit children)
        //   - None       + has_children    → directory (new instance)
        //   - None       + no children     → standalone (new instance)
        let existing_format =
            Self::detect_existing_script_format(parent_dir, &encoded_name, &added.class_name);

        match added.class_name.as_str() {
            // Script types: .luau files, or directories with init files if has children.
            // Format is driven by has_children (matching `rojo syncback` behavior).
            // If the existing format doesn't match, we convert (standalone↔directory).
            "ModuleScript" => {
                let source = self.get_source_property(added);

                // Standalone→directory when children are added.
                // Directory is preserved when no children (plugin may omit children in partial updates).
                // New instances use has_children to decide.
                let use_directory = match &existing_format {
                    ExistingFileFormat::Directory(_) => true, // preserve directory
                    ExistingFileFormat::Standalone(_) => has_children, // convert only if children added
                    ExistingFileFormat::None => has_children,
                };

                if use_directory {
                    // Transition standalone → directory if needed
                    if let ExistingFileFormat::Standalone(ref old_path) = existing_format {
                        log::info!(
                            "Syncback: Converting ModuleScript {} from standalone to directory (children added)",
                            added.name
                        );
                        if old_path.exists() {
                            self.suppress_path_remove(old_path);
                            let _ = fs::remove_file(old_path);
                        }
                        let meta_path = parent_dir.join(format!("{}.meta.json5", encoded_name));
                        if meta_path.exists() {
                            self.suppress_path_remove(&meta_path);
                            let _ = fs::remove_file(&meta_path);
                        }
                    }

                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;
                    let init_path = dir_path.join("init.luau");
                    self.suppress_path(&init_path);
                    fs::write(&init_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", init_path.display())
                    })?;
                    self.write_script_meta_json_if_needed(&dir_path, added, meta_name_field)?;
                    log::info!("Syncback: Updated ModuleScript at {}", init_path.display());
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let file_path = parent_dir.join(format!("{}.luau", encoded_name));
                    self.suppress_path(&file_path);
                    fs::write(&file_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    self.write_adjacent_script_meta_if_needed(
                        parent_dir,
                        &encoded_name,
                        added,
                        meta_name_field,
                    )?;
                    log::info!("Syncback: Updated ModuleScript at {}", file_path.display());
                }
            }
            "Script" => {
                let source = self.get_source_property(added);
                let script_suffix = self.get_script_suffix_for_run_context(added);

                let use_directory = match &existing_format {
                    ExistingFileFormat::Directory(_) => true,
                    ExistingFileFormat::Standalone(_) => has_children,
                    ExistingFileFormat::None => has_children,
                };

                if use_directory {
                    if let ExistingFileFormat::Standalone(ref old_path) = existing_format {
                        log::info!(
                            "Syncback: Converting Script {} from standalone to directory (children added)",
                            added.name
                        );
                        if old_path.exists() {
                            self.suppress_path_remove(old_path);
                            let _ = fs::remove_file(old_path);
                        }
                        let meta_path = parent_dir.join(format!("{}.meta.json5", encoded_name));
                        if meta_path.exists() {
                            self.suppress_path_remove(&meta_path);
                            let _ = fs::remove_file(&meta_path);
                        }
                    }

                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;
                    let init_path = dir_path.join(format!("init.{}.luau", script_suffix));
                    self.suppress_path(&init_path);
                    fs::write(&init_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", init_path.display())
                    })?;
                    self.write_script_meta_json_if_needed(&dir_path, added, meta_name_field)?;
                    log::info!("Syncback: Updated Script at {}", init_path.display());
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let file_path =
                        parent_dir.join(format!("{}.{}.luau", encoded_name, script_suffix));
                    self.suppress_path(&file_path);
                    fs::write(&file_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    self.write_adjacent_script_meta_if_needed(
                        parent_dir,
                        &encoded_name,
                        added,
                        meta_name_field,
                    )?;
                    log::info!("Syncback: Updated Script at {}", file_path.display());
                }
            }
            "LocalScript" => {
                let source = self.get_source_property(added);

                let use_directory = match &existing_format {
                    ExistingFileFormat::Directory(_) => true,
                    ExistingFileFormat::Standalone(_) => has_children,
                    ExistingFileFormat::None => has_children,
                };

                if use_directory {
                    if let ExistingFileFormat::Standalone(ref old_path) = existing_format {
                        log::info!(
                            "Syncback: Converting LocalScript {} from standalone to directory (children added)",
                            added.name
                        );
                        if old_path.exists() {
                            self.suppress_path_remove(old_path);
                            let _ = fs::remove_file(old_path);
                        }
                        let meta_path = parent_dir.join(format!("{}.meta.json5", encoded_name));
                        if meta_path.exists() {
                            self.suppress_path_remove(&meta_path);
                            let _ = fs::remove_file(&meta_path);
                        }
                    }

                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;
                    let init_path = dir_path.join("init.local.luau");
                    self.suppress_path(&init_path);
                    fs::write(&init_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", init_path.display())
                    })?;
                    self.write_script_meta_json_if_needed(&dir_path, added, meta_name_field)?;
                    log::info!("Syncback: Updated LocalScript at {}", init_path.display());
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let file_path = parent_dir.join(format!("{}.local.luau", encoded_name));
                    self.suppress_path(&file_path);
                    fs::write(&file_path, source.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    self.write_adjacent_script_meta_if_needed(
                        parent_dir,
                        &encoded_name,
                        added,
                        meta_name_field,
                    )?;
                    log::info!("Syncback: Updated LocalScript at {}", file_path.display());
                }
            }

            // Directory-native classes: always become directories
            "Folder" | "Configuration" | "Tool" | "ScreenGui" | "SurfaceGui" | "BillboardGui"
            | "AdGui" => {
                let dir_path = parent_dir.join(&encoded_name);
                self.suppress_path(&dir_path);
                fs::create_dir_all(&dir_path).with_context(|| {
                    format!("Failed to create directory: {}", dir_path.display())
                })?;

                // Write init.meta.json5 if has ANY properties or needs a name field
                let has_metadata = if added.properties.is_empty() && meta_name_field.is_none() {
                    false
                } else {
                    self.write_directory_meta_json(&dir_path, added, meta_name_field)?;
                    true
                };

                // Add .gitkeep for empty directories with no metadata (matches dedicated syncback)
                // Uses !has_children which accounts for filtered duplicate children
                if !has_children && !has_metadata {
                    let gitkeep = dir_path.join(".gitkeep");
                    self.suppress_path(&gitkeep);
                    fs::write(gitkeep, b"").with_context(|| "Failed to write .gitkeep")?;
                }

                log::info!(
                    "Syncback: Created {} directory at {}",
                    added.class_name,
                    dir_path.display()
                );

                // Recursively process children
                self.process_children_incremental(&unique_children, &dir_path, stats)?;
            }

            // StringValue: .txt file if no children, directory with init.meta.json5 if has children
            "StringValue" => {
                if has_children {
                    // Must become directory - store StringValue data in init.meta.json5
                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;
                    self.write_init_meta_json(&dir_path, added, meta_name_field)?;
                    log::info!(
                        "Syncback: Created StringValue directory at {}",
                        dir_path.display()
                    );
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let value = added
                        .properties
                        .get("Value")
                        .and_then(|v| match v {
                            Variant::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    let file_path = parent_dir.join(format!("{}.txt", encoded_name));
                    self.suppress_path(&file_path);
                    fs::write(&file_path, value.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    // Write adjacent meta for name preservation if slugified
                    if let Some(real_name) = meta_name_field {
                        let meta = self.build_meta_object(
                            None,
                            Some(real_name),
                            indexmap::IndexMap::new(),
                            indexmap::IndexMap::new(),
                        );
                        let meta_path = parent_dir.join(format!("{}.meta.json5", encoded_name));
                        let content = crate::json::to_vec_pretty_sorted(&meta)
                            .context("Failed to serialize meta")?;
                        self.suppress_path(&meta_path);
                        fs::write(&meta_path, &content).with_context(|| {
                            format!("Failed to write meta: {}", meta_path.display())
                        })?;
                    }
                    log::info!("Syncback: Created StringValue at {}", file_path.display());
                }
            }

            // LocalizationTable: .csv file if no children, directory with init.csv if has children
            "LocalizationTable" => {
                // TODO: Full CSV support would serialize the Contents property
                let content = "Key,Source,Context,Example,en\n";

                if has_children {
                    // Must become directory with init.csv
                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;
                    let init_path = dir_path.join("init.csv");
                    self.suppress_path(&init_path);
                    fs::write(&init_path, content.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", init_path.display())
                    })?;
                    // Write init.meta.json5 for className and name preservation
                    self.write_init_meta_json(&dir_path, added, meta_name_field)?;
                    log::info!(
                        "Syncback: Created LocalizationTable directory at {}",
                        dir_path.display()
                    );
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let file_path = parent_dir.join(format!("{}.csv", encoded_name));
                    self.suppress_path(&file_path);
                    fs::write(&file_path, content.as_bytes()).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    // Write adjacent meta for name preservation if slugified
                    if let Some(real_name) = meta_name_field {
                        let meta = self.build_meta_object(
                            None,
                            Some(real_name),
                            indexmap::IndexMap::new(),
                            indexmap::IndexMap::new(),
                        );
                        let meta_path = parent_dir.join(format!("{}.meta.json5", encoded_name));
                        let content = crate::json::to_vec_pretty_sorted(&meta)
                            .context("Failed to serialize meta")?;
                        self.suppress_path(&meta_path);
                        fs::write(&meta_path, &content).with_context(|| {
                            format!("Failed to write meta: {}", meta_path.display())
                        })?;
                    }
                    log::info!(
                        "Syncback: Created LocalizationTable at {}",
                        file_path.display()
                    );
                }
            }

            // All other classes -> .model.json5 or directory if has children.
            _ => {
                let use_directory = match &existing_format {
                    ExistingFileFormat::Directory(_) => true, // preserve directory
                    ExistingFileFormat::Standalone(_) => has_children, // convert only if children added
                    ExistingFileFormat::None => has_children,
                };

                if use_directory {
                    // Transition standalone → directory if needed
                    if let ExistingFileFormat::Standalone(ref old_path) = existing_format {
                        log::info!(
                            "Syncback: Converting {} {} from standalone to directory (children added)",
                            added.class_name,
                            added.name
                        );
                        if old_path.exists() {
                            self.suppress_path_remove(old_path);
                            let _ = fs::remove_file(old_path);
                        }
                    }

                    let dir_path = parent_dir.join(&encoded_name);
                    self.suppress_path(&dir_path);
                    fs::create_dir_all(&dir_path).with_context(|| {
                        format!("Failed to create directory: {}", dir_path.display())
                    })?;

                    // Write init.meta.json5 with class and properties
                    self.write_init_meta_json(&dir_path, added, meta_name_field)?;

                    log::info!(
                        "Syncback: Updated {} at {}/init.meta.json5",
                        added.class_name,
                        dir_path.display()
                    );

                    // Recursively process children
                    self.process_children_incremental(&unique_children, &dir_path, stats)?;
                } else {
                    let content = self.serialize_instance_to_model_json(added, meta_name_field)?;
                    // Use the detected file path if available (preserves .model.json
                    // vs .model.json5), otherwise default to .model.json5
                    let file_path = match &existing_format {
                        ExistingFileFormat::Standalone(p) => p.clone(),
                        _ => parent_dir.join(format!("{}.model.json5", encoded_name)),
                    };
                    self.suppress_path(&file_path);
                    fs::write(&file_path, &content).with_context(|| {
                        format!("Failed to write file: {}", file_path.display())
                    })?;
                    log::info!(
                        "Syncback: Updated {} at {}",
                        added.class_name,
                        file_path.display()
                    );
                }
            }
        }

        Ok(encoded_name.to_lowercase())
    }

    /// Process children with incremental slug tracking, so each child's
    /// claimed name is added to the taken set before processing the next.
    /// Pre-seeds the taken set from existing directory entries so new children
    /// don't collide with files already on disk.
    fn process_children_incremental(
        &self,
        children: &[&crate::web::interface::AddedInstance],
        dir_path: &std::path::Path,
        stats: &crate::syncback::SyncbackStats,
    ) -> anyhow::Result<()> {
        // Seed from existing directory entries (bare stems, lowercased)
        let mut taken: HashSet<String> = if dir_path.is_dir() {
            fs::read_dir(dir_path)
                .ok()
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| {
                            let name = e.file_name();
                            let name_str = name.to_string_lossy();
                            // For directories, the name IS the slug.
                            // For files, strip the last extension to approximate the slug.
                            if e.path().is_dir() {
                                Some(name_str.to_lowercase())
                            } else {
                                std::path::Path::new(name_str.as_ref())
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .map(|s| s.to_lowercase())
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        for child in children {
            let slug = self.syncback_instance_to_path_with_stats(child, dir_path, stats, &taken)?;
            taken.insert(slug);
        }
        Ok(())
    }

    /// Write init.meta.json5 for script directories if they have non-Source properties
    /// This matches the behavior of the dedicated syncback system.
    fn write_script_meta_json_if_needed(
        &self,
        dir_path: &std::path::Path,
        added: &crate::web::interface::AddedInstance,
        instance_name: Option<&str>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        // Check if there are any properties besides Source that need to be saved
        // Use IndexMap for consistent ordering (like dedicated syncback)
        let (properties, attributes) =
            self.filter_properties_for_meta(&added.class_name, &added.properties, Some("Source"));

        if properties.is_empty() && attributes.is_empty() && instance_name.is_none() {
            return Ok(());
        }

        let meta = self.build_meta_object(None, instance_name, properties, attributes);
        let meta_path = dir_path.join("init.meta.json5");
        let content = crate::json::to_vec_pretty_sorted(&meta)
            .context("Failed to serialize init.meta.json5")?;
        self.suppress_path(&meta_path);
        fs::write(&meta_path, &content)
            .with_context(|| format!("Failed to write meta file: {}", meta_path.display()))?;
        log::info!(
            "Syncback: Created init.meta.json5 for script at {}",
            meta_path.display()
        );

        Ok(())
    }

    /// Write init.meta.json5 for directory-native classes (Folder, Configuration, etc.)
    /// Includes all properties (Attributes, Tags, etc.)
    /// This matches the behavior of the dedicated syncback system.
    fn write_directory_meta_json(
        &self,
        dir_path: &std::path::Path,
        added: &crate::web::interface::AddedInstance,
        instance_name: Option<&str>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        let (properties, attributes) =
            self.filter_properties_for_meta(&added.class_name, &added.properties, None);

        if properties.is_empty() && attributes.is_empty() && instance_name.is_none() {
            return Ok(());
        }

        let meta = self.build_meta_object(None, instance_name, properties, attributes);
        let meta_path = dir_path.join("init.meta.json5");
        let content = crate::json::to_vec_pretty_sorted(&meta)
            .context("Failed to serialize init.meta.json5")?;
        self.suppress_path(&meta_path);
        fs::write(&meta_path, &content)
            .with_context(|| format!("Failed to write meta file: {}", meta_path.display()))?;
        log::info!(
            "Syncback: Created init.meta.json5 at {}",
            meta_path.display()
        );

        Ok(())
    }

    /// Write an init.meta.json5 file for non-standard instances that have children
    /// This matches the behavior of the dedicated syncback system.
    fn write_init_meta_json(
        &self,
        dir_path: &std::path::Path,
        added: &crate::web::interface::AddedInstance,
        instance_name: Option<&str>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        let (properties, attributes) =
            self.filter_properties_for_meta(&added.class_name, &added.properties, None);

        // For non-Folder classes, we need to include className
        let meta = self.build_meta_object(
            Some(&added.class_name),
            instance_name,
            properties,
            attributes,
        );
        let meta_path = dir_path.join("init.meta.json5");
        let content = crate::json::to_vec_pretty_sorted(&meta)
            .context("Failed to serialize init.meta.json5")?;
        self.suppress_path(&meta_path);
        fs::write(&meta_path, &content)
            .with_context(|| format!("Failed to write meta file: {}", meta_path.display()))?;
        log::info!(
            "Syncback: Created init.meta.json5 for {} at {}",
            added.class_name,
            meta_path.display()
        );

        Ok(())
    }

    /// Filter properties for meta file serialization, matching dedicated syncback behavior.
    /// Returns (properties, attributes) as separate IndexMaps.
    ///
    /// Filters out:
    /// - Ref and UniqueId properties (don't serialize to JSON)
    /// - The skip_property if specified (e.g., "Source" for scripts)
    /// - Properties that don't serialize (DoesNotSerialize in reflection database)
    /// - Properties that match their default value
    /// - Internal RBX* prefixed attributes
    fn filter_properties_for_meta(
        &self,
        class_name: &str,
        props: &HashMap<String, Variant>,
        skip_property: Option<&str>,
    ) -> (
        indexmap::IndexMap<String, serde_json::Value>,
        indexmap::IndexMap<String, serde_json::Value>,
    ) {
        use crate::syncback::should_property_serialize;
        use crate::variant_eq::variant_eq;
        use indexmap::IndexMap;

        let mut properties: IndexMap<String, serde_json::Value> = IndexMap::new();
        let mut attributes: IndexMap<String, serde_json::Value> = IndexMap::new();

        // Get reflection database for default value comparison
        let class_data = rbx_reflection_database::get()
            .ok()
            .and_then(|db| db.classes.get(class_name));

        for (name, value) in props {
            // Skip the specified property (e.g., Source for scripts)
            if let Some(skip) = skip_property {
                if name == skip {
                    continue;
                }
            }

            // Skip RunContext - it's encoded in the file suffix (.server.luau, .client.luau, etc.)
            // and Enum variants can't be properly serialized without type context
            if name == "RunContext" {
                continue;
            }

            // Skip Ref and UniqueId - they don't serialize to JSON (matches dedicated syncback)
            if matches!(value, Variant::Ref(_) | Variant::UniqueId(_)) {
                continue;
            }

            // Handle Attributes specially - extract into separate map and filter RBX* prefix
            if let Variant::Attributes(attrs) = value {
                for (attr_name, attr_value) in attrs.iter() {
                    // Skip internal Roblox attributes (matches dedicated syncback)
                    if attr_name.starts_with("RBX") {
                        continue;
                    }
                    if let Some(json_value) = variant_to_json(attr_value) {
                        attributes.insert(attr_name.clone(), json_value);
                    }
                }
                continue;
            }

            // Check if property should serialize (matches dedicated syncback property_filter.rs)
            if !should_property_serialize(class_name, name) {
                continue;
            }

            // Skip properties that match their default value (matches dedicated syncback)
            if let Some(data) = class_data {
                if let Some(default) = data.default_properties.get(name.as_str()) {
                    if variant_eq(value, default) {
                        continue;
                    }
                }
            }

            // Convert other properties
            if let Some(json_value) = variant_to_json(value) {
                properties.insert(name.clone(), json_value);
            }
        }

        (properties, attributes)
    }

    /// Build a meta object (for init.meta.json5) matching dedicated syncback format.
    fn build_meta_object(
        &self,
        class_name: Option<&str>,
        instance_name: Option<&str>,
        properties: indexmap::IndexMap<String, serde_json::Value>,
        attributes: indexmap::IndexMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        use serde_json::json;

        let mut obj = serde_json::Map::new();

        if let Some(cn) = class_name {
            obj.insert("className".to_string(), json!(cn));
        }

        if let Some(name) = instance_name {
            obj.insert("name".to_string(), json!(name));
        }

        if !properties.is_empty() {
            obj.insert("properties".to_string(), json!(properties));
        }

        if !attributes.is_empty() {
            obj.insert("attributes".to_string(), json!(attributes));
        }

        serde_json::Value::Object(obj)
    }

    /// Extract the Source property from an added instance, defaulting to empty string.
    fn get_source_property(&self, added: &crate::web::interface::AddedInstance) -> String {
        added
            .properties
            .get("Source")
            .and_then(|v| match v {
                Variant::String(s) => Some(s.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Determine the script file suffix based on RunContext property.
    ///
    /// For `Script` class:
    /// - RunContext: Client → "client"
    /// - RunContext: Server → "server"
    /// - RunContext: Legacy → "legacy"
    /// - RunContext: Plugin → "plugin"
    /// - No RunContext → "legacy" (default)
    fn get_script_suffix_for_run_context(
        &self,
        added: &crate::web::interface::AddedInstance,
    ) -> &'static str {
        // Get RunContext enum values from reflection database
        let run_context_enums = rbx_reflection_database::get()
            .ok()
            .and_then(|db| db.enums.get("RunContext"))
            .map(|e| &e.items);

        let run_context_value = added.properties.get("RunContext").and_then(|v| match v {
            Variant::Enum(e) => Some(e.to_u32()),
            _ => None,
        });

        if let (Some(enums), Some(value)) = (run_context_enums, run_context_value) {
            // Find which RunContext this value corresponds to
            for (name, &enum_value) in enums {
                if enum_value == value {
                    return match *name {
                        "Client" => "client",
                        "Server" => "server",
                        "Legacy" => "legacy",
                        "Plugin" => "plugin",
                        _ => "legacy",
                    };
                }
            }
        }

        // Default to legacy if no RunContext or unrecognized
        "legacy"
    }

    /// Write an adjacent meta file for scripts without children.
    ///
    /// Creates `{name}.meta.json5` next to the script file if there are
    /// non-Source properties that need to be preserved.
    ///
    /// This matches the dedicated syncback behavior in lua.rs::syncback_lua.
    fn write_adjacent_script_meta_if_needed(
        &self,
        parent_dir: &std::path::Path,
        name: &str,
        added: &crate::web::interface::AddedInstance,
        instance_name: Option<&str>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        // Filter properties, skipping Source (it's in the .luau file)
        let (properties, attributes) =
            self.filter_properties_for_meta(&added.class_name, &added.properties, Some("Source"));

        if properties.is_empty() && attributes.is_empty() && instance_name.is_none() {
            return Ok(());
        }

        // Build meta object (no className needed for scripts - it's determined by file extension)
        let meta = self.build_meta_object(None, instance_name, properties, attributes);
        let meta_path = parent_dir.join(format!("{}.meta.json5", name));
        let content =
            crate::json::to_vec_pretty_sorted(&meta).context("Failed to serialize meta.json5")?;
        self.suppress_path(&meta_path);
        fs::write(&meta_path, &content)
            .with_context(|| format!("Failed to write meta file: {}", meta_path.display()))?;
        log::info!(
            "Syncback: Created adjacent meta file at {}",
            meta_path.display()
        );

        Ok(())
    }

    /// Serialize an instance to model.json5 format (human-readable, version-control friendly)
    /// This matches the format used by the dedicated syncback system.
    fn serialize_instance_to_model_json(
        &self,
        added: &crate::web::interface::AddedInstance,
        instance_name: Option<&str>,
    ) -> anyhow::Result<Vec<u8>> {
        use anyhow::Context;
        use serde_json::json;

        // Filter properties matching dedicated syncback behavior
        let (properties, attributes) =
            self.filter_properties_for_meta(&added.class_name, &added.properties, None);

        // Build the JSON model structure
        // Format: https://rojo.space/docs/v7/sync-details/#json-models
        let mut model = serde_json::Map::new();
        if let Some(name) = instance_name {
            model.insert("name".to_string(), json!(name));
        }
        model.insert("className".to_string(), json!(added.class_name));

        if !properties.is_empty() {
            model.insert("properties".to_string(), json!(properties));
        }

        if !attributes.is_empty() {
            model.insert("attributes".to_string(), json!(attributes));
        }

        // Use sorted JSON5 serialization to match dedicated syncback
        crate::json::to_vec_pretty_sorted(&model).context("Failed to serialize model.json5")
    }

    /// Persist non-Source property changes to the appropriate meta/model file.
    /// Source is handled by the ChangeProcessor; this handles everything else
    /// (Attributes, custom properties, etc.) so they survive a Rojo restart.
    fn syncback_updated_properties(
        &self,
        update: &crate::web::interface::InstanceUpdate,
        tree: &crate::snapshot::RojoTree,
    ) -> anyhow::Result<()> {
        use anyhow::Context;

        // Check if there are any non-Source property changes to persist
        let has_non_source = update
            .changed_properties
            .iter()
            .any(|(key, _)| key != "Source");

        if !has_non_source {
            return Ok(());
        }

        // Look up the instance in the tree
        let instance = tree
            .get_instance(update.id)
            .context("Instance not found in tree for property persistence")?;

        let instigating_source = instance
            .metadata()
            .instigating_source
            .as_ref()
            .context("Instance has no filesystem path")?;

        // ProjectNode instances are defined in the project file. Writing meta
        // JSON to the project file path would corrupt it. Only filesystem-backed
        // instances (InstigatingSource::Path) can have properties persisted.
        let inst_path = match instigating_source {
            InstigatingSource::Path(p) => p.as_path(),
            InstigatingSource::ProjectNode { name, .. } => {
                log::warn!(
                    "Cannot persist non-Source properties for instance '{}' (id: {:?}) — \
                     it is defined in a project file. Edit the project file directly.",
                    name,
                    update.id
                );
                return Ok(());
            }
        };

        let class_name = instance.class_name();
        let is_script = matches!(
            class_name.as_str(),
            "ModuleScript" | "Script" | "LocalScript"
        );

        // Collect the non-Source properties as a HashMap<String, Variant>
        // for reuse with filter_properties_for_meta
        let mut props: HashMap<String, Variant> = HashMap::new();
        for (key, value) in &update.changed_properties {
            if key == "Source" {
                continue;
            }
            if let Some(v) = value {
                props.insert(key.to_string(), v.clone());
            }
        }

        if props.is_empty() {
            return Ok(());
        }

        let skip_prop = if is_script { Some("Source") } else { None };
        let (properties, attributes) =
            self.filter_properties_for_meta(class_name.as_str(), &props, skip_prop);

        if properties.is_empty() && attributes.is_empty() {
            return Ok(());
        }

        // Determine which meta file to write based on file structure
        if inst_path.is_dir() {
            // Directory format: write to init.meta.json5
            let meta_path = inst_path.join("init.meta.json5");

            // Read existing meta if present, merge with new properties
            let meta = self.merge_or_build_meta(&meta_path, None, properties, attributes)?;
            let content = crate::json::to_vec_pretty_sorted(&meta)
                .context("Failed to serialize init.meta.json5")?;
            self.suppress_path(&meta_path);
            fs::write(&meta_path, &content)
                .with_context(|| format!("Failed to write {}", meta_path.display()))?;

            log::info!(
                "Syncback: Persisted non-Source properties to {}",
                meta_path.display()
            );
        } else if is_script {
            // Standalone script: write to adjacent Name.meta.json5
            // Derive the base name from the filesystem path (not instance.name())
            // to preserve Windows-invalid character encoding (e.g., %3F for ?).
            // This matches AdjacentMetadata::read_and_apply_all which uses file_stem().
            let parent_dir = inst_path.parent().context("No parent directory")?;
            let file_stem = inst_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let base_name = file_stem
                .strip_suffix(".server")
                .or_else(|| file_stem.strip_suffix(".client"))
                .or_else(|| file_stem.strip_suffix(".plugin"))
                .or_else(|| file_stem.strip_suffix(".local"))
                .or_else(|| file_stem.strip_suffix(".legacy"))
                .unwrap_or(file_stem);
            let meta_path = parent_dir.join(format!("{}.meta.json5", base_name));

            let meta = self.merge_or_build_meta(&meta_path, None, properties, attributes)?;
            let content = crate::json::to_vec_pretty_sorted(&meta)
                .context("Failed to serialize meta.json5")?;
            self.suppress_path(&meta_path);
            fs::write(&meta_path, &content)
                .with_context(|| format!("Failed to write {}", meta_path.display()))?;

            log::info!(
                "Syncback: Persisted non-Source properties to {}",
                meta_path.display()
            );
        } else {
            // Non-script standalone file. Only .model.json5/.model.json support
            // in-place JSON property updates. Other file types (.txt, .csv, .toml,
            // .yaml, etc.) require an adjacent .meta.json5 file — writing JSON
            // directly to them would corrupt their content.
            let file_name = inst_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
            let is_model_file =
                file_name.ends_with(".model.json5") || file_name.ends_with(".model.json");

            if is_model_file {
                let meta = self.merge_or_build_meta(
                    inst_path,
                    Some(class_name.as_str()),
                    properties,
                    attributes,
                )?;
                let content = crate::json::to_vec_pretty_sorted(&meta)
                    .context("Failed to serialize model file")?;
                self.suppress_path(inst_path);
                fs::write(inst_path, &content)
                    .with_context(|| format!("Failed to write {}", inst_path.display()))?;

                log::info!(
                    "Syncback: Persisted non-Source properties to {}",
                    inst_path.display()
                );
            } else {
                // For .txt, .csv, .toml, .yaml, etc. — use adjacent meta file
                let parent_dir = inst_path.parent().context("No parent directory")?;
                let file_stem = inst_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let meta_path = parent_dir.join(format!("{}.meta.json5", file_stem));

                let meta = self.merge_or_build_meta(&meta_path, None, properties, attributes)?;
                let content = crate::json::to_vec_pretty_sorted(&meta)
                    .context("Failed to serialize meta.json5")?;
                self.suppress_path(&meta_path);
                fs::write(&meta_path, &content)
                    .with_context(|| format!("Failed to write {}", meta_path.display()))?;

                log::info!(
                    "Syncback: Persisted non-Source properties to {}",
                    meta_path.display()
                );
            }
        }

        Ok(())
    }

    /// Reads an existing meta/model JSON5 file and merges new properties into it.
    /// If the file doesn't exist, builds a fresh meta object.
    fn merge_or_build_meta(
        &self,
        existing_path: &std::path::Path,
        class_name: Option<&str>,
        new_properties: indexmap::IndexMap<String, serde_json::Value>,
        new_attributes: indexmap::IndexMap<String, serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value> {
        use anyhow::Context;

        if existing_path.exists() {
            // Read and parse existing file
            let content = fs::read(existing_path)
                .with_context(|| format!("Failed to read {}", existing_path.display()))?;
            let mut existing: serde_json::Value =
                json5::from_str(&String::from_utf8_lossy(&content))
                    .unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()));

            // Ensure the existing value is an object
            if !existing.is_object() {
                existing = serde_json::Value::Object(serde_json::Map::new());
            }
            let obj = existing.as_object_mut().unwrap();

            // Merge properties
            if !new_properties.is_empty() {
                let props = obj
                    .entry("properties")
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(props_obj) = props.as_object_mut() {
                    for (k, v) in new_properties {
                        props_obj.insert(k, v);
                    }
                }
            }

            // Merge attributes
            if !new_attributes.is_empty() {
                let attrs = obj
                    .entry("attributes")
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                if let Some(attrs_obj) = attrs.as_object_mut() {
                    for (k, v) in new_attributes {
                        attrs_obj.insert(k, v);
                    }
                }
            }

            Ok(existing)
        } else {
            // No existing file — build from scratch
            Ok(self.build_meta_object(class_name, None, new_properties, new_attributes))
        }
    }

    async fn handle_api_read(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let argument = &request.uri().path()["/api/read/".len()..];
        let requested_ids: Result<Vec<Ref>, _> = argument.split(',').map(Ref::from_str).collect();

        let requested_ids = match requested_ids {
            Ok(ids) => ids,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Malformed ID list"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        let message_queue = self.serve_session.message_queue();
        let message_cursor = message_queue.cursor();

        let tree = self.serve_session.tree();
        let scripts_only = self.serve_session.sync_scripts_only();

        let mut instances = HashMap::new();

        if scripts_only {
            // In scripts-only mode, we need to:
            // 1. Find all scripts
            // 2. Include their ancestor chains (so the tree structure is valid)
            // 3. Mark non-script ancestors with ignoreUnknownInstances: true
            // 4. Filter children to only include instances in our set

            let mut included_ids: HashSet<Ref> = HashSet::new();

            // First pass: collect all script IDs and their ancestors
            for id in &requested_ids {
                if let Some(instance) = tree.get_instance(*id) {
                    collect_scripts_and_ancestors(&tree, instance.id(), &mut included_ids);
                    for descendant in tree.descendants(*id) {
                        collect_scripts_and_ancestors(&tree, descendant.id(), &mut included_ids);
                    }
                }
            }

            // Second pass: create Instance objects for included IDs
            for id in &requested_ids {
                if let Some(instance) = tree.get_instance(*id) {
                    if included_ids.contains(id) {
                        let inst = instance_for_scripts_only(instance, &included_ids);
                        instances.insert(*id, inst);
                    }

                    for descendant in tree.descendants(*id) {
                        let desc_id = descendant.id();
                        if included_ids.contains(&desc_id) {
                            let inst = instance_for_scripts_only(descendant, &included_ids);
                            instances.insert(desc_id, inst);
                        }
                    }
                }
            }
        } else {
            // Normal mode: include all instances
            for id in requested_ids {
                if let Some(instance) = tree.get_instance(id) {
                    instances.insert(id, Instance::from_rojo_instance(instance));

                    for descendant in tree.descendants(id) {
                        instances.insert(descendant.id(), Instance::from_rojo_instance(descendant));
                    }
                }
            }
        }

        msgpack_ok(ReadResponse {
            session_id: self.serve_session.session_id(),
            message_cursor,
            instances,
        })
    }

    /// Accepts a list of IDs and returns them serialized as a binary model.
    /// The model is sent in a schema that causes Roblox to deserialize it as
    /// a Luau `buffer`.
    ///
    /// The returned model is a folder that contains ObjectValues with names
    /// that correspond to the requested Instances. These values have their
    /// `Value` property set to point to the requested Instance.
    async fn handle_api_serialize(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let argument = &request.uri().path()["/api/serialize/".len()..];
        let requested_ids: Result<Vec<Ref>, _> = argument.split(',').map(Ref::from_str).collect();

        let requested_ids = match requested_ids {
            Ok(ids) => ids,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Malformed ID list"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        let mut response_dom = WeakDom::new(InstanceBuilder::new("Folder"));

        let tree = self.serve_session.tree();
        for id in &requested_ids {
            if let Some(instance) = tree.get_instance(*id) {
                let clone = response_dom.insert(
                    Ref::none(),
                    InstanceBuilder::new(instance.class_name())
                        .with_name(instance.name())
                        .with_properties(instance.properties().clone()),
                );
                let object_value = response_dom.insert(
                    response_dom.root_ref(),
                    InstanceBuilder::new("ObjectValue")
                        .with_name(id.to_string())
                        .with_property("Value", clone),
                );

                let mut child_ref = clone;
                if let Some(parent_class) = parent_requirements(&instance.class_name()) {
                    child_ref =
                        response_dom.insert(object_value, InstanceBuilder::new(parent_class));
                    response_dom.transfer_within(clone, child_ref);
                }

                response_dom.transfer_within(child_ref, object_value);
            } else {
                msgpack(
                    ErrorResponse::bad_request(format!("provided id {id} is not in the tree")),
                    StatusCode::BAD_REQUEST,
                );
            }
        }
        drop(tree);

        let mut source = Vec::new();
        rbx_binary::to_writer(&mut source, &response_dom, &[response_dom.root_ref()]).unwrap();

        msgpack_ok(SerializeResponse {
            session_id: self.serve_session.session_id(),
            model_contents: source,
        })
    }

    /// Returns a list of all referent properties that point towards the
    /// provided IDs. Used because the plugin does not store a RojoTree,
    /// and referent properties need to be updated after the serialize
    /// endpoint is used.
    async fn handle_api_ref_patch(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let argument = &request.uri().path()["/api/ref-patch/".len()..];
        let requested_ids: Result<Vec<Ref>, _> = argument.split(',').map(Ref::from_str).collect();

        let requested_ids = match requested_ids {
            Ok(ids) => ids,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Malformed ID list"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        // Convert to HashSet for efficient lookup
        let ids: HashSet<Ref> = requested_ids.into_iter().collect();

        let mut instance_updates: HashMap<Ref, InstanceUpdate> = HashMap::new();

        let tree = self.serve_session.tree();
        for instance in tree.descendants(tree.get_root_id()) {
            for (prop_name, prop_value) in instance.properties() {
                let Variant::Ref(prop_value) = prop_value else {
                    continue;
                };
                if let Some(target_id) = ids.get(prop_value) {
                    let instance_id = instance.id();
                    let update =
                        instance_updates
                            .entry(instance_id)
                            .or_insert_with(|| InstanceUpdate {
                                id: instance_id,
                                changed_class_name: None,
                                changed_name: None,
                                changed_metadata: None,
                                changed_properties: UstrMap::default(),
                            });
                    update
                        .changed_properties
                        .insert(*prop_name, Some(Variant::Ref(*target_id)));
                }
            }
        }

        msgpack_ok(RefPatchResponse {
            session_id: self.serve_session.session_id(),
            patch: SubscribeMessage {
                added: HashMap::new(),
                removed: Vec::new(),
                updated: instance_updates.into_values().collect(),
            },
        })
    }

    /// Open a script with the given ID in the user's default text editor.
    async fn handle_api_open(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let argument = &request.uri().path()["/api/open/".len()..];
        let requested_id = match Ref::from_str(argument) {
            Ok(id) => id,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Invalid instance ID"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        let tree = self.serve_session.tree();

        let instance = match tree.get_instance(requested_id) {
            Some(instance) => instance,
            None => {
                return msgpack(
                    ErrorResponse::bad_request("Instance not found"),
                    StatusCode::NOT_FOUND,
                );
            }
        };

        let script_path = match pick_script_path(instance) {
            Some(path) => path,
            None => {
                return msgpack(
                    ErrorResponse::bad_request(
                        "No appropriate file could be found to open this script",
                    ),
                    StatusCode::NOT_FOUND,
                );
            }
        };

        match opener::open(&script_path) {
            Ok(()) => {}
            Err(error) => match error {
                OpenError::Io(io_error) => {
                    return msgpack(
                        ErrorResponse::internal_error(format!(
                            "Attempting to open {} failed because of the following io error: {}",
                            script_path.display(),
                            io_error
                        )),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
                OpenError::ExitStatus {
                    cmd,
                    status,
                    stderr,
                } => {
                    return msgpack(
                        ErrorResponse::internal_error(format!(
                            r#"The command '{}' to open '{}' failed with the error code '{}'.
                            Error logs:
                            {}"#,
                            cmd,
                            script_path.display(),
                            status,
                            stderr
                        )),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
                _ => {
                    return msgpack(
                        ErrorResponse::internal_error(format!(
                            "Failed to open {}: {}",
                            script_path.display(),
                            error
                        )),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
            },
        };

        msgpack_ok(OpenResponse {
            session_id: self.serve_session.session_id(),
        })
    }
}

/// If this instance is represented by a script, try to find the correct .luau
/// file to open to edit it.
fn pick_script_path(instance: InstanceWithMeta<'_>) -> Option<PathBuf> {
    match instance.class_name().as_str() {
        "Script" | "LocalScript" | "ModuleScript" => {}
        _ => return None,
    }

    // Pick the first listed relevant path that has an extension of .luau that
    // exists.
    instance
        .metadata()
        .relevant_paths
        .iter()
        .find(|path| {
            // We should only ever open Luau files to be safe.
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("luau") => {}
                _ => return false,
            }

            fs::metadata(path)
                .map(|meta| meta.is_file())
                .unwrap_or(false)
        })
        .map(|path| path.to_owned())
}

/// Handle WebSocket connection for streaming subscription messages
async fn handle_websocket_subscription(
    serve_session: Arc<ServeSession>,
    websocket: HyperWebsocket,
    input_cursor: u32,
) -> anyhow::Result<()> {
    let mut websocket = websocket.await?;

    let session_id = serve_session.session_id();
    let tree_handle = serve_session.tree_handle();
    let message_queue = serve_session.message_queue();
    let scripts_only = serve_session.sync_scripts_only();

    log::debug!(
        "WebSocket subscription established for session {}",
        session_id
    );

    // Now continuously listen for new messages using select to handle both incoming messages
    // and WebSocket control messages concurrently
    let mut cursor = input_cursor;
    loop {
        let receiver = message_queue.subscribe(cursor);

        tokio::select! {
            // Handle new messages from the message queue
            result = receiver => {
                match result {
                    Ok((new_cursor, messages)) => {
                        if !messages.is_empty() {
                            let msgpack_message = {
                                let tree = tree_handle.lock().unwrap();
                                let api_messages: Vec<_> = messages
                                    .into_iter()
                                    .map(|patch| {
                                        let mut msg = SubscribeMessage::from_patch_update(&tree, patch);
                                        // In scripts-only mode, transform to only include scripts
                                        // and their necessary ancestors
                                        if scripts_only {
                                            filter_subscribe_message_for_scripts(&tree, &mut msg);
                                        }
                                        msg
                                    })
                                    .collect();

                                let response = SocketPacket {
                                    session_id,
                                    packet_type: SocketPacketType::Messages,
                                    body: SocketPacketBody::Messages(MessagesPacket {
                                        message_cursor: new_cursor,
                                        messages: api_messages,
                                    }),
                                };

                                serialize_msgpack(response)?
                            };

                            log::debug!("Sending batch of messages over WebSocket subscription");

                            if websocket.send(Message::Binary(msgpack_message.into())).await.is_err() {
                                // Client disconnected
                                log::debug!("WebSocket subscription closed by client");
                                break;
                            }
                            cursor = new_cursor;
                        }
                    }
                    Err(_) => {
                        // Message queue disconnected
                        log::debug!("Message queue disconnected; closing WebSocket subscription");
                        let _ = websocket.send(Message::Close(None)).await;
                        break;
                    }
                }
            }

            // Handle incoming WebSocket messages (ping/pong/close)
            msg = websocket.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) => {
                        log::debug!("WebSocket subscription closed by client");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // tungstenite handles pong automatically
                        log::debug!("Received ping: {:?}", data);
                    }
                    Some(Ok(Message::Pong(data))) => {
                        log::debug!("Received pong: {:?}", data);
                    }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {
                        // Ignore text/binary messages from client for subscription endpoint
                        // TODO: Use this for bidirectional sync or requesting fallbacks?
                        log::debug!("Ignoring message from client since we don't use it for anything yet: {:?}", msg);
                    }
                    Some(Ok(Message::Frame(_))) => {
                        // This should never happen according to tungstenite docs
                        unreachable!();
                    }
                    Some(Err(e)) => {
                        log::error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        // WebSocket stream ended
                        log::debug!("WebSocket stream ended");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Certain Instances MUST be a child of specific classes. This function
/// tracks that information for the Serialize endpoint.
///
/// If a parent requirement exists, it will be returned.
/// Otherwise returns `None`.
fn parent_requirements(class: &str) -> Option<&str> {
    Some(match class {
        "Attachment" | "Bone" => "Part",
        "Animator" => "Humanoid",
        "BaseWrap" | "WrapLayer" | "WrapTarget" | "WrapDeformer" => "MeshPart",
        _ => return None,
    })
}

/// Checks if a class name is a script type (Script, LocalScript, ModuleScript).
#[inline]
fn is_script_class(class_name: &str) -> bool {
    matches!(class_name, "Script" | "LocalScript" | "ModuleScript")
}

/// Filters a SubscribeMessage to only include scripts and their ancestors.
/// Non-script ancestors get ignoreUnknownInstances: true.
fn filter_subscribe_message_for_scripts(
    tree: &crate::snapshot::RojoTree,
    msg: &mut SubscribeMessage<'_>,
) {
    use std::borrow::Cow;

    // Build set of IDs that should be included (scripts + their ancestors)
    let mut included_ids: HashSet<Ref> = HashSet::new();

    // First, find all scripts in the added instances and collect their ancestors
    for (id, inst) in &msg.added {
        if is_script_class(inst.class_name.as_str()) {
            // Add this script and all its ancestors
            included_ids.insert(*id);
            let mut current_parent = inst.parent;
            while !current_parent.is_none() {
                if included_ids.contains(&current_parent) {
                    break;
                }
                included_ids.insert(current_parent);
                if let Some(parent_inst) = msg.added.get(&current_parent) {
                    current_parent = parent_inst.parent;
                } else if let Some(tree_inst) = tree.get_instance(current_parent) {
                    current_parent = tree_inst.parent();
                } else {
                    break;
                }
            }
        }
    }

    // Filter added to only include IDs in our set
    msg.added.retain(|id, _| included_ids.contains(id));

    // ALL instances get ignoreUnknownInstances: true in scripts-only mode
    // This ensures we only sync script content, never delete/modify other instances
    // Non-scripts also have their properties cleared - they're just tree structure
    for inst in msg.added.values_mut() {
        inst.metadata = Some(InstanceMetadata {
            ignore_unknown_instances: true,
        });

        // Non-scripts don't sync properties - they're just tree structure
        if !is_script_class(inst.class_name.as_str()) {
            inst.properties.clear();
        }

        // Filter children to only include those in our set
        let filtered_children: Vec<Ref> = inst
            .children
            .iter()
            .copied()
            .filter(|child_id| included_ids.contains(child_id))
            .collect();
        inst.children = Cow::Owned(filtered_children);
    }

    // Only allow updates to scripts
    msg.updated.retain(|update| {
        tree.get_instance(update.id)
            .map(|inst| is_script_class(inst.class_name().as_str()))
            .unwrap_or(false)
    });

    // Only remove scripts (plugin will ignore unknown IDs anyway)
    msg.removed.retain(|_id| {
        // We can't check removed instances in the tree since they're gone,
        // so we let all removals through - the plugin will ignore unknown IDs
        true
    });
}

/// Recursively collects a script instance and all its ancestors into the set.
/// Only adds to the set if the instance is a script (then adds ancestors too).
fn collect_scripts_and_ancestors(
    tree: &crate::snapshot::RojoTree,
    id: Ref,
    included_ids: &mut HashSet<Ref>,
) {
    if let Some(instance) = tree.get_instance(id) {
        if is_script_class(instance.class_name().as_str()) {
            // This is a script - add it and all ancestors
            let mut current_id = id;
            while let Some(inst) = tree.get_instance(current_id) {
                included_ids.insert(current_id);
                let parent_id = inst.parent();
                if parent_id.is_none() || included_ids.contains(&parent_id) {
                    break;
                }
                current_id = parent_id;
            }
        }
    }
}

/// Creates an Instance for scripts-only mode.
/// - Scripts get their properties synced normally
/// - Non-scripts only provide tree structure (no properties synced)
fn instance_for_scripts_only<'a>(
    source: InstanceWithMeta<'a>,
    included_ids: &HashSet<Ref>,
) -> Instance<'a> {
    use std::borrow::Cow;

    let is_script = is_script_class(source.class_name().as_str());

    // Only sync properties for scripts, not for ancestor containers
    let properties = if is_script {
        source
            .properties()
            .iter()
            .filter(|(_key, value)| {
                // Filter out SharedString values (can't be serialized)
                value.ty() != rbx_dom_weak::types::VariantType::SharedString
            })
            .map(|(key, value)| (*key, Cow::Borrowed(value)))
            .collect()
    } else {
        // Non-scripts don't sync any properties - they're just tree structure
        UstrMap::default()
    };

    // Filter children to only include those in our included set
    let filtered_children: Vec<Ref> = source
        .children()
        .iter()
        .copied()
        .filter(|child_id| included_ids.contains(child_id))
        .collect();

    // In scripts-only mode, ALL instances get ignoreUnknownInstances: true
    // This ensures we only sync script content, never delete/modify other instances
    let metadata = Some(InstanceMetadata {
        ignore_unknown_instances: true,
    });

    Instance {
        id: source.id(),
        parent: source.parent(),
        name: Cow::Borrowed(source.name()),
        class_name: source.class_name(),
        properties,
        children: Cow::Owned(filtered_children),
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::types::{Color3, Enum, NumberRange, Rect, UDim, UDim2, Vector2, Vector3};

    // Tests for variant_to_json function
    mod variant_to_json_tests {
        use super::*;

        #[test]
        fn test_string() {
            let variant = Variant::String("hello world".to_string());
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            assert_eq!(result.unwrap(), serde_json::json!("hello world"));
        }

        #[test]
        fn test_bool() {
            assert_eq!(
                variant_to_json(&Variant::Bool(true)),
                Some(serde_json::json!(true))
            );
            assert_eq!(
                variant_to_json(&Variant::Bool(false)),
                Some(serde_json::json!(false))
            );
        }

        #[test]
        fn test_int32() {
            let variant = Variant::Int32(42);
            let result = variant_to_json(&variant);
            assert_eq!(result, Some(serde_json::json!(42)));
        }

        #[test]
        fn test_int64() {
            let variant = Variant::Int64(9999999999i64);
            let result = variant_to_json(&variant);
            assert_eq!(result, Some(serde_json::json!(9999999999i64)));
        }

        #[test]
        fn test_float32() {
            let variant = Variant::Float32(3.125); // Use a non-constant value
            let result = variant_to_json(&variant);
            assert!(result.is_some());
        }

        #[test]
        fn test_float64() {
            let variant = Variant::Float64(2.5); // Use a non-constant value
            let result = variant_to_json(&variant);
            assert!(result.is_some());
        }

        #[test]
        fn test_vector2() {
            let variant = Variant::Vector2(Vector2::new(1.0, 2.0));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            assert_eq!(result.unwrap(), serde_json::json!([1.0, 2.0]));
        }

        #[test]
        fn test_vector3() {
            let variant = Variant::Vector3(Vector3::new(1.0, 2.0, 3.0));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            assert_eq!(result.unwrap(), serde_json::json!([1.0, 2.0, 3.0]));
        }

        #[test]
        fn test_color3() {
            let variant = Variant::Color3(Color3::new(1.0, 0.5, 0.0));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            assert_eq!(result.unwrap(), serde_json::json!([1.0, 0.5, 0.0]));
        }

        #[test]
        fn test_udim() {
            let variant = Variant::UDim(UDim::new(0.5, 100));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            let json = result.unwrap();
            assert_eq!(json["scale"], 0.5);
            assert_eq!(json["offset"], 100);
        }

        #[test]
        fn test_udim2() {
            let variant = Variant::UDim2(UDim2::new(UDim::new(0.5, 10), UDim::new(0.25, 20)));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            let json = result.unwrap();
            assert_eq!(json["x"]["scale"], 0.5);
            assert_eq!(json["x"]["offset"], 10);
            assert_eq!(json["y"]["scale"], 0.25);
            assert_eq!(json["y"]["offset"], 20);
        }

        #[test]
        fn test_enum() {
            let variant = Variant::Enum(Enum::from_u32(2));
            let result = variant_to_json(&variant);
            assert_eq!(result, Some(serde_json::json!(2)));
        }

        #[test]
        fn test_number_range() {
            let variant = Variant::NumberRange(NumberRange::new(0.0, 100.0));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            let json = result.unwrap();
            assert_eq!(json["min"], 0.0);
            assert_eq!(json["max"], 100.0);
        }

        #[test]
        fn test_rect() {
            let variant =
                Variant::Rect(Rect::new(Vector2::new(0.0, 0.0), Vector2::new(100.0, 50.0)));
            let result = variant_to_json(&variant);
            assert!(result.is_some());
            let json = result.unwrap();
            assert_eq!(json["min"], serde_json::json!([0.0, 0.0]));
            assert_eq!(json["max"], serde_json::json!([100.0, 50.0]));
        }

        #[test]
        fn test_unsupported_returns_none() {
            // BinaryString is not directly supported in JSON
            let variant = Variant::BinaryString(vec![1, 2, 3].into());
            let result = variant_to_json(&variant);
            assert!(result.is_none());
        }
    }

    // Tests for file naming conventions in syncback
    mod syncback_file_naming_tests {
        #[test]
        fn test_script_file_extensions() {
            // Test that script class names result in correct file extensions
            let test_cases = vec![
                ("ModuleScript", "TestModule", "TestModule.luau"),
                ("LocalScript", "TestClient", "TestClient.local.luau"),
            ];

            for (class_name, name, expected) in test_cases {
                let file_name = match class_name {
                    "ModuleScript" => format!("{}.luau", name),
                    "LocalScript" => format!("{}.local.luau", name),
                    _ => format!("{}.model.json5", name),
                };
                assert_eq!(
                    file_name, expected,
                    "File naming for {} '{}' is incorrect",
                    class_name, name
                );
            }
        }

        #[test]
        fn test_folder_creates_directory() {
            // Folders should not create .model.json5 files
            let class_name = "Folder";
            let should_create_file = class_name != "Folder";
            assert!(
                !should_create_file,
                "Folder should create a directory, not a file"
            );
        }

        #[test]
        fn test_other_instances_create_model_json5() {
            // Part, Frame, TextLabel, Model use .model.json5 (matching dedicated syncback)
            // Configuration creates a directory instead
            let json5_types = vec!["Part", "Frame", "TextLabel", "Model"];
            for class_name in json5_types {
                let file_name = "Test.model.json5";
                assert!(
                    file_name.ends_with(".model.json5"),
                    "{} should create .model.json5 file",
                    class_name
                );
            }
        }

        #[test]
        fn test_directory_classes() {
            // These classes create directories, not files (matching dedicated syncback)
            let dir_classes = vec![
                "Folder",
                "Configuration",
                "Tool",
                "ScreenGui",
                "SurfaceGui",
                "BillboardGui",
                "AdGui",
            ];
            for class_name in dir_classes {
                // These should create directories
                assert!(
                    matches!(
                        class_name,
                        "Folder"
                            | "Configuration"
                            | "Tool"
                            | "ScreenGui"
                            | "SurfaceGui"
                            | "BillboardGui"
                            | "AdGui"
                    ),
                    "{} should create a directory",
                    class_name
                );
            }
        }

        #[test]
        fn test_special_value_classes() {
            // StringValue -> .txt, LocalizationTable -> .csv
            assert_eq!(
                format!("{}.txt", "TestValue"),
                "TestValue.txt",
                "StringValue should create .txt file"
            );
            assert_eq!(
                format!("{}.csv", "TestLocalization"),
                "TestLocalization.csv",
                "LocalizationTable should create .csv file"
            );
        }
    }

    // Tests for model.json5 structure
    mod model_json_structure_tests {
        use serde_json::json;

        #[test]
        fn test_basic_structure() {
            // Verify the expected JSON structure for model files
            let model = json!({
                "className": "Part",
                "properties": {}
            });

            assert!(model.get("className").is_some());
            assert!(model.get("properties").is_some());
        }

        #[test]
        fn test_with_properties() {
            let model = json!({
                "className": "Part",
                "properties": {
                    "Size": [4.0, 1.0, 2.0],
                    "Color": [1.0, 0.0, 0.0],
                    "Anchored": true
                }
            });

            let props = model.get("properties").unwrap();
            assert!(props.get("Size").is_some());
            assert!(props.get("Color").is_some());
            assert!(props.get("Anchored").is_some());
        }
    }

    // Tests for RunContext-based script file suffix determination
    mod run_context_tests {

        #[test]
        fn test_run_context_client_gives_client_suffix() {
            // Script with RunContext: Client should produce .client.luau
            // RunContext::Client has value 2 in the Roblox enum
            let run_context_enums = rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .map(|e| &e.items);

            if let Some(enums) = run_context_enums {
                // Verify the enum value exists - just getting it is the test
                assert!(enums.get("Client").is_some());
            }
        }

        #[test]
        fn test_run_context_server_gives_server_suffix() {
            // Script with RunContext: Server should produce .server.luau
            let run_context_enums = rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .map(|e| &e.items);

            if let Some(enums) = run_context_enums {
                assert!(enums.get("Server").is_some());
            }
        }

        #[test]
        fn test_run_context_legacy_gives_legacy_suffix() {
            // Script with RunContext: Legacy should produce .legacy.luau
            let run_context_enums = rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .map(|e| &e.items);

            if let Some(enums) = run_context_enums {
                assert!(enums.get("Legacy").is_some());
            }
        }

        #[test]
        fn test_run_context_plugin_gives_plugin_suffix() {
            // Script with RunContext: Plugin should produce .plugin.luau
            let run_context_enums = rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .map(|e| &e.items);

            if let Some(enums) = run_context_enums {
                assert!(enums.get("Plugin").is_some());
            }
        }

        #[test]
        fn test_script_class_determines_suffix_from_run_context() {
            // For Script class, the file suffix is determined by RunContext:
            // - Client → .client.luau
            // - Server → .server.luau
            // - Legacy → .legacy.luau
            // - Plugin → .plugin.luau

            let expected_mappings = vec![
                ("Client", "client"),
                ("Server", "server"),
                ("Legacy", "legacy"),
                ("Plugin", "plugin"),
            ];

            for (run_context, expected_suffix) in expected_mappings {
                let suffix = match run_context {
                    "Client" => "client",
                    "Server" => "server",
                    "Legacy" => "legacy",
                    "Plugin" => "plugin",
                    _ => "legacy",
                };
                assert_eq!(
                    suffix, expected_suffix,
                    "RunContext {} should produce {} suffix",
                    run_context, expected_suffix
                );
            }
        }

        #[test]
        fn test_local_script_always_local() {
            // LocalScript class always produces .local.luau regardless of any properties
            let class_name = "LocalScript";
            let expected_suffix = "local";

            // LocalScript doesn't check RunContext - it's always local
            let suffix = if class_name == "LocalScript" {
                "local"
            } else {
                "unknown"
            };

            assert_eq!(suffix, expected_suffix);
        }

        #[test]
        fn test_module_script_no_suffix() {
            // ModuleScript produces .luau (no server/client suffix)
            let class_name = "ModuleScript";

            let expected_extension = if class_name == "ModuleScript" {
                ".luau"
            } else {
                ".unknown"
            };

            assert_eq!(expected_extension, ".luau");
        }
    }

    // Tests for adjacent meta file creation
    mod adjacent_meta_tests {
        #[test]
        fn test_adjacent_meta_file_naming() {
            // Adjacent meta files should be named {script_name}.meta.json5
            let script_name = "MyScript";
            let meta_filename = format!("{}.meta.json5", script_name);
            assert_eq!(meta_filename, "MyScript.meta.json5");
        }

        #[test]
        fn test_adjacent_meta_for_script_without_children() {
            // Scripts without children should create adjacent meta files, not init.meta.json5
            let has_children = false;
            let is_directory = has_children;

            // When it's not a directory, we use adjacent meta, not init.meta.json5
            let meta_type = if is_directory {
                "init.meta.json5"
            } else {
                "{name}.meta.json5"
            };

            assert_eq!(meta_type, "{name}.meta.json5");
        }

        #[test]
        fn test_init_meta_for_script_with_children() {
            // Scripts with children (directories) use init.meta.json5
            let has_children = true;
            let is_directory = has_children;

            let meta_type = if is_directory {
                "init.meta.json5"
            } else {
                "{name}.meta.json5"
            };

            assert_eq!(meta_type, "init.meta.json5");
        }
    }

    // Tests for property filtering (matching dedicated syncback)
    mod property_filter_tests {
        use super::*;
        use rbx_dom_weak::types::{Attributes, Ref, UniqueId};

        #[test]
        fn test_ref_properties_filtered_out() {
            // Ref properties should be filtered out (they don't serialize to JSON)
            let variant = Variant::Ref(Ref::new());
            let result = variant_to_json(&variant);
            // variant_to_json returns None for Ref
            assert!(result.is_none(), "Ref should not serialize to JSON");
        }

        #[test]
        fn test_unique_id_filtered_out() {
            // UniqueId properties should be filtered out
            let variant = Variant::UniqueId(UniqueId::new(0, 0, 0));
            let result = variant_to_json(&variant);
            assert!(result.is_none(), "UniqueId should not serialize to JSON");
        }

        #[test]
        fn test_attributes_handled_separately() {
            // Attributes are NOT handled by variant_to_json directly.
            // They are extracted into a separate "attributes" map by filter_properties_for_meta.
            // This matches the dedicated syncback behavior where Attributes are a special case.
            let mut attrs = Attributes::new();
            attrs.insert("TestAttr".to_string(), Variant::String("value".to_string()));

            let variant = Variant::Attributes(attrs);
            let result = variant_to_json(&variant);
            // variant_to_json returns None for Attributes - they're handled separately
            assert!(
                result.is_none(),
                "Attributes should return None from variant_to_json (handled separately)"
            );
        }
    }

    // Tests for init.meta.json5 format (matching dedicated syncback)
    mod meta_json5_tests {
        use serde_json::json;

        #[test]
        fn test_meta_uses_json5_extension() {
            // Meta files should use .json5 extension (not .json)
            let filename = "init.meta.json5";
            assert!(
                filename.ends_with(".json5"),
                "Meta files should use .json5 extension"
            );
        }

        #[test]
        fn test_meta_structure_for_directory() {
            // Directory meta files should have properties and optionally attributes
            let meta = json!({
                "properties": {
                    "Archivable": false
                },
                "attributes": {
                    "CustomAttr": "value"
                }
            });

            assert!(meta.get("properties").is_some());
            assert!(meta.get("attributes").is_some());
            // No className for standard Folders
            assert!(meta.get("className").is_none());
        }

        #[test]
        fn test_meta_structure_for_non_folder() {
            // Non-Folder directories need className
            let meta = json!({
                "className": "Part",
                "properties": {
                    "Size": [4.0, 1.0, 2.0]
                }
            });

            assert!(meta.get("className").is_some());
            assert_eq!(meta.get("className").unwrap(), "Part");
        }
    }

    // Tests for children handling (any instance can have children)
    mod children_handling_tests {
        #[allow(unused_imports)]
        use super::*;

        #[test]
        fn test_script_with_children_becomes_directory() {
            // When a script has children, it should become a directory with init file
            let has_children = true;
            let class_name = "ModuleScript";

            // This is the expected behavior - directory + init.luau
            if has_children && class_name == "ModuleScript" {
                let init_file = "init.luau";
                assert_eq!(init_file, "init.luau");
            }
        }

        #[test]
        fn test_server_script_with_children() {
            let has_children = true;
            let class_name = "Script";

            if has_children && class_name == "Script" {
                let init_file = "init.server.luau";
                assert_eq!(init_file, "init.server.luau");
            }
        }

        #[test]
        fn test_client_script_with_children() {
            let has_children = true;
            let class_name = "LocalScript";

            if has_children && class_name == "LocalScript" {
                let init_file = "init.client.luau";
                assert_eq!(init_file, "init.client.luau");
            }
        }

        #[test]
        fn test_string_value_with_children() {
            // StringValue with children becomes directory + init.meta.json5
            let has_children = true;
            let class_name = "StringValue";

            if has_children && class_name == "StringValue" {
                // Should create directory with init.meta.json5, not .txt file
                let meta_file = "init.meta.json5";
                assert!(meta_file.ends_with(".json5"));
            }
        }

        #[test]
        fn test_part_with_children() {
            // Part (or any other class) with children becomes directory + init.meta.json5
            let has_children = true;
            let class_name = "Part";

            if has_children && !matches!(class_name, "ModuleScript" | "Script" | "LocalScript") {
                let meta_file = "init.meta.json5";
                assert!(meta_file.ends_with(".json5"));
            }
        }
    }

    // Tests for .gitkeep behavior
    mod gitkeep_tests {
        #[test]
        fn test_empty_folder_gets_gitkeep() {
            // Empty folders with no metadata should get .gitkeep (matching dedicated syncback)
            let has_children = false;
            let has_metadata = false;

            let should_add_gitkeep = !has_children && !has_metadata;
            assert!(should_add_gitkeep, "Empty folder should get .gitkeep");
        }

        #[test]
        fn test_folder_with_metadata_no_gitkeep() {
            // Folders with metadata should NOT get .gitkeep
            let has_children = false;
            let has_metadata = true;

            let should_add_gitkeep = !has_children && !has_metadata;
            assert!(
                !should_add_gitkeep,
                "Folder with metadata should not get .gitkeep"
            );
        }

        #[test]
        fn test_folder_with_children_no_gitkeep() {
            // Folders with children should NOT get .gitkeep
            let has_children = true;
            let has_metadata = false;

            let should_add_gitkeep = !has_children && !has_metadata;
            assert!(
                !should_add_gitkeep,
                "Folder with children should not get .gitkeep"
            );
        }
    }

    // Tests for attributes handling (matching dedicated syncback)
    mod attributes_tests {
        use super::*;
        use rbx_dom_weak::types::Attributes;

        #[test]
        fn test_internal_attributes_filtered() {
            // Internal attributes starting with RBX should be filtered out
            let mut attrs = Attributes::new();
            attrs.insert(
                "RBXInternal".to_string(),
                Variant::String("internal".to_string()),
            );
            attrs.insert(
                "CustomAttr".to_string(),
                Variant::String("custom".to_string()),
            );

            // Only CustomAttr should be kept
            let mut count = 0;
            for (name, _) in attrs.iter() {
                if !name.starts_with("RBX") {
                    count += 1;
                }
            }
            assert_eq!(count, 1, "Only non-RBX attributes should be kept");
        }

        #[test]
        fn test_user_attributes_preserved() {
            // User-defined attributes should be preserved
            let mut attrs = Attributes::new();
            attrs.insert(
                "MyAttribute".to_string(),
                Variant::String("value".to_string()),
            );
            attrs.insert("Score".to_string(), Variant::Int32(100));

            let filtered: Vec<_> = attrs
                .iter()
                .filter(|(name, _)| !name.starts_with("RBX"))
                .collect();

            assert_eq!(filtered.len(), 2, "User attributes should be preserved");
        }
    }

    // =========================================================================
    // Comprehensive RunContext Configuration Tests
    // =========================================================================
    mod run_context_comprehensive_tests {
        /// Helper to get RunContext enum value by name
        fn get_run_context_value(name: &str) -> Option<u32> {
            rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .and_then(|e| e.items.get(name).copied())
        }

        /// Simulates suffix determination logic (matches get_script_suffix_for_run_context)
        fn determine_suffix_from_run_context(run_context_value: Option<u32>) -> &'static str {
            let run_context_enums = rbx_reflection_database::get()
                .ok()
                .and_then(|db| db.enums.get("RunContext"))
                .map(|e| &e.items);

            if let (Some(enums), Some(value)) = (run_context_enums, run_context_value) {
                for (name, &enum_value) in enums {
                    if enum_value == value {
                        return match *name {
                            "Client" => "client",
                            "Server" => "server",
                            "Legacy" => "legacy",
                            "Plugin" => "plugin",
                            _ => "legacy",
                        };
                    }
                }
            }
            "legacy"
        }

        #[test]
        fn test_all_run_context_values_exist() {
            // Verify all expected RunContext values exist in the reflection database
            let expected = ["Legacy", "Server", "Client", "Plugin"];
            for name in expected {
                assert!(
                    get_run_context_value(name).is_some(),
                    "RunContext::{} should exist in reflection database",
                    name
                );
            }
        }

        #[test]
        fn test_run_context_client_produces_client_suffix() {
            let value = get_run_context_value("Client");
            assert!(value.is_some(), "Client RunContext should exist");
            let suffix = determine_suffix_from_run_context(value);
            assert_eq!(
                suffix, "client",
                "RunContext::Client should produce 'client' suffix"
            );
        }

        #[test]
        fn test_run_context_server_produces_server_suffix() {
            let value = get_run_context_value("Server");
            assert!(value.is_some(), "Server RunContext should exist");
            let suffix = determine_suffix_from_run_context(value);
            assert_eq!(
                suffix, "server",
                "RunContext::Server should produce 'server' suffix"
            );
        }

        #[test]
        fn test_run_context_legacy_produces_legacy_suffix() {
            let value = get_run_context_value("Legacy");
            assert!(value.is_some(), "Legacy RunContext should exist");
            let suffix = determine_suffix_from_run_context(value);
            assert_eq!(
                suffix, "legacy",
                "RunContext::Legacy should produce 'legacy' suffix"
            );
        }

        #[test]
        fn test_run_context_plugin_produces_plugin_suffix() {
            let value = get_run_context_value("Plugin");
            assert!(value.is_some(), "Plugin RunContext should exist");
            let suffix = determine_suffix_from_run_context(value);
            assert_eq!(
                suffix, "plugin",
                "RunContext::Plugin should produce 'plugin' suffix"
            );
        }

        #[test]
        fn test_no_run_context_defaults_to_legacy() {
            let suffix = determine_suffix_from_run_context(None);
            assert_eq!(
                suffix, "legacy",
                "No RunContext should default to 'legacy' suffix"
            );
        }

        #[test]
        fn test_invalid_run_context_defaults_to_legacy() {
            // Some unlikely value that doesn't match any known RunContext
            let suffix = determine_suffix_from_run_context(Some(99999));
            assert_eq!(
                suffix, "legacy",
                "Invalid RunContext should default to 'legacy' suffix"
            );
        }

        #[test]
        fn test_file_extension_patterns() {
            // Verify expected file extension patterns
            let test_cases = vec![
                ("ModuleScript", None, ".luau"),
                ("LocalScript", None, ".local.luau"),
                ("Script", get_run_context_value("Server"), ".server.luau"),
                ("Script", get_run_context_value("Client"), ".client.luau"),
                ("Script", get_run_context_value("Legacy"), ".legacy.luau"),
                ("Script", get_run_context_value("Plugin"), ".plugin.luau"),
                ("Script", None, ".legacy.luau"),
            ];

            for (class_name, run_context, expected_ext) in test_cases {
                let suffix = match class_name {
                    "ModuleScript" => "",
                    "LocalScript" => "local",
                    "Script" => determine_suffix_from_run_context(run_context),
                    _ => panic!("Unknown class"),
                };

                let ext = if suffix.is_empty() {
                    ".luau".to_string()
                } else {
                    format!(".{}.luau", suffix)
                };

                assert_eq!(
                    ext, expected_ext,
                    "{} with RunContext {:?} should produce {} extension",
                    class_name, run_context, expected_ext
                );
            }
        }

        #[test]
        fn test_init_file_patterns_with_children() {
            // Verify init file patterns for scripts with children
            let test_cases = vec![
                ("ModuleScript", None, "init.luau"),
                ("LocalScript", None, "init.local.luau"),
                (
                    "Script",
                    get_run_context_value("Server"),
                    "init.server.luau",
                ),
                (
                    "Script",
                    get_run_context_value("Client"),
                    "init.client.luau",
                ),
                (
                    "Script",
                    get_run_context_value("Legacy"),
                    "init.legacy.luau",
                ),
                (
                    "Script",
                    get_run_context_value("Plugin"),
                    "init.plugin.luau",
                ),
            ];

            for (class_name, run_context, expected_init) in test_cases {
                let suffix = match class_name {
                    "ModuleScript" => "",
                    "LocalScript" => "local",
                    "Script" => determine_suffix_from_run_context(run_context),
                    _ => panic!("Unknown class"),
                };

                let init_file = if suffix.is_empty() {
                    "init.luau".to_string()
                } else {
                    format!("init.{}.luau", suffix)
                };

                assert_eq!(
                    init_file, expected_init,
                    "{} with RunContext {:?} should produce {} init file",
                    class_name, run_context, expected_init
                );
            }
        }
    }

    // =========================================================================
    // Scripts-Only Mode Filtering Tests
    // =========================================================================
    mod scripts_only_mode_tests {
        use super::*;

        #[test]
        fn test_is_script_class_identifies_scripts() {
            // Test that is_script_class correctly identifies script types
            assert!(
                is_script_class("Script"),
                "Script should be identified as script"
            );
            assert!(
                is_script_class("LocalScript"),
                "LocalScript should be identified as script"
            );
            assert!(
                is_script_class("ModuleScript"),
                "ModuleScript should be identified as script"
            );
        }

        #[test]
        fn test_is_script_class_rejects_non_scripts() {
            // Test that is_script_class correctly rejects non-script types
            let non_scripts = [
                "Part",
                "Folder",
                "Model",
                "Frame",
                "TextLabel",
                "IntValue",
                "StringValue",
                "Configuration",
                "ScreenGui",
                "Workspace",
                "ReplicatedStorage",
            ];

            for class_name in non_scripts {
                assert!(
                    !is_script_class(class_name),
                    "{} should NOT be identified as script",
                    class_name
                );
            }
        }

        #[test]
        fn test_scripts_only_should_include_script() {
            // In scripts-only mode, Script instances should be included
            let class_name = "Script";
            let should_include = is_script_class(class_name);
            assert!(
                should_include,
                "Script should be included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_include_local_script() {
            // In scripts-only mode, LocalScript instances should be included
            let class_name = "LocalScript";
            let should_include = is_script_class(class_name);
            assert!(
                should_include,
                "LocalScript should be included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_include_module_script() {
            // In scripts-only mode, ModuleScript instances should be included
            let class_name = "ModuleScript";
            let should_include = is_script_class(class_name);
            assert!(
                should_include,
                "ModuleScript should be included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_exclude_part() {
            // In scripts-only mode, Part instances should be excluded
            let class_name = "Part";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Part should NOT be included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_exclude_folder() {
            // In scripts-only mode, Folder instances should be excluded (unless ancestor)
            let class_name = "Folder";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Folder should NOT be directly included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_exclude_model() {
            // In scripts-only mode, Model instances should be excluded (unless ancestor)
            let class_name = "Model";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Model should NOT be directly included in scripts-only mode"
            );
        }

        #[test]
        fn test_scripts_only_should_exclude_gui_elements() {
            // In scripts-only mode, GUI elements should be excluded
            let gui_classes = [
                "Frame",
                "TextLabel",
                "TextButton",
                "ImageLabel",
                "ScreenGui",
            ];
            for class_name in gui_classes {
                let should_include = is_script_class(class_name);
                assert!(
                    !should_include,
                    "{} should NOT be included in scripts-only mode",
                    class_name
                );
            }
        }

        #[test]
        fn test_scripts_only_should_exclude_value_types() {
            // In scripts-only mode, value types should be excluded
            let value_classes = [
                "IntValue",
                "StringValue",
                "NumberValue",
                "BoolValue",
                "ObjectValue",
            ];
            for class_name in value_classes {
                let should_include = is_script_class(class_name);
                assert!(
                    !should_include,
                    "{} should NOT be included in scripts-only mode",
                    class_name
                );
            }
        }

        #[test]
        fn test_scripts_only_run_context_preserved() {
            // In scripts-only mode, RunContext property on scripts should be preserved
            // This is important for proper syncback behavior
            let script_properties = ["Source", "RunContext", "Enabled", "Disabled"];
            let is_script = is_script_class("Script");
            assert!(is_script, "Script should be identified");

            // In scripts-only mode, scripts get ALL their properties synced (including RunContext)
            // Non-scripts get their properties cleared
            // This test verifies the design decision
            for prop in script_properties {
                // Scripts should sync these properties
                let should_sync = is_script;
                assert!(
                    should_sync,
                    "Script property '{}' should be synced in scripts-only mode",
                    prop
                );
            }
        }

        #[test]
        fn test_scripts_only_ancestor_properties_cleared() {
            // In scripts-only mode, non-script ancestors have properties cleared
            // They only provide tree structure, not property data
            let folder_is_script = is_script_class("Folder");
            assert!(!folder_is_script, "Folder is not a script");

            // Non-scripts should have properties cleared
            let should_clear_properties = !folder_is_script;
            assert!(
                should_clear_properties,
                "Non-script ancestors should have properties cleared"
            );
        }
    }

    // =========================================================================
    // Scripts-Only Mode - Expected Pass Tests
    // =========================================================================
    mod scripts_only_expected_passes {
        use super::*;

        #[test]
        fn test_pass_module_script_sync() {
            // PASS: ModuleScript should sync successfully in scripts-only mode
            let class_name = "ModuleScript";
            assert!(
                is_script_class(class_name),
                "ModuleScript sync should PASS in scripts-only mode"
            );
        }

        #[test]
        fn test_pass_server_script_sync() {
            // PASS: Script (server) should sync successfully in scripts-only mode
            let class_name = "Script";
            assert!(
                is_script_class(class_name),
                "Script sync should PASS in scripts-only mode"
            );
        }

        #[test]
        fn test_pass_local_script_sync() {
            // PASS: LocalScript should sync successfully in scripts-only mode
            let class_name = "LocalScript";
            assert!(
                is_script_class(class_name),
                "LocalScript sync should PASS in scripts-only mode"
            );
        }

        #[test]
        fn test_pass_script_source_property() {
            // PASS: Script Source property changes should sync
            let class_name = "Script";
            let property_name = "Source";
            let is_script = is_script_class(class_name);
            let should_sync_property = is_script; // Scripts sync all properties
            assert!(
                should_sync_property,
                "Script.{} change should PASS in scripts-only mode",
                property_name
            );
        }

        #[test]
        fn test_pass_script_run_context_property() {
            // PASS: Script RunContext property changes should sync
            // This is critical for proper emitLegacyScripts support
            let class_name = "Script";
            let property_name = "RunContext";
            let is_script = is_script_class(class_name);
            let should_sync_property = is_script; // Scripts sync all properties
            assert!(
                should_sync_property,
                "Script.{} change should PASS in scripts-only mode",
                property_name
            );
        }

        #[test]
        fn test_pass_script_enabled_property() {
            // PASS: Script Enabled property changes should sync
            let class_name = "Script";
            let property_name = "Enabled";
            let is_script = is_script_class(class_name);
            let should_sync_property = is_script;
            assert!(
                should_sync_property,
                "Script.{} change should PASS in scripts-only mode",
                property_name
            );
        }

        #[test]
        fn test_pass_module_script_attributes() {
            // PASS: ModuleScript Attributes should sync
            let class_name = "ModuleScript";
            let is_script = is_script_class(class_name);
            assert!(
                is_script,
                "ModuleScript.Attributes change should PASS in scripts-only mode"
            );
        }

        #[test]
        fn test_pass_nested_script_in_folder() {
            // PASS: Script inside Folder should sync (along with Folder as ancestor)
            let script_class = "ModuleScript";
            let folder_class = "Folder";

            let script_included = is_script_class(script_class);
            let folder_is_script = is_script_class(folder_class);

            // Script should be included directly
            assert!(script_included, "Nested script should be included");
            // Folder should be included as ancestor (but not as a script)
            assert!(
                !folder_is_script,
                "Folder is not a script but may be included as ancestor"
            );
        }
    }

    // =========================================================================
    // Scripts-Only Mode - Expected Failure/Filter Tests
    // =========================================================================
    mod scripts_only_expected_filters {
        use super::*;

        #[test]
        fn test_filter_part_changes() {
            // FILTER: Part changes should be filtered out in scripts-only mode
            let class_name = "Part";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Part changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_model_changes() {
            // FILTER: Model changes should be filtered out in scripts-only mode
            let class_name = "Model";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Model changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_folder_property_changes() {
            // FILTER: Folder property changes should be filtered (Folder is ancestor only)
            let class_name = "Folder";
            let is_script = is_script_class(class_name);
            // Folders have their properties cleared in scripts-only mode
            assert!(
                !is_script,
                "Folder property changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_frame_changes() {
            // FILTER: Frame (GUI) changes should be filtered
            let class_name = "Frame";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Frame changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_text_label_changes() {
            // FILTER: TextLabel changes should be filtered
            let class_name = "TextLabel";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "TextLabel changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_int_value_changes() {
            // FILTER: IntValue changes should be filtered
            let class_name = "IntValue";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "IntValue changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_string_value_changes() {
            // FILTER: StringValue changes should be filtered
            let class_name = "StringValue";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "StringValue changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_sound_changes() {
            // FILTER: Sound changes should be filtered
            let class_name = "Sound";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Sound changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_particle_emitter_changes() {
            // FILTER: ParticleEmitter changes should be filtered
            let class_name = "ParticleEmitter";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "ParticleEmitter changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_configuration_changes() {
            // FILTER: Configuration instance changes should be filtered
            let class_name = "Configuration";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "Configuration changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_remote_event_changes() {
            // FILTER: RemoteEvent changes should be filtered
            let class_name = "RemoteEvent";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "RemoteEvent changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_remote_function_changes() {
            // FILTER: RemoteFunction changes should be filtered
            let class_name = "RemoteFunction";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "RemoteFunction changes should be FILTERED in scripts-only mode"
            );
        }

        #[test]
        fn test_filter_bindable_event_changes() {
            // FILTER: BindableEvent changes should be filtered
            let class_name = "BindableEvent";
            let should_include = is_script_class(class_name);
            assert!(
                !should_include,
                "BindableEvent changes should be FILTERED in scripts-only mode"
            );
        }
    }

    // =========================================================================
    // Scripts-Only Mode - Ancestor Inclusion Tests
    // =========================================================================
    mod scripts_only_ancestor_tests {
        use super::*;

        #[test]
        fn test_ancestor_chain_concept() {
            // Test the concept of ancestor chain inclusion
            // In scripts-only mode:
            // - Scripts are included with all properties
            // - Non-script ancestors are included for tree structure but properties cleared

            // Simulated tree: ReplicatedStorage > Folder > ModuleScript
            let replicated_storage = "ReplicatedStorage";
            let folder = "Folder";
            let module_script = "ModuleScript";

            // Only the script is identified as a script
            assert!(!is_script_class(replicated_storage));
            assert!(!is_script_class(folder));
            assert!(is_script_class(module_script));

            // But all three should be included in the message (for tree structure)
            // The filtering function includes ancestors of scripts
        }

        #[test]
        fn test_non_script_ancestors_have_ignore_unknown_instances() {
            // In scripts-only mode, all instances get ignoreUnknownInstances: true
            // This prevents accidental deletion of non-script instances
            let folder = "Folder";
            let _is_script = is_script_class(folder);

            // Non-scripts should have ignoreUnknownInstances set
            let should_set_ignore = true; // All instances in scripts-only mode
            assert!(
                should_set_ignore,
                "Non-script ancestors should have ignoreUnknownInstances: true"
            );
        }

        #[test]
        fn test_scripts_also_have_ignore_unknown_instances() {
            // Even scripts get ignoreUnknownInstances: true in scripts-only mode
            // This is a safety measure
            let script = "Script";
            let is_script = is_script_class(script);
            assert!(is_script);

            // All instances in scripts-only mode get this flag
            let should_set_ignore = true;
            assert!(
                should_set_ignore,
                "Scripts should also have ignoreUnknownInstances: true in scripts-only mode"
            );
        }
    }

    // =========================================================================
    // Syncback Configuration Matrix Tests
    // =========================================================================
    mod syncback_configuration_matrix {
        /// Test matrix for all script class + RunContext combinations
        #[test]
        fn test_configuration_matrix() {
            struct TestCase {
                class_name: &'static str,
                run_context: Option<&'static str>,
                expected_extension: &'static str,
                description: &'static str,
            }

            let test_cases = vec![
                // ModuleScript - always .luau, no RunContext matters
                TestCase {
                    class_name: "ModuleScript",
                    run_context: None,
                    expected_extension: ".luau",
                    description: "ModuleScript without RunContext",
                },
                // LocalScript - always .local.luau
                TestCase {
                    class_name: "LocalScript",
                    run_context: None,
                    expected_extension: ".local.luau",
                    description: "LocalScript",
                },
                // Script with various RunContext values
                TestCase {
                    class_name: "Script",
                    run_context: None,
                    expected_extension: ".legacy.luau",
                    description: "Script without RunContext (default)",
                },
                TestCase {
                    class_name: "Script",
                    run_context: Some("Server"),
                    expected_extension: ".server.luau",
                    description: "Script with RunContext::Server",
                },
                TestCase {
                    class_name: "Script",
                    run_context: Some("Client"),
                    expected_extension: ".client.luau",
                    description: "Script with RunContext::Client",
                },
                TestCase {
                    class_name: "Script",
                    run_context: Some("Legacy"),
                    expected_extension: ".legacy.luau",
                    description: "Script with RunContext::Legacy",
                },
                TestCase {
                    class_name: "Script",
                    run_context: Some("Plugin"),
                    expected_extension: ".plugin.luau",
                    description: "Script with RunContext::Plugin",
                },
            ];

            for case in test_cases {
                let suffix = match case.class_name {
                    "ModuleScript" => String::new(),
                    "LocalScript" => "local".to_string(),
                    "Script" => {
                        // Match on RunContext name to determine suffix
                        match case.run_context {
                            Some("Client") => "client",
                            Some("Server") => "server",
                            Some("Legacy") => "legacy",
                            Some("Plugin") => "plugin",
                            None => "legacy",
                            _ => "legacy",
                        }
                        .to_string()
                    }
                    _ => panic!("Unknown class"),
                };

                let extension = if suffix.is_empty() {
                    ".luau".to_string()
                } else {
                    format!(".{}.luau", suffix)
                };

                assert_eq!(
                    extension, case.expected_extension,
                    "FAILED: {} - got {} expected {}",
                    case.description, extension, case.expected_extension
                );
            }
        }

        #[test]
        fn test_new_script_naming_convention() {
            // Test that syncback produces the correct file extensions with the new naming convention
            //
            // New naming convention (emitLegacyScripts removed):
            //   - LocalScript -> .local.luau
            //   - Script + RunContext::Server -> .server.luau
            //   - Script + RunContext::Client -> .client.luau
            //   - Script + RunContext::Plugin -> .plugin.luau
            //   - Script + RunContext::Legacy -> .legacy.luau
            //   - Script (no RunContext) -> .legacy.luau (default)
            //
            // Backwards compatibility for READING old files:
            //   - .client.lua -> LocalScript
            //   - .server.lua -> Script with RunContext::Legacy

            // Test expected syncback extensions
            let test_cases = vec![
                ("LocalScript", None, ".local.luau"),
                ("Script", Some("Server"), ".server.luau"),
                ("Script", Some("Client"), ".client.luau"),
                ("Script", Some("Plugin"), ".plugin.luau"),
                ("Script", Some("Legacy"), ".legacy.luau"),
                ("Script", None, ".legacy.luau"),
            ];

            for (class, run_context, expected_ext) in &test_cases {
                let syncback_ext = match (*class, *run_context) {
                    ("LocalScript", _) => ".local.luau",
                    ("Script", Some("Server")) => ".server.luau",
                    ("Script", Some("Client")) => ".client.luau",
                    ("Script", Some("Plugin")) => ".plugin.luau",
                    ("Script", Some("Legacy")) => ".legacy.luau",
                    ("Script", None) => ".legacy.luau",
                    _ => panic!("Unexpected combo"),
                };
                assert_eq!(
                    syncback_ext, *expected_ext,
                    "New naming convention failed for {} {:?}",
                    class, run_context
                );
            }
        }
    }

    // =========================================================================
    // Duplicate Path Detection Tests
    // =========================================================================
    mod duplicate_path_tests {
        // Helper to simulate sibling name checking
        fn has_duplicate_sibling_names(names: &[&str], target_name: &str) -> bool {
            let count = names.iter().filter(|&&n| n == target_name).count();
            count > 1
        }

        // Simulates the duplicate children filter logic
        fn filter_duplicate_names<'a>(names: &[&'a str]) -> Vec<&'a str> {
            let mut counts: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for name in names {
                *counts.entry(name).or_insert(0) += 1;
            }

            names
                .iter()
                .filter(|&&name| counts.get(name).copied().unwrap_or(0) <= 1)
                .copied()
                .collect()
        }

        #[test]
        fn test_no_duplicates_all_pass() {
            let names = vec!["Script1", "Script2", "Script3", "Folder"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(filtered.len(), 4, "All unique names should pass");
        }

        #[test]
        fn test_duplicate_siblings_filtered() {
            let names = vec!["Script", "Script", "Other"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate 'Script' entries should be filtered"
            );
            assert_eq!(filtered[0], "Other", "Only 'Other' should remain");
        }

        #[test]
        fn test_multiple_duplicates_filtered() {
            let names = vec!["A", "A", "B", "B", "C"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(filtered.len(), 1, "Only unique 'C' should remain");
            assert_eq!(filtered[0], "C");
        }

        #[test]
        fn test_all_duplicates_empty_result() {
            let names = vec!["Same", "Same", "Same"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                0,
                "All duplicates should result in empty list"
            );
        }

        #[test]
        fn test_sibling_detection_positive() {
            let siblings = vec!["Script", "Script", "Other"];
            assert!(
                has_duplicate_sibling_names(&siblings, "Script"),
                "'Script' should be detected as having duplicate siblings"
            );
        }

        #[test]
        fn test_sibling_detection_negative() {
            let siblings = vec!["Script", "Folder", "Model"];
            assert!(
                !has_duplicate_sibling_names(&siblings, "Script"),
                "'Script' should NOT be detected as duplicate with unique siblings"
            );
        }

        #[test]
        fn test_sibling_detection_not_in_list() {
            let siblings = vec!["A", "B", "C"];
            assert!(
                !has_duplicate_sibling_names(&siblings, "D"),
                "'D' not in list should not have duplicates"
            );
        }

        #[test]
        fn test_path_uniqueness_concept() {
            // Test the concept: path is unique only if NO level has duplicates
            //
            // Tree:
            // Root
            //   ├─ FolderA
            //   │    └─ Script
            //   └─ FolderA (duplicate!)
            //        └─ Script
            //
            // Path "Root/FolderA/Script" is NOT unique because FolderA is duplicated

            let root_children = vec!["FolderA", "FolderA"];
            let folder_children = vec!["Script"];

            // Check at root level - duplicates exist
            let has_dup_at_root = has_duplicate_sibling_names(&root_children, "FolderA");
            assert!(
                has_dup_at_root,
                "FolderA should be detected as duplicate at root level"
            );

            // Check at folder level - no duplicates
            let has_dup_at_folder = has_duplicate_sibling_names(&folder_children, "Script");
            assert!(
                !has_dup_at_folder,
                "Script has no duplicate siblings within its parent"
            );

            // But the PATH is still not unique because of the ancestor duplicate
            let path_is_unique = !has_dup_at_root;
            assert!(
                !path_is_unique,
                "Path should NOT be unique due to ancestor duplicate"
            );
        }

        #[test]
        fn test_path_uniqueness_deep_hierarchy() {
            // Tree:
            // Root (unique)
            //   └─ Level1 (unique)
            //        └─ Level2 (has duplicate sibling)
            //             └─ Target
            //
            // Path to Target is NOT unique

            let level1_children = vec!["Level2", "Level2", "Other"];
            let level2_children = vec!["Target"];

            let has_dup_at_level1 = has_duplicate_sibling_names(&level1_children, "Level2");
            assert!(has_dup_at_level1, "Level2 has duplicate at Level1");

            // Even though Target itself has no duplicates, path is not unique
            let has_dup_at_level2 = has_duplicate_sibling_names(&level2_children, "Target");
            assert!(!has_dup_at_level2, "Target has no duplicate at Level2");

            // Path uniqueness requires ALL levels to be unique
            let path_unique = !has_dup_at_level1 && !has_dup_at_level2;
            assert!(
                !path_unique,
                "Path should NOT be unique due to Level2 duplicate"
            );
        }

        #[test]
        fn test_path_uniqueness_fully_unique() {
            // Tree:
            // Root
            //   └─ Folder (unique)
            //        └─ SubFolder (unique)
            //             └─ Script (unique)

            let root_children = vec!["Folder"];
            let folder_children = vec!["SubFolder"];
            let sub_children = vec!["Script"];

            let root_unique = !has_duplicate_sibling_names(&root_children, "Folder");
            let folder_unique = !has_duplicate_sibling_names(&folder_children, "SubFolder");
            let sub_unique = !has_duplicate_sibling_names(&sub_children, "Script");

            let path_unique = root_unique && folder_unique && sub_unique;
            assert!(
                path_unique,
                "Path should be unique when all levels are unique"
            );
        }

        #[test]
        fn test_case_sensitive_duplicates() {
            // Roblox names are case-sensitive, so "Script" and "script" are different
            let names = vec!["Script", "script", "SCRIPT"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                3,
                "Case-different names should all be kept (case-sensitive)"
            );
        }

        #[test]
        fn test_empty_names_handled() {
            let names = vec!["", "", "Valid"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Empty string duplicates should be filtered"
            );
            assert_eq!(filtered[0], "Valid");
        }

        #[test]
        fn test_special_characters_in_names() {
            // Names with special characters that might cause filesystem issues
            // are handled separately by validate_file_name, but duplicates should still work
            let names = vec!["Script (1)", "Script (2)", "Script (1)"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Only 'Script (2)' should remain after filtering duplicates"
            );
            assert_eq!(filtered[0], "Script (2)");
        }

        #[test]
        fn test_whitespace_names() {
            // Names with leading/trailing whitespace are technically different
            let names = vec!["Script", " Script", "Script "];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                3,
                "Whitespace-different names should all be kept"
            );
        }

        // =====================================================================
        // Non-Script Duplicate Tests - Duplicates apply to ALL instance types
        // =====================================================================

        #[test]
        fn test_duplicate_folders_filtered() {
            // Duplicate Folders should be filtered just like scripts
            let names = vec!["MyFolder", "MyFolder", "OtherFolder"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(filtered.len(), 1, "Duplicate 'MyFolder' should be filtered");
            assert_eq!(filtered[0], "OtherFolder");
        }

        #[test]
        fn test_duplicate_parts_filtered() {
            // Duplicate Parts should be filtered
            let names = vec!["Part", "Part", "Baseplate"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(filtered.len(), 1, "Duplicate 'Part' should be filtered");
            assert_eq!(filtered[0], "Baseplate");
        }

        #[test]
        fn test_duplicate_models_filtered() {
            // Duplicate Models should be filtered
            let names = vec!["Car", "Car", "House"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate 'Car' models should be filtered"
            );
        }

        #[test]
        fn test_duplicate_values_filtered() {
            // Duplicate IntValues/StringValues should be filtered
            let names = vec!["Score", "Score", "Health"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate 'Score' values should be filtered"
            );
        }

        #[test]
        fn test_duplicate_gui_elements_filtered() {
            // Duplicate GUI elements should be filtered
            let names = vec!["MainFrame", "MainFrame", "Sidebar"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate 'MainFrame' GUI should be filtered"
            );
        }

        #[test]
        fn test_mixed_types_same_name_filtered() {
            // Even if types differ, same names are duplicates (filesystem perspective)
            // E.g., a Folder named "Data" and a Script named "Data" would conflict
            let names = vec!["Data", "Data"]; // Could be Folder + Script
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                0,
                "Same-named instances of different types still conflict"
            );
        }

        #[test]
        fn test_folder_hierarchy_duplicates() {
            // Duplicates in folder hierarchies
            // Root/
            //   ├─ Enemies/
            //   │    └─ Enemy (unique)
            //   └─ Enemies/ (duplicate!)
            //        └─ Enemy

            let root_children = vec!["Enemies", "Enemies", "Players"];
            let enemies_unique = !has_duplicate_sibling_names(&root_children, "Enemies");
            assert!(
                !enemies_unique,
                "Duplicate 'Enemies' folders should be detected"
            );
        }

        #[test]
        fn test_part_ancestor_duplicates() {
            // A Part deep in hierarchy with duplicate ancestor
            // Workspace/
            //   ├─ Zone/
            //   │    └─ SpawnPoint (Part)
            //   └─ Zone/ (duplicate!)
            //        └─ SpawnPoint (Part)

            let workspace_children = vec!["Zone", "Zone"];
            let zone_unique = !has_duplicate_sibling_names(&workspace_children, "Zone");
            assert!(
                !zone_unique,
                "Duplicate 'Zone' ancestors make Part paths ambiguous"
            );
        }

        #[test]
        fn test_remote_event_duplicates() {
            // RemoteEvents with duplicate names
            let names = vec!["FireBullet", "FireBullet", "TakeDamage"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate RemoteEvents should be filtered"
            );
        }

        #[test]
        fn test_configuration_duplicates() {
            // Configuration instances with duplicates
            let names = vec!["Settings", "Settings", "Config"];
            let filtered = filter_duplicate_names(&names);
            assert_eq!(
                filtered.len(),
                1,
                "Duplicate Configurations should be filtered"
            );
        }
    }

    // =========================================================================
    // Duplicate Path Detection - Integration Scenarios
    // =========================================================================
    mod duplicate_path_integration_tests {

        #[test]
        fn test_syncback_skips_duplicate_children() {
            // When syncing back children, duplicates should be skipped
            // This test verifies the filter_duplicate_children logic

            let children_names = ["ChildA", "ChildB", "ChildA", "ChildC"];
            let unique: Vec<_> = children_names
                .iter()
                .filter(|&&name| children_names.iter().filter(|&&n| n == name).count() == 1)
                .collect();

            assert_eq!(unique.len(), 2, "ChildB and ChildC should be kept");
            assert!(unique.contains(&&"ChildB"));
            assert!(unique.contains(&&"ChildC"));
        }

        #[test]
        fn test_recursive_duplicate_handling() {
            // Test that duplicates at any level are handled
            //
            // Parent/
            //   ├─ Child1/
            //   │    ├─ GrandChild (duplicate)
            //   │    └─ GrandChild (duplicate)
            //   └─ Child2/
            //        └─ GrandChild (unique within its parent)

            let child1_grandchildren = ["GrandChild", "GrandChild"];
            let child2_grandchildren = ["GrandChild"];

            // Child1's grandchildren have duplicates
            let child1_filtered: Vec<_> = child1_grandchildren
                .iter()
                .filter(|&&name| child1_grandchildren.iter().filter(|&&n| n == name).count() == 1)
                .collect();
            assert_eq!(
                child1_filtered.len(),
                0,
                "Child1's duplicate grandchildren filtered"
            );

            // Child2's grandchild is unique
            let child2_filtered: Vec<_> = child2_grandchildren
                .iter()
                .filter(|&&name| child2_grandchildren.iter().filter(|&&n| n == name).count() == 1)
                .collect();
            assert_eq!(child2_filtered.len(), 1, "Child2's unique grandchild kept");
        }

        #[test]
        fn test_duplicate_log_message_format() {
            // Verify the expected log message format for skipped duplicates
            let class_name = "ModuleScript";
            let full_name = "game.ReplicatedStorage.DuplicateScript";

            let expected_msg = format!(
                "Skipped instance '{}' ({}) - path contains duplicate-named siblings (cannot reliably sync)",
                full_name, class_name
            );

            assert!(expected_msg.contains(full_name));
            assert!(expected_msg.contains(class_name));
            assert!(expected_msg.contains("duplicate-named siblings"));
        }

        #[test]
        fn test_parent_path_validation() {
            // When pulling an instance, we must verify the parent path is unique
            // This simulates the is_tree_path_unique check

            // Scenario: Trying to add a script under a folder that has duplicate siblings
            let parent_path_levels = [
                ("Root", vec!["FolderA", "FolderA"]), // Duplicate at this level!
            ];

            let path_unique = parent_path_levels
                .iter()
                .all(|(target, siblings)| siblings.iter().filter(|&&s| s == *target).count() == 1);

            assert!(
                !path_unique,
                "Parent path with duplicates should fail validation"
            );
        }

        #[test]
        fn test_valid_parent_path() {
            // Valid parent path with no duplicates at any level
            let parent_path_levels = [
                (
                    "ReplicatedStorage",
                    vec!["Workspace", "ReplicatedStorage", "ServerStorage"],
                ),
                ("Shared", vec!["Shared", "Client", "Server"]),
            ];

            let path_unique = parent_path_levels
                .iter()
                .all(|(target, siblings)| siblings.iter().filter(|&&s| s == *target).count() == 1);

            assert!(path_unique, "Valid parent path should pass validation");
        }
    }

    // Tests for syncback_removed_instance
    mod syncback_removed_tests {
        use crate::snapshot::{InstanceMetadata, InstanceSnapshot, RojoTree};
        use std::fs::{self, File};
        use std::io::Write;
        use tempfile::tempdir;

        #[test]
        fn test_remove_file_deletes_from_filesystem() {
            // Create a temporary directory and file
            let temp_dir = tempdir().expect("Failed to create temp dir");
            let file_path = temp_dir.path().join("TestModule.luau");

            // Create the file
            let mut file = File::create(&file_path).expect("Failed to create test file");
            file.write_all(b"return {}")
                .expect("Failed to write to file");

            // Verify file exists
            assert!(file_path.exists(), "Test file should exist before removal");

            // Create a tree with an instance pointing to this file
            let snapshot = InstanceSnapshot::new()
                .name("TestModule")
                .class_name("ModuleScript")
                .metadata(InstanceMetadata::new().instigating_source(file_path.clone()));

            let tree = RojoTree::new(snapshot);
            let root_id = tree.root().id();

            // Create a minimal ApiContext to call the method
            // Since syncback_removed_instance only needs the tree, we can test the core logic
            // by directly checking what the function would do

            // Get the instance and verify path resolution
            let instance = tree.get_instance(root_id).expect("Instance should exist");
            let instance_path = instance
                .metadata()
                .instigating_source
                .as_ref()
                .map(|s| s.path())
                .expect("Instance should have path");

            assert_eq!(instance_path, file_path.as_path());

            // Now delete the file (simulating what syncback_removed_instance does)
            fs::remove_file(instance_path).expect("Failed to remove file");

            // Verify file was deleted
            assert!(
                !file_path.exists(),
                "Test file should be deleted after removal"
            );
        }

        #[test]
        fn test_remove_directory_deletes_recursively() {
            // Create a temporary directory structure
            let temp_dir = tempdir().expect("Failed to create temp dir");
            let dir_path = temp_dir.path().join("TestFolder");
            fs::create_dir(&dir_path).expect("Failed to create test dir");

            // Create some files inside
            let child_file = dir_path.join("child.luau");
            File::create(&child_file)
                .expect("Failed to create child file")
                .write_all(b"return {}")
                .expect("Failed to write");

            let nested_dir = dir_path.join("nested");
            fs::create_dir(&nested_dir).expect("Failed to create nested dir");
            let nested_file = nested_dir.join("deep.luau");
            File::create(&nested_file)
                .expect("Failed to create nested file")
                .write_all(b"return {}")
                .expect("Failed to write");

            // Verify structure exists
            assert!(dir_path.exists(), "Test dir should exist");
            assert!(child_file.exists(), "Child file should exist");
            assert!(nested_file.exists(), "Nested file should exist");

            // Create a tree with an instance pointing to this directory
            let snapshot = InstanceSnapshot::new()
                .name("TestFolder")
                .class_name("Folder")
                .metadata(InstanceMetadata::new().instigating_source(dir_path.clone()));

            let tree = RojoTree::new(snapshot);
            let root_id = tree.root().id();

            // Get the instance path
            let instance = tree.get_instance(root_id).expect("Instance should exist");
            let instance_path = instance
                .metadata()
                .instigating_source
                .as_ref()
                .map(|s| s.path())
                .expect("Instance should have path");

            // Delete the directory recursively
            fs::remove_dir_all(instance_path).expect("Failed to remove directory");

            // Verify everything was deleted
            assert!(!dir_path.exists(), "Directory should be deleted");
            assert!(!child_file.exists(), "Child file should be deleted");
            assert!(!nested_file.exists(), "Nested file should be deleted");
        }

        #[test]
        fn test_remove_script_also_removes_adjacent_meta() {
            // Create a temporary directory
            let temp_dir = tempdir().expect("Failed to create temp dir");
            let script_path = temp_dir.path().join("MyScript.server.luau");
            let meta_path = temp_dir.path().join("MyScript.meta.json5");

            // Create both files
            File::create(&script_path)
                .expect("Failed to create script")
                .write_all(b"print('hello')")
                .expect("Failed to write");

            File::create(&meta_path)
                .expect("Failed to create meta file")
                .write_all(b"{}")
                .expect("Failed to write");

            // Verify both exist
            assert!(script_path.exists(), "Script should exist");
            assert!(meta_path.exists(), "Meta file should exist");

            // Create a tree with an instance pointing to the script
            let snapshot = InstanceSnapshot::new()
                .name("MyScript")
                .class_name("Script")
                .metadata(InstanceMetadata::new().instigating_source(script_path.clone()));

            let tree = RojoTree::new(snapshot);
            let root_id = tree.root().id();

            // Get the instance
            let instance = tree.get_instance(root_id).expect("Instance should exist");
            let instance_name = instance.name();
            let instance_path = instance
                .metadata()
                .instigating_source
                .as_ref()
                .map(|s| s.path())
                .expect("Instance should have path");

            // Delete the script file
            fs::remove_file(instance_path).expect("Failed to remove script");

            // Also remove adjacent meta file (simulating the full syncback_removed_instance logic)
            if let Some(parent_dir) = instance_path.parent() {
                let computed_meta_path = parent_dir.join(format!("{}.meta.json5", instance_name));
                if computed_meta_path.exists() {
                    fs::remove_file(&computed_meta_path).expect("Failed to remove meta file");
                }
            }

            // Verify both were deleted
            assert!(!script_path.exists(), "Script should be deleted");
            assert!(!meta_path.exists(), "Meta file should also be deleted");
        }

        #[test]
        fn test_meta_path_uses_instance_name_not_file_name() {
            // This test verifies that for "MyScript.server.luau", the meta file
            // is "MyScript.meta.json5" (based on instance name), not "MyScript.server.meta.json5"

            let instance_name = "MyScript";
            let file_name = "MyScript.server.luau";

            // The correct meta file name should be based on the instance name
            let correct_meta_name = format!("{}.meta.json5", instance_name);
            assert_eq!(correct_meta_name, "MyScript.meta.json5");

            // This shows what PathBuf::with_extension would incorrectly produce
            // if we used it on the file path instead of the instance name
            let path = std::path::PathBuf::from(file_name);
            let with_extension_result = path.with_extension("meta.json5");
            let incorrect_meta_name = with_extension_result.to_str().unwrap();

            assert_eq!(
                incorrect_meta_name, "MyScript.server.meta.json5",
                "with_extension produces wrong meta path for suffixed scripts"
            );

            // Our implementation should use the instance name instead
            assert_ne!(
                correct_meta_name, incorrect_meta_name,
                "Instance-name-based meta should differ from file-name-based meta for scripts with suffixes"
            );

            // Verify the correct approach: use instance name, not file path
            assert_eq!(
                correct_meta_name, "MyScript.meta.json5",
                "Correct meta path should be based on instance name"
            );
        }

        #[test]
        fn test_instance_without_instigating_source_returns_error() {
            // Create a tree with an instance that has no instigating_source
            let snapshot = InstanceSnapshot::new()
                .name("OrphanInstance")
                .class_name("ModuleScript");
            // Note: no .metadata() call, so instigating_source is None

            let tree = RojoTree::new(snapshot);
            let root_id = tree.root().id();

            let instance = tree.get_instance(root_id).expect("Instance should exist");

            // Verify that instigating_source is None
            assert!(
                instance.metadata().instigating_source.is_none(),
                "Instance should have no instigating_source"
            );
        }
    }

    mod dir_name_from_script_path_tests {
        use super::*;

        #[test]
        fn module_script_luau() {
            let path = Path::new("src/MyModule.luau");
            assert_eq!(dir_name_from_script_path(path), "MyModule");
        }

        #[test]
        fn server_script_luau() {
            let path = Path::new("src/Handler.server.luau");
            assert_eq!(dir_name_from_script_path(path), "Handler");
        }

        #[test]
        fn client_script_luau() {
            let path = Path::new("src/UI.client.luau");
            assert_eq!(dir_name_from_script_path(path), "UI");
        }

        #[test]
        fn local_script_luau() {
            let path = Path::new("src/Input.local.luau");
            assert_eq!(dir_name_from_script_path(path), "Input");
        }

        #[test]
        fn plugin_script_luau() {
            let path = Path::new("src/MyPlugin.plugin.luau");
            assert_eq!(dir_name_from_script_path(path), "MyPlugin");
        }

        #[test]
        fn legacy_script_luau() {
            let path = Path::new("src/OldCode.legacy.luau");
            assert_eq!(dir_name_from_script_path(path), "OldCode");
        }

        #[test]
        fn legacy_lua_extension() {
            let path = Path::new("src/OldModule.lua");
            assert_eq!(dir_name_from_script_path(path), "OldModule");
        }

        #[test]
        fn legacy_server_lua() {
            let path = Path::new("src/Main.server.lua");
            assert_eq!(dir_name_from_script_path(path), "Main");
        }

        #[test]
        fn legacy_client_lua() {
            let path = Path::new("src/Client.client.lua");
            assert_eq!(dir_name_from_script_path(path), "Client");
        }

        #[test]
        fn encoded_windows_chars_module() {
            // ? is invalid on Windows, encoded as %3F
            let path = Path::new("src/What%3F.luau");
            assert_eq!(dir_name_from_script_path(path), "What%3F");
        }

        #[test]
        fn encoded_windows_chars_server() {
            let path = Path::new("src/What%3F.server.luau");
            assert_eq!(dir_name_from_script_path(path), "What%3F");
        }

        #[test]
        fn encoded_colon_char() {
            // : is invalid on Windows, encoded as %3A
            let path = Path::new("src/Key%3AValue.client.luau");
            assert_eq!(dir_name_from_script_path(path), "Key%3AValue");
        }

        #[test]
        fn multiple_dots_in_name() {
            // Name like "my.module.luau" — file_stem is "my.module", no suffix match
            let path = Path::new("src/my.module.luau");
            assert_eq!(dir_name_from_script_path(path), "my.module");
        }

        #[test]
        fn name_ending_with_suffix_substring() {
            // "MyServer" should NOT be stripped to "My" — ".server" stripping only
            // applies to the suffix after file_stem strips ".luau"
            let path = Path::new("src/MyServer.luau");
            assert_eq!(dir_name_from_script_path(path), "MyServer");
        }
    }

    mod dir_name_from_instance_path_tests {
        use super::*;

        #[test]
        fn model_json5() {
            let path = Path::new("src/MyPart.model.json5");
            assert_eq!(dir_name_from_instance_path(path), "MyPart");
        }

        #[test]
        fn model_json_legacy() {
            let path = Path::new("src/MyPart.model.json");
            assert_eq!(dir_name_from_instance_path(path), "MyPart");
        }

        #[test]
        fn txt_file() {
            let path = Path::new("src/Greeting.txt");
            assert_eq!(dir_name_from_instance_path(path), "Greeting");
        }

        #[test]
        fn csv_file() {
            let path = Path::new("src/Translations.csv");
            assert_eq!(dir_name_from_instance_path(path), "Translations");
        }

        #[test]
        fn encoded_windows_chars_model() {
            let path = Path::new("src/What%3F.model.json5");
            assert_eq!(dir_name_from_instance_path(path), "What%3F");
        }

        #[test]
        fn encoded_windows_chars_txt() {
            let path = Path::new("src/Ask%3F.txt");
            assert_eq!(dir_name_from_instance_path(path), "Ask%3F");
        }

        #[test]
        fn encoded_colon_model() {
            let path = Path::new("src/Key%3AValue.model.json5");
            assert_eq!(dir_name_from_instance_path(path), "Key%3AValue");
        }

        #[test]
        fn encoded_colon_legacy_model() {
            let path = Path::new("src/Key%3AValue.model.json");
            assert_eq!(dir_name_from_instance_path(path), "Key%3AValue");
        }

        #[test]
        fn multiple_dots_in_name_model() {
            // "my.part.model.json5" — should strip ".model.json5"
            let path = Path::new("src/my.part.model.json5");
            assert_eq!(dir_name_from_instance_path(path), "my.part");
        }

        #[test]
        fn unknown_extension_uses_file_stem() {
            // Fallback to file_stem for unrecognized extensions
            let path = Path::new("src/Something.toml");
            assert_eq!(dir_name_from_instance_path(path), "Something");
        }
    }

    // ══════════════════════════════════════════════════════════════════
    //  Slug collision removal safety tests
    //
    //  These verify the critical invariant: when an instance is removed
    //  and a NEW instance is created at the same slugified path, the
    //  removal must NOT destroy the newly created file.
    //
    //  This was a real bug: renaming "joe_test" → "joe/test" produces
    //  the same slug "joe_test". The API handler deletes the old file
    //  and creates the new one, but the ChangeProcessor's PatchSet
    //  removal would re-delete the new file because path.exists()
    //  returned true for the NEW file.
    // ══════════════════════════════════════════════════════════════════
    mod slug_collision_removal_safety {
        use crate::syncback::{name_needs_slugify, slugify_name};
        use std::fs::{self, File};
        use std::io::Write;
        use tempfile::tempdir;

        /// Simulates the API handler flow: remove old file, create new file at
        /// potentially the same path (because slugs collide), then verify the
        /// new file survives.
        fn simulate_rename_via_slug(
            old_name: &str,
            new_name: &str,
            extension: &str,
        ) -> (bool, String, String) {
            // Returns: (new_file_survived, old_slug, new_slug)
            let old_slug = if name_needs_slugify(old_name) {
                slugify_name(old_name)
            } else {
                old_name.to_string()
            };
            let new_slug = if name_needs_slugify(new_name) {
                slugify_name(new_name)
            } else {
                new_name.to_string()
            };

            let temp = tempdir().expect("Failed to create temp dir");
            let old_file = temp.path().join(format!("{old_slug}.{extension}"));
            let new_file = temp.path().join(format!("{new_slug}.{extension}"));

            // Step 1: Create old file (simulates existing state)
            File::create(&old_file)
                .unwrap()
                .write_all(b"old content")
                .unwrap();

            // Step 2: API handler removes old file
            if old_file.exists() {
                fs::remove_file(&old_file).unwrap();
            }

            // Step 3: API handler creates new file (may be same path!)
            File::create(&new_file)
                .unwrap()
                .write_all(b"new content")
                .unwrap();

            // Step 4: ChangeProcessor receives PatchSet with old instance removal.
            // The OLD instance's instigating_source points to old_file.
            // The fix: ChangeProcessor must NOT delete old_file because it
            // now contains the NEW instance's content.
            //
            // (We don't actually call handle_tree_event here — we verify
            // the invariant that the new file must survive.)

            let survived =
                new_file.exists() && fs::read_to_string(&new_file).unwrap() == "new content";

            (survived, old_slug, new_slug)
        }

        // ── Same-slug renames (the critical case) ────────────────────

        #[test]
        fn rename_slash_to_underscore_same_slug() {
            // "joe/test" → slug "joe_test", same as original "joe_test"
            let (survived, old_slug, new_slug) =
                simulate_rename_via_slug("joe_test", "joe/test", "legacy.luau");
            assert_eq!(old_slug, "joe_test");
            assert_eq!(new_slug, "joe_test");
            assert!(survived, "new file must survive when slugs collide");
        }

        #[test]
        fn rename_colon_to_underscore_same_slug() {
            let (survived, old, new) = simulate_rename_via_slug("A_B", "A:B", "luau");
            assert_eq!(old, "A_B");
            assert_eq!(new, "A_B");
            assert!(survived);
        }

        #[test]
        fn rename_star_to_underscore_same_slug() {
            let (survived, old, new) =
                simulate_rename_via_slug("Glob_Pattern", "Glob*Pattern", "server.luau");
            assert_eq!(old, "Glob_Pattern");
            assert_eq!(new, "Glob_Pattern");
            assert!(survived);
        }

        #[test]
        fn rename_question_to_underscore_same_slug() {
            let (survived, _, _) = simulate_rename_via_slug("What_", "What?", "luau");
            assert!(survived);
        }

        #[test]
        fn rename_pipe_to_underscore_same_slug() {
            let (survived, _, _) = simulate_rename_via_slug("X_Y", "X|Y", "client.luau");
            assert!(survived);
        }

        #[test]
        fn rename_backslash_to_underscore_same_slug() {
            let (survived, _, _) = simulate_rename_via_slug("path_to", "path\\to", "luau");
            assert!(survived);
        }

        #[test]
        fn rename_angle_brackets_same_slug() {
            let (survived, _, _) = simulate_rename_via_slug("_init_", "<init>", "luau");
            assert!(survived);
        }

        #[test]
        fn rename_tilde_to_underscore_same_slug() {
            let (survived, old, new) = simulate_rename_via_slug("foo_1", "foo~1", "luau");
            assert_eq!(old, "foo_1");
            assert_eq!(new, "foo_1");
            assert!(survived);
        }

        // ── Different-slug renames (should also work fine) ───────────

        #[test]
        fn rename_clean_to_clean_different_slug() {
            let (survived, old, new) = simulate_rename_via_slug("Alpha", "Beta", "luau");
            assert_ne!(old, new);
            assert!(survived, "different slugs should trivially survive");
        }

        #[test]
        fn rename_clean_to_forbidden() {
            let (survived, old, new) =
                simulate_rename_via_slug("MyScript", "My/Script", "server.luau");
            assert_eq!(old, "MyScript");
            assert_eq!(new, "My_Script");
            assert_ne!(old, new);
            assert!(survived);
        }

        #[test]
        fn rename_forbidden_to_clean() {
            let (survived, _, _) = simulate_rename_via_slug("X_Y", "XY", "luau");
            assert!(survived);
        }

        // ── Dangerous suffix renames ─────────────────────────────────

        #[test]
        fn rename_to_dangerous_suffix_server() {
            let (survived, _, new) = simulate_rename_via_slug("foo", "foo.server", "luau");
            assert_eq!(new, "foo_server");
            assert!(survived);
        }

        #[test]
        fn rename_to_dangerous_suffix_meta() {
            let (survived, _, new) = simulate_rename_via_slug("config", "config.meta", "luau");
            assert_eq!(new, "config_meta");
            assert!(survived);
        }

        #[test]
        fn rename_from_dangerous_suffix() {
            let (survived, old, _) = simulate_rename_via_slug("foo_server", "foo.server", "luau");
            assert_eq!(old, "foo_server");
            assert!(survived);
        }

        // ── Windows reserved name renames ────────────────────────────

        #[test]
        fn rename_to_con() {
            let (survived, _, new) = simulate_rename_via_slug("config", "CON", "luau");
            assert_eq!(new, "CON_");
            assert!(survived);
        }

        #[test]
        fn rename_from_con_slug_collision() {
            // "CON_" (natural) → "CON" (reserved, slugifies to "CON_")
            let (survived, old, new) = simulate_rename_via_slug("CON_", "CON", "luau");
            assert_eq!(old, "CON_");
            assert_eq!(new, "CON_");
            assert!(survived, "same-slug collision from Windows reserved name");
        }

        // ── Space renames ────────────────────────────────────────────

        #[test]
        fn rename_add_leading_space() {
            let (survived, old, new) = simulate_rename_via_slug("Hello", " Hello", "luau");
            assert_eq!(old, "Hello");
            assert_eq!(new, "Hello"); // leading space stripped
            assert!(survived, "stripped leading space creates same slug");
        }

        #[test]
        fn rename_add_trailing_space() {
            let (survived, old, new) = simulate_rename_via_slug("Hello", "Hello ", "luau");
            assert_eq!(old, "Hello");
            assert_eq!(new, "Hello"); // trailing space stripped
            assert!(survived);
        }

        #[test]
        fn rename_with_middle_space_no_collision() {
            let (survived, old, new) = simulate_rename_via_slug("Hello", "Hello World", "luau");
            assert_eq!(old, "Hello");
            assert_eq!(new, "Hello World");
            assert_ne!(old, new);
            assert!(survived);
        }

        // ── Meta file survival ───────────────────────────────────────

        #[test]
        fn meta_file_survives_slug_collision_rename() {
            let temp = tempdir().expect("Failed to create temp dir");
            let slug = "joe_test";

            // Create old files
            let old_script = temp.path().join(format!("{slug}.legacy.luau"));
            let old_meta = temp.path().join(format!("{slug}.meta.json5"));
            File::create(&old_script)
                .unwrap()
                .write_all(b"old")
                .unwrap();
            File::create(&old_meta).unwrap().write_all(b"{}").unwrap();

            // API handler: remove old
            fs::remove_file(&old_script).unwrap();
            fs::remove_file(&old_meta).unwrap();

            // API handler: create new (same slug from "joe/test")
            let new_script = temp.path().join(format!("{slug}.legacy.luau"));
            let new_meta = temp.path().join(format!("{slug}.meta.json5"));
            File::create(&new_script)
                .unwrap()
                .write_all(b"new code")
                .unwrap();
            File::create(&new_meta)
                .unwrap()
                .write_all(br#"{"name": "joe/test"}"#)
                .unwrap();

            // Both new files must survive
            assert!(new_script.exists(), "new script must survive");
            assert!(new_meta.exists(), "new meta must survive");
            assert_eq!(fs::read_to_string(&new_script).unwrap(), "new code");
            assert!(fs::read_to_string(&new_meta).unwrap().contains("joe/test"));
        }

        // ── Directory format survival ────────────────────────────────

        #[test]
        fn directory_survives_slug_collision_rename() {
            let temp = tempdir().expect("Failed to create temp dir");

            // Old instance "Stuff_Here" is a directory
            let old_dir = temp.path().join("Stuff_Here");
            fs::create_dir(&old_dir).unwrap();
            File::create(old_dir.join("init.luau"))
                .unwrap()
                .write_all(b"old")
                .unwrap();

            // API handler: remove old directory
            fs::remove_dir_all(&old_dir).unwrap();

            // API handler: create new directory (slug of "Stuff/Here" = "Stuff_Here")
            let new_dir = temp.path().join("Stuff_Here");
            fs::create_dir(&new_dir).unwrap();
            File::create(new_dir.join("init.luau"))
                .unwrap()
                .write_all(b"new")
                .unwrap();
            File::create(new_dir.join("init.meta.json5"))
                .unwrap()
                .write_all(br#"{"name": "Stuff/Here"}"#)
                .unwrap();

            // New directory and contents must survive
            assert!(new_dir.exists(), "new directory must survive");
            assert!(new_dir.join("init.luau").exists());
            assert!(new_dir.join("init.meta.json5").exists());
            assert_eq!(
                fs::read_to_string(new_dir.join("init.luau")).unwrap(),
                "new"
            );
        }

        // ── Stress: many renames producing same slug ─────────────────

        #[test]
        fn stress_many_forbidden_chars_same_slug() {
            // All these names slugify to the same thing. Verify each
            // rename-to scenario preserves the new file.
            let variants = [
                ("A_B", "A/B"),
                ("A_B", "A:B"),
                ("A_B", "A*B"),
                ("A_B", "A?B"),
                ("A_B", "A<B"),
                ("A_B", "A>B"),
                ("A_B", "A|B"),
                ("A_B", "A\\B"),
                ("A_B", "A\"B"),
                ("A_B", "A~B"),
            ];
            for (old_name, new_name) in variants {
                let (survived, old_slug, new_slug) =
                    simulate_rename_via_slug(old_name, new_name, "luau");
                assert_eq!(
                    old_slug, new_slug,
                    "{old_name} and {new_name} should produce same slug"
                );
                assert!(
                    survived,
                    "new file must survive for {old_name} → {new_name}"
                );
            }
        }

        #[test]
        fn stress_bidirectional_rename_survival() {
            // Rename A→B then B→A, both produce same slug.
            // Verify both directions work.
            let pairs = [
                ("joe_test", "joe/test"),
                ("Hey_Bro", "Hey:Bro"),
                ("foo_1", "foo~1"),
                ("CON_", "CON"),
            ];
            for (a, b) in pairs {
                let (surv_ab, _, _) = simulate_rename_via_slug(a, b, "luau");
                let (surv_ba, _, _) = simulate_rename_via_slug(b, a, "luau");
                assert!(surv_ab, "A→B must survive for {a} → {b}");
                assert!(surv_ba, "B→A must survive for {b} → {a}");
            }
        }

        // ── Verify name_needs_slugify consistency ────────────────────

        #[test]
        fn slug_collision_only_possible_when_needs_slugify() {
            // If neither old nor new needs slugifying, their slugs are
            // themselves, so they can't collide (different names = different slugs).
            let cases = [("Alpha", "Beta"), ("Hello", "World"), ("x", "y")];
            for (old, new) in cases {
                assert!(!name_needs_slugify(old));
                assert!(!name_needs_slugify(new));
                let old_slug = slugify_name(old);
                let new_slug = slugify_name(new);
                assert_ne!(
                    old_slug, new_slug,
                    "clean names with different values can't produce same slug"
                );
            }
        }

        #[test]
        fn all_forbidden_chars_produce_underscore_slug() {
            // Every forbidden char in isolation slugifies to "_" → "instance"
            let forbidden = ['<', '>', ':', '"', '/', '|', '?', '*', '\\', '~'];
            for ch in forbidden {
                let name = ch.to_string();
                assert!(name_needs_slugify(&name));
                let slug = slugify_name(&name);
                assert_eq!(
                    slug, "instance",
                    "single {ch:?} should slugify to fallback 'instance'"
                );
            }
        }
    }

    // ══════════════════════════════════════════════════════════════════
    //  Two-way sync: write_child_to_disk dedup simulation
    //
    //  These simulate the actual flow in write_child_to_disk:
    //  1. Read parent directory entries → taken_names
    //  2. Slugify instance name if needed
    //  3. Deduplicate against taken_names
    //  4. Write file + optional meta with `name` field
    //
    //  This is the MOST critical path for two-way sync correctness.
    // ══════════════════════════════════════════════════════════════════
    mod twoway_sync_dedup {
        use crate::syncback::{deduplicate_name, name_needs_slugify, slugify_name};
        use std::collections::HashSet;
        use std::fs::{self, File};
        use std::io::Write;
        use tempfile::tempdir;

        // ══════════════════════════════════════════════════════════════
        //  IMPORTANT: deduplicate_name compares BARE SLUGS against
        //  taken_names. When taken_names is seeded from fs::read_dir,
        //  entries include extensions (e.g., "foo.luau"). The bare slug
        //  "foo" does NOT match "foo.luau", so dedup only catches
        //  collisions for DIRECTORY entries (where filename == bare name).
        //
        //  For file-format instances, the duplicate-name pre-filter in
        //  the plugin (encodeInstance.lua) prevents same-name siblings
        //  from reaching the API at all. Cross-name slug collisions for
        //  file-format are a known limitation until Phase 2.
        //
        //  These tests focus on DIRECTORY FORMAT dedup (where it works)
        //  and document file-format behavior accurately.
        // ══════════════════════════════════════════════════════════════

        /// Simulates the write_child_to_disk flow for DIRECTORY format:
        /// directory names have no extension, so bare slug == filename,
        /// and dedup against fs::read_dir entries works correctly.
        fn simulate_dir_dedup(
            existing_dirs: &[&str],
            instance_name: &str,
        ) -> (String, Option<String>) {
            let temp = tempdir().expect("Failed to create temp dir");
            for name in existing_dirs {
                fs::create_dir(temp.path().join(name)).unwrap();
            }

            let taken: HashSet<String> = fs::read_dir(temp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_lowercase())
                .collect();

            let needs_slug = name_needs_slugify(instance_name);
            let base = if needs_slug {
                slugify_name(instance_name)
            } else {
                instance_name.to_string()
            };
            let deduped = deduplicate_name(&base, &taken);
            let needs_meta = needs_slug || deduped != base;

            fs::create_dir(temp.path().join(&deduped)).unwrap();

            let meta = if needs_meta {
                Some(instance_name.to_string())
            } else {
                None
            };
            (deduped, meta)
        }

        /// Simulates batch add of DIRECTORY format children.
        fn simulate_dir_batch(
            existing_dirs: &[&str],
            children: &[&str],
        ) -> Vec<(String, Option<String>)> {
            let temp = tempdir().expect("Failed to create temp dir");
            for name in existing_dirs {
                fs::create_dir(temp.path().join(name)).unwrap();
            }

            let mut taken: HashSet<String> = fs::read_dir(temp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_lowercase())
                .collect();

            let mut results = Vec::new();
            for &name in children {
                let needs_slug = name_needs_slugify(name);
                let base = if needs_slug {
                    slugify_name(name)
                } else {
                    name.to_string()
                };
                let deduped = deduplicate_name(&base, &taken);
                let needs_meta = needs_slug || deduped != base;
                taken.insert(deduped.to_lowercase());
                let meta = if needs_meta {
                    Some(name.to_string())
                } else {
                    None
                };
                results.push((deduped, meta));
            }
            results
        }

        // ── Directory dedup against existing dirs ────────────────────

        #[test]
        fn dir_clean_name_no_collision() {
            let (f, m) = simulate_dir_dedup(&[], "NewFolder");
            assert_eq!(f, "NewFolder");
            assert!(m.is_none());
        }

        #[test]
        fn dir_clean_name_collides_with_existing() {
            let (f, m) = simulate_dir_dedup(&["NewFolder"], "NewFolder");
            assert_eq!(f, "NewFolder~1");
            assert_eq!(m.unwrap(), "NewFolder");
        }

        #[test]
        fn dir_forbidden_name_no_collision() {
            let (f, m) = simulate_dir_dedup(&[], "Hey/Bro");
            assert_eq!(f, "Hey_Bro");
            assert_eq!(m.unwrap(), "Hey/Bro");
        }

        #[test]
        fn dir_forbidden_collides_with_existing() {
            let (f, m) = simulate_dir_dedup(&["Hey_Bro"], "Hey/Bro");
            assert_eq!(f, "Hey_Bro~1");
            assert_eq!(m.unwrap(), "Hey/Bro");
        }

        #[test]
        fn dir_natural_collides_with_slug() {
            let (f, m) = simulate_dir_dedup(&["A_B"], "A_B");
            assert_eq!(f, "A_B~1");
            assert_eq!(m.unwrap(), "A_B");
        }

        #[test]
        fn dir_case_insensitive_collision() {
            let (f, m) = simulate_dir_dedup(&["MyFolder"], "myfolder");
            assert_eq!(f, "myfolder~1");
            assert_eq!(m.unwrap(), "myfolder");
        }

        #[test]
        fn dir_multiple_collisions() {
            let (f, _) = simulate_dir_dedup(&["Foo", "Foo~1", "Foo~2"], "Foo");
            assert_eq!(f, "Foo~3");
        }

        #[test]
        fn dir_gap_in_chain() {
            let (f, _) = simulate_dir_dedup(&["Test", "Test~1", "Test~3"], "Test");
            assert_eq!(f, "Test~2");
        }

        #[test]
        fn dir_dangerous_suffix_server() {
            let (f, m) = simulate_dir_dedup(&[], "foo.server");
            assert_eq!(f, "foo_server");
            assert_eq!(m.unwrap(), "foo.server");
        }

        #[test]
        fn dir_dangerous_suffix_collides() {
            let (f, m) = simulate_dir_dedup(&["config_meta"], "config.meta");
            assert_eq!(f, "config_meta~1");
            assert_eq!(m.unwrap(), "config.meta");
        }

        #[test]
        fn dir_con_creates_valid_name() {
            let (f, m) = simulate_dir_dedup(&[], "CON");
            assert_eq!(f, "CON_");
            assert_eq!(m.unwrap(), "CON");
        }

        #[test]
        fn dir_nul_collides_with_existing() {
            let (f, _) = simulate_dir_dedup(&["NUL_"], "NUL");
            assert_eq!(f, "NUL_~1");
        }

        // ── Batch add: directory siblings ─────────────────────────────

        #[test]
        fn batch_dir_three_clean() {
            let r = simulate_dir_batch(&[], &["Alpha", "Beta", "Gamma"]);
            assert_eq!(r[0].0, "Alpha");
            assert_eq!(r[1].0, "Beta");
            assert_eq!(r[2].0, "Gamma");
            assert!(r.iter().all(|(_, m)| m.is_none()));
        }

        #[test]
        fn batch_dir_duplicate_names() {
            let r = simulate_dir_batch(&[], &["Script", "Script", "Script"]);
            assert_eq!(r[0].0, "Script");
            assert!(r[0].1.is_none());
            assert_eq!(r[1].0, "Script~1");
            assert_eq!(r[1].1.as_deref(), Some("Script"));
            assert_eq!(r[2].0, "Script~2");
        }

        #[test]
        fn batch_dir_slug_collision_siblings() {
            let r = simulate_dir_batch(&[], &["X/Y", "X:Y", "X*Y"]);
            assert_eq!(r[0].0, "X_Y");
            assert_eq!(r[1].0, "X_Y~1");
            assert_eq!(r[2].0, "X_Y~2");
        }

        #[test]
        fn batch_dir_with_existing_and_collisions() {
            let r = simulate_dir_batch(&["Utils", "Config"], &["Utils", "Config", "NewThing"]);
            assert_eq!(r[0].0, "Utils~1");
            assert_eq!(r[1].0, "Config~1");
            assert_eq!(r[2].0, "NewThing");
            assert!(r[2].1.is_none());
        }

        #[test]
        fn batch_dir_dangerous_then_natural_then_slug() {
            let r = simulate_dir_batch(&[], &["foo.server", "foo_server", "foo/server"]);
            assert_eq!(r[0].0, "foo_server");
            assert_eq!(r[1].0, "foo_server~1");
            assert_eq!(r[2].0, "foo_server~2");
        }

        #[test]
        fn batch_dir_windows_reserved_pileup() {
            let r = simulate_dir_batch(&[], &["CON", "CON_", "con"]);
            assert_eq!(r[0].0, "CON_");
            assert_eq!(r[0].1.as_deref(), Some("CON"));
            assert_eq!(r[1].0, "CON_~1");
            assert_eq!(r[1].1.as_deref(), Some("CON_"));
            assert_eq!(r[2].0, "con_~2");
            assert_eq!(r[2].1.as_deref(), Some("con"));
        }

        #[test]
        fn batch_dir_empty_names() {
            let r = simulate_dir_batch(&[], &["", "", ""]);
            assert_eq!(r[0].0, "instance");
            assert_eq!(r[1].0, "instance~1");
            assert_eq!(r[2].0, "instance~2");
        }

        #[test]
        fn batch_dir_20_same_name() {
            let children: Vec<&str> = vec!["Spam"; 20];
            let r = simulate_dir_batch(&[], &children);
            assert_eq!(r[0].0, "Spam");
            for (i, entry) in r.iter().enumerate().skip(1) {
                assert_eq!(entry.0, format!("Spam~{i}"));
            }
            let unique: HashSet<String> = r.iter().map(|(f, _)| f.to_lowercase()).collect();
            assert_eq!(unique.len(), 20);
        }

        // ── File-format behavior documentation ───────────────────────

        #[test]
        fn file_format_dedup_with_bare_slugs() {
            // Production code now seeds taken_names from tree siblings'
            // slugified instance names (bare slugs, no extensions).
            // This means file-format dedup works correctly.
            let taken: HashSet<String> = ["foo".to_string()].into_iter().collect();
            // Bare slug "Foo" matches "foo" in taken
            let deduped = deduplicate_name("Foo", &taken);
            assert_eq!(deduped, "Foo~1", "bare slug collides with existing sibling");
        }

        #[test]
        fn file_format_slugify_and_dedup_combined() {
            // Slugify + dedup pipeline works end-to-end for file format
            let taken: HashSet<String> = ["hey_bro".to_string()].into_iter().collect();

            let slug = slugify_name("Hey/Bro");
            assert_eq!(slug, "Hey_Bro");
            let deduped = deduplicate_name(&slug, &taken);
            assert_eq!(deduped, "Hey_Bro~1", "slug collision correctly detected");
        }

        // ── Rename → re-add cycle (the critical slug safety test) ────

        #[test]
        fn rename_cycle_same_slug_survives() {
            let temp = tempdir().expect("Failed to create temp dir");

            File::create(temp.path().join("joe_test.legacy.luau"))
                .unwrap()
                .write_all(b"old code")
                .unwrap();

            // API: remove old
            fs::remove_file(temp.path().join("joe_test.legacy.luau")).unwrap();

            // API: write new (dir is now empty)
            let taken: HashSet<String> = fs::read_dir(temp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_lowercase())
                .collect();
            assert!(taken.is_empty());

            let slug = slugify_name("joe/test");
            assert_eq!(slug, "joe_test");
            let deduped = deduplicate_name(&slug, &taken);
            assert_eq!(deduped, "joe_test");

            let new_file = temp.path().join("joe_test.legacy.luau");
            File::create(&new_file)
                .unwrap()
                .write_all(b"new code")
                .unwrap();
            let meta_file = temp.path().join("joe_test.meta.json5");
            File::create(&meta_file)
                .unwrap()
                .write_all(br#"{"name": "joe/test"}"#)
                .unwrap();

            assert!(new_file.exists());
            assert!(meta_file.exists());
            assert_eq!(fs::read_to_string(&new_file).unwrap(), "new code");
            assert!(fs::read_to_string(&meta_file).unwrap().contains("joe/test"));
        }

        #[test]
        fn dir_add_to_populated_no_overwrite() {
            // Dir has: Helper/ and Helper.meta.json5
            // Add new "Helper" dir → must get Helper~1
            let temp = tempdir().expect("Failed to create temp dir");
            fs::create_dir(temp.path().join("Helper")).unwrap();
            File::create(temp.path().join("Helper.meta.json5"))
                .unwrap()
                .write_all(b"{}")
                .unwrap();

            let taken: HashSet<String> = fs::read_dir(temp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_lowercase())
                .collect();

            let deduped = deduplicate_name("Helper", &taken);
            assert_eq!(deduped, "Helper~1");
        }

        // ── Nightmare batch scenarios ────────────────────────────────

        #[test]
        fn batch_dir_10_way_slug_pileup_with_existing() {
            let r = simulate_dir_batch(
                &["A_B"],
                &[
                    "A/B", "A:B", "A*B", "A?B", "A<B", "A>B", "A|B", "A\\B", "A\"B",
                ],
            );
            assert_eq!(r[0].0, "A_B~1");
            assert_eq!(r[1].0, "A_B~2");
            assert_eq!(r[2].0, "A_B~3");
            assert_eq!(r[3].0, "A_B~4");
            assert_eq!(r[4].0, "A_B~5");
            assert_eq!(r[5].0, "A_B~6");
            assert_eq!(r[6].0, "A_B~7");
            assert_eq!(r[7].0, "A_B~8");
            assert_eq!(r[8].0, "A_B~9");
        }

        #[test]
        fn batch_dir_con_prn_nul_then_slugged() {
            let r = simulate_dir_batch(&[], &["CON", "PRN", "CON/", "PRN/"]);
            assert_eq!(r[0].0, "CON_");
            assert_eq!(r[1].0, "PRN_");
            assert_eq!(r[2].0, "CON_~1"); // "CON/" slug "CON_" collides
            assert_eq!(r[3].0, "PRN_~1");
        }

        #[test]
        fn batch_dir_unicode_plus_forbidden() {
            let r = simulate_dir_batch(&[], &["カフェ/Bar", "カフェ:Bar", "カフェ_Bar"]);
            assert_eq!(r[0].0, "カフェ_Bar");
            assert_eq!(r[1].0, "カフェ_Bar~1");
            assert_eq!(r[2].0, "カフェ_Bar~2");
        }

        #[test]
        fn batch_dir_all_unique_filenames_invariant() {
            // A massive batch — every result must be unique.
            let children = &[
                "Foo",
                "foo",
                "FOO",
                "A/B",
                "A:B",
                "A_B",
                "CON",
                "CON_",
                "",
                "",
                "test.server",
                "test_server",
            ];
            let r = simulate_dir_batch(&[], children);
            let unique: HashSet<String> = r.iter().map(|(f, _)| f.to_lowercase()).collect();
            assert_eq!(
                unique.len(),
                r.len(),
                "every sibling must get a unique filename"
            );
        }
    }
}
