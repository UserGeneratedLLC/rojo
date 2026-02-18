use crossbeam_channel::{select, Receiver, RecvError, Sender};
use jod_thread::JoinHandle;
use memofs::{IoResultExt, Vfs, VfsEvent};
use rbx_dom_weak::types::{Ref, Variant};
use std::collections::HashSet;
use std::fmt;
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
    syncback::{
        dedup_suffix::{compute_cleanup_action, parse_dedup_suffix, DedupCleanupAction},
        deduplicate_name, name_needs_slugify, slugify_name, strip_script_suffix,
    },
};

/// Wrapper that displays a path relative to a project root directory.
struct RelPath<'a> {
    path: &'a Path,
    root: &'a Path,
}

impl fmt::Display for RelPath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path
            .strip_prefix(self.root)
            .unwrap_or(self.path)
            .display()
            .fmt(f)
    }
}

/// Returns a display wrapper that shows `path` relative to `root`.
fn rel_path<'a>(path: &'a Path, root: &'a Path) -> RelPath<'a> {
    RelPath { path, root }
}

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
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        tree: Arc<Mutex<RojoTree>>,
        vfs: Arc<Vfs>,
        message_queue: Arc<MessageQueue<AppliedPatchSet>>,
        tree_mutation_receiver: Receiver<PatchSet>,
        suppressed_paths: Arc<Mutex<std::collections::HashMap<PathBuf, (usize, usize)>>>,
        ref_path_index: Arc<Mutex<crate::RefPathIndex>>,
        project_root: PathBuf,
        critical_error_receiver: Option<Receiver<memofs::WatcherCriticalError>>,
        git_repo_root: Option<PathBuf>,
    ) -> Self {
        let (shutdown_sender, shutdown_receiver) = crossbeam_channel::bounded(1);
        let vfs_receiver = vfs.event_receiver();
        // Use crossbeam::never() for callers that don't provide an error receiver
        // (non-serve commands). never() blocks forever without selecting.
        let critical_error_receiver =
            critical_error_receiver.unwrap_or_else(crossbeam_channel::never);
        // Canonicalize project_root so path comparisons work with the
        // \\?\ prefix that std::fs::canonicalize adds on Windows.
        let project_root = std::fs::canonicalize(&project_root).unwrap_or(project_root);
        let task = JobThreadContext {
            tree,
            vfs,
            message_queue,
            pending_recovery: Mutex::new(Vec::new()),
            suppressed_paths,
            project_root,
            ref_path_index,
            git_repo_root,
        };

        let job_thread = jod_thread::Builder::new()
            .name("ChangeProcessor thread".to_owned())
            .spawn(move || {
                log::trace!("ChangeProcessor thread started");

                // Tracks when to run the next reconciliation pass. Set to
                // Some(future_instant) after VFS events arrive, cleared
                // after reconcile_tree() runs. This ensures we only do the
                // full re-snapshot once per burst of activity.
                let mut reconcile_at: Option<Instant> = None;

                loop {
                    // Compute the timeout for the default branch.
                    // If a reconciliation is pending, wake up when it's due
                    // (clamped to at least 50ms to avoid busy-spinning).
                    // Otherwise use the normal 500ms sweep interval.
                    let timeout = match reconcile_at {
                        Some(deadline) => {
                            let remaining = deadline.saturating_duration_since(Instant::now());
                            remaining.max(Duration::from_millis(50))
                        }
                        None => Duration::from_millis(500),
                    };

                    select! {
                        recv(vfs_receiver) -> event => {
                            let mut all_patches = task.handle_vfs_event(event?);

                            // Drain any pending events that arrived during processing.
                            // This ensures that multi-event filesystem operations (e.g.,
                            // rename = REMOVE + CREATE on Windows) produce a single
                            // batched message instead of separate per-event messages,
                            // giving consistent behavior across platforms.
                            while let Ok(event) = vfs_receiver.try_recv() {
                                all_patches.extend(task.handle_vfs_event(event));
                            }

                            all_patches.extend(task.process_pending_recoveries());

                            if !all_patches.is_empty() {
                                let merged = AppliedPatchSet::merge(all_patches);
                                if !merged.is_empty() {
                                    task.message_queue.push_messages(&[merged]);
                                }
                            }

                            // Schedule a reconciliation 200ms from now if one isn't pending.
                            if reconcile_at.is_none() {
                                reconcile_at = Some(Instant::now() + Duration::from_millis(200));
                            }

                            // If the deadline has passed, reconcile now. This check runs
                            // inside the VFS branch because during sustained event bursts,
                            // the `default` branch never fires (the VFS channel is never
                            // idle long enough).
                            if reconcile_at.is_some_and(|d| Instant::now() >= d) {
                                task.reconcile_tree();
                                reconcile_at = None;
                            }
                        },
                        recv(tree_mutation_receiver) -> patch_set => {
                            task.handle_tree_event(patch_set?);
                        },
                        recv(critical_error_receiver) -> err => {
                            if let Ok(memofs::WatcherCriticalError::RescanRequired) = err {
                                log::warn!(
                                    "VFS watcher lost events (RescanRequired). \
                                     Triggering full tree reconciliation."
                                );
                                task.reconcile_tree();
                                reconcile_at = None;
                            }
                        },
                        recv(shutdown_receiver) -> _ => {
                            log::trace!("ChangeProcessor shutdown signal received...");
                            return Ok(());
                        },
                        default(timeout) => {
                            task.process_pending_recoveries();

                            // If a reconciliation deadline has passed, run it now.
                            if reconcile_at.is_some_and(|d| Instant::now() >= d) {
                                task.reconcile_tree();
                                reconcile_at = None;
                            }
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

    /// Root directory of the project, used to display relative paths in logs.
    project_root: PathBuf,

    /// Index of meta/model files that contain `Rojo_Ref_*` attributes.
    /// Shared with ApiService for efficient rename path updates.
    ref_path_index: Arc<Mutex<crate::RefPathIndex>>,

    /// Git repository root, if the project is in a git repo.
    /// Used for auto-staging Source writes.
    git_repo_root: Option<PathBuf>,
}

impl JobThreadContext {
    /// Returns a display wrapper that shows `path` relative to the project root.
    fn display_path<'a>(&'a self, path: &'a Path) -> RelPath<'a> {
        rel_path(path, &self.project_root)
    }

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

    /// Suppress the next Remove VFS event for the given path.
    fn suppress_path_remove(&self, path: &Path) {
        let mut suppressed = self.suppressed_paths.lock().unwrap();
        let key = Self::suppression_key(path);
        suppressed.entry(key).or_insert((0, 0)).0 += 1;
    }

    /// Upsert the `name` field in a `.meta.json5` file, suppressing filesystem
    /// events to avoid feedback loops.
    fn upsert_meta_name_field(&self, meta_path: &Path, real_name: &str) {
        self.suppress_path(meta_path);
        if let Err(err) = crate::syncback::meta::upsert_meta_name(meta_path, real_name) {
            self.unsuppress_path(meta_path);
            log::error!(
                "Failed to upsert name in meta file {}: {}",
                self.display_path(meta_path),
                err
            );
        }
    }

    /// Upsert the `name` field inside a `.model.json5` / `.model.json` file,
    /// suppressing filesystem events.
    fn upsert_model_name_field(&self, model_path: &Path, real_name: &str) {
        self.suppress_path(model_path);
        if let Err(err) = crate::syncback::meta::upsert_model_name(model_path, real_name) {
            self.unsuppress_path(model_path);
            log::error!(
                "Failed to upsert name in model file {}: {}",
                self.display_path(model_path),
                err
            );
        }
    }

    /// Remove the `name` field from a `.model.json5` / `.model.json` file,
    /// suppressing filesystem events.
    fn remove_model_name_field(&self, model_path: &Path) {
        use crate::syncback::meta::RemoveNameOutcome;
        self.suppress_path(model_path);
        match crate::syncback::meta::remove_model_name(model_path) {
            Ok(RemoveNameOutcome::NoOp) => {
                self.unsuppress_path(model_path);
            }
            Ok(RemoveNameOutcome::FieldRemoved) => {
                // File was rewritten — suppress_path already covers it.
            }
            Ok(RemoveNameOutcome::FileDeleted) => {
                // Model files shouldn't be deleted (they have className etc),
                // but handle for completeness.
                self.unsuppress_path(model_path);
                self.suppress_path_remove(model_path);
            }
            Err(err) => {
                self.unsuppress_path(model_path);
                log::error!(
                    "Failed to remove name from model file {}: {}",
                    self.display_path(model_path),
                    err
                );
            }
        }
    }

    /// After an instance is renamed, update all `Rojo_Ref_*` attributes on
    /// disk that reference the old path prefix, replacing it with the new
    /// prefix.
    ///
    /// Uses the `RefPathIndex` for O(affected_files) lookup instead of
    /// scanning the full tree. After updating files, also updates the index
    /// keys and filesystem paths so future renames remain efficient.
    fn update_ref_paths_after_rename(
        &self,
        old_path: &str,
        new_path: &str,
        tree: &crate::snapshot::RojoTree,
    ) {
        if old_path == new_path {
            return;
        }

        let files_from_index = self.ref_path_index.lock().unwrap().find_by_prefix(old_path);

        if files_from_index.is_empty() {
            return;
        }

        let old_segment = old_path.rsplit('/').next().unwrap_or(old_path);
        let new_segment = new_path.rsplit('/').next().unwrap_or(new_path);

        let slugified_old = if name_needs_slugify(old_segment) {
            Some(slugify_name(old_segment))
        } else {
            None
        };
        let slugified_new = || -> String {
            if name_needs_slugify(new_segment) {
                slugify_name(new_segment)
            } else {
                new_segment.to_string()
            }
        };

        let original_paths = files_from_index.clone();
        let files_to_check: Vec<PathBuf> = files_from_index
            .into_iter()
            .map(|file_path| {
                if file_path.exists() {
                    return file_path;
                }
                let mut result = PathBuf::new();
                let mut replaced = false;
                for comp in file_path.components() {
                    if !replaced {
                        if let std::path::Component::Normal(os_str) = comp {
                            if let Some(s) = os_str.to_str() {
                                if s == old_segment {
                                    result.push(slugified_new());
                                    replaced = true;
                                    continue;
                                }
                                if let Some(ref slug) = slugified_old {
                                    if s == slug.as_str() {
                                        result.push(slugified_new());
                                        replaced = true;
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    result.push(comp);
                }
                if replaced {
                    result
                } else {
                    file_path
                }
            })
            .collect();

        let mut updated_count = 0;
        for file_path in &files_to_check {
            let source_abs = tree
                .get_ids_at_path(file_path)
                .first()
                .map(|&id| crate::ref_target_path_from_tree(tree, id))
                .unwrap_or_default();

            self.suppress_path(file_path);
            match crate::syncback::meta::update_ref_paths_in_file(
                file_path,
                old_path,
                new_path,
                &source_abs,
            ) {
                Ok(true) => {
                    updated_count += 1;
                }
                Ok(false) => {
                    self.unsuppress_path(file_path);
                }
                Err(err) => {
                    self.unsuppress_path(file_path);
                    log::warn!(
                        "Failed to update Rojo_Ref_* paths in {}: {}",
                        self.display_path(file_path),
                        err
                    );
                }
            }
        }

        // Update the index: both the path keys AND the filesystem paths.
        if updated_count > 0 {
            let mut index = self.ref_path_index.lock().unwrap();
            index.update_prefix(old_path, new_path);
            // Also update filesystem paths in the index entries
            for (old_file, new_file) in original_paths.iter().zip(files_to_check.iter()) {
                if old_file != new_file {
                    index.rename_file(old_file, new_file);
                }
            }

            log::info!(
                "Updated Rojo_Ref_* paths in {} file(s): '{}' -> '{}'",
                updated_count,
                old_path,
                new_path
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
                // File was deleted, not rewritten. Swap: undo the
                // pre-emptive Write suppression and add a Remove
                // suppression instead so the counts are (1, 0).
                self.unsuppress_path(meta_path);
                self.suppress_path_remove(meta_path);
            }
            Ok(RemoveNameOutcome::FieldRemoved) => {
                // File was rewritten — suppress_path already covers it.
            }
            Err(err) => {
                self.unsuppress_path(meta_path);
                log::error!(
                    "Failed to remove name from meta file {}: {}",
                    self.display_path(meta_path),
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
                self.display_path(current_path),
                ids
            );

            if !ids.is_empty() {
                break ids.to_vec();
            }

            log::info!(
                "apply_patches: no IDs at {}, trying parent...",
                self.display_path(current_path)
            );
            match current_path.parent() {
                // Stop walking if we've reached or passed the project root.
                Some(parent) if parent.starts_with(&self.project_root) => {
                    current_path = parent;
                }
                _ => break Vec::new(),
            }
        };

        if affected_ids.is_empty() {
            log::info!(
                "apply_patches: no affected instances found for path {}",
                self.display_path(&path)
            );
        }

        for id in affected_ids {
            if let Some(result) =
                compute_and_apply_changes(&mut tree, &self.vfs, id, &self.project_root)
            {
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

    fn handle_vfs_event(&self, event: VfsEvent) -> Vec<AppliedPatchSet> {
        // Log EVERY VFS event at INFO level for diagnostics.
        // This is intentionally verbose — it is critical for debugging
        // file watcher desync issues (e.g., rapid delete+recreate).
        match &event {
            VfsEvent::Create(path) => log::info!("VFS event: CREATE {}", self.display_path(path)),
            VfsEvent::Write(path) => log::info!("VFS event: WRITE {}", self.display_path(path)),
            VfsEvent::Remove(path) => log::info!("VFS event: REMOVE {}", self.display_path(path)),
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
                            self.display_path(path)
                        );
                        return Vec::new();
                    }
                }
            }
        }

        // Update the VFS immediately with the event.
        self.vfs
            .commit_event(&event)
            .expect("Error applying VFS change");

        // On Windows, ReadDirectoryChangesW fires a directory-level WRITE event
        // when a file inside the directory is modified, in addition to the
        // file-level WRITE event. The directory event arrives first, but at that
        // point the VFS cache still has the old file content (only the directory
        // entry was committed, not the file). Re-snapshotting from the directory
        // event reads stale data and produces incorrect patches. The file-level
        // event that follows will correctly invalidate the cache and re-snapshot.
        // Skip WRITE events for directories on Windows only -- macOS kqueue uses
        // directory WRITE events meaningfully (e.g., file creation/deletion
        // notifications) so they must be processed there.
        #[cfg(target_os = "windows")]
        if let VfsEvent::Write(ref path) = event {
            if path.is_dir() {
                log::info!(
                    "VFS event SKIPPED (directory WRITE, deferring to file events): {}",
                    self.display_path(path)
                );
                return Vec::new();
            }
        }

        // For a given VFS event, we might have many changes to different parts
        // of the tree. Calculate and apply all of these changes.
        let applied_patches = match event {
            VfsEvent::Create(path) | VfsEvent::Write(path) => {
                match self.vfs.canonicalize(&path) {
                    Ok(canonical_path) => {
                        log::info!(
                            "VFS: canonicalize OK for {} -> {}",
                            self.display_path(&path),
                            self.display_path(&canonical_path)
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
                                self.display_path(&path)
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
                                        self.display_path(&path),
                                        self.display_path(&resolved)
                                    );
                                    self.apply_patches(resolved)
                                }
                                Err(err) => {
                                    log::info!(
                                        "VFS: Skipping Create/Write for {} — \
                                         parent no longer exists: {}",
                                        self.display_path(&path),
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
                        log::info!("VFS: Remove resolved to {}", self.display_path(&resolved));
                        self.apply_patches(resolved)
                    }
                    Err(err) => {
                        log::info!(
                            "VFS: Skipping remove event for {} — parent no longer exists: {}",
                            self.display_path(&path),
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

        applied_patches
    }

    /// Processes any pending recovery checks for paths that were recently
    /// removed. If a path has reappeared on the real filesystem after the
    /// recovery delay, we trigger a re-snapshot to bring the tree back in sync.
    fn process_pending_recoveries(&self) -> Vec<AppliedPatchSet> {
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

        let mut all_patches = Vec::new();
        for path in ready {
            if std::fs::metadata(&path).is_ok() {
                log::info!(
                    "VFS recovery: path {} was removed but has reappeared on disk. Re-snapshotting.",
                    self.display_path(&path)
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
                    all_patches.extend(patches);
                }
            } else {
                log::info!(
                    "VFS recovery: path {} confirmed removed from disk.",
                    self.display_path(&path)
                );
            }
        }
        all_patches
    }

    /// Re-snapshots the entire project from the real filesystem and patches
    /// the in-memory tree to correct any drift from missed VFS events.
    /// Called when the file watcher reports `RescanRequired`.
    fn reconcile_tree(&self) {
        use crate::snapshot::InstanceContext;

        let start = Instant::now();
        let instance_context = InstanceContext::new();

        let snapshot = match snapshot_from_vfs(&instance_context, &self.vfs, &self.project_root) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Tree reconciliation snapshot error: {:?}", e);
                return;
            }
        };

        let mut tree = self.tree.lock().unwrap();
        let root_id = tree.get_root_id();
        let patch_set = compute_patch_set(snapshot, &tree, root_id);

        // Only reconcile structural drift (added/removed instances).
        // Property updates from compute_patch_set can produce false positives
        // (e.g., Ref properties differ between snapshot IDs and tree IDs).
        // Property changes are already handled correctly by individual VFS events.
        if patch_set.removed_instances.is_empty() && patch_set.added_instances.is_empty() {
            log::info!(
                "Tree reconciliation: no drift detected ({:.1?})",
                start.elapsed()
            );
            return;
        }

        let added = patch_set.added_instances.len();
        let removed = patch_set.removed_instances.len();

        // Strip property updates to avoid false-positive patches
        let structural_patch = PatchSet {
            added_instances: patch_set.added_instances,
            removed_instances: patch_set.removed_instances,
            updated_instances: Vec::new(),
            stage_ids: HashSet::new(),
            stage_paths: Vec::new(),
        };

        let applied = apply_patch_set(&mut tree, structural_patch);
        drop(tree);

        self.message_queue.push_messages(&[applied]);
        log::info!(
            "Tree reconciliation: corrected {} added, {} removed ({:.1?})",
            added,
            removed,
            start.elapsed()
        );
    }

    fn handle_tree_event(&self, mut patch_set: PatchSet) {
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
            // (handle_api_write → remove_instance_at_path) already deleted
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
                                    self.display_path(path)
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

            // Dedup suffix cleanup: after removals, check if any dedup
            // groups need suffix renames (base-name promotion, group-to-1).
            // Must run BEFORE apply_patch_set removes the instances from
            // the tree, because we need parent/sibling relationships.
            //
            // Build a set of ALL removed Refs so the sibling enumeration
            // skips co-removed instances (Fix 2: batch-removal awareness).
            let removed_set: std::collections::HashSet<Ref> =
                patch_set.removed_instances.iter().copied().collect();
            // Collect metadata updates for survivors whose filesystem path
            // changed due to dedup cleanup renames (Fix 1: metadata drift).
            let mut dedup_metadata_updates: Vec<(Ref, PathBuf)> = Vec::new();

            for &removed_id in &patch_set.removed_instances {
                let (parent_ref, removed_fs_name, removed_file_dir) = {
                    let Some(inst) = tree.get_instance(removed_id) else {
                        continue;
                    };
                    let Some(source) = &inst.metadata().instigating_source else {
                        continue;
                    };
                    if matches!(source, InstigatingSource::ProjectNode { .. }) {
                        continue;
                    }
                    let parent = inst.parent();
                    let fs_name = tree.filesystem_name_for(removed_id);
                    // Derive the directory containing this file from its own
                    // instigating source path. This works even when the parent
                    // is a ProjectNode (where instigating_source is not a Path).
                    let file_dir = match source {
                        InstigatingSource::Path(p) => {
                            let fname = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
                            if fname.starts_with("init.") {
                                // Directory-format: path is dir/init.luau,
                                // the containing dir is the grandparent
                                p.parent().and_then(|d| d.parent()).map(|d| d.to_path_buf())
                            } else {
                                p.parent().map(|d| d.to_path_buf())
                            }
                        }
                        _ => None,
                    };
                    (parent, fs_name, file_dir)
                };

                if parent_ref.is_none() {
                    continue;
                }

                // Parse the dedup suffix from the removed instance's FS name
                let removed_stem = removed_fs_name
                    .split('.')
                    .next()
                    .unwrap_or(&removed_fs_name);
                let (base_stem, _) = match parse_dedup_suffix(removed_stem) {
                    Some((base, n)) => (base.to_string(), Some(n)),
                    None => (removed_stem.to_string(), None),
                };

                // Find siblings that share the same dedup base stem AND extension.
                // Matching by stem alone would incorrectly group across different
                // middleware types (e.g., Foo.server.luau and Foo.luau have the
                // same stem "Foo" but different dedup keys).
                let removed_extension =
                    removed_fs_name.find('.').map(|i| &removed_fs_name[i + 1..]);

                let Some(parent_inst) = tree.get_instance(parent_ref) else {
                    continue;
                };
                let mut remaining_stems: Vec<String> = Vec::new();
                let deleted_was_base = parse_dedup_suffix(removed_stem).is_none();

                for &sibling_ref in parent_inst.children() {
                    // Skip ALL instances being removed in this patch, not just
                    // the current one. This prevents co-removed siblings from
                    // inflating the remaining count and defeating cleanup rules.
                    if removed_set.contains(&sibling_ref) {
                        continue;
                    }
                    let sibling_fs = tree.filesystem_name_for(sibling_ref);
                    let sibling_stem = sibling_fs.split('.').next().unwrap_or(&sibling_fs);
                    let sibling_ext = sibling_fs.find('.').map(|i| &sibling_fs[i + 1..]);

                    // Only group siblings with the same extension (same dedup key space)
                    if sibling_ext != removed_extension {
                        continue;
                    }

                    let sibling_base = match parse_dedup_suffix(sibling_stem) {
                        Some((b, _)) => b,
                        None => sibling_stem,
                    };
                    if sibling_base.eq_ignore_ascii_case(&base_stem) {
                        remaining_stems.push(sibling_stem.to_string());
                    }
                }

                if remaining_stems.is_empty() {
                    continue;
                }

                let extension = removed_extension;

                // Get the parent directory path (derived from the removed
                // instance's own path, not the parent's instigating source,
                // because the parent may be a ProjectNode).
                let Some(parent_dir) = removed_file_dir.clone() else {
                    continue;
                };

                let action = compute_cleanup_action(
                    &base_stem,
                    extension,
                    &remaining_stems,
                    deleted_was_base,
                    &parent_dir,
                );

                match action {
                    DedupCleanupAction::None => {}
                    DedupCleanupAction::RemoveSuffix { from, to }
                    | DedupCleanupAction::PromoteLowest { from, to } => {
                        log::info!(
                            "Dedup cleanup: renaming {} -> {}",
                            self.display_path(&from),
                            self.display_path(&to),
                        );
                        self.suppress_path_any(&from);
                        self.suppress_path(&to);
                        if let Err(e) = fs::rename(&from, &to) {
                            log::warn!(
                                "Dedup cleanup rename failed: {} -> {}: {}",
                                from.display(),
                                to.display(),
                                e
                            );
                            self.unsuppress_path(&to);
                        } else {
                            // Also rename the adjacent meta file if it exists
                            // (standalone files only; directory-format instances
                            // have init.meta.json5 inside the directory which
                            // moves automatically with the dir rename).
                            if from.is_file() || !from.exists() {
                                if let (Some(from_parent), Some(from_name), Some(to_name)) = (
                                    from.parent(),
                                    from.file_stem().and_then(|s| s.to_str()),
                                    to.file_stem().and_then(|s| s.to_str()),
                                ) {
                                    let from_base = strip_script_suffix(from_name);
                                    let to_base = strip_script_suffix(to_name);
                                    let old_meta =
                                        from_parent.join(format!("{}.meta.json5", from_base));
                                    if old_meta.exists() {
                                        let new_meta =
                                            from_parent.join(format!("{}.meta.json5", to_base));
                                        self.suppress_path_any(&old_meta);
                                        self.suppress_path(&new_meta);
                                        if fs::rename(&old_meta, &new_meta).is_err() {
                                            self.unsuppress_path_any(&old_meta);
                                            self.unsuppress_path(&new_meta);
                                        }
                                    }
                                }
                            }

                            // Update ref paths if the renamed instance has refs
                            let old_ref_segment =
                                from.file_name().and_then(|f| f.to_str()).unwrap_or("");
                            let new_ref_segment =
                                to.file_name().and_then(|f| f.to_str()).unwrap_or("");
                            if old_ref_segment != new_ref_segment {
                                let parent_path =
                                    crate::ref_target_path_from_tree(&tree, parent_ref);
                                let old_prefix = format!("{}/{}", parent_path, old_ref_segment);
                                let new_prefix = format!("{}/{}", parent_path, new_ref_segment);
                                self.update_ref_paths_after_rename(&old_prefix, &new_prefix, &tree);
                            }

                            // Fix 1: Update the renamed survivor's in-memory
                            // metadata so InstigatingSource::Path and
                            // relevant_paths point to the new filesystem path.
                            let old_from_name =
                                from.file_name().and_then(|f| f.to_str()).unwrap_or("");
                            for &sibling_ref in parent_inst.children() {
                                if removed_set.contains(&sibling_ref) {
                                    continue;
                                }
                                let sibling_fs = tree.filesystem_name_for(sibling_ref);
                                if sibling_fs == old_from_name {
                                    // This is the survivor being renamed.
                                    // Compute its new instigating source path.
                                    if let Some(meta) = tree.get_metadata(sibling_ref) {
                                        if let Some(InstigatingSource::Path(old_path)) =
                                            &meta.instigating_source
                                        {
                                            let new_source = if old_path
                                                .file_name()
                                                .and_then(|f| f.to_str())
                                                .map(|f| f.starts_with("init."))
                                                .unwrap_or(false)
                                            {
                                                // Directory-format: old source
                                                // is dir/init.luau; the dir was
                                                // renamed so update the dir
                                                // portion.
                                                let init_name =
                                                    old_path.file_name().unwrap().to_str().unwrap();
                                                to.join(init_name)
                                            } else {
                                                // Standalone file: new source
                                                // IS the `to` path.
                                                to.clone()
                                            };
                                            dedup_metadata_updates.push((sibling_ref, new_source));
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            // Collect (Ref, new_instigating_source) for instances whose
            // filesystem path changed (rename / ClassName transition).
            // Applied after the PatchSet to keep metadata in sync.
            let mut metadata_updates: Vec<(Ref, PathBuf)> = Vec::new();

            // Paths to stage via git add after all writes complete.
            // Starts with pre-resolved paths from api.rs, then Source writes are appended.
            let mut pending_stage_paths = std::mem::take(&mut patch_set.stage_paths);

            for update in &patch_set.updated_instances {
                let id = update.id;

                // Capture the old path BEFORE rename for Rojo_Ref_* path updates.
                // Must be computed before the `tree.get_instance(id)` borrow.
                let old_ref_path =
                    if update.changed_name.is_some() || update.changed_class_name.is_some() {
                        Some(crate::ref_target_path_from_tree(&tree, id))
                    } else {
                        None
                    };

                // The new filesystem name segment after a rename. Set during
                // rename handling and used by the ref path update code to build
                // the correct filesystem-name-based ref path.
                let mut new_ref_segment: Option<String> = None;

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
                                                // Directory renames: fs::rename fails safely
                                                // on all platforms if the target is a non-empty
                                                // directory, so no dedup is needed here (unlike
                                                // the file rename path below, where Unix
                                                // fs::rename silently overwrites).
                                                let new_dir_path =
                                                    grandparent.join(&slugified_new_name);

                                                // Track the effective directory path after any rename
                                                let effective_dir_path;

                                                if new_dir_path != dir_path {
                                                    log::info!(
                                                        "Two-way sync: Renaming directory {} -> {}",
                                                        self.display_path(dir_path),
                                                        self.display_path(&new_dir_path)
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
                                                        new_ref_segment =
                                                            Some(slugified_new_name.clone());
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
                                                ".legacy", ".model",
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

                                            // Guard against rename collision: if the target
                                            // path already exists and isn't our own file,
                                            // deduplicate the name against siblings.
                                            let deduped_new_name;
                                            {
                                                let candidate_file = if extension.is_empty() {
                                                    format!(
                                                        "{}{}",
                                                        slugified_new_name, script_suffix
                                                    )
                                                } else {
                                                    format!(
                                                        "{}{}.{}",
                                                        slugified_new_name,
                                                        script_suffix,
                                                        extension
                                                    )
                                                };
                                                let candidate = parent.join(&candidate_file);
                                                if candidate != *path && candidate.exists() {
                                                    let mut taken: std::collections::HashSet<
                                                        String,
                                                    > = std::collections::HashSet::new();
                                                    if let Ok(entries) = fs::read_dir(parent) {
                                                        for entry in entries.flatten() {
                                                            let ep = entry.path();
                                                            let slug = if ep.is_dir() {
                                                                ep.file_name()
                                                                    .and_then(|f| f.to_str())
                                                                    .unwrap_or("")
                                                                    .to_lowercase()
                                                            } else {
                                                                let s = ep
                                                                    .file_stem()
                                                                    .and_then(|f| f.to_str())
                                                                    .unwrap_or("");
                                                                strip_script_suffix(s)
                                                                    .to_lowercase()
                                                            };
                                                            taken.insert(slug);
                                                        }
                                                    }
                                                    // Free the slot we're vacating
                                                    taken.remove(&old_base.to_lowercase());
                                                    deduped_new_name = deduplicate_name(
                                                        &slugified_new_name,
                                                        &taken,
                                                    );
                                                } else {
                                                    deduped_new_name = slugified_new_name.clone();
                                                }
                                            }

                                            let new_file_name = if extension.is_empty() {
                                                format!("{}{}", deduped_new_name, script_suffix)
                                            } else {
                                                format!(
                                                    "{}{}.{}",
                                                    deduped_new_name, script_suffix, extension
                                                )
                                            };
                                            let new_path = parent.join(&new_file_name);

                                            // Track the effective meta base name after any rename
                                            let effective_meta_base: &str;

                                            if new_path != *path {
                                                log::info!(
                                                    "Two-way sync: Renaming {} -> {}",
                                                    self.display_path(path),
                                                    self.display_path(&new_path)
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
                                                    new_ref_segment = Some(new_file_name.clone());
                                                    let old_meta = parent
                                                        .join(format!("{}.meta.json5", old_base));
                                                    let new_meta = parent.join(format!(
                                                        "{}.meta.json5",
                                                        deduped_new_name
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
                                                    effective_meta_base = &deduped_new_name;
                                                }
                                            } else {
                                                effective_meta_base = old_base;
                                            }

                                            // Always update the name field when the
                                            // instance name changed, even if the path didn't
                                            // change (e.g. "Foo/Bar" → "Foo|Bar" both slugify
                                            // to "Foo_Bar"). Use deduped_new_name because
                                            // dedup may have appended ~N.
                                            //
                                            // For .model.json5/.model.json files, the name
                                            // field lives INSIDE the model file, not in
                                            // adjacent .meta.json5.
                                            if script_suffix == ".model" {
                                                let model_file = overridden_source_path
                                                    .as_deref()
                                                    .unwrap_or(path.as_path());
                                                if deduped_new_name != *new_name {
                                                    self.upsert_model_name_field(
                                                        model_file, new_name,
                                                    );
                                                } else {
                                                    self.remove_model_name_field(model_file);
                                                }
                                            } else {
                                                let current_meta = parent.join(format!(
                                                    "{}.meta.json5",
                                                    effective_meta_base
                                                ));
                                                if deduped_new_name != *new_name {
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
                                                    self.display_path(&actual_file),
                                                    self.display_path(&new_path)
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
                                                    // For standalone files, the ref path
                                                    // segment changes with the extension.
                                                    // For init files, the directory name
                                                    // (ref segment) is unchanged.
                                                    if !is_init {
                                                        new_ref_segment =
                                                            Some(new_file_name.clone());
                                                    }
                                                }
                                            }
                                        } else {
                                            log::warn!(
                                                "Cannot change ClassName for directory {} \
                                                 — no init file found inside",
                                                self.display_path(path)
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
                                        self.display_path(write_path)
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
                                    } else if patch_set.stage_ids.contains(&id) {
                                        pending_stage_paths.push(write_path.clone());
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

                // After rename, update any Rojo_Ref_* paths that referenced the
                // old path of this instance (or its descendants). Done outside the
                // `if let Some(instance)` block to avoid borrow conflicts.
                //
                // NOTE: The tree name hasn't been updated yet (apply_patch_set
                // runs after this loop), so we can't use full_path_of for the
                // new path. Construct it by replacing the last path segment
                // with the NEW filesystem name (set during rename handling).
                if let Some(ref old_ref_path) = old_ref_path {
                    if let Some(ref segment) = new_ref_segment {
                        // Use the filesystem name computed during rename handling
                        let segments: Vec<&str> = old_ref_path.split('/').collect();
                        let new_ref_path = if segments.len() > 1 {
                            let parent = segments[..segments.len() - 1].join("/");
                            format!("{}/{}", parent, segment)
                        } else {
                            segment.clone()
                        };
                        if *old_ref_path != new_ref_path {
                            self.update_ref_paths_after_rename(old_ref_path, &new_ref_path, &tree);
                        }
                    } else if update.changed_name.is_some() || update.changed_class_name.is_some() {
                        // Rename or class change was requested but no filesystem
                        // rename happened (e.g., ProjectNode, init-file class
                        // change where directory name stays the same). No ref
                        // path update needed.
                        log::trace!(
                            "Skipping ref path update for {:?}: no filesystem rename",
                            id
                        );
                    }
                }
            }

            let applied = apply_patch_set(&mut tree, patch_set);

            // Update metadata for instances whose filesystem path changed.
            // This keeps instigating_source and path_to_ids in sync so that
            // subsequent VFS events (and future renames) target the correct
            // instance and path.
            for (id, new_instigating_source) in
                metadata_updates.into_iter().chain(dedup_metadata_updates)
            {
                if let Some(old_metadata) = tree.get_metadata(id).cloned() {
                    let mut new_metadata = old_metadata;
                    new_metadata.instigating_source =
                        Some(InstigatingSource::Path(new_instigating_source.clone()));
                    new_metadata.relevant_paths = rebuild_relevant_paths(&new_instigating_source);
                    tree.update_metadata(id, new_metadata);
                }
            }

            // Consolidated git staging: one git_add call for all paths
            // (pre-resolved from api.rs + Source writes from this function).
            if !pending_stage_paths.is_empty() {
                if let Some(ref repo_root) = self.git_repo_root {
                    crate::git::git_add(repo_root, &pending_stage_paths);
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

fn compute_and_apply_changes(
    tree: &mut RojoTree,
    vfs: &Vfs,
    id: Ref,
    project_root: &Path,
) -> Option<ComputeResult> {
    // Use rel_path(p, project_root) inline for log display.

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
                rel_path(path, project_root),
                id,
                if is_init_file {
                    format!(
                        " (init file, will snapshot parent dir {})",
                        rel_path(snapshot_path, project_root)
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
                        rel_path(snapshot_path, project_root)
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
                            rel_path(path, project_root)
                        );

                        let snapshot =
                            match snapshot_from_vfs(&metadata.context, vfs, snapshot_path) {
                                Ok(snapshot) => snapshot,
                                Err(err) => {
                                    log::error!(
                                        "Recovery snapshot error for {}: {:?}",
                                        rel_path(snapshot_path, project_root),
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
                            rel_path(path, project_root),
                            rel_path(snapshot_path, project_root)
                        );

                        let snapshot =
                            match snapshot_from_vfs(&metadata.context, vfs, snapshot_path) {
                                Ok(snapshot) => snapshot,
                                Err(err) => {
                                    log::error!(
                                        "Directory re-snapshot error for {}: {:?}",
                                        rel_path(snapshot_path, project_root),
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
                            rel_path(path, project_root)
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
                        rel_path(path, project_root),
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
                rel_path(path, project_root)
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
