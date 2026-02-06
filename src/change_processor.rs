use crossbeam_channel::{select, Receiver, RecvError, Sender};
use jod_thread::JoinHandle;
use memofs::{IoResultExt, Vfs, VfsEvent};
use rbx_dom_weak::types::{Ref, Variant};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{
    fs,
    sync::{Arc, Mutex},
};

use crate::{
    message_queue::MessageQueue,
    snapshot::{
        apply_patch_set, compute_patch_set, AppliedPatchSet, InstigatingSource, PatchSet, RojoTree,
    },
    snapshot_middleware::{snapshot_from_vfs, snapshot_project_node},
};

/// Processes file change events, updates the DOM, and sends those updates
/// through a channel for other stuff to consume.
///
/// Owns the connection between Rojo's VFS and its DOM by holding onto another
/// thread that processes messages.
///
/// Consumers of ChangeProcessor, like ServeSession, are intended to communicate
/// with this object via channels.
///
/// ChangeProcessor expects to be the only writer to the RojoTree and Vfs
/// objects passed to it.
pub struct ChangeProcessor {
    /// Controls the runtime of the processor thread. When signaled, the job
    /// thread will finish its current work and terminate.
    ///
    /// This channel should be signaled before dropping ChangeProcessor or we'll
    /// hang forever waiting for the message processing loop to terminate.
    shutdown_sender: Sender<()>,

    /// A handle to the message processing thread. When dropped, we'll block
    /// until it's done.
    ///
    /// Allowed to be unused because dropping this value has side effects.
    #[allow(unused)]
    job_thread: JoinHandle<Result<(), RecvError>>,
}

impl ChangeProcessor {
    /// Spin up the ChangeProcessor, connecting it to the given tree, VFS, and
    /// outbound message queue.
    pub fn start(
        tree: Arc<Mutex<RojoTree>>,
        vfs: Arc<Vfs>,
        message_queue: Arc<MessageQueue<AppliedPatchSet>>,
        tree_mutation_receiver: Receiver<PatchSet>,
        suppressed_paths: Arc<Mutex<std::collections::HashSet<PathBuf>>>,
    ) -> Self {
        let (shutdown_sender, shutdown_receiver) = crossbeam_channel::bounded(1);
        let vfs_receiver = vfs.event_receiver();
        let task = JobThreadContext {
            tree,
            vfs,
            message_queue,
            pending_recovery: Mutex::new(Vec::new()),
            suppressed_paths,
        };

        let job_thread = jod_thread::Builder::new()
            .name("ChangeProcessor thread".to_owned())
            .spawn(move || {
                log::trace!("ChangeProcessor thread started");

                loop {
                    select! {
                        recv(vfs_receiver) -> event => {
                            task.handle_vfs_event(event?);
                            task.process_pending_recoveries();
                        },
                        recv(tree_mutation_receiver) -> patch_set => {
                            task.handle_tree_event(patch_set?);
                        },
                        recv(shutdown_receiver) -> _ => {
                            log::trace!("ChangeProcessor shutdown signal received...");
                            return Ok(());
                        },
                        default(Duration::from_millis(500)) => {
                            // Periodic sweep even when no events arrive —
                            // catches files recreated after their Remove was processed.
                            task.process_pending_recoveries();
                        },
                    }
                }
            })
            .expect("Could not start ChangeProcessor thread");

        Self {
            shutdown_sender,
            job_thread,
        }
    }
}

impl Drop for ChangeProcessor {
    fn drop(&mut self) {
        // Signal the job thread to start spinning down. Without this we'll hang
        // forever waiting for the thread to finish its infinite loop.
        let _ = self.shutdown_sender.send(());

        // After this function ends, the job thread will be joined. It might
        // block for a small period of time while it processes its last work.
    }
}

/// Contains all of the state needed to synchronize the DOM and VFS.
struct JobThreadContext {
    /// A handle to the DOM we're managing.
    tree: Arc<Mutex<RojoTree>>,

    /// A handle to the VFS we're managing.
    vfs: Arc<Vfs>,

    /// Whenever changes are applied to the DOM, we should push those changes
    /// into this message queue to inform any connected clients.
    message_queue: Arc<MessageQueue<AppliedPatchSet>>,

    /// Paths recently removed from the tree that should be re-checked after a
    /// delay. On Windows, rapid delete+recreate (e.g., editor undo) can cause
    /// the Remove event to arrive but the Create event to be lost. We record
    /// removed paths here and periodically verify they are still gone.
    pending_recovery: Mutex<Vec<(PathBuf, Instant)>>,

    /// Paths recently written by the API's syncback. Events for these paths
    /// are suppressed (one-shot) to avoid redundant re-snapshots.
    suppressed_paths: Arc<Mutex<std::collections::HashSet<PathBuf>>>,
}

impl JobThreadContext {
    /// Computes and applies patches to the DOM for a given file path.
    ///
    /// This function finds the nearest ancestor to the given path that has associated instances
    /// in the tree.
    /// It then computes and applies changes for each affected instance ID and
    /// returns a vector of applied patch sets.
    fn apply_patches(&self, path: PathBuf) -> Vec<AppliedPatchSet> {
        let mut tree = self.tree.lock().unwrap();
        let mut applied_patches = Vec::new();

        // Find the nearest ancestor to this path that has
        // associated instances in the tree. This helps make sure
        // that we handle additions correctly, especially if we
        // receive events for descendants of a large tree being
        // created all at once.
        let mut current_path = path.as_path();
        let affected_ids = loop {
            let ids = tree.get_ids_at_path(current_path);

            log::info!(
                "apply_patches: path {} affects IDs {:?}",
                current_path.display(),
                ids
            );

            if !ids.is_empty() {
                break ids.to_vec();
            }

            log::info!("apply_patches: no IDs at {}, trying parent...", current_path.display());
            match current_path.parent() {
                Some(parent) => current_path = parent,
                None => break Vec::new(),
            }
        };

        if affected_ids.is_empty() {
            log::info!(
                "apply_patches: no affected instances found for path {}",
                path.display()
            );
        }

        for id in affected_ids {
            if let Some(result) = compute_and_apply_changes(&mut tree, &self.vfs, id) {
                // If an instance was removed, schedule a recovery check
                // in case the path is recreated momentarily.
                if let Some(removed_path) = result.removed_path {
                    let mut pending = self.pending_recovery.lock().unwrap();
                    pending.push((removed_path, Instant::now()));
                }

                if !result.applied.is_empty() {
                    applied_patches.push(result.applied);
                }
            }
        }

        applied_patches
    }

    fn handle_vfs_event(&self, event: VfsEvent) {
        // Log EVERY VFS event at INFO level for diagnostics.
        // This is intentionally verbose — it is critical for debugging
        // file watcher desync issues (e.g., rapid delete+recreate).
        match &event {
            VfsEvent::Create(path) => log::info!("VFS event: CREATE {}", path.display()),
            VfsEvent::Write(path) => log::info!("VFS event: WRITE {}", path.display()),
            VfsEvent::Remove(path) => log::info!("VFS event: REMOVE {}", path.display()),
            _ => log::info!("VFS event: OTHER {:?}", event),
        }

        // Check if this event should be suppressed (one-shot, from API syncback).
        let event_path = match &event {
            VfsEvent::Create(p) | VfsEvent::Write(p) | VfsEvent::Remove(p) => Some(p.clone()),
            _ => None,
        };
        if let Some(ref path) = event_path {
            let mut suppressed = self.suppressed_paths.lock().unwrap();
            if suppressed.remove(path) {
                // Still commit so VFS stays consistent, but skip patching.
                self.vfs
                    .commit_event(&event)
                    .expect("Error applying VFS change");
                log::info!("VFS event SUPPRESSED (API syncback echo): {}", path.display());
                return;
            }
        }

        // Update the VFS immediately with the event.
        self.vfs
            .commit_event(&event)
            .expect("Error applying VFS change");

        // For a given VFS event, we might have many changes to different parts
        // of the tree. Calculate and apply all of these changes.
        let applied_patches = match event {
            VfsEvent::Create(path) | VfsEvent::Write(path) => {
                // The path might have been deleted before we could process this event
                // (e.g., during syncback when directories are being removed).
                // If canonicalize fails, retry once after a brief delay — on Windows,
                // the file may still be locked by the editor process.
                match self.vfs.canonicalize(&path) {
                    Ok(canonical_path) => {
                        log::info!(
                            "VFS: canonicalize OK for {} -> {}",
                            path.display(),
                            canonical_path.display()
                        );
                        self.apply_patches(canonical_path)
                    }
                    Err(err) => {
                        log::info!(
                            "VFS: canonicalize FAILED for {} ({}), retrying after 50ms...",
                            path.display(),
                            err
                        );
                        std::thread::sleep(Duration::from_millis(50));
                        match self.vfs.canonicalize(&path) {
                            Ok(canonical_path) => {
                                log::info!(
                                    "VFS: retry canonicalize OK for {} -> {}",
                                    path.display(),
                                    canonical_path.display()
                                );
                                self.apply_patches(canonical_path)
                            }
                            Err(err2) => {
                                log::warn!(
                                    "VFS: Create/Write event DROPPED for {} after retry — \
                                     canonicalize failed: {}. File may be desynchronized!",
                                    path.display(),
                                    err2
                                );
                                Vec::new()
                            }
                        }
                    }
                }
            }
            VfsEvent::Remove(path) => {
                // MemoFS does not track parent removals yet, so we can canonicalize
                // the parent path safely and then append the removed path's file name.
                // However, if the parent was also deleted (e.g., when deleting a directory
                // tree), canonicalize will fail - in that case, just skip this event.
                let parent = path.parent().unwrap();
                let file_name = path.file_name().unwrap();
                match self.vfs.canonicalize(parent) {
                    Ok(parent_normalized) => {
                        let resolved = parent_normalized.join(file_name);
                        log::info!(
                            "VFS: Remove resolved to {}",
                            resolved.display()
                        );
                        self.apply_patches(resolved)
                    }
                    Err(err) => {
                        log::info!(
                            "VFS: Skipping remove event for {} — parent no longer exists: {}",
                            path.display(),
                            err
                        );
                        Vec::new()
                    }
                }
            }
            _ => {
                log::warn!("Unhandled VFS event: {:?}", event);
                Vec::new()
            }
        };

        // Log patch application summary at INFO level
        if !applied_patches.is_empty() {
            let total_added: usize = applied_patches.iter().map(|p| p.added.len()).sum();
            let total_removed: usize = applied_patches.iter().map(|p| p.removed.len()).sum();
            let total_updated: usize = applied_patches.iter().map(|p| p.updated.len()).sum();
            if total_added > 0 || total_removed > 0 || total_updated > 0 {
                log::info!(
                    "VFS event applied: {} added, {} removed, {} updated",
                    total_added,
                    total_removed,
                    total_updated
                );
            }
        } else {
            log::info!("VFS event applied: no changes");
        }

        // Notify anyone listening to the message queue about the changes we
        // just made.
        self.message_queue.push_messages(&applied_patches);
    }

    /// Processes any pending recovery checks for paths that were recently
    /// removed. If a path has reappeared on the real filesystem after the
    /// recovery delay, we trigger a re-snapshot to bring the tree back in sync.
    fn process_pending_recoveries(&self) {
        const RECOVERY_DELAY: Duration = Duration::from_millis(200);

        let ready: Vec<PathBuf> = {
            let mut pending = self.pending_recovery.lock().unwrap();
            let now = Instant::now();

            // Drain entries that are old enough to check
            let mut ready = Vec::new();
            pending.retain(|(path, recorded_at)| {
                if now.duration_since(*recorded_at) >= RECOVERY_DELAY {
                    ready.push(path.clone());
                    false // remove from pending
                } else {
                    true // keep in pending
                }
            });
            ready
        };

        for path in ready {
            if std::fs::metadata(&path).is_ok() {
                log::info!(
                    "VFS recovery: path {} was removed but has reappeared on disk. Re-snapshotting.",
                    path.display()
                );
                let patches = self.apply_patches(path);
                if !patches.is_empty() {
                    let total_added: usize = patches.iter().map(|p| p.added.len()).sum();
                    let total_removed: usize = patches.iter().map(|p| p.removed.len()).sum();
                    let total_updated: usize = patches.iter().map(|p| p.updated.len()).sum();
                    log::info!(
                        "VFS recovery applied: {} added, {} removed, {} updated",
                        total_added,
                        total_removed,
                        total_updated
                    );
                    self.message_queue.push_messages(&patches);
                }
            } else {
                log::info!(
                    "VFS recovery: path {} confirmed removed from disk.",
                    path.display()
                );
            }
        }
    }

    fn handle_tree_event(&self, patch_set: PatchSet) {
        // Log incoming patch summary at debug level
        log::debug!(
            "Processing client patch: {} removed, {} added, {} updated",
            patch_set.removed_instances.len(),
            patch_set.added_instances.len(),
            patch_set.updated_instances.len()
        );

        let applied_patch = {
            let mut tree = self.tree.lock().unwrap();

            for &id in &patch_set.removed_instances {
                if let Some(instance) = tree.get_instance(id) {
                    if let Some(instigating_source) = &instance.metadata().instigating_source {
                        match instigating_source {
                            InstigatingSource::Path(path) => {
                                // Guard: file may already be deleted by the API's
                                // syncback_removed_instance before this PatchSet arrives.
                                if path.exists() {
                                    if path.is_dir() {
                                        log::info!(
                                            "Two-way sync: Removing directory {}",
                                            path.display()
                                        );
                                        if let Err(err) = fs::remove_dir_all(path) {
                                            log::error!(
                                                "Failed to remove directory {:?} for instance {:?}: {}",
                                                path,
                                                id,
                                                err
                                            );
                                        }
                                    } else {
                                        log::info!(
                                            "Two-way sync: Removing file {}",
                                            path.display()
                                        );
                                        if let Err(err) = fs::remove_file(path) {
                                            log::error!(
                                                "Failed to remove file {:?} for instance {:?}: {}",
                                                path,
                                                id,
                                                err
                                            );
                                        }
                                    }
                                } else {
                                    log::info!(
                                        "Two-way sync: File already removed: {}",
                                        path.display()
                                    );
                                }
                            }
                            InstigatingSource::ProjectNode { .. } => {
                                log::warn!(
                                    "Cannot remove instance {:?}, it's from a project file",
                                    id
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "Cannot remove instance {:?}, it is not an instigating source.",
                            id
                        );
                    }
                } else {
                    log::warn!("Cannot remove instance {:?}, it does not exist.", id);
                }
            }

            for update in &patch_set.updated_instances {
                let id = update.id;

                if let Some(instance) = tree.get_instance(id) {
                    // Handle instance rename on disk
                    if let Some(ref new_name) = update.changed_name {
                        if let Some(instigating_source) = &instance.metadata().instigating_source {
                            match instigating_source {
                                InstigatingSource::Path(path) => {
                                    if path.exists() {
                                        let old_name = instance.name();
                                        // Compute new path by replacing the old name
                                        // in the filename/directory name
                                        if let Some(parent) = path.parent() {
                                            let old_file_name = path
                                                .file_name()
                                                .and_then(|f| f.to_str())
                                                .unwrap_or("");
                                            let new_file_name =
                                                old_file_name.replacen(old_name, new_name, 1);
                                            let new_path = parent.join(&new_file_name);

                                            if new_path != *path {
                                                log::info!(
                                                    "Two-way sync: Renaming {} -> {}",
                                                    path.display(),
                                                    new_path.display()
                                                );
                                                if let Err(err) = fs::rename(path, &new_path) {
                                                    log::error!(
                                                        "Failed to rename {:?} to {:?}: {}",
                                                        path,
                                                        new_path,
                                                        err
                                                    );
                                                } else {
                                                    // Also rename adjacent meta file if it exists
                                                    let old_meta =
                                                        parent.join(format!("{}.meta.json5", old_name));
                                                    if old_meta.exists() {
                                                        let new_meta = parent
                                                            .join(format!("{}.meta.json5", new_name));
                                                        let _ = fs::rename(&old_meta, &new_meta);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                InstigatingSource::ProjectNode { .. } => {
                                    log::warn!(
                                        "Cannot rename instance {:?} — defined in project file",
                                        id
                                    );
                                }
                            }
                        } else {
                            log::warn!(
                                "Cannot rename instance {:?} — no instigating source",
                                id
                            );
                        }
                    }

                    // Handle ClassName changes (script class transitions)
                    if let Some(ref new_class) = update.changed_class_name {
                        if let Some(instigating_source) = &instance.metadata().instigating_source {
                            match instigating_source {
                                InstigatingSource::Path(path) => {
                                    let old_class = instance.class_name();
                                    let old_is_script = matches!(
                                        old_class.as_str(),
                                        "ModuleScript" | "Script" | "LocalScript"
                                    );
                                    let new_is_script = matches!(
                                        new_class.as_str(),
                                        "ModuleScript" | "Script" | "LocalScript"
                                    );

                                    if old_is_script && new_is_script && path.exists() {
                                        // Script-to-script transition: rename the file extension
                                        if let Some(parent) = path.parent() {
                                            let name = instance.name();
                                            let new_suffix = match new_class.as_str() {
                                                "ModuleScript" => "",
                                                "Script" => ".server",
                                                "LocalScript" => ".local",
                                                _ => "",
                                            };
                                            let is_init = path
                                                .file_name()
                                                .and_then(|f| f.to_str())
                                                .map(|f| f.starts_with("init."))
                                                .unwrap_or(false);

                                            let new_file_name = if is_init {
                                                if new_suffix.is_empty() {
                                                    "init.luau".to_string()
                                                } else {
                                                    format!("init{}.luau", new_suffix)
                                                }
                                            } else if new_suffix.is_empty() {
                                                format!("{}.luau", name)
                                            } else {
                                                format!("{}{}.luau", name, new_suffix)
                                            };

                                            let new_path = parent.join(&new_file_name);
                                            if new_path != *path {
                                                log::info!(
                                                    "Two-way sync: Changing class {} -> {}, \
                                                     renaming {} -> {}",
                                                    old_class,
                                                    new_class,
                                                    path.display(),
                                                    new_path.display()
                                                );
                                                if let Err(err) = fs::rename(path, &new_path) {
                                                    log::error!(
                                                        "Failed to rename {:?} to {:?}: {}",
                                                        path,
                                                        new_path,
                                                        err
                                                    );
                                                }
                                            }
                                        }
                                    } else if old_is_script != new_is_script {
                                        log::warn!(
                                            "Cannot change ClassName from {} to {} — \
                                             cross-category changes (script <-> non-script) \
                                             are not yet supported",
                                            old_class,
                                            new_class
                                        );
                                    }
                                }
                                InstigatingSource::ProjectNode { .. } => {
                                    log::warn!(
                                        "Cannot change ClassName for {:?} — defined in project file",
                                        id
                                    );
                                }
                            }
                        }
                    }

                    if update.changed_metadata.is_some() {
                        log::warn!("Cannot change metadata yet.");
                    }

                    for (key, changed_value) in &update.changed_properties {
                        if key == "Source" {
                            if let Some(instigating_source) =
                                &instance.metadata().instigating_source
                            {
                                match instigating_source {
                                    InstigatingSource::Path(path) => {
                                        if let Some(Variant::String(value)) = changed_value {
                                            log::info!(
                                                "Two-way sync: Writing Source to {}",
                                                path.display()
                                            );
                                            if let Err(err) = fs::write(path, value) {
                                                log::error!(
                                                    "Failed to write Source to {:?} for instance {:?}: {}",
                                                    path,
                                                    id,
                                                    err
                                                );
                                            }
                                        } else {
                                            log::warn!("Cannot change Source to non-string value.");
                                        }
                                    }
                                    InstigatingSource::ProjectNode { .. } => {
                                        log::warn!(
                                            "Cannot update instance {:?}, it's from a project file",
                                            id
                                        );
                                    }
                                }
                            } else {
                                log::warn!(
                                    "Cannot update instance {:?}, it is not an instigating source.",
                                    id
                                );
                            }
                        } else {
                            log::trace!("Skipping non-Source property change: {}", key);
                        }
                    }
                } else {
                    log::warn!("Cannot update instance {:?}, it does not exist.", id);
                }
            }

            apply_patch_set(&mut tree, patch_set)
        };

        if !applied_patch.is_empty() {
            self.message_queue.push_messages(&[applied_patch]);
        }
    }
}

/// Result of computing and applying changes to a single instance.
/// Includes the path that was removed (if any) for pending recovery tracking.
struct ComputeResult {
    applied: AppliedPatchSet,
    /// If the instance was removed because its path no longer exists,
    /// record the path here so the caller can schedule a recovery check.
    removed_path: Option<PathBuf>,
}

fn compute_and_apply_changes(tree: &mut RojoTree, vfs: &Vfs, id: Ref) -> Option<ComputeResult> {
    let metadata = tree
        .get_metadata(id)
        .expect("metadata missing for instance present in tree");

    let instigating_source = match &metadata.instigating_source {
        Some(path) => path,
        None => {
            log::error!(
                "Instance {:?} did not have an instigating source, but was considered for an update.",
                id
            );
            log::error!("This is a bug. Please file an issue!");
            return None;
        }
    };

    // How we process a file change event depends on what created this
    // file/folder in the first place.
    match instigating_source {
        InstigatingSource::Path(path) => {
            log::info!(
                "compute_and_apply_changes: checking path {} for instance {:?}",
                path.display(),
                id
            );

            match vfs.metadata(path).with_not_found() {
                Ok(Some(_)) => {
                    // Our instance was previously created from a path and that
                    // path still exists. We can generate a snapshot starting at
                    // that path and use it as the source for our patch.
                    log::info!(
                        "compute_and_apply_changes: path EXISTS via VFS, re-snapshotting {}",
                        path.display()
                    );

                    let snapshot = match snapshot_from_vfs(&metadata.context, vfs, path) {
                        Ok(snapshot) => snapshot,
                        Err(err) => {
                            log::error!("Snapshot error: {:?}", err);
                            return None;
                        }
                    };

                    let patch_set = compute_patch_set(snapshot, tree, id);
                    let applied = apply_patch_set(tree, patch_set);
                    Some(ComputeResult {
                        applied,
                        removed_path: None,
                    })
                }
                Ok(None) => {
                    // Path not found via VFS. Before removing, verify it's truly
                    // gone from the REAL filesystem. On Windows, rapid
                    // delete+recreate (e.g., editor undo) can cause Remove events
                    // where the file has already been recreated by the time we
                    // process the event.
                    if std::fs::metadata(path).is_ok() {
                        log::info!(
                            "compute_and_apply_changes: VFS says path removed but REAL \
                             filesystem confirms it EXISTS: {}. Re-snapshotting instead \
                             of removing.",
                            path.display()
                        );

                        let snapshot = match snapshot_from_vfs(&metadata.context, vfs, path) {
                            Ok(snapshot) => snapshot,
                            Err(err) => {
                                log::error!("Recovery snapshot error for {}: {:?}", path.display(), err);
                                return None;
                            }
                        };

                        let patch_set = compute_patch_set(snapshot, tree, id);
                        let applied = apply_patch_set(tree, patch_set);
                        Some(ComputeResult {
                            applied,
                            removed_path: None,
                        })
                    } else {
                        // Path is genuinely gone from both VFS and real filesystem.
                        // Remove the instance, but record the path for recovery
                        // checking — the file might be recreated momentarily.
                        log::info!(
                            "compute_and_apply_changes: path NOT FOUND on disk, removing \
                             instance {:?} for {}. Scheduling recovery check.",
                            id,
                            path.display()
                        );

                        let removed_path = path.to_path_buf();
                        let mut patch_set = PatchSet::new();
                        patch_set.removed_instances.push(id);

                        let applied = apply_patch_set(tree, patch_set);
                        Some(ComputeResult {
                            applied,
                            removed_path: Some(removed_path),
                        })
                    }
                }
                Err(err) => {
                    log::error!(
                        "Error processing filesystem change for {}: {:?}",
                        path.display(),
                        err
                    );
                    None
                }
            }
        }

        InstigatingSource::ProjectNode {
            path,
            name,
            node,
            parent_class,
        } => {
            // This instance is the direct subject of a project node. Since
            // there might be information associated with our instance from
            // the project file, we snapshot the entire project node again.
            log::info!(
                "compute_and_apply_changes: re-snapshotting project node '{}' at {}",
                name,
                path.display()
            );

            let snapshot_result = snapshot_project_node(
                &metadata.context,
                path,
                name,
                node,
                vfs,
                parent_class.as_ref().map(|name| name.as_str()),
            );

            let snapshot = match snapshot_result {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    log::error!("{:?}", err);
                    return None;
                }
            };

            let patch_set = compute_patch_set(snapshot, tree, id);
            let applied = apply_patch_set(tree, patch_set);
            Some(ComputeResult {
                applied,
                removed_path: None,
            })
        }
    }
}
