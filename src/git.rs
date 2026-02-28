use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

use rbx_dom_weak::types::Ref;
use sha1::{Digest, Sha1};

use crate::{
    snapshot::{is_script_class, RojoTree},
    web::interface::GitMetadata,
};

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

#[cfg(test)]
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

/// Retrieves git blob hashes for multiple object refs in a single subprocess.
/// Returns a Vec with the same length as `object_refs`, where each element is
/// `Some(sha1_hex)` if the object exists, or `None` if missing.
///
/// Uses `git cat-file --batch-check` which outputs the stored blob hash
/// directly, avoiding the need to download and re-hash file contents.
fn git_batch_check_hashes(repo_root: &Path, object_refs: &[String]) -> Vec<Option<String>> {
    if object_refs.is_empty() {
        return Vec::new();
    }

    let mut child = match Command::new("git")
        .args([
            "-C",
            &repo_root.to_string_lossy(),
            "cat-file",
            "--batch-check",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            log::warn!("Failed to spawn git cat-file --batch-check: {}", err);
            return vec![None; object_refs.len()];
        }
    };

    let stdin = child.stdin.take().unwrap();
    let refs_for_writer: Vec<String> = object_refs.to_vec();
    let writer_thread = thread::spawn(move || {
        let mut stdin = stdin;
        for object_ref in &refs_for_writer {
            if writeln!(stdin, "{}", object_ref).is_err() {
                break;
            }
        }
    });

    let expected = object_refs.len();
    let mut results = Vec::with_capacity(expected);

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            if line.ends_with(" missing") {
                results.push(None);
            } else if let Some(sha1) = line.split(' ').next() {
                if sha1.len() == 40 && sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                    results.push(Some(sha1.to_string()));
                } else {
                    results.push(None);
                }
            } else {
                results.push(None);
            }

            if results.len() >= expected {
                break;
            }
        }
    }

    while results.len() < expected {
        results.push(None);
    }

    let _ = writer_thread.join();
    let _ = child.wait();
    results
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
/// `repo_root` must be the git repository root (from `git_repo_root()`),
/// cached at session start by `ServeSession`.
///
/// Uses a three-phase approach to avoid holding the tree lock during I/O:
/// 1. Run git commands to get changed files (no lock)
/// 2. Briefly lock tree to resolve file paths to instance Refs and class names
/// 3. Single `git cat-file --batch-check` call to get blob hashes (no lock)
pub fn compute_git_metadata(tree_handle: &Arc<Mutex<RojoTree>>, repo_root: &Path) -> GitMetadata {
    let changed_files = match git_changed_files(repo_root) {
        Some(files) => files,
        None => {
            return GitMetadata {
                changed_ids: Vec::new(),
                script_committed_hashes: HashMap::new(),
            };
        }
    };

    if changed_files.is_empty() {
        return GitMetadata {
            changed_ids: Vec::new(),
            script_committed_hashes: HashMap::new(),
        };
    }

    // Brief lock: resolve changed file paths to instance Refs and class names
    let resolved: Vec<ResolvedInstance> = {
        let tree = tree_handle.lock().unwrap();
        let mut result = Vec::new();

        for rel_path in &changed_files {
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
    }; // Lock released

    let changed_ids: Vec<Ref> = resolved.iter().map(|ri| ri.id).collect();
    let mut script_committed_hashes: HashMap<Ref, Vec<String>> = HashMap::new();

    struct ScriptRef {
        resolved_idx: usize,
        is_staged: bool,
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
            is_staged: false,
        });

        object_refs.push(format!(":0:{}", path_str));
        script_refs.push(ScriptRef {
            resolved_idx: idx,
            is_staged: true,
        });
    }

    let batch_hashes = git_batch_check_hashes(repo_root, &object_refs);

    for (i, info) in script_refs.iter().enumerate() {
        let sha1 = match batch_hashes.get(i).and_then(|h| h.as_ref()) {
            Some(h) => h,
            None => continue,
        };
        let ri = &resolved[info.resolved_idx];
        let hashes = script_committed_hashes
            .entry(ri.id)
            .or_insert_with(|| Vec::with_capacity(2));

        if info.is_staged {
            if hashes.is_empty() || hashes[0] != *sha1 {
                hashes.push(sha1.clone());
            }
        } else {
            hashes.push(sha1.clone());
        }
    }

    GitMetadata {
        changed_ids,
        script_committed_hashes,
    }
}

/// Refreshes the git index if the project is in a git repository.
///
/// This is useful because syncback may rewrite files with identical content,
/// which can cause git to report them as modified due to timestamp changes.
pub fn refresh_git_index(project_dir: &Path) {
    let mut check_dir = Some(project_dir);
    let mut is_git_repo = false;
    while let Some(dir) = check_dir {
        if dir.join(".git").exists() {
            is_git_repo = true;
            break;
        }
        check_dir = dir.parent();
    }

    if is_git_repo {
        log::info!("Refreshing git index...");
        match Command::new("git")
            .args(["update-index", "--refresh", "-q"])
            .current_dir(project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(_) => log::info!("Git index refreshed."),
            Err(e) => log::warn!("Failed to run git update-index --refresh: {}", e),
        }
    } else {
        log::debug!("Not a git repository, skipping index refresh.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn git_init(dir: &Path) {
        Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "init"])
            .output()
            .expect("git init failed");
        Command::new("git")
            .args([
                "-C",
                &dir.to_string_lossy(),
                "config",
                "user.email",
                "test@test.com",
            ])
            .output()
            .unwrap();
        Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "config", "user.name", "Test"])
            .output()
            .unwrap();
    }

    fn git_commit_all(dir: &Path, msg: &str) {
        Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "add", "-A"])
            .output()
            .unwrap();
        Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "commit", "-m", msg])
            .output()
            .unwrap();
    }

    fn git_stage(dir: &Path, file: &str) {
        Command::new("git")
            .args(["-C", &dir.to_string_lossy(), "add", file])
            .output()
            .unwrap();
    }

    fn git_is_staged(dir: &Path, file: &str) -> bool {
        let output = Command::new("git")
            .args([
                "-C",
                &dir.to_string_lossy(),
                "diff",
                "--cached",
                "--name-only",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().any(|l| l.trim() == file)
    }

    // -----------------------------------------------------------------------
    // compute_blob_sha1
    // -----------------------------------------------------------------------

    #[test]
    fn blob_sha1_empty_content() {
        let hash = compute_blob_sha1("");
        // git hash-object -t blob --stdin <<< '' produces the hash for empty blob
        // "blob 0\0" -> SHA1 = e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
        assert_eq!(hash, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn blob_sha1_hello_world() {
        // Verify against `echo -n "hello world" | git hash-object --stdin`
        // "blob 11\0hello world" -> SHA1 = 95d09f2b10159347eece71399a7e2e907ea3df4f
        let hash = compute_blob_sha1("hello world");
        assert_eq!(hash, "95d09f2b10159347eece71399a7e2e907ea3df4f");
    }

    #[test]
    fn blob_sha1_multiline_script() {
        let content = "local foo = 1\nlocal bar = 2\nreturn foo + bar\n";
        let hash = compute_blob_sha1(content);
        assert_eq!(hash.len(), 40);
        // Same content must produce same hash
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
        fs::write(dir.path().join("test.luau"), content).unwrap();

        let output = Command::new("git")
            .args([
                "-C",
                &dir.path().to_string_lossy(),
                "hash-object",
                "test.luau",
            ])
            .output()
            .unwrap();
        let git_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();

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
        // Subdirectory should resolve to the same repo root
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

        let changed = git_changed_files(dir.path());
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

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("script.luau")));
    }

    #[test]
    fn changed_files_staged_modification() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("script.luau"), "staged version").unwrap();
        git_stage(dir.path(), "script.luau");

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("script.luau")));
    }

    #[test]
    fn changed_files_untracked_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("existing.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("new_file.luau"), "new").unwrap();

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("new_file.luau")));
    }

    #[test]
    fn changed_files_deleted_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("to_delete.luau"), "content").unwrap();
        git_commit_all(dir.path(), "init");
        fs::remove_file(dir.path().join("to_delete.luau")).unwrap();

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("to_delete.luau")));
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

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("will_modify.luau")));
        assert!(changed.contains(&PathBuf::from("untracked.luau")));
        assert!(!changed.contains(&PathBuf::from("committed.luau")));
    }

    #[test]
    fn changed_files_no_head_returns_none() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        // No commits at all, but a file exists
        fs::write(dir.path().join("new.luau"), "content").unwrap();

        let changed = git_changed_files(dir.path());
        // Should still return Some with the untracked file
        assert!(changed.is_some());
        assert!(changed.unwrap().contains(&PathBuf::from("new.luau")));
    }

    #[test]
    fn changed_files_subdirectory() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/module.luau"), "original").unwrap();
        git_commit_all(dir.path(), "init");
        fs::write(dir.path().join("src/module.luau"), "modified").unwrap();

        let changed = git_changed_files(dir.path()).unwrap();
        assert!(changed.contains(&PathBuf::from("src/module.luau")));
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

        // File not staged, show :0: should return committed version (same as HEAD)
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
    // git_batch_check_hashes
    // -----------------------------------------------------------------------

    #[test]
    fn batch_check_empty_refs() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        let results = git_batch_check_hashes(dir.path(), &[]);
        assert!(results.is_empty());
    }

    #[test]
    fn batch_check_single_ref() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("script.luau"), "local x = 42\nreturn x").unwrap();
        git_commit_all(dir.path(), "init");

        let refs = vec!["HEAD:script.luau".to_string()];
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        let results = git_batch_check_hashes(dir.path(), &refs);
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
        // Should not panic or error
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
    // Hash consistency: our SHA1 matches git hash-object for various content
    // -----------------------------------------------------------------------

    #[test]
    fn hash_consistency_empty_file() {
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("empty.luau"), "").unwrap();

        let output = Command::new("git")
            .args([
                "-C",
                &dir.path().to_string_lossy(),
                "hash-object",
                "empty.luau",
            ])
            .output()
            .unwrap();
        let git_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(compute_blob_sha1(""), git_hash);
    }

    #[test]
    fn hash_consistency_unicode_content() {
        let content = "-- Unicode: 日本語テスト\nlocal x = '🎮'\n";
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("unicode.luau"), content).unwrap();

        let output = Command::new("git")
            .args([
                "-C",
                &dir.path().to_string_lossy(),
                "hash-object",
                "unicode.luau",
            ])
            .output()
            .unwrap();
        let git_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(compute_blob_sha1(content), git_hash);
    }

    #[test]
    fn hash_consistency_large_file() {
        let content: String = (0..10000)
            .map(|i| format!("local var_{} = {}\n", i, i))
            .collect();
        let dir = tempdir().unwrap();
        git_init(dir.path());
        fs::write(dir.path().join("large.luau"), &content).unwrap();

        let output = Command::new("git")
            .args([
                "-C",
                &dir.path().to_string_lossy(),
                "hash-object",
                "large.luau",
            ])
            .output()
            .unwrap();
        let git_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(compute_blob_sha1(&content), git_hash);
    }
}
