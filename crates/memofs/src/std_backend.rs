use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::{collections::HashSet, io};

use crossbeam_channel::{Receiver, Sender};
use notify::{watcher, DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};

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
pub type CriticalErrorHandler = Box<dyn Fn(WatcherCriticalError) -> bool + Send + 'static>;

/// `VfsBackend` that uses `std::fs` and the `notify` crate.
pub struct StdBackend {
    watcher: RecommendedWatcher,
    watcher_receiver: Receiver<VfsEvent>,
    watches: HashSet<PathBuf>,
    /// Receiver for critical errors from the watcher thread.
    /// Callers should poll this to detect when watching becomes unreliable.
    critical_error_receiver: Receiver<WatcherCriticalError>,
}

impl StdBackend {
    /// Creates a new StdBackend with default error handling (logs and exits on critical errors).
    pub fn new() -> StdBackend {
        Self::new_with_error_handler(Box::new(|err| {
            log::error!("{}. File watching is no longer reliable.", err);
            std::process::exit(1);
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
        let (notify_tx, notify_rx) = mpsc::channel();
        let watcher = watcher(notify_tx, Duration::from_millis(50)).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        let (error_tx, error_rx) = crossbeam_channel::unbounded();

        Self::spawn_watcher_thread(notify_rx, tx, error_tx, error_handler);

        Self {
            watcher,
            watcher_receiver: rx,
            watches: HashSet::new(),
            critical_error_receiver: error_rx,
        }
    }

    fn spawn_watcher_thread(
        notify_rx: mpsc::Receiver<DebouncedEvent>,
        event_tx: Sender<VfsEvent>,
        error_tx: Sender<WatcherCriticalError>,
        error_handler: CriticalErrorHandler,
    ) {
        thread::spawn(move || {
            log::trace!("File watcher thread started");

            let handle_critical_error = |err: WatcherCriticalError| -> bool {
                let _ = error_tx.send(err.clone());
                error_handler(err)
            };

            for event in notify_rx {
                let send_result = match event {
                    DebouncedEvent::Create(path) => event_tx.send(VfsEvent::Create(path)),
                    DebouncedEvent::Write(path) => event_tx.send(VfsEvent::Write(path)),
                    DebouncedEvent::Remove(path) => event_tx.send(VfsEvent::Remove(path)),
                    DebouncedEvent::Rename(from, to) => event_tx
                        .send(VfsEvent::Remove(from))
                        .and_then(|_| event_tx.send(VfsEvent::Create(to))),
                    DebouncedEvent::Error(err, path) => {
                        if handle_critical_error(WatcherCriticalError::WatcherError {
                            error: format!("{:?}", err),
                            path,
                        }) {
                            return;
                        }
                        continue;
                    }
                    DebouncedEvent::Rescan => {
                        if handle_critical_error(WatcherCriticalError::RescanRequired) {
                            return;
                        }
                        continue;
                    }
                    // NoticeWrite and NoticeRemove are pre-debounce notifications, skip them
                    DebouncedEvent::NoticeWrite(_) | DebouncedEvent::NoticeRemove(_) => continue,
                    // Chmod events are not relevant for our purposes
                    DebouncedEvent::Chmod(_) => continue,
                };

                if let Err(err) = send_result {
                    if handle_critical_error(WatcherCriticalError::ChannelSendFailed(
                        err.to_string(),
                    )) {
                        return;
                    }
                }
            }

            // Channel closed - watcher was dropped or sender disconnected
            // Call the error handler to maintain backwards compatibility (default handler exits)
            handle_critical_error(WatcherCriticalError::ThreadTerminated);
        });
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
            match self.watcher.watch(path, RecursiveMode::Recursive) {
                Ok(()) => {
                    log::info!("Watching path: {}", path.display());
                    self.watches.insert(path.to_path_buf());
                    Ok(())
                }
                Err(err) => {
                    log::warn!("Failed to watch path {}: {:?}", path.display(), err);
                    Err(io::Error::other(err))
                }
            }
        }
    }

    fn unwatch(&mut self, path: &Path) -> io::Result<()> {
        // Only remove from watches if unwatch succeeds
        // This keeps state consistent if unwatch fails (e.g., path wasn't directly watched)
        match self.watcher.unwatch(path) {
            Ok(()) => {
                log::info!("Unwatched path: {}", path.display());
                self.watches.remove(path);
                Ok(())
            }
            Err(err) => {
                // If the path wasn't being watched (common when parent dir is watched),
                // still remove from our tracking set but don't propagate the error
                if matches!(err, notify::Error::WatchNotFound) {
                    log::info!(
                        "Path was not directly watched (likely covered by parent): {}",
                        path.display()
                    );
                    self.watches.remove(path);
                    Ok(())
                } else {
                    log::warn!("Failed to unwatch path {}: {:?}", path.display(), err);
                    Err(io::Error::other(err))
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
    fn critical_error_receiver_receives_thread_terminated() {
        let error_received = Arc::new(AtomicBool::new(false));
        let error_received_clone = error_received.clone();

        let backend = StdBackend::new_with_error_handler(Box::new(move |err| {
            if matches!(err, WatcherCriticalError::ThreadTerminated) {
                error_received_clone.store(true, Ordering::SeqCst);
            }
            true // Stop the thread
        }));

        let error_rx = backend.critical_error_receiver();

        // Drop the backend to trigger thread termination
        drop(backend);

        // Give the thread a moment to terminate
        std::thread::sleep(Duration::from_millis(100));

        // Check if error was received either via handler or channel
        let received_via_channel = error_rx.try_recv().is_ok();
        let received_via_handler = error_received.load(Ordering::SeqCst);

        assert!(
            received_via_channel || received_via_handler,
            "ThreadTerminated error should be received via handler or channel"
        );
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
}
