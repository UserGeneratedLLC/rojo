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
//! NOTE: Tests operate on pre-existing instances already in the Rojo tree
//! (from the initial snapshot). Adding new instances via the API writes files
//! to disk but relies on VFS events for tree updates; with echo suppression,
//! those events are suppressed for the paths the API wrote. This is correct
//! behavior — the plugin reconciles its own tree and doesn't need Rojo to
//! re-snapshot what it just sent. Tests that need to interact with the tree
//! (read back IDs, send updates) must use instances from the initial snapshot.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use std::{fs, thread};

use librojo::web_api::{AddedInstance, InstanceUpdate, WriteRequest};
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, UstrMap};

use crate::rojo_test::serve_util::run_serve_test;

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

        let updated = fs::read_to_string(&file_path).unwrap();
        assert!(
            updated.contains("Updated via two-way sync"),
            "File should contain new Source, got: {}",
            updated
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

        assert_not_exists(&old_path, "Old file after rename");
        assert_file_exists(&new_path, "New file after rename");

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

        assert_not_exists(
            &session.path().join("src").join("existing.luau"),
            "Old script gone",
        );
        assert_not_exists(&old_meta, "Old meta gone");
        assert_file_exists(&new_script, "Renamed script exists");
        assert_file_exists(&new_meta, "Renamed meta exists");
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

        assert_not_exists(&module_path, "Module file after class change");
        assert_file_exists(&script_path, "Script file after class change");
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

        assert_not_exists(&module_path, "Module file after class change");
        assert_file_exists(&local_path, "LocalScript file after class change");
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
        let content = fs::read_to_string(&file).unwrap();
        assert!(content.contains("Round trip v1"), "Step 1: Source updated");

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

        assert_not_exists(&file, "Step 2: old file gone");
        let new_file = src.join("GameService.luau");
        assert_file_exists(&new_file, "Step 2: renamed file exists");
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

        // With proper echo suppression, the cursor should not advance excessively
        assert!(
            cursor_delta < 10,
            "Message cursor should not advance excessively (delta={}), \
             indicating echo suppression is working",
            cursor_delta
        );
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

        assert!(
            !old_dir.exists(),
            "Old directory should be gone after rename: {}",
            old_dir.display()
        );
        assert!(
            new_dir.is_dir(),
            "New directory should exist after rename: {}",
            new_dir.display()
        );

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

        assert_not_exists(&old_init, "init.luau after class change to Script");
        assert_file_exists(&new_init, "init.server.luau after class change to Script");

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

        assert_not_exists(&old_path, "Script file after class change to ModuleScript");
        assert_file_exists(&new_path, "ModuleScript file after class change");
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

        assert_not_exists(&old_path, "Script file after class change to LocalScript");
        assert_file_exists(&new_path, "LocalScript file after class change");
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

        assert_not_exists(&old_path, "LocalScript file after class change to Script");
        assert_file_exists(&new_path, "Script file after class change");
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

        assert_not_exists(&old_path, "Old file after combined rename+source");
        assert_file_exists(&new_path, "Renamed file after combined rename+source");

        let content = fs::read_to_string(&new_path).unwrap();
        assert!(
            content.contains("Written after rename"),
            "Source should be written to the NEW file location, got: {}",
            content
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

        assert_not_exists(&old_path, "ModuleScript file after ClassName+Source");
        assert_file_exists(&new_path, "Script file after ClassName+Source");

        let content = fs::read_to_string(&new_path).unwrap();
        assert!(
            content.contains("Source after class change"),
            "Source should be in the new file, got: {}",
            content
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
// Tests: Encoded name directory conversion (path encoding fix)
// ---------------------------------------------------------------------------

/// Test 22: Adding a child to a standalone ModuleScript with Rojo-encoded
/// characters (e.g., `What%QUESTION%Module.luau`) creates the directory using
/// the filesystem-encoded name (`What%QUESTION%Module/`), NOT the decoded
/// instance name (`What?Module/`).
///
/// This is the core regression test for the path encoding fix in
/// `convert_standalone_script_to_directory`.
#[test]
fn add_child_to_encoded_module_creates_encoded_directory() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_module_id) =
            get_encoded_names_instance(&session, "What?Module");

        let standalone_file = session
            .path()
            .join("src")
            .join("What%QUESTION%Module.luau");
        // The CORRECT directory name uses the encoded filesystem name
        let encoded_dir = session.path().join("src").join("What%QUESTION%Module");
        // The WRONG directory name would use the decoded instance name
        let decoded_dir = session.path().join("src").join("What?Module");

        assert_file_exists(&standalone_file, "Encoded standalone file before child add");

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

        // Directory should use ENCODED name (the fix)
        assert!(
            encoded_dir.is_dir(),
            "Directory should use encoded name (What%QUESTION%Module/): {}",
            encoded_dir.display()
        );

        // Decoded name directory should NOT exist (the bug we fixed)
        assert!(
            !decoded_dir.exists(),
            "Directory with decoded name (What?Module/) must NOT be created: {}",
            decoded_dir.display()
        );

        // init.luau should exist inside the encoded directory
        let init_file = encoded_dir.join("init.luau");
        assert_file_exists(&init_file, "init.luau inside encoded directory");

        // Child should be created inside the encoded directory
        let child_file = encoded_dir.join("EncodedChild.luau");
        assert_file_exists(&child_file, "Child inside encoded directory");
    });
}

/// Test 23: Adding a child to a standalone Script with Rojo-encoded characters
/// AND a script suffix (e.g., `Key%COLON%Script.server.luau`) strips the suffix
/// correctly when creating the directory name.
///
/// The directory should be `Key%COLON%Script/`, not `Key%COLON%Script.server/`
/// or `Key:Script/`.
#[test]
fn add_child_to_encoded_script_strips_suffix_correctly() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_script_id) =
            get_encoded_names_instance(&session, "Key:Script");

        let standalone_file = session
            .path()
            .join("src")
            .join("Key%COLON%Script.server.luau");
        let encoded_dir = session.path().join("src").join("Key%COLON%Script");
        // Wrong: decoded name
        let decoded_dir = session.path().join("src").join("Key:Script");
        // Wrong: suffix not stripped
        let unsuffixed_dir = session.path().join("src").join("Key%COLON%Script.server");

        assert_file_exists(&standalone_file, "Encoded script before child add");

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
            "Directory should be Key%COLON%Script/ (suffix stripped, encoded): {}",
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
        assert_file_exists(&init_file, "init.server.luau inside encoded directory");

        let child_file = encoded_dir.join("ScriptChild.luau");
        assert_file_exists(&child_file, "Child inside encoded script directory");
    });
}

/// Test 24: Adding a child to a non-script standalone instance with Rojo-encoded
/// characters (e.g., `What%QUESTION%Model.model.json5`) creates the directory
/// using the encoded name with the compound extension stripped.
///
/// This exercises `convert_standalone_instance_to_directory`.
#[test]
fn add_child_to_encoded_model_creates_encoded_directory() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_model_id) =
            get_encoded_names_instance(&session, "What?Model");

        let standalone_file = session
            .path()
            .join("src")
            .join("What%QUESTION%Model.model.json5");
        let encoded_dir = session.path().join("src").join("What%QUESTION%Model");
        let decoded_dir = session.path().join("src").join("What?Model");

        assert_file_exists(&standalone_file, "Encoded model file before child add");

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
            "Directory should use encoded name (What%QUESTION%Model/): {}",
            encoded_dir.display()
        );
        assert!(
            !decoded_dir.exists(),
            "Directory with decoded name must NOT be created: {}",
            decoded_dir.display()
        );

        // init.meta.json5 should exist inside the directory (converted from .model.json5)
        let init_meta = encoded_dir.join("init.meta.json5");
        assert_file_exists(&init_meta, "init.meta.json5 inside encoded model directory");

        let child_file = encoded_dir.join("ModelChild.luau");
        assert_file_exists(&child_file, "Child inside encoded model directory");
    });
}

/// Test 25: Non-Source property update on a standalone script with Rojo-encoded
/// name creates the adjacent meta file using the encoded filesystem name,
/// not the decoded instance name.
///
/// e.g., `What%QUESTION%Module.meta.json5` NOT `What?Module.meta.json5`
#[test]
fn property_update_on_encoded_script_uses_encoded_meta_path() {
    run_serve_test("syncback_encoded_names", |session, _redactions| {
        let (session_id, encoded_module_id) =
            get_encoded_names_instance(&session, "What?Module");

        let script_file = session
            .path()
            .join("src")
            .join("What%QUESTION%Module.luau");
        let encoded_meta = session
            .path()
            .join("src")
            .join("What%QUESTION%Module.meta.json5");
        let decoded_meta = session
            .path()
            .join("src")
            .join("What?Module.meta.json5");

        assert_file_exists(&script_file, "Encoded script file before property update");
        assert_not_exists(&encoded_meta, "Meta file before property update");

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

        // Meta file should use encoded name
        assert_file_exists(
            &encoded_meta,
            "Meta file should use encoded name (What%QUESTION%Module.meta.json5)",
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
