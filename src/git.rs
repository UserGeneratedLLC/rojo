use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::Context;
use rbx_dom_weak::types::Ref;
use sha1::{Digest, Sha1};

use crate::{
    snapshot::{is_script_class, RojoTree},
    web::interface::GitMetadata,
};

pub fn git_repo_root(project_root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let path_str = stdout.trim();
    if path_str.is_empty() {
        return None;
    }

    Some(PathBuf::from(path_str))
}

/// Returns the current HEAD commit SHA, or `None` if there are no commits
/// or the project is not in a git repo.
pub fn git_head_commit(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let hex = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex)
    } else {
        None
    }
}

struct ChangedFiles {
    tracked: HashSet<PathBuf>,
    untracked: HashSet<PathBuf>,
}

impl ChangedFiles {
    fn all(&self) -> HashSet<PathBuf> {
        self.tracked.union(&self.untracked).cloned().collect()
    }

    fn is_empty(&self) -> bool {
        self.tracked.is_empty() && self.untracked.is_empty()
    }
}

fn git_changed_files_impl(
    repo_root: &Path,
    initial_head: Option<&str>,
    project_prefixes: &[String],
) -> Option<ChangedFiles> {
    let mut tracked = HashSet::new();
    let mut untracked = HashSet::new();

    let mut status_cmd = Command::new("git");
    status_cmd
        .args(["status", "--porcelain", "--no-renames", "-uall"])
        .current_dir(repo_root);
    if !project_prefixes.is_empty() {
        status_cmd.arg("--");
        for prefix in project_prefixes {
            status_cmd.arg(prefix);
        }
    }

    let (status_output, diff_output) = if let Some(ih) = initial_head {
        let mut diff_cmd = Command::new("git");
        diff_cmd
            .args(["diff", ih, "--name-only"])
            .current_dir(repo_root);
        if !project_prefixes.is_empty() {
            diff_cmd.arg("--");
            for prefix in project_prefixes {
                diff_cmd.arg(prefix);
            }
        }

        std::thread::scope(|s| {
            let diff_handle = s.spawn(move || diff_cmd.output().ok());
            let status = status_cmd.output().ok();
            let diff = diff_handle.join().ok().flatten();
            (status, diff)
        })
    } else {
        (status_cmd.output().ok(), None)
    };

    let output = status_output?;
    if output.status.success() {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.len() < 4 {
                continue;
            }
            let xy = &line[..2];
            let path = &line[3..];
            if xy == "??" {
                untracked.insert(PathBuf::from(path));
            } else {
                tracked.insert(PathBuf::from(path));
            }
        }
    }

    if let Some(output) = diff_output {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracked.insert(PathBuf::from(trimmed));
                }
            }
        }
    }

    Some(ChangedFiles { tracked, untracked })
}

fn git_batch_check_hashes_impl(repo_root: &Path, object_refs: &[String]) -> Vec<Option<String>> {
    if object_refs.is_empty() {
        return Vec::new();
    }

    let mut child = match Command::new("git")
        .args(["cat-file", "--batch-check"])
        .current_dir(repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to spawn git cat-file --batch-check: {}", e);
            return object_refs.iter().map(|_| None).collect();
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        for r in object_refs {
            let _ = writeln!(stdin, "{}", r);
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("git cat-file --batch-check failed: {}", e);
            return object_refs.iter().map(|_| None).collect();
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .map(|line| {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] != "missing" {
                let sha = parts[0];
                if sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Some(sha.to_string());
                }
            }
            None
        })
        .collect()
}

#[cfg(test)]
fn git_show(repo_root: &Path, object_ref: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["show", object_ref])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

#[cfg(test)]
fn git_show_head(repo_root: &Path, rel_path: &Path) -> Option<String> {
    let object_ref = format!("HEAD:{}", rel_path.to_string_lossy().replace('\\', "/"));
    git_show(repo_root, &object_ref)
}

#[cfg(test)]
fn git_show_staged(repo_root: &Path, rel_path: &Path) -> Option<String> {
    let object_ref = format!(":0:{}", rel_path.to_string_lossy().replace('\\', "/"));
    git_show(repo_root, &object_ref)
}

pub fn compute_blob_sha1(content: &str) -> String {
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn compute_blob_sha1_bytes(content: &[u8]) -> String {
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// Pre-computed git index blob hashes for fast content comparison.
///
/// During syncback, we can avoid writing files whose content matches what's
/// already on disk. For files tracked in the git index, comparing the blob
/// SHA1 of the new content against the index entry's hash avoids reading
/// the file from disk entirely.
pub struct GitIndexCache {
    entries: HashMap<PathBuf, String>,
}

impl GitIndexCache {
    pub fn new(project_root: &Path) -> Option<Self> {
        let repo_root = git_repo_root(project_root)?;

        let output = Command::new("git")
            .args(["ls-files", "--stage"])
            .current_dir(&repo_root)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let mut entries = HashMap::new();
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            // Format: <mode> <sha1> <stage>\t<path>
            let (meta, path) = match line.split_once('\t') {
                Some(pair) => pair,
                None => continue,
            };
            let parts: Vec<&str> = meta.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let hash_hex = parts[1].to_string();
            let rel_path = PathBuf::from(path);

            let abs_path = repo_root.join(&rel_path);
            let project_rel = abs_path
                .strip_prefix(project_root)
                .unwrap_or(&rel_path)
                .to_path_buf();
            entries.insert(project_rel, hash_hex);
        }

        log::debug!("GitIndexCache: loaded {} index entries", entries.len());
        Some(Self { entries })
    }

    /// Returns `true` if the new content's blob SHA1 matches the git index
    /// entry for this path, meaning the file on disk almost certainly
    /// already has this content.
    pub fn file_matches_index(&self, rel_path: &Path, content: &[u8]) -> bool {
        let normalized = PathBuf::from(rel_path.to_string_lossy().replace('\\', "/"));
        let index_hash = match self
            .entries
            .get(&normalized)
            .or_else(|| self.entries.get(rel_path))
        {
            Some(h) => h,
            None => return false,
        };
        let content_hash = compute_blob_sha1_bytes(content);
        &content_hash == index_hash
    }
}

pub fn git_add(repo_root: &Path, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let output = match Command::new("git")
        .arg("add")
        .arg("--")
        .args(paths)
        .current_dir(repo_root)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::warn!("Failed to run git add: {}", e);
            return;
        }
    };

    if !output.status.success() {
        log::warn!(
            "git add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Returns true if the given file path is staged in the git index.
pub fn git_is_staged(repo_root: &Path, file: &str) -> bool {
    let output = match Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(repo_root)
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|l| l.trim() == file)
}

/// Stage all worktree files and create a commit.
pub fn git_add_all_and_commit(dir: &Path, message: &str) {
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .output();
    match add {
        Ok(o) if !o.status.success() => {
            log::warn!("git add -A failed: {}", String::from_utf8_lossy(&o.stderr));
            return;
        }
        Err(e) => {
            log::warn!("Failed to run git add: {}", e);
            return;
        }
        _ => {}
    }

    let commit = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output();
    match commit {
        Ok(o) if !o.status.success() => {
            log::warn!("git commit failed: {}", String::from_utf8_lossy(&o.stderr));
        }
        Err(e) => {
            log::warn!("Failed to run git commit: {}", e);
        }
        _ => {
            log::info!("Committed: {}", message);
        }
    }
}

/// Initialize a new git repository with line-ending config for cross-platform consistency.
pub fn git_init_repo(dir: &Path) -> anyhow::Result<()> {
    let init = Command::new("git")
        .arg("init")
        .current_dir(dir)
        .output()
        .context("Failed to run git init")?;
    if !init.status.success() {
        anyhow::bail!("git init failed: {}", String::from_utf8_lossy(&init.stderr));
    }

    let _ = Command::new("git")
        .args(["config", "--local", "core.autocrlf", "false"])
        .current_dir(dir)
        .output();
    let _ = Command::new("git")
        .args(["config", "--local", "core.eol", "lf"])
        .current_dir(dir)
        .output();
    let _ = Command::new("git")
        .args(["config", "--local", "core.safecrlf", "false"])
        .current_dir(dir)
        .output();

    Ok(())
}

/// Shallow-clone a git repository into `target_dir`, depth 1.
pub fn git_clone_shallow(url: &str, target_dir: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(target_dir)
        .output()
        .context("Failed to run git clone")?;
    if !output.status.success() {
        anyhow::bail!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Register a git submodule at `path` within the repo rooted at `repo_dir`.
///
/// If `path` already contains a valid clone, git skips the network fetch and
/// just records the submodule in `.gitmodules` and the index.
pub fn git_submodule_add(repo_dir: &Path, url: &str, path: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["submodule", "add", url, path])
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git submodule add")?;
    if !output.status.success() {
        anyhow::bail!(
            "git submodule add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Set a git config value in the repo at `repo_dir`.
pub fn git_config_set(repo_dir: &Path, key: &str, value: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["config", key, value])
        .current_dir(repo_dir)
        .output()
        .context("Failed to run git config")?;
    if !output.status.success() {
        anyhow::bail!(
            "git config failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

struct ResolvedInstance {
    id: Ref,
    class_name: String,
    rel_path: PathBuf,
}

/// Computes git metadata for the two-way sync confirmation UI.
///
/// Uses a three-phase approach to avoid holding the tree lock during I/O:
/// 1. Run git commands to get changed files scoped to project prefixes (no lock)
/// 2. Briefly lock tree to resolve file paths to instance Refs and class names
/// 3. Single `git cat-file --batch-check` to get blob hashes (no lock)
pub fn compute_git_metadata(
    tree_handle: &Arc<Mutex<RojoTree>>,
    repo_root: &Path,
    initial_head: Option<&str>,
    project_prefixes: &[String],
) -> GitMetadata {
    let total_t = Instant::now();

    let t = Instant::now();
    let changed_files = match git_changed_files_impl(repo_root, initial_head, project_prefixes) {
        Some(files) => files,
        None => {
            log::debug!(
                "[TIMING] compute_git_metadata: total {}ms (no changed files)",
                total_t.elapsed().as_millis()
            );
            return GitMetadata {
                changed_ids: Vec::new(),
                script_committed_hashes: HashMap::new(),
                new_file_ids: Vec::new(),
            };
        }
    };
    log::debug!(
        "[TIMING] compute_git_metadata: git_changed_files_impl {}ms ({} tracked, {} untracked)",
        t.elapsed().as_millis(),
        changed_files.tracked.len(),
        changed_files.untracked.len()
    );

    if changed_files.is_empty() {
        log::debug!(
            "[TIMING] compute_git_metadata: total {}ms (empty changeset)",
            total_t.elapsed().as_millis()
        );
        return GitMetadata {
            changed_ids: Vec::new(),
            script_committed_hashes: HashMap::new(),
            new_file_ids: Vec::new(),
        };
    }

    let untracked_set = &changed_files.untracked;
    let all_changed = changed_files.all();

    let t = Instant::now();
    let resolved: Vec<ResolvedInstance> = {
        let tree = tree_handle.lock().unwrap();
        let mut result = Vec::new();

        for rel_path in &all_changed {
            let abs_path = repo_root.join(rel_path);
            let canonical_path = std::fs::canonicalize(&abs_path).ok();

            let first = tree.get_ids_at_path(&abs_path);
            let ids = if !first.is_empty() {
                first
            } else if let Some(ref canon) = canonical_path {
                tree.get_ids_at_path(canon)
            } else {
                first
            };

            for &id in ids {
                if let Some(instance) = tree.get_instance(id) {
                    result.push(ResolvedInstance {
                        id,
                        class_name: instance.class_name().to_string(),
                        rel_path: rel_path.clone(),
                    });
                }
            }
        }

        result
    };
    log::debug!(
        "[TIMING] compute_git_metadata: tree resolution {}ms ({} changed -> {} resolved)",
        t.elapsed().as_millis(),
        all_changed.len(),
        resolved.len()
    );

    let changed_ids: Vec<Ref> = resolved.iter().map(|ri| ri.id).collect();
    let new_file_ids: Vec<Ref> = resolved
        .iter()
        .filter(|ri| untracked_set.contains(&ri.rel_path))
        .map(|ri| ri.id)
        .collect();
    let mut script_committed_hashes: HashMap<Ref, Vec<String>> = HashMap::new();

    #[derive(Clone, Copy)]
    enum RefKind {
        Head,
        Staged,
        InitialHead,
    }

    struct ScriptRef {
        resolved_idx: usize,
        kind: RefKind,
    }

    let mut object_refs = Vec::new();
    let mut script_refs: Vec<ScriptRef> = Vec::new();

    for (idx, ri) in resolved.iter().enumerate() {
        if !is_script_class(&ri.class_name) {
            continue;
        }
        let path_str = ri.rel_path.to_string_lossy().replace('\\', "/");

        object_refs.push(format!("HEAD:{}", path_str));
        script_refs.push(ScriptRef {
            resolved_idx: idx,
            kind: RefKind::Head,
        });

        object_refs.push(format!(":0:{}", path_str));
        script_refs.push(ScriptRef {
            resolved_idx: idx,
            kind: RefKind::Staged,
        });

        if let Some(ih) = initial_head {
            object_refs.push(format!("{}:{}", ih, path_str));
            script_refs.push(ScriptRef {
                resolved_idx: idx,
                kind: RefKind::InitialHead,
            });
        }
    }

    let t = Instant::now();
    let batch_hashes = git_batch_check_hashes_impl(repo_root, &object_refs);
    log::debug!(
        "[TIMING] compute_git_metadata: batch_check_hashes {}ms ({} refs)",
        t.elapsed().as_millis(),
        object_refs.len()
    );

    for (i, info) in script_refs.iter().enumerate() {
        let sha1 = match batch_hashes.get(i).and_then(|h| h.as_ref()) {
            Some(h) => h,
            None => continue,
        };
        let ri = &resolved[info.resolved_idx];
        let hashes = script_committed_hashes
            .entry(ri.id)
            .or_insert_with(|| Vec::with_capacity(3));

        match info.kind {
            RefKind::Head => {
                hashes.push(sha1.clone());
            }
            RefKind::Staged | RefKind::InitialHead => {
                if !hashes.contains(sha1) {
                    hashes.push(sha1.clone());
                }
            }
        }
    }

    log::debug!(
        "[TIMING] compute_git_metadata: total {}ms ({} changed_ids, {} script_hashes, {} new_file_ids)",
        total_t.elapsed().as_millis(),
        changed_ids.len(),
        script_committed_hashes.len(),
        new_file_ids.len()
    );

    GitMetadata {
        changed_ids,
        script_committed_hashes,
        new_file_ids,
    }
}

/// Refreshes the git index if the project is in a git repository.
///
/// This is useful because syncback may rewrite files with identical content,
/// which can cause git to report them as modified due to timestamp changes.
pub fn refresh_git_index(project_dir: &Path) {
    log::info!("Refreshing git index...");
    let output = match Command::new("git")
        .args(["update-index", "--refresh"])
        .current_dir(project_dir)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            log::debug!("Failed to run git update-index --refresh: {}", e);
            return;
        }
    };

    if output.status.success() {
        log::info!("Git index refreshed.");
    } else {
        log::warn!(
            "git update-index --refresh failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn git_init(dir: &Path) {
        git_init_repo(dir).expect("git init failed");
        let config_path = dir.join(".git/config");
        let mut content = fs::read_to_string(&config_path).unwrap_or_default();
        content.push_str("[user]\n\tname = Test\n\temail = test@test.com\n");
        fs::write(&config_path, content).unwrap();
    }

    fn git_commit_all(dir: &Path, msg: &str) {
        git_add_all_and_commit(dir, msg);
    }

    fn git_stage(dir: &Path, file: &str) {
        git_add(dir, &[PathBuf::from(file)]);
    }

    fn git_hash_object(dir: &Path, content: &[u8]) -> String {
        let output = Command::new("git")
            .args(["hash-object", "--stdin"])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                child.stdin.take().unwrap().write_all(content).unwrap();
                child.wait_with_output()
            })
            .expect("git hash-object failed");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    // -----------------------------------------------------------------------
    // compute_blob_sha1
    // -----------------------------------------------------------------------

    #[test]
    fn blob_sha1_empty_content() {
        let hash = compute_blob_sha1("");
        assert_eq!(hash, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn blob_sha1_hello_world() {
        let hash = compute_blob_sha1("hello world");
        assert_eq!(hash, "95d09f2b10159347eece71399a7e2e907ea3df4f");
    }

    #[test]
    fn blob_sha1_multiline_script() {
        let content = "local foo = 1\nlocal bar = 2\nreturn foo + bar\n";
        let hash = compute_blob_sha1(content);
        assert_eq!(hash.len(), 40);
        assert_eq!(hash, compute_blob_sha1(content));
    }

    #[test]
    fn blob_sha1_different_content_different_hash() {
        let h1 = compute_blob_sha1("version 1");
        let h2 = compute_blob_sha1("version 2");
        assert_ne!(h1, h2);
    }

    #[test]
    fn blob_sha1_matches_git_hash_object() {
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let content = "print('test script')\n";
        let git_hash = git_hash_object(dir.path(), content.as_bytes());
        let our_hash = compute_blob_sha1(content);
        assert_eq!(our_hash, git_hash);
    }

    // -----------------------------------------------------------------------
    // git_repo_root
    // -----------------------------------------------------------------------

    #[test]
    fn repo_root_not_a_repo() {
        let dir = tempdir().unwrap();
        assert!(git_repo_root(dir.path()).is_none());
    }

    #[test]
    fn repo_root_valid_repo() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        let root = git_repo_root(dir.path()).unwrap();
        assert!(root.exists());
    }

    #[test]
    fn repo_root_from_subdirectory() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        let sub = dir.path().join("sub").join("deep");
        fs::create_dir_all(&sub).unwrap();
        let root = git_repo_root(&sub).unwrap();
        assert_eq!(
            root.canonicalize().unwrap(),
            git_repo_root(dir.path()).unwrap().canonicalize().unwrap()
        );
    }

    // -----------------------------------------------------------------------
    // git_changed_files
    // -----------------------------------------------------------------------

    #[test]
    fn changed_files_no_changes() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("file.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");

        let changed = git_changed_files_impl(dir.path(), None, &[]);
        assert!(changed.is_some());
        assert!(changed.unwrap().is_empty());
    }

    #[test]
    fn changed_files_unstaged_modification() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("script.luau"), "modified").unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(changed.tracked.contains(&PathBuf::from("script.luau")));
        assert!(changed.untracked.is_empty());
    }

    #[test]
    fn changed_files_staged_modification() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("script.luau"), "staged version").unwrap();
        git_stage(dir.path(), "script.luau");

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(changed.tracked.contains(&PathBuf::from("script.luau")));
    }

    #[test]
    fn changed_files_untracked_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("existing.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("new_file.luau"), "new").unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(changed.untracked.contains(&PathBuf::from("new_file.luau")));
        assert!(!changed.tracked.contains(&PathBuf::from("new_file.luau")));
    }

    #[test]
    fn changed_files_deleted_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("to_delete.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::remove_file(dir.path().join("to_delete.luau")).unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(changed.tracked.contains(&PathBuf::from("to_delete.luau")));
    }

    #[test]
    fn changed_files_multiple_types() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("committed.luau"), "ok").unwrap();
        fs::write(dir.path().join("will_modify.luau"), "old").unwrap();
        git_commit_all(dir.path(), "init");

        fs::write(dir.path().join("will_modify.luau"), "new").unwrap();
        fs::write(dir.path().join("untracked.luau"), "brand new").unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        let all = changed.all();
        assert!(all.contains(&PathBuf::from("will_modify.luau")));
        assert!(all.contains(&PathBuf::from("untracked.luau")));
        assert!(!all.contains(&PathBuf::from("committed.luau")));
        assert!(changed.tracked.contains(&PathBuf::from("will_modify.luau")));
        assert!(changed.untracked.contains(&PathBuf::from("untracked.luau")));
    }

    #[test]
    fn changed_files_no_head_returns_none() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("new.luau"), "content").unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]);
        assert!(changed.is_some());
        assert!(changed
            .unwrap()
            .untracked
            .contains(&PathBuf::from("new.luau")));
    }

    #[test]
    fn changed_files_subdirectory() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/module.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("src/module.luau"), "modified").unwrap();

        let changed = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(changed.tracked.contains(&PathBuf::from("src/module.luau")));
    }

    #[test]
    fn changed_files_with_initial_head_finds_committed() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");

        let initial_head = git_head_commit(dir.path()).unwrap();

        fs::write(dir.path().join("script.luau"), "modified").unwrap();
        git_commit_all(dir.path(), "edit");

        let without = git_changed_files_impl(dir.path(), None, &[]).unwrap();
        assert!(
            !without.all().contains(&PathBuf::from("script.luau")),
            "without initial_head, committed file should NOT appear"
        );

        let with = git_changed_files_impl(dir.path(), Some(&initial_head), &[]).unwrap();
        assert!(
            with.tracked.contains(&PathBuf::from("script.luau")),
            "with initial_head, committed file should appear"
        );
    }

    // -----------------------------------------------------------------------
    // git_show_head / git_show_staged
    // -----------------------------------------------------------------------

    #[test]
    fn show_head_committed_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "committed content").unwrap();
        git_commit_all(dir.path(), "init");

        let content = git_show_head(dir.path(), Path::new("script.luau"));
        assert_eq!(content.as_deref(), Some("committed content"));
    }

    #[test]
    fn show_head_new_file_returns_none() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("dummy.luau"), "x").unwrap();
        git_commit_all(dir.path(), "init");

        let content = git_show_head(dir.path(), Path::new("nonexistent.luau"));
        assert!(content.is_none());
    }

    #[test]
    fn show_head_returns_committed_not_working_tree() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "v1").unwrap();
        git_commit_all(dir.path(), "v1");
        fs::write(dir.path().join("script.luau"), "v2 modified").unwrap();

        let content = git_show_head(dir.path(), Path::new("script.luau"));
        assert_eq!(content.as_deref(), Some("v1"));
    }

    #[test]
    fn show_staged_returns_staged_content() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "v1").unwrap();
        git_commit_all(dir.path(), "v1");
        fs::write(dir.path().join("script.luau"), "v2 staged").unwrap();
        git_stage(dir.path(), "script.luau");

        let content = git_show_staged(dir.path(), Path::new("script.luau"));
        assert_eq!(content.as_deref(), Some("v2 staged"));
    }

    #[test]
    fn show_staged_returns_none_for_unstaged() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "v1").unwrap();
        git_commit_all(dir.path(), "init");

        let staged = git_show_staged(dir.path(), Path::new("script.luau"));
        let head = git_show_head(dir.path(), Path::new("script.luau"));
        assert_eq!(staged, head);
    }

    #[test]
    fn show_staged_differs_from_head_when_staged() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "v1").unwrap();
        git_commit_all(dir.path(), "v1");
        fs::write(dir.path().join("script.luau"), "v2 staged").unwrap();
        git_stage(dir.path(), "script.luau");

        let head = git_show_head(dir.path(), Path::new("script.luau")).unwrap();
        let staged = git_show_staged(dir.path(), Path::new("script.luau")).unwrap();
        assert_ne!(head, staged);
        assert_eq!(head, "v1");
        assert_eq!(staged, "v2 staged");
    }

    #[test]
    fn show_head_subdirectory_path() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/init.luau"), "init content").unwrap();
        git_commit_all(dir.path(), "init");

        let content = git_show_head(dir.path(), Path::new("src/init.luau"));
        assert_eq!(content.as_deref(), Some("init content"));
    }

    // -----------------------------------------------------------------------
    // git_batch_check_hashes (via git cat-file --batch-check)
    // -----------------------------------------------------------------------

    #[test]
    fn batch_check_empty_refs() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        let results = git_batch_check_hashes_impl(dir.path(), &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn batch_check_single_ref() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "local x = 42\nreturn x").unwrap();
        git_commit_all(dir.path(), "init");

        let refs = vec!["HEAD:script.luau".to_string()];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 1);

        let batch_hash = results[0].as_ref().unwrap();
        let content = git_show_head(dir.path(), Path::new("script.luau")).unwrap();
        let expected_hash = compute_blob_sha1(&content);
        assert_eq!(*batch_hash, expected_hash);
    }

    #[test]
    fn batch_check_missing_ref() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("dummy.luau"), "x").unwrap();
        git_commit_all(dir.path(), "init");

        let refs = vec!["HEAD:nonexistent.luau".to_string()];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_none());
    }

    #[test]
    fn batch_check_head_and_staged_same() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("a.luau"), "content a").unwrap();
        git_commit_all(dir.path(), "init");

        let refs = vec!["HEAD:a.luau".to_string(), ":0:a.luau".to_string()];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_some());
        assert_eq!(results[0], results[1]);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &compute_blob_sha1("content a")
        );
    }

    #[test]
    fn batch_check_head_and_staged_differ() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("b.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("b.luau"), "staged version").unwrap();
        git_stage(dir.path(), "b.luau");

        let refs = vec!["HEAD:b.luau".to_string(), ":0:b.luau".to_string()];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_some());
        assert!(results[1].is_some());
        assert_ne!(results[0], results[1]);
        assert_eq!(results[0].as_ref().unwrap(), &compute_blob_sha1("original"));
        assert_eq!(
            results[1].as_ref().unwrap(),
            &compute_blob_sha1("staged version")
        );
    }

    #[test]
    fn batch_check_multiple_files_mixed() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("a.luau"), "content a").unwrap();
        fs::write(dir.path().join("b.luau"), "content b").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("b.luau"), "content b modified").unwrap();
        git_stage(dir.path(), "b.luau");

        let refs = vec![
            "HEAD:a.luau".to_string(),
            ":0:a.luau".to_string(),
            "HEAD:b.luau".to_string(),
            ":0:b.luau".to_string(),
            "HEAD:nonexistent.luau".to_string(),
        ];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 5);

        assert_eq!(results[0], results[1]); // a: HEAD == staged
        assert_ne!(results[2], results[3]); // b: HEAD != staged
        assert!(results[4].is_none()); // missing

        assert_eq!(
            results[0].as_ref().unwrap(),
            &compute_blob_sha1("content a")
        );
        assert_eq!(
            results[2].as_ref().unwrap(),
            &compute_blob_sha1("content b")
        );
        assert_eq!(
            results[3].as_ref().unwrap(),
            &compute_blob_sha1("content b modified")
        );
    }

    #[test]
    fn batch_check_subdirectory_path() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/init.luau"), "init content").unwrap();
        git_commit_all(dir.path(), "init");

        let refs = vec!["HEAD:src/init.luau".to_string()];
        let results = git_batch_check_hashes_impl(dir.path(), &refs);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &compute_blob_sha1("init content")
        );
    }

    // -----------------------------------------------------------------------
    // git_add
    // -----------------------------------------------------------------------

    #[test]
    fn git_add_stages_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("file.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("file.luau"), "modified").unwrap();

        assert!(!git_is_staged(dir.path(), "file.luau"));
        git_add(dir.path(), &[PathBuf::from("file.luau")]);
        assert!(git_is_staged(dir.path(), "file.luau"));
    }

    #[test]
    fn git_add_stages_multiple_files() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("a.luau"), "a").unwrap();
        fs::write(dir.path().join("b.luau"), "b").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("a.luau"), "a modified").unwrap();
        fs::write(dir.path().join("b.luau"), "b modified").unwrap();

        git_add(
            dir.path(),
            &[PathBuf::from("a.luau"), PathBuf::from("b.luau")],
        );
        assert!(git_is_staged(dir.path(), "a.luau"));
        assert!(git_is_staged(dir.path(), "b.luau"));
    }

    #[test]
    fn git_add_empty_paths_is_noop() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        git_add(dir.path(), &[]);
    }

    #[test]
    fn git_add_stages_new_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("existing.luau"), "x").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("new.luau"), "new content").unwrap();

        git_add(dir.path(), &[PathBuf::from("new.luau")]);
        assert!(git_is_staged(dir.path(), "new.luau"));
    }

    // -----------------------------------------------------------------------
    // is_script_class
    // -----------------------------------------------------------------------

    #[test]
    fn script_class_detection() {
        assert!(is_script_class("Script"));
        assert!(is_script_class("LocalScript"));
        assert!(is_script_class("ModuleScript"));
        assert!(!is_script_class("Folder"));
        assert!(!is_script_class("Part"));
        assert!(!is_script_class("Model"));
        assert!(!is_script_class("StringValue"));
    }

    // -----------------------------------------------------------------------
    // Hash consistency: our SHA1 matches git hash-object
    // -----------------------------------------------------------------------

    #[test]
    fn hash_consistency_empty_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let git_hash = git_hash_object(dir.path(), b"");
        assert_eq!(compute_blob_sha1(""), git_hash);
    }

    #[test]
    fn hash_consistency_unicode_content() {
        let content = "-- Unicode: 日本語テスト\nlocal x = '🎮'\n";
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let git_hash = git_hash_object(dir.path(), content.as_bytes());
        assert_eq!(compute_blob_sha1(content), git_hash);
    }

    #[test]
    fn hash_consistency_large_file() {
        let content: String = (0..10000)
            .map(|i| format!("local var_{} = {}\n", i, i))
            .collect();
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let git_hash = git_hash_object(dir.path(), content.as_bytes());
        assert_eq!(compute_blob_sha1(&content), git_hash);
    }
}
