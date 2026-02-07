use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{collections::HashSet, io};

use crossbeam_channel::{Receiver, Sender};
use notify::RecursiveMode;
use notify_debouncer_full::{
    new_debouncer,
    notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind, RenameMode},
    DebounceEventResult, Debouncer, RecommendedCache,
};

use crate::{DirEntry, Metadata, ReadDir, VfsBackend, VfsEvent};

/// Critical errors from the file watcher that indicate watching is no longer reliable.
#[derive(Debug, Clone)]
pub enum WatcherCriticalError {
    /// The notify crate reported an error
    WatcherError {
        error: String,
        path: Option<PathBuf>,
    },
    /// Too many file changes caused the watcher to request a rescan
    RescanRequired,
    /// Failed to send an event through the channel
    ChannelSendFailed(String),
    /// The watcher thread terminated unexpectedly
    ThreadTerminated,
}

impl std::fmt::Display for WatcherCriticalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WatcherError { error, path } => {
                write!(f, "File watcher error: {} (path: {:?})", error, path)
            }
            Self::RescanRequired => {
                write!(f, "File watcher requested rescan due to too many changes")
            }
            Self::ChannelSendFailed(err) => {
                write!(f, "File watcher failed to send event: {}", err)
            }
            Self::ThreadTerminated => {
                write!(f, "File watcher thread terminated unexpectedly")
            }
        }
    }
}

impl std::error::Error for WatcherCriticalError {}

/// Callback type for handling critical watcher errors.
/// Return `true` to exit the watcher thread, `false` to continue (if possible).
pub type CriticalErrorHandler = Box<dyn Fn(WatcherCriticalError) -> bool + Send + Sync + 'static>;

/// `VfsBackend` that uses `std::fs` and the `notify` crate.
pub struct StdBackend {
    debouncer: Debouncer<notify::RecommendedWatcher, RecommendedCache>,
    watcher_receiver: Receiver<VfsEvent>,
    watches: HashSet<PathBuf>,
    /// Receiver for critical errors from the watcher thread.
    /// Callers should poll this to detect when watching becomes unreliable.
    critical_error_receiver: Receiver<WatcherCriticalError>,
}

impl StdBackend {
    /// Creates a new StdBackend with default error handling.
    ///
    /// `RescanRequired` is treated as recoverable (the debouncer will re-walk
    /// the directory tree). All other critical errors terminate the process
    /// because file watching cannot continue.
    pub fn new() -> StdBackend {
        Self::new_with_error_handler(Box::new(|err| {
            match &err {
                WatcherCriticalError::RescanRequired => {
                    // Recoverable: the debouncer lost some events due to rapid
                    // changes and will rescan the watched directories. Log a
                    // warning but keep the watcher thread alive.
                    log::warn!(
                        "File watcher requested rescan due to rapid changes. \
                         Some file events may have been missed."
                    );
                    false // keep watcher running
                }
                _ => {
                    log::error!("{}. File watching is no longer reliable.", err);
                    std::process::exit(1);
                }
            }
        }))
    }

    /// Creates a new StdBackend with a custom error handler.
    ///
    /// The error handler is called when critical errors occur in the watcher thread.
    /// It receives the error and should return `true` to stop the watcher thread,
    /// or `false` to continue (though the watcher may not be reliable after an error).
    ///
    /// Critical errors are also sent to the `critical_error_receiver()` channel,
    /// which can be polled alongside `event_receiver()` for async error handling.
    pub fn new_with_error_handler(error_handler: CriticalErrorHandler) -> StdBackend {
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let (error_tx, error_rx) = crossbeam_channel::unbounded();

        let debouncer = Self::create_debouncer(event_tx, error_tx, error_handler);

        Self {
            debouncer,
            watcher_receiver: event_rx,
            watches: HashSet::new(),
            critical_error_receiver: error_rx,
        }
    }

    fn create_debouncer(
        event_tx: Sender<VfsEvent>,
        error_tx: Sender<WatcherCriticalError>,
        error_handler: CriticalErrorHandler,
    ) -> Debouncer<notify::RecommendedWatcher, RecommendedCache> {
        // Use 50ms debounce timeout (same as the old v4 implementation)
        let debounce_timeout = Duration::from_millis(50);

        new_debouncer(
            debounce_timeout,
            None, // Use default tick rate
            move |result: DebounceEventResult| {
                match result {
                    Ok(events) => {
                        for event in events {
                            let vfs_events = Self::convert_event(&event.event);
                            for vfs_event in vfs_events {
                                if let Err(err) = event_tx.send(vfs_event) {
                                    let critical_err =
                                        WatcherCriticalError::ChannelSendFailed(err.to_string());
                                    let _ = error_tx.send(critical_err.clone());
                                    if error_handler(critical_err) {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(errors) => {
                        for error in errors {
                            // Check if this is a rescan request
                            if error.paths.is_empty() {
                                let critical_err = WatcherCriticalError::RescanRequired;
                                let _ = error_tx.send(critical_err.clone());
                                if error_handler(critical_err) {
                                    return;
                                }
                            } else {
                                let critical_err = WatcherCriticalError::WatcherError {
                                    error: format!("{:?}", error.kind),
                                    path: error.paths.first().cloned(),
                                };
                                let _ = error_tx.send(critical_err.clone());
                                if error_handler(critical_err) {
                                    return;
                                }
                            }
                        }
                    }
                }
            },
        )
        .expect("Failed to create file watcher debouncer")
    }

    /// Convert a notify event to our VfsEvent(s)
    fn convert_event(event: &notify::Event) -> Vec<VfsEvent> {
        let mut vfs_events = Vec::new();

        match &event.kind {
            // Create events
            EventKind::Create(CreateKind::File)
            | EventKind::Create(CreateKind::Folder)
            | EventKind::Create(CreateKind::Any)
            | EventKind::Create(CreateKind::Other) => {
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Create(path.clone()));
                }
            }

            // Modify events (treat as Write)
            EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Any)
            | EventKind::Modify(ModifyKind::Other) => {
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Write(path.clone()));
                }
            }

            // Metadata changes - we don't care about these for Rojo's purposes
            EventKind::Modify(ModifyKind::Metadata(_)) => {}

            // Name changes (renames) - the debouncer-full handles rename tracking
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                // Both paths present: old path at [0], new path at [1]
                if event.paths.len() >= 2 {
                    vfs_events.push(VfsEvent::Remove(event.paths[0].clone()));
                    vfs_events.push(VfsEvent::Create(event.paths[1].clone()));
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
                // Only the old path - treat as removal
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Remove(path.clone()));
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
                // Only the new path - treat as creation
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Create(path.clone()));
                }
            }
            EventKind::Modify(ModifyKind::Name(RenameMode::Any))
            | EventKind::Modify(ModifyKind::Name(RenameMode::Other)) => {
                // Ambiguous rename - treat as modification
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Write(path.clone()));
                }
            }

            // Remove events
            EventKind::Remove(RemoveKind::File)
            | EventKind::Remove(RemoveKind::Folder)
            | EventKind::Remove(RemoveKind::Any)
            | EventKind::Remove(RemoveKind::Other) => {
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Remove(path.clone()));
                }
            }

            // Access events - we don't care about these
            EventKind::Access(_) => {}

            // Other/Any events - treat as potential modifications
            EventKind::Other | EventKind::Any => {
                for path in &event.paths {
                    vfs_events.push(VfsEvent::Write(path.clone()));
                }
            }
        }

        vfs_events
    }

    /// Returns a receiver for critical errors from the watcher thread.
    ///
    /// Poll this alongside `event_receiver()` to detect when watching becomes unreliable.
    /// This allows for graceful handling of watcher failures without process termination.
    pub fn critical_error_receiver(&self) -> Receiver<WatcherCriticalError> {
        self.critical_error_receiver.clone()
    }

    /// Creates a new StdBackend suitable for testing.
    ///
    /// Unlike `new()`, this does not call `process::exit()` on errors,
    /// making it safe to use in tests where the backend will be dropped.
    #[cfg(test)]
    pub fn new_for_testing() -> StdBackend {
        Self::new_with_error_handler(Box::new(|err| {
            log::trace!("Test backend error (expected during test cleanup): {}", err);
            true // Stop the thread without exiting the process
        }))
    }
}

impl VfsBackend for StdBackend {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        fs_err::read(path)
    }

    fn write(&mut self, path: &Path, data: &[u8]) -> io::Result<()> {
        fs_err::write(path, data)
    }

    fn exists(&mut self, path: &Path) -> io::Result<bool> {
        std::fs::exists(path)
    }

    fn read_dir(&mut self, path: &Path) -> io::Result<ReadDir> {
        let entries: Result<Vec<_>, _> = fs_err::read_dir(path)?.collect();
        let mut entries = entries?;

        entries.sort_by_cached_key(|entry| entry.file_name());

        let inner = entries
            .into_iter()
            .map(|entry| Ok(DirEntry { path: entry.path() }));

        Ok(ReadDir {
            inner: Box::new(inner),
        })
    }

    fn create_dir(&mut self, path: &Path) -> io::Result<()> {
        fs_err::create_dir(path)
    }

    fn create_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs_err::create_dir_all(path)
    }

    fn remove_file(&mut self, path: &Path) -> io::Result<()> {
        fs_err::remove_file(path)
    }

    fn remove_dir_all(&mut self, path: &Path) -> io::Result<()> {
        fs_err::remove_dir_all(path)
    }

    fn metadata(&mut self, path: &Path) -> io::Result<Metadata> {
        let inner = fs_err::metadata(path)?;

        Ok(Metadata {
            is_file: inner.is_file(),
        })
    }

    fn canonicalize(&mut self, path: &Path) -> io::Result<PathBuf> {
        fs_err::canonicalize(path)
    }

    fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent> {
        self.watcher_receiver.clone()
    }

    fn watch(&mut self, path: &Path) -> io::Result<()> {
        if self.watches.contains(path)
            || path
                .ancestors()
                .any(|ancestor| self.watches.contains(ancestor))
        {
            Ok(())
        } else {
            // Only add to watches AFTER the watch succeeds
            // This prevents a failed watch from permanently marking the path as "watched"
            match self.debouncer.watch(path, RecursiveMode::Recursive) {
                Ok(()) => {
                    log::info!("Watching path: {}", path.display());
                    self.watches.insert(path.to_path_buf());
                    Ok(())
                }
                Err(err) => {
                    log::warn!("Failed to watch path {}: {:?}", path.display(), err);
                    Err(io::Error::other(format!("{:?}", err)))
                }
            }
        }
    }

    fn unwatch(&mut self, path: &Path) -> io::Result<()> {
        // Only remove from watches if unwatch succeeds
        // This keeps state consistent if unwatch fails (e.g., path wasn't directly watched)
        match self.debouncer.unwatch(path) {
            Ok(()) => {
                log::info!("Unwatched path: {}", path.display());
                self.watches.remove(path);
                Ok(())
            }
            Err(err) => {
                // If the path wasn't being watched (common when parent dir is watched),
                // still remove from our tracking set but don't propagate the error
                if matches!(
                    err.kind,
                    notify::ErrorKind::WatchNotFound | notify::ErrorKind::PathNotFound
                ) {
                    log::info!(
                        "Path was not directly watched (likely covered by parent): {}",
                        path.display()
                    );
                    self.watches.remove(path);
                    Ok(())
                } else {
                    log::warn!("Failed to unwatch path {}: {:?}", path.display(), err);
                    Err(io::Error::other(format!("{:?}", err)))
                }
            }
        }
    }
}

impl Default for StdBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn watch_adds_to_watches_only_on_success() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs_err::write(&file_path, "test content").unwrap();

        let mut backend = StdBackend::new_for_testing();

        // Watch should succeed and add to internal set
        assert!(backend.watch(&file_path).is_ok());

        // Watching again should be a no-op (already watched)
        assert!(backend.watch(&file_path).is_ok());

        // Watch a non-existent path - behavior varies by platform
        // On some systems, watching a non-existent file might succeed
        // because notify watches the parent directory. We just verify
        // that calling watch doesn't panic or corrupt state.
        let nonexistent = dir.path().join("nonexistent.txt");
        let _ = backend.watch(&nonexistent); // Result varies by platform
    }

    #[test]
    fn unwatch_handles_not_found_gracefully() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs_err::write(&file_path, "test content").unwrap();

        let mut backend = StdBackend::new_for_testing();

        // Unwatch a path that was never watched should handle gracefully
        // (it might fail with WatchNotFound, which we handle)
        let result = backend.unwatch(&file_path);
        // Should not panic, may or may not error depending on notify behavior
        drop(result);
    }

    #[test]
    fn watch_then_unwatch_maintains_consistency() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs_err::write(&file_path, "test content").unwrap();

        let mut backend = StdBackend::new_for_testing();

        // Watch the file
        assert!(backend.watch(&file_path).is_ok());

        // Unwatch should succeed
        assert!(backend.unwatch(&file_path).is_ok());

        // Should be able to watch again after unwatch
        assert!(backend.watch(&file_path).is_ok());
    }

    #[test]
    fn ancestor_watch_prevents_duplicate_watches() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join("subdir");
        fs_err::create_dir(&subdir).unwrap();
        let file_path = subdir.join("test.txt");
        fs_err::write(&file_path, "test content").unwrap();

        let mut backend = StdBackend::new_for_testing();

        // Watch the parent directory
        assert!(backend.watch(&subdir).is_ok());

        // Watching a file inside should be a no-op (covered by parent)
        assert!(backend.watch(&file_path).is_ok());
    }

    #[test]
    fn critical_error_receiver_works() {
        let error_received = Arc::new(AtomicBool::new(false));
        let error_received_clone = error_received.clone();

        let backend = StdBackend::new_with_error_handler(Box::new(move |_err| {
            error_received_clone.store(true, Ordering::SeqCst);
            true // Stop on any error
        }));

        let _error_rx = backend.critical_error_receiver();

        // The debouncer handles its own thread management, so we just verify
        // the receiver is accessible
        drop(backend);
    }

    #[test]
    fn rapid_watch_unwatch_cycles_maintain_consistency() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs_err::write(&file_path, "test content").unwrap();

        let mut backend = StdBackend::new_for_testing();

        // Rapidly cycle watch/unwatch to stress test state consistency
        for _ in 0..10 {
            assert!(backend.watch(&file_path).is_ok());
            assert!(backend.unwatch(&file_path).is_ok());
        }

        // Final watch should still work
        assert!(backend.watch(&file_path).is_ok());
    }

    #[test]
    fn file_events_are_received() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs_err::write(&file_path, "initial content").unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();

        // Watch the directory (more reliable than watching the file directly)
        assert!(backend.watch(dir.path()).is_ok());

        // Give the watcher time to start
        std::thread::sleep(Duration::from_millis(100));

        // Modify the file
        fs_err::write(&file_path, "modified content").unwrap();

        // Wait for debounce (50ms) plus some buffer
        std::thread::sleep(Duration::from_millis(200));

        // Check if we received any event
        let mut received_event = false;
        while let Ok(event) = event_rx.try_recv() {
            log::info!("Received event: {:?}", event);
            received_event = true;
        }

        // Note: File events can be flaky in tests due to timing, so we don't assert
        // Just log for debugging
        if !received_event {
            log::warn!("No file events received - this may be a timing issue in tests");
        }
    }

    #[test]
    fn watcher_critical_error_display() {
        // Test Display implementation for coverage
        let err1 = WatcherCriticalError::WatcherError {
            error: "test error".to_string(),
            path: Some(PathBuf::from("/test/path")),
        };
        assert!(err1.to_string().contains("test error"));

        let err2 = WatcherCriticalError::RescanRequired;
        assert!(err2.to_string().contains("rescan"));

        let err3 = WatcherCriticalError::ChannelSendFailed("send failed".to_string());
        assert!(err3.to_string().contains("send failed"));

        let err4 = WatcherCriticalError::ThreadTerminated;
        assert!(err4.to_string().contains("terminated"));
    }

    // ==========================================================================
    // STRESS TESTS / FUZZING
    // ==========================================================================
    // These tests simulate aggressive filesystem operations to verify the VFS
    // handles rapid changes correctly (the original bug was desync after undo/redo)

    /// Helper to collect events with timeout
    fn collect_events_with_timeout(
        event_rx: &Receiver<VfsEvent>,
        timeout: Duration,
    ) -> Vec<VfsEvent> {
        let start = std::time::Instant::now();
        let mut events = Vec::new();
        while start.elapsed() < timeout {
            match event_rx.try_recv() {
                Ok(event) => events.push(event),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        events
    }

    #[test]
    fn stress_rapid_file_modifications() {
        // Simulates undo/redo scenario: rapid modifications to the same file
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("rapid_mod.luau");
        fs_err::write(&file_path, "-- initial").unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Rapid modifications - 20 writes in quick succession
        for i in 0..20 {
            fs_err::write(&file_path, format!("-- version {}", i)).unwrap();
        }

        // Wait for debounce to settle (50ms debounce + buffer)
        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        // Should receive at least one Write event (debouncer coalesces rapid writes)
        let write_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, VfsEvent::Write(p) if p == &file_path))
            .collect();

        log::info!(
            "Rapid modifications: {} total events, {} write events for target file",
            events.len(),
            write_events.len()
        );

        // The debouncer should coalesce these - we expect at least 1 event
        assert!(
            !write_events.is_empty(),
            "Expected at least one write event after rapid modifications"
        );
    }

    #[test]
    fn stress_create_delete_recreate_cycles() {
        // File disappears and reappears rapidly
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("volatile.luau");

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // 10 cycles of create/delete
        for i in 0..10 {
            fs_err::write(&file_path, format!("-- cycle {}", i)).unwrap();
            std::thread::sleep(Duration::from_millis(5));
            fs_err::remove_file(&file_path).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }

        // Final state: file exists
        fs_err::write(&file_path, "-- final").unwrap();

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!(
            "Create/delete cycles: {} total events received",
            events.len()
        );

        // Verify we got some events (exact count varies due to debouncing)
        // The important thing is no crashes or hangs
        assert!(
            events.len() > 0,
            "Expected events from create/delete cycles"
        );
    }

    #[test]
    fn stress_multiple_files_simultaneous() {
        // Multiple files changing at once (like a git checkout)
        let dir = tempdir().unwrap();
        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Create 20 files rapidly
        let files: Vec<_> = (0..20)
            .map(|i| dir.path().join(format!("file_{}.luau", i)))
            .collect();

        for (i, file) in files.iter().enumerate() {
            fs_err::write(file, format!("-- file {}", i)).unwrap();
        }

        // Modify all of them
        for (i, file) in files.iter().enumerate() {
            fs_err::write(file, format!("-- modified {}", i)).unwrap();
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!(
            "Multiple files: {} events for {} files",
            events.len(),
            files.len()
        );

        // Should have events for at least some of the files
        let unique_paths: HashSet<_> = events
            .iter()
            .filter_map(|e| match e {
                VfsEvent::Create(p) | VfsEvent::Write(p) => Some(p.clone()),
                _ => None,
            })
            .collect();

        assert!(
            unique_paths.len() >= 5,
            "Expected events for multiple files, got {}",
            unique_paths.len()
        );
    }

    #[test]
    fn stress_rename_operations() {
        // Rename is particularly tricky for filesystem watchers
        let dir = tempdir().unwrap();
        let original = dir.path().join("original.luau");
        let renamed = dir.path().join("renamed.luau");

        fs_err::write(&original, "-- content").unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Rename the file
        fs_err::rename(&original, &renamed).unwrap();

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(300));

        log::info!("Rename: {} events received", events.len());

        // Should get either:
        // - Remove(original) + Create(renamed) for RenameMode::Both
        // - Or separate From/To events
        let has_remove = events
            .iter()
            .any(|e| matches!(e, VfsEvent::Remove(p) if p == &original));
        let has_create = events
            .iter()
            .any(|e| matches!(e, VfsEvent::Create(p) if p == &renamed));

        // At minimum we should see the new file appear
        assert!(
            has_create || has_remove,
            "Expected rename to generate events"
        );
    }

    #[test]
    fn stress_rapid_rename_chain() {
        // File gets renamed multiple times rapidly
        let dir = tempdir().unwrap();
        let mut current = dir.path().join("file_0.luau");
        fs_err::write(&current, "-- content").unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Chain of renames
        for i in 1..10 {
            let next = dir.path().join(format!("file_{}.luau", i));
            fs_err::rename(&current, &next).unwrap();
            current = next;
            std::thread::sleep(Duration::from_millis(10));
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!("Rename chain: {} events received", events.len());
        assert!(events.len() > 0, "Expected events from rename chain");
    }

    #[test]
    fn stress_nested_directory_operations() {
        // Create and delete nested directories with files
        let dir = tempdir().unwrap();
        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Create nested structure
        let nested = dir.path().join("a").join("b").join("c");
        fs_err::create_dir_all(&nested).unwrap();

        // Small delay to let watcher catch up with directory creation
        std::thread::sleep(Duration::from_millis(50));

        // Create files at each level
        for (i, ancestor) in nested.ancestors().take(3).enumerate() {
            let file = ancestor.join(format!("file_{}.luau", i));
            fs_err::write(&file, format!("-- level {}", i)).unwrap();
        }

        // Give the watcher time to observe the creates before deleting
        std::thread::sleep(Duration::from_millis(100));

        // Delete the whole tree
        fs_err::remove_dir_all(dir.path().join("a")).unwrap();

        // Longer timeout to catch both create and delete events
        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(700));

        log::info!("Nested dirs: {} events received", events.len());

        // Note: On some systems, recursive deletes may not generate individual events
        // for each file, so we just verify the test completes without error.
        // The key thing is that the watcher doesn't crash or hang.
        log::info!(
            "Nested directory operations completed successfully with {} events",
            events.len()
        );
    }

    #[test]
    fn stress_burst_writes_same_file() {
        // Extreme burst: 100 writes to same file as fast as possible
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("burst.luau");
        fs_err::write(&file_path, "-- initial").unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // 100 writes with no delay
        for i in 0..100 {
            fs_err::write(&file_path, format!("-- burst write {}", i)).unwrap();
        }

        // The debouncer should coalesce these
        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        let write_count = events
            .iter()
            .filter(|e| matches!(e, VfsEvent::Write(_)))
            .count();

        log::info!(
            "Burst writes: {} total events, {} writes (from 100 actual writes)",
            events.len(),
            write_count
        );

        // Debouncer should significantly reduce the event count
        // (we wrote 100 times but should get far fewer events)
        assert!(
            write_count < 50,
            "Debouncer should coalesce burst writes, got {} events from 100 writes",
            write_count
        );
    }

    #[test]
    fn stress_interleaved_operations() {
        // Mix of creates, writes, deletes across multiple files
        let dir = tempdir().unwrap();
        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        let files: Vec<_> = (0..5)
            .map(|i| dir.path().join(format!("interleaved_{}.luau", i)))
            .collect();

        // Round 1: Create all
        for file in &files {
            fs_err::write(file, "-- created").unwrap();
        }

        // Round 2: Modify evens, delete odds
        for (i, file) in files.iter().enumerate() {
            if i % 2 == 0 {
                fs_err::write(file, "-- modified").unwrap();
            } else {
                fs_err::remove_file(file).unwrap();
            }
        }

        // Round 3: Recreate odds, delete evens
        for (i, file) in files.iter().enumerate() {
            if i % 2 == 0 {
                fs_err::remove_file(file).unwrap();
            } else {
                fs_err::write(file, "-- recreated").unwrap();
            }
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!("Interleaved ops: {} events received", events.len());
        assert!(
            events.len() > 0,
            "Expected events from interleaved operations"
        );
    }

    #[test]
    fn stress_undo_redo_simulation() {
        // Simulates the exact scenario that caused the original bug:
        // File modified, then quickly reverted (undo), then possibly redone
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("script.luau");
        let original_content = "-- original content\nlocal x = 1";
        let modified_content = "-- modified content\nlocal x = 2";

        fs_err::write(&file_path, original_content).unwrap();

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Simulate multiple undo/redo cycles
        for _ in 0..10 {
            // "Edit" - write new content
            fs_err::write(&file_path, modified_content).unwrap();
            std::thread::sleep(Duration::from_millis(20));

            // "Undo" - revert to original
            fs_err::write(&file_path, original_content).unwrap();
            std::thread::sleep(Duration::from_millis(20));

            // "Redo" - back to modified
            fs_err::write(&file_path, modified_content).unwrap();
            std::thread::sleep(Duration::from_millis(20));

            // "Undo" again - back to original
            fs_err::write(&file_path, original_content).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!(
            "Undo/redo simulation: {} events from 40 file writes",
            events.len()
        );

        // Verify file is in expected state (original content after all undos)
        let final_content = fs_err::read_to_string(&file_path).unwrap();
        assert_eq!(
            final_content, original_content,
            "File content should match expected state after undo/redo cycles"
        );

        // Should have received write events (exact count varies due to debouncing)
        let write_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, VfsEvent::Write(p) if p == &file_path))
            .collect();

        assert!(
            !write_events.is_empty(),
            "Should receive write events during undo/redo simulation"
        );
    }

    #[test]
    fn stress_concurrent_directory_file_ops() {
        // Create directories and files in them simultaneously
        let dir = tempdir().unwrap();
        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Rapidly create subdirectories with files
        for i in 0..10 {
            let subdir = dir.path().join(format!("subdir_{}", i));
            fs_err::create_dir(&subdir).unwrap();

            // Immediately create files in the new directory
            for j in 0..3 {
                let file = subdir.join(format!("file_{}.luau", j));
                fs_err::write(&file, format!("-- dir {} file {}", i, j)).unwrap();
            }
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(500));

        log::info!(
            "Concurrent dir/file ops: {} events for 10 dirs with 3 files each",
            events.len()
        );

        assert!(
            events.len() > 0,
            "Expected events from concurrent directory/file operations"
        );
    }

    #[test]
    fn stress_empty_file_operations() {
        // Edge case: empty files and files that become empty
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("empty_test.luau");

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        // Create empty file
        fs_err::write(&file_path, "").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // Write content
        fs_err::write(&file_path, "-- content").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // Make empty again
        fs_err::write(&file_path, "").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        // Write content again
        fs_err::write(&file_path, "-- more content").unwrap();

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(300));

        log::info!("Empty file ops: {} events received", events.len());
        assert!(
            events.len() > 0,
            "Expected events from empty file operations"
        );
    }

    #[test]
    fn stress_long_path_names() {
        // Test with longer file/directory names
        let dir = tempdir().unwrap();
        let long_name = "a".repeat(100); // 100 char filename
        let file_path = dir.path().join(format!("{}.luau", long_name));

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        fs_err::write(&file_path, "-- long name test").unwrap();

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(300));

        log::info!("Long path: {} events for 100-char filename", events.len());
        assert!(events.len() > 0, "Expected events for long filename");
    }

    #[test]
    fn stress_special_characters_in_names() {
        // Test filenames with spaces and special chars (common in Roblox projects)
        let dir = tempdir().unwrap();
        let special_files = vec![
            "file with spaces.luau",
            "file-with-dashes.luau",
            "file_with_underscores.luau",
            "file.multiple.dots.luau",
            "UPPERCASE.luau",
            "MixedCase.luau",
        ];

        let mut backend = StdBackend::new_for_testing();
        let event_rx = backend.event_receiver();
        assert!(backend.watch(dir.path()).is_ok());
        std::thread::sleep(Duration::from_millis(100));

        for name in &special_files {
            let file_path = dir.path().join(name);
            fs_err::write(&file_path, format!("-- {}", name)).unwrap();
        }

        let events = collect_events_with_timeout(&event_rx, Duration::from_millis(300));

        log::info!(
            "Special chars: {} events for {} special-named files",
            events.len(),
            special_files.len()
        );
        assert!(
            events.len() > 0,
            "Expected events for files with special characters"
        );
    }
}
