use crossbeam_channel::{select, Receiver, RecvError, Sender};
use jod_thread::JoinHandle;
use memofs::{IoResultExt, Vfs, VfsEvent};
use rbx_dom_weak::types::{Ref, Variant};
use std::path::{Path, PathBuf};
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
    syncback::{name_needs_slugify, slugify_name},
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
        suppressed_paths: Arc<Mutex<std::collections::HashMap<PathBuf, (usize, usize)>>>,
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
    /// are suppressed to avoid redundant re-snapshots. Values are `(remove_count, create_write_count)`.
    suppressed_paths: Arc<Mutex<std::collections::HashMap<PathBuf, (usize, usize)>>>,
}

impl JobThreadContext {
    /// Find the init file inside a directory-format script folder.
    /// Returns the path to the first `init.*.luau` or `init.*.lua` found.
    fn find_init_file(dir: &Path) -> Option<PathBuf> {
        // Check known init file names in priority order
        let candidates = [
            "init.luau",
            "init.server.luau",
            "init.client.luau",
            "init.local.luau",
            "init.plugin.luau",
            "init.legacy.luau",
            "init.lua",
            "init.server.lua",
            "init.client.lua",
            "init.local.lua",
        ];
        for name in &candidates {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
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
    /// Prevents re-processing a file-watcher event we triggered ourselves.
    fn suppress_path(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        suppressed.entry(key).or_insert((0, 0)).1 += 1;
    }

    /// Remove a Create/Write suppression previously added by [`suppress_path`].
    /// Called when the filesystem operation failed, so that future VFS events
    /// for that path are not incorrectly swallowed.
    fn unsuppress_path(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        if let Some(counts) = suppressed.get_mut(&key) {
            counts.1 = counts.1.saturating_sub(1);
            if counts.0 == 0 && counts.1 == 0 {
                suppressed.remove(&key);
            }
        }
    }

    /// Suppress the next VFS event of ANY type for the given path.
    /// Used for the **old** path of renames: different platforms deliver
    /// different event types for the source of a rename (REMOVE on
    /// Linux/Windows, stale CREATE on macOS FSEvents).
    fn suppress_path_any(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        let entry = suppressed.entry(key).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += 1;
    }

    /// Remove both counters previously added by [`suppress_path_any`].
    fn unsuppress_path_any(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        if let Some(counts) = suppressed.get_mut(&key) {
            counts.0 = counts.0.saturating_sub(1);
            counts.1 = counts.1.saturating_sub(1);
            if counts.0 == 0 && counts.1 == 0 {
                suppressed.remove(&key);
            }
        }
    }

    /// Upsert the `name` field in a `.meta.json5` file, suppressing filesystem
    /// events to avoid feedback loops.
    fn upsert_meta_name_field(&self, meta_path: &Path, real_name: &str) {
        self.suppress_path(meta_path);
        if let Err(err) = crate::syncback::meta::upsert_meta_name(meta_path, real_name) {
            self.unsuppress_path(meta_path);
            log::error!(
                "Failed to upsert name in meta file {}: {}",
                meta_path.display(),
                err
            );
        }
    }

    /// Remove the `name` field from a `.meta.json5` file, suppressing filesystem
    /// events. If the file becomes empty after removal, deletes it entirely.
    fn remove_meta_name_field(&self, meta_path: &Path) {
        use crate::syncback::meta::RemoveNameOutcome;
        // Suppress for the write/delete that may follow
        self.suppress_path(meta_path);
        match crate::syncback::meta::remove_meta_name(meta_path) {
            Ok(RemoveNameOutcome::NoOp) => {
                self.unsuppress_path(meta_path);
            }
            Ok(RemoveNameOutcome::FileDeleted) => {
                // File was deleted — also suppress the Remove event.
                // suppress_path already covers Create/Write; we need
                // suppress_path_any for the Remove event as well.
                self.suppress_path_any(meta_path);
            }
            Ok(RemoveNameOutcome::FieldRemoved) => {
                // File was rewritten — suppress_path already covers it.
            }
            Err(err) => {
                self.unsuppress_path(meta_path);
                log::error!(
                    "Failed to remove name from meta file {}: {}",
                    meta_path.display(),
                    err
                );
            }
        }
    }

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

            log::info!(
                "apply_patches: no IDs at {}, trying parent...",
                current_path.display()
            );
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
        // Suppressions are event-type-aware: a Remove suppression only matches
        // Remove events, and a Create/Write suppression only matches Create/Write
        // events. This prevents a Remove suppression from consuming a Create event
        // (which can happen on macOS when FSEvents coalesces delete+recreate).
        let event_path = match &event {
            VfsEvent::Create(p) | VfsEvent::Write(p) | VfsEvent::Remove(p) => Some(p.clone()),
            _ => None,
        };
        if let Some(ref path) = event_path {
            let canonical = std::fs::canonicalize(path).ok();
            let mut suppressed = self.suppressed_paths.lock().unwrap();
            // Determine which key matches: try canonical first (most likely
            // to match what suppress_path stored), then fall back to raw.
            let matched_key = canonical
                .as_ref()
                .filter(|c| suppressed.contains_key(c.as_path()))
                .cloned()
                .or_else(|| {
                    if suppressed.contains_key(path) {
                        Some(path.clone())
                    } else {
                        None
                    }
                });
            if let Some(key) = matched_key {
                if let Some(counts) = suppressed.get_mut(&key) {
                    let should_suppress = match &event {
                        VfsEvent::Remove(_) if counts.0 > 0 => {
                            counts.0 -= 1;
                            true
                        }
                        VfsEvent::Create(_) | VfsEvent::Write(_) if counts.1 > 0 => {
                            counts.1 -= 1;
                            true
                        }
                        _ => false,
                    };
                    if counts.0 == 0 && counts.1 == 0 {
                        suppressed.remove(&key);
                    }
                    if should_suppress {
                        drop(suppressed);
                        // Still commit so VFS stays consistent, but skip patching.
                        self.vfs
                            .commit_event(&event)
                            .expect("Error applying VFS change");
                        log::info!(
                            "VFS event SUPPRESSED (API syncback echo): {}",
                            path.display()
                        );
                        return;
                    }
                }
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
                match self.vfs.canonicalize(&path) {
                    Ok(canonical_path) => {
                        log::info!(
                            "VFS: canonicalize OK for {} -> {}",
                            path.display(),
                            canonical_path.display()
                        );
                        self.apply_patches(canonical_path)
                    }
                    Err(_) => {
                        // The path doesn't exist on disk. Two possible causes:
                        //
                        // 1. Phantom event from a rename we performed — macOS FSEvents
                        //    can deliver a stale CREATE for the old path. If a pending
                        //    suppression exists for this path, consume it and skip.
                        //
                        // 2. The file was deleted between the event firing and us
                        //    processing it. Fall back to parent-directory canonicalize
                        //    (same strategy as the Remove handler) so the tree can
                        //    reconcile the disappearance.
                        let consumed = {
                            let key = Self::suppression_key(&path);
                            let mut suppressed = self.suppressed_paths.lock().unwrap();
                            if let Some(counts) = suppressed.get_mut(&key) {
                                if counts.0 > 0 || counts.1 > 0 {
                                    // Drain whichever counter is available
                                    if counts.1 > 0 {
                                        counts.1 -= 1;
                                    } else {
                                        counts.0 -= 1;
                                    }
                                    if counts.0 == 0 && counts.1 == 0 {
                                        suppressed.remove(&key);
                                    }
                                    true
                                } else {
                                    false
                                }
                            } else {
                                false
                            }
                        };

                        if consumed {
                            log::info!(
                                "VFS: phantom Create/Write for non-existent {} — \
                                 consumed pending suppression (likely stale rename event)",
                                path.display()
                            );
                            Vec::new()
                        } else {
                            // No suppression — the file genuinely vanished. Use the
                            // parent-directory fallback so the tree can reconcile.
                            let parent = path.parent().unwrap();
                            let file_name = path.file_name().unwrap();
                            match self.vfs.canonicalize(parent) {
                                Ok(parent_normalized) => {
                                    let resolved = parent_normalized.join(file_name);
                                    log::info!(
                                        "VFS: Create/Write for vanished file {} — \
                                         resolved via parent to {}",
                                        path.display(),
                                        resolved.display()
                                    );
                                    self.apply_patches(resolved)
                                }
                                Err(err) => {
                                    log::info!(
                                        "VFS: Skipping Create/Write for {} — \
                                         parent no longer exists: {}",
                                        path.display(),
                                        err
                                    );
                                    Vec::new()
                                }
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
                        log::info!("VFS: Remove resolved to {}", resolved.display());
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

            // NOTE: We do NOT delete files from disk here. The API handler
            // (handle_api_write → syncback_removed_instance) already deleted
            // the files before sending this PatchSet. Re-deleting would
            // destroy any new file created at the same path — e.g., when an
            // instance is renamed from "joe_test" to "joe/test", the new slug
            // is also "joe_test", so the API creates a new file at the same
            // path that the removal just deleted. If we deleted again here,
            // the new file would be destroyed.
            //
            // The PatchSet removal only needs to update the DOM tree (handled
            // by apply_patch_set below).
            for &id in &patch_set.removed_instances {
                if let Some(instance) = tree.get_instance(id) {
                    if let Some(instigating_source) = &instance.metadata().instigating_source {
                        match instigating_source {
                            InstigatingSource::Path(path) => {
                                log::info!(
                                    "Two-way sync: Removing instance {:?} from tree (path: {})",
                                    id,
                                    path.display()
                                );
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

            // Collect (Ref, new_instigating_source) for instances whose
            // filesystem path changed (rename / ClassName transition).
            // Applied after the PatchSet to keep metadata in sync.
            let mut metadata_updates: Vec<(Ref, PathBuf)> = Vec::new();

            for update in &patch_set.updated_instances {
                let id = update.id;

                if let Some(instance) = tree.get_instance(id) {
                    // Track the current source file path — rename and ClassName
                    // handlers may move the file, so the Source write must target
                    // the new location instead of the stale instigating_source.
                    let mut overridden_source_path: Option<PathBuf> = None;

                    // Handle instance rename on disk
                    if let Some(ref new_name) = update.changed_name {
                        if let Some(instigating_source) = &instance.metadata().instigating_source {
                            match instigating_source {
                                InstigatingSource::Path(path) => {
                                    if path.exists() {
                                        let file_name =
                                            path.file_name().and_then(|f| f.to_str()).unwrap_or("");

                                        // For directory-format scripts, the instigating_source
                                        // is the init file (e.g., src/MyModule/init.luau).
                                        // The instance name corresponds to the parent directory,
                                        // not the init file, so we must rename the directory.
                                        let is_init_file = file_name.starts_with("init.");

                                        if is_init_file {
                                            let dir_path = path.parent().unwrap();
                                            if let Some(grandparent) = dir_path.parent() {
                                                let dir_name = dir_path
                                                    .file_name()
                                                    .and_then(|f| f.to_str())
                                                    .unwrap_or("");
                                                // Slugify the new name for filesystem safety
                                                let slugified_new_name =
                                                    if name_needs_slugify(new_name) {
                                                        slugify_name(new_name)
                                                    } else {
                                                        new_name.to_string()
                                                    };
                                                let new_dir_path =
                                                    grandparent.join(&slugified_new_name);

                                                // Track the effective directory path after any rename
                                                let effective_dir_path;

                                                if new_dir_path != dir_path {
                                                    log::info!(
                                                        "Two-way sync: Renaming directory {} -> {}",
                                                        dir_path.display(),
                                                        new_dir_path.display()
                                                    );
                                                    self.suppress_path_any(dir_path);
                                                    self.suppress_path(&new_dir_path);
                                                    if let Err(err) =
                                                        fs::rename(dir_path, &new_dir_path)
                                                    {
                                                        self.unsuppress_path_any(dir_path);
                                                        self.unsuppress_path(&new_dir_path);
                                                        log::error!(
                                                            "Failed to rename directory {:?} to {:?}: {}",
                                                            dir_path,
                                                            new_dir_path,
                                                            err
                                                        );
                                                        effective_dir_path = dir_path.to_path_buf();
                                                    } else {
                                                        // The init file moved with the directory.
                                                        overridden_source_path =
                                                            Some(new_dir_path.join(file_name));
                                                        let old_meta = grandparent.join(format!(
                                                            "{}.meta.json5",
                                                            dir_name
                                                        ));
                                                        if old_meta.exists() {
                                                            let new_meta =
                                                                grandparent.join(format!(
                                                                    "{}.meta.json5",
                                                                    slugified_new_name
                                                                ));
                                                            self.suppress_path_any(&old_meta);
                                                            self.suppress_path(&new_meta);
                                                            if fs::rename(&old_meta, &new_meta)
                                                                .is_err()
                                                            {
                                                                self.unsuppress_path_any(&old_meta);
                                                                self.unsuppress_path(&new_meta);
                                                            }
                                                        }
                                                        effective_dir_path = new_dir_path.clone();
                                                    }
                                                } else {
                                                    effective_dir_path = dir_path.to_path_buf();
                                                }

                                                // Always update the meta name field when the
                                                // instance name changed, even if the directory
                                                // path didn't change (e.g. "Foo/Bar" → "Foo|Bar"
                                                // both slugify to "Foo_Bar").
                                                let init_meta =
                                                    effective_dir_path.join("init.meta.json5");
                                                if slugified_new_name != *new_name {
                                                    self.upsert_meta_name_field(
                                                        &init_meta, new_name,
                                                    );
                                                } else {
                                                    self.remove_meta_name_field(&init_meta);
                                                }
                                            }
                                        } else if let Some(parent) = path.parent() {
                                            // Derive the suffix from the existing filename
                                            // rather than using replacen with the instance
                                            // name, which may differ from the filesystem
                                            // name (e.g., slugified names).
                                            let extension = path
                                                .extension()
                                                .and_then(|e| e.to_str())
                                                .unwrap_or("");
                                            let stem = path
                                                .file_stem()
                                                .and_then(|s| s.to_str())
                                                .unwrap_or("");
                                            let known_suffixes = [
                                                ".server", ".client", ".plugin", ".local",
                                                ".legacy",
                                            ];
                                            let script_suffix = known_suffixes
                                                .iter()
                                                .find(|s| stem.ends_with(*s))
                                                .copied()
                                                .unwrap_or("");
                                            let old_base = if script_suffix.is_empty() {
                                                stem
                                            } else {
                                                &stem[..stem.len() - script_suffix.len()]
                                            };

                                            let slugified_new_name = if name_needs_slugify(new_name)
                                            {
                                                slugify_name(new_name)
                                            } else {
                                                new_name.to_string()
                                            };
                                            let new_file_name = if extension.is_empty() {
                                                format!("{}{}", slugified_new_name, script_suffix)
                                            } else {
                                                format!(
                                                    "{}{}.{}",
                                                    slugified_new_name, script_suffix, extension
                                                )
                                            };
                                            let new_path = parent.join(&new_file_name);

                                            // Track the effective meta base name after any rename
                                            let effective_meta_base: &str;

                                            if new_path != *path {
                                                log::info!(
                                                    "Two-way sync: Renaming {} -> {}",
                                                    path.display(),
                                                    new_path.display()
                                                );
                                                self.suppress_path_any(path);
                                                self.suppress_path(&new_path);
                                                if let Err(err) = fs::rename(path, &new_path) {
                                                    self.unsuppress_path_any(path);
                                                    self.unsuppress_path(&new_path);
                                                    log::error!(
                                                        "Failed to rename {:?} to {:?}: {}",
                                                        path,
                                                        new_path,
                                                        err
                                                    );
                                                    effective_meta_base = old_base;
                                                } else {
                                                    overridden_source_path = Some(new_path.clone());
                                                    let old_meta = parent
                                                        .join(format!("{}.meta.json5", old_base));
                                                    let new_meta = parent.join(format!(
                                                        "{}.meta.json5",
                                                        slugified_new_name
                                                    ));
                                                    if old_meta.exists() {
                                                        self.suppress_path_any(&old_meta);
                                                        self.suppress_path(&new_meta);
                                                        if fs::rename(&old_meta, &new_meta).is_err()
                                                        {
                                                            self.unsuppress_path_any(&old_meta);
                                                            self.unsuppress_path(&new_meta);
                                                        }
                                                    }
                                                    effective_meta_base = &slugified_new_name;
                                                }
                                            } else {
                                                effective_meta_base = old_base;
                                            }

                                            // Always update the meta name field when the
                                            // instance name changed, even if the path didn't
                                            // change (e.g. "Foo/Bar" → "Foo|Bar" both slugify
                                            // to "Foo_Bar").
                                            let current_meta = parent.join(format!(
                                                "{}.meta.json5",
                                                effective_meta_base
                                            ));
                                            if slugified_new_name != *new_name {
                                                self.upsert_meta_name_field(
                                                    &current_meta,
                                                    new_name,
                                                );
                                            } else {
                                                self.remove_meta_name_field(&current_meta);
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
                            log::warn!("Cannot rename instance {:?} — no instigating source", id);
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

                                    // If a rename handler already moved the file,
                                    // use the new path instead of the stale
                                    // instigating_source path.
                                    let effective_path =
                                        overridden_source_path.as_deref().unwrap_or(path.as_path());

                                    if old_is_script && new_is_script && effective_path.exists() {
                                        // Script-to-script transition: rename the file extension.
                                        // For directory-format scripts, the path is the directory
                                        // (e.g., src/MyModule/), not the init file inside. We must
                                        // find and rename the init file, not the directory.
                                        // Resolve the actual file to rename. For directories,
                                        // find the init file inside; for files, use directly.
                                        let init_result = if effective_path.is_dir() {
                                            Self::find_init_file(effective_path)
                                                .map(|f| (f.clone(), effective_path.to_path_buf()))
                                        } else {
                                            Some((
                                                effective_path.to_path_buf(),
                                                effective_path
                                                    .parent()
                                                    .unwrap_or(effective_path)
                                                    .to_path_buf(),
                                            ))
                                        };

                                        if let Some((actual_file, file_parent)) = init_result {
                                            let new_suffix = match new_class.as_str() {
                                                "ModuleScript" => "",
                                                "Script" => ".server",
                                                "LocalScript" => ".local",
                                                _ => "",
                                            };
                                            let is_init = actual_file
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
                                            } else {
                                                // Derive the base name from the filesystem
                                                // path, not instance.name() (which may
                                                // differ from the slugified filename).
                                                let stem = actual_file
                                                    .file_stem()
                                                    .and_then(|s| s.to_str())
                                                    .unwrap_or("");
                                                let known_suffixes = [
                                                    ".server", ".client", ".plugin", ".local",
                                                    ".legacy",
                                                ];
                                                let base = known_suffixes
                                                    .iter()
                                                    .find_map(|s| stem.strip_suffix(s))
                                                    .unwrap_or(stem);
                                                if new_suffix.is_empty() {
                                                    format!("{}.luau", base)
                                                } else {
                                                    format!("{}{}.luau", base, new_suffix)
                                                }
                                            };

                                            let new_path = file_parent.join(&new_file_name);
                                            if new_path != actual_file {
                                                log::info!(
                                                    "Two-way sync: Changing class {} -> {}, \
                                                     renaming {} -> {}",
                                                    old_class,
                                                    new_class,
                                                    actual_file.display(),
                                                    new_path.display()
                                                );
                                                self.suppress_path_any(&actual_file);
                                                self.suppress_path(&new_path);
                                                if let Err(err) =
                                                    fs::rename(&actual_file, &new_path)
                                                {
                                                    self.unsuppress_path_any(&actual_file);
                                                    self.unsuppress_path(&new_path);
                                                    log::error!(
                                                        "Failed to rename {:?} to {:?}: {}",
                                                        actual_file,
                                                        new_path,
                                                        err
                                                    );
                                                } else {
                                                    overridden_source_path = Some(new_path.clone());
                                                }
                                            }
                                        } else {
                                            log::warn!(
                                                "Cannot change ClassName for directory {} \
                                                 — no init file found inside",
                                                path.display()
                                            );
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
                            // If a rename or ClassName change moved the file
                            // earlier in this update, write to the new location
                            // instead of the stale instigating_source path.
                            let source_path = if let Some(ref overridden) = overridden_source_path {
                                Some(overridden.clone())
                            } else if let Some(instigating_source) =
                                &instance.metadata().instigating_source
                            {
                                match instigating_source {
                                    InstigatingSource::Path(path) => Some(path.clone()),
                                    InstigatingSource::ProjectNode { .. } => {
                                        log::warn!(
                                            "Cannot update instance {:?}, it's from a project file",
                                            id
                                        );
                                        None
                                    }
                                }
                            } else {
                                log::warn!(
                                    "Cannot update instance {:?}, it is not an instigating source.",
                                    id
                                );
                                None
                            };

                            if let Some(ref write_path) = source_path {
                                if let Some(Variant::String(value)) = changed_value {
                                    log::info!(
                                        "Two-way sync: Writing Source to {}",
                                        write_path.display()
                                    );
                                    self.suppress_path(write_path);
                                    if let Err(err) = fs::write(write_path, value) {
                                        self.unsuppress_path(write_path);
                                        log::error!(
                                            "Failed to write Source to {:?} for instance {:?}: {}",
                                            write_path,
                                            id,
                                            err
                                        );
                                    }
                                } else {
                                    log::warn!("Cannot change Source to non-string value.");
                                }
                            }
                        } else {
                            log::trace!("Skipping non-Source property change: {}", key);
                        }
                    }

                    // Record metadata update so instigating_source and
                    // relevant_paths stay in sync after rename / class change.
                    if let Some(ref new_path) = overridden_source_path {
                        metadata_updates.push((id, new_path.clone()));
                    }
                } else {
                    log::warn!("Cannot update instance {:?}, it does not exist.", id);
                }
            }

            let applied = apply_patch_set(&mut tree, patch_set);

            // Update metadata for instances whose filesystem path changed.
            // This keeps instigating_source and path_to_ids in sync so that
            // subsequent VFS events (and future renames) target the correct
            // instance and path.
            for (id, new_instigating_source) in metadata_updates {
                if let Some(old_metadata) = tree.get_metadata(id).cloned() {
                    let mut new_metadata = old_metadata;
                    new_metadata.instigating_source =
                        Some(InstigatingSource::Path(new_instigating_source.clone()));
                    new_metadata.relevant_paths = rebuild_relevant_paths(&new_instigating_source);
                    tree.update_metadata(id, new_metadata);
                }
            }

            applied
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
            // For directory-format scripts, the instigating_source is the
            // init file (e.g., init.luau) so that two-way sync writes to the
            // correct file. However, snapshot_from_vfs returns None for init
            // files because they are handled as part of the parent directory
            // snapshot. Detect this case and snapshot the parent directory.
            let is_init_file = path
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.starts_with("init."))
                .unwrap_or(false);
            let snapshot_path = if is_init_file {
                path.parent().unwrap_or(path.as_path())
            } else {
                path.as_path()
            };

            log::info!(
                "compute_and_apply_changes: checking path {} for instance {:?}{}",
                path.display(),
                id,
                if is_init_file {
                    format!(
                        " (init file, will snapshot parent dir {})",
                        snapshot_path.display()
                    )
                } else {
                    String::new()
                }
            );

            match vfs.metadata(path).with_not_found() {
                Ok(Some(_)) => {
                    // Our instance was previously created from a path and that
                    // path still exists. We can generate a snapshot starting at
                    // that path and use it as the source for our patch.
                    log::info!(
                        "compute_and_apply_changes: path EXISTS via VFS, re-snapshotting {}",
                        snapshot_path.display()
                    );

                    let snapshot = match snapshot_from_vfs(&metadata.context, vfs, snapshot_path) {
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

                        let snapshot =
                            match snapshot_from_vfs(&metadata.context, vfs, snapshot_path) {
                                Ok(snapshot) => snapshot,
                                Err(err) => {
                                    log::error!(
                                        "Recovery snapshot error for {}: {:?}",
                                        snapshot_path.display(),
                                        err
                                    );
                                    return None;
                                }
                            };

                        let patch_set = compute_patch_set(snapshot, tree, id);
                        let applied = apply_patch_set(tree, patch_set);
                        Some(ComputeResult {
                            applied,
                            removed_path: None,
                        })
                    } else if is_init_file && std::fs::metadata(snapshot_path).is_ok() {
                        // Init file was deleted but the parent directory still
                        // exists. Re-snapshot the directory — it will become a
                        // Folder (or whatever the directory middleware produces
                        // without an init file).
                        log::info!(
                            "compute_and_apply_changes: init file {} deleted but parent \
                             directory {} still exists. Re-snapshotting directory.",
                            path.display(),
                            snapshot_path.display()
                        );

                        let snapshot =
                            match snapshot_from_vfs(&metadata.context, vfs, snapshot_path) {
                                Ok(snapshot) => snapshot,
                                Err(err) => {
                                    log::error!(
                                        "Directory re-snapshot error for {}: {:?}",
                                        snapshot_path.display(),
                                        err
                                    );
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

/// Rebuild the `relevant_paths` list for an instance given its new
/// `instigating_source` path. This mirrors the logic in the snapshot
/// middleware so that `path_to_ids` stays correct after a rename.
fn rebuild_relevant_paths(new_path: &Path) -> Vec<PathBuf> {
    let file_name = new_path.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let is_init = file_name.starts_with("init.");

    if is_init {
        // Directory-format script: relevant paths are the directory itself
        // plus every possible init file variant inside it.
        let dir_path = new_path.parent().unwrap().to_path_buf();
        vec![
            dir_path.clone(),
            dir_path.join("init.luau"),
            dir_path.join("init.server.luau"),
            dir_path.join("init.client.luau"),
            dir_path.join("init.plugin.luau"),
            dir_path.join("init.local.luau"),
            dir_path.join("init.legacy.luau"),
            dir_path.join("init.csv"),
            dir_path.join("init.lua"),
            dir_path.join("init.server.lua"),
            dir_path.join("init.client.lua"),
            dir_path.join("init.meta.json5"),
        ]
    } else {
        // Standalone file: relevant paths are the file itself and its
        // adjacent meta file.
        let stem = new_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let base_name = stem
            .strip_suffix(".server")
            .or_else(|| stem.strip_suffix(".client"))
            .or_else(|| stem.strip_suffix(".plugin"))
            .or_else(|| stem.strip_suffix(".local"))
            .or_else(|| stem.strip_suffix(".legacy"))
            .unwrap_or(stem);
        vec![
            new_path.to_path_buf(),
            new_path.with_file_name(format!("{base_name}.meta.json5")),
        ]
    }
}
