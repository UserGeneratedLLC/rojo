use std::{
    collections::HashSet,
    io,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Instant,
};

use crossbeam_channel::Sender;
use memofs::{PrefetchCache, Vfs};
use thiserror::Error;

use crate::{
    change_processor::ChangeProcessor,
    message_queue::MessageQueue,
    project::{Project, ProjectError},
    session_id::SessionId,
    snapshot::{
        apply_patch_set, compute_patch_set, AppliedPatchSet, InstanceContext, InstanceSnapshot,
        PatchSet, RojoTree,
    },
    snapshot_middleware::snapshot_from_vfs,
};

/// Set to `true` to validate on plugin connect (useful for testing, do not enable on production).
const VALIDATE_TREE_ON_CONNECT: bool = false;

/// Result of a read-only tree freshness check. Reports how many instances
/// differ between the in-memory tree and the real filesystem.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeFreshnessReport {
    pub is_fresh: bool,
    pub added: usize,
    pub removed: usize,
    pub updated: usize,
    pub elapsed_ms: f64,
}

/// Contains all of the state for a Rojo serve session. A serve session is used
/// when we need to build a Rojo tree and possibly rebuild it when input files
/// change.
///
/// Nothing here is specific to any Rojo interface. Though the primary way to
/// interact with a serve session is Rojo's HTTP right now, there's no reason
/// why Rojo couldn't expose an IPC or channels-based API for embedding in the
/// future. `ServeSession` would be roughly the right interface to expose for
/// those cases.
pub struct ServeSession {
    /// The object responsible for listening to changes from the in-memory
    /// filesystem, applying them, updating the Roblox instance tree, and
    /// routing messages through the session's message queue to any connected
    /// clients.
    ///
    /// SHOULD BE DROPPED FIRST! ServeSession and ChangeProcessor communicate
    /// with eachother via channels. If ServeSession hangs up those channels
    /// before dropping the ChangeProcessor, its thread will panic with a
    /// RecvError, causing the main thread to panic on drop.
    ///
    /// `None` for oneshot sessions (syncback, upload, plugin install) that
    /// don't need live filesystem monitoring.
    ///
    /// Allowed to be unused because it has side effects when dropped.
    #[allow(unused)]
    change_processor: Option<ChangeProcessor>,

    /// When the serve session was started. Used only for user-facing
    /// diagnostics.
    start_time: Instant,

    /// The root project for the serve session.
    ///
    /// This will be defined if a folder with a `default.project.json5` file was
    /// used for starting the serve session, or if the user specified a full
    /// path to a `.project.json5` file.
    root_project: Project,

    /// A randomly generated ID for this serve session. It's used to ensure that
    /// a client doesn't begin connecting to a different server part way through
    /// an operation that needs to be atomic.
    session_id: SessionId,

    /// The tree of Roblox instances associated with this session that will be
    /// updated in real-time. This is derived from the session's VFS and will
    /// eventually be mutable to connected clients.
    tree: Arc<Mutex<RojoTree>>,

    /// An in-memory filesystem containing all of the files relevant for this
    /// live session.
    ///
    /// The main use for accessing it from the session is for debugging issues
    /// with Rojo's live-sync protocol.
    vfs: Arc<Vfs>,

    /// A queue of changes that have been applied to `tree` that affect clients.
    ///
    /// Clients to the serve session will subscribe to this queue either
    /// directly or through the HTTP API to be notified of mutations that need
    /// to be applied.
    message_queue: Arc<MessageQueue<AppliedPatchSet>>,

    /// A channel to send mutation requests on. These will be handled by the
    /// ChangeProcessor and trigger changes in the tree.
    /// `None` for oneshot sessions.
    tree_mutation_sender: Option<Sender<PatchSet>>,

    /// Paths recently written by the API's syncback. The ChangeProcessor
    /// checks this map and suppresses the file watcher echo for these paths
    /// to avoid redundant re-snapshots and WebSocket messages.
    /// Values are `(remove_count, create_write_count)` — each API write increments
    /// the appropriate counter, each suppressed VFS event decrements it.
    /// `None` for oneshot sessions.
    #[allow(dead_code, clippy::type_complexity)]
    suppressed_paths:
        Option<Arc<Mutex<std::collections::HashMap<std::path::PathBuf, (usize, usize)>>>>,

    /// Index of meta/model files that contain `Rojo_Ref_*` attributes.
    /// Shared between ApiService (writes) and ChangeProcessor (rename updates).
    /// `None` for oneshot sessions.
    ref_path_index: Option<Arc<Mutex<crate::RefPathIndex>>>,

    /// Root of the git repository, if the project is inside one.
    /// Computed once at session start for use by auto-staging.
    git_repo_root: Option<std::path::PathBuf>,
}

/// Collect all filesystem paths reachable from the project tree's `$path`
/// entries, then read file contents and compute canonical paths in parallel.
fn prefetch_project_files(project: &Project) -> io::Result<PrefetchCache> {
    use rayon::prelude::*;
    use std::collections::HashMap;
    use walkdir::WalkDir;

    let folder = project.folder_location();

    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    collect_path_roots(&project.tree, folder, &mut roots);

    if roots.is_empty() {
        return Ok(PrefetchCache {
            files: HashMap::new(),
            canonical: HashMap::new(),
            is_file: HashMap::new(),
        });
    }

    let walk_start = Instant::now();

    let mut entries: Vec<walkdir::DirEntry> = Vec::new();
    for root in &roots {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).follow_links(true) {
            if let Ok(e) = entry {
                entries.push(e);
            }
        }
    }

    // Also include the project file itself (read by project middleware).
    if let Ok(meta) = std::fs::metadata(&project.file_location) {
        if meta.is_file() {
            entries.push(
                WalkDir::new(&project.file_location)
                    .into_iter()
                    .next()
                    .unwrap()
                    .unwrap(),
            );
        }
    }

    let walk_elapsed = walk_start.elapsed();
    let file_count = entries.iter().filter(|e| e.file_type().is_file()).count();
    let dir_count = entries.len() - file_count;

    log::info!(
        "Prefetch walk: {} files + {} dirs from {} root(s) in {:.1?}",
        file_count,
        dir_count,
        roots.len(),
        walk_elapsed,
    );

    let read_start = Instant::now();

    let file_data: Vec<_> = entries
        .par_iter()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            std::fs::read(&path).ok().map(|c| (path, c))
        })
        .collect();

    let read_elapsed = read_start.elapsed();
    let canon_start = Instant::now();

    let canonical_data: Vec<_> = entries
        .par_iter()
        .filter_map(|e| {
            let path = e.path().to_path_buf();
            std::fs::canonicalize(&path).ok().map(|c| (path, c))
        })
        .collect();

    let canon_elapsed = canon_start.elapsed();

    log::info!(
        "Prefetch I/O: read {} files in {:.1?}, canonicalize {} paths in {:.1?}",
        file_data.len(),
        read_elapsed,
        canonical_data.len(),
        canon_elapsed,
    );

    let is_file: HashMap<_, _> = entries
        .iter()
        .map(|e| (e.path().to_path_buf(), e.file_type().is_file()))
        .collect();

    Ok(PrefetchCache {
        files: file_data.into_iter().collect::<HashMap<_, _>>(),
        canonical: canonical_data.into_iter().collect::<HashMap<_, _>>(),
        is_file,
    })
}

/// Recursively collect all `$path` directories from the project tree.
fn collect_path_roots(node: &crate::project::ProjectNode, base: &Path, out: &mut Vec<PathBuf>) {
    if let Some(path_node) = &node.path {
        let resolved = base.join(path_node.path());
        out.push(resolved);
    }
    for child_node in node.children.values() {
        collect_path_roots(child_node, base, out);
    }
}

impl ServeSession {
    /// Shared initialization: loads the project and builds the initial
    /// snapshot tree. Used by both `new()` and `new_oneshot()`.
    fn init_tree(vfs: &Vfs, start_path: &Path) -> Result<(Project, RojoTree), ServeSessionError> {
        log::trace!("Starting new ServeSession at path {}", start_path.display());

        let root_project = Project::load_initial_project(vfs, start_path)?;

        if std::env::var("ATLAS_SEQUENTIAL").is_err() {
            let prefetch_start = Instant::now();
            match prefetch_project_files(&root_project) {
                Ok(cache) => {
                    log::info!(
                        "Prefetch total: {} files + {} canonical paths in {:.1?}",
                        cache.files.len(),
                        cache.canonical.len(),
                        prefetch_start.elapsed()
                    );
                    vfs.set_prefetch_cache(cache);
                }
                Err(err) => {
                    log::warn!("Prefetch failed, falling back to sequential reads: {err}");
                }
            }
        }

        let mut tree = RojoTree::new(InstanceSnapshot::new());
        let root_id = tree.get_root_id();
        let instance_context = InstanceContext::new();

        let snap_start = Instant::now();
        log::trace!("Generating snapshot of instances from VFS");
        let snapshot = snapshot_from_vfs(&instance_context, vfs, start_path)?;
        log::info!("Snapshot built in {:.1?}", snap_start.elapsed());

        vfs.clear_prefetch_cache();

        let patch_start = Instant::now();
        log::trace!("Computing initial patch set");
        let patch_set = compute_patch_set(snapshot, &tree, root_id);

        log::trace!("Applying initial patch set");
        apply_patch_set(&mut tree, patch_set);
        log::info!("Patch computed + applied in {:.1?}", patch_start.elapsed());

        Ok((root_project, tree))
    }

    /// Start a new serve session from the given in-memory filesystem and start
    /// path.
    ///
    /// The project file is expected to be loaded out-of-band since it's
    /// currently loaded from the filesystem directly instead of through the
    /// in-memory filesystem layer.
    pub fn new<P: AsRef<Path>>(
        vfs: Vfs,
        start_path: P,
        critical_error_receiver: Option<crossbeam_channel::Receiver<memofs::WatcherCriticalError>>,
    ) -> Result<Self, ServeSessionError> {
        let start_path = start_path.as_ref();
        let start_time = Instant::now();

        let (root_project, tree) = Self::init_tree(&vfs, start_path)?;

        let session_id = SessionId::new();
        let message_queue = MessageQueue::new();

        let tree = Arc::new(Mutex::new(tree));
        let message_queue = Arc::new(message_queue);
        let vfs = Arc::new(vfs);

        let (tree_mutation_sender, tree_mutation_receiver) = crossbeam_channel::unbounded();
        let suppressed_paths = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let ref_path_index = {
            let mut index = crate::RefPathIndex::new();
            let tree_guard = tree.lock().unwrap();
            index.populate_from_dir(root_project.folder_location(), &tree_guard);
            drop(tree_guard);
            Arc::new(Mutex::new(index))
        };

        let git_repo_root = crate::git::git_repo_root(root_project.folder_location());

        log::trace!("Starting ChangeProcessor");
        let change_processor = ChangeProcessor::start(
            Arc::clone(&tree),
            Arc::clone(&vfs),
            Arc::clone(&message_queue),
            tree_mutation_receiver,
            Arc::clone(&suppressed_paths),
            Arc::clone(&ref_path_index),
            root_project.folder_location().to_path_buf(),
            root_project.file_location.clone(),
            critical_error_receiver,
            git_repo_root.clone(),
        );

        Ok(Self {
            change_processor: Some(change_processor),
            start_time,
            session_id,
            root_project,
            tree,
            message_queue,
            tree_mutation_sender: Some(tree_mutation_sender),
            vfs,
            suppressed_paths: Some(suppressed_paths),
            ref_path_index: Some(ref_path_index),
            git_repo_root,
        })
    }

    /// Create a lightweight oneshot session that builds the project tree
    /// but does NOT start a ChangeProcessor thread or filesystem watcher.
    ///
    /// Use this for commands that only need a snapshot of the tree and
    /// don't require live updates (syncback, upload, plugin install).
    pub fn new_oneshot<P: AsRef<Path>>(vfs: Vfs, start_path: P) -> Result<Self, ServeSessionError> {
        let start_path = start_path.as_ref();
        let start_time = Instant::now();

        let (root_project, tree) = Self::init_tree(&vfs, start_path)?;

        Ok(Self {
            change_processor: None,
            start_time,
            session_id: SessionId::new(),
            root_project,
            tree: Arc::new(Mutex::new(tree)),
            message_queue: Arc::new(MessageQueue::new()),
            tree_mutation_sender: None,
            vfs: Arc::new(vfs),
            suppressed_paths: None,
            ref_path_index: None,
            git_repo_root: None,
        })
    }

    pub fn tree_handle(&self) -> Arc<Mutex<RojoTree>> {
        Arc::clone(&self.tree)
    }

    pub fn tree(&self) -> MutexGuard<'_, RojoTree> {
        self.tree.lock().unwrap()
    }

    pub fn tree_mutation_sender(&self) -> Sender<PatchSet> {
        self.tree_mutation_sender
            .clone()
            .expect("tree_mutation_sender is not available on oneshot sessions")
    }

    /// Returns a handle to the suppressed paths map, used to avoid
    /// file watcher echo when the API writes files to disk.
    #[allow(dead_code)]
    pub fn suppressed_paths(
        &self,
    ) -> Arc<Mutex<std::collections::HashMap<std::path::PathBuf, (usize, usize)>>> {
        Arc::clone(
            self.suppressed_paths
                .as_ref()
                .expect("suppressed_paths is not available on oneshot sessions"),
        )
    }

    /// Returns a handle to the Ref path index, shared between ApiService
    /// and ChangeProcessor for efficient rename path updates.
    pub fn ref_path_index(&self) -> Arc<Mutex<crate::RefPathIndex>> {
        Arc::clone(
            self.ref_path_index
                .as_ref()
                .expect("ref_path_index is not available on oneshot sessions"),
        )
    }

    #[allow(unused)]
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    pub fn message_queue(&self) -> &MessageQueue<AppliedPatchSet> {
        &self.message_queue
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn project_name(&self) -> &str {
        self.root_project
            .name
            .as_ref()
            .expect("all top-level projects must have their name set")
    }

    pub fn project_port(&self) -> Option<u16> {
        self.root_project.serve_port
    }

    pub fn place_id(&self) -> Option<u64> {
        self.root_project.place_id
    }

    pub fn game_id(&self) -> Option<u64> {
        self.root_project.game_id
    }

    pub fn start_time(&self) -> Instant {
        self.start_time
    }

    pub fn serve_place_ids(&self) -> Option<&HashSet<u64>> {
        self.root_project.serve_place_ids.as_ref()
    }

    pub fn blocked_place_ids(&self) -> Option<&HashSet<u64>> {
        self.root_project.blocked_place_ids.as_ref()
    }

    pub fn serve_address(&self) -> Option<IpAddr> {
        self.root_project.serve_address
    }

    pub fn root_dir(&self) -> &Path {
        self.root_project.folder_location()
    }

    pub fn git_repo_root(&self) -> Option<&Path> {
        self.git_repo_root.as_deref()
    }

    pub fn root_project(&self) -> &Project {
        &self.root_project
    }

    /// Returns whether sync should only include script instances.
    /// When enabled, only Script, LocalScript, and ModuleScript are synced.
    pub fn sync_scripts_only(&self) -> bool {
        self.root_project.sync_scripts_only.unwrap_or(false)
    }

    /// Returns whether hidden/internal services should be ignored during sync.
    /// When enabled, only "visible" services like Workspace, ReplicatedStorage, etc.
    /// are included in sync operations.
    ///
    /// Checks root-level `ignoreHiddenServices` first, then falls back to
    /// `syncbackRules.ignoreHiddenServices` for backward compatibility.
    /// Defaults to `true` if neither is specified.
    pub fn ignore_hidden_services(&self) -> bool {
        // Root-level setting takes precedence
        if let Some(value) = self.root_project.ignore_hidden_services {
            return value;
        }
        // Fall back to syncbackRules for backward compatibility
        self.root_project
            .syncback_rules
            .as_ref()
            .map(|rules| rules.ignore_hidden_services())
            .unwrap_or(true)
    }

    /// Read-only check: re-snapshots from disk and returns how many
    /// instances differ between the in-memory tree and the real filesystem.
    /// Does NOT apply corrections — the tree is left unchanged.
    pub fn check_tree_freshness(&self) -> TreeFreshnessReport {
        let start = Instant::now();
        let start_path: &Path = &self.root_project.file_location;
        let instance_context = InstanceContext::new();

        let snapshot = match snapshot_from_vfs(&instance_context, &self.vfs, start_path) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Tree freshness check snapshot error: {:?}", e);
                return TreeFreshnessReport {
                    is_fresh: false,
                    added: 0,
                    removed: 0,
                    updated: 0,
                    elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
                };
            }
        };

        let tree = self.tree.lock().unwrap();
        let root_id = tree.get_root_id();
        let patch_set = compute_patch_set(snapshot, &tree, root_id);

        let added = patch_set.added_instances.len();
        let removed = patch_set.removed_instances.len();
        let updated = patch_set.updated_instances.len();

        TreeFreshnessReport {
            is_fresh: added == 0 && removed == 0 && updated == 0,
            added,
            removed,
            updated,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }

    /// Re-snapshots the project tree from the real filesystem and patches
    /// the in-memory tree to correct any drift caused by missed VFS watcher
    /// events. Called on plugin connect to guarantee the tree is fresh.
    ///
    /// Skipped on freshly-started sessions (< 5 s) where the tree is
    /// guaranteed correct and the VFS watcher should handle all changes.
    ///
    /// Controlled by [`VALIDATE_TREE_ON_CONNECT`]; set that constant to
    /// `false` to disable this entirely during testing.
    pub fn validate_tree(&self) -> Vec<AppliedPatchSet> {
        if !VALIDATE_TREE_ON_CONNECT {
            log::debug!("Tree validation skipped (VALIDATE_TREE_ON_CONNECT = false)");
            return Vec::new();
        }

        // On a freshly-started session the tree was just built from the
        // filesystem and cannot be stale. Skip validation to avoid racing
        // with the VFS watcher on early file changes.
        const MIN_SESSION_AGE: std::time::Duration = std::time::Duration::from_secs(5);
        if self.start_time.elapsed() < MIN_SESSION_AGE {
            log::debug!(
                "Tree validation skipped (session age {:.1?} < {:.1?})",
                self.start_time.elapsed(),
                MIN_SESSION_AGE
            );
            return Vec::new();
        }

        let start = Instant::now();
        let start_path: &Path = &self.root_project.file_location;
        let instance_context = InstanceContext::new();

        let snapshot = match snapshot_from_vfs(&instance_context, &self.vfs, start_path) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Tree validation snapshot error: {:?}", e);
                return Vec::new();
            }
        };

        let mut tree = self.tree.lock().unwrap();
        let root_id = tree.get_root_id();
        let patch_set = compute_patch_set(snapshot, &tree, root_id);

        if patch_set.removed_instances.is_empty()
            && patch_set.added_instances.is_empty()
            && patch_set.updated_instances.is_empty()
        {
            log::info!(
                "Tree validation complete (no corrections needed) in {:.1?}",
                start.elapsed()
            );
            return Vec::new();
        }

        log::info!("Tree validation found stale state, applying corrections");
        let applied = apply_patch_set(&mut tree, patch_set);
        log::info!("Tree validation complete in {:.1?}", start.elapsed());
        vec![applied]
    }
}

#[derive(Debug, Error)]
pub enum ServeSessionError {
    #[error(transparent)]
    Io {
        #[from]
        source: io::Error,
    },

    #[error(transparent)]
    Project {
        #[from]
        source: ProjectError,
    },

    #[error(transparent)]
    Other {
        #[from]
        source: anyhow::Error,
    },
}
