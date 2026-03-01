use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

use gix::bstr::ByteSlice;
use rbx_dom_weak::types::Ref;
use sha1::{Digest, Sha1};

use crate::{
    snapshot::{is_script_class, RojoTree},
    web::interface::GitMetadata,
};

fn open_repo(path: &Path) -> Option<gix::Repository> {
    gix::discover(path).ok()
}

#[cfg(windows)]
fn strip_unc_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_owned()
    }
}

#[cfg(not(windows))]
fn strip_unc_prefix(path: &Path) -> PathBuf {
    path.to_owned()
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
) -> Option<ChangedFiles> {
    let mut tracked = HashSet::new();
    let mut untracked = HashSet::new();

    let t = Instant::now();
    let diff_base = initial_head.unwrap_or("HEAD");
    let base_tree_id = resolve_tree_id(repo, diff_base);
    let head_tree_id = resolve_tree_id(repo, "HEAD");
    log::trace!(
        "[TIMING] compute_git_metadata: resolve_tree_id {}ms",
        t.elapsed().as_millis()
    );

    let diff_failed = base_tree_id.is_none();

    // If initial_head differs from HEAD, diff those trees for committed-since-start changes
    if let (Some(base_id), Some(head_id)) = (base_tree_id, head_tree_id) {
        if base_id != head_id {
            let t = Instant::now();
            if let (Ok(base_obj), Ok(head_obj)) =
                (repo.find_object(base_id), repo.find_object(head_id))
            {
                if let (Ok(base_tree), Ok(head_tree)) =
                    (base_obj.peel_to_tree(), head_obj.peel_to_tree())
                {
                    collect_tree_diff_paths(repo, &base_tree, &head_tree, &mut tracked);
                }
            }
            log::trace!(
                "[TIMING] compute_git_metadata: collect_tree_diff_paths {}ms ({} found)",
                t.elapsed().as_millis(),
                tracked.len()
            );
        }
    }

    // Compare base tree vs index for staged changes, and index vs worktree for unstaged
    if let Some(base_id) = base_tree_id.or(head_tree_id) {
        if let Ok(index) = repo.open_index() {
            let t = Instant::now();
            collect_tree_index_changes(repo, base_id, &index, &mut tracked);
            log::trace!(
                "[TIMING] compute_git_metadata: collect_tree_index_changes {}ms",
                t.elapsed().as_millis()
            );

            let t = Instant::now();
            let before = tracked.len();
            collect_index_worktree_changes(repo, &index, &mut tracked);
            log::trace!(
                "[TIMING] compute_git_metadata: collect_index_worktree_changes {}ms ({} entries, {} worktree-dirty)",
                t.elapsed().as_millis(),
                index.entries().len(),
                tracked.len() - before
            );
        }
    } else if let Ok(index) = repo.open_index() {
        for entry in index.entries() {
            tracked.insert(PathBuf::from(entry.path(&index).to_str_lossy().as_ref()));
        }
        let t = Instant::now();
        let before = tracked.len();
        collect_index_worktree_changes(repo, &index, &mut tracked);
        log::trace!(
            "[TIMING] compute_git_metadata: collect_index_worktree_changes (no tree) {}ms ({} entries, {} worktree-dirty)",
            t.elapsed().as_millis(),
            index.entries().len(),
            tracked.len() - before
        );
    }

    let t = Instant::now();
    collect_untracked_files(repo, &mut untracked);
    log::trace!(
        "[TIMING] compute_git_metadata: collect_untracked_files {}ms ({} found)",
        t.elapsed().as_millis(),
        untracked.len()
    );

    if tracked.is_empty() && untracked.is_empty() && diff_failed {
        return None;
    }

    Some(ChangedFiles { tracked, untracked })
}

fn collect_tree_diff_paths(
    repo: &gix::Repository,
    old_tree: &gix::Tree<'_>,
    new_tree: &gix::Tree<'_>,
    tracked: &mut HashSet<PathBuf>,
) {
    if let Ok(changes) = repo.diff_tree_to_tree(old_tree, Some(new_tree), None) {
        for change in changes {
            let path_str = change.location().to_str_lossy();
            tracked.insert(PathBuf::from(path_str.as_ref()));
        }
    }
}

fn collect_tree_index_changes(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
    index: &gix::index::File,
    tracked: &mut HashSet<PathBuf>,
) {
    let tree_obj = match repo.find_object(tree_id) {
        Ok(o) => o,
        Err(_) => return,
    };
    let tree = match tree_obj.peel_to_tree() {
        Ok(t) => t,
        Err(_) => return,
    };

    for entry in index.entries() {
        let path_bstr = entry.path(index);
        let path_str = path_bstr.to_str_lossy();

        let tree_entry_id = tree
            .lookup_entry_by_path(path_str.as_ref())
            .ok()
            .flatten()
            .map(|e| e.object_id());

        match tree_entry_id {
            Some(tid) if tid == entry.id => {}
            _ => {
                tracked.insert(PathBuf::from(path_str.as_ref()));
            }
        }
    }

    collect_tree_only_paths(repo, &tree, "", index, tracked);
}

fn collect_tree_only_paths(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    prefix: &str,
    index: &gix::index::File,
    tracked: &mut HashSet<PathBuf>,
) {
    for entry in tree.iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.filename().to_str_lossy();
        let full_path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", prefix, name)
        };

        if entry.mode().is_tree() {
            if let Ok(sub_obj) = entry.id().object() {
                if let Ok(sub_tree) = sub_obj.peel_to_tree() {
                    collect_tree_only_paths(repo, &sub_tree, &full_path, index, tracked);
                }
            }
        } else if entry.mode().is_blob() || entry.mode().is_executable() {
            let path_as_bstr: &gix::bstr::BStr = full_path.as_bytes().as_bstr();
            if index.entry_by_path(path_as_bstr).is_none() {
                tracked.insert(PathBuf::from(&full_path));
            }
        }
    }
}

fn compute_blob_object_id(content: &[u8]) -> gix::ObjectId {
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    gix::ObjectId::from_bytes_or_panic(&hasher.finalize())
}

fn collect_index_worktree_changes(
    repo: &gix::Repository,
    index: &gix::index::File,
    tracked: &mut HashSet<PathBuf>,
) {
    let work_dir = match repo.workdir() {
        Some(d) => d.to_owned(),
        None => return,
    };

    let stat_options: gix::index::entry::stat::Options = Default::default();

    for entry in index.entries() {
        let path_bstr = entry.path(index);
        let path_str = path_bstr.to_str_lossy();
        let full_path = work_dir.join(path_str.as_ref());

        match gix::index::fs::Metadata::from_path_no_follow(&full_path) {
            Ok(fs_meta) => {
                if let Ok(fs_stat) = gix::index::entry::Stat::from_fs(&fs_meta) {
                    if entry.stat.matches(&fs_stat, stat_options) {
                        continue;
                    }
                }
                let content = match std::fs::read(&full_path) {
                    Ok(c) => c,
                    Err(_) => {
                        tracked.insert(PathBuf::from(path_str.as_ref()));
                        continue;
                    }
                };
                if compute_blob_object_id(&content) != entry.id {
                    tracked.insert(PathBuf::from(path_str.as_ref()));
                }
            }
            Err(_) => {
                tracked.insert(PathBuf::from(path_str.as_ref()));
            }
        }
    }
}

fn collect_untracked_files(repo: &gix::Repository, untracked: &mut HashSet<PathBuf>) {
    let work_dir = match repo.workdir() {
        Some(d) => d.to_owned(),
        None => return,
    };
    let index = repo.open_index().unwrap_or_else(|_| {
        gix::index::File::from_state(
            gix::index::State::new(repo.object_hash()),
            repo.index_path(),
        )
    });

    let mut tracked_dirs: HashSet<String> = HashSet::new();
    tracked_dirs.insert(String::new());
    for entry in index.entries() {
        let path_str = entry.path(&index).to_str_lossy();
        let mut pos = 0;
        while let Some(slash) = path_str[pos..].find('/') {
            let prefix = &path_str[..pos + slash];
            tracked_dirs.insert(prefix.to_string());
            pos += slash + 1;
        }
    }

    fn walk_dir(
        base: &Path,
        dir: &Path,
        index: &gix::index::File,
        tracked_dirs: &HashSet<String>,
        untracked: &mut HashSet<PathBuf>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if name == ".git" {
                continue;
            }

            let rel = match path.strip_prefix(base) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    if tracked_dirs.contains(rel_str.as_ref() as &str) {
                        walk_dir(base, &path, index, tracked_dirs, untracked);
                    }
                } else if meta.is_file() {
                    let path_as_bstr: &gix::bstr::BStr = rel_str.as_bytes().as_bstr();
                    let is_tracked = index.entry_by_path(path_as_bstr).is_some();
                    if !is_tracked {
                        untracked.insert(PathBuf::from(&rel_str));
                    }
                }
            }
        }
    }

    walk_dir(&work_dir, &work_dir, &index, &tracked_dirs, untracked);
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

    let repo = match open_repo(repo_root) {
        Some(r) => r,
        None => return,
    };
    let mut index = match repo.open_index() {
        Ok(i) => i,
        Err(_) => return,
    };

    let work_dir = match repo.workdir() {
        Some(d) => d.to_owned(),
        None => return,
    };

    for path in paths {
        let abs_path = if path.is_absolute() {
            path.to_owned()
        } else {
            work_dir.join(path)
        };
        let norm_abs = strip_unc_prefix(&abs_path);
        let norm_work = strip_unc_prefix(&work_dir);
        let rel_path = match norm_abs.strip_prefix(&norm_work) {
            Ok(r) => r.to_owned(),
            Err(_) => path.to_owned(),
        };
        let content = match std::fs::read(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let blob_id = match repo.write_blob(&content) {
            Ok(id) => id.detach(),
            Err(_) => continue,
        };

        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        let bstr_path: gix::bstr::BString = rel_str.into();

        let gix_meta = match gix::index::fs::Metadata::from_path_no_follow(&abs_path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let stat = match gix::index::entry::Stat::from_fs(&gix_meta) {
            Ok(s) => s,
            Err(_) => gix::index::entry::Stat::default(),
        };

        if let Some(existing) = index
            .entry_mut_by_path_and_stage(bstr_path.as_ref(), gix::index::entry::Stage::Unconflicted)
        {
            existing.id = blob_id;
            existing.stat = stat;
        } else {
            index.dangerously_push_entry(
                stat,
                blob_id,
                gix::index::entry::Flags::empty(),
                gix::index::entry::Mode::FILE,
                bstr_path.as_ref(),
            );
        }
    }

    index.sort_entries();
    if let Err(e) = index.write(gix::index::write::Options::default()) {
        log::warn!("Failed to write git index: {}", e);
    }
}

/// Build a tree object from sorted index entries and write it to the ODB.
pub fn write_index_tree(repo: &gix::Repository, index: &gix::index::File) -> Option<gix::ObjectId> {
    use gix::objs::tree;
    use std::collections::BTreeMap;

    enum TreeNode {
        Blob(gix::ObjectId, tree::EntryKind),
        Tree(BTreeMap<String, TreeNode>),
    }

    let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();

    for entry in index.entries() {
        let path = entry.path(index).to_str_lossy();
        let parts: Vec<&str> = path.split('/').collect();

        let mut current = &mut root;
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                let kind = if entry.mode == gix::index::entry::Mode::FILE_EXECUTABLE {
                    tree::EntryKind::BlobExecutable
                } else {
                    tree::EntryKind::Blob
                };
                current.insert(part.to_string(), TreeNode::Blob(entry.id, kind));
            } else {
                current = match current
                    .entry(part.to_string())
                    .or_insert_with(|| TreeNode::Tree(BTreeMap::new()))
                {
                    TreeNode::Tree(ref mut map) => map,
                    _ => return None,
                };
            }
        }
    }

    fn write_recursive(
        repo: &gix::Repository,
        nodes: &BTreeMap<String, TreeNode>,
    ) -> Option<gix::ObjectId> {
        let mut tree = gix::objs::Tree::empty();
        for (name, node) in nodes {
            match node {
                TreeNode::Blob(id, kind) => {
                    tree.entries.push(tree::Entry {
                        mode: (*kind).into(),
                        filename: name.as_str().into(),
                        oid: *id,
                    });
                }
                TreeNode::Tree(children) => {
                    let subtree_id = write_recursive(repo, children)?;
                    tree.entries.push(tree::Entry {
                        mode: tree::EntryKind::Tree.into(),
                        filename: name.as_str().into(),
                        oid: subtree_id,
                    });
                }
            }
        }
        repo.write_object(&tree).ok().map(|id| id.detach())
    }

    write_recursive(repo, &root)
}

/// Stage all worktree files and create a commit.
pub fn git_add_all_and_commit(dir: &Path, message: &str) {
    let repo = match open_repo(dir) {
        Some(r) => r,
        None => return,
    };
    let work_dir = match repo.workdir() {
        Some(d) => d.to_owned(),
        None => return,
    };

    log::info!("Staging all files...");
    let mut index = gix::index::File::from_state(
        gix::index::State::new(repo.object_hash()),
        repo.index_path(),
    );

    fn add_dir(repo: &gix::Repository, base: &Path, dir: &Path, index: &mut gix::index::File) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if name == ".git" {
                continue;
            }
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    add_dir(repo, base, &path, index);
                } else if ft.is_file() {
                    let rel = match path.strip_prefix(base) {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    let content = match std::fs::read(&path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let blob_id = match repo.write_blob(&content) {
                        Ok(id) => id.detach(),
                        Err(_) => continue,
                    };
                    let gix_meta = match gix::index::fs::Metadata::from_path_no_follow(&path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let stat = gix::index::entry::Stat::from_fs(&gix_meta).unwrap_or_default();
                    let bstr_path: &gix::bstr::BStr = rel_str.as_bytes().as_bstr();
                    index.dangerously_push_entry(
                        stat,
                        blob_id,
                        gix::index::entry::Flags::empty(),
                        gix::index::entry::Mode::FILE,
                        bstr_path,
                    );
                }
            }
        }
    }

    add_dir(&repo, &work_dir, &work_dir, &mut index);
    index.sort_entries();
    if index.write(gix::index::write::Options::default()).is_err() {
        log::warn!("Failed to write git index");
        return;
    }

    log::info!("Committing: {}", message);
    let tree_id = match write_index_tree(&repo, &index) {
        Some(id) => id,
        None => {
            log::warn!("Failed to write tree from index");
            return;
        }
    };

    let parents: Vec<gix::ObjectId> = repo
        .head_id()
        .ok()
        .map(|id| id.detach())
        .into_iter()
        .collect();
    if let Err(e) = repo.commit("HEAD", message, tree_id, parents.iter().copied()) {
        log::warn!("Failed to commit: {}", e);
    }
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
/// 1. Use gix to get changed files (no lock)
/// 2. Briefly lock tree to resolve file paths to instance Refs and class names
/// 3. Direct object/index lookups to get blob hashes (no lock)
pub fn compute_git_metadata(
    tree_handle: &Arc<Mutex<RojoTree>>,
    repo_root: &Path,
    initial_head: Option<&str>,
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
    log::trace!(
        "[TIMING] compute_git_metadata: open_repo {}ms",
        t.elapsed().as_millis()
    );

    let t = Instant::now();
    let changed_files = match git_changed_files_impl(&repo, initial_head) {
        Some(files) => files,
        None => {
            log::trace!(
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
    log::trace!(
        "[TIMING] compute_git_metadata: git_changed_files_impl {}ms ({} tracked, {} untracked)",
        t.elapsed().as_millis(),
        changed_files.tracked.len(),
        changed_files.untracked.len()
    );

    if changed_files.is_empty() {
        log::trace!(
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
    log::trace!(
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
    log::trace!(
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

    log::trace!(
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
    let repo = match open_repo(project_dir) {
        Some(r) => r,
        None => {
            log::debug!("Not a git repository, skipping index refresh.");
            return;
        }
    };

    log::info!("Refreshing git index...");
    match repo.open_index() {
        Ok(mut index) => {
            let work_dir = match repo.workdir() {
                Some(d) => d.to_owned(),
                None => return,
            };

            let paths_and_stats: Vec<_> = index
                .entries()
                .iter()
                .filter_map(|entry| {
                    let path_str = entry.path(&index).to_str_lossy();
                    let full_path = work_dir.join(path_str.as_ref());
                    let gix_meta =
                        gix::index::fs::Metadata::from_path_no_follow(&full_path).ok()?;
                    let stat = gix::index::entry::Stat::from_fs(&gix_meta).ok()?;
                    Some((path_str.to_string(), stat))
                })
                .collect();

            for (path_str, stat) in paths_and_stats {
                let bstr_path: &gix::bstr::BStr = path_str.as_bytes().as_bstr();
                if let Some(entry) = index
                    .entry_mut_by_path_and_stage(bstr_path, gix::index::entry::Stage::Unconflicted)
                {
                    entry.stat = stat;
                }
            }

            match index.write(gix::index::write::Options::default()) {
                Ok(_) => log::info!("Git index refreshed."),
                Err(e) => log::warn!("Failed to write refreshed git index: {}", e),
            }
        }
        Err(e) => log::warn!("Failed to open git index for refresh: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn git_init(dir: &Path) {
        let mut repo = gix::init(dir).expect("gix init failed");
        {
            let mut config = repo.config_snapshot_mut();
            config
                .set_raw_value(&gix::config::tree::User::NAME, "Test")
                .unwrap();
            config
                .set_raw_value(&gix::config::tree::User::EMAIL, "test@test.com")
                .unwrap();
            let _ = config.commit_auto_rollback().unwrap();
        }
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

    #[test]
    fn blob_sha1_matches_gix_hash_object() {
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let content = "print('test script')\n";
        fs::write(dir.path().join("test.luau"), content).unwrap();

        let repo = open_repo(dir.path()).unwrap();
        let blob_id = repo.write_blob(content.as_bytes()).unwrap();
        let gix_hash = blob_id.to_hex().to_string();

        let our_hash = compute_blob_sha1(content);
        assert_eq!(our_hash, gix_hash);
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
        let changed = git_changed_files_impl(&repo, None);
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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
        let changed = git_changed_files_impl(&repo, None);
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
        let changed = git_changed_files_impl(&repo, None).unwrap();
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

        let without = git_changed_files_impl(&repo, None).unwrap();
        assert!(
            !without.all().contains(&PathBuf::from("script.luau")),
            "without initial_head, committed file should NOT appear"
        );

        let with = git_changed_files_impl(&repo, Some(&initial_head)).unwrap();
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

        let repo = open_repo(dir.path()).unwrap();
        let gix_hash = repo.write_blob(b"").unwrap().to_hex().to_string();
        assert_eq!(compute_blob_sha1(""), gix_hash);
    }

    #[test]
    fn hash_consistency_unicode_content() {
        let content = "-- Unicode: 日本語テスト\nlocal x = '🎮'\n";
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let repo = open_repo(dir.path()).unwrap();
        let gix_hash = repo
            .write_blob(content.as_bytes())
            .unwrap()
            .to_hex()
            .to_string();
        assert_eq!(compute_blob_sha1(content), gix_hash);
    }

    #[test]
    fn hash_consistency_large_file() {
        let content: String = (0..10000)
            .map(|i| format!("local var_{} = {}\n", i, i))
            .collect();
        let dir = tempdir().unwrap();
        git_init(dir.path());

        let repo = open_repo(dir.path()).unwrap();
        let gix_hash = repo
            .write_blob(content.as_bytes())
            .unwrap()
            .to_hex()
            .to_string();
        assert_eq!(compute_blob_sha1(&content), gix_hash);
    }
}
