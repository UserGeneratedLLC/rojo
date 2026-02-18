use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(windows)]
use std::{thread, time::Duration};

use memofs::Vfs;
use rayon::prelude::*;

/// Maximum number of retry attempts for filesystem operations on Windows.
/// Windows can have transient "Access denied" errors due to antivirus scanning,
/// filesystem timing, or file handle release delays.
#[cfg(windows)]
const MAX_RETRIES: u32 = 3;

/// Initial delay between retries (doubles on each retry).
#[cfg(windows)]
const INITIAL_RETRY_DELAY_MS: u64 = 10;

/// Writes to a file with retry logic for transient Windows errors.
#[cfg(windows)]
fn write_with_retry(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut last_error = None;
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;

    for attempt in 0..=MAX_RETRIES {
        match std::fs::write(path, contents) {
            Ok(()) => return Ok(()),
            Err(err) => {
                // Only retry on "Access denied" (os error 5) or "Sharing violation" (os error 32)
                let should_retry = err
                    .raw_os_error()
                    .is_some_and(|code| code == 5 || code == 32);

                if should_retry && attempt < MAX_RETRIES {
                    log::trace!(
                        "Retrying write to {} after error (attempt {}): {}",
                        path.display(),
                        attempt + 1,
                        err
                    );
                    thread::sleep(Duration::from_millis(delay_ms));
                    delay_ms *= 2; // Exponential backoff
                    last_error = Some(err);
                } else {
                    return Err(err);
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// On non-Windows platforms, just write directly without retry logic.
#[cfg(not(windows))]
fn write_with_retry(path: &Path, contents: &[u8]) -> io::Result<()> {
    std::fs::write(path, contents)
}

/// Removes a file with retry logic for transient Windows errors.
#[cfg(windows)]
fn remove_file_with_retry(path: &Path) -> io::Result<()> {
    let mut last_error = None;
    let mut delay_ms = INITIAL_RETRY_DELAY_MS;

    for attempt in 0..=MAX_RETRIES {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                // Only retry on "Access denied" (os error 5) or "Sharing violation" (os error 32)
                let should_retry = err
                    .raw_os_error()
                    .is_some_and(|code| code == 5 || code == 32);

                if should_retry && attempt < MAX_RETRIES {
                    log::trace!(
                        "Retrying remove of {} after error (attempt {}): {}",
                        path.display(),
                        attempt + 1,
                        err
                    );
                    thread::sleep(Duration::from_millis(delay_ms));
                    delay_ms *= 2;
                    last_error = Some(err);
                } else {
                    return Err(err);
                }
            }
        }
    }

    Err(last_error.unwrap())
}

/// On non-Windows platforms, just remove directly without retry logic.
#[cfg(not(windows))]
fn remove_file_with_retry(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// A simple representation of a subsection of a file system.
#[derive(Default)]
pub struct FsSnapshot {
    /// Paths representing new files mapped to their contents.
    added_files: HashMap<PathBuf, Vec<u8>>,
    /// Paths representing new directories.
    added_dirs: HashSet<PathBuf>,
    /// Paths representing removed files.
    removed_files: HashSet<PathBuf>,
    /// Paths representing removed directories.
    removed_dirs: HashSet<PathBuf>,
}

impl FsSnapshot {
    /// Creates a new `FsSnapshot`.
    pub fn new() -> Self {
        Self {
            added_files: HashMap::new(),
            added_dirs: HashSet::new(),
            removed_files: HashSet::new(),
            removed_dirs: HashSet::new(),
        }
    }

    /// Adds the given path to the `FsSnapshot` as a file with the given
    /// contents, then returns it.
    pub fn with_added_file<P: AsRef<Path>>(mut self, path: P, data: Vec<u8>) -> Self {
        self.added_files.insert(path.as_ref().to_path_buf(), data);
        self
    }

    /// Adds the given path to the `FsSnapshot` as a file with the given
    /// then returns it.
    pub fn with_added_dir<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.added_dirs.insert(path.as_ref().to_path_buf());
        self
    }

    /// Merges two `FsSnapshot`s together.
    #[inline]
    pub fn merge(&mut self, other: Self) {
        self.added_files.extend(other.added_files);
        self.added_dirs.extend(other.added_dirs);
        self.removed_files.extend(other.removed_files);
        self.removed_dirs.extend(other.removed_dirs);
    }

    /// Merges two `FsSnapshot`s together, with a filter applied to the paths.
    #[inline]
    pub fn merge_with_filter<F>(&mut self, other: Self, mut predicate: F)
    where
        F: FnMut(&Path) -> bool,
    {
        self.added_files
            .extend(other.added_files.into_iter().filter(|(k, _)| predicate(k)));
        self.added_dirs
            .extend(other.added_dirs.into_iter().filter(|p| predicate(p)));
        self.removed_files
            .extend(other.removed_files.into_iter().filter(|p| predicate(p)));
        self.removed_dirs
            .extend(other.removed_dirs.into_iter().filter(|p| predicate(p)));
    }

    /// Adds the provided path as a file with the given contents.
    pub fn add_file<P: AsRef<Path>>(&mut self, path: P, data: Vec<u8>) {
        self.added_files.insert(path.as_ref().to_path_buf(), data);
    }

    /// Adds the provided path as a directory.
    pub fn add_dir<P: AsRef<Path>>(&mut self, path: P) {
        self.added_dirs.insert(path.as_ref().to_path_buf());
    }

    /// Removes the provided path, as a file.
    pub fn remove_file<P: AsRef<Path>>(&mut self, path: P) {
        self.removed_files.insert(path.as_ref().to_path_buf());
    }

    /// Removes the provided path, as a directory.
    pub fn remove_dir<P: AsRef<Path>>(&mut self, path: P) {
        self.removed_dirs.insert(path.as_ref().to_path_buf());
    }

    /// Writes the `FsSnapshot` to the provided VFS, using the provided `base`
    /// as a root for the other paths in the `FsSnapshot`.
    ///
    /// This includes removals, but makes no effort to minimize work done.
    pub fn write_to_vfs<P: AsRef<Path>>(&self, base: P, vfs: &Vfs) -> io::Result<()> {
        let mut lock = vfs.lock();

        let base_path = base.as_ref();
        for dir_path in &self.added_dirs {
            match lock.create_dir_all(base_path.join(dir_path)) {
                Ok(_) => (),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
                Err(err) => return Err(err),
            };
        }
        for (path, contents) in &self.added_files {
            lock.write(base_path.join(path), contents)?;
        }
        for dir_path in &self.removed_dirs {
            let full_path = base_path.join(dir_path);
            match lock.remove_dir_all(&full_path) {
                Ok(()) => (),
                // Directory might have already been removed (e.g., added twice via different code paths)
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    log::debug!(
                        "Directory already removed or doesn't exist: {}",
                        full_path.display()
                    );
                }
                Err(err) => return Err(err),
            }
        }
        // Only remove files that aren't already inside a directory we're removing.
        // remove_dir_all already deleted those files recursively.
        // Also handle the case where the same file might be listed twice (absolute vs relative path)
        // by gracefully handling "file not found" errors.
        for path in &self.removed_files {
            let is_inside_removed_dir = self.removed_dirs.iter().any(|dir| path.starts_with(dir));
            if is_inside_removed_dir {
                continue;
            }
            let full_path = base_path.join(path);
            match lock.remove_file(&full_path) {
                Ok(()) => (),
                // File might have already been removed (e.g., by remove_dir_all, or listed twice
                // with different path formats like absolute vs relative)
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    log::debug!(
                        "File already removed or doesn't exist: {}",
                        full_path.display()
                    );
                }
                Err(err) => return Err(err),
            }
        }
        drop(lock);

        log::debug!(
            "Wrote {} directories and {} files to the file system",
            self.added_dirs.len(),
            self.added_files.len()
        );
        // Count how many files were skipped because they're inside removed directories
        let files_inside_dirs = self
            .removed_files
            .iter()
            .filter(|path| self.removed_dirs.iter().any(|dir| path.starts_with(dir)))
            .count();
        log::debug!(
            "Removed {} directories and {} files from the file system ({} files skipped, inside removed dirs)",
            self.removed_dirs.len(),
            self.removed_files.len() - files_inside_dirs,
            files_inside_dirs
        );
        Ok(())
    }

    /// Writes the `FsSnapshot` to the provided VFS using parallel file writes.
    ///
    /// This is optimized for syncback operations where many files need to be written.
    /// Directory creation and removal remain sequential (ordering constraints),
    /// but file writes and file removals are parallelized using rayon.
    ///
    /// This bypasses the VFS lock for file writes, using `std::fs` directly.
    /// This is safe because syncback uses a oneshot VFS with no caching or watching.
    pub fn write_to_vfs_parallel<P: AsRef<Path>>(&self, base: P, vfs: &Vfs) -> io::Result<()> {
        let base_path = base.as_ref();

        // Phase 1: Create directories (sequential - parent must exist before child)
        {
            let mut lock = vfs.lock();
            for dir_path in &self.added_dirs {
                match lock.create_dir_all(base_path.join(dir_path)) {
                    Ok(_) => (),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(err) => return Err(err),
                };
            }
        } // Release lock before parallel phase

        // Phase 2: Write files (parallel - independent operations)
        // On Windows, use retry logic for transient "Access denied" errors that can
        // occur due to antivirus scanning or filesystem timing issues.
        let write_errors = AtomicUsize::new(0);
        let first_error: std::sync::Mutex<Option<io::Error>> = std::sync::Mutex::new(None);

        self.added_files.par_iter().for_each(|(path, contents)| {
            let full_path = base_path.join(path);
            if let Err(err) = write_with_retry(&full_path, contents) {
                write_errors.fetch_add(1, Ordering::Relaxed);
                let mut guard = first_error.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(err);
                }
            }
        });

        // Check for write errors
        if let Some(err) = first_error.into_inner().unwrap() {
            let error_count = write_errors.load(Ordering::Relaxed);
            if error_count > 1 {
                log::warn!("{} additional file write errors occurred", error_count - 1);
            }
            return Err(err);
        }

        // Phase 3: Remove files not inside removed directories (parallel)
        // Uses retry logic on Windows for transient errors.
        let files_to_remove: Vec<_> = self
            .removed_files
            .iter()
            .filter(|path| !self.removed_dirs.iter().any(|dir| path.starts_with(dir)))
            .collect();

        let remove_errors = AtomicUsize::new(0);
        let first_remove_error: std::sync::Mutex<Option<io::Error>> = std::sync::Mutex::new(None);

        files_to_remove.par_iter().for_each(|path| {
            let full_path = base_path.join(path);
            if let Err(err) = remove_file_with_retry(&full_path) {
                remove_errors.fetch_add(1, Ordering::Relaxed);
                let mut guard = first_remove_error.lock().unwrap();
                if guard.is_none() {
                    *guard = Some(err);
                }
            }
        });

        // Check for remove errors
        if let Some(err) = first_remove_error.into_inner().unwrap() {
            let error_count = remove_errors.load(Ordering::Relaxed);
            if error_count > 1 {
                log::warn!(
                    "{} additional file removal errors occurred",
                    error_count - 1
                );
            }
            return Err(err);
        }

        // Phase 4: Remove directories (sequential - uses VFS for proper unwatch handling)
        {
            let mut lock = vfs.lock();
            for dir_path in &self.removed_dirs {
                let full_path = base_path.join(dir_path);
                match lock.remove_dir_all(&full_path) {
                    Ok(()) => (),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {
                        log::debug!(
                            "Directory already removed or doesn't exist: {}",
                            full_path.display()
                        );
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        log::debug!(
            "Wrote {} directories and {} files to the file system (parallel)",
            self.added_dirs.len(),
            self.added_files.len()
        );

        let files_inside_dirs = self.removed_files.len() - files_to_remove.len();
        log::debug!(
            "Removed {} directories and {} files from the file system ({} files skipped, inside removed dirs)",
            self.removed_dirs.len(),
            files_to_remove.len(),
            files_inside_dirs
        );

        Ok(())
    }

    /// Returns whether this `FsSnapshot` is empty or not.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.added_files.is_empty()
            && self.added_dirs.is_empty()
            && self.removed_files.is_empty()
            && self.removed_dirs.is_empty()
    }

    /// Returns a list of paths that would be added by this `FsSnapshot`.
    #[inline]
    pub fn added_paths(&self) -> Vec<&Path> {
        let mut list = Vec::with_capacity(self.added_files.len() + self.added_dirs.len());
        list.extend(self.added_files());
        list.extend(self.added_dirs());

        list
    }

    /// Returns a list of paths that would be removed by this `FsSnapshot`.
    #[inline]
    pub fn removed_paths(&self) -> Vec<&Path> {
        let mut list = Vec::with_capacity(self.removed_files.len() + self.removed_dirs.len());
        list.extend(self.removed_files());
        list.extend(self.removed_dirs());

        list
    }

    /// Returns a list of file paths that would be added by this `FsSnapshot`
    #[inline]
    pub fn added_files(&self) -> Vec<&Path> {
        let mut added_files: Vec<_> = self.added_files.keys().map(PathBuf::as_path).collect();
        added_files.sort_unstable();
        added_files
    }

    /// Returns a list of directory paths that would be added by this `FsSnapshot`
    #[inline]
    pub fn added_dirs(&self) -> Vec<&Path> {
        let mut added_dirs: Vec<_> = self.added_dirs.iter().map(PathBuf::as_path).collect();
        added_dirs.sort_unstable();
        added_dirs
    }

    /// Returns a list of file paths that would be removed by this `FsSnapshot`
    #[inline]
    pub fn removed_files(&self) -> Vec<&Path> {
        let mut removed_files: Vec<_> = self.removed_files.iter().map(PathBuf::as_path).collect();
        removed_files.sort_unstable();
        removed_files
    }

    /// Returns a list of directory paths that would be removed by this `FsSnapshot`
    #[inline]
    pub fn removed_dirs(&self) -> Vec<&Path> {
        let mut removed_dirs: Vec<_> = self.removed_dirs.iter().map(PathBuf::as_path).collect();
        removed_dirs.sort_unstable();
        removed_dirs
    }

    /// Post-process `Rojo_Ref_*` attribute paths in meta/model JSON files.
    ///
    /// For each entry in `substitutions` (old_path → new_path), finds all
    /// `.meta.json5` and `.model.json5` files in the snapshot and replaces
    /// occurrences of the old path with the new path **only on lines that
    /// contain a `Rojo_Ref_` key**. This prevents unrelated string values
    /// (property values, comments, etc.) from being altered by the
    /// substitution.
    pub fn fix_ref_paths(&mut self, substitutions: &[(String, String)]) {
        if substitutions.is_empty() {
            return;
        }

        let ref_paths: Vec<PathBuf> = self
            .added_files
            .keys()
            .filter(|p| {
                let name = p.file_name().and_then(|f| f.to_str()).unwrap_or("");
                name.ends_with(".meta.json5")
                    || name.ends_with(".meta.json")
                    || name.ends_with(".model.json5")
                    || name.ends_with(".model.json")
            })
            .cloned()
            .collect();

        for path in ref_paths {
            let Some(contents) = self.added_files.get_mut(&path) else {
                continue;
            };
            let Ok(text) = std::str::from_utf8(contents) else {
                continue;
            };
            if !text.contains("Rojo_Ref_") {
                continue;
            }

            // Attribute-scoped replacement: only apply substitutions on
            // lines that contain a Rojo_Ref_ key. This prevents accidental
            // mutation of unrelated string values in the same file.
            let mut modified_lines: Vec<String> = Vec::new();
            let mut any_changed = false;
            for line in text.split('\n') {
                if line.contains("Rojo_Ref_") {
                    let mut new_line = line.to_string();
                    for (old_path, new_path) in substitutions {
                        if old_path == new_path {
                            continue;
                        }
                        new_line = new_line.replace(old_path.as_str(), new_path.as_str());
                    }
                    if new_line != line {
                        any_changed = true;
                    }
                    modified_lines.push(new_line);
                } else {
                    modified_lines.push(line.to_string());
                }
            }

            if any_changed {
                *contents = modified_lines.join("\n").into_bytes();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_ref_paths_only_touches_ref_lines() {
        let mut snap = FsSnapshot::new();

        let meta_content = r#"{
  "properties": {
    "Description": "Path is Workspace/Foo"
  },
  "attributes": {
    "Rojo_Ref_PrimaryPart": "Workspace/Foo"
  }
}"#;
        snap.added_files.insert(
            PathBuf::from("/test/init.meta.json5"),
            meta_content.as_bytes().to_vec(),
        );

        let substitutions = vec![("Workspace/Foo".to_string(), "Workspace/Foo~2".to_string())];
        snap.fix_ref_paths(&substitutions);

        let result = std::str::from_utf8(
            snap.added_files
                .get(Path::new("/test/init.meta.json5"))
                .unwrap(),
        )
        .unwrap();

        assert!(
            result.contains(r#""Rojo_Ref_PrimaryPart": "Workspace/Foo~2""#),
            "Rojo_Ref line should be updated. Got:\n{}",
            result
        );
        assert!(
            result.contains(r#""Description": "Path is Workspace/Foo""#),
            "Non-ref property line should NOT be changed. Got:\n{}",
            result
        );
    }

    #[test]
    fn fix_ref_paths_ignores_non_meta_files() {
        let mut snap = FsSnapshot::new();

        snap.added_files.insert(
            PathBuf::from("/test/script.luau"),
            b"-- Rojo_Ref_PrimaryPart Workspace/Foo".to_vec(),
        );

        let substitutions = vec![("Workspace/Foo".to_string(), "Workspace/Foo~2".to_string())];
        snap.fix_ref_paths(&substitutions);

        let result = std::str::from_utf8(
            snap.added_files
                .get(Path::new("/test/script.luau"))
                .unwrap(),
        )
        .unwrap();
        assert!(
            result.contains("Workspace/Foo"),
            "Non-meta file should not be touched. Got: {}",
            result
        );
        assert!(
            !result.contains("Workspace/Foo~2"),
            "Non-meta file should not have substitution applied"
        );
    }

    /// Regression test: the old approach used tentative paths (e.g.,
    /// "Beams1/001") as substitution keys. When multiple duplicate-named
    /// instances shared the same tentative path, sequential `str::replace`
    /// calls would chain — each substitution's result contained the next
    /// substitution's search key as a prefix, producing corrupted paths
    /// like "001~5~7~4~6~3~8~2".
    ///
    /// The fix uses unique per-Ref placeholders as keys, making each
    /// substitution independent. This test verifies the old failure mode
    /// cannot recur.
    #[test]
    fn fix_ref_paths_no_chaining_with_unique_placeholders() {
        let mut snap = FsSnapshot::new();

        // Simulate a Beam with Attachment0 → placeholder for one specific
        // Attachment, and Attachment1 → placeholder for another.
        let meta_content = r#"{
  "attributes": {
    "Rojo_Ref_Attachment0": "__ROJO_REF_00000000000000000000000000000001__",
    "Rojo_Ref_Attachment1": "__ROJO_REF_00000000000000000000000000000002__"
  },
  "className": "Beam"
}"#;
        snap.added_files.insert(
            PathBuf::from("/test/BeamA.model.json5"),
            meta_content.as_bytes().to_vec(),
        );

        // Each placeholder maps to a distinct dedup'd path.
        let substitutions = vec![
            (
                "__ROJO_REF_00000000000000000000000000000001__".to_string(),
                "Beams1/001~5".to_string(),
            ),
            (
                "__ROJO_REF_00000000000000000000000000000002__".to_string(),
                "Beams1/002~7.model.json5".to_string(),
            ),
        ];
        snap.fix_ref_paths(&substitutions);

        let result = std::str::from_utf8(
            snap.added_files
                .get(Path::new("/test/BeamA.model.json5"))
                .unwrap(),
        )
        .unwrap();

        assert!(
            result.contains(r#""Rojo_Ref_Attachment0": "Beams1/001~5""#),
            "Attachment0 should resolve to 001~5. Got:\n{result}"
        );
        assert!(
            result.contains(r#""Rojo_Ref_Attachment1": "Beams1/002~7.model.json5""#),
            "Attachment1 should resolve to 002~7. Got:\n{result}"
        );
        assert!(
            !result.contains("__ROJO_REF_"),
            "no placeholders should remain. Got:\n{result}"
        );
    }

    /// Regression test: verifies that the OLD tentative-path approach
    /// would chain substitutions, producing corrupted paths. This test
    /// documents the exact failure mode to prevent future regressions.
    #[test]
    fn old_tentative_substitution_would_chain() {
        let mut snap = FsSnapshot::new();

        let meta_content = r#"{
  "attributes": {
    "Rojo_Ref_Attachment0": "Beams1/001"
  }
}"#;
        snap.added_files.insert(
            PathBuf::from("/test/BeamA.model.json5"),
            meta_content.as_bytes().to_vec(),
        );

        // Simulate the old broken substitution list: same source, different targets.
        // This is what happened when all 8 duplicate "001" Attachments shared
        // the tentative path "Beams1/001".
        let substitutions = vec![
            ("Beams1/001".to_string(), "Beams1/001~5".to_string()),
            ("Beams1/001".to_string(), "Beams1/001~3".to_string()),
            ("Beams1/001".to_string(), "Beams1/001~2".to_string()),
        ];
        snap.fix_ref_paths(&substitutions);

        let result = std::str::from_utf8(
            snap.added_files
                .get(Path::new("/test/BeamA.model.json5"))
                .unwrap(),
        )
        .unwrap();

        // The old approach chains: "001" → "001~5" → "001~3~5" → "001~2~3~5"
        // This is the documented bug — the result is corrupted.
        assert!(
            result.contains("~5") && result.contains("~3") && result.contains("~2"),
            "old approach should chain suffixes (documenting the bug). Got:\n{result}"
        );
        assert!(
            !result.contains(r#""Rojo_Ref_Attachment0": "Beams1/001~5""#),
            "old approach should NOT produce the correct single-suffix result"
        );
    }

    /// Tests that multiple Rojo_Ref_* attributes on the same line or in
    /// the same file each resolve to their correct target independently.
    #[test]
    fn fix_ref_paths_multiple_refs_same_file() {
        let mut snap = FsSnapshot::new();

        let meta_content = r#"{
  "attributes": {
    "Rojo_Ref_Attachment0": "__ROJO_REF_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa__",
    "Rojo_Ref_Attachment1": "__ROJO_REF_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb__",
    "Rojo_Ref_PrimaryPart": "__ROJO_REF_cccccccccccccccccccccccccccccccc__"
  }
}"#;
        snap.added_files.insert(
            PathBuf::from("/test/init.meta.json5"),
            meta_content.as_bytes().to_vec(),
        );

        let substitutions = vec![
            (
                "__ROJO_REF_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa__".to_string(),
                "Beams1/001~2".to_string(),
            ),
            (
                "__ROJO_REF_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb__".to_string(),
                "Beams1/001~5".to_string(),
            ),
            (
                "__ROJO_REF_cccccccccccccccccccccccccccccccc__".to_string(),
                "Workspace/Model/Handle.model.json5".to_string(),
            ),
        ];
        snap.fix_ref_paths(&substitutions);

        let result = std::str::from_utf8(
            snap.added_files
                .get(Path::new("/test/init.meta.json5"))
                .unwrap(),
        )
        .unwrap();

        assert!(
            result.contains(r#""Rojo_Ref_Attachment0": "Beams1/001~2""#),
            "Attachment0 wrong. Got:\n{result}"
        );
        assert!(
            result.contains(r#""Rojo_Ref_Attachment1": "Beams1/001~5""#),
            "Attachment1 wrong. Got:\n{result}"
        );
        assert!(
            result.contains(
                r#""Rojo_Ref_PrimaryPart": "Workspace/Model/Handle.model.json5""#
            ),
            "PrimaryPart wrong. Got:\n{result}"
        );
    }
}
