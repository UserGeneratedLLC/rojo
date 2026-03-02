use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use anyhow::Context;
use gix::bstr::{BString, ByteSlice};
use rbx_dom_weak::types::Ref;
use sha1::{Digest, Sha1};

use crate::{
    snapshot::{is_script_class, RojoTree},
    web::interface::GitMetadata,
};

fn open_repo(path: &Path) -> Option<gix::Repository> {
    gix::discover(path).ok()
}

pub fn git_repo_root(project_root: &Path) -> Option<PathBuf> {
    let repo = open_repo(project_root)?;
    repo.workdir().map(|p| p.to_owned())
}

/// Returns the current HEAD commit SHA, or `None` if there are no commits
/// or the project is not in a git repo.
pub fn git_head_commit(repo_root: &Path) -> Option<String> {
    let repo = open_repo(repo_root)?;
    let head_id = repo.head_id().ok()?;
    let hex = head_id.to_hex().to_string();
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

fn resolve_tree_id(repo: &gix::Repository, rev: &str) -> Option<gix::ObjectId> {
    let id = repo.rev_parse_single(rev.as_bytes()).ok()?;
    let tree = id.object().ok()?.peel_to_tree().ok()?;
    Some(tree.id().detach())
}

fn git_changed_files_impl(
    repo: &gix::Repository,
    initial_head: Option<&str>,
    project_prefixes: &[String],
) -> Option<ChangedFiles> {
    let mut tracked = HashSet::new();
    let mut untracked = HashSet::new();

    let patterns: Vec<BString> = project_prefixes
        .iter()
        .map(|p| BString::from(p.as_str()))
        .collect();

    let t = Instant::now();
    let mut platform = match repo.status(gix::progress::Discard) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Failed to initialize git status: {}", e);
            return None;
        }
    };

    if let Some(ih) = initial_head {
        if let Some(tree_id) = resolve_tree_id(repo, ih) {
            platform = platform.head_tree(tree_id);
        }
    }

    let iter = match platform.into_iter(patterns) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("Failed to create git status iterator: {}", e);
            return None;
        }
    };
    log::debug!(
        "[TIMING] compute_git_metadata: status iterator created {}ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    for item in iter {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        match &item {
            gix::status::Item::TreeIndex(_) => {
                tracked.insert(PathBuf::from(item.location().to_str_lossy().as_ref()));
            }
            gix::status::Item::IndexWorktree(iw_item) => match iw_item {
                gix::status::index_worktree::Item::DirectoryContents { .. } => {
                    untracked.insert(PathBuf::from(item.location().to_str_lossy().as_ref()));
                }
                _ => {
                    tracked.insert(PathBuf::from(item.location().to_str_lossy().as_ref()));
                }
            },
        }
    }
    log::debug!(
        "[TIMING] compute_git_metadata: status iteration {}ms ({} tracked, {} untracked)",
        t.elapsed().as_millis(),
        tracked.len(),
        untracked.len()
    );

    Some(ChangedFiles { tracked, untracked })
}

fn resolve_object_ref(repo: &gix::Repository, object_ref: &str) -> Option<String> {
    if let Some(path) = object_ref.strip_prefix(":0:") {
        let index = repo.open_index().ok()?;
        let bstr_path: &gix::bstr::BStr = path.as_bytes().as_bstr();
        let entry = index.entry_by_path(bstr_path)?;
        Some(entry.id.to_hex().to_string())
    } else if let Some((rev, path)) = object_ref.split_once(':') {
        let id = repo.rev_parse_single(rev.as_bytes()).ok()?;
        let tree = id.object().ok()?.peel_to_tree().ok()?;
        let entry = tree.lookup_entry_by_path(path).ok()??;
        Some(entry.object_id().to_hex().to_string())
    } else {
        None
    }
}

fn git_batch_check_hashes_impl(
    repo: &gix::Repository,
    object_refs: &[String],
) -> Vec<Option<String>> {
    object_refs
        .iter()
        .map(|r| resolve_object_ref(repo, r))
        .collect()
}

#[cfg(test)]
fn git_show(repo_root: &Path, object_ref: &str) -> Option<String> {
    let repo = open_repo(repo_root)?;
    if let Some(path) = object_ref.strip_prefix(":0:") {
        let index = repo.open_index().ok()?;
        let bstr_path: &gix::bstr::BStr = path.as_bytes().as_bstr();
        let entry = index.entry_by_path(bstr_path)?;
        let obj = repo.find_object(entry.id).ok()?;
        String::from_utf8(obj.data.to_vec()).ok()
    } else if object_ref.contains(':') {
        let (rev, path) = object_ref.split_once(':')?;
        let id = repo.rev_parse_single(rev.as_bytes()).ok()?;
        let tree = id.object().ok()?.peel_to_tree().ok()?;
        let entry = tree.lookup_entry_by_path(path).ok()??;
        let obj = entry.id().object().ok()?;
        String::from_utf8(obj.data.to_vec()).ok()
    } else {
        let id = repo.rev_parse_single(object_ref.as_bytes()).ok()?;
        let obj = id.object().ok()?;
        String::from_utf8(obj.data.to_vec()).ok()
    }
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
        let repo = open_repo(project_root)?;
        let repo_root = repo.workdir()?.to_owned();
        let index = repo.open_index().ok()?;

        let mut entries = HashMap::new();
        for entry in index.entries() {
            let path_bstr = entry.path(&index);
            let path_str = path_bstr.to_str().ok()?;
            let rel_path = PathBuf::from(path_str);
            let hash_hex = entry.id.to_hex().to_string();

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

    let output = match std::process::Command::new("git")
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
    let output = match std::process::Command::new("git")
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
    let add = std::process::Command::new("git")
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

    let commit = std::process::Command::new("git")
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
    let init = std::process::Command::new("git")
        .arg("init")
        .current_dir(dir)
        .output()
        .context("Failed to run git init")?;
    if !init.status.success() {
        anyhow::bail!("git init failed: {}", String::from_utf8_lossy(&init.stderr));
    }

    let config = std::process::Command::new("git")
        .args(["config", "--local", "core.autocrlf", "false"])
        .current_dir(dir)
        .output();
    if let Ok(o) = &config {
        if !o.status.success() {
            log::warn!("Failed to set core.autocrlf");
        }
    }
    let _ = std::process::Command::new("git")
        .args(["config", "--local", "core.eol", "lf"])
        .current_dir(dir)
        .output();
    let _ = std::process::Command::new("git")
        .args(["config", "--local", "core.safecrlf", "false"])
        .current_dir(dir)
        .output();

    Ok(())
}

/// Shallow-clone a git repository into `target_dir`, depth 1.
pub fn git_clone_shallow(url: &str, target_dir: &Path) -> anyhow::Result<()> {
    let output = std::process::Command::new("git")
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

struct ResolvedInstance {
    id: Ref,
    class_name: String,
    rel_path: PathBuf,
}

/// Computes git metadata for the two-way sync confirmation UI.
///
/// `repo_root` must be the git repository root (from `git_repo_root()`),
/// cached at session start by `ServeSession`.
///
/// Uses a three-phase approach to avoid holding the tree lock during I/O:
/// 1. Use gix to get changed files scoped to project prefixes (no lock)
/// 2. Briefly lock tree to resolve file paths to instance Refs and class names
/// 3. Direct object/index lookups to get blob hashes (no lock)
pub fn compute_git_metadata(
    tree_handle: &Arc<Mutex<RojoTree>>,
    repo_root: &Path,
    initial_head: Option<&str>,
    project_prefixes: &[String],
) -> GitMetadata {
    let total_t = Instant::now();

    let t = Instant::now();
    let repo = match open_repo(repo_root) {
        Some(r) => r,
        None => {
            return GitMetadata {
                changed_ids: Vec::new(),
                script_committed_hashes: HashMap::new(),
                new_file_ids: Vec::new(),
            };
        }
    };
    log::debug!(
        "[TIMING] compute_git_metadata: open_repo {}ms",
        t.elapsed().as_millis()
    );

    log::debug!(
        "[TIMING] compute_git_metadata: using {} project prefixes",
        project_prefixes.len()
    );

    let t = Instant::now();
    let changed_files = match git_changed_files_impl(&repo, initial_head, project_prefixes) {
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
    let batch_hashes = git_batch_check_hashes_impl(&repo, &object_refs);
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
    let output = match std::process::Command::new("git")
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
        let repo = open_repo(dir).expect("repo not found");
        git_add(repo.workdir().unwrap_or(dir), &[PathBuf::from(file)]);
    }

    fn git_is_staged(dir: &Path, file: &str) -> bool {
        let repo = open_repo(dir).expect("repo not found");
        let index = match repo.open_index() {
            Ok(i) => i,
            Err(_) => return false,
        };
        let bstr_path: &gix::bstr::BStr = file.as_bytes().as_bstr();
        let index_entry = match index.entry_by_path(bstr_path) {
            Some(e) => e,
            None => return false,
        };

        let head_blob_id = repo
            .head_commit()
            .ok()
            .and_then(|c| c.tree().ok())
            .and_then(|t| t.lookup_entry_by_path(file).ok().flatten())
            .map(|e| e.object_id());

        match head_blob_id {
            Some(head_id) => index_entry.id != head_id,
            None => true,
        }
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

    fn git_hash_object(dir: &Path, content: &[u8]) -> String {
        let output = std::process::Command::new("git")
            .args(["hash-object", "--stdin"])
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(content).unwrap();
                child.wait_with_output()
            })
            .expect("git hash-object failed");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]);
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
        assert!(changed.tracked.contains(&PathBuf::from("script.luau")));
    }

    #[test]
    fn changed_files_untracked_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("existing.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("new_file.luau"), "new").unwrap();

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]);
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

        let repo = open_repo(dir.path()).unwrap();
        let changed = git_changed_files_impl(&repo, None, &[]).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();

        let without = git_changed_files_impl(&repo, None, &[]).unwrap();
        assert!(
            !without.all().contains(&PathBuf::from("script.luau")),
            "without initial_head, committed file should NOT appear"
        );

        let with = git_changed_files_impl(&repo, Some(&initial_head), &[]).unwrap();
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
    // git_batch_check_hashes (via resolve_object_ref)
    // -----------------------------------------------------------------------

    #[test]
    fn batch_check_empty_refs() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        let repo = open_repo(dir.path()).unwrap();
        let results = git_batch_check_hashes_impl(&repo, &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn batch_check_single_ref() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "local x = 42\nreturn x").unwrap();
        git_commit_all(dir.path(), "init");

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec!["HEAD:script.luau".to_string()];
        let results = git_batch_check_hashes_impl(&repo, &refs);
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

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec!["HEAD:nonexistent.luau".to_string()];
        let results = git_batch_check_hashes_impl(&repo, &refs);
        assert_eq!(results.len(), 1);
        assert!(results[0].is_none());
    }

    #[test]
    fn batch_check_head_and_staged_same() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("a.luau"), "content a").unwrap();
        git_commit_all(dir.path(), "init");

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec!["HEAD:a.luau".to_string(), ":0:a.luau".to_string()];
        let results = git_batch_check_hashes_impl(&repo, &refs);
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

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec!["HEAD:b.luau".to_string(), ":0:b.luau".to_string()];
        let results = git_batch_check_hashes_impl(&repo, &refs);
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

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec![
            "HEAD:a.luau".to_string(),
            ":0:a.luau".to_string(),
            "HEAD:b.luau".to_string(),
            ":0:b.luau".to_string(),
            "HEAD:nonexistent.luau".to_string(),
        ];
        let results = git_batch_check_hashes_impl(&repo, &refs);
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

        let repo = open_repo(dir.path()).unwrap();
        let refs = vec!["HEAD:src/init.luau".to_string()];
        let results = git_batch_check_hashes_impl(&repo, &refs);
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
    // Hash consistency: our SHA1 matches gix blob hashing
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
