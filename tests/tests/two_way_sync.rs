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
