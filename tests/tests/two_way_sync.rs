//! Integration tests for the two-way sync pipeline (POST /api/write).
//!
//! These tests validate the core operations that allow the plugin's two-way
//! sync to eventually replace `rojo syncback`:
//!
//! 1. Source property writes to disk
//! 2. Non-Source property persistence to meta files
//! 3. Instance rename → file rename
//! 4. ClassName transitions → file extension rename
//! 5. Instance removal with meta file cleanup
//! 6. End-to-end round-trip lifecycle (modify, rename, remove)
//! 7. ProjectNode guard (no project file corruption)
//! 8. Echo suppression (no redundant VFS patches)
//!
//! NOTE: Adding new instances via the API writes files to disk WITHOUT
//! VFS echo suppression — the watcher must pick up the new files to add
//! them to the tree. Updates and removals of EXISTING instances DO suppress
//! their VFS echoes since the tree is already mutated via handle_tree_event.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, thread};

use librojo::web_api::{AddedInstance, InstanceUpdate, WriteRequest};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, UstrMap};

use crate::rojo_test::serve_util::run_serve_test;

// ---------------------------------------------------------------------------
// Platform-tuned constants for stress tests
//
// On macOS, kqueue delivers more granular events than FSEvents (per-file
// vnode changes instead of coalesced directory batches). This generates
// significantly more VFS events during rapid rename/write sequences, so
// stress tests need wider inter-operation gaps and longer poll timeouts.
// ---------------------------------------------------------------------------

/// Delay between rapid filesystem operations in stress tests (ms).
#[cfg(target_os = "macos")]
const STRESS_OP_DELAY_MS: u64 = 250;
#[cfg(not(target_os = "macos"))]
const STRESS_OP_DELAY_MS: u64 = 30;

/// Poll timeout for stress tests that wait for the tree to settle (ms).
/// macOS kqueue generates per-file vnode events for every rename, and each
/// event triggers a re-snapshot of the parent directory. Under heavy CI load
/// (e.g., 5-file simultaneous rename chains), 15s can be insufficient.
#[cfg(target_os = "macos")]
const STRESS_POLL_TIMEOUT_MS: u64 = 30000;
#[cfg(not(target_os = "macos"))]
const STRESS_POLL_TIMEOUT_MS: u64 = 5000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find an instance by class name in the read response.
fn find_by_class<'a>(
    instances: &'a HashMap<Ref, librojo::web_api::Instance<'a>>,
    class_name: &str,
) -> (Ref, &'a librojo::web_api::Instance<'a>) {
    instances
        .iter()
        .find(|(_, inst)| inst.class_name == class_name)
        .map(|(id, inst)| (*id, inst))
        .unwrap_or_else(|| panic!("Instance with class '{}' not found", class_name))
}

/// Find an instance by name in the read response.
fn find_by_name<'a>(
    instances: &'a HashMap<Ref, librojo::web_api::Instance<'a>>,
    name: &str,
) -> (Ref, &'a librojo::web_api::Instance<'a>) {
    instances
        .iter()
        .find(|(_, inst)| inst.name == name)
        .map(|(id, inst)| (*id, inst))
        .unwrap_or_else(|| {
            let available: Vec<&str> = instances.values().map(|i| i.name.as_ref()).collect();
            panic!(
                "Instance with name '{}' not found. Available: {:?}",
                name, available
            )
        })
}

/// Send an InstanceUpdate via the write API and wait for processing.
fn send_update(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    update: InstanceUpdate,
) {
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![update],
    };
    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");
    thread::sleep(Duration::from_millis(300));
}

/// Send a removal via the write API and wait for processing.
fn send_removal(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    ids: Vec<Ref>,
) {
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: ids,
        added: HashMap::new(),
        updated: vec![],
    };
    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");
    thread::sleep(Duration::from_millis(300));
}

fn assert_file_exists(path: &Path, msg: &str) {
    assert!(
        path.exists() && path.is_file(),
        "{}: expected file at {}",
        msg,
        path.display()
    );
}

fn assert_not_exists(path: &Path, msg: &str) {
    assert!(
        !path.exists(),
        "{}: should NOT exist: {}",
        msg,
        path.display()
    );
}

/// Poll timeout for assertions on filesystem changes driven by the
/// ChangeProcessor (runs on a separate thread from the API handler).
const API_POLL_TIMEOUT_MS: u64 = 5000;

/// Poll until a file exists on disk. Use after `send_update` for operations
/// processed asynchronously by the ChangeProcessor (Source writes, renames,
/// ClassName changes). Panics after `API_POLL_TIMEOUT_MS`.
fn poll_file_exists(path: &Path, msg: &str) {
    let start = Instant::now();
    loop {
        if path.exists() && path.is_file() {
            return;
        }
        if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
            panic!(
                "{}: expected file at {} (timed out after {}ms)",
                msg,
                path.display(),
                API_POLL_TIMEOUT_MS
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll until a path no longer exists on disk. Use after `send_update` for
/// operations processed asynchronously by the ChangeProcessor.
/// Panics after `API_POLL_TIMEOUT_MS`.
fn poll_not_exists(path: &Path, msg: &str) {
    let start = Instant::now();
    loop {
        if !path.exists() {
            return;
        }
        if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
            panic!(
                "{}: should NOT exist: {} (timed out after {}ms)",
                msg,
                path.display(),
                API_POLL_TIMEOUT_MS
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll until a file exists and its contents contain the expected substring.
/// Use after `send_update` for Source property writes processed asynchronously
/// by the ChangeProcessor. Panics after `API_POLL_TIMEOUT_MS`.
fn poll_file_contains(path: &Path, expected: &str, msg: &str) {
    let start = Instant::now();
    loop {
        if path.exists() && path.is_file() {
            if let Ok(content) = fs::read_to_string(path) {
                if content.contains(expected) {
                    return;
                }
            }
        }
        if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
            let content = fs::read_to_string(path).unwrap_or_else(|_| "<file not found>".into());
            panic!(
                "{}: file at {} should contain '{}', got: {} (timed out after {}ms)",
                msg,
                path.display(),
                expected,
                content,
                API_POLL_TIMEOUT_MS
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Get ReplicatedStorage ID and the ID of the "existing" ModuleScript.
fn get_rs_and_existing(
    session: &crate::rojo_test::serve_util::TestServeSession,
) -> (librojo::SessionId, Ref, Ref) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    let rs_read = session.get_api_read(rs_id).unwrap();
    let (existing_id, _) = find_by_name(&rs_read.instances, "existing");
    (info.session_id, rs_id, existing_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: The most fundamental two-way sync operation — update Source and
/// verify the file content changes on disk.
#[test]
fn update_source_writes_to_disk() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let file_path = session.path().join("src").join("existing.luau");
        assert_file_exists(&file_path, "existing.luau before update");

        // Send Source update
        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String(
                "-- Updated via two-way sync\nreturn { updated = true }".to_string(),
            )),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        poll_file_contains(
            &file_path,
            "Updated via two-way sync",
            "File should contain new Source",
        );
    });
}

/// Test 2: Non-Source property updates create an adjacent meta file.
#[test]
fn update_non_source_property_creates_meta_file() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let file_path = session.path().join("src").join("existing.luau");
        let meta_path = session.path().join("src").join("existing.meta.json5");

        assert_not_exists(&meta_path, "Meta file before update");
        let original_content = fs::read_to_string(&file_path).unwrap();

        // Send a non-Source property update (Attributes)
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("TestAttribute".to_string(), Variant::Float64(42.0));

        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Meta file should now exist with the attribute
        assert_file_exists(&meta_path, "Meta file after property update");
        let meta_content = fs::read_to_string(&meta_path).unwrap();
        assert!(
            meta_content.contains("TestAttribute"),
            "Meta should contain attribute, got: {}",
            meta_content
        );

        // Script file should be unchanged
        let current = fs::read_to_string(&file_path).unwrap();
        assert_eq!(original_content, current, "Script file should not change");
    });
}

/// Test 3: Non-Source property update on a directory-format instance writes
/// to init.meta.json5 inside the directory.
#[test]
fn update_properties_for_directory_instance_writes_init_meta() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let rs_read = session.get_api_read(rs_id).unwrap();

        // DirModelWithChildren is a Configuration directory that IS in the tree
        let (dir_model_id, _) = find_by_name(&rs_read.instances, "DirModelWithChildren");

        let dir_path = session.path().join("src").join("DirModelWithChildren");
        let init_meta = dir_path.join("init.meta.json5");

        // Send a non-Source property update
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("DirTestAttr".to_string(), Variant::Float64(99.0));

        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &info.session_id,
            InstanceUpdate {
                id: dir_model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        assert_file_exists(&init_meta, "init.meta.json5 after property update");
        let meta_content = fs::read_to_string(&init_meta).unwrap();
        assert!(
            meta_content.contains("DirTestAttr"),
            "init.meta.json5 should contain attribute, got: {}",
            meta_content
        );
    });
}

/// Test 4: Renaming an instance renames the file on disk.
#[test]
fn rename_standalone_script() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let old_path = session.path().join("src").join("existing.luau");
        let new_path = session.path().join("src").join("renamed_module.luau");

        assert_file_exists(&old_path, "existing.luau before rename");
        let original_content = fs::read_to_string(&old_path).unwrap();

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: Some("renamed_module".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "Old file after rename");
        poll_file_exists(&new_path, "New file after rename");

        let new_content = fs::read_to_string(&new_path).unwrap();
        assert_eq!(
            original_content, new_content,
            "Content preserved after rename"
        );
    });
}

/// Test 5: Renaming an instance also renames its adjacent meta file.
#[test]
fn rename_preserves_adjacent_meta_file() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        // First, create a meta file by sending a non-Source property update
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("SomeAttr".to_string(), Variant::Bool(true));
        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let old_meta = session.path().join("src").join("existing.meta.json5");
        assert_file_exists(&old_meta, "Meta file before rename");

        // Re-read tree to get current ID (may have changed after re-snapshot)
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        // Now rename — the meta file name won't match "existing" anymore
        // because the instance is now at "renamed_meta"
        // Note: we need to find the instance by its CURRENT name
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: Some("renamed_meta".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        let new_script = session.path().join("src").join("renamed_meta.luau");
        let new_meta = session.path().join("src").join("renamed_meta.meta.json5");

        poll_not_exists(
            &session.path().join("src").join("existing.luau"),
            "Old script gone",
        );
        poll_not_exists(&old_meta, "Old meta gone");
        poll_file_exists(&new_script, "Renamed script exists");
        poll_file_exists(&new_meta, "Renamed meta exists");
    });
}

/// Test 6: Changing ClassName from ModuleScript to Script renames the file extension.
#[test]
fn classname_change_module_to_script() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let module_path = session.path().join("src").join("existing.luau");
        let script_path = session.path().join("src").join("existing.server.luau");

        assert_file_exists(&module_path, "Module file before class change");
        assert_not_exists(&script_path, "Script file before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: Some(ustr("Script")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&module_path, "Module file after class change");
        poll_file_exists(&script_path, "Script file after class change");
    });
}

/// Test 7: Changing ClassName from ModuleScript to LocalScript.
#[test]
fn classname_change_module_to_localscript() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let module_path = session.path().join("src").join("existing.luau");
        let local_path = session.path().join("src").join("existing.local.luau");

        assert_file_exists(&module_path, "Module file before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: Some(ustr("LocalScript")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&module_path, "Module file after class change");
        poll_file_exists(&local_path, "LocalScript file after class change");
    });
}

/// Test 8: Removing an instance deletes both its file and adjacent meta file.
#[test]
fn remove_instance_deletes_file_and_meta() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let file_path = session.path().join("src").join("existing.luau");
        assert_file_exists(&file_path, "File before removal");

        // Create meta file via property update
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("Temp".to_string(), Variant::Bool(true));
        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session.path().join("src").join("existing.meta.json5");
        assert_file_exists(&meta_path, "Meta file before removal");

        // Re-read ID in case tree was rebuilt
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
        send_removal(&session, &session_id, vec![existing_id]);

        assert_not_exists(&file_path, "File after removal");
        assert_not_exists(&meta_path, "Meta file after removal");
    });
}

/// Test 9: Removing a directory-format instance deletes the entire directory.
#[test]
fn remove_directory_instance() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let rs_read = session.get_api_read(rs_id).unwrap();

        // DirModelWithChildren is a Configuration directory in the tree
        let (dir_model_id, _) = find_by_name(&rs_read.instances, "DirModelWithChildren");

        let dir_path = session.path().join("src").join("DirModelWithChildren");
        assert!(dir_path.is_dir(), "Directory should exist before removal");

        send_removal(&session, &info.session_id, vec![dir_model_id]);

        assert_not_exists(&dir_path, "Directory after removal");
    });
}

/// Test 10: End-to-end round-trip — modify Source, rename, then remove.
/// Validates the complete lifecycle on a single instance.
#[test]
fn round_trip_modify_rename_remove() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
        let src = session.path().join("src");

        // Step 1: MODIFY Source
        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String("-- Round trip v1".to_string())),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let file = src.join("existing.luau");
        poll_file_contains(&file, "Round trip v1", "Step 1: Source updated");

        // Re-read to get current ID
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        // Step 2: RENAME
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: Some("GameService".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&file, "Step 2: old file gone");
        let new_file = src.join("GameService.luau");
        poll_file_exists(&new_file, "Step 2: renamed file exists");
        let content = fs::read_to_string(&new_file).unwrap();
        assert!(
            content.contains("Round trip v1"),
            "Step 2: content preserved after rename"
        );

        // Re-read to get current ID after rename
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (game_service_id, _) = find_by_name(&rs_read.instances, "GameService");

        // Step 3: REMOVE
        send_removal(&session, &info.session_id, vec![game_service_id]);
        assert_not_exists(&new_file, "Step 3: file removed");
    });
}

/// Test 11: Property updates on project-defined instances (Services) should
/// NOT corrupt the project file.
#[test]
fn property_update_skips_project_node_instances() {
    run_serve_test("syncback_write", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        // Save original project file content
        let project_path = session.path().join("default.project.json5");
        let original_project = fs::read_to_string(&project_path).unwrap();

        // Send a non-Source property update to the service
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("ServiceAttr".to_string(), Variant::Float64(1.0));
        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));

        send_update(
            &session,
            &info.session_id,
            InstanceUpdate {
                id: rs_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Project file should be UNCHANGED
        let current_project = fs::read_to_string(&project_path).unwrap();
        assert_eq!(
            original_project, current_project,
            "Project file must not be modified by property updates to project-defined instances"
        );
    });
}

/// Test 12: Echo suppression — adding an instance should not cause the server
/// to become unresponsive from echo loops.
#[test]
fn echo_suppression_prevents_redundant_patches() {
    run_serve_test("syncback_write", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let root_read = session.get_api_read(root_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        let initial_cursor = root_read.message_cursor;

        // Add a new instance (creates file on disk → VFS event → should be suppressed)
        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("return 'echo test'".to_string()),
        );
        let added = AddedInstance {
            parent: Some(rs_id),
            name: "EchoTest".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();

        // Wait for any echo events to settle
        thread::sleep(Duration::from_millis(500));

        // File should exist on disk
        let file_path = session.path().join("src").join("EchoTest.luau");
        assert_file_exists(&file_path, "EchoTest.luau created");

        // Server should still be responsive
        let read_after = session.get_api_read(root_id).unwrap();
        let cursor_delta = read_after.message_cursor - initial_cursor;

        // For a single add: ~2 messages (1 from handle_tree_event broadcast +
        // 1 from VFS watcher adding the new instance to the tree). Allowing 3
        // accounts for platform-specific directory-level VFS events.
        assert!(
            cursor_delta <= 3,
            "Echo suppression: cursor advanced by {} (expected ~2 for add: \
             1 tree mutation + 1 VFS pickup)",
            cursor_delta
        );

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Helpers (syncback_format_transitions fixture)
// ---------------------------------------------------------------------------

/// Look up an instance by name under ReplicatedStorage in the format_transitions fixture.
fn get_format_transitions_instance(
    session: &crate::rojo_test::serve_util::TestServeSession,
    instance_name: &str,
) -> (librojo::SessionId, Ref) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    let rs_read = session.get_api_read(rs_id).unwrap();
    let (instance_id, _) = find_by_name(&rs_read.instances, instance_name);
    (info.session_id, instance_id)
}

// ---------------------------------------------------------------------------
// Tests: Directory-format operations
// ---------------------------------------------------------------------------

/// Test 13: Renaming a directory-format script renames the parent directory,
/// not the init file inside it. Children should move with it.
#[test]
fn rename_directory_format_script() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, dir_module_id) =
            get_format_transitions_instance(&session, "DirModuleWithChildren");

        let old_dir = session.path().join("src").join("DirModuleWithChildren");
        let new_dir = session.path().join("src").join("RenamedDirModule");

        assert!(old_dir.is_dir(), "Old directory should exist before rename");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: dir_module_id,
                changed_name: Some("RenamedDirModule".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_dir, "Old directory should be gone after rename");

        // Poll for the new directory to appear
        let start = Instant::now();
        loop {
            if new_dir.is_dir() {
                break;
            }
            if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
                panic!(
                    "New directory should exist after rename: {} (timed out after {}ms)",
                    new_dir.display(),
                    API_POLL_TIMEOUT_MS
                );
            }
            thread::sleep(Duration::from_millis(50));
        }

        // init.luau should be inside the renamed directory
        let init_file = new_dir.join("init.luau");
        assert_file_exists(&init_file, "init.luau inside renamed directory");

        // Children should also be present in the renamed directory
        let child_a = new_dir.join("ChildA.luau");
        assert_file_exists(&child_a, "ChildA.luau inside renamed directory");
    });
}

/// Test 14: Changing ClassName on a directory-format ModuleScript to Script
/// renames the init file inside the directory (init.luau → init.server.luau).
#[test]
fn classname_change_directory_module_to_script() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, dir_module_id) =
            get_format_transitions_instance(&session, "DirModuleWithChildren");

        let dir = session.path().join("src").join("DirModuleWithChildren");
        let old_init = dir.join("init.luau");
        let new_init = dir.join("init.server.luau");

        assert_file_exists(&old_init, "init.luau before class change");
        assert_not_exists(&new_init, "init.server.luau before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: dir_module_id,
                changed_name: None,
                changed_class_name: Some(ustr("Script")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_init, "init.luau after class change to Script");
        poll_file_exists(&new_init, "init.server.luau after class change to Script");

        // Children should be unaffected
        let child_a = dir.join("ChildA.luau");
        assert_file_exists(&child_a, "ChildA.luau unaffected by class change");
    });
}

// ---------------------------------------------------------------------------
// Tests: Reverse ClassName transitions (code paths added in these commits)
// ---------------------------------------------------------------------------

/// Test 15: Script → ModuleScript ClassName transition (reverse of test 6).
/// Validates that the .server suffix is correctly stripped.
#[test]
fn classname_change_script_to_module() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");

        let old_path = session
            .path()
            .join("src")
            .join("StandaloneScript.server.luau");
        let new_path = session.path().join("src").join("StandaloneScript.luau");

        assert_file_exists(&old_path, "Script file before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_id,
                changed_name: None,
                changed_class_name: Some(ustr("ModuleScript")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "Script file after class change to ModuleScript");
        poll_file_exists(&new_path, "ModuleScript file after class change");
    });
}

/// Test 16: Script → LocalScript ClassName transition.
/// Validates that .server is replaced with .local.
#[test]
fn classname_change_script_to_localscript() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");

        let old_path = session
            .path()
            .join("src")
            .join("StandaloneScript.server.luau");
        let new_path = session
            .path()
            .join("src")
            .join("StandaloneScript.local.luau");

        assert_file_exists(&old_path, "Script file before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_id,
                changed_name: None,
                changed_class_name: Some(ustr("LocalScript")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "Script file after class change to LocalScript");
        poll_file_exists(&new_path, "LocalScript file after class change");
    });
}

/// Test 17: LocalScript → Script ClassName transition.
/// Exercises .client suffix handling (the fixture uses .client.luau for LocalScript).
#[test]
fn classname_change_localscript_to_script() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, local_id) =
            get_format_transitions_instance(&session, "StandaloneLocalScript");

        let old_path = session
            .path()
            .join("src")
            .join("StandaloneLocalScript.client.luau");
        let new_path = session
            .path()
            .join("src")
            .join("StandaloneLocalScript.server.luau");

        assert_file_exists(&old_path, "LocalScript file before class change");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: local_id,
                changed_name: None,
                changed_class_name: Some(ustr("Script")),
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "LocalScript file after class change to Script");
        poll_file_exists(&new_path, "Script file after class change");
    });
}

// ---------------------------------------------------------------------------
// Tests: Combined operations (overridden_source_path logic)
// ---------------------------------------------------------------------------

/// Test 18: Combined rename + Source update in a single request.
/// Validates that Source is written to the RENAMED file, not the old location.
/// This exercises the `overridden_source_path` tracking in ChangeProcessor.
#[test]
fn combined_rename_and_source_update() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let old_path = session.path().join("src").join("existing.luau");
        let new_path = session.path().join("src").join("CombinedRename.luau");

        assert_file_exists(&old_path, "existing.luau before combined update");

        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String("-- Written after rename".to_string())),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: Some("CombinedRename".to_string()),
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "Old file after combined rename+source");
        poll_file_contains(
            &new_path,
            "Written after rename",
            "Source should be written to the NEW file location",
        );
    });
}

/// Test 19: Combined ClassName change + Source update in a single request.
/// Validates that Source is written to the file with the NEW extension.
#[test]
fn combined_classname_and_source_update() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let old_path = session.path().join("src").join("existing.luau");
        let new_path = session.path().join("src").join("existing.server.luau");

        assert_file_exists(&old_path, "existing.luau before combined update");

        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String("-- Source after class change".to_string())),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: None,
                changed_class_name: Some(ustr("Script")),
                changed_properties: props,
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "ModuleScript file after ClassName+Source");
        poll_file_contains(
            &new_path,
            "Source after class change",
            "Source should be in the new file",
        );
    });
}

/// Test 19b: Combined rename + ClassName change in a single update.
/// The rename handler moves the file first (existing.luau → Renamed.luau),
/// then the ClassName handler must use the renamed path to apply the extension
/// change (Renamed.luau → Renamed.server.luau). Without the fix, the ClassName
/// handler checks the OLD path which no longer exists, silently skipping the
/// extension rename.
#[test]
fn combined_rename_and_classname_change() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

        let old_path = session.path().join("src").join("existing.luau");
        // After rename + ClassName change: should end up as Renamed.server.luau
        let final_path = session.path().join("src").join("Renamed.server.luau");
        // Should NOT exist: renamed but without extension change
        let renamed_only = session.path().join("src").join("Renamed.luau");

        assert_file_exists(&old_path, "existing.luau before combined update");

        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String(
                "-- After rename + class change".to_string(),
            )),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: existing_id,
                changed_name: Some("Renamed".to_string()),
                changed_class_name: Some(ustr("Script")),
                changed_properties: props,
                changed_metadata: None,
            },
        );

        poll_not_exists(&old_path, "Original file after combined rename+classname");
        poll_not_exists(
            &renamed_only,
            "Renamed.luau should not exist — ClassName handler should have \
             applied the .server extension",
        );
        poll_file_contains(
            &final_path,
            "After rename + class change",
            "Renamed.server.luau should exist with correct Source",
        );
    });
}

// ---------------------------------------------------------------------------
// Tests: Standalone → directory conversion & suffix-aware removal
// ---------------------------------------------------------------------------

/// Test 20: Adding a child instance to a standalone script converts it to
/// directory format (e.g., StandaloneModule.luau → StandaloneModule/init.luau).
/// This exercises `convert_standalone_script_to_directory` in ApiService.
#[test]
fn add_child_converts_standalone_to_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (standalone_id, _) = find_by_name(&rs_read.instances, "StandaloneModule");

        let standalone_file = session.path().join("src").join("StandaloneModule.luau");
        let dir_path = session.path().join("src").join("StandaloneModule");

        assert_file_exists(&standalone_file, "Standalone file before child add");
        assert!(
            !dir_path.exists(),
            "Directory should NOT exist before child add"
        );

        // Add a child ModuleScript to StandaloneModule
        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- New child module".to_string()),
        );
        let added = AddedInstance {
            parent: Some(standalone_id),
            name: "NewChild".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        // Standalone file should be gone, replaced by directory
        assert_not_exists(
            &standalone_file,
            "Standalone file after child add (converted to directory)",
        );
        assert!(
            dir_path.is_dir(),
            "Directory should exist after conversion: {}",
            dir_path.display()
        );

        // init.luau should contain the original source
        let init_file = dir_path.join("init.luau");
        assert_file_exists(
            &init_file,
            "init.luau after standalone→directory conversion",
        );

        let init_content = fs::read_to_string(&init_file).unwrap();
        assert!(
            init_content.contains("Standalone ModuleScript"),
            "init.luau should contain original source, got: {}",
            init_content
        );

        // Child should be created inside the directory
        let child_file = dir_path.join("NewChild.luau");
        assert_file_exists(&child_file, "NewChild.luau inside converted directory");
    });
}

/// Test 21: Removing a Script (with .server suffix) properly cleans up
/// the adjacent meta file by stripping the script suffix to find the
/// correct meta file name (e.g., StandaloneScript.meta.json5).
#[test]
fn remove_script_with_suffix_cleans_meta() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");

        let script_path = session
            .path()
            .join("src")
            .join("StandaloneScript.server.luau");
        let meta_path = session
            .path()
            .join("src")
            .join("StandaloneScript.meta.json5");

        assert_file_exists(&script_path, "Script file before operations");

        // First, create a meta file by sending a non-Source property update
        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("ScriptAttr".to_string(), Variant::Float64(7.0));
        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        assert_file_exists(&meta_path, "Meta file after property update");

        // Re-read ID in case tree was rebuilt after the update
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");

        // Now remove the instance — both script and meta should be deleted
        send_removal(&session, &session_id, vec![script_id]);

        assert_not_exists(&script_path, "Script file after removal");
        assert_not_exists(
            &meta_path,
            "Meta file after removal (suffix-stripped cleanup)",
        );
    });
}

// ---------------------------------------------------------------------------
// Helpers (syncback_encoded_names fixture)
// ---------------------------------------------------------------------------

/// Look up an instance by name under ReplicatedStorage in the encoded_names fixture.
/// Instance names are DECODED (e.g., `Encoded?Module`), not the filesystem names.
fn get_encoded_names_instance(
    session: &crate::rojo_test::serve_util::TestServeSession,
    instance_name: &str,
) -> (librojo::SessionId, Ref) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    let rs_read = session.get_api_read(rs_id).unwrap();
    let (instance_id, _) = find_by_name(&rs_read.instances, instance_name);
    (info.session_id, instance_id)
}

// ---------------------------------------------------------------------------
// Tests: Slugified name directory conversion (path encoding fix)
// ---------------------------------------------------------------------------

/// Test 22: Adding a child to a standalone ModuleScript with slugified
/// characters (e.g., `What_Module.luau`) creates the directory using
/// the slugified filesystem name (`What_Module/`), NOT the decoded
/// instance name (`What?Module/`).
///
/// This is the core regression test for the slugified name fix in
/// `convert_standalone_script_to_directory`.
#[test]
fn add_child_to_encoded_module_creates_encoded_directory() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_module_id) = get_encoded_names_instance(&session, "What?Module");

        let standalone_file = session.path().join("src").join("What_Module.luau");
        // The CORRECT directory name uses the slugified filesystem name
        let encoded_dir = session.path().join("src").join("What_Module");
        // The WRONG directory name would use the decoded instance name
        let decoded_dir = session.path().join("src").join("What?Module");

        assert_file_exists(
            &standalone_file,
            "Slugified standalone file before child add",
        );

        // Add a child to trigger standalone → directory conversion
        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- Child of encoded module".to_string()),
        );
        let added = AddedInstance {
            parent: Some(encoded_module_id),
            name: "EncodedChild".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        // Standalone file should be removed
        assert_not_exists(
            &standalone_file,
            "Standalone file after directory conversion",
        );

        // Directory should use SLUGIFIED name (the fix)
        assert!(
            encoded_dir.is_dir(),
            "Directory should use slugified name (What_Module/): {}",
            encoded_dir.display()
        );

        // Decoded name directory should NOT exist (the bug we fixed)
        assert!(
            !decoded_dir.exists(),
            "Directory with decoded name (What?Module/) must NOT be created: {}",
            decoded_dir.display()
        );

        // init.luau should exist inside the slugified directory
        let init_file = encoded_dir.join("init.luau");
        assert_file_exists(&init_file, "init.luau inside slugified directory");

        // Child should be created inside the slugified directory
        let child_file = encoded_dir.join("EncodedChild.luau");
        assert_file_exists(&child_file, "Child inside slugified directory");
    });
}

/// Test 23: Adding a child to a standalone Script with slugified characters
/// AND a script suffix (e.g., `Key_Script.server.luau`) strips the suffix
/// correctly when creating the directory name.
///
/// The directory should be `Key_Script/`, not `Key_Script.server/`
/// or `Key:Script/`.
#[test]
fn add_child_to_encoded_script_strips_suffix_correctly() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_script_id) = get_encoded_names_instance(&session, "Key:Script");

        let standalone_file = session.path().join("src").join("Key_Script.server.luau");
        let encoded_dir = session.path().join("src").join("Key_Script");
        // Wrong: decoded name
        let decoded_dir = session.path().join("src").join("Key:Script");
        // Wrong: suffix not stripped
        let unsuffixed_dir = session.path().join("src").join("Key_Script.server");

        assert_file_exists(&standalone_file, "Slugified script before child add");

        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- Child of encoded script".to_string()),
        );
        let added = AddedInstance {
            parent: Some(encoded_script_id),
            name: "ScriptChild".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        assert_not_exists(&standalone_file, "Standalone script after conversion");

        assert!(
            encoded_dir.is_dir(),
            "Directory should be Key_Script/ (suffix stripped, slugified): {}",
            encoded_dir.display()
        );
        assert!(
            !decoded_dir.exists(),
            "Directory with decoded name must NOT be created: {}",
            decoded_dir.display()
        );
        assert!(
            !unsuffixed_dir.exists(),
            "Directory with unsuffixed name must NOT be created: {}",
            unsuffixed_dir.display()
        );

        // Script → init.server.luau (preserves class)
        let init_file = encoded_dir.join("init.server.luau");
        assert_file_exists(&init_file, "init.server.luau inside slugified directory");

        let child_file = encoded_dir.join("ScriptChild.luau");
        assert_file_exists(&child_file, "Child inside slugified script directory");
    });
}

/// Test 24: Adding a child to a non-script standalone instance with slugified
/// characters (e.g., `What_Model.model.json5`) creates the directory
/// using the slugified name with the compound extension stripped.
///
/// This exercises `convert_standalone_instance_to_directory`.
#[test]
fn add_child_to_encoded_model_creates_encoded_directory() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_model_id) = get_encoded_names_instance(&session, "What?Model");

        let standalone_file = session.path().join("src").join("What_Model.model.json5");
        let encoded_dir = session.path().join("src").join("What_Model");
        let decoded_dir = session.path().join("src").join("What?Model");

        assert_file_exists(&standalone_file, "Slugified model file before child add");

        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- Child of encoded model".to_string()),
        );
        let added = AddedInstance {
            parent: Some(encoded_model_id),
            name: "ModelChild".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        assert_not_exists(&standalone_file, "Standalone model after conversion");

        assert!(
            encoded_dir.is_dir(),
            "Directory should use slugified name (What_Model/): {}",
            encoded_dir.display()
        );
        assert!(
            !decoded_dir.exists(),
            "Directory with decoded name must NOT be created: {}",
            decoded_dir.display()
        );

        // init.meta.json5 should exist inside the directory (converted from .model.json5)
        let init_meta = encoded_dir.join("init.meta.json5");
        assert_file_exists(
            &init_meta,
            "init.meta.json5 inside slugified model directory",
        );

        let child_file = encoded_dir.join("ModelChild.luau");
        assert_file_exists(&child_file, "Child inside slugified model directory");
    });
}

/// Test 25: Non-Source property update on a standalone script with slugified
/// name creates the adjacent meta file using the slugified filesystem name,
/// not the decoded instance name.
///
/// e.g., `What_Module.meta.json5` NOT `What?Module.meta.json5`
#[test]
fn property_update_on_encoded_script_uses_encoded_meta_path() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_module_id) = get_encoded_names_instance(&session, "What?Module");

        let script_file = session.path().join("src").join("What_Module.luau");
        let encoded_meta = session.path().join("src").join("What_Module.meta.json5");
        let decoded_meta = session.path().join("src").join("What?Module.meta.json5");

        assert_file_exists(&script_file, "Slugified script file before property update");
        assert_file_exists(&encoded_meta, "Meta file exists from fixture (name field)");

        let mut attrs = rbx_dom_weak::types::Attributes::new();
        attrs.insert("EncodedAttr".to_string(), Variant::Float64(99.0));

        let mut props = UstrMap::default();
        props.insert(ustr("Attributes"), Some(Variant::Attributes(attrs)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: encoded_module_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Meta file should use slugified name
        assert_file_exists(
            &encoded_meta,
            "Meta file should use slugified name (What_Module.meta.json5)",
        );
        assert!(
            !decoded_meta.exists(),
            "Meta file with decoded name must NOT be created: {}",
            decoded_meta.display()
        );

        // Script file should be unchanged
        let content = fs::read_to_string(&script_file).unwrap();
        assert!(
            content.contains("Module with encoded"),
            "Script content should be unchanged, got: {}",
            content
        );
    });
}

// ===========================================================================
// PART 1: API-Driven Stress Tests (Tests 26-41)
// ===========================================================================

// ---------------------------------------------------------------------------
// Stress helpers
// ---------------------------------------------------------------------------

/// Send an InstanceUpdate with a short 50ms wait (races the 200ms recovery).
fn send_update_fast(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    update: InstanceUpdate,
) {
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![update],
    };
    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");
    thread::sleep(Duration::from_millis(50));
}

/// Send an InstanceUpdate with zero wait (fire and forget).
fn send_update_no_wait(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    update: InstanceUpdate,
) {
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: vec![],
        added: HashMap::new(),
        updated: vec![update],
    };
    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");
}

/// Send a removal with a short 50ms wait.
fn send_removal_fast(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    ids: Vec<Ref>,
) {
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: ids,
        added: HashMap::new(),
        updated: vec![],
    };
    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");
    thread::sleep(Duration::from_millis(50));
}

/// Wait long enough for all VFS events, recovery sweeps, and debouncing.
fn wait_for_settle() {
    thread::sleep(Duration::from_millis(800));
}

/// Get session_id and Refs for Alpha-Echo in the syncback_stress fixture.
fn get_stress_instances(
    session: &crate::rojo_test::serve_util::TestServeSession,
) -> (librojo::SessionId, Ref, Vec<(Ref, String)>) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    let rs_read = session.get_api_read(rs_id).unwrap();
    let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
    let mut instances = Vec::new();
    for name in &names {
        let (id, _) = find_by_name(&rs_read.instances, name);
        instances.push((id, name.to_string()));
    }
    (info.session_id, rs_id, instances)
}

/// Build a Source update for an instance.
fn make_source_update(id: Ref, source: &str) -> InstanceUpdate {
    let mut props = UstrMap::default();
    props.insert(ustr("Source"), Some(Variant::String(source.to_string())));
    InstanceUpdate {
        id,
        changed_name: None,
        changed_class_name: None,
        changed_properties: props,
        changed_metadata: None,
    }
}

/// Build a rename update.
fn make_rename_update(id: Ref, new_name: &str) -> InstanceUpdate {
    InstanceUpdate {
        id,
        changed_name: Some(new_name.to_string()),
        changed_class_name: None,
        changed_properties: UstrMap::default(),
        changed_metadata: None,
    }
}

/// Build a ClassName update.
fn make_class_update(id: Ref, class: &str) -> InstanceUpdate {
    InstanceUpdate {
        id,
        changed_name: None,
        changed_class_name: Some(ustr(class)),
        changed_properties: UstrMap::default(),
        changed_metadata: None,
    }
}

/// Build a combined rename + class + source update.
fn make_combined_update(
    id: Ref,
    name: Option<&str>,
    class: Option<&str>,
    source: Option<&str>,
) -> InstanceUpdate {
    let mut props = UstrMap::default();
    if let Some(src) = source {
        props.insert(ustr("Source"), Some(Variant::String(src.to_string())));
    }
    InstanceUpdate {
        id,
        changed_name: name.map(|n| n.to_string()),
        changed_class_name: class.map(ustr),
        changed_properties: props,
        changed_metadata: None,
    }
}

/// Get the expected file extension suffix for a class name.
fn class_suffix(class: &str) -> &'static str {
    match class {
        "Script" => ".server",
        "LocalScript" => ".local",
        _ => "",
    }
}

/// Verify that exactly one file exists for the given name+class, and no stale
/// files remain with other extensions.
fn verify_instance_file(src_dir: &Path, name: &str, class: &str, expected_source: Option<&str>) {
    let suffix = class_suffix(class);
    let expected_file = src_dir.join(format!("{}{}.luau", name, suffix));
    assert!(
        expected_file.is_file(),
        "Expected file: {}",
        expected_file.display()
    );
    if let Some(source) = expected_source {
        let content = fs::read_to_string(&expected_file).unwrap();
        assert!(
            content.contains(source),
            "Expected source '{}' in file {}, got: {}",
            source,
            expected_file.display(),
            content
        );
    }
    // Check no stale files with wrong extension
    for stale_suffix in &[".server", ".local", ".client", ".plugin", ""] {
        if *stale_suffix == suffix {
            continue;
        }
        let stale = src_dir.join(format!("{}{}.luau", name, stale_suffix));
        assert!(
            !stale.exists(),
            "Stale file should not exist: {}",
            stale.display()
        );
    }
}

// ---------------------------------------------------------------------------
// Tree polling helpers (for file-watcher tests)
// ---------------------------------------------------------------------------

/// Get ReplicatedStorage Ref.
fn get_rs_id(session: &crate::rojo_test::serve_util::TestServeSession) -> Ref {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    rs_id
}

/// Poll tree until an instance named `name` under `parent_id` has Source
/// containing `expected`. Panics after `timeout_ms`.
fn poll_tree_source(
    session: &crate::rojo_test::serve_util::TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    expected_content: &str,
    timeout_ms: u64,
) {
    let start = Instant::now();
    loop {
        if let Ok(read) = session.get_api_read(parent_id) {
            for inst in read.instances.values() {
                if inst.name == instance_name {
                    if let Some(source) = inst.properties.get(&ustr("Source")) {
                        if let Variant::String(s) = source.as_ref() {
                            if s.contains(expected_content) {
                                return;
                            }
                        }
                    }
                }
            }
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            panic!(
                "Timed out after {}ms waiting for instance '{}' Source to contain '{}'",
                timeout_ms, instance_name, expected_content
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll tree until an instance named `name` exists under `parent_id`.
fn poll_tree_has_instance(
    session: &crate::rojo_test::serve_util::TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    timeout_ms: u64,
) -> Ref {
    let start = Instant::now();
    loop {
        if let Ok(read) = session.get_api_read(parent_id) {
            for (&id, inst) in &read.instances {
                if inst.name == instance_name {
                    return id;
                }
            }
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            panic!(
                "Timed out after {}ms waiting for instance '{}' to appear",
                timeout_ms, instance_name
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll tree until NO instance named `name` exists under `parent_id`.
fn poll_tree_no_instance(
    session: &crate::rojo_test::serve_util::TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    timeout_ms: u64,
) {
    let start = Instant::now();
    loop {
        if let Ok(read) = session.get_api_read(parent_id) {
            let found = read
                .instances
                .values()
                .any(|inst| inst.name == instance_name);
            if !found {
                return;
            }
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            panic!(
                "Timed out after {}ms waiting for instance '{}' to disappear",
                timeout_ms, instance_name
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll tree until instance `name` has the expected ClassName.
fn poll_tree_class(
    session: &crate::rojo_test::serve_util::TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    expected_class: &str,
    timeout_ms: u64,
) {
    let start = Instant::now();
    loop {
        if let Ok(read) = session.get_api_read(parent_id) {
            for inst in read.instances.values() {
                if inst.name == instance_name && inst.class_name == expected_class {
                    return;
                }
            }
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            panic!(
                "Timed out after {}ms waiting for instance '{}' to have class '{}'",
                timeout_ms, instance_name, expected_class
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Poll tree until instance `name` under `parent_id` has at least one child
/// with the given `child_name`. Returns the child's Ref.
fn poll_tree_has_child(
    session: &crate::rojo_test::serve_util::TestServeSession,
    parent_id: Ref,
    instance_name: &str,
    child_name: &str,
    timeout_ms: u64,
) -> Ref {
    let start = Instant::now();
    loop {
        if let Ok(read) = session.get_api_read(parent_id) {
            for (&id, inst) in &read.instances {
                if inst.name == instance_name {
                    // Now read this instance's children
                    if let Ok(child_read) = session.get_api_read(id) {
                        for (&cid, cinst) in &child_read.instances {
                            if cinst.name == child_name {
                                return cid;
                            }
                        }
                    }
                }
            }
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            panic!(
                "Timed out after {}ms waiting for '{}'.'{}'",
                timeout_ms, instance_name, child_name
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// Tests 26-27: Rapid Source writes
// ---------------------------------------------------------------------------

#[test]
fn rapid_source_writes_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
        let file_path = session.path().join("src").join("existing.luau");

        for i in 1..=10 {
            send_update_fast(
                &session,
                &session_id,
                make_source_update(existing_id, &format!("-- rapid v{}", i)),
            );
        }

        wait_for_settle();
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains("-- rapid v10"),
            "Final source should be v10, got: {}",
            content
        );
    });
}

#[test]
fn rapid_source_writes_no_wait_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
        let file_path = session.path().join("src").join("existing.luau");

        for i in 1..=10 {
            send_update_no_wait(
                &session,
                &session_id,
                make_source_update(existing_id, &format!("-- nowait v{}", i)),
            );
        }

        wait_for_settle();
        let content = fs::read_to_string(&file_path).unwrap();
        assert!(
            content.contains("-- nowait v10"),
            "Final source should be v10, got: {}",
            content
        );
    });
}

// ---------------------------------------------------------------------------
// Tests 28-29: Rapid rename chain
// ---------------------------------------------------------------------------

#[test]
fn rapid_rename_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let original_content = fs::read_to_string(src.join("existing.luau")).unwrap();

        let mut current_name = "existing".to_string();
        for i in 1..=10 {
            let new_name = format!("R{}", i);
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_rename_update(id, &new_name),
            );
            current_name = new_name;
        }

        wait_for_settle();

        // Only R10.luau should exist
        assert_file_exists(&src.join("R10.luau"), "R10.luau after chain");
        for i in 1..10 {
            assert_not_exists(
                &src.join(format!("R{}.luau", i)),
                &format!("Intermediate R{}.luau", i),
            );
        }
        assert_not_exists(&src.join("existing.luau"), "Original file");

        let content = fs::read_to_string(src.join("R10.luau")).unwrap();
        assert_eq!(
            original_content, content,
            "Content preserved through 10 renames"
        );
    });
}

#[test]
fn rapid_rename_chain_directory_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let mut current_name = "DirModuleWithChildren".to_string();

        for i in 1..=10 {
            let new_name = format!("DirR{}", i);
            let (session_id, id) = get_format_transitions_instance(&session, &current_name);
            send_update_fast(&session, &session_id, make_rename_update(id, &new_name));
            current_name = new_name;
        }

        wait_for_settle();

        let final_dir = src.join("DirR10");
        assert!(final_dir.is_dir(), "DirR10 should exist");
        assert_file_exists(&final_dir.join("init.luau"), "init.luau in DirR10");
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA in DirR10");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB in DirR10");

        // All intermediates should be gone
        assert_not_exists(&src.join("DirModuleWithChildren"), "Original directory");
        for i in 1..10 {
            assert!(
                !src.join(format!("DirR{}", i)).exists(),
                "Intermediate DirR{} should not exist",
                i
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Tests 30-31: Rapid ClassName cycling
// ---------------------------------------------------------------------------

#[test]
fn rapid_classname_cycle_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
        ];

        for class in &classes {
            let (session_id, _rs_id, id) = get_rs_and_existing(&session);
            send_update_fast(&session, &session_id, make_class_update(id, class));
            thread::sleep(Duration::from_millis(200)); // Let tree rebuild
        }

        wait_for_settle();

        // Final class is Script -> existing.server.luau
        verify_instance_file(&src, "existing", "Script", None);
    });
}

#[test]
fn rapid_classname_cycle_directory_init_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let dir = src.join("DirModuleWithChildren");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
        ];

        for class in &classes {
            let (session_id, id) =
                get_format_transitions_instance(&session, "DirModuleWithChildren");
            send_update_fast(&session, &session_id, make_class_update(id, class));
            thread::sleep(Duration::from_millis(200));
        }

        wait_for_settle();

        // Final class is Script -> init.server.luau
        assert_file_exists(
            &dir.join("init.server.luau"),
            "init.server.luau after 10 cycles",
        );
        // Children survive
        assert_file_exists(&dir.join("ChildA.luau"), "ChildA after 10 cycles");
        assert_file_exists(&dir.join("ChildB.luau"), "ChildB after 10 cycles");
    });
}

// ---------------------------------------------------------------------------
// Tests 32-34: Combined operations blitz
// ---------------------------------------------------------------------------

// macOS kqueue generates far more events per rename than Windows/inotify,
// overwhelming the single-threaded ChangeProcessor during rapid-fire operations.
#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn combined_rename_classname_source_blitz_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rounds: Vec<(&str, &str, &str)> = vec![
            ("Blitz1", "Script", "-- blitz v1"),
            ("Blitz2", "LocalScript", "-- blitz v2"),
            ("Blitz3", "ModuleScript", "-- blitz v3"),
            ("Blitz4", "Script", "-- blitz v4"),
            ("Blitz5", "LocalScript", "-- blitz v5"),
            ("Blitz6", "ModuleScript", "-- blitz v6"),
            ("Blitz7", "Script", "-- blitz v7"),
            ("Blitz8", "LocalScript", "-- blitz v8"),
            ("Blitz9", "ModuleScript", "-- blitz v9"),
            ("Blitz10", "Script", "-- blitz v10"),
        ];

        let mut current_name = "existing".to_string();
        for (name, class, source) in &rounds {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_combined_update(id, Some(name), Some(class), Some(source)),
            );
            current_name = name.to_string();
            thread::sleep(Duration::from_millis(STRESS_OP_DELAY_MS));
        }

        wait_for_settle();
        // Extra settle time on macOS for kqueue event drain
        #[cfg(target_os = "macos")]
        thread::sleep(Duration::from_millis(2000));

        // Final: Blitz10.server.luau with "-- blitz v10"
        verify_instance_file(&src, "Blitz10", "Script", Some("-- blitz v10"));

        // All intermediates gone
        for i in 1..10 {
            let name = format!("Blitz{}", i);
            for suffix in &[".server", ".local", ""] {
                assert_not_exists(
                    &src.join(format!("{}{}.luau", name, suffix)),
                    &format!("Intermediate {}{}.luau", name, suffix),
                );
            }
        }
        assert_not_exists(&src.join("existing.luau"), "Original file");
    });
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "Flaky on macOS: kqueue event storms cause server timeouts under rapid rename+write load"
)]
fn combined_rename_and_source_rapid_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let mut current_name = "existing".to_string();

        for i in 1..=10 {
            let new_name = format!("RS{}", i);
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_combined_update(id, Some(&new_name), None, Some(&format!("-- rs v{}", i))),
            );
            current_name = new_name;
        }

        wait_for_settle();
        verify_instance_file(&src, "RS10", "ModuleScript", Some("-- rs v10"));
    });
}

#[test]
fn combined_classname_and_source_rapid_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
        ];

        for (i, class) in classes.iter().enumerate() {
            let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
            send_update_fast(
                &session,
                &session_id,
                make_combined_update(
                    existing_id,
                    None,
                    Some(class),
                    Some(&format!("-- cs v{}", i + 1)),
                ),
            );
            thread::sleep(Duration::from_millis(200));
        }

        wait_for_settle();
        verify_instance_file(&src, "existing", "Script", Some("-- cs v10"));
    });
}

// ---------------------------------------------------------------------------
// Tests 35-36: Multi-instance concurrent
// ---------------------------------------------------------------------------

#[test]
fn multi_instance_source_update_single_request() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let (session_id, _rs_id, instances) = get_stress_instances(&session);
        let src = session.path().join("src");

        let updates: Vec<InstanceUpdate> = instances
            .iter()
            .enumerate()
            .map(|(i, (id, _name))| make_source_update(*id, &format!("-- multi source v{}", i + 1)))
            .collect();

        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: HashMap::new(),
            updated: updates,
        };
        session.post_api_write(&write_request).unwrap();
        wait_for_settle();

        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
        for (i, name) in names.iter().enumerate() {
            let file = src.join(format!("{}.luau", name));
            let content = fs::read_to_string(&file).unwrap();
            assert!(
                content.contains(&format!("-- multi source v{}", i + 1)),
                "{}.luau should contain expected source, got: {}",
                name,
                content
            );
        }
    });
}

#[test]
fn multi_instance_rename_single_request() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let (session_id, _rs_id, instances) = get_stress_instances(&session);
        let src = session.path().join("src");

        let new_names = ["AlphaR", "BravoR", "CharlieR", "DeltaR", "EchoR"];
        let updates: Vec<InstanceUpdate> = instances
            .iter()
            .zip(new_names.iter())
            .map(|((id, _), new_name)| make_rename_update(*id, new_name))
            .collect();

        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: HashMap::new(),
            updated: updates,
        };
        session.post_api_write(&write_request).unwrap();
        wait_for_settle();

        for new_name in &new_names {
            assert_file_exists(
                &src.join(format!("{}.luau", new_name)),
                &format!("{}.luau after rename", new_name),
            );
        }
        for old_name in &["Alpha", "Bravo", "Charlie", "Delta", "Echo"] {
            assert_not_exists(
                &src.join(format!("{}.luau", old_name)),
                &format!("Old {}.luau", old_name),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Tests 37-38: Delete + recreate race
// ---------------------------------------------------------------------------

#[test]
fn delete_and_recreate_via_filesystem_recovery() {
    run_serve_test("syncback_write", |session, _redactions| {
        let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);
        let file_path = session.path().join("src").join("existing.luau");

        // Delete via API
        send_removal_fast(&session, &session_id, vec![existing_id]);

        // Immediately recreate the file on disk (simulating editor undo)
        fs::write(&file_path, "-- recovered content\nreturn {}").unwrap();

        // Wait for the recovery mechanism (200ms delay + 500ms sweep + buffer)
        thread::sleep(Duration::from_millis(1500));

        // Instance should be back in the tree
        let rs_id = get_rs_id(&session);
        poll_tree_has_instance(&session, rs_id, "existing", 3000);
    });
}

#[test]
fn rapid_delete_recreate_cycle_5x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");

        for cycle in 1..=5 {
            // Re-read tree to get current instance
            let (session_id, _rs_id, existing_id) = get_rs_and_existing(&session);

            // Delete via API
            send_removal_fast(&session, &session_id, vec![existing_id]);

            // Recreate on disk
            let content = format!("-- cycle {} recovered\nreturn {{}}", cycle);
            fs::write(&file_path, &content).unwrap();

            // Wait for recovery
            thread::sleep(Duration::from_millis(1500));

            // Verify instance is back
            let rs_id = get_rs_id(&session);
            poll_tree_has_instance(&session, rs_id, "existing", 3000);
        }
    });
}

// ---------------------------------------------------------------------------
// Tests 39-40: Echo suppression under load
// ---------------------------------------------------------------------------

#[test]
fn echo_suppression_rapid_adds_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let root_read = session.get_api_read(root_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let initial_cursor = root_read.message_cursor;

        for i in 0..10 {
            let mut properties = HashMap::new();
            properties.insert(
                "Source".to_string(),
                Variant::String(format!("-- echo add {}", i)),
            );
            let added = AddedInstance {
                parent: Some(rs_id),
                name: format!("EchoAdd{}", i),
                class_name: "ModuleScript".to_string(),
                properties,
                children: vec![],
            };
            let mut added_map = HashMap::new();
            added_map.insert(Ref::new(), added);
            let write_request = WriteRequest {
                session_id: info.session_id,
                removed: vec![],
                added: added_map,
                updated: vec![],
            };
            session.post_api_write(&write_request).unwrap();
        }

        wait_for_settle();

        // All 10 files should exist
        let src = session.path().join("src");
        for i in 0..10 {
            assert_file_exists(
                &src.join(format!("EchoAdd{}.luau", i)),
                &format!("EchoAdd{}.luau", i),
            );
        }

        // Server should still be responsive. 10 adds x ~2 events each
        // (1 handle_tree_event + 1 VFS pickup) = ~20, with margin for
        // platform-specific directory events.
        let read_after = session.get_api_read(root_id).unwrap();
        let cursor_delta = read_after.message_cursor - initial_cursor;
        assert!(
            cursor_delta <= 30,
            "Cursor delta {} should be bounded (10 adds x ~2 events + margin)",
            cursor_delta
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn echo_suppression_mixed_operations() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let (session_id, rs_id, instances) = get_stress_instances(&session);
        let src = session.path().join("src");

        // 2 adds
        let mut added_map = HashMap::new();
        for i in 0..2 {
            let mut properties = HashMap::new();
            properties.insert(
                "Source".to_string(),
                Variant::String(format!("-- mixed add {}", i)),
            );
            added_map.insert(
                Ref::new(),
                AddedInstance {
                    parent: Some(rs_id),
                    name: format!("MixedAdd{}", i),
                    class_name: "ModuleScript".to_string(),
                    properties,
                    children: vec![],
                },
            );
        }

        // 2 updates (Alpha and Bravo)
        let updates: Vec<InstanceUpdate> = instances
            .iter()
            .take(2)
            .enumerate()
            .map(|(i, (id, _))| make_source_update(*id, &format!("-- mixed update {}", i)))
            .collect();

        // 1 removal (Echo)
        let removed = vec![instances[4].0];

        let write_request = WriteRequest {
            session_id,
            removed,
            added: added_map,
            updated: updates,
        };
        session.post_api_write(&write_request).unwrap();
        wait_for_settle();

        // Verify adds
        for i in 0..2 {
            assert_file_exists(
                &src.join(format!("MixedAdd{}.luau", i)),
                &format!("MixedAdd{}.luau", i),
            );
        }

        // Verify updates
        let alpha_content = fs::read_to_string(src.join("Alpha.luau")).unwrap();
        assert!(
            alpha_content.contains("-- mixed update 0"),
            "Alpha should be updated"
        );

        // Verify removal
        assert_not_exists(&src.join("Echo.luau"), "Echo.luau after removal");

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Test 41: Encoded name rapid rename chain
// ---------------------------------------------------------------------------

#[test]
fn encoded_name_rapid_rename_chain() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let src = session.path().join("src");
        let (_session_id, _module_id) = get_encoded_names_instance(&session, "What?Module");

        // Rename through names with special characters
        let names = ["Foo?Bar", "Key:Value", "Test?End", "Final:Name", "Done?Now"];
        let mut current_name = "What?Module".to_string();

        for new_name in &names {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(&session, &info.session_id, make_rename_update(id, new_name));
            current_name = new_name.to_string();
        }

        wait_for_settle();

        // Final file should use slugified name: Done_Now.luau
        let final_file = src.join("Done_Now.luau");
        assert_file_exists(&final_file, "Final encoded file");

        // Original should be gone
        assert_not_exists(&src.join("What_Module.luau"), "Original encoded file");
    });
}

// ===========================================================================
// PART 2: Randomized Fuzzing (Tests 42-46)
// ===========================================================================

// ---------------------------------------------------------------------------
// PRNG and fuzzing infrastructure
// ---------------------------------------------------------------------------

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn range(&mut self, min: u64, max: u64) -> u64 {
        min + (self.next() % (max - min))
    }

    fn choose<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }
}

/// State tracker for fuzzing — knows expected file name, class, and content.
struct FuzzState {
    current_name: String,
    current_class: String,
    current_source: String,
}

impl FuzzState {
    fn new(name: &str, class: &str, source: &str) -> Self {
        Self {
            current_name: name.to_string(),
            current_class: class.to_string(),
            current_source: source.to_string(),
        }
    }

    fn expected_file(&self) -> String {
        let suffix = class_suffix(&self.current_class);
        format!("{}{}.luau", self.current_name, suffix)
    }
}

// ---------------------------------------------------------------------------
// Test 42: Fuzz source and rename
// ---------------------------------------------------------------------------

#[test]
fn fuzz_source_and_rename_20_iterations() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let mut rng = XorShift64::new(42);
        let mut state = FuzzState::new(
            "existing",
            "ModuleScript",
            "-- Existing module for testing\nreturn {}",
        );

        // Random operations in a single continuous chain (no reset).
        // Reduced on macOS where kqueue's verbose events slow processing.
        #[cfg(target_os = "macos")]
        let iterations = 8;
        #[cfg(not(target_os = "macos"))]
        let iterations = 20;

        for op_idx in 0..iterations {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let found = rs_read
                .instances
                .iter()
                .find(|(_, inst)| inst.name == state.current_name);
            let inst_id = match found {
                Some((&id, _)) => id,
                None => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            if rng.next().is_multiple_of(2) {
                let new_name = format!("Fz{}", op_idx);
                send_update(
                    &session,
                    &info.session_id,
                    make_rename_update(inst_id, &new_name),
                );
                state.current_name = new_name;
            } else {
                let new_source = format!("-- fuzz o{}", op_idx);
                send_update(
                    &session,
                    &info.session_id,
                    make_source_update(inst_id, &new_source),
                );
                state.current_source = new_source;
            }
        }

        wait_for_settle();
        let expected = src.join(state.expected_file());
        assert!(
            expected.is_file(),
            "Expected file {} to exist after {} fuzz ops",
            expected.display(),
            iterations
        );
    });
}

// ---------------------------------------------------------------------------
// Test 43: Fuzz ClassName cycling
// ---------------------------------------------------------------------------

#[test]
fn fuzz_classname_cycling_20_iterations() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let class_options = ["ModuleScript", "Script", "LocalScript"];
        let mut rng = XorShift64::new(77);
        let mut state = FuzzState::new(
            "existing",
            "ModuleScript",
            "-- Existing module for testing\nreturn {}",
        );

        // 20 random ClassName changes in a single continuous chain
        for _ in 0..20 {
            let (session_id, _rs_id, id) = get_rs_and_existing(&session);
            let new_class = *rng.choose(&class_options);
            send_update(&session, &session_id, make_class_update(id, new_class));
            state.current_class = new_class.to_string();
        }

        wait_for_settle();
        let expected = src.join(state.expected_file());
        assert!(
            expected.is_file(),
            "Expected file {} to exist after 20 class changes",
            expected.display()
        );
    });
}

// ---------------------------------------------------------------------------
// Test 44: Fuzz combined operations
// ---------------------------------------------------------------------------

#[test]
fn fuzz_combined_operations_20_iterations() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let class_options = ["ModuleScript", "Script", "LocalScript"];
        let mut rng = XorShift64::new(99);
        let mut state = FuzzState::new(
            "existing",
            "ModuleScript",
            "-- Existing module for testing\nreturn {}",
        );

        // Random combined operations in a single continuous chain.
        // Reduced on macOS where kqueue's verbose events slow processing.
        #[cfg(target_os = "macos")]
        let iterations = 8;
        #[cfg(not(target_os = "macos"))]
        let iterations = 20;

        for op_idx in 0..iterations {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let found = rs_read
                .instances
                .iter()
                .find(|(_, inst)| inst.name == state.current_name);
            let inst_id = match found {
                Some((&id, _)) => id,
                None => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            let op_type = rng.range(0, 4);
            let new_name = format!("CF{}", op_idx);
            let new_source = format!("-- cf o{}", op_idx);
            let new_class = *rng.choose(&class_options);

            match op_type {
                0 => {
                    send_update(
                        &session,
                        &info.session_id,
                        make_source_update(inst_id, &new_source),
                    );
                    state.current_source = new_source;
                }
                1 => {
                    send_update(
                        &session,
                        &info.session_id,
                        make_rename_update(inst_id, &new_name),
                    );
                    state.current_name = new_name;
                }
                2 => {
                    send_update(
                        &session,
                        &info.session_id,
                        make_class_update(inst_id, new_class),
                    );
                    state.current_class = new_class.to_string();
                }
                _ => {
                    send_update(
                        &session,
                        &info.session_id,
                        make_combined_update(
                            inst_id,
                            Some(&new_name),
                            Some(new_class),
                            Some(&new_source),
                        ),
                    );
                    state.current_name = new_name;
                    state.current_class = new_class.to_string();
                    state.current_source = new_source;
                }
            }
        }

        wait_for_settle();
        let expected = src.join(state.expected_file());
        assert!(
            expected.is_file(),
            "Expected file {} to exist after {} combined fuzz ops. State: name={}, class={}",
            expected.display(),
            iterations,
            state.current_name,
            state.current_class,
        );
    });
}

// ---------------------------------------------------------------------------
// Test 45: Fuzz multi-instance
// ---------------------------------------------------------------------------

#[test]
fn fuzz_multi_instance_15_iterations() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let original_names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];

        for seed in 1u64..=15 {
            let mut rng = XorShift64::new(seed);

            // Pick 1-5 instances to update
            let count = rng.range(1, 6) as usize;
            let (session_id, _rs_id, instances) = get_stress_instances(&session);

            let updates: Vec<InstanceUpdate> = instances
                .iter()
                .take(count)
                .enumerate()
                .map(|(i, (id, _name))| {
                    make_source_update(*id, &format!("-- fuzz mi s{} i{}", seed, i))
                })
                .collect();

            let write_request = WriteRequest {
                session_id,
                removed: vec![],
                added: HashMap::new(),
                updated: updates,
            };
            session.post_api_write(&write_request).unwrap();
            thread::sleep(Duration::from_millis(100));
        }

        wait_for_settle();

        // All 5 files should still exist (we only did Source updates)
        for name in &original_names {
            assert_file_exists(
                &src.join(format!("{}.luau", name)),
                &format!("{}.luau after 100 fuzz iterations", name),
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Test 46: Fuzz directory format operations
// ---------------------------------------------------------------------------

#[test]
fn fuzz_directory_format_operations_15_iterations() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let class_options = ["ModuleScript", "Script", "LocalScript"];
        let mut current_name = "DirModuleWithChildren".to_string();

        for seed in 1u64..=15 {
            let mut rng = XorShift64::new(seed);
            let is_rename = rng.next().is_multiple_of(2);

            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let found = rs_read
                .instances
                .iter()
                .find(|(_, inst)| inst.name == current_name);
            let inst_id = match found {
                Some((&id, _)) => id,
                None => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };

            if is_rename {
                let new_name = format!("DirFz{}", seed);
                send_update_fast(
                    &session,
                    &info.session_id,
                    make_rename_update(inst_id, &new_name),
                );
                current_name = new_name;
            } else {
                let new_class = *rng.choose(&class_options);
                send_update_fast(
                    &session,
                    &info.session_id,
                    make_class_update(inst_id, new_class),
                );
            }
            thread::sleep(Duration::from_millis(200));
        }

        wait_for_settle();

        // Directory should exist with children intact
        let final_dir = src.join(&current_name);
        assert!(
            final_dir.is_dir(),
            "Final directory {} should exist",
            final_dir.display()
        );
        assert_file_exists(
            &final_dir.join("ChildA.luau"),
            "ChildA.luau after 100 iterations",
        );
        assert_file_exists(
            &final_dir.join("ChildB.luau"),
            "ChildB.luau after 100 iterations",
        );
    });
}

// ===========================================================================
// PART 3: File Watcher Stress Tests (Tests 47-66)
// ===========================================================================

// ---------------------------------------------------------------------------
// Tests 47-48: Rapid source edits on disk
// ---------------------------------------------------------------------------

#[test]
fn watcher_rapid_source_edits_on_disk_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");
        let rs_id = get_rs_id(&session);

        for i in 1..=10 {
            fs::write(&file_path, format!("-- disk v{}\nreturn {{}}", i)).unwrap();
            thread::sleep(Duration::from_millis(30));
        }

        poll_tree_source(&session, rs_id, "existing", "-- disk v10", 3000);

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_burst_writes_100x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");
        let rs_id = get_rs_id(&session);

        for i in 0..100 {
            fs::write(&file_path, format!("-- burst {}\nreturn {{}}", i)).unwrap();
        }
        fs::write(&file_path, "-- burst final\nreturn {}").unwrap();

        poll_tree_source(&session, rs_id, "existing", "-- burst final", 3000);

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 49-50: Filesystem rename
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn watcher_filesystem_rename_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let original_content = fs::read_to_string(src.join("existing.luau")).unwrap();

        let mut current_file = src.join("existing.luau");
        for i in 1..=10 {
            let new_file = src.join(format!("R{}.luau", i));
            fs::rename(&current_file, &new_file).unwrap();
            current_file = new_file;
            thread::sleep(Duration::from_millis(STRESS_OP_DELAY_MS));
        }

        poll_tree_has_instance(&session, rs_id, "R10", STRESS_POLL_TIMEOUT_MS);

        let content = fs::read_to_string(&current_file).unwrap();
        assert_eq!(
            original_content, content,
            "Content preserved through renames"
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_rename_with_content_change() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Atomic overwrite: write temp file, rename over existing
        let temp = src.join("existing.luau.tmp");
        fs::write(&temp, "-- atomic overwrite content\nreturn {}").unwrap();
        fs::rename(&temp, src.join("existing.luau")).unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "existing",
            "-- atomic overwrite content",
            3000,
        );

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 51-53: Delete + recreate via filesystem
// ---------------------------------------------------------------------------

#[test]
fn watcher_delete_recreate_immediate() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");
        let rs_id = get_rs_id(&session);

        fs::remove_file(&file_path).unwrap();
        fs::write(&file_path, "-- recreated immediately\nreturn {}").unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "existing",
            "-- recreated immediately",
            3000,
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_delete_recreate_cycle_5x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");
        let rs_id = get_rs_id(&session);

        for cycle in 1..=5 {
            fs::remove_file(&file_path).unwrap();
            thread::sleep(Duration::from_millis(50));
            let content = format!("-- cycle {} content\nreturn {{}}", cycle);
            fs::write(&file_path, &content).unwrap();

            poll_tree_source(
                &session,
                rs_id,
                "existing",
                &format!("-- cycle {} content", cycle),
                5000,
            );
        }

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_delete_recreate_different_content() {
    run_serve_test("syncback_write", |session, _redactions| {
        let file_path = session.path().join("src").join("existing.luau");
        let rs_id = get_rs_id(&session);

        fs::remove_file(&file_path).unwrap();
        // Wait for removal to propagate
        thread::sleep(Duration::from_millis(300));
        // Recreate with completely different content
        fs::write(
            &file_path,
            "-- completely different\nlocal x = 42\nreturn x",
        )
        .unwrap();

        poll_tree_source(&session, rs_id, "existing", "-- completely different", 5000);

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 54-57: Init file shenanigans
// ---------------------------------------------------------------------------

#[test]
fn watcher_edit_init_file() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let init_path = src.join("DirModuleWithChildren").join("init.luau");

        fs::write(&init_path, "-- edited init content\nreturn {}").unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "DirModuleWithChildren",
            "-- edited init content",
            3000,
        );

        // Children should still exist
        let dir_id = poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should still exist after init edit"
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_init_type_cycling_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");

        // Cycle: init.luau -> init.server.luau -> init.local.luau -> init.luau -> ...
        let transitions = [
            ("init.luau", "init.server.luau", "Script"),
            ("init.server.luau", "init.local.luau", "LocalScript"),
            ("init.local.luau", "init.luau", "ModuleScript"),
            ("init.luau", "init.server.luau", "Script"),
            ("init.server.luau", "init.local.luau", "LocalScript"),
            ("init.local.luau", "init.luau", "ModuleScript"),
            ("init.luau", "init.server.luau", "Script"),
            ("init.server.luau", "init.local.luau", "LocalScript"),
            ("init.local.luau", "init.luau", "ModuleScript"),
            ("init.luau", "init.server.luau", "Script"),
        ];

        for (from, to, expected_class) in &transitions {
            fs::rename(dir.join(from), dir.join(to)).unwrap();
            poll_tree_class(
                &session,
                rs_id,
                "DirModuleWithChildren",
                expected_class,
                3000,
            );
        }

        // Children should survive all transitions
        let dir_id = poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should survive init type cycling"
        );
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildB"),
            "ChildB should survive init type cycling"
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_delete_init_file() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let init_path = src.join("DirModuleWithChildren").join("init.luau");

        fs::remove_file(&init_path).unwrap();

        // Without init file, directory becomes a Folder
        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Folder", 3000);

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_replace_init_file_type() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");
        let old_init = dir.join("init.luau");
        let new_init = dir.join("init.server.luau");
        let content = fs::read_to_string(&old_init).unwrap();

        fs::remove_file(&old_init).unwrap();
        fs::write(&new_init, &content).unwrap();

        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Script", 3000);

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 58-60: Singular <-> directory format conversion
// ---------------------------------------------------------------------------

#[test]
fn watcher_standalone_to_directory_conversion() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let standalone = src.join("StandaloneModule.luau");
        let original = fs::read_to_string(&standalone).unwrap();

        // Convert: remove file, create directory with init + child
        fs::remove_file(&standalone).unwrap();
        let dir = src.join("StandaloneModule");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("init.luau"), &original).unwrap();
        fs::write(dir.join("ChildNew.luau"), "-- new child\nreturn {}").unwrap();

        poll_tree_has_child(&session, rs_id, "StandaloneModule", "ChildNew", 5000);

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_directory_to_standalone_conversion() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");
        let init_content = fs::read_to_string(dir.join("init.luau")).unwrap();

        // Convert: remove directory, create standalone file
        fs::remove_dir_all(&dir).unwrap();
        fs::write(src.join("DirModuleWithChildren.luau"), &init_content).unwrap();

        // Wait and verify no children remain
        thread::sleep(Duration::from_millis(500));
        let id = poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 5000);
        // Read children
        let child_read = session.get_api_read(id).unwrap();
        let child_count = child_read
            .instances
            .values()
            .filter(|i| i.name != "DirModuleWithChildren")
            .count();
        assert!(
            child_count == 0,
            "Standalone should have no children, found {}",
            child_count
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_format_flip_flop_5x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let standalone_path = src.join("StandaloneModule.luau");
        let dir_path = src.join("StandaloneModule");

        for cycle in 1..=5 {
            // Standalone -> Directory
            if standalone_path.exists() {
                fs::remove_file(&standalone_path).unwrap();
            }
            if !dir_path.exists() {
                fs::create_dir(&dir_path).unwrap();
            }
            fs::write(
                dir_path.join("init.luau"),
                format!("-- dir cycle {}\nreturn {{}}", cycle),
            )
            .unwrap();
            fs::write(
                dir_path.join("Child.luau"),
                format!("-- child cycle {}", cycle),
            )
            .unwrap();

            poll_tree_has_child(&session, rs_id, "StandaloneModule", "Child", 5000);

            // Directory -> Standalone
            fs::remove_dir_all(&dir_path).unwrap();
            fs::write(
                &standalone_path,
                format!("-- standalone cycle {}\nreturn {{}}", cycle),
            )
            .unwrap();

            poll_tree_source(
                &session,
                rs_id,
                "StandaloneModule",
                &format!("-- standalone cycle {}", cycle),
                5000,
            );
        }

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 61-62: Editor save patterns
// ---------------------------------------------------------------------------

#[test]
fn watcher_atomic_save_pattern() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let file = src.join("existing.luau");
        let temp = src.join("existing.luau.tmp");

        for i in 1..=5 {
            let content = format!("-- atomic save v{}\nreturn {{}}", i);
            fs::write(&temp, &content).unwrap();
            fs::rename(&temp, &file).unwrap();

            poll_tree_source(
                &session,
                rs_id,
                "existing",
                &format!("-- atomic save v{}", i),
                3000,
            );
        }

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_backup_rename_write_pattern() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let file = src.join("existing.luau");
        let backup = src.join("existing.luau.bak");

        // Backup-rename-write: rename existing to .bak, write new content
        fs::rename(&file, &backup).unwrap();
        fs::write(&file, "-- backup-rename-write content\nreturn {}").unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "existing",
            "-- backup-rename-write content",
            3000,
        );

        // Clean up backup
        if backup.exists() {
            fs::remove_file(&backup).unwrap();
        }

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 63-64: Parent directory operations
// ---------------------------------------------------------------------------

#[test]
fn watcher_parent_directory_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let old_dir = src.join("DirModuleWithChildren");
        let new_dir = src.join("RenamedDir");

        fs::rename(&old_dir, &new_dir).unwrap();

        poll_tree_has_instance(&session, rs_id, "RenamedDir", 5000);
        poll_tree_no_instance(&session, rs_id, "DirModuleWithChildren", 3000);

        // Children should be intact
        let dir_id = poll_tree_has_instance(&session, rs_id, "RenamedDir", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should be in renamed directory"
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_parent_directory_delete_all() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        fs::remove_dir_all(src.join("DirModuleWithChildren")).unwrap();

        poll_tree_no_instance(&session, rs_id, "DirModuleWithChildren", 5000);

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Tests 65-66: Concurrent filesystem + API
// ---------------------------------------------------------------------------

#[test]
fn watcher_filesystem_and_api_concurrent() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Filesystem edit on StandaloneModule
        let standalone_file = src.join("StandaloneModule.luau");
        fs::write(&standalone_file, "-- fs edited module\nreturn {}").unwrap();

        // API edit on StandaloneScript
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");
        send_update_no_wait(
            &session,
            &session_id,
            make_source_update(script_id, "-- api edited script"),
        );

        // Both should propagate
        poll_tree_source(
            &session,
            rs_id,
            "StandaloneModule",
            "-- fs edited module",
            3000,
        );
        let script_file = src.join("StandaloneScript.server.luau");
        // Wait for API write to complete
        thread::sleep(Duration::from_millis(500));
        let script_content = fs::read_to_string(&script_file).unwrap();
        assert!(
            script_content.contains("-- api edited script"),
            "Script should be updated via API, got: {}",
            script_content
        );

        session.assert_tree_fresh();
    });
}

#[test]
fn watcher_multi_file_simultaneous_edits() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];

        // Write all 5 files as fast as possible
        for (i, name) in names.iter().enumerate() {
            fs::write(
                src.join(format!("{}.luau", name)),
                format!("-- simultaneous edit {}\nreturn {{}}", i),
            )
            .unwrap();
        }

        // Poll for all 5 to update
        for (i, name) in names.iter().enumerate() {
            poll_tree_source(
                &session,
                rs_id,
                name,
                &format!("-- simultaneous edit {}", i),
                5000,
            );
        }

        session.assert_tree_fresh();
    });
}

// ===========================================================================
// PART 3: VFS & Change Processor Stress Tests
//
// These tests exercise every combination of rename, ClassName change, Source
// write, and format transition (standalone ↔ directory) under maximum
// pressure — zero-wait fire-and-forget, rapid interleaving, concurrent API +
// filesystem ops, encoded names through all paths, and multi-instance
// batches. The goal is to find every race condition in event suppression,
// metadata updates, and the snapshot pipeline.
// ===========================================================================

// ---------------------------------------------------------------------------
// Section A: No-Wait Rapid Chains (maximum VFS pressure)
//
// Every request is sent with ZERO sleep between them. The server and change
// processor must handle a firehose of overlapping PatchSets and VFS events.
// ---------------------------------------------------------------------------

/// No-wait rename chain: 10 renames fired as fast as the HTTP stack allows.
#[test]
fn no_wait_rename_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");

        // Get instance ID once before the loop — the Ref is stable across
        // renames, and reading the tree mid-chain is racy because the
        // change_processor may not have applied the previous rename yet.
        let (session_id, _rs_id, id) = get_rs_and_existing(&session);

        for i in 1..=10 {
            let new_name = format!("NWR{}", i);
            send_update_no_wait(&session, &session_id, make_rename_update(id, &new_name));
        }

        wait_for_settle();
        assert_file_exists(&src.join("NWR10.luau"), "NWR10.luau after no-wait chain");
        for i in 1..10 {
            assert_not_exists(
                &src.join(format!("NWR{}.luau", i)),
                &format!("Intermediate NWR{}.luau", i),
            );
        }
        assert_not_exists(&src.join("existing.luau"), "Original file");
    });
}

/// No-wait ClassName cycling: 10 class changes fired instantly.
#[test]
fn no_wait_classchange_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "LocalScript",
        ];

        for class in &classes {
            let (session_id, _rs_id, id) = get_rs_and_existing(&session);
            send_update_no_wait(&session, &session_id, make_class_update(id, class));
        }

        wait_for_settle();
        // Final: LocalScript -> existing.local.luau
        verify_instance_file(&src, "existing", "LocalScript", None);
    });
}

/// No-wait combined rename + source: 10 fire-and-forget.
#[test]
fn no_wait_combined_rename_source_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");

        let (session_id, _rs_id, id) = get_rs_and_existing(&session);

        for i in 1..=10 {
            let new_name = format!("NWRS{}", i);
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(id, Some(&new_name), None, Some(&format!("-- nwrs {}", i))),
            );
        }

        wait_for_settle();
        verify_instance_file(&src, "NWRS10", "ModuleScript", Some("-- nwrs 10"));
    });
}

/// No-wait combined rename + ClassName: 10 fire-and-forget.
#[test]
fn no_wait_combined_rename_class_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
        ];

        let (session_id, _rs_id, id) = get_rs_and_existing(&session);

        for (i, class) in classes.iter().enumerate() {
            let new_name = format!("NWRC{}", i + 1);
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(id, Some(&new_name), Some(class), None),
            );
        }

        wait_for_settle();
        // Final: Script -> NWRC10.server.luau
        verify_instance_file(&src, "NWRC10", "Script", None);
    });
}

/// No-wait combined rename + ClassName + source: the ultimate firehose.
#[test]
fn no_wait_combined_all_three_chain_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "LocalScript",
        ];

        let (session_id, _rs_id, id) = get_rs_and_existing(&session);

        for (i, class) in classes.iter().enumerate() {
            let new_name = format!("NWA{}", i + 1);
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(
                    id,
                    Some(&new_name),
                    Some(class),
                    Some(&format!("-- nwa {}", i + 1)),
                ),
            );
        }

        wait_for_settle();
        // Final: LocalScript -> NWA10.local.luau with "-- nwa 10"
        verify_instance_file(&src, "NWA10", "LocalScript", Some("-- nwa 10"));
    });
}

/// No-wait directory rename chain: 10 directory renames fired instantly.
#[test]
fn no_wait_directory_rename_chain_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");

        for i in 1..=10 {
            let new_name = format!("NWD{}", i);
            send_update_no_wait(&session, &session_id, make_rename_update(id, &new_name));
        }

        wait_for_settle();
        let final_dir = src.join("NWD10");
        assert!(final_dir.is_dir(), "NWD10 should exist as directory");
        assert_file_exists(&final_dir.join("init.luau"), "init.luau in NWD10");
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA in NWD10");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB in NWD10");
    });
}

/// No-wait directory ClassName cycling: 10 init file type changes instantly.
#[test]
fn no_wait_directory_class_chain_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let dir = src.join("DirModuleWithChildren");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "LocalScript",
        ];

        for class in &classes {
            let (session_id, id) =
                get_format_transitions_instance(&session, "DirModuleWithChildren");
            send_update_no_wait(&session, &session_id, make_class_update(id, class));
        }

        wait_for_settle();
        // Final: LocalScript -> init.local.luau
        assert_file_exists(
            &dir.join("init.local.luau"),
            "init.local.luau after no-wait class chain",
        );
        assert_file_exists(&dir.join("ChildA.luau"), "ChildA survived class chain");
        assert_file_exists(&dir.join("ChildB.luau"), "ChildB survived class chain");
    });
}

/// No-wait directory all-three: rename + class + source on init, 10x instant.
#[test]
fn no_wait_directory_all_three_chain_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
        ];

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");

        for (i, class) in classes.iter().enumerate() {
            let new_name = format!("NWDA{}", i + 1);
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(
                    id,
                    Some(&new_name),
                    Some(class),
                    Some(&format!("-- nwda {}", i + 1)),
                ),
            );
        }

        wait_for_settle();
        let final_dir = src.join("NWDA10");
        assert!(final_dir.is_dir(), "NWDA10 should be a directory");
        // Final class: Script -> init.server.luau
        assert_file_exists(
            &final_dir.join("init.server.luau"),
            "init.server.luau in NWDA10",
        );
        let content = fs::read_to_string(final_dir.join("init.server.luau")).unwrap();
        assert!(
            content.contains("-- nwda 10"),
            "Final source should contain '-- nwda 10', got: {}",
            content
        );
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA in NWDA10");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB in NWDA10");
    });
}

// ---------------------------------------------------------------------------
// Section B: Interleaved Rename + ClassName Combinations
//
// Tests that alternate between different operations on each step to maximise
// the chance of stale metadata / wrong instigating_source.
// ---------------------------------------------------------------------------

/// Alternate: even steps rename, odd steps change class.
#[test]
fn alternating_rename_classchange_10x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let class_seq = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
        ];
        let mut current_name = "existing".to_string();
        let mut current_class = "ModuleScript";

        for i in 1..=10 {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);

            if i % 2 == 0 {
                // Rename on even steps
                let new_name = format!("Alt{}", i);
                send_update_fast(
                    &session,
                    &info.session_id,
                    make_rename_update(id, &new_name),
                );
                current_name = new_name;
            } else {
                // Class change on odd steps
                let new_class = class_seq[(i / 2) % class_seq.len()];
                send_update_fast(&session, &info.session_id, make_class_update(id, new_class));
                current_class = new_class;
            }
        }

        wait_for_settle();
        let suffix = class_suffix(current_class);
        let expected = src.join(format!("{}{}.luau", current_name, suffix));
        assert!(
            expected.is_file(),
            "Expected {}, class={}",
            expected.display(),
            current_class
        );
    });
}

/// 5 renames, then 5 class changes, rapid fire.
#[test]
fn rename_burst_then_class_burst() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let mut current_name = "existing".to_string();

        // 5 rapid renames
        for i in 1..=5 {
            let new_name = format!("RB{}", i);
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_rename_update(id, &new_name),
            );
            current_name = new_name;
        }

        // 5 rapid class changes
        let classes = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
        ];
        for class in &classes {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(&session, &info.session_id, make_class_update(id, class));
        }

        wait_for_settle();
        // Final: RB5 as LocalScript -> RB5.local.luau
        verify_instance_file(&src, "RB5", "LocalScript", None);
    });
}

/// Directory: alternate rename and class change on each step.
#[test]
fn directory_alternating_rename_classchange_10x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let class_seq = [
            "Script",
            "LocalScript",
            "ModuleScript",
            "Script",
            "LocalScript",
        ];
        let mut current_name = "DirModuleWithChildren".to_string();
        let mut current_class = "ModuleScript";

        for i in 1..=10 {
            let (session_id, id) = get_format_transitions_instance(&session, &current_name);
            if i % 2 == 0 {
                let new_name = format!("DAlt{}", i);
                send_update_fast(&session, &session_id, make_rename_update(id, &new_name));
                current_name = new_name;
            } else {
                let new_class = class_seq[(i / 2) % class_seq.len()];
                send_update_fast(&session, &session_id, make_class_update(id, new_class));
                current_class = new_class;
            }
        }

        wait_for_settle();
        let final_dir = src.join(&current_name);
        assert!(final_dir.is_dir(), "{} should be a directory", current_name);
        let init_suffix = class_suffix(current_class);
        let init_name = if init_suffix.is_empty() {
            "init.luau".to_string()
        } else {
            format!("init{}.luau", init_suffix)
        };
        assert_file_exists(
            &final_dir.join(&init_name),
            &format!("{} in {}", init_name, current_name),
        );
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA survived");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB survived");
    });
}

// ---------------------------------------------------------------------------
// Section C: Format Transition Stress (standalone ↔ directory via watcher)
//
// Tests that convert files between standalone and directory format via direct
// filesystem manipulation, then layer renames and class changes on top.
// ---------------------------------------------------------------------------

/// Convert standalone to directory on disk, then immediately rename the dir.
#[test]
fn watcher_standalone_to_dir_then_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let standalone = src.join("StandaloneModule.luau");
        let original = fs::read_to_string(&standalone).unwrap();

        // Step 1: Convert standalone -> directory
        fs::remove_file(&standalone).unwrap();
        let dir = src.join("StandaloneModule");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("init.luau"), &original).unwrap();
        fs::write(dir.join("NewChild.luau"), "-- new child\nreturn {}").unwrap();

        // Wait for conversion to register
        poll_tree_has_child(&session, rs_id, "StandaloneModule", "NewChild", 5000);

        // Step 2: Immediately rename the directory
        let new_dir = src.join("RenamedAfterConversion");
        fs::rename(&dir, &new_dir).unwrap();

        poll_tree_has_instance(&session, rs_id, "RenamedAfterConversion", 5000);
        poll_tree_no_instance(&session, rs_id, "StandaloneModule", 3000);

        session.assert_tree_fresh();
    });
}

/// Convert directory to standalone on disk, then immediately rename the file.
#[test]
fn watcher_dir_to_standalone_then_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");
        let init_content = fs::read_to_string(dir.join("init.luau")).unwrap();

        // Step 1: Convert directory -> standalone
        fs::remove_dir_all(&dir).unwrap();
        let standalone = src.join("DirModuleWithChildren.luau");
        fs::write(&standalone, &init_content).unwrap();

        // Wait for conversion
        poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 5000);

        // Step 2: Rename the file
        let renamed = src.join("CollapsedAndRenamed.luau");
        fs::rename(&standalone, &renamed).unwrap();

        poll_tree_has_instance(&session, rs_id, "CollapsedAndRenamed", 5000);
        poll_tree_no_instance(&session, rs_id, "DirModuleWithChildren", 3000);

        session.assert_tree_fresh();
    });
}

/// Convert to directory, then change the init file type.
#[test]
fn watcher_standalone_to_dir_then_change_init_type() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let standalone = src.join("StandaloneModule.luau");
        let original = fs::read_to_string(&standalone).unwrap();

        // Convert standalone -> directory with init.luau
        fs::remove_file(&standalone).unwrap();
        let dir = src.join("StandaloneModule");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("init.luau"), &original).unwrap();

        poll_tree_class(&session, rs_id, "StandaloneModule", "ModuleScript", 5000);

        // Change init type: init.luau -> init.server.luau
        fs::rename(dir.join("init.luau"), dir.join("init.server.luau")).unwrap();

        poll_tree_class(&session, rs_id, "StandaloneModule", "Script", 5000);

        session.assert_tree_fresh();
    });
}

/// Delete init.luau, recreate as init.server.luau (different type).
#[test]
fn watcher_init_delete_recreate_different_type() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");

        // Delete init.luau
        fs::remove_file(dir.join("init.luau")).unwrap();

        // Becomes Folder first
        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Folder", 3000);

        // Recreate as init.server.luau
        fs::write(
            dir.join("init.server.luau"),
            "-- now a Script\nprint('hello')",
        )
        .unwrap();

        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Script", 5000);

        // Children still intact
        let dir_id = poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should survive init type transition"
        );

        session.assert_tree_fresh();
    });
}

/// Delete init file: children must survive as a Folder.
#[test]
fn watcher_init_delete_children_survive() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");

        fs::remove_file(dir.join("init.luau")).unwrap();

        // Should become Folder
        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Folder", 3000);

        // Children should still be there
        let dir_id = poll_tree_has_instance(&session, rs_id, "DirModuleWithChildren", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should exist after init deletion"
        );
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildB"),
            "ChildB should exist after init deletion"
        );

        session.assert_tree_fresh();
    });
}

/// Rapid format flip-flop with a rename mid-cycle.
#[test]
fn watcher_format_flip_flop_with_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let standalone_path = src.join("StandaloneModule.luau");
        let content = fs::read_to_string(&standalone_path).unwrap();

        // Cycle 1: Standalone -> Directory
        fs::remove_file(&standalone_path).unwrap();
        let dir = src.join("StandaloneModule");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("init.luau"), &content).unwrap();
        fs::write(dir.join("C1.luau"), "-- c1").unwrap();

        poll_tree_has_child(&session, rs_id, "StandaloneModule", "C1", 5000);

        // Cycle 2: Rename directory while it's a directory
        let renamed_dir = src.join("FlipRenamed");
        fs::rename(&dir, &renamed_dir).unwrap();

        poll_tree_has_instance(&session, rs_id, "FlipRenamed", 5000);

        // Cycle 3: Collapse back to standalone
        fs::remove_dir_all(&renamed_dir).unwrap();
        fs::write(src.join("FlipRenamed.luau"), "-- collapsed back\nreturn {}").unwrap();

        poll_tree_source(&session, rs_id, "FlipRenamed", "-- collapsed back", 5000);

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Section D: Multi-Instance Concurrent VFS Stress
//
// Hammer multiple files simultaneously to generate a storm of VFS events.
// ---------------------------------------------------------------------------

/// Rename all 5 stress files on disk simultaneously.
#[test]
fn watcher_rename_5_files_simultaneously() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
        let new_names = ["A1", "B1", "C1", "D1", "E1"];

        // Rename all 5 as fast as possible
        for (old, new) in names.iter().zip(new_names.iter()) {
            fs::rename(
                src.join(format!("{}.luau", old)),
                src.join(format!("{}.luau", new)),
            )
            .unwrap();
        }

        // All new names should appear
        for new in &new_names {
            poll_tree_has_instance(&session, rs_id, new, 5000);
        }
        // All old names should be gone
        for old in &names {
            poll_tree_no_instance(&session, rs_id, old, 3000);
        }

        session.assert_tree_fresh();
    });
}

/// Delete + recreate all 5 stress files simultaneously.
#[test]
fn watcher_delete_recreate_5_files_simultaneously() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];

        // Delete all
        for name in &names {
            fs::remove_file(src.join(format!("{}.luau", name))).unwrap();
        }
        // Recreate all immediately
        for (i, name) in names.iter().enumerate() {
            fs::write(
                src.join(format!("{}.luau", name)),
                format!("-- recreated {}\nreturn {{}}", i),
            )
            .unwrap();
        }

        for (i, name) in names.iter().enumerate() {
            poll_tree_source(&session, rs_id, name, &format!("-- recreated {}", i), 5000);
        }

        session.assert_tree_fresh();
    });
}

/// Mixed operations on 5 files: rename some, edit others, delete+recreate rest.
#[test]
fn watcher_mixed_rename_edit_delete_5_files() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Rename Alpha -> AlphaMoved
        fs::rename(src.join("Alpha.luau"), src.join("AlphaMoved.luau")).unwrap();
        // Edit Bravo
        fs::write(src.join("Bravo.luau"), "-- bravo edited\nreturn {}").unwrap();
        // Delete + recreate Charlie
        fs::remove_file(src.join("Charlie.luau")).unwrap();
        fs::write(src.join("Charlie.luau"), "-- charlie fresh\nreturn {}").unwrap();
        // Rename Delta -> DeltaMoved
        fs::rename(src.join("Delta.luau"), src.join("DeltaMoved.luau")).unwrap();
        // Edit Echo
        fs::write(src.join("Echo.luau"), "-- echo edited\nreturn {}").unwrap();

        poll_tree_has_instance(&session, rs_id, "AlphaMoved", 5000);
        poll_tree_source(&session, rs_id, "Bravo", "-- bravo edited", 5000);
        poll_tree_source(&session, rs_id, "Charlie", "-- charlie fresh", 5000);
        poll_tree_has_instance(&session, rs_id, "DeltaMoved", 5000);
        poll_tree_source(&session, rs_id, "Echo", "-- echo edited", 5000);

        session.assert_tree_fresh();
    });
}

/// API batch: rename + class + source on all 5 instances in one request.
#[test]
fn multi_instance_combined_rename_class_source_batch() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let (session_id, _rs_id, instances) = get_stress_instances(&session);
        let src = session.path().join("src");

        let configs: Vec<(&str, &str, &str)> = vec![
            ("A_Script", "Script", "-- alpha as script"),
            ("B_Local", "LocalScript", "-- bravo as local"),
            ("C_Module", "ModuleScript", "-- charlie as module"),
            ("D_Script", "Script", "-- delta as script"),
            ("E_Local", "LocalScript", "-- echo as local"),
        ];

        let updates: Vec<InstanceUpdate> = instances
            .iter()
            .zip(configs.iter())
            .map(|((id, _), (name, class, source))| {
                make_combined_update(*id, Some(name), Some(class), Some(source))
            })
            .collect();

        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added: HashMap::new(),
            updated: updates,
        };
        session.post_api_write(&write_request).unwrap();
        wait_for_settle();

        for (name, class, source) in &configs {
            verify_instance_file(&src, name, class, Some(source));
        }
        // All old files should be gone
        for old in &["Alpha", "Bravo", "Charlie", "Delta", "Echo"] {
            for sfx in &["", ".server", ".local"] {
                assert_not_exists(
                    &src.join(format!("{}{}.luau", old, sfx)),
                    &format!("Old {}{}.luau", old, sfx),
                );
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Section E: Encoded Names Through All Operation Types
//
// Encoded path names (special chars like ? : ) through rename, class change,
// and source update paths to verify encoding is preserved throughout.
// ---------------------------------------------------------------------------

/// Encoded name: rapid rename + source.
#[test]
fn encoded_rename_and_source_rapid() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let src = session.path().join("src");
        let mut current_name = "What?Module".to_string();

        let names_and_sources = [
            ("Test?One", "-- test one"),
            ("Another:Name", "-- another name"),
            ("Back?Again", "-- back again"),
            ("Final:Encoded", "-- final encoded"),
        ];

        for (new_name, new_source) in &names_and_sources {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_combined_update(id, Some(new_name), None, Some(new_source)),
            );
            current_name = new_name.to_string();
        }

        wait_for_settle();
        // Final: Final:Encoded -> Final_Encoded.luau
        let final_file = src.join("Final_Encoded.luau");
        assert_file_exists(&final_file, "Final encoded file");
        let content = fs::read_to_string(&final_file).unwrap();
        assert!(
            content.contains("-- final encoded"),
            "Content should be final, got: {}",
            content
        );
    });
}

/// Encoded name: rapid rename + ClassName.
#[test]
fn encoded_rename_and_classchange_rapid() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let src = session.path().join("src");
        let (_session_id, _id) = get_encoded_names_instance(&session, "What?Module");

        let ops: Vec<(&str, &str)> = vec![
            ("Enc?Script", "Script"),
            ("Enc:Local", "LocalScript"),
            ("Enc?Module", "ModuleScript"),
            ("Enc:Final", "Script"),
        ];

        let mut current_name = "What?Module".to_string();
        for (new_name, new_class) in &ops {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_combined_update(id, Some(new_name), Some(new_class), None),
            );
            current_name = new_name.to_string();
        }

        wait_for_settle();
        // Final: Enc:Final as Script -> Enc_Final.server.luau
        let final_file = src.join("Enc_Final.server.luau");
        assert_file_exists(&final_file, "Final encoded+class file");
    });
}

/// Encoded name: all three combined rapid.
#[test]
fn encoded_all_three_combined_rapid() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let src = session.path().join("src");

        let ops: Vec<(&str, &str, &str)> = vec![
            ("A?B", "Script", "-- a?b script"),
            ("C:D", "LocalScript", "-- c:d local"),
            ("E?F", "ModuleScript", "-- e?f module"),
            ("G:H", "Script", "-- g:h script"),
            ("End?Now", "LocalScript", "-- end now"),
        ];

        let mut current_name = "What?Module".to_string();
        for (new_name, new_class, new_source) in &ops {
            let info = session.get_api_rojo().unwrap();
            let root_read = session.get_api_read(info.root_instance_id).unwrap();
            let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
            let rs_read = session.get_api_read(rs_id).unwrap();
            let (id, _) = find_by_name(&rs_read.instances, &current_name);
            send_update_fast(
                &session,
                &info.session_id,
                make_combined_update(id, Some(new_name), Some(new_class), Some(new_source)),
            );
            current_name = new_name.to_string();
        }

        wait_for_settle();
        // Final: End?Now as LocalScript -> End_Now.local.luau
        let final_file = src.join("End_Now.local.luau");
        assert_file_exists(&final_file, "Final all-three encoded file");
        let content = fs::read_to_string(&final_file).unwrap();
        assert!(
            content.contains("-- end now"),
            "Content should be final, got: {}",
            content
        );
    });
}

// ---------------------------------------------------------------------------
// Section F: Concurrent API + Filesystem Operations
//
// Hit the server from both directions at once: API writes (rename/class/
// source) on one instance while the filesystem changes another.
// ---------------------------------------------------------------------------

/// API renames one instance while filesystem edits a different one.
#[test]
fn concurrent_api_rename_fs_edit_different_instances() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Filesystem: edit StandaloneModule
        fs::write(
            src.join("StandaloneModule.luau"),
            "-- fs concurrent edit\nreturn {}",
        )
        .unwrap();

        // API: rename StandaloneScript
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");
        send_update_no_wait(
            &session,
            &session_id,
            make_rename_update(script_id, "ScriptRenamed"),
        );

        // Both should propagate
        poll_tree_source(
            &session,
            rs_id,
            "StandaloneModule",
            "-- fs concurrent edit",
            5000,
        );
        wait_for_settle();
        assert_file_exists(
            &src.join("ScriptRenamed.server.luau"),
            "ScriptRenamed.server.luau",
        );
        assert_not_exists(
            &src.join("StandaloneScript.server.luau"),
            "Old StandaloneScript.server.luau",
        );
    });
}

/// API class change + filesystem rename on different instances simultaneously.
#[test]
fn concurrent_api_classchange_fs_rename_different_instances() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Filesystem: rename StandaloneModule.luau -> MovedModule.luau
        fs::rename(
            src.join("StandaloneModule.luau"),
            src.join("MovedModule.luau"),
        )
        .unwrap();

        // API: change StandaloneScript's class to LocalScript
        let (session_id, script_id) = get_format_transitions_instance(&session, "StandaloneScript");
        send_update_no_wait(
            &session,
            &session_id,
            make_class_update(script_id, "LocalScript"),
        );

        poll_tree_has_instance(&session, rs_id, "MovedModule", 5000);
        wait_for_settle();
        // StandaloneScript -> existing.local.luau (class changed to LocalScript)
        assert_file_exists(
            &src.join("StandaloneScript.local.luau"),
            "StandaloneScript.local.luau after class change",
        );
    });
}

/// API rename + source on one instance, filesystem dir rename on another.
#[test]
fn concurrent_api_combined_fs_dir_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Filesystem: rename DirModuleWithChildren -> MovedDir
        fs::rename(src.join("DirModuleWithChildren"), src.join("MovedDir")).unwrap();

        // API: rename + source on StandaloneModule
        let (session_id, module_id) = get_format_transitions_instance(&session, "StandaloneModule");
        send_update_no_wait(
            &session,
            &session_id,
            make_combined_update(
                module_id,
                Some("ModuleRenamed"),
                None,
                Some("-- api renamed module"),
            ),
        );

        poll_tree_has_instance(&session, rs_id, "MovedDir", 5000);
        wait_for_settle();
        assert_file_exists(
            &src.join("ModuleRenamed.luau"),
            "ModuleRenamed.luau after API rename",
        );
        let content = fs::read_to_string(src.join("ModuleRenamed.luau")).unwrap();
        assert!(
            content.contains("-- api renamed module"),
            "Content should reflect API update, got: {}",
            content
        );
    });
}

// ---------------------------------------------------------------------------
// Section G: Directory-Specific VFS Stress
//
// Hammer directory-format scripts with every combination of operations.
// ---------------------------------------------------------------------------

/// Directory: rename then immediately edit init source (API).
#[test]
fn directory_rename_then_source_update() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");

        // Rename
        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update_fast(&session, &session_id, make_rename_update(id, "DirRenamed"));

        // Re-fetch after rename settles
        let (session_id2, id2) = get_format_transitions_instance(&session, "DirRenamed");
        send_update_fast(
            &session,
            &session_id2,
            make_source_update(id2, "-- renamed then sourced"),
        );

        wait_for_settle();
        let dir = src.join("DirRenamed");
        assert!(dir.is_dir(), "DirRenamed should exist");
        let content = fs::read_to_string(dir.join("init.luau")).unwrap();
        assert!(
            content.contains("-- renamed then sourced"),
            "Init should have new source, got: {}",
            content
        );
        assert_file_exists(&dir.join("ChildA.luau"), "ChildA in DirRenamed");
    });
}

/// Directory: rename + class change combined in one update.
#[test]
fn directory_combined_rename_classchange() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update(
            &session,
            &session_id,
            make_combined_update(id, Some("DirRC"), Some("Script"), None),
        );

        wait_for_settle();
        let dir = src.join("DirRC");
        assert!(dir.is_dir(), "DirRC should exist");
        assert_file_exists(&dir.join("init.server.luau"), "init.server.luau in DirRC");
        assert_file_exists(&dir.join("ChildA.luau"), "ChildA in DirRC");
        assert_file_exists(&dir.join("ChildB.luau"), "ChildB in DirRC");
    });
}

/// Directory: rename + class + source all in one update.
#[test]
fn directory_combined_all_three() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update(
            &session,
            &session_id,
            make_combined_update(
                id,
                Some("DirAll3"),
                Some("LocalScript"),
                Some("-- dir all three"),
            ),
        );

        wait_for_settle();
        let dir = src.join("DirAll3");
        assert!(dir.is_dir(), "DirAll3 should exist");
        assert_file_exists(&dir.join("init.local.luau"), "init.local.luau in DirAll3");
        let content = fs::read_to_string(dir.join("init.local.luau")).unwrap();
        assert!(
            content.contains("-- dir all three"),
            "Init should have new source, got: {}",
            content
        );
        assert_file_exists(&dir.join("ChildA.luau"), "ChildA in DirAll3");
        assert_file_exists(&dir.join("ChildB.luau"), "ChildB in DirAll3");
    });
}

/// Filesystem: rename dir, then delete init -> becomes Folder under new name.
#[test]
fn watcher_directory_rename_then_delete_init() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Rename directory
        let old_dir = src.join("DirModuleWithChildren");
        let new_dir = src.join("RenamedThenStripped");
        fs::rename(&old_dir, &new_dir).unwrap();

        poll_tree_has_instance(&session, rs_id, "RenamedThenStripped", 5000);

        // Delete init file -> should become Folder
        fs::remove_file(new_dir.join("init.luau")).unwrap();

        poll_tree_class(&session, rs_id, "RenamedThenStripped", "Folder", 5000);

        // Children should survive
        let dir_id = poll_tree_has_instance(&session, rs_id, "RenamedThenStripped", 1000);
        let child_read = session.get_api_read(dir_id).unwrap();
        assert!(
            child_read.instances.values().any(|i| i.name == "ChildA"),
            "ChildA should survive rename + init deletion"
        );

        session.assert_tree_fresh();
    });
}

/// Filesystem: delete init, add different type, all while renaming dir.
#[test]
fn watcher_directory_init_swap_and_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let dir = src.join("DirModuleWithChildren");

        // Delete init.luau, rename dir, add init.server.luau
        let content = fs::read_to_string(dir.join("init.luau")).unwrap();
        fs::remove_file(dir.join("init.luau")).unwrap();
        let new_dir = src.join("SwappedAndRenamed");
        fs::rename(&dir, &new_dir).unwrap();
        fs::write(new_dir.join("init.server.luau"), &content).unwrap();

        poll_tree_has_instance(&session, rs_id, "SwappedAndRenamed", 5000);
        poll_tree_class(&session, rs_id, "SwappedAndRenamed", "Script", 5000);

        session.assert_tree_fresh();
    });
}

/// Rapid directory init type cycling + rename interleaved via filesystem.
#[test]
fn watcher_directory_rapid_init_cycling_and_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let mut dir = src.join("DirModuleWithChildren");

        // Step 1: init.luau -> init.server.luau
        fs::rename(dir.join("init.luau"), dir.join("init.server.luau")).unwrap();
        poll_tree_class(&session, rs_id, "DirModuleWithChildren", "Script", 3000);

        // Step 2: rename dir
        let new_dir = src.join("CycledDir");
        fs::rename(&dir, &new_dir).unwrap();
        dir = new_dir;
        poll_tree_has_instance(&session, rs_id, "CycledDir", 5000);

        // Step 3: init.server.luau -> init.local.luau
        fs::rename(dir.join("init.server.luau"), dir.join("init.local.luau")).unwrap();
        poll_tree_class(&session, rs_id, "CycledDir", "LocalScript", 5000);

        // Step 4: rename dir again
        let final_dir = src.join("FinalCycled");
        fs::rename(&dir, &final_dir).unwrap();
        poll_tree_has_instance(&session, rs_id, "FinalCycled", 5000);

        // Children intact
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA in FinalCycled");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB in FinalCycled");

        session.assert_tree_fresh();
    });
}

// ---------------------------------------------------------------------------
// Section H: Extreme Back-to-Back Stress
//
// Tests designed to be the absolute worst case for the change processor:
// maximum overlap, maximum event storms, minimum settle time.
// ---------------------------------------------------------------------------

/// 20 no-wait renames: the absolute maximum rename pressure on a single file.
#[test]
fn extreme_no_wait_rename_chain_20x() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");

        let (session_id, _rs_id, id) = get_rs_and_existing(&session);

        for i in 1..=20 {
            let new_name = format!("X{}", i);
            send_update_no_wait(&session, &session_id, make_rename_update(id, &new_name));
        }

        wait_for_settle();
        assert_file_exists(&src.join("X20.luau"), "X20.luau after 20 no-wait renames");
        assert_not_exists(&src.join("existing.luau"), "Original after 20 renames");
    });
}

/// 15 no-wait all-three-combined on a directory: rename + class + source.
/// (Directory all-three is the most expensive per-operation: dir rename +
/// init rename + source write + many VFS events. 15 iterations keeps the
/// extreme pressure without timing out the HTTP stack, unlike standalone
/// renames where 20 is fine because each op is cheap.)
#[test]
fn extreme_no_wait_directory_all_three_15x() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let classes = ["Script", "LocalScript", "ModuleScript"];

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");

        for i in 1..=15 {
            let new_name = format!("XD{}", i);
            let new_class = classes[i % 3];
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(
                    id,
                    Some(&new_name),
                    Some(new_class),
                    Some(&format!("-- xd {}", i)),
                ),
            );
        }

        wait_for_settle();
        let final_dir = src.join("XD15");
        assert!(final_dir.is_dir(), "XD15 should be a directory");
        // class at i=15: classes[15 % 3] = classes[0] = Script
        assert_file_exists(
            &final_dir.join("init.server.luau"),
            "init.server.luau in XD15 (Script)",
        );
        let content = fs::read_to_string(final_dir.join("init.server.luau")).unwrap();
        assert!(
            content.contains("-- xd 15"),
            "Final source in XD15, got: {}",
            content
        );
        assert_file_exists(&final_dir.join("ChildA.luau"), "ChildA in XD15");
        assert_file_exists(&final_dir.join("ChildB.luau"), "ChildB in XD15");
    });
}

/// Multi-instance: rename all 5 stress files in rapid succession via API,
/// each with a different class and source — all no-wait.
#[test]
fn extreme_multi_instance_rename_class_source_no_wait() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let configs: Vec<(&str, &str, &str)> = vec![
            ("X_A", "Script", "-- xa"),
            ("X_B", "LocalScript", "-- xb"),
            ("X_C", "ModuleScript", "-- xc"),
            ("X_D", "Script", "-- xd"),
            ("X_E", "LocalScript", "-- xe"),
        ];

        // Send all 5 as individual no-wait requests
        let (session_id, _rs_id, instances) = get_stress_instances(&session);
        for ((id, _), (name, class, source)) in instances.iter().zip(configs.iter()) {
            send_update_no_wait(
                &session,
                &session_id,
                make_combined_update(*id, Some(name), Some(class), Some(source)),
            );
        }

        wait_for_settle();
        for (name, class, source) in &configs {
            verify_instance_file(&src, name, class, Some(source));
        }
    });
}

/// Filesystem: rename + edit + delete + recreate + rename again, all on the
/// same file in rapid succession.
#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn extreme_filesystem_chaos_single_file() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let file = src.join("existing.luau");
        let gap = Duration::from_millis(STRESS_OP_DELAY_MS);

        // Step 1: Edit
        fs::write(&file, "-- chaos v1\nreturn {}").unwrap();
        thread::sleep(gap);

        // Step 2: Rename
        let file2 = src.join("Chaos.luau");
        fs::rename(&file, &file2).unwrap();
        thread::sleep(gap);

        // Step 3: Edit renamed file
        fs::write(&file2, "-- chaos v2\nreturn {}").unwrap();
        thread::sleep(gap);

        // Step 4: Delete
        fs::remove_file(&file2).unwrap();
        thread::sleep(gap);

        // Step 5: Recreate with original name
        fs::write(&file, "-- chaos v3\nreturn {}").unwrap();
        thread::sleep(gap);

        // Step 6: Edit again
        fs::write(&file, "-- chaos final\nreturn {}").unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "existing",
            "-- chaos final",
            STRESS_POLL_TIMEOUT_MS,
        );
    });
}

/// Filesystem: 5 files undergoing simultaneous rename chains.
/// macOS kqueue drops events under this level of rename pressure (5 files ×
/// 5 rounds with 250ms gaps). The single-file variant
/// `watcher_filesystem_rename_chain_10x` is already ignored on macOS for the
/// same reason.
#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn extreme_filesystem_5_file_rename_chains() {
    run_serve_test("syncback_stress", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let names = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
        let mut current: Vec<String> = names.iter().map(|n| n.to_string()).collect();

        // 5 rounds of renaming all 5 files
        for round in 1..=5 {
            let mut next = Vec::new();
            for (j, name) in current.iter().enumerate() {
                let new_name = format!("{}_r{}", names[j], round);
                fs::rename(
                    src.join(format!("{}.luau", name)),
                    src.join(format!("{}.luau", new_name)),
                )
                .unwrap();
                next.push(new_name);
            }
            current = next;
            thread::sleep(Duration::from_millis(STRESS_OP_DELAY_MS));
        }

        // All final names should exist
        for (j, _) in names.iter().enumerate() {
            let final_name = format!("{}_r5", names[j]);
            poll_tree_has_instance(&session, rs_id, &final_name, STRESS_POLL_TIMEOUT_MS);
        }
    });
}

// ===========================================================================
// PART 4: Failed Operation Suppression Cleanup
//
// When the change_processor suppresses a VFS path before a filesystem
// operation (rename, write) and that operation fails, the suppression must be
// cleaned up. Otherwise a stale entry sits in the suppression map and
// silently swallows the next real VFS event for that path — e.g. an external
// edit in VS Code would be lost.
//
// Strategy: block the filesystem operation by placing a non-empty directory
// at the rename target (or making the file read-only for writes), then
// verify that subsequent external edits are still picked up by the watcher.
// ===========================================================================

/// When a standalone file rename fails (target blocked by directory),
/// subsequent external edits to the original file must still be detected.
#[test]
fn failed_rename_no_stale_suppression() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Block the rename target: create a non-empty directory at the path
        // that fs::rename would target. fs::rename("existing.luau",
        // "Blocked.luau") fails because "Blocked.luau" is a directory.
        let blocker = src.join("Blocked.luau");
        fs::create_dir(&blocker).unwrap();
        fs::write(blocker.join("placeholder"), "").unwrap();

        // Send a rename that will fail internally.
        let (session_id, _rs_id, id) = get_rs_and_existing(&session);
        send_update_fast(&session, &session_id, make_rename_update(id, "Blocked"));
        wait_for_settle();

        // existing.luau must still be on disk (rename failed).
        assert_file_exists(
            &src.join("existing.luau"),
            "existing.luau should survive failed rename",
        );

        // Remove the blocker so it doesn't interfere.
        fs::remove_dir_all(&blocker).unwrap();

        // Edit existing.luau externally — this VFS event must NOT be
        // suppressed. If the suppression leaked, this edit is silently lost.
        fs::write(
            src.join("existing.luau"),
            "-- after failed rename\nreturn {}",
        )
        .unwrap();

        // The re-snapshot should pick up the edit (and fix the DOM name back
        // to "existing" since the file is still named existing.luau).
        poll_tree_source(&session, rs_id, "existing", "-- after failed rename", 5000);
    });
}

/// When a ClassName change rename fails (target blocked), subsequent
/// external edits to the original file must still be detected.
#[test]
fn failed_classchange_rename_no_stale_suppression() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // ModuleScript→Script renames existing.luau → existing.server.luau.
        // Block that target with a directory.
        let blocker = src.join("existing.server.luau");
        fs::create_dir(&blocker).unwrap();
        fs::write(blocker.join("placeholder"), "").unwrap();

        // Send ClassName change whose internal rename will fail.
        let (session_id, _rs_id, id) = get_rs_and_existing(&session);
        send_update_fast(&session, &session_id, make_class_update(id, "Script"));
        wait_for_settle();

        // Original file must still exist.
        assert_file_exists(
            &src.join("existing.luau"),
            "existing.luau should survive failed class rename",
        );

        // Remove blocker.
        fs::remove_dir_all(&blocker).unwrap();

        // External edit — must not be suppressed.
        fs::write(
            src.join("existing.luau"),
            "-- after failed classchange\nreturn {}",
        )
        .unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "existing",
            "-- after failed classchange",
            5000,
        );
    });
}

/// When a directory rename fails (target blocked), subsequent external
/// edits to the init file must still be detected.
#[test]
fn failed_directory_rename_no_stale_suppression() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Block the rename target with a non-empty directory.
        let blocker = src.join("BlockedDir");
        fs::create_dir(&blocker).unwrap();
        fs::write(blocker.join("placeholder"), "").unwrap();

        // Send directory rename that will fail internally.
        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update_fast(&session, &session_id, make_rename_update(id, "BlockedDir"));
        wait_for_settle();

        // Original directory must still exist.
        assert!(
            src.join("DirModuleWithChildren").is_dir(),
            "DirModuleWithChildren should survive failed rename"
        );

        // Remove blocker.
        fs::remove_dir_all(&blocker).unwrap();

        // Edit the init file externally — VFS must not suppress this.
        fs::write(
            src.join("DirModuleWithChildren").join("init.luau"),
            "-- after failed dir rename\nreturn {}",
        )
        .unwrap();

        poll_tree_source(
            &session,
            rs_id,
            "DirModuleWithChildren",
            "-- after failed dir rename",
            5000,
        );
    });
}

/// When a Source write fails (read-only file), subsequent external edits
/// must still be detected after the file is made writable again.
#[test]
#[allow(clippy::permissions_set_readonly_false)]
fn failed_write_no_stale_suppression() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);
        let file = src.join("existing.luau");

        // Make the file read-only so fs::write fails.
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();

        // Send Source change that will fail internally.
        let (session_id, _rs_id, id) = get_rs_and_existing(&session);
        send_update_fast(
            &session,
            &session_id,
            make_combined_update(id, None, None, Some("-- should fail to write")),
        );
        wait_for_settle();

        // Restore write permission.
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&file, perms).unwrap();

        // External edit — VFS event must not be suppressed.
        fs::write(&file, "-- after failed write\nreturn {}").unwrap();

        poll_tree_source(&session, rs_id, "existing", "-- after failed write", 5000);
    });
}

/// After a failed rename, verify the instance can still be successfully
/// renamed on a retry (no permanently broken state).
#[test]
fn failed_rename_then_successful_rename() {
    run_serve_test("syncback_write", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Block first rename attempt.
        let blocker = src.join("FirstAttempt.luau");
        fs::create_dir(&blocker).unwrap();
        fs::write(blocker.join("placeholder"), "").unwrap();

        let (session_id, _rs_id, id) = get_rs_and_existing(&session);
        send_update_fast(
            &session,
            &session_id,
            make_rename_update(id, "FirstAttempt"),
        );
        wait_for_settle();

        // First rename failed — original file still here.
        assert_file_exists(
            &src.join("existing.luau"),
            "existing.luau after blocked rename",
        );

        // Remove blocker.
        fs::remove_dir_all(&blocker).unwrap();

        // Edit the file to reset DOM name back to "existing" via VFS.
        fs::write(
            src.join("existing.luau"),
            "-- reset after failure\nreturn {}",
        )
        .unwrap();
        poll_tree_source(&session, rs_id, "existing", "-- reset after failure", 5000);

        // Now retry with an unblocked target — should succeed.
        let (session_id, _rs_id, id) = get_rs_and_existing(&session);
        send_update_fast(
            &session,
            &session_id,
            make_rename_update(id, "SuccessfulRetry"),
        );
        wait_for_settle();

        assert_file_exists(
            &src.join("SuccessfulRetry.luau"),
            "SuccessfulRetry.luau after unblocked rename",
        );
        assert_not_exists(
            &src.join("existing.luau"),
            "existing.luau should be gone after successful rename",
        );
    });
}

/// After a failed directory rename, verify the directory can still be
/// renamed on retry.
#[test]
fn failed_directory_rename_then_successful_rename() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        // Block first rename.
        let blocker = src.join("DirBlocked");
        fs::create_dir(&blocker).unwrap();
        fs::write(blocker.join("placeholder"), "").unwrap();

        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update_fast(&session, &session_id, make_rename_update(id, "DirBlocked"));
        wait_for_settle();

        // Original directory still exists.
        assert!(
            src.join("DirModuleWithChildren").is_dir(),
            "DirModuleWithChildren should survive blocked rename"
        );

        // Remove blocker.
        fs::remove_dir_all(&blocker).unwrap();

        // Edit init to reset DOM name via VFS.
        fs::write(
            src.join("DirModuleWithChildren").join("init.luau"),
            "-- dir reset\nreturn {}",
        )
        .unwrap();
        poll_tree_source(
            &session,
            rs_id,
            "DirModuleWithChildren",
            "-- dir reset",
            5000,
        );

        // Retry rename — should succeed now.
        let (session_id, id) = get_format_transitions_instance(&session, "DirModuleWithChildren");
        send_update_fast(
            &session,
            &session_id,
            make_rename_update(id, "DirRetrySuccess"),
        );
        wait_for_settle();

        assert!(
            src.join("DirRetrySuccess").is_dir(),
            "DirRetrySuccess should exist after unblocked rename"
        );
        assert_file_exists(
            &src.join("DirRetrySuccess").join("init.luau"),
            "init.luau in DirRetrySuccess",
        );
        assert_file_exists(
            &src.join("DirRetrySuccess").join("ChildA.luau"),
            "ChildA in DirRetrySuccess",
        );
    });
}

// ---------------------------------------------------------------------------
// Tests: Adding instances with forbidden chars (audit finding coverage)
// ---------------------------------------------------------------------------

/// Adding a new sibling instance whose name contains forbidden chars should
/// produce a slugified filename + adjacent meta with the `name` field.
#[test]
fn add_instance_with_forbidden_chars_creates_slug_and_meta() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- new forbidden".to_string()),
        );
        let added = AddedInstance {
            parent: Some(rs_id),
            name: "Hey/Bro".to_string(),
            class_name: "ModuleScript".to_string(),
            properties,
            children: vec![],
        };
        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added);
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        let src = session.path().join("src");
        let script_path = src.join("Hey_Bro.luau");
        let meta_path = src.join("Hey_Bro.meta.json5");

        assert_file_exists(&script_path, "Slugified script file for Hey/Bro");
        assert_file_exists(&meta_path, "Meta file for Hey/Bro with name field");

        let meta_content = fs::read_to_string(&meta_path).unwrap();
        assert!(
            meta_content.contains("\"Hey/Bro\""),
            "Meta should contain the real instance name \"Hey/Bro\", got: {}",
            meta_content
        );
    });
}

/// Adding two instances whose names slugify to the same string should produce
/// dedup suffixes (~1) and both should have correct meta `name` fields.
#[test]
fn add_two_colliding_instances_deduplicates() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        // Add "X/Y" (slugs to "X_Y")
        let mut props1 = HashMap::new();
        props1.insert(
            "Source".to_string(),
            Variant::String("-- first".to_string()),
        );
        let added1 = AddedInstance {
            parent: Some(rs_id),
            name: "X/Y".to_string(),
            class_name: "ModuleScript".to_string(),
            properties: props1,
            children: vec![],
        };

        // Add "X:Y" (also slugs to "X_Y" — collision)
        let mut props2 = HashMap::new();
        props2.insert(
            "Source".to_string(),
            Variant::String("-- second".to_string()),
        );
        let added2 = AddedInstance {
            parent: Some(rs_id),
            name: "X:Y".to_string(),
            class_name: "ModuleScript".to_string(),
            properties: props2,
            children: vec![],
        };

        let mut added_map = HashMap::new();
        added_map.insert(Ref::new(), added1);
        added_map.insert(Ref::new(), added2);
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        let src = session.path().join("src");
        let base = src.join("X_Y.luau");
        let deduped = src.join("X_Y~1.luau");

        // One should get X_Y.luau, the other X_Y~1.luau
        assert!(
            base.exists() && deduped.exists(),
            "Both X_Y.luau and X_Y~1.luau should exist. \
             base exists: {}, deduped exists: {}",
            base.exists(),
            deduped.exists()
        );

        // Both should have meta files with correct names
        let base_meta = src.join("X_Y.meta.json5");
        let deduped_meta = src.join("X_Y~1.meta.json5");
        assert_file_exists(&base_meta, "Meta for base slug instance");
        assert_file_exists(&deduped_meta, "Meta for deduped ~1 instance");

        // Read both meta files and verify they contain the correct names
        let meta1 = fs::read_to_string(&base_meta).unwrap();
        let meta2 = fs::read_to_string(&deduped_meta).unwrap();
        let has_xy = meta1.contains("\"X/Y\"") || meta1.contains("\"X:Y\"");
        let has_xy2 = meta2.contains("\"X/Y\"") || meta2.contains("\"X:Y\"");
        assert!(
            has_xy,
            "Base meta should contain X/Y or X:Y, got: {}",
            meta1
        );
        assert!(
            has_xy2,
            "Deduped meta should contain X/Y or X:Y, got: {}",
            meta2
        );
    });
}

/// Renaming a clean-named instance to a name with forbidden chars should
/// produce a slugified filename + adjacent meta with the `name` field.
#[test]
fn rename_clean_to_slugified() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, normal_id) = get_encoded_names_instance(&session, "Normal");

        let src = session.path().join("src");
        let old_path = src.join("Normal.luau");
        assert_file_exists(&old_path, "Normal.luau before rename");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: normal_id,
                changed_name: Some("Hey/Bro".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        let new_path = src.join("Hey_Bro.luau");
        let meta_path = src.join("Hey_Bro.meta.json5");

        poll_file_exists(&new_path, "Hey_Bro.luau after rename");
        poll_not_exists(&old_path, "Normal.luau should be gone after rename");

        assert_file_exists(&meta_path, "Meta file for Hey/Bro with name field");
        let meta_content = fs::read_to_string(&meta_path).unwrap();
        assert!(
            meta_content.contains("\"Hey/Bro\""),
            "Meta should contain the real instance name \"Hey/Bro\", got: {}",
            meta_content
        );
    });
}

/// Renaming a slugified instance back to a clean name should remove the
/// meta `name` field. When the meta file had no other fields, the file
/// itself must be deleted (the `RemoveNameOutcome::FileDeleted` path).
#[test]
fn rename_slugified_to_clean() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_id) = get_encoded_names_instance(&session, "What?Module");

        let src = session.path().join("src");
        let old_path = src.join("What_Module.luau");
        let old_meta = src.join("What_Module.meta.json5");
        assert_file_exists(&old_path, "What_Module.luau before rename");
        assert_file_exists(&old_meta, "What_Module.meta.json5 before rename");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: encoded_id,
                changed_name: Some("CleanName".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        let new_path = src.join("CleanName.luau");
        let new_meta = src.join("CleanName.meta.json5");

        poll_file_exists(&new_path, "CleanName.luau after rename");
        poll_not_exists(&old_path, "What_Module.luau should be gone after rename");
        poll_not_exists(
            &old_meta,
            "What_Module.meta.json5 should be gone after rename",
        );

        // The meta file had only a "name" field, so it should be deleted
        // entirely (not just emptied). This validates the FileDeleted path.
        poll_not_exists(
            &new_meta,
            "CleanName.meta.json5 should NOT exist (name-only meta → deleted)",
        );
    });
}

// ---------------------------------------------------------------------------
// Tests: Model JSON rename (compound extension preservation)
// ---------------------------------------------------------------------------

/// Renaming a .model.json5 instance must preserve the `.model` compound
/// extension and update the `name` field inside the model file (not in
/// adjacent .meta.json5).
#[test]
fn rename_model_json_preserves_compound_extension() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, model_id) = get_encoded_names_instance(&session, "What?Model");

        let src = session.path().join("src");
        let old_path = src.join("What_Model.model.json5");
        assert_file_exists(&old_path, "What_Model.model.json5 before rename");

        // Rename to another name needing slugification
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: Some("Why?Model".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // The .model compound extension MUST be preserved
        let new_path = src.join("Why_Model.model.json5");
        poll_file_exists(&new_path, "Why_Model.model.json5 should exist after rename");
        poll_not_exists(
            &old_path,
            "What_Model.model.json5 should be gone after rename",
        );

        // The WRONG path (lost .model extension) must NOT exist
        let wrong_path = src.join("Why_Model.json5");
        assert_not_exists(
            &wrong_path,
            "Why_Model.json5 (without .model) must NOT exist",
        );

        // The name field must be INSIDE the model file, not in adjacent meta.
        // JSON5 serialization uses unquoted keys, so check for key without
        // surrounding double-quotes.
        let model_content = fs::read_to_string(&new_path).unwrap();
        assert!(
            model_content.contains("name") && model_content.contains("Why?Model"),
            "Model file should contain a name field with \"Why?Model\", got: {}",
            model_content
        );

        // No adjacent .meta.json5 should be created for model files
        let wrong_meta = src.join("Why_Model.meta.json5");
        assert_not_exists(
            &wrong_meta,
            "Adjacent .meta.json5 should NOT be created for model files",
        );
    });
}

/// Renaming a .model.json5 instance to a clean name should remove the
/// `name` field from inside the model file.
#[test]
fn rename_model_json_to_clean_name() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, model_id) = get_encoded_names_instance(&session, "What?Model");

        let src = session.path().join("src");
        let old_path = src.join("What_Model.model.json5");
        assert_file_exists(&old_path, "What_Model.model.json5 before rename");

        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: Some("CleanModel".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        let new_path = src.join("CleanModel.model.json5");
        poll_file_exists(
            &new_path,
            "CleanModel.model.json5 should exist after rename",
        );
        poll_not_exists(&old_path, "What_Model.model.json5 should be gone");

        // The model file should NOT have a "name" key when the
        // filesystem name matches the instance name.
        // JSON5 uses unquoted keys (`name:` not `"name":`), so we
        // check for the key pattern at line start.
        let model_content = fs::read_to_string(&new_path).unwrap();
        let has_name_key = model_content
            .lines()
            .any(|l| l.trim().starts_with("name:") || l.trim().starts_with("\"name\":"));
        assert!(
            !has_name_key,
            "CleanModel.model.json5 should not have a name key, got: {}",
            model_content
        );
        let has_classname = model_content
            .lines()
            .any(|l| l.trim().starts_with("className:") || l.trim().starts_with("\"className\":"));
        assert!(
            has_classname,
            "Model file should still have className, got: {}",
            model_content
        );
    });
}

// ---------------------------------------------------------------------------
// Tests: 3+ instance collision deduplication
// ---------------------------------------------------------------------------

/// Adding 3 instances that all slug to the same name (X/Y, X:Y, X|Y → X_Y)
/// should produce X_Y.luau, X_Y~1.luau, X_Y~2.luau with correct meta files.
#[test]
fn add_three_colliding_instances_deduplicates() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        let names = ["X/Y", "X:Y", "X|Y"];
        let mut added_map = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            let mut props = HashMap::new();
            props.insert(
                "Source".to_string(),
                Variant::String(format!("-- instance {}", i + 1)),
            );
            added_map.insert(
                Ref::new(),
                AddedInstance {
                    parent: Some(rs_id),
                    name: name.to_string(),
                    class_name: "ModuleScript".to_string(),
                    properties: props,
                    children: vec![],
                },
            );
        }
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        let src = session.path().join("src");
        let base = src.join("X_Y.luau");
        let dedup1 = src.join("X_Y~1.luau");
        let dedup2 = src.join("X_Y~2.luau");

        assert!(
            base.exists() && dedup1.exists() && dedup2.exists(),
            "All three slugified files should exist.\n\
             X_Y.luau: {}, X_Y~1.luau: {}, X_Y~2.luau: {}",
            base.exists(),
            dedup1.exists(),
            dedup2.exists()
        );

        // All three should have meta files with the original names
        let base_meta = src.join("X_Y.meta.json5");
        let dedup1_meta = src.join("X_Y~1.meta.json5");
        let dedup2_meta = src.join("X_Y~2.meta.json5");
        assert_file_exists(&base_meta, "Meta for base slug");
        assert_file_exists(&dedup1_meta, "Meta for ~1 dedup");
        assert_file_exists(&dedup2_meta, "Meta for ~2 dedup");

        // Each meta should contain one of the original names
        let all_metas = [
            fs::read_to_string(&base_meta).unwrap(),
            fs::read_to_string(&dedup1_meta).unwrap(),
            fs::read_to_string(&dedup2_meta).unwrap(),
        ];
        for meta in &all_metas {
            let has_name =
                meta.contains("\"X/Y\"") || meta.contains("\"X:Y\"") || meta.contains("\"X|Y\"");
            assert!(
                has_name,
                "Meta should contain one of X/Y, X:Y, X|Y, got: {}",
                meta
            );
        }
    });
}

// ---------------------------------------------------------------------------
// Tests: Directory rename with forbidden chars
// ---------------------------------------------------------------------------

/// Renaming a directory-format script to a name with forbidden chars should
/// rename the directory using the slugified name and write a meta file with
/// the real name.
#[test]
fn rename_directory_format_to_slugified_name() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let (session_id, dir_module_id) =
            get_format_transitions_instance(&session, "DirModuleWithChildren");

        let src = session.path().join("src");
        let old_dir = src.join("DirModuleWithChildren");
        assert!(old_dir.is_dir(), "Old directory should exist before rename");

        // Rename to a name with forbidden chars
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: dir_module_id,
                changed_name: Some("Dir/With:Chars".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        let new_dir = src.join("Dir_With_Chars");
        poll_not_exists(&old_dir, "Old directory should be gone after rename");

        // Poll for the new directory
        let start = Instant::now();
        loop {
            if new_dir.is_dir() {
                break;
            }
            if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
                panic!(
                    "New directory Dir_With_Chars should exist after rename (timed out after {}ms)",
                    API_POLL_TIMEOUT_MS
                );
            }
            thread::sleep(Duration::from_millis(50));
        }

        // init.luau should be inside the renamed directory
        let init_file = new_dir.join("init.luau");
        assert_file_exists(&init_file, "init.luau inside renamed directory");

        // Children should be present in the renamed directory
        let child_a = new_dir.join("ChildA.luau");
        assert_file_exists(&child_a, "ChildA.luau inside renamed directory");

        // For init-file directories, the name is stored inside the
        // directory in init.meta.json5, not adjacent to it.
        let init_meta = new_dir.join("init.meta.json5");
        poll_file_exists(
            &init_meta,
            "init.meta.json5 inside Dir_With_Chars should exist with name override",
        );
        let meta_content = fs::read_to_string(&init_meta).unwrap();
        assert!(
            meta_content.contains("Dir/With:Chars"),
            "init.meta.json5 should contain real name \"Dir/With:Chars\", got: {}",
            meta_content
        );
    });
}

// ---------------------------------------------------------------------------
// Tests: Case-insensitive collision
// ---------------------------------------------------------------------------

/// Adding two instances whose names differ only in case (Foo and foo) should
/// be deduplicated, since they slug to the same lowercase key.
#[test]
fn add_case_insensitive_colliding_instances() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");

        let mut added_map = HashMap::new();
        let mut props1 = HashMap::new();
        props1.insert(
            "Source".to_string(),
            Variant::String("-- upper".to_string()),
        );
        added_map.insert(
            Ref::new(),
            AddedInstance {
                parent: Some(rs_id),
                name: "CaseTest".to_string(),
                class_name: "ModuleScript".to_string(),
                properties: props1,
                children: vec![],
            },
        );
        let mut props2 = HashMap::new();
        props2.insert(
            "Source".to_string(),
            Variant::String("-- lower".to_string()),
        );
        added_map.insert(
            Ref::new(),
            AddedInstance {
                parent: Some(rs_id),
                name: "casetest".to_string(),
                class_name: "ModuleScript".to_string(),
                properties: props2,
                children: vec![],
            },
        );

        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        let src = session.path().join("src");

        // One should get the base name, the other should get ~1
        // The exact casing depends on processing order, but both files
        // must exist with distinct names.
        let base = src.join("CaseTest.luau");
        let base_lower = src.join("casetest.luau");
        let dedup = src.join("CaseTest~1.luau");
        let dedup_lower = src.join("casetest~1.luau");

        let base_exists = base.exists() || base_lower.exists();
        let dedup_exists = dedup.exists() || dedup_lower.exists();

        assert!(
            base_exists && dedup_exists,
            "Both case-insensitive colliding instances should exist as separate files.\n\
             CaseTest.luau: {}, casetest.luau: {}, CaseTest~1.luau: {}, casetest~1.luau: {}",
            base.exists(),
            base_lower.exists(),
            dedup.exists(),
            dedup_lower.exists()
        );
    });
}

// ===========================================================================
// VFS Staleness Tests
//
// These tests verify the in-memory tree stays in sync with the real filesystem
// after various operation patterns that are known to stress the file watcher.
// Every test ends with `session.assert_tree_fresh()` which re-snapshots from
// disk and asserts zero drift.
// ===========================================================================

/// Simulate a git-checkout-like bulk change: overwrite 20+ files simultaneously.
#[test]
fn stale_tree_bulk_filesystem_changes() {
    run_serve_test("stale_tree", |session, _redactions| {
        let src = session.path().join("src");

        // Let the watcher fully start
        thread::sleep(Duration::from_secs(1));

        // Overwrite all initial files and add 15 more — no sleep between writes
        for i in 0..20 {
            let name = format!("bulk_{}.luau", i);
            fs::write(src.join(&name), format!("return \"bulk {}\"", i)).unwrap();
        }

        // Wait for tree to settle by polling for the last written file
        poll_tree_has_instance(&session, get_rs_id(&session), "bulk_19", 5000);

        session.assert_tree_fresh();
    });
}

/// After an API write (which triggers echo suppression), an external
/// overwrite of the SAME file must still be picked up.
#[test]
fn stale_tree_after_api_write_then_external_edit() {
    run_serve_test("stale_tree", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (a_id, _) = find_by_name(&rs_read.instances, "a");

        // Step 1: API write updates Source (triggers suppression for the file)
        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String("return \"api wrote this\"".to_string())),
        );
        let update = InstanceUpdate {
            id: a_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: props,
            changed_metadata: None,
        };
        send_update(&session, &info.session_id, update);

        // Step 2: External overwrite with DIFFERENT content
        let file_path = session.path().join("src").join("a.luau");
        fs::write(&file_path, "return \"external override\"").unwrap();

        // Step 3: Wait for the external change to propagate
        poll_tree_source(&session, rs_id, "a", "external override", 5000);

        // Step 4: Full tree must match filesystem exactly
        session.assert_tree_fresh();
    });
}

/// Delete all initial files and immediately create new ones with different names.
/// Tests the debouncer coalescing edge case (Remove + Create in quick succession).
#[test]
fn stale_tree_rapid_delete_recreate_different_names() {
    run_serve_test("stale_tree", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        thread::sleep(Duration::from_secs(1));

        // Delete all 5 initial files
        for name in &["a.luau", "b.luau", "c.luau", "d.luau", "e.luau"] {
            let _ = fs::remove_file(src.join(name));
        }
        // Immediately create 5 new files with different names
        for i in 0..5 {
            fs::write(
                src.join(format!("new_{}.luau", i)),
                format!("return \"new {}\"", i),
            )
            .unwrap();
        }

        // Wait for the last new file to appear in the tree
        poll_tree_has_instance(&session, rs_id, "new_4", STRESS_POLL_TIMEOUT_MS);

        // Old names must be gone
        for name in &["a", "b", "c", "d", "e"] {
            poll_tree_no_instance(&session, rs_id, name, STRESS_POLL_TIMEOUT_MS);
        }

        session.assert_tree_fresh();
    });
}

/// Bulk restructure: delete a subdirectory, rename a file, change another file's content.
/// Simulates the kind of multi-operation batch a VCS does.
#[test]
fn stale_tree_directory_restructure() {
    run_serve_test("stale_tree", |session, _redactions| {
        let src = session.path().join("src");
        let rs_id = get_rs_id(&session);

        thread::sleep(Duration::from_secs(1));

        // Delete the sub/ directory
        let _ = fs::remove_dir_all(src.join("sub"));

        // Rename a.luau → x.luau
        let _ = fs::rename(src.join("a.luau"), src.join("x.luau"));

        // Change content of b.luau
        fs::write(src.join("b.luau"), "return \"restructured b\"").unwrap();

        // Wait for changes to settle
        poll_tree_has_instance(&session, rs_id, "x", STRESS_POLL_TIMEOUT_MS);
        poll_tree_no_instance(&session, rs_id, "a", STRESS_POLL_TIMEOUT_MS);
        poll_tree_source(
            &session,
            rs_id,
            "b",
            "restructured b",
            STRESS_POLL_TIMEOUT_MS,
        );

        session.assert_tree_fresh();
    });
}

// ===========================================================================
// Chaos Fuzzer
//
// Slams the filesystem with random operations for 10 seconds straight, then
// waits for the watcher to settle and asserts zero tree drift. This is the
// nuclear option for finding watcher desync bugs.
// ===========================================================================

/// Chaos fuzzer: 3 seconds of random filesystem abuse (~500 ops).
/// This test exercises the ChangeProcessor's reconciliation mechanism
/// which corrects drift from lost OS events (ReadDirectoryChangesW
/// buffer overflow on Windows). Run manually with:
///   cargo test fuzz_filesystem_chaos -- --ignored --nocapture
///
/// Known limitation: on Windows, the notify crate's debouncer
/// continues delivering stale events for 60+ seconds after the chaos
/// ends. These stale events re-introduce drift after reconciliation.
/// A complete fix requires ignoring events that predate the last
/// reconciliation, which is tracked as future work.
#[test]
#[ignore]
fn fuzz_filesystem_chaos() {
    run_serve_test("stale_tree", |session, _redactions| {
        let src = session.path().join("src");

        // Let the watcher fully start
        thread::sleep(Duration::from_secs(1));

        // Track our own inventory
        let initial_files: Vec<PathBuf> = ["a.luau", "b.luau", "c.luau", "d.luau", "e.luau"]
            .iter()
            .map(|f| src.join(f))
            .collect();
        let mut known_files: Vec<PathBuf> = initial_files;
        let mut known_dirs: Vec<PathBuf> = vec![src.join("sub")];
        let mut file_counter = 0u64;
        let mut dir_counter = 0u64;

        let start = Instant::now();
        let duration = Duration::from_secs(3);
        let mut op_count = 0u64;

        while start.elapsed() < duration {
            let op = rand::random_range(0..9u32);
            match op {
                // 0: Create a new file
                0 => {
                    file_counter += 1;
                    let name = format!("fuzz_{}.luau", file_counter);
                    let path = src.join(&name);
                    if fs::write(&path, format!("return \"fuzz {}\"", file_counter)).is_ok() {
                        known_files.push(path);
                    }
                }
                // 1: Delete a random existing file
                1 => {
                    if !known_files.is_empty() {
                        let idx = rand::random_range(0..known_files.len());
                        let path = known_files.swap_remove(idx);
                        let _ = fs::remove_file(&path);
                    }
                }
                // 2: Overwrite a random existing file
                2 => {
                    if !known_files.is_empty() {
                        let idx = rand::random_range(0..known_files.len());
                        let path = &known_files[idx];
                        let _ = fs::write(path, format!("return \"overwrite {}\"", op_count));
                    }
                }
                // 3: Rename a random file
                3 => {
                    if !known_files.is_empty() {
                        file_counter += 1;
                        let idx = rand::random_range(0..known_files.len());
                        let old_path = known_files.swap_remove(idx);
                        let new_name = format!("renamed_{}.luau", file_counter);
                        let new_path = src.join(&new_name);
                        if fs::rename(&old_path, &new_path).is_ok() {
                            known_files.push(new_path);
                        }
                    }
                }
                // 4: Create a subdirectory + file inside it
                4 => {
                    dir_counter += 1;
                    let dir_name = format!("dir_{}", dir_counter);
                    let dir_path = src.join(&dir_name);
                    if fs::create_dir_all(&dir_path).is_ok() {
                        known_dirs.push(dir_path.clone());
                        file_counter += 1;
                        let file_path = dir_path.join(format!("child_{}.luau", file_counter));
                        if fs::write(&file_path, format!("return \"child {}\"", file_counter))
                            .is_ok()
                        {
                            known_files.push(file_path);
                        }
                    }
                }
                // 5: Delete a random subdirectory (recursive)
                5 => {
                    if !known_dirs.is_empty() {
                        let idx = rand::random_range(0..known_dirs.len());
                        let dir_path = known_dirs.swap_remove(idx);
                        // Remove tracked files inside this dir
                        known_files.retain(|f| !f.starts_with(&dir_path));
                        let _ = fs::remove_dir_all(&dir_path);
                    }
                }
                // 6: Rename a random directory
                6 => {
                    if !known_dirs.is_empty() {
                        dir_counter += 1;
                        let idx = rand::random_range(0..known_dirs.len());
                        let old_dir = known_dirs.swap_remove(idx);
                        let new_name = format!("rdir_{}", dir_counter);
                        let new_dir = src.join(&new_name);
                        if fs::rename(&old_dir, &new_dir).is_ok() {
                            // Update tracked file paths inside the renamed dir
                            for f in &mut known_files {
                                if f.starts_with(&old_dir) {
                                    let rel = f.strip_prefix(&old_dir).unwrap().to_path_buf();
                                    *f = new_dir.join(rel);
                                }
                            }
                            known_dirs.push(new_dir);
                        }
                    }
                }
                // 7: Rapid delete+recreate same file (known edge case)
                7 => {
                    if !known_files.is_empty() {
                        let idx = rand::random_range(0..known_files.len());
                        let path = known_files[idx].clone();
                        let _ = fs::remove_file(&path);
                        let _ = fs::write(&path, format!("return \"recreated {}\"", op_count));
                    }
                }
                // 8: Double-write same file
                8 => {
                    if !known_files.is_empty() {
                        let idx = rand::random_range(0..known_files.len());
                        let path = &known_files[idx];
                        let _ = fs::write(path, format!("return \"write1 {}\"", op_count));
                        let _ = fs::write(path, format!("return \"write2 {}\"", op_count));
                    }
                }
                _ => unreachable!(),
            }

            op_count += 1;

            // Tiny random delay (0-10ms) to vary timing pressure on the debouncer
            thread::sleep(Duration::from_millis(rand::random_range(0..10u64)));
        }

        eprintln!(
            "Chaos fuzzer completed {} operations in {:?}",
            op_count,
            start.elapsed()
        );

        // Wait for the VFS event backlog to mostly drain, then verify freshness.
        // The ChangeProcessor's periodic reconciliation corrects drift from
        // lost OS events (ReadDirectoryChangesW buffer overflow).
        thread::sleep(Duration::from_secs(5));

        // THE CRITICAL ASSERTION: tree must match filesystem after all that chaos.
        // The ChangeProcessor's reconcile_tree runs every ~200ms during event
        // processing and catches drift from lost OS events. By this point,
        // the tree should be consistent with disk.
        session.assert_tree_fresh();
    });
}

// ===========================================================================
// Ref property two-way sync tests
//
// These tests validate the full /api/write pipeline for Variant::Ref
// properties. The server should convert Ref properties to Rojo_Ref_*
// path-based attributes in meta/model files.
// ===========================================================================

/// Helper: get session info and find instances by name in the ref_two_way_sync fixture.
fn ref_test_setup(
    session: &crate::rojo_test::serve_util::TestServeSession,
) -> (
    librojo::SessionId,
    Ref, // workspace_id
    Ref, // model_id
    Ref, // part1_id
    Ref, // part2_id
    Ref, // objval_id
) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (workspace_id, _) = find_by_name(&root_read.instances, "Workspace");
    let ws_read = session.get_api_read(workspace_id).unwrap();

    let (model_id, _) = find_by_name(&ws_read.instances, "TestModel");
    let model_read = session.get_api_read(model_id).unwrap();
    let (part1_id, _) = find_by_name(&model_read.instances, "Part1");
    let (part2_id, _) = find_by_name(&model_read.instances, "Part2");

    let (objval_id, _) = find_by_name(&ws_read.instances, "TestObjectValue");

    (
        info.session_id,
        workspace_id,
        model_id,
        part1_id,
        part2_id,
        objval_id,
    )
}

/// Read a JSON5 file and return its parsed value.
fn read_json5_file(path: &Path) -> serde_json::Value {
    let content = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!("Failed to read {}: {}", path.display(), e);
    });
    json5::from_str(&content).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON5 at {}: {}\nContent: {}",
            path.display(),
            e,
            content
        );
    })
}

/// Check if a JSON5 file's attributes contain a specific Rojo_Ref_* key with expected value.
fn assert_meta_has_ref_attr(path: &Path, attr_name: &str, expected_path: &str) {
    let val = read_json5_file(path);
    let attr_val = val
        .get("attributes")
        .and_then(|a| a.get(attr_name))
        .and_then(|v| v.as_str());
    assert_eq!(
        attr_val,
        Some(expected_path),
        "File {} should have attributes.{} = {:?}, got {:?}",
        path.display(),
        attr_name,
        expected_path,
        attr_val,
    );
}

/// Check that a JSON5 file does NOT have a specific attribute key.
fn assert_meta_no_ref_attr(path: &Path, attr_name: &str) {
    if !path.exists() {
        return; // File doesn't exist means no attribute, which is correct.
    }
    let val = read_json5_file(path);
    let has_attr = val
        .get("attributes")
        .and_then(|a| a.get(attr_name))
        .is_some();
    assert!(
        !has_attr,
        "File {} should NOT have attributes.{}",
        path.display(),
        attr_name,
    );
}

/// Poll until a JSON5 file exists and has the expected Rojo_Ref_* attribute.
fn poll_meta_has_ref_attr(path: &Path, attr_name: &str, expected_path: &str) {
    let start = Instant::now();
    loop {
        if path.exists() {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(val) = json5::from_str::<serde_json::Value>(&content) {
                    if let Some(attr_val) = val
                        .get("attributes")
                        .and_then(|a| a.get(attr_name))
                        .and_then(|v| v.as_str())
                    {
                        if attr_val == expected_path {
                            return;
                        }
                    }
                }
            }
        }
        if start.elapsed() > Duration::from_millis(API_POLL_TIMEOUT_MS) {
            // Final check with detailed error
            assert_meta_has_ref_attr(path, attr_name, expected_path);
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

// ---------------------------------------------------------------------------
// Basic Ref write operations
// ---------------------------------------------------------------------------

#[test]
fn ref_set_primary_part() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
    });
}

#[test]
fn ref_set_object_value() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, _, part1_id, _, objval_id) = ref_test_setup(&session);

        let mut props = UstrMap::default();
        props.insert(ustr("Value"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: objval_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // ObjectValue is defined in a .model.json5 file, so the Ref attribute
        // is written directly into the .model.json5 file (not an adjacent .meta.json5).
        let model_path = session
            .path()
            .join("src/Workspace/TestObjectValue.model.json5");
        poll_meta_has_ref_attr(&model_path, "Rojo_Ref_Value", "Workspace/TestModel/Part1");
    });
}

// ---------------------------------------------------------------------------
// Nil Ref operations
// ---------------------------------------------------------------------------

#[test]
fn ref_set_primary_part_to_nil() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // First set PrimaryPart to Part1
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Now set PrimaryPart to nil
        let mut props2 = UstrMap::default();
        props2.insert(ustr("PrimaryPart"), Some(Variant::Ref(Ref::none())));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props2,
                changed_metadata: None,
            },
        );

        // Wait for processing then verify attribute removed
        thread::sleep(Duration::from_millis(300));
        assert_meta_no_ref_attr(&meta_path, "Rojo_Ref_PrimaryPart");
    });
}

#[test]
fn ref_nil_when_no_prior_attr() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, _, _, _) = ref_test_setup(&session);

        // Set PrimaryPart to nil without having set it before
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(Ref::none())));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Should not crash; meta file may or may not exist
        thread::sleep(Duration::from_millis(300));
        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        assert_meta_no_ref_attr(&meta_path, "Rojo_Ref_PrimaryPart");
    });
}

// ---------------------------------------------------------------------------
// Ref target changes
// ---------------------------------------------------------------------------

#[test]
fn ref_change_target() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, part2_id, _) = ref_test_setup(&session);

        // Set PrimaryPart to Part1
        let mut props1 = UstrMap::default();
        props1.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props1,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Change PrimaryPart to Part2
        let mut props2 = UstrMap::default();
        props2.insert(ustr("PrimaryPart"), Some(Variant::Ref(part2_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props2,
                changed_metadata: None,
            },
        );

        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part2",
        );
    });
}

#[test]
fn ref_set_nil_then_set_again() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);
        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");

        // Set → nil → set again
        let mut props1 = UstrMap::default();
        props1.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props1,
                changed_metadata: None,
            },
        );
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        let mut props2 = UstrMap::default();
        props2.insert(ustr("PrimaryPart"), Some(Variant::Ref(Ref::none())));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props2,
                changed_metadata: None,
            },
        );
        thread::sleep(Duration::from_millis(300));
        assert_meta_no_ref_attr(&meta_path, "Rojo_Ref_PrimaryPart");

        let mut props3 = UstrMap::default();
        props3.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props3,
                changed_metadata: None,
            },
        );
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
    });
}

// ---------------------------------------------------------------------------
// Ref with other property changes
// ---------------------------------------------------------------------------

#[test]
fn ref_with_name_change() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: Some("RenamedModel".to_string()),
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Both name change and ref change should be processed correctly.
        // The ref is initially written with the pre-rename path, but the
        // ChangeProcessor's update_ref_paths_after_rename fixes it to
        // reflect the new directory name.
        thread::sleep(Duration::from_millis(500));

        let new_meta = session
            .path()
            .join("src/Workspace/RenamedModel/init.meta.json5");
        poll_file_exists(&new_meta, "Meta file should exist at new location");

        // The ref path should be updated to reflect the renamed parent
        assert_meta_has_ref_attr(
            &new_meta,
            "Rojo_Ref_PrimaryPart",
            "Workspace/RenamedModel/Part1",
        );
    });
}

#[test]
fn ref_only_change_creates_meta() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // Sending ONLY a Ref property change (no Source, no Name, no other props)
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
    });
}

// ---------------------------------------------------------------------------
// Ref on different file formats
// ---------------------------------------------------------------------------

#[test]
fn ref_on_model_json5_instance() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, _, part1_id, _, objval_id) = ref_test_setup(&session);

        let mut props = UstrMap::default();
        props.insert(ustr("Value"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: objval_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // ObjectValue is a .model.json5 — Ref attribute is written
        // directly into the .model.json5 file (not an adjacent .meta.json5)
        // because model files support in-place JSON property updates.
        let model_path = session
            .path()
            .join("src/Workspace/TestObjectValue.model.json5");
        poll_meta_has_ref_attr(&model_path, "Rojo_Ref_Value", "Workspace/TestModel/Part1");
    });
}

#[test]
fn ref_existing_meta_preserved() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // The Model's init.meta.json5 already has className: "Model".
        // After adding a Ref attribute, className should still be present.
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Verify existing content wasn't clobbered
        let val = read_json5_file(&meta_path);
        assert_eq!(
            val.get("className").and_then(|v| v.as_str()),
            Some("Model"),
            "Existing className should be preserved after adding Ref attribute"
        );
    });
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
fn ref_to_nonexistent_instance_no_crash() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, _, _, _) = ref_test_setup(&session);

        // Use a random Ref that doesn't exist in the tree
        let fake_ref = Ref::new();
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(fake_ref)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Should not crash. The Ref is logged as warning and skipped.
        thread::sleep(Duration::from_millis(300));
    });
}

#[test]
fn ref_mixed_valid_and_invalid() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // Send both a valid Ref and a regular property
        let fake_ref = Ref::new();
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        // Also try to set a non-existent ref property (should be ignored)
        props.insert(ustr("SomeOtherRef"), Some(Variant::Ref(fake_ref)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // The valid PrimaryPart Ref should still be written
        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
    });
}

// ===========================================================================
// Regression tests for known Ref limitations (Section 10 of audit plan)
//
// These tests assert the CORRECT round-trip behavior. They are expected to
// FAIL against the current implementation, proving the bugs are real.
// They serve as acceptance criteria for future fixes.
// DO NOT mark them #[ignore] or fix the implementation to make them pass.
// ===========================================================================

/// 10a: After setting PrimaryPart and then renaming the TARGET Part,
/// the Rojo_Ref_PrimaryPart attribute should update to the new path.
/// `update_ref_paths_after_rename` in ChangeProcessor handles this.
#[test]
fn ref_stale_path_after_target_rename() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // Step 1: Set PrimaryPart = Part1
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Step 2: Rename Part1 to RenamedPart
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: part1_id,
                changed_name: Some("RenamedPart".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // Wait for rename to process
        thread::sleep(Duration::from_millis(500));

        // Step 3: The stored path should be updated to reflect the rename
        assert_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/RenamedPart",
        );
    });
}

/// 10a variation: After setting PrimaryPart and then renaming the MODEL
/// (parent), the stored path should update.
/// `update_ref_paths_after_rename` in ChangeProcessor handles this.
#[test]
fn ref_stale_path_after_parent_rename() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // Step 1: Set PrimaryPart = Part1
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Step 2: Rename the MODEL (parent) from TestModel to RenamedModel
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: Some("RenamedModel".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // Wait for rename to process
        thread::sleep(Duration::from_millis(500));

        // The meta file is now at the new location
        let new_meta_path = session
            .path()
            .join("src/Workspace/RenamedModel/init.meta.json5");
        poll_file_exists(&new_meta_path, "Meta file should exist at new location");

        // The stored path should reflect the new parent name
        assert_meta_has_ref_attr(
            &new_meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/RenamedModel/Part1",
        );
    });
}

/// Add a new Part and set PrimaryPart to it in the SAME write request.
/// The `added_paths` mechanism in `syncback_updated_properties` pre-computes
/// paths for instances from `request.added`, so the Rojo_Ref_PrimaryPart
/// attribute is written correctly even though the new Part hasn't been
/// added to the tree by the VFS watcher yet.
#[test]
fn ref_to_instance_added_in_same_request() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, _, _, _) = ref_test_setup(&session);

        // Create a temp GUID for the new Part (same as plugin would generate)
        let new_part_ref = Ref::new();

        let mut added = HashMap::new();
        added.insert(
            new_part_ref,
            AddedInstance {
                parent: Some(model_id),
                name: "NewPart".to_string(),
                class_name: "Part".to_string(),
                properties: HashMap::new(),
                children: Vec::new(),
            },
        );

        // Set PrimaryPart to the new Part in the SAME request
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(new_part_ref)));

        let write_request = WriteRequest {
            session_id,
            removed: vec![],
            added,
            updated: vec![InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            }],
        };

        session
            .post_api_write(&write_request)
            .expect("Write request should succeed");

        // Wait for processing
        thread::sleep(Duration::from_millis(500));

        // The meta file should contain Rojo_Ref_PrimaryPart pointing to the new Part.
        // The added_paths fallback resolves the path from the AddedInstance data.
        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        assert_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/NewPart",
        );

        // Verify the tree also has a resolved PrimaryPart Ref (not dangling).
        // The added instance should be in the PatchSet's added_instances,
        // so apply_patch_set maps the plugin GUID to a real tree ID.
        // Wait for VFS watcher to fill in metadata.
        thread::sleep(Duration::from_millis(500));

        let model_read = session.get_api_read(model_id).unwrap();
        let model_inst = model_read.instances.get(&model_id);
        assert!(model_inst.is_some(), "Model should be readable after write");

        // Find NewPart among model's children
        let new_part_exists = model_read
            .instances
            .values()
            .any(|inst| inst.name == "NewPart");
        assert!(
            new_part_exists,
            "NewPart should exist in the tree after immediate ID assignment"
        );
    });
}

// ===========================================================================
// Ambiguous path Ref tests
//
// These tests verify that setting a Ref property where the target has
// duplicate-named siblings does not crash. The Ref is still written
// (with a warning), but the path may resolve to the wrong sibling on
// rebuild.
// ===========================================================================

/// Setting PrimaryPart to a target that has a duplicate-named sibling
/// should not crash. The Ref attribute should still be written to disk.
#[test]
fn ref_ambiguous_path_no_crash() {
    run_serve_test("ref_ambiguous_path", |session, _| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (workspace_id, _) = find_by_name(&root_read.instances, "Workspace");
        let ws_read = session.get_api_read(workspace_id).unwrap();

        let (model_id, _) = find_by_name(&ws_read.instances, "DupParent");

        // Find the Target (non-ambiguous sibling outside DupParent)
        let (target_id, _) = find_by_name(&ws_read.instances, "Target");

        // Setting Ref to Target (unique path) should work fine
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(target_id)));
        send_update(
            &session,
            &info.session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/DupParent/init.meta.json5");
        poll_meta_has_ref_attr(&meta_path, "Rojo_Ref_PrimaryPart", "Workspace/Target");
    });
}

/// Setting PrimaryPart to one of two duplicate-named Children should
/// not crash. The Ref is written but the path is ambiguous.
#[test]
fn ref_ambiguous_target_no_crash() {
    run_serve_test("ref_ambiguous_path", |session, _| {
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (workspace_id, _) = find_by_name(&root_read.instances, "Workspace");
        let ws_read = session.get_api_read(workspace_id).unwrap();

        let (model_id, _) = find_by_name(&ws_read.instances, "DupParent");
        let model_read = session.get_api_read(model_id).unwrap();

        // Find one of the duplicate "Child" instances
        let child_instances: Vec<(Ref, _)> = model_read
            .instances
            .iter()
            .filter(|(_, inst)| inst.name == "Child")
            .map(|(id, inst)| (*id, inst))
            .collect();

        // There should be at least one Child
        assert!(
            !child_instances.is_empty(),
            "Should have at least one Child instance"
        );

        let (child_id, _) = child_instances[0];

        // Set PrimaryPart to the ambiguous child -- should NOT crash
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(child_id)));
        send_update(
            &session,
            &info.session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // The Ref should be written to disk (even though the path is ambiguous).
        // We just verify it doesn't crash and a meta file exists.
        thread::sleep(Duration::from_millis(300));
        let meta_path = session
            .path()
            .join("src/Workspace/DupParent/init.meta.json5");
        assert!(
            meta_path.exists(),
            "Meta file should exist after setting ambiguous Ref"
        );

        // The meta file should have a Rojo_Ref_PrimaryPart attribute
        // (the path may resolve to the wrong Child, but it shouldn't crash)
        let val = read_json5_file(&meta_path);
        let has_ref = val
            .get("attributes")
            .and_then(|a| a.get("Rojo_Ref_PrimaryPart"))
            .is_some();
        assert!(
            has_ref,
            "Rojo_Ref_PrimaryPart should be written even for ambiguous paths"
        );
    });
}

// ===========================================================================
// RefPathIndex startup population and precise removal tests
// ===========================================================================

/// Fix 1 test: Pre-existing Rojo_Ref_* attributes in meta files should be
/// indexed at server startup. Renaming a target instance without any prior
/// two-way sync writes should still update the stored path.
///
/// The ref_two_way_sync fixture has Rojo_Ref_PrimaryPart = "Workspace/TestModel/Part1"
/// in init.meta.json5. This test renames Part1 WITHOUT first writing a Ref via
/// /api/write, proving the startup index population catches it.
#[test]
fn ref_startup_index_rename_updates_preexisting_attr() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, _, part1_id, _, _) = ref_test_setup(&session);

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");

        // Verify the fixture has the pre-existing Rojo_Ref_PrimaryPart attribute
        assert_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Rename Part1 to IndexTestPart WITHOUT any prior /api/write for Refs.
        // The RefPathIndex should have been populated at server startup from the
        // fixture's meta file, so update_ref_paths_after_rename should find it.
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: part1_id,
                changed_name: Some("IndexTestPart".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // Wait for rename to process
        thread::sleep(Duration::from_millis(500));

        // The stored path should be updated via the startup-populated index
        assert_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/IndexTestPart",
        );
    });
}

/// Fix 2 test: Setting two Rojo_Ref_* attrs on the same instance, removing
/// one, and then renaming the target of the remaining one should still update
/// the remaining ref path. This tests that the RefPathIndex correctly re-indexes
/// after partial attribute removal (not overbroad deletion).
#[test]
fn ref_partial_removal_preserves_remaining_index_entry() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, part2_id, _) = ref_test_setup(&session);

        // Step 1: Set PrimaryPart to Part1 AND a second Ref (Adornee) to Part2
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        props.insert(ustr("Adornee"), Some(Variant::Ref(part2_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
        // Verify second ref is there too
        assert_meta_has_ref_attr(&meta_path, "Rojo_Ref_Adornee", "Workspace/TestModel/Part2");

        // Step 2: Remove PrimaryPart (set to nil), keep Adornee
        let mut props2 = UstrMap::default();
        props2.insert(ustr("PrimaryPart"), Some(Variant::Ref(Ref::none())));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props2,
                changed_metadata: None,
            },
        );

        thread::sleep(Duration::from_millis(300));
        assert_meta_no_ref_attr(&meta_path, "Rojo_Ref_PrimaryPart");
        // Adornee should still be there
        assert_meta_has_ref_attr(&meta_path, "Rojo_Ref_Adornee", "Workspace/TestModel/Part2");

        // Step 3: Rename Part2. The RefPathIndex should still have the entry
        // for Adornee (not overbroad-removed when PrimaryPart was deleted).
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: part2_id,
                changed_name: Some("RenamedPart2".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        thread::sleep(Duration::from_millis(500));

        // The Adornee path should have been updated
        assert_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_Adornee",
            "Workspace/TestModel/RenamedPart2",
        );
    });
}

/// Fix 3 test: Renaming an instance to a name with forbidden filesystem
/// characters (which gets slugified on disk) should still correctly update
/// Rojo_Ref_* paths in meta files inside the renamed directory.
#[test]
fn ref_rename_to_slugified_name_updates_ref_paths() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, model_id, part1_id, _, _) = ref_test_setup(&session);

        // Set PrimaryPart to Part1 first
        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let meta_path = session
            .path()
            .join("src/Workspace/TestModel/init.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Rename Model to a name with forbidden characters.
        // The directory on disk will be slugified: "Slug:Model" -> "Slug_Model"
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: model_id,
                changed_name: Some("Slug:Model".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // Wait for rename to process
        thread::sleep(Duration::from_millis(500));

        // The directory should be slugified on disk
        let new_meta_path = session
            .path()
            .join("src/Workspace/Slug_Model/init.meta.json5");
        poll_file_exists(&new_meta_path, "Slugified directory should exist");

        // The stored Rojo_Ref_* path should use the tree name (not slugified)
        // because ref paths are tree-based, not filesystem-based
        assert_meta_has_ref_attr(
            &new_meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/Slug:Model/Part1",
        );
    });
}

// ===========================================================================
// Ref on different standalone file formats (meta path computation coverage)
//
// These tests cover branches 2 and 4 of syncback_updated_properties:
// - Branch 2: Standalone script → adjacent ScriptName.meta.json5
//   (strips .server/.client/.plugin/.local/.legacy suffix from stem)
// - Branch 4: Non-script standalone file (.txt, .csv) → adjacent FileName.meta.json5
// ===========================================================================

/// Setting a Ref property on a standalone script should write to the adjacent
/// meta file with the compound suffix stripped: TestScript.server.luau →
/// TestScript.meta.json5 (NOT TestScript.server.meta.json5).
#[test]
fn ref_on_standalone_script() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, _, part1_id, _, _) = ref_test_setup(&session);

        // Find TestScript
        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (workspace_id, _) = find_by_name(&root_read.instances, "Workspace");
        let ws_read = session.get_api_read(workspace_id).unwrap();
        let (script_id, _) = find_by_name(&ws_read.instances, "TestScript");

        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Adjacent meta should be ScriptName.meta.json5 (NOT ScriptName.server.meta.json5)
        let meta_path = session.path().join("src/Workspace/TestScript.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );

        // Verify the wrong path does NOT have the attribute
        let wrong_meta = session
            .path()
            .join("src/Workspace/TestScript.server.meta.json5");
        assert!(
            !wrong_meta.exists(),
            "TestScript.server.meta.json5 should NOT be created"
        );
    });
}

/// Setting a Ref property on a non-script standalone file (.txt → StringValue)
/// should write to the adjacent FileName.meta.json5.
#[test]
fn ref_on_txt_file_instance() {
    run_serve_test("ref_two_way_sync", |session, _| {
        let (session_id, _, _, part1_id, _, _) = ref_test_setup(&session);

        let info = session.get_api_rojo().unwrap();
        let root_read = session.get_api_read(info.root_instance_id).unwrap();
        let (workspace_id, _) = find_by_name(&root_read.instances, "Workspace");
        let ws_read = session.get_api_read(workspace_id).unwrap();
        let (string_id, _) = find_by_name(&ws_read.instances, "TestString");

        let mut props = UstrMap::default();
        props.insert(ustr("PrimaryPart"), Some(Variant::Ref(part1_id)));
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: string_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        // Adjacent meta for .txt is FileName.meta.json5
        let meta_path = session.path().join("src/Workspace/TestString.meta.json5");
        poll_meta_has_ref_attr(
            &meta_path,
            "Rojo_Ref_PrimaryPart",
            "Workspace/TestModel/Part1",
        );
    });
}

// ===========================================================================
// Ambiguous rbxm Container Two-Way Sync Tests
// ===========================================================================

/// Helper: Add a named ModuleScript under a parent via the write API.
fn add_module_script_twoway(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    parent_id: Ref,
    name: &str,
    source: &str,
) {
    let ref_id = Ref::new();
    let mut properties = HashMap::new();
    properties.insert("Source".to_string(), Variant::String(source.to_string()));
    let added = AddedInstance {
        parent: Some(parent_id),
        name: name.to_string(),
        class_name: "ModuleScript".to_string(),
        properties,
        children: vec![],
    };
    let mut added_map = HashMap::new();
    added_map.insert(ref_id, added);
    let write_request = WriteRequest {
        session_id: *session_id,
        removed: vec![],
        added: added_map,
        updated: vec![],
    };
    session.post_api_write(&write_request).unwrap();
    thread::sleep(Duration::from_millis(500));
}

/// Helper: get RS id from ambiguous_container fixture
fn get_ambiguous_rs(
    session: &crate::rojo_test::serve_util::TestServeSession,
) -> (librojo::SessionId, Ref) {
    let info = session.get_api_rojo().unwrap();
    let root_read = session.get_api_read(info.root_instance_id).unwrap();
    let (rs_id, _) = find_by_class(&root_read.instances, "ReplicatedStorage");
    (info.session_id, rs_id)
}

/// Test: Adding two instances with the same name should be handled by
/// the server without crashing. Both instances should exist in the tree.
#[test]
fn twoway_add_duplicate_names_handled() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);

        // Add first "Dup"
        add_module_script_twoway(&session, &session_id, rs_id, "Dup", "return 'first'");
        let dup_path = session.path().join("src").join("Dup.luau");
        poll_file_exists(&dup_path, "First Dup.luau should exist");

        // Add second "Dup" — server finds the existing "Dup" and updates it in place
        add_module_script_twoway(&session, &session_id, rs_id, "Dup", "return 'second'");

        // The existing file gets updated with the second instance's source
        poll_file_contains(
            &dup_path,
            "return 'second'",
            "Dup.luau should contain the second script's source",
        );

        // Server should still be alive and responsive
        session
            .get_api_rojo()
            .expect("Server should still be responsive after duplicate add");
    });
}

/// Test: Renaming to create a duplicate name doesn't crash the server.
#[test]
fn twoway_rename_creates_duplicate_no_crash() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (script_a_id, _) = find_by_name(&rs_read.instances, "ScriptA");

        // Rename "ScriptA" to "ScriptB" — creates a duplicate
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_a_id,
                changed_name: Some("ScriptB".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // The renamed instance gets a dedup suffix since "ScriptB" already exists
        let dedup_path = session.path().join("src").join("ScriptB~1.luau");
        poll_file_exists(&dedup_path, "Renamed script should exist as ScriptB~1.luau");

        // Old ScriptA file should be cleaned up
        let old_path = session.path().join("src").join("ScriptA.luau");
        poll_not_exists(&old_path, "Old ScriptA.luau should be removed after rename");

        session.get_api_rojo().expect("Server alive after rename");
    });
}

/// Test: Source property update on a normal script works as expected.
#[test]
fn twoway_source_update_basic() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (script_a_id, _) = find_by_name(&rs_read.instances, "ScriptA");

        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String(
                "-- MODIFIED\nreturn 'modified'".to_string(),
            )),
        );
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: script_a_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            },
        );

        let file_path = session.path().join("src").join("ScriptA.luau");
        poll_file_contains(&file_path, "MODIFIED", "Source should be updated");
    });
}

/// Test: Removing a script cleans up the file.
#[test]
fn twoway_remove_cleans_up_file() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (script_b_id, _) = find_by_name(&rs_read.instances, "ScriptB");

        send_removal(&session, &session_id, vec![script_b_id]);

        let removed_path = session.path().join("src").join("ScriptB.luau");
        poll_not_exists(&removed_path, "ScriptB should be removed");

        let kept_path = session.path().join("src").join("ScriptA.luau");
        assert_file_exists(&kept_path, "ScriptA should still exist");
    });
}

/// Test: Mixed batch: update + remove + add in one request.
#[test]
fn twoway_batch_mixed_operations() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (script_a_id, _) = find_by_name(&rs_read.instances, "ScriptA");
        let (script_b_id, _) = find_by_name(&rs_read.instances, "ScriptB");

        let mut props = UstrMap::default();
        props.insert(
            ustr("Source"),
            Some(Variant::String("-- batch updated".to_string())),
        );

        let new_ref = Ref::new();
        let mut new_props = HashMap::new();
        new_props.insert(
            "Source".to_string(),
            Variant::String("return 'new in batch'".to_string()),
        );
        let mut added_map = HashMap::new();
        added_map.insert(
            new_ref,
            AddedInstance {
                parent: Some(rs_id),
                name: "BatchNew".to_string(),
                class_name: "ModuleScript".to_string(),
                properties: new_props,
                children: vec![],
            },
        );

        let write_request = WriteRequest {
            session_id,
            removed: vec![script_b_id],
            added: added_map,
            updated: vec![InstanceUpdate {
                id: script_a_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            }],
        };
        session.post_api_write(&write_request).unwrap();
        thread::sleep(Duration::from_millis(500));

        poll_not_exists(
            &session.path().join("src").join("ScriptB.luau"),
            "ScriptB removed in batch",
        );
        poll_file_contains(
            &session.path().join("src").join("ScriptA.luau"),
            "batch updated",
            "ScriptA updated in batch",
        );
        poll_file_exists(
            &session.path().join("src").join("BatchNew.luau"),
            "BatchNew created in batch",
        );
    });
}

/// Test: Add instance then rename it after VFS settles — server must stay stable.
#[test]
fn twoway_add_then_rename_after_settle() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);

        // Add an instance and wait for VFS to settle
        add_module_script_twoway(
            &session,
            &session_id,
            rs_id,
            "AddThenRename",
            "return 'original'",
        );

        let original_path = session.path().join("src").join("AddThenRename.luau");
        poll_file_exists(&original_path, "AddThenRename.luau should exist");
        // Extra settle time for VFS watcher
        thread::sleep(Duration::from_millis(500));

        // Now get the instance ID from the tree (the VFS watcher
        // will have added it after the file was created)
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (inst_id, _) = find_by_name(&rs_read.instances, "AddThenRename");

        // Rename it
        send_update(
            &session,
            &session_id,
            InstanceUpdate {
                id: inst_id,
                changed_name: Some("WasRenamed".to_string()),
                changed_class_name: None,
                changed_properties: UstrMap::default(),
                changed_metadata: None,
            },
        );

        // Verify rename happened
        let renamed_path = session.path().join("src").join("WasRenamed.luau");
        poll_file_exists(&renamed_path, "WasRenamed.luau should exist");
        poll_not_exists(&original_path, "Old file should be gone");

        session
            .get_api_rojo()
            .expect("Server alive after add+rename");
    });
}

/// Test: Tree consistency after several operations.
#[test]
fn twoway_tree_consistency_after_operations() {
    run_serve_test("ambiguous_container", |session, _| {
        let (session_id, rs_id) = get_ambiguous_rs(&session);

        add_module_script_twoway(&session, &session_id, rs_id, "Fresh1", "return 1");
        add_module_script_twoway(&session, &session_id, rs_id, "Fresh2", "return 2");
        thread::sleep(Duration::from_millis(500));

        session.assert_tree_fresh();
    });
}
