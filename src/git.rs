use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use rbx_dom_weak::types::Ref;
use sha1::{Digest, Sha1};

use crate::{snapshot::RojoTree, web::interface::GitMetadata};

const SCRIPT_CLASSES: &[&str] = &["Script", "LocalScript", "ModuleScript"];

fn is_script_class(class: &str) -> bool {
    SCRIPT_CLASSES.contains(&class)
}

pub fn git_repo_root(project_root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args([
            "-C",
            &project_root.to_string_lossy(),
            "rev-parse",
            "--show-toplevel",
        ])
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

fn git_changed_files(repo_root: &Path) -> Option<HashSet<PathBuf>> {
    let mut changed = HashSet::new();

    let diff_output = Command::new("git")
        .args([
            "-C",
            &repo_root.to_string_lossy(),
            "diff",
            "HEAD",
            "--name-only",
        ])
        .output()
        .ok()?;

    if diff_output.status.success() {
        for line in String::from_utf8_lossy(&diff_output.stdout).lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                changed.insert(PathBuf::from(trimmed));
            }
        }
    } else {
        log::debug!("git diff HEAD failed (possibly no commits yet), skipping diff");
    }

    let untracked_output = Command::new("git")
        .args([
            "-C",
            &repo_root.to_string_lossy(),
            "ls-files",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;

    if untracked_output.status.success() {
        for line in String::from_utf8_lossy(&untracked_output.stdout).lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                changed.insert(PathBuf::from(trimmed));
            }
        }
    }

    if changed.is_empty() && !diff_output.status.success() {
        return None;
    }

    Some(changed)
}

fn git_show(repo_root: &Path, object_ref: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C", &repo_root.to_string_lossy(), "show", object_ref])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

fn git_show_head(repo_root: &Path, rel_path: &Path) -> Option<String> {
    let object_ref = format!("HEAD:{}", rel_path.to_string_lossy().replace('\\', "/"));
    git_show(repo_root, &object_ref)
}

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

pub fn git_add(repo_root: &Path, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }

    let mut cmd = Command::new("git");
    cmd.args(["-C", &repo_root.to_string_lossy(), "add", "--"]);
    for path in paths {
        cmd.arg(path);
    }

    match cmd.output() {
        Ok(output) => {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                log::warn!("git add failed: {}", stderr.trim());
            }
        }
        Err(err) => {
            log::warn!("Failed to run git add: {}", err);
        }
    }
}

struct ResolvedInstance {
    id: Ref,
    class_name: String,
    rel_path: PathBuf,
}

/// Computes git metadata for the two-way sync confirmation UI.
///
/// Uses a two-phase approach to avoid holding the tree lock during I/O:
/// 1. Run git commands to get changed files (no lock)
/// 2. Briefly lock tree to resolve file paths to instance Refs and class names
/// 3. Run git show for each changed script and compute hashes (no lock)
pub fn compute_git_metadata(
    tree_handle: &Arc<Mutex<RojoTree>>,
    project_root: &Path,
) -> Option<GitMetadata> {
    let repo_root = git_repo_root(project_root)?;
    let changed_files = git_changed_files(&repo_root)?;

    if changed_files.is_empty() {
        return Some(GitMetadata {
            changed_ids: Vec::new(),
            script_committed_hashes: HashMap::new(),
        });
    }

    // Brief lock: resolve changed file paths to instance Refs and class names
    let resolved: Vec<ResolvedInstance> = {
        let tree = tree_handle.lock().unwrap();
        let mut result = Vec::new();

        for rel_path in &changed_files {
            let abs_path = repo_root.join(rel_path);
            let ids = tree.get_ids_at_path(&abs_path);

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
    }; // Lock released

    let changed_ids: Vec<Ref> = resolved.iter().map(|ri| ri.id).collect();
    let mut script_committed_hashes: HashMap<Ref, Vec<String>> = HashMap::new();

    for ri in &resolved {
        if !is_script_class(&ri.class_name) {
            continue;
        }

        let mut hashes = Vec::with_capacity(2);

        if let Some(head_content) = git_show_head(&repo_root, &ri.rel_path) {
            hashes.push(compute_blob_sha1(&head_content));
        }

        if let Some(staged_content) = git_show_staged(&repo_root, &ri.rel_path) {
            let staged_hash = compute_blob_sha1(&staged_content);
            if hashes.is_empty() || hashes[0] != staged_hash {
                hashes.push(staged_hash);
            }
        }

        if !hashes.is_empty() {
            script_committed_hashes.insert(ri.id, hashes);
        }
    }

    Some(GitMetadata {
        changed_ids,
        script_committed_hashes,
    })
}
