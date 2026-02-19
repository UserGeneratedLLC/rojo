/*!
Implementation of a virtual filesystem with a configurable backend and file
watching.

memofs is currently an unstable minimum viable library. Its primary consumer is
[Atlas](https://github.com/UserGeneratedLLC/rojo), a build system for Roblox.

## Current Features
* API similar to `std::fs`
* Configurable backends
    * `StdBackend`, which uses `std::fs` and the `notify` crate
    * `NoopBackend`, which always throws errors
    * `InMemoryFs`, a simple in-memory filesystem useful for testing

## Future Features
* Hash-based hierarchical memoization keys (hence the name)
* Configurable caching (write-through, write-around, write-back)
*/

mod in_memory_fs;
mod noop_backend;
mod snapshot;
mod std_backend;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::{io, str};

pub use in_memory_fs::InMemoryFs;
pub use noop_backend::NoopBackend;
pub use snapshot::VfsSnapshot;
pub use std_backend::{CriticalErrorHandler, StdBackend, WatcherCriticalError};

/// Pre-read file contents, canonical paths, and metadata for fast startup.
///
/// Populated in parallel (via rayon + walkdir) before snapshot building,
/// then consumed by VFS reads during `snapshot_from_vfs()`. Cleared after
/// the initial tree build to avoid stale data during live operation.
pub struct PrefetchCache {
    pub files: HashMap<PathBuf, Vec<u8>>,
    pub canonical: HashMap<PathBuf, PathBuf>,
    /// `true` = file, `false` = directory. Paths not in the map fall through
    /// to the backend (e.g. init-file probes for paths that don't exist).
    pub is_file: HashMap<PathBuf, bool>,
    /// Directory path -> sorted child paths. Consumed once per directory.
    pub children: HashMap<PathBuf, Vec<PathBuf>>,
}

mod sealed {
    use super::*;

    /// Sealing trait for VfsBackend.
    pub trait Sealed {}

    impl Sealed for NoopBackend {}
    impl Sealed for StdBackend {}
    impl Sealed for InMemoryFs {}
}

/// Trait that transforms `io::Result<T>` into `io::Result<Option<T>>`.
///
/// `Ok(None)` takes the place of IO errors whose `io::ErrorKind` is `NotFound`.
pub trait IoResultExt<T> {
    fn with_not_found(self) -> io::Result<Option<T>>;
}

impl<T> IoResultExt<T> for io::Result<T> {
    fn with_not_found(self) -> io::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(err)
                }
            }
        }
    }
}

/// Backend that can be used to create a `Vfs`.
///
/// This trait is sealed and cannot not be implemented outside this crate.
pub trait VfsBackend: sealed::Sealed + Send + 'static {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    fn write(&mut self, path: &Path, data: &[u8]) -> io::Result<()>;
    fn exists(&mut self, path: &Path) -> io::Result<bool>;
    fn read_dir(&mut self, path: &Path) -> io::Result<ReadDir>;
    fn create_dir(&mut self, path: &Path) -> io::Result<()>;
    fn create_dir_all(&mut self, path: &Path) -> io::Result<()>;
    fn metadata(&mut self, path: &Path) -> io::Result<Metadata>;
    fn remove_file(&mut self, path: &Path) -> io::Result<()>;
    fn remove_dir_all(&mut self, path: &Path) -> io::Result<()>;
    fn canonicalize(&mut self, path: &Path) -> io::Result<PathBuf>;

    fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent>;
    fn watch(&mut self, path: &Path) -> io::Result<()>;
    fn unwatch(&mut self, path: &Path) -> io::Result<()>;
}

/// Vfs equivalent to [`std::fs::DirEntry`][std::fs::DirEntry].
///
/// [std::fs::DirEntry]: https://doc.rust-lang.org/stable/std/fs/struct.DirEntry.html
pub struct DirEntry {
    pub(crate) path: PathBuf,
}

impl DirEntry {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Vfs equivalent to [`std::fs::ReadDir`][std::fs::ReadDir].
///
/// [std::fs::ReadDir]: https://doc.rust-lang.org/stable/std/fs/struct.ReadDir.html
pub struct ReadDir {
    pub(crate) inner: Box<dyn Iterator<Item = io::Result<DirEntry>>>,
}

impl Iterator for ReadDir {
    type Item = io::Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

/// Vfs equivalent to [`std::fs::Metadata`][std::fs::Metadata].
///
/// [std::fs::Metadata]: https://doc.rust-lang.org/stable/std/fs/struct.Metadata.html
#[derive(Debug)]
pub struct Metadata {
    pub(crate) is_file: bool,
}

impl Metadata {
    pub fn is_file(&self) -> bool {
        self.is_file
    }

    pub fn is_dir(&self) -> bool {
        !self.is_file
    }
}

/// Represents an event that a filesystem can raise that might need to be
/// handled.
#[derive(Debug)]
#[non_exhaustive]
pub enum VfsEvent {
    Create(PathBuf),
    Write(PathBuf),
    Remove(PathBuf),
}

/// Contains implementation details of the Vfs, wrapped by `Vfs` and `VfsLock`,
/// the public interfaces to this type.
struct VfsInner {
    backend: Box<dyn VfsBackend>,
    watch_enabled: bool,
    prefetch_cache: Option<PrefetchCache>,
}

impl VfsInner {
    /// Read raw bytes from the prefetch cache or the backend.
    /// Removes the entry from the cache on hit to free memory.
    fn read_raw(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        if let Some(cache) = &mut self.prefetch_cache {
            if let Some(contents) = cache.files.remove(path) {
                if self.watch_enabled {
                    self.backend.watch(path)?;
                }
                return Ok(contents);
            }
        }

        let contents = self.backend.read(path)?;

        if self.watch_enabled {
            self.backend.watch(path)?;
        }

        Ok(contents)
    }

    fn read<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Arc<Vec<u8>>> {
        let path = path.as_ref();
        Ok(Arc::new(self.read_raw(path)?))
    }

    fn read_to_string<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Arc<String>> {
        let path = path.as_ref();
        let contents = self.read_raw(path)?;

        let contents_str = str::from_utf8(&contents).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("File was not valid UTF-8: {}", path.display()),
            )
        })?;

        Ok(Arc::new(contents_str.into()))
    }

    fn exists<P: AsRef<Path>>(&mut self, path: P) -> io::Result<bool> {
        let path = path.as_ref();
        self.backend.exists(path)
    }

    fn write<P: AsRef<Path>, C: AsRef<[u8]>>(&mut self, path: P, contents: C) -> io::Result<()> {
        let path = path.as_ref();
        let contents = contents.as_ref();
        self.backend.write(path, contents)
    }

    fn read_dir<P: AsRef<Path>>(&mut self, path: P) -> io::Result<ReadDir> {
        let path = path.as_ref();

        if let Some(cache) = &mut self.prefetch_cache {
            if let Some(child_paths) = cache.children.remove(path) {
                if self.watch_enabled {
                    self.backend.watch(path)?;
                }
                let inner = child_paths.into_iter().map(|p| Ok(DirEntry { path: p }));
                return Ok(ReadDir {
                    inner: Box::new(inner),
                });
            }
        }

        let dir = self.backend.read_dir(path)?;

        if self.watch_enabled {
            self.backend.watch(path)?;
        }

        Ok(dir)
    }

    fn create_dir<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.backend.create_dir(path)
    }

    fn create_dir_all<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.backend.create_dir_all(path)
    }

    fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        if self.watch_enabled {
            let _ = self.backend.unwatch(path);
        }
        self.backend.remove_file(path)
    }

    fn remove_dir_all<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        if self.watch_enabled {
            let _ = self.backend.unwatch(path);
        }
        self.backend.remove_dir_all(path)
    }

    fn metadata<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Metadata> {
        let path = path.as_ref();

        if let Some(cache) = &self.prefetch_cache {
            if let Some(&is_file) = cache.is_file.get(path) {
                return Ok(Metadata { is_file });
            }
        }

        self.backend.metadata(path)
    }

    fn canonicalize<P: AsRef<Path>>(&mut self, path: P) -> io::Result<PathBuf> {
        let path = path.as_ref();

        if let Some(cache) = &mut self.prefetch_cache {
            if let Some(canonical) = cache.canonical.remove(path) {
                return Ok(canonical);
            }
        }

        self.backend.canonicalize(path)
    }

    fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent> {
        self.backend.event_receiver()
    }

    fn commit_event(&mut self, event: &VfsEvent) -> io::Result<()> {
        // NOTE: We intentionally do NOT unwatch on Remove events.
        // The path may be recreated immediately (e.g., editor undo), and
        // unwatching causes future events for the recreated path to be missed.
        // Stale watches are harmless — notify silently ignores events for
        // non-existent paths, and the watch will be cleaned up when the
        // parent is unwatched.
        let _ = event;
        Ok(())
    }
}

/// A virtual filesystem with a configurable backend.
///
/// All operations on the Vfs take a lock on an internal backend. For performing
/// large batches of operations, it might be more performant to call `lock()`
/// and use [`VfsLock`](struct.VfsLock.html) instead.
pub struct Vfs {
    inner: Mutex<VfsInner>,
}

impl Vfs {
    /// Creates a new `Vfs` with the default backend, `StdBackend`.
    pub fn new_default() -> Self {
        Self::new(StdBackend::new())
    }

    /// Creates a new `Vfs` with the default backend, also returning
    /// the critical error receiver for monitoring watcher health.
    ///
    /// Use this when you need to detect `RescanRequired` errors and
    /// trigger a tree reconciliation after lost events.
    pub fn new_default_with_errors() -> (Self, crossbeam_channel::Receiver<WatcherCriticalError>) {
        let backend = StdBackend::new();
        let error_rx = backend.critical_error_receiver();
        (Self::new(backend), error_rx)
    }

    /// Creates a new `Vfs` suitable for one-shot operations like syncback.
    ///
    /// Unlike `new_default()`, this creates a backend that:
    /// - Has file watching disabled by default
    /// - Uses a non-fatal error handler for watcher issues (logs instead of exiting)
    ///
    /// This is ideal for CLI commands that don't need real-time file watching
    /// and shouldn't be terminated if the watcher thread encounters issues.
    pub fn new_oneshot() -> Self {
        let backend = StdBackend::new_with_error_handler(Box::new(|err| {
            // Log the error but don't exit - one-shot operations don't need file watching
            log::debug!(
                "File watcher issue (non-fatal for one-shot operation): {}",
                err
            );
            true // Stop the watcher thread, but don't exit the process
        }));
        let vfs = Self::new(backend);
        vfs.set_watch_enabled(false);
        vfs
    }

    /// Creates a new `Vfs` with the given backend.
    pub fn new<B: VfsBackend>(backend: B) -> Self {
        let lock = VfsInner {
            backend: Box::new(backend),
            watch_enabled: true,
            prefetch_cache: None,
        };

        Self {
            inner: Mutex::new(lock),
        }
    }

    /// Load a prefetch cache for fast initial reads.
    ///
    /// File reads and canonicalize calls will check the cache before hitting
    /// the backend. Call [`clear_prefetch_cache`] after the initial snapshot
    /// build to free memory and ensure live operations get fresh data.
    pub fn set_prefetch_cache(&self, cache: PrefetchCache) {
        let mut inner = self.inner.lock().unwrap();
        inner.prefetch_cache = Some(cache);
    }

    /// Drop the prefetch cache, freeing memory.
    ///
    /// After this call, all reads go through the backend as normal.
    pub fn clear_prefetch_cache(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.prefetch_cache = None;
    }

    /// Manually lock the Vfs, useful for large batches of operations.
    pub fn lock(&self) -> VfsLock<'_> {
        VfsLock {
            inner: self.inner.lock().unwrap(),
        }
    }

    /// Turns automatic file watching on or off. Enabled by default.
    ///
    /// Turning off file watching may be useful for single-use cases, especially
    /// on platforms like macOS where registering file watches has significant
    /// performance cost.
    pub fn set_watch_enabled(&self, enabled: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.watch_enabled = enabled;
    }

    /// Read a file from the VFS, or the underlying backend if it isn't
    /// resident.
    ///
    /// Roughly equivalent to [`std::fs::read`][std::fs::read].
    ///
    /// [std::fs::read]: https://doc.rust-lang.org/stable/std/fs/fn.read.html
    #[inline]
    pub fn read<P: AsRef<Path>>(&self, path: P) -> io::Result<Arc<Vec<u8>>> {
        let path = path.as_ref();
        self.inner.lock().unwrap().read(path)
    }

    /// Read a file from the VFS (or from the underlying backend if it isn't
    /// resident) into a string.
    ///
    /// Roughly equivalent to [`std::fs::read_to_string`][std::fs::read_to_string].
    ///
    /// [std::fs::read_to_string]: https://doc.rust-lang.org/stable/std/fs/fn.read_to_string.html
    #[inline]
    pub fn read_to_string<P: AsRef<Path>>(&self, path: P) -> io::Result<Arc<String>> {
        let path = path.as_ref();
        self.inner.lock().unwrap().read_to_string(path)
    }

    /// Read a file from the VFS (or the underlying backend if it isn't
    /// resident) into a string, and normalize its line endings to LF.
    ///
    /// Roughly equivalent to [`std::fs::read_to_string`][std::fs::read_to_string], but also performs
    /// line ending normalization.
    ///
    /// [std::fs::read_to_string]: https://doc.rust-lang.org/stable/std/fs/fn.read_to_string.html
    #[inline]
    pub fn read_to_string_lf_normalized<P: AsRef<Path>>(&self, path: P) -> io::Result<Arc<String>> {
        let path = path.as_ref();
        let contents = self.inner.lock().unwrap().read_to_string(path)?;

        Ok(contents.replace("\r\n", "\n").into())
    }

    /// Write a file to the VFS and the underlying backend.
    ///
    /// Roughly equivalent to [`std::fs::write`][std::fs::write].
    ///
    /// [std::fs::write]: https://doc.rust-lang.org/stable/std/fs/fn.write.html
    #[inline]
    pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(&self, path: P, contents: C) -> io::Result<()> {
        let path = path.as_ref();
        let contents = contents.as_ref();
        self.inner.lock().unwrap().write(path, contents)
    }

    /// Read all of the children of a directory.
    ///
    /// Roughly equivalent to [`std::fs::read_dir`][std::fs::read_dir].
    ///
    /// [std::fs::read_dir]: https://doc.rust-lang.org/stable/std/fs/fn.read_dir.html
    #[inline]
    pub fn read_dir<P: AsRef<Path>>(&self, path: P) -> io::Result<ReadDir> {
        let path = path.as_ref();
        self.inner.lock().unwrap().read_dir(path)
    }

    /// Return whether the given path exists.
    ///
    /// Roughly equivalent to [`std::fs::exists`][std::fs::exists].
    ///
    /// [std::fs::exists]: https://doc.rust-lang.org/stable/std/fs/fn.exists.html
    #[inline]
    pub fn exists<P: AsRef<Path>>(&self, path: P) -> io::Result<bool> {
        let path = path.as_ref();
        self.inner.lock().unwrap().exists(path)
    }

    /// Creates a directory at the provided location.
    ///
    /// Roughly equivalent to [`std::fs::create_dir`][std::fs::create_dir].
    /// Similiar to that function, this function will fail if the parent of the
    /// path does not exist.
    ///
    /// [std::fs::create_dir]: https://doc.rust-lang.org/stable/std/fs/fn.create_dir.html
    #[inline]
    pub fn create_dir<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.lock().unwrap().create_dir(path)
    }

    /// Creates a directory at the provided location, recursively creating
    /// all parent components if they are missing.
    ///
    /// Roughly equivalent to [`std::fs::create_dir_all`][std::fs::create_dir_all].
    ///
    /// [std::fs::create_dir_all]: https://doc.rust-lang.org/stable/std/fs/fn.create_dir_all.html
    #[inline]
    pub fn create_dir_all<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.lock().unwrap().create_dir_all(path)
    }

    /// Remove a file.
    ///
    /// Roughly equivalent to [`std::fs::remove_file`][std::fs::remove_file].
    ///
    /// [std::fs::remove_file]: https://doc.rust-lang.org/stable/std/fs/fn.remove_file.html
    #[inline]
    pub fn remove_file<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.lock().unwrap().remove_file(path)
    }

    /// Remove a directory and all of its descendants.
    ///
    /// Roughly equivalent to [`std::fs::remove_dir_all`][std::fs::remove_dir_all].
    ///
    /// [std::fs::remove_dir_all]: https://doc.rust-lang.org/stable/std/fs/fn.remove_dir_all.html
    #[inline]
    pub fn remove_dir_all<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.lock().unwrap().remove_dir_all(path)
    }

    /// Query metadata about the given path.
    ///
    /// Roughly equivalent to [`std::fs::metadata`][std::fs::metadata].
    ///
    /// [std::fs::metadata]: https://doc.rust-lang.org/stable/std/fs/fn.metadata.html
    #[inline]
    pub fn metadata<P: AsRef<Path>>(&self, path: P) -> io::Result<Metadata> {
        let path = path.as_ref();
        self.inner.lock().unwrap().metadata(path)
    }

    /// Normalize a path via the underlying backend.
    ///
    /// Roughly equivalent to [`std::fs::canonicalize`][std::fs::canonicalize]. Relative paths are
    /// resolved against the backend's current working directory (if applicable) and errors are
    /// surfaced directly from the backend.
    ///
    /// [std::fs::canonicalize]: https://doc.rust-lang.org/stable/std/fs/fn.canonicalize.html
    #[inline]
    pub fn canonicalize<P: AsRef<Path>>(&self, path: P) -> io::Result<PathBuf> {
        let path = path.as_ref();
        self.inner.lock().unwrap().canonicalize(path)
    }

    /// Retrieve a handle to the event receiver for this `Vfs`.
    #[inline]
    pub fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent> {
        self.inner.lock().unwrap().event_receiver()
    }

    /// Commit an event to this `Vfs`.
    #[inline]
    pub fn commit_event(&self, event: &VfsEvent) -> io::Result<()> {
        self.inner.lock().unwrap().commit_event(event)
    }
}

/// A locked handle to a [`Vfs`](struct.Vfs.html), created by `Vfs::lock`.
///
/// Implements roughly the same API as [`Vfs`](struct.Vfs.html).
pub struct VfsLock<'a> {
    inner: MutexGuard<'a, VfsInner>,
}

impl VfsLock<'_> {
    /// Turns automatic file watching on or off. Enabled by default.
    ///
    /// Turning off file watching may be useful for single-use cases, especially
    /// on platforms like macOS where registering file watches has significant
    /// performance cost.
    pub fn set_watch_enabled(&mut self, enabled: bool) {
        self.inner.watch_enabled = enabled;
    }

    /// Read a file from the VFS, or the underlying backend if it isn't
    /// resident.
    ///
    /// Roughly equivalent to [`std::fs::read`][std::fs::read].
    ///
    /// [std::fs::read]: https://doc.rust-lang.org/stable/std/fs/fn.read.html
    #[inline]
    pub fn read<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Arc<Vec<u8>>> {
        let path = path.as_ref();
        self.inner.read(path)
    }

    /// Write a file to the VFS and the underlying backend.
    ///
    /// Roughly equivalent to [`std::fs::write`][std::fs::write].
    ///
    /// [std::fs::write]: https://doc.rust-lang.org/stable/std/fs/fn.write.html
    #[inline]
    pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(
        &mut self,
        path: P,
        contents: C,
    ) -> io::Result<()> {
        let path = path.as_ref();
        let contents = contents.as_ref();
        self.inner.write(path, contents)
    }

    /// Read all of the children of a directory.
    ///
    /// Roughly equivalent to [`std::fs::read_dir`][std::fs::read_dir].
    ///
    /// [std::fs::read_dir]: https://doc.rust-lang.org/stable/std/fs/fn.read_dir.html
    #[inline]
    pub fn read_dir<P: AsRef<Path>>(&mut self, path: P) -> io::Result<ReadDir> {
        let path = path.as_ref();
        self.inner.read_dir(path)
    }

    /// Creates a directory at the provided location.
    ///
    /// Roughly equivalent to [`std::fs::create_dir`][std::fs::create_dir].
    /// Similiar to that function, this function will fail if the parent of the
    /// path does not exist.
    ///
    /// [std::fs::create_dir]: https://doc.rust-lang.org/stable/std/fs/fn.create_dir.html
    #[inline]
    pub fn create_dir<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.create_dir(path)
    }

    /// Creates a directory at the provided location, recursively creating
    /// all parent components if they are missing.
    ///
    /// Roughly equivalent to [`std::fs::create_dir_all`][std::fs::create_dir_all].
    ///
    /// [std::fs::create_dir_all]: https://doc.rust-lang.org/stable/std/fs/fn.create_dir_all.html
    #[inline]
    pub fn create_dir_all<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.create_dir_all(path)
    }

    /// Remove a file.
    ///
    /// Roughly equivalent to [`std::fs::remove_file`][std::fs::remove_file].
    ///
    /// [std::fs::remove_file]: https://doc.rust-lang.org/stable/std/fs/fn.remove_file.html
    #[inline]
    pub fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.remove_file(path)
    }

    /// Remove a directory and all of its descendants.
    ///
    /// Roughly equivalent to [`std::fs::remove_dir_all`][std::fs::remove_dir_all].
    ///
    /// [std::fs::remove_dir_all]: https://doc.rust-lang.org/stable/std/fs/fn.remove_dir_all.html
    #[inline]
    pub fn remove_dir_all<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        self.inner.remove_dir_all(path)
    }

    /// Query metadata about the given path.
    ///
    /// Roughly equivalent to [`std::fs::metadata`][std::fs::metadata].
    ///
    /// [std::fs::metadata]: https://doc.rust-lang.org/stable/std/fs/fn.metadata.html
    #[inline]
    pub fn metadata<P: AsRef<Path>>(&mut self, path: P) -> io::Result<Metadata> {
        let path = path.as_ref();
        self.inner.metadata(path)
    }

    /// Normalize a path via the underlying backend.
    #[inline]
    pub fn normalize<P: AsRef<Path>>(&mut self, path: P) -> io::Result<PathBuf> {
        let path = path.as_ref();
        self.inner.canonicalize(path)
    }

    /// Retrieve a handle to the event receiver for this `Vfs`.
    #[inline]
    pub fn event_receiver(&self) -> crossbeam_channel::Receiver<VfsEvent> {
        self.inner.event_receiver()
    }

    /// Commit an event to this `Vfs`.
    #[inline]
    pub fn commit_event(&mut self, event: &VfsEvent) -> io::Result<()> {
        self.inner.commit_event(event)
    }
}

#[cfg(test)]
mod test {
    use crate::{InMemoryFs, PrefetchCache, StdBackend, Vfs, VfsSnapshot};
    use std::collections::HashMap;
    use std::io;
    use std::path::PathBuf;

    /// https://github.com/rojo-rbx/rojo/issues/899
    #[test]
    fn read_to_string_lf_normalized_keeps_trailing_newline() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("test", VfsSnapshot::file("bar\r\nfoo\r\n\r\n"))
            .unwrap();

        let vfs = Vfs::new(imfs);

        assert_eq!(
            vfs.read_to_string_lf_normalized("test").unwrap().as_str(),
            "bar\nfoo\n\n"
        );
    }

    /// https://github.com/rojo-rbx/rojo/issues/1200
    #[test]
    fn canonicalize_in_memory_success() {
        let mut imfs = InMemoryFs::new();
        let contents = "Lorem ipsum dolor sit amet.".to_string();

        imfs.load_snapshot("/test/file.txt", VfsSnapshot::file(contents.to_string()))
            .unwrap();

        let vfs = Vfs::new(imfs);

        assert_eq!(
            vfs.canonicalize("/test/nested/../file.txt").unwrap(),
            PathBuf::from("/test/file.txt")
        );
        assert_eq!(
            vfs.read_to_string(vfs.canonicalize("/test/nested/../file.txt").unwrap())
                .unwrap()
                .to_string(),
            contents.to_string()
        );
    }

    #[test]
    fn canonicalize_in_memory_missing_errors() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);

        let err = vfs.canonicalize("test").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn canonicalize_std_backend_success() {
        let contents = "Lorem ipsum dolor sit amet.".to_string();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        fs_err::write(&file_path, contents.to_string()).unwrap();

        let vfs = Vfs::new(StdBackend::new_for_testing());
        let canonicalized = vfs.canonicalize(&file_path).unwrap();
        assert_eq!(canonicalized, file_path.canonicalize().unwrap());
        assert_eq!(
            vfs.read_to_string(&canonicalized).unwrap().to_string(),
            contents.to_string()
        );
    }

    #[test]
    fn canonicalize_std_backend_missing_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test");

        let vfs = Vfs::new(StdBackend::new_for_testing());
        let err = vfs.canonicalize(&file_path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    fn make_prefetch(files: Vec<(&str, &[u8])>, canonical: Vec<(&str, &str)>) -> PrefetchCache {
        PrefetchCache {
            files: files
                .into_iter()
                .map(|(k, v)| (PathBuf::from(k), v.to_vec()))
                .collect(),
            canonical: canonical
                .into_iter()
                .map(|(k, v)| (PathBuf::from(k), PathBuf::from(v)))
                .collect(),
            is_file: HashMap::new(),
            children: HashMap::new(),
        }
    }

    #[test]
    fn prefetch_cache_read_hit() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"cached")], vec![]));

        assert_eq!(vfs.read("test").unwrap().as_slice(), b"cached");
    }

    #[test]
    fn prefetch_cache_read_depletion() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("test", VfsSnapshot::file("backend"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"cached")], vec![]));

        assert_eq!(vfs.read("test").unwrap().as_slice(), b"cached");
        assert_eq!(vfs.read("test").unwrap().as_slice(), b"backend");
    }

    #[test]
    fn prefetch_cache_read_miss_falls_through() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("other", VfsSnapshot::file("backend"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"cached")], vec![]));

        assert_eq!(vfs.read("other").unwrap().as_slice(), b"backend");
    }

    #[test]
    fn prefetch_cache_read_to_string_hit() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"hello")], vec![]));

        assert_eq!(vfs.read_to_string("test").unwrap().as_str(), "hello");
    }

    #[test]
    fn prefetch_cache_lf_normalized_hit() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"line1\r\nline2\r\n")], vec![]));

        assert_eq!(
            vfs.read_to_string_lf_normalized("test").unwrap().as_str(),
            "line1\nline2\n"
        );
    }

    #[test]
    fn prefetch_cache_canonicalize_hit() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![], vec![("src/foo", "/abs/src/foo")]));

        assert_eq!(
            vfs.canonicalize("src/foo").unwrap(),
            PathBuf::from("/abs/src/foo")
        );
    }

    #[test]
    fn prefetch_cache_canonicalize_miss_falls_through() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/real/file.txt", VfsSnapshot::file("x"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![], vec![("other", "/abs/other")]));

        assert_eq!(
            vfs.canonicalize("/real/file.txt").unwrap(),
            PathBuf::from("/real/file.txt")
        );
    }

    #[test]
    fn prefetch_cache_clear_restores_backend() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("test", VfsSnapshot::file("backend"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"cached")], vec![]));
        vfs.clear_prefetch_cache();

        assert_eq!(vfs.read("test").unwrap().as_slice(), b"backend");
    }

    #[test]
    fn prefetch_cache_no_cache_same_as_none() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("test", VfsSnapshot::file("data"))
            .unwrap();
        let vfs = Vfs::new(imfs);

        assert_eq!(vfs.read("test").unwrap().as_slice(), b"data");
    }

    #[test]
    fn prefetch_cache_watch_registered_on_cache_hit() {
        let contents = "hello world".to_string();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        fs_err::write(&file_path, &contents).unwrap();

        let vfs = Vfs::new(StdBackend::new_for_testing());
        let mut cache_files = HashMap::new();
        cache_files.insert(file_path.clone(), contents.as_bytes().to_vec());
        vfs.set_prefetch_cache(PrefetchCache {
            files: cache_files,
            canonical: HashMap::new(),
            is_file: HashMap::new(),
            children: HashMap::new(),
        });

        let result = vfs.read(&file_path).unwrap();
        assert_eq!(result.as_slice(), contents.as_bytes());
    }

    #[test]
    fn prefetch_cache_read_to_string_invalid_utf8() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(
            vec![("test", &[0xFF, 0xFE, 0x00, 0x80])],
            vec![],
        ));

        let err = vfs.read_to_string("test").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn prefetch_cache_canonicalize_depletion() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/real/path", VfsSnapshot::file("x"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(
            vec![],
            vec![("/real/path", "/canonical/path")],
        ));

        assert_eq!(
            vfs.canonicalize("/real/path").unwrap(),
            PathBuf::from("/canonical/path"),
            "First call should hit cache"
        );
        assert_eq!(
            vfs.canonicalize("/real/path").unwrap(),
            PathBuf::from("/real/path"),
            "Second call should fall through to InMemoryFs backend"
        );
    }

    #[test]
    fn prefetch_cache_set_overwrite_previous() {
        let imfs = InMemoryFs::new();
        let vfs = Vfs::new(imfs);

        vfs.set_prefetch_cache(make_prefetch(vec![("a", b"first")], vec![]));
        vfs.set_prefetch_cache(make_prefetch(vec![("a", b"second")], vec![]));

        assert_eq!(vfs.read("a").unwrap().as_slice(), b"second");
    }

    #[test]
    fn prefetch_cache_concurrent_reads_from_threads() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let mut cache_files = HashMap::new();
        for i in 0..100 {
            let path = dir.path().join(format!("file_{i}.txt"));
            let content = format!("content_{i}");
            fs_err::write(&path, &content).unwrap();
            cache_files.insert(path, content.into_bytes());
        }

        let vfs = Arc::new(Vfs::new(StdBackend::new_for_testing()));
        vfs.set_prefetch_cache(PrefetchCache {
            files: cache_files,
            canonical: HashMap::new(),
            is_file: HashMap::new(),
            children: HashMap::new(),
        });

        let handles: Vec<_> = (0..100)
            .map(|i| {
                let vfs = Arc::clone(&vfs);
                let path = dir.path().join(format!("file_{i}.txt"));
                let expected = format!("content_{i}");
                std::thread::spawn(move || {
                    let data = vfs.read(&path).unwrap();
                    assert_eq!(
                        String::from_utf8(data.to_vec()).unwrap(),
                        expected,
                        "File {i} content mismatch"
                    );
                })
            })
            .collect();

        for h in handles {
            h.join().expect("Thread panicked");
        }
    }

    #[test]
    fn prefetch_cache_many_files_std_backend() {
        let dir = tempfile::tempdir().unwrap();
        let mut cache_files = HashMap::new();
        let mut canonical = HashMap::new();

        for i in 0..50 {
            let path = dir.path().join(format!("f{i}.txt"));
            let content = format!("data_{i}");
            fs_err::write(&path, &content).unwrap();
            cache_files.insert(path.clone(), content.into_bytes());
            if let Ok(c) = path.canonicalize() {
                canonical.insert(path, c);
            }
        }

        let vfs = Vfs::new(StdBackend::new_for_testing());
        vfs.set_prefetch_cache(PrefetchCache {
            files: cache_files,
            canonical,
            is_file: HashMap::new(),
            children: HashMap::new(),
        });

        for i in 0..50 {
            let path = dir.path().join(format!("f{i}.txt"));
            let data = vfs.read_to_string(&path).unwrap();
            assert_eq!(data.as_str(), &format!("data_{i}"));
        }

        for i in 0..50 {
            let path = dir.path().join(format!("f{i}.txt"));
            let data = vfs.read_to_string(&path).unwrap();
            assert_eq!(
                data.as_str(),
                &format!("data_{i}"),
                "Second read (backend) of f{i}.txt diverged"
            );
        }
    }

    #[test]
    fn prefetch_cache_read_after_write_ignores_cache() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("test", VfsSnapshot::file("original"))
            .unwrap();
        let vfs = Vfs::new(imfs);
        vfs.set_prefetch_cache(make_prefetch(vec![("test", b"cached")], vec![]));

        vfs.write("test", b"written").unwrap();
        let data = vfs.read("test").unwrap();
        assert_eq!(
            data.as_slice(),
            b"cached",
            "Cache entry should still be consumed first"
        );

        let data2 = vfs.read("test").unwrap();
        assert_eq!(
            data2.as_slice(),
            b"written",
            "After cache depleted, should see the written data"
        );
    }
}
