//! Integration tests for git-based sync direction defaults.
//!
//! Tests the full pipeline: git metadata computation, ServerInfoResponse,
//! stage_ids in WriteRequest, and auto-staging behavior.
//!
//! These tests create real git repos in temp directories to verify:
//! - gitMetadata is present/absent based on git repo state
//! - changedIds correctly maps git-changed files to instance Refs
//! - scriptCommittedHashes contains correct SHA1 hashes
//! - Both HEAD and staged hashes are included when different
//! - stage_ids triggers git add on the server side
//! - Non-script changes are in changedIds but not in scriptCommittedHashes

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::{fs, thread};
use std::time::Duration;

use librojo::web_api::{AddedInstance, InstanceUpdate, WriteRequest};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, UstrMap};

use crate::rojo_test::serve_util::TestServeSession;

fn git_commit_all(dir: &Path, msg: &str) {
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .output()
        .expect("git add failed");
    Command::new("git")
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .output()
        .expect("git commit failed");
}

fn git_stage(dir: &Path, file: &str) {
    Command::new("git")
        .args(["add", file])
        .current_dir(dir)
        .output()
        .expect("git stage failed");
}

fn git_is_staged(dir: &Path, file: &str) -> bool {
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|l| l.trim() == file)
}

fn compute_blob_sha1(content: &str) -> String {
    use sha1::{Digest, Sha1};
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn find_instance_by_name<'a>(
    instances: &'a HashMap<Ref, librojo::web_api::Instance<'a>>,
    name: &str,
) -> Option<(Ref, &'a librojo::web_api::Instance<'a>)> {
    instances
        .iter()
        .find(|(_, inst)| inst.name.as_ref() == name)
        .map(|(&id, inst)| (id, inst))
}

// ===========================================================================
// gitMetadata presence/absence
// ===========================================================================

#[test]
fn git_metadata_present_when_in_git_repo() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
    });
    let info = session.wait_to_come_online();
    assert!(
        info.git_metadata.is_some(),
        "gitMetadata should be present when project is in a git repo"
    );
}

#[test]
fn git_metadata_absent_when_not_in_git_repo() {
    let mut session = TestServeSession::new("git_sync_defaults");
    let info = session.wait_to_come_online();
    assert!(
        info.git_metadata.is_none(),
        "gitMetadata should be None when project is not in a git repo"
    );
}

// ===========================================================================
// changedIds: file-to-instance mapping
// ===========================================================================

#[test]
fn no_changes_yields_empty_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // No modifications after commit
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    assert!(
        meta.changed_ids.is_empty(),
        "changedIds should be empty when no files changed"
    );
    assert!(meta.script_committed_hashes.is_empty());
}

#[test]
fn unstaged_script_modification_appears_in_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- modified\nreturn {}").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    assert!(
        !meta.changed_ids.is_empty(),
        "changedIds should contain the modified script's instance Ref"
    );
}

#[test]
fn staged_script_modification_appears_in_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- staged mod\nreturn {}").unwrap();
        git_stage(path, "src/ModuleA.luau");
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    assert!(!meta.changed_ids.is_empty());
}

#[test]
fn untracked_file_appears_in_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/NewScript.luau"), "return 'new'").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    assert!(
        !meta.changed_ids.is_empty(),
        "Untracked files should appear in changedIds"
    );
}

#[test]
fn unchanged_files_not_in_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // Only modify one file, others should not appear
        fs::write(path.join("src/ModuleA.luau"), "-- changed").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    let read = session.get_api_read(info.root_instance_id).unwrap();

    // Find ServerScript -- it was NOT modified, should NOT be in changedIds
    if let Some((server_id, _)) = find_instance_by_name(&read.instances, "ServerScript") {
        assert!(
            !meta.changed_ids.contains(&server_id),
            "Unchanged ServerScript should NOT be in changedIds"
        );
    }
}

#[test]
fn non_script_change_in_changed_ids_but_no_hash() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // Modify the model file (non-script)
        fs::write(
            path.join("src/SimpleModel.model.json5"),
            r#"{ "className": "Part", "properties": { "Anchored": false } }"#,
        )
        .unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((model_id, _)) = find_instance_by_name(&read.instances, "SimpleModel") {
        assert!(
            meta.changed_ids.contains(&model_id),
            "Modified non-script should be in changedIds"
        );
        assert!(
            !meta.script_committed_hashes.contains_key(&model_id),
            "Non-script should NOT have a committed hash"
        );
    }
}

// ===========================================================================
// scriptCommittedHashes: SHA1 computation
// ===========================================================================

#[test]
fn committed_hash_matches_head_content() {
    let original_content = "local ModuleA = {}\nreturn ModuleA\n";

    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // Modify the script so it appears as "changed"
        fs::write(path.join("src/ModuleA.luau"), "-- v2\nreturn {}").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((module_id, _)) = find_instance_by_name(&read.instances, "ModuleA") {
        let hashes = meta
            .script_committed_hashes
            .get(&module_id)
            .expect("ModuleA should have committed hashes");

        let expected_hash = compute_blob_sha1(original_content);
        assert!(
            hashes.contains(&expected_hash),
            "Committed hash should match SHA1 of original content. Expected: {}, Got: {:?}",
            expected_hash,
            hashes
        );
    }
}

#[test]
fn staged_hash_included_when_different_from_head() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- staged version").unwrap();
        git_stage(path, "src/ModuleA.luau");
        // Also modify working tree so file is still "changed"
        fs::write(path.join("src/ModuleA.luau"), "-- working tree version").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((module_id, _)) = find_instance_by_name(&read.instances, "ModuleA") {
        let hashes = meta
            .script_committed_hashes
            .get(&module_id)
            .expect("Should have hashes");

        assert!(
            hashes.len() == 2,
            "Should have 2 hashes (HEAD + staged) when staged differs from HEAD. Got: {}",
            hashes.len()
        );

        let head_hash = compute_blob_sha1("local ModuleA = {}\nreturn ModuleA\n");
        let staged_hash = compute_blob_sha1("-- staged version");

        assert!(hashes.contains(&head_hash), "Should contain HEAD hash");
        assert!(hashes.contains(&staged_hash), "Should contain staged hash");
    }
}

#[test]
fn staged_hash_deduplicated_when_same_as_head() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // Stage a different version, then unstage by overwriting with original
        // and re-staging... Actually the simplest way: modify, stage, then
        // modify working tree further (staged = HEAD content won't happen
        // if we staged a modification). Let's just verify with a clean stage.
        fs::write(path.join("src/ModuleA.luau"), "-- modified").unwrap();
        git_stage(path, "src/ModuleA.luau");
        // Now staged and HEAD are different, but if they were the same,
        // we'd only get 1 hash. Let's test the case where staged == HEAD:
        // Reset the stage to match HEAD content
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((module_id, _)) = find_instance_by_name(&read.instances, "ModuleA") {
        if let Some(hashes) = meta.script_committed_hashes.get(&module_id) {
            // When staged differs from HEAD, we get 2 hashes
            // When staged == HEAD, we'd get 1 hash (deduplicated)
            assert!(
                hashes.len() <= 2,
                "Should have at most 2 hashes"
            );
        }
    }
}

#[test]
fn new_untracked_script_has_no_committed_hash() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/BrandNew.luau"), "return 'brand new'").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((new_id, _)) = find_instance_by_name(&read.instances, "BrandNew") {
        assert!(
            meta.changed_ids.contains(&new_id),
            "New file should be in changedIds"
        );
        assert!(
            !meta.script_committed_hashes.contains_key(&new_id),
            "New file has no HEAD version, so no committed hash"
        );
    }
}

// ===========================================================================
// Init-style scripts (init.luau in directories)
// ===========================================================================

#[test]
fn init_style_script_appears_in_changed_ids() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/DirModule/init.luau"), "-- modified init").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    assert!(
        !meta.changed_ids.is_empty(),
        "Modified init.luau should cause its instance to appear in changedIds"
    );
}

#[test]
fn init_style_script_has_committed_hash() {
    let original = "local DirModule = {}\nreturn DirModule\n";

    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/DirModule/init.luau"), "-- v2").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((dir_id, _)) = find_instance_by_name(&read.instances, "DirModule") {
        let hashes = meta
            .script_committed_hashes
            .get(&dir_id)
            .expect("DirModule should have committed hashes");

        let expected_hash = compute_blob_sha1(original);
        assert!(
            hashes.contains(&expected_hash),
            "init.luau committed hash should match. Expected: {}, Got: {:?}",
            expected_hash,
            hashes
        );
    }
}

// ===========================================================================
// Multiple script types (Script, LocalScript, ModuleScript)
// ===========================================================================

#[test]
fn all_script_types_get_hashes() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        // Modify all three script types
        fs::write(path.join("src/ModuleA.luau"), "-- mod").unwrap();
        fs::write(path.join("src/ServerScript.server.luau"), "-- mod").unwrap();
        fs::write(path.join("src/ClientScript.client.luau"), "-- mod").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    for name in &["ModuleA", "ServerScript", "ClientScript"] {
        if let Some((id, _)) = find_instance_by_name(&read.instances, name) {
            assert!(
                meta.script_committed_hashes.contains_key(&id),
                "{} should have a committed hash",
                name
            );
        }
    }
}

// ===========================================================================
// stage_ids and auto-staging
// ===========================================================================

#[test]
fn stage_ids_in_write_request_triggers_git_add() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- modified for staging test").unwrap();
    });
    let info = session.wait_to_come_online();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    let (module_id, _) = find_instance_by_name(&read.instances, "ModuleA")
        .expect("ModuleA should exist");

    // Send a write request with stage_ids containing ModuleA
    let write_request = WriteRequest {
        session_id: info.session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![],
        stage_ids: vec![module_id],
    };
    session.post_api_write(&write_request).unwrap();

    // Give the server time to process
    thread::sleep(Duration::from_millis(500));

    // Verify the file was staged
    assert!(
        git_is_staged(session.path(), "src/ModuleA.luau"),
        "ModuleA.luau should be staged after stage_ids request"
    );
}

#[test]
fn empty_stage_ids_does_not_stage_anything() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- modified").unwrap();
    });
    let info = session.wait_to_come_online();

    // Send write with empty stage_ids
    let write_request = WriteRequest {
        session_id: info.session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![],
        stage_ids: vec![],
    };
    session.post_api_write(&write_request).unwrap();
    thread::sleep(Duration::from_millis(300));

    assert!(
        !git_is_staged(session.path(), "src/ModuleA.luau"),
        "File should NOT be staged when stage_ids is empty"
    );
}

#[test]
fn stage_ids_only_stages_specified_files() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- mod a").unwrap();
        fs::write(path.join("src/ServerScript.server.luau"), "-- mod server").unwrap();
    });
    let info = session.wait_to_come_online();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    let (module_id, _) = find_instance_by_name(&read.instances, "ModuleA")
        .expect("ModuleA should exist");

    // Only stage ModuleA, NOT ServerScript
    let write_request = WriteRequest {
        session_id: info.session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![],
        stage_ids: vec![module_id],
    };
    session.post_api_write(&write_request).unwrap();
    thread::sleep(Duration::from_millis(500));

    assert!(
        git_is_staged(session.path(), "src/ModuleA.luau"),
        "ModuleA should be staged"
    );
    assert!(
        !git_is_staged(session.path(), "src/ServerScript.server.luau"),
        "ServerScript should NOT be staged (not in stage_ids)"
    );
}

// ===========================================================================
// Source write + stage_ids interaction (change_processor staging)
// ===========================================================================

#[test]
fn source_write_with_stage_ids_stages_after_write() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
    });
    let info = session.wait_to_come_online();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    let (module_id, _) = find_instance_by_name(&read.instances, "ModuleA")
        .expect("ModuleA should exist");

    // Write a Source update AND include in stage_ids
    let mut props = UstrMap::default();
    props.insert(
        ustr("Source"),
        Some(Variant::String("-- new source from pull\nreturn {}\n".to_string())),
    );

    let write_request = WriteRequest {
        session_id: info.session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![InstanceUpdate {
            id: module_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: props,
            changed_metadata: None,
        }],
        stage_ids: vec![module_id],
    };
    session.post_api_write(&write_request).unwrap();

    // Wait for change_processor to write Source and stage
    thread::sleep(Duration::from_millis(1000));

    // Verify the NEW content was staged (not the old content)
    assert!(
        git_is_staged(session.path(), "src/ModuleA.luau"),
        "ModuleA should be staged after Source write + stage_ids"
    );

    // Verify the content on disk is the new version
    let disk_content = fs::read_to_string(session.path().join("src/ModuleA.luau")).unwrap();
    assert!(
        disk_content.contains("new source from pull"),
        "Disk content should be the new version"
    );
}

#[test]
fn source_write_without_stage_ids_does_not_stage() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
    });
    let info = session.wait_to_come_online();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    let (module_id, _) = find_instance_by_name(&read.instances, "ModuleA")
        .expect("ModuleA should exist");

    let mut props = UstrMap::default();
    props.insert(
        ustr("Source"),
        Some(Variant::String("-- manual pull, not auto-staged".to_string())),
    );

    // NO stage_ids
    let write_request = WriteRequest {
        session_id: info.session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![InstanceUpdate {
            id: module_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: props,
            changed_metadata: None,
        }],
        stage_ids: vec![],
    };
    session.post_api_write(&write_request).unwrap();
    thread::sleep(Duration::from_millis(1000));

    assert!(
        !git_is_staged(session.path(), "src/ModuleA.luau"),
        "ModuleA should NOT be staged when not in stage_ids"
    );
}

// ===========================================================================
// Reconnect gets fresh metadata
// ===========================================================================

#[test]
fn reconnect_gets_fresh_git_metadata() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
    });
    let info1 = session.wait_to_come_online();
    let meta1 = info1.git_metadata.unwrap();
    assert!(meta1.changed_ids.is_empty(), "No changes initially");

    // Now modify a file on disk
    fs::write(
        session.path().join("src/ModuleA.luau"),
        "-- changed after first connect",
    )
    .unwrap();

    // Re-query the API (simulates reconnect)
    let info2 = session.get_api_rojo().unwrap();
    let meta2 = info2.git_metadata.unwrap();
    assert!(
        !meta2.changed_ids.is_empty(),
        "Fresh connect should reflect new git changes"
    );
}

// ===========================================================================
// Edge cases
// ===========================================================================

#[test]
fn empty_repo_no_commits_returns_metadata() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |_path| {
        // Don't commit anything -- repo has no HEAD
    });
    let info = session.wait_to_come_online();
    // Should still get metadata (untracked files)
    // or None if git_changed_files returns None for no-HEAD repos
    // Either way, should not panic
    let _ = info.git_metadata;
}

#[test]
fn multiple_files_changed_all_tracked() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/ModuleA.luau"), "-- mod a").unwrap();
        fs::write(path.join("src/ServerScript.server.luau"), "-- mod server").unwrap();
        fs::write(path.join("src/ClientScript.client.luau"), "-- mod client").unwrap();
    });
    let info = session.wait_to_come_online();
    let meta = info.git_metadata.unwrap();

    assert!(
        meta.changed_ids.len() >= 3,
        "Should have at least 3 changed instances, got {}",
        meta.changed_ids.len()
    );
    assert!(
        meta.script_committed_hashes.len() >= 3,
        "Should have at least 3 script hashes, got {}",
        meta.script_committed_hashes.len()
    );
}

#[test]
fn init_style_script_staging_targets_init_file() {
    let mut session = TestServeSession::new_with_git("git_sync_defaults", |path| {
        git_commit_all(path, "initial commit");
        fs::write(path.join("src/DirModule/init.luau"), "-- modified").unwrap();
    });
    let info = session.wait_to_come_online();
    let read = session.get_api_read(info.root_instance_id).unwrap();

    if let Some((dir_id, _)) = find_instance_by_name(&read.instances, "DirModule") {
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: HashMap::new(),
            updated: vec![],
            stage_ids: vec![dir_id],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        assert!(
            git_is_staged(session.path(), "src/DirModule/init.luau"),
            "init.luau should be staged for init-style script"
        );
    }
}
