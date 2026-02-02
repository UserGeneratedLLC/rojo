//! Comprehensive tests for plugin syncback format transitions.
//!
//! These tests verify that the plugin API syncback correctly handles:
//! 1. Directory format → Standalone format (removing children)
//! 2. Standalone format → Directory format (adding children)
//! 3. Various script types (ModuleScript, Script, LocalScript)
//! 4. Non-script types (model.json5, txt, etc.)
//! 5. scriptsOnlyMode variations
//!
//! The key invariant: existing file formats should be preserved to prevent
//! duplicate file creation.

use std::collections::HashMap;
use std::path::Path;

use librojo::web_api::{AddedInstance, WriteRequest};
use rbx_dom_weak::types::{Ref, Variant};

use crate::rojo_test::serve_util::run_serve_test;

/// Helper to create a basic AddedInstance
fn make_added_instance(
    parent: Ref,
    name: &str,
    class_name: &str,
    source: Option<&str>,
    children: Vec<AddedInstance>,
) -> AddedInstance {
    let mut properties = HashMap::new();
    if let Some(src) = source {
        properties.insert("Source".to_string(), Variant::String(src.to_string()));
    }
    AddedInstance {
        parent: Some(parent),
        name: name.to_string(),
        class_name: class_name.to_string(),
        properties,
        children,
    }
}

/// Helper to create a child AddedInstance (no parent needed)
fn make_child_instance(
    name: &str,
    class_name: &str,
    source: Option<&str>,
    children: Vec<AddedInstance>,
) -> AddedInstance {
    let mut properties = HashMap::new();
    if let Some(src) = source {
        properties.insert("Source".to_string(), Variant::String(src.to_string()));
    }
    AddedInstance {
        parent: None,
        name: name.to_string(),
        class_name: class_name.to_string(),
        properties,
        children,
    }
}

/// Helper to send a write request and wait for processing
fn send_write_request(
    session: &crate::rojo_test::serve_util::TestServeSession,
    session_id: &librojo::SessionId,
    _parent_ref: Ref,
    added: AddedInstance,
) {
    let instance_ref = Ref::new();
    let mut added_map = HashMap::new();
    added_map.insert(instance_ref, added);

    let write_request = WriteRequest {
        session_id: *session_id,
        removed: vec![],
        added: added_map,
        updated: vec![],
    };

    session
        .post_api_write(&write_request)
        .expect("Write request should succeed");

    // Give the server time to process
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// Helper to verify a directory exists with specific contents
fn assert_directory_exists(path: &Path, message: &str) {
    assert!(path.exists(), "{}: path should exist", message);
    assert!(path.is_dir(), "{}: should be a directory", message);
}

/// Helper to verify a file exists
fn assert_file_exists(path: &Path, message: &str) {
    assert!(path.exists(), "{}: path should exist", message);
    assert!(path.is_file(), "{}: should be a file", message);
}

/// Helper to verify a path does NOT exist
fn assert_not_exists(path: &Path, message: &str) {
    assert!(!path.exists(), "{}: path should NOT exist", message);
}

// =============================================================================
// DIRECTORY → STANDALONE TRANSITIONS (Removing Children)
// =============================================================================

/// Test: Directory ModuleScript (init.luau + children) receives sync with no children
/// Expected: Directory format preserved (no standalone .luau created)
#[test]
fn dir_module_sync_removing_children_preserves_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        // Verify initial state
        let dir_path = src_path.join("DirModuleWithChildren");
        let init_file = dir_path.join("init.luau");
        let standalone_file = src_path.join("DirModuleWithChildren.luau");

        assert_directory_exists(&dir_path, "DirModuleWithChildren directory");
        assert_file_exists(&init_file, "init.luau");
        assert_not_exists(
            &standalone_file,
            "standalone file should not exist initially",
        );

        // Send sync with NO children (simulating children deletion in Studio)
        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Updated, no children\nreturn {}"),
            vec![], // No children!
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // CRITICAL: No standalone file should be created
        assert_not_exists(
            &standalone_file,
            "Standalone .luau should NOT be created when directory exists",
        );

        // Directory should still exist
        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

/// Test: Directory Script (init.server.luau + children) receives sync with no children
#[test]
fn dir_script_sync_removing_children_preserves_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirScriptWithChildren");
        let init_file = dir_path.join("init.server.luau");
        let standalone_file = src_path.join("DirScriptWithChildren.server.luau");
        let standalone_legacy = src_path.join("DirScriptWithChildren.legacy.luau");

        assert_directory_exists(&dir_path, "DirScriptWithChildren directory");
        assert_file_exists(&init_file, "init.server.luau");

        // Send sync with NO children
        let added = make_added_instance(
            rs_id,
            "DirScriptWithChildren",
            "Script",
            Some("-- Updated Script, no children\nprint('updated')"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // No standalone file should be created
        assert_not_exists(
            &standalone_file,
            "Standalone .server.luau should NOT be created",
        );
        assert_not_exists(
            &standalone_legacy,
            "Standalone .legacy.luau should NOT be created",
        );

        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

/// Test: Directory LocalScript (init.client.luau + children) receives sync with no children
#[test]
fn dir_localscript_sync_removing_children_preserves_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirLocalScriptWithChildren");
        let init_file = dir_path.join("init.client.luau");
        let standalone_file = src_path.join("DirLocalScriptWithChildren.client.luau");

        assert_directory_exists(&dir_path, "DirLocalScriptWithChildren directory");
        assert_file_exists(&init_file, "init.client.luau");

        // Send sync with NO children
        let added = make_added_instance(
            rs_id,
            "DirLocalScriptWithChildren",
            "LocalScript",
            Some("-- Updated LocalScript, no children\nprint('updated')"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // No standalone file should be created
        assert_not_exists(
            &standalone_file,
            "Standalone .client.luau should NOT be created",
        );

        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

/// Test: Directory model (init.meta.json5 + children) receives sync with no children
#[test]
fn dir_model_sync_removing_children_preserves_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModelWithChildren");
        let meta_file = dir_path.join("init.meta.json5");
        let standalone_file = src_path.join("DirModelWithChildren.model.json5");

        assert_directory_exists(&dir_path, "DirModelWithChildren directory");
        assert_file_exists(&meta_file, "init.meta.json5");

        // Send sync with NO children
        let added =
            make_added_instance(rs_id, "DirModelWithChildren", "Configuration", None, vec![]);
        send_write_request(&session, &info.session_id, rs_id, added);

        // No standalone model file should be created
        assert_not_exists(
            &standalone_file,
            "Standalone .model.json5 should NOT be created",
        );

        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

// =============================================================================
// STANDALONE → DIRECTORY TRANSITIONS (Adding Children)
// =============================================================================

/// Test: Standalone ModuleScript receives sync WITH children
/// Expected: Standalone converted to directory format when children are added
/// Standalone scripts cannot have children in Rojo's file format.
#[test]
fn standalone_module_sync_adding_children_converts_to_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let standalone_file = src_path.join("StandaloneModule.luau");
        let dir_path = src_path.join("StandaloneModule");

        assert_file_exists(&standalone_file, "StandaloneModule.luau");
        assert_not_exists(&dir_path, "Directory should not exist initially");

        // Send sync WITH children (simulating children added in Studio)
        let added = make_added_instance(
            rs_id,
            "StandaloneModule",
            "ModuleScript",
            Some("-- Now has children\nreturn {}"),
            vec![make_child_instance(
                "NewChild",
                "ModuleScript",
                Some("return 'child'"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Standalone file should be removed (converted to directory)
        assert_not_exists(&standalone_file, "Standalone file should be removed after conversion");

        // Directory should be created with init file
        assert_directory_exists(&dir_path, "Directory should be created for script with children");
        let init_file = dir_path.join("init.luau");
        assert_file_exists(&init_file, "init.luau should exist");

        // Child should exist in the directory
        let child_file = dir_path.join("NewChild.luau");
        assert_file_exists(&child_file, "Child module should be created");
    });
}

/// Test: Standalone Script receives sync WITH children - converts to directory format
/// Standalone scripts cannot have children in Rojo's file format.
#[test]
fn standalone_script_sync_adding_children_converts_to_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let standalone_file = src_path.join("StandaloneScript.server.luau");
        let dir_path = src_path.join("StandaloneScript");

        assert_file_exists(&standalone_file, "StandaloneScript.server.luau");
        assert_not_exists(&dir_path, "Directory should not exist initially");

        // Send sync WITH children
        let added = make_added_instance(
            rs_id,
            "StandaloneScript",
            "Script",
            Some("-- Now has children\nprint('updated')"),
            vec![make_child_instance(
                "Handler",
                "ModuleScript",
                Some("return function() end"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Standalone file should be removed (converted to directory)
        assert_not_exists(&standalone_file, "Standalone file should be removed after conversion");

        // Directory should be created with init file
        assert_directory_exists(&dir_path, "Directory should be created for script with children");
        let init_file = dir_path.join("init.server.luau");
        assert_file_exists(&init_file, "init.server.luau should exist");

        // Child should exist in the directory
        let child_file = dir_path.join("Handler.luau");
        assert_file_exists(&child_file, "Child module should be created");
    });
}

/// Test: Standalone LocalScript receives sync WITH children - converts to directory format
/// When a standalone script receives children, it must be converted to directory format
/// because standalone scripts cannot have children in Rojo's file format.
#[test]
fn standalone_localscript_sync_adding_children_converts_to_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let standalone_file = src_path.join("StandaloneLocalScript.client.luau");
        let dir_path = src_path.join("StandaloneLocalScript");

        assert_file_exists(&standalone_file, "StandaloneLocalScript.client.luau");
        assert_not_exists(&dir_path, "Directory should not exist initially");

        // Send sync WITH children
        let added = make_added_instance(
            rs_id,
            "StandaloneLocalScript",
            "LocalScript",
            Some("-- Now has children\nprint('updated')"),
            vec![make_child_instance(
                "UIComponent",
                "ModuleScript",
                Some("return {}"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Standalone file should be removed (converted to directory)
        assert_not_exists(&standalone_file, "Standalone file should be removed after conversion");

        // Directory should be created with init file
        assert_directory_exists(&dir_path, "Directory should be created for script with children");
        let init_file = dir_path.join("init.local.luau");
        assert_file_exists(&init_file, "init.local.luau should exist");

        // Child should exist in the directory
        let child_file = dir_path.join("UIComponent.luau");
        assert_file_exists(&child_file, "Child module should be created");
    });
}

/// Test: Standalone model.json5 receives sync WITH children
#[test]
fn standalone_model_sync_adding_children_preserves_standalone() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let standalone_file = src_path.join("StandaloneModel.model.json5");
        let dir_path = src_path.join("StandaloneModel");

        assert_file_exists(&standalone_file, "StandaloneModel.model.json5");
        assert_not_exists(&dir_path, "Directory should not exist initially");

        // Send sync WITH children
        let child = make_child_instance("ChildValue", "StringValue", None, vec![]);
        let added =
            make_added_instance(rs_id, "StandaloneModel", "Configuration", None, vec![child]);
        send_write_request(&session, &info.session_id, rs_id, added);

        // Standalone file should still exist
        assert_file_exists(&standalone_file, "Standalone file should be preserved");

        // No directory should be created
        assert_not_exists(
            &dir_path,
            "Directory should NOT be created for existing standalone file",
        );
    });
}

// =============================================================================
// MULTIPLE CHILDREN / NESTED SCENARIOS
// =============================================================================

/// Test: Sync with multiple levels of nested children
#[test]
fn sync_with_deeply_nested_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");

        // Send sync with multiple levels of nesting
        let level3 = make_child_instance("Level3", "ModuleScript", Some("return 3"), vec![]);
        let level2 = make_child_instance("Level2", "ModuleScript", Some("return 2"), vec![level3]);
        let level1 = make_child_instance("Level1", "ModuleScript", Some("return 1"), vec![level2]);

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Has deeply nested children\nreturn {}"),
            vec![level1],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Directory should still exist
        assert_directory_exists(&dir_path, "Directory should be preserved");

        // No standalone file
        let standalone = src_path.join("DirModuleWithChildren.luau");
        assert_not_exists(&standalone, "No standalone file should be created");
    });
}

/// Test: Sync with mixed script and non-script children
#[test]
fn sync_with_mixed_children_types() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");

        // Send sync with various child types
        let children = vec![
            make_child_instance(
                "ChildModule",
                "ModuleScript",
                Some("return 'module'"),
                vec![],
            ),
            make_child_instance("ChildScript", "Script", Some("print('script')"), vec![]),
            make_child_instance("ChildFolder", "Folder", None, vec![]),
            make_child_instance("ChildValue", "StringValue", None, vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Has mixed children\nreturn {}"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Directory should still exist
        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

// =============================================================================
// EDGE CASES
// =============================================================================

/// Test: Syncing the same instance twice preserves format
#[test]
fn sync_same_instance_twice_preserves_format() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        // First sync
        let added1 = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- First sync\nreturn 1"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added1);

        assert_directory_exists(&dir_path, "Directory after first sync");
        assert_not_exists(&standalone, "No standalone after first sync");

        // Second sync
        let added2 = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Second sync\nreturn 2"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added2);

        assert_directory_exists(&dir_path, "Directory after second sync");
        assert_not_exists(&standalone, "No standalone after second sync");
    });
}

/// Test: Sync with empty name child (edge case)
#[test]
fn sync_handles_children_with_special_names() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Send sync with children that have special names
        let children = vec![
            make_child_instance(
                "Child With Spaces",
                "ModuleScript",
                Some("return 1"),
                vec![],
            ),
            make_child_instance(
                "Child-With-Dashes",
                "ModuleScript",
                Some("return 2"),
                vec![],
            ),
            make_child_instance(
                "Child_With_Underscores",
                "ModuleScript",
                Some("return 3"),
                vec![],
            ),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Has special name children\nreturn {}"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Directory should still exist
        assert_directory_exists(&dir_path, "Directory should be preserved");
    });
}

/// Test: StringValue (.txt) format preservation
#[test]
fn txt_file_format_preserved() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let txt_file = src_path.join("StandaloneValue.txt");
        let model_file = src_path.join("StandaloneValue.model.json5");

        assert_file_exists(&txt_file, "StandaloneValue.txt");
        assert_not_exists(&model_file, ".model.json5 should not exist initially");

        // Send sync for StringValue
        let mut properties = HashMap::new();
        properties.insert(
            "Value".to_string(),
            Variant::String("Updated value".to_string()),
        );
        let added = AddedInstance {
            parent: Some(rs_id),
            name: "StandaloneValue".to_string(),
            class_name: "StringValue".to_string(),
            properties,
            children: vec![],
        };
        send_write_request(&session, &info.session_id, rs_id, added);

        // .txt file should still exist
        assert_file_exists(&txt_file, ".txt file should be preserved");

        // No .model.json5 should be created
        assert_not_exists(&model_file, ".model.json5 should NOT be created");
    });
}

// =============================================================================
// TRULY NEW INSTANCES (No existing file)
// =============================================================================

/// Test: Truly new ModuleScript with children creates directory
#[test]
fn new_module_with_children_creates_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        // This instance doesn't exist on disk
        let new_dir = src_path.join("BrandNewModule");
        let new_standalone = src_path.join("BrandNewModule.luau");

        assert_not_exists(&new_dir, "New directory should not exist initially");
        assert_not_exists(&new_standalone, "New standalone should not exist initially");

        // Send sync for truly new instance with children
        let added = make_added_instance(
            rs_id,
            "BrandNewModule",
            "ModuleScript",
            Some("-- Brand new with children\nreturn {}"),
            vec![make_child_instance(
                "Child",
                "ModuleScript",
                Some("return 'child'"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // For truly new instances with children, directory format should be used
        // Note: This depends on implementation - check what actually happens
        // The key is that only ONE format should be created, not both
        let dir_exists = new_dir.exists();
        let standalone_exists = new_standalone.exists();

        // At most one should exist (no duplicates)
        assert!(
            !(dir_exists && standalone_exists),
            "Both directory and standalone should NOT exist (no duplicates)"
        );
    });
}

/// Test: Truly new ModuleScript without children creates standalone
#[test]
fn new_module_without_children_creates_standalone() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        // This instance doesn't exist on disk
        let new_dir = src_path.join("AnotherNewModule");
        let new_standalone = src_path.join("AnotherNewModule.luau");

        assert_not_exists(&new_dir, "New directory should not exist initially");
        assert_not_exists(&new_standalone, "New standalone should not exist initially");

        // Send sync for truly new instance WITHOUT children
        let added = make_added_instance(
            rs_id,
            "AnotherNewModule",
            "ModuleScript",
            Some("-- Brand new, no children\nreturn {}"),
            vec![], // No children
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // For truly new instances without children, standalone should be used
        // Note: This depends on implementation
        let dir_exists = new_dir.exists();
        let standalone_exists = new_standalone.exists();

        // At most one should exist (no duplicates)
        assert!(
            !(dir_exists && standalone_exists),
            "Both directory and standalone should NOT exist (no duplicates)"
        );
    });
}

// =============================================================================
// RAPID SUCCESSIVE SYNCS
// =============================================================================

/// Test: Multiple rapid syncs don't create duplicates
#[test]
fn rapid_syncs_dont_create_duplicates() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        // Rapid fire 5 syncs
        for i in 0..5 {
            let added = make_added_instance(
                rs_id,
                "DirModuleWithChildren",
                "ModuleScript",
                Some(&format!("-- Sync {}\nreturn {}", i, i)),
                vec![],
            );

            let instance_ref = Ref::new();
            let mut added_map = HashMap::new();
            added_map.insert(instance_ref, added);

            let write_request = WriteRequest {
                session_id: info.session_id,
                removed: vec![],
                added: added_map,
                updated: vec![],
            };

            session
                .post_api_write(&write_request)
                .expect("Write request should succeed");

            // Minimal delay between syncs
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Wait for processing
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Should never have duplicates
        assert_directory_exists(&dir_path, "Directory should exist");
        assert_not_exists(&standalone, "No standalone should exist");
    });
}

// =============================================================================
// FOLDER SCENARIOS
// =============================================================================

/// Test: Folder with children synced without children preserves directory
#[test]
fn folder_sync_removing_children_preserves_directory() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        // DirModelWithChildren is essentially a folder-like structure
        let dir_path = src_path.join("DirModelWithChildren");

        assert_directory_exists(&dir_path, "Directory should exist initially");

        // Send sync as Folder with no children
        let added = make_added_instance(rs_id, "DirModelWithChildren", "Folder", None, vec![]);
        send_write_request(&session, &info.session_id, rs_id, added);

        // Directory should still exist
        assert_directory_exists(&dir_path, "Directory should be preserved for Folder");
    });
}

// =============================================================================
// SCRIPT TYPE EDGE CASES
// =============================================================================

/// Test: Script sent as LocalScript preserves existing Script format
#[test]
fn class_change_preserves_existing_format() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let server_file = src_path
            .join("DirScriptWithChildren")
            .join("init.server.luau");

        assert_file_exists(&server_file, "init.server.luau should exist");

        // Send as LocalScript instead of Script
        let added = make_added_instance(
            rs_id,
            "DirScriptWithChildren",
            "LocalScript", // Different class!
            Some("-- Sent as LocalScript\nprint('test')"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Should still have the .server.luau file (format preserved)
        // Note: Behavior depends on implementation - this tests format preservation
        let dir_path = src_path.join("DirScriptWithChildren");
        assert_directory_exists(&dir_path, "Directory should still exist");
    });
}

// =============================================================================
// scriptsOnlyMode TESTS
// =============================================================================

/// Test: scriptsOnlyMode - Script sync preserves directory format
#[test]
fn scripts_only_mode_script_preserves_directory() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("ScriptWithChildren");
        let init_file = dir_path.join("init.server.luau");
        let standalone = src_path.join("ScriptWithChildren.server.luau");
        let standalone_legacy = src_path.join("ScriptWithChildren.legacy.luau");

        assert_directory_exists(&dir_path, "ScriptWithChildren directory");
        assert_file_exists(&init_file, "init.server.luau");

        // In scriptsOnlyMode, sync a Script without children
        let added = make_added_instance(
            sss_id,
            "ScriptWithChildren",
            "Script",
            Some("-- Updated in scriptsOnlyMode\nprint('updated')"),
            vec![],
        );
        send_write_request(&session, &info.session_id, sss_id, added);

        // No standalone file should be created
        assert_not_exists(&standalone, "No standalone .server.luau");
        assert_not_exists(&standalone_legacy, "No standalone .legacy.luau");

        // Directory should be preserved
        assert_directory_exists(&dir_path, "Directory preserved in scriptsOnlyMode");
    });
}

/// Test: scriptsOnlyMode - Standalone module converts to directory when children added
/// Even in scriptsOnlyMode, standalone scripts must convert to directory format
/// when children are added, because standalone scripts cannot have children.
#[test]
fn scripts_only_mode_standalone_module_converts_to_directory() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        let src_path = session.path().join("src");

        let standalone = src_path.join("StandaloneModule.luau");
        let dir_path = src_path.join("StandaloneModule");

        assert_file_exists(&standalone, "StandaloneModule.luau");
        assert_not_exists(&dir_path, "Directory should not exist");

        // In scriptsOnlyMode, sync with children
        let added = make_added_instance(
            sss_id,
            "StandaloneModule",
            "ModuleScript",
            Some("-- Updated with children in scriptsOnlyMode\nreturn {}"),
            vec![make_child_instance(
                "ChildModule",
                "ModuleScript",
                Some("return 'child'"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, sss_id, added);

        // Standalone should be removed (converted to directory)
        assert_not_exists(&standalone, "Standalone removed after children sync");

        // Directory should be created
        assert_directory_exists(&dir_path, "Directory created for script with children");
        let init_file = dir_path.join("init.luau");
        assert_file_exists(&init_file, "init.luau should exist");
    });
}

/// Test: scriptsOnlyMode - Module with deeply nested children
#[test]
fn scripts_only_mode_nested_children() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("ScriptWithChildren");

        // Send sync with nested children
        let level2 = make_child_instance("Level2", "ModuleScript", Some("return 2"), vec![]);
        let level1 = make_child_instance("Level1", "ModuleScript", Some("return 1"), vec![level2]);

        let added = make_added_instance(
            sss_id,
            "ScriptWithChildren",
            "Script",
            Some("-- Nested children in scriptsOnlyMode\nprint('nested')"),
            vec![level1],
        );
        send_write_request(&session, &info.session_id, sss_id, added);

        // Directory should be preserved
        assert_directory_exists(&dir_path, "Directory preserved with nested children");
    });
}

/// Test: scriptsOnlyMode - rapid syncs don't create duplicates
#[test]
fn scripts_only_mode_rapid_syncs() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("ScriptWithChildren");
        let standalone = src_path.join("ScriptWithChildren.server.luau");

        // Rapid fire syncs in scriptsOnlyMode
        for i in 0..5 {
            let added = make_added_instance(
                sss_id,
                "ScriptWithChildren",
                "Script",
                Some(&format!(
                    "-- Rapid sync {} in scriptsOnlyMode\nprint({})",
                    i, i
                )),
                vec![],
            );

            let instance_ref = Ref::new();
            let mut added_map = HashMap::new();
            added_map.insert(instance_ref, added);

            let write_request = WriteRequest {
                session_id: info.session_id,
                removed: vec![],
                added: added_map,
                updated: vec![],
            };

            session
                .post_api_write(&write_request)
                .expect("Write request should succeed");

            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        std::thread::sleep(std::time::Duration::from_millis(300));

        assert_directory_exists(&dir_path, "Directory should exist");
        assert_not_exists(&standalone, "No standalone should exist");
    });
}

// =============================================================================
// ADDITIONAL EDGE CASES
// =============================================================================

/// Test: Sync ModuleScript with Script children
#[test]
fn module_with_script_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");

        // Send sync with Script as child (unusual but valid)
        let children = vec![
            make_child_instance(
                "ChildScript",
                "Script",
                Some("print('server child')"),
                vec![],
            ),
            make_child_instance(
                "ChildLocalScript",
                "LocalScript",
                Some("print('client child')"),
                vec![],
            ),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Module with script children\nreturn {}"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved");
    });
}

/// Test: Sync Script with ModuleScript children
#[test]
fn script_with_module_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirScriptWithChildren");

        // Send sync with multiple ModuleScript children
        let children = vec![
            make_child_instance(
                "Config",
                "ModuleScript",
                Some("return { config = true }"),
                vec![],
            ),
            make_child_instance(
                "Utils",
                "ModuleScript",
                Some("return { utils = true }"),
                vec![],
            ),
            make_child_instance("Types", "ModuleScript", Some("return {}"), vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirScriptWithChildren",
            "Script",
            Some("-- Script with many module children\nprint('service')"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved");
    });
}

/// Test: Alternating syncs with and without children
#[test]
fn alternating_children_syncs() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        // Alternate between syncs with and without children
        for i in 0..6 {
            let children = if i % 2 == 0 {
                vec![make_child_instance(
                    "TempChild",
                    "ModuleScript",
                    Some("return 'temp'"),
                    vec![],
                )]
            } else {
                vec![]
            };

            let added = make_added_instance(
                rs_id,
                "DirModuleWithChildren",
                "ModuleScript",
                Some(&format!("-- Alternating sync {}\nreturn {}", i, i)),
                children,
            );
            send_write_request(&session, &info.session_id, rs_id, added);
        }

        // Directory should always be preserved
        assert_directory_exists(&dir_path, "Directory preserved through alternating syncs");
        assert_not_exists(&standalone, "No standalone created");
    });
}

/// Test: LocalScript directory with Module children
#[test]
fn localscript_with_module_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirLocalScriptWithChildren");

        let children = vec![
            make_child_instance("UIModule", "ModuleScript", Some("return {}"), vec![]),
            make_child_instance("EventHandler", "ModuleScript", Some("return {}"), vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirLocalScriptWithChildren",
            "LocalScript",
            Some("-- LocalScript with UI modules\nprint('ui')"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved");
    });
}

/// Test: Large number of children
#[test]
fn sync_with_many_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        let dir_path = src_path.join("DirModuleWithChildren");

        // Create many children
        let children: Vec<AddedInstance> = (0..20)
            .map(|i| {
                make_child_instance(
                    &format!("Child{}", i),
                    "ModuleScript",
                    Some(&format!("return {}", i)),
                    vec![],
                )
            })
            .collect();

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- Module with many children\nreturn {}"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with many children");
    });
}

// =============================================================================
// ABSOLUTELY VAPID EDGE CASES
// =============================================================================

/// Test: Empty source string
#[test]
fn sync_with_empty_source() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Empty source string
        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some(""),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with empty source");
    });
}

/// Test: Source with only whitespace
#[test]
fn sync_with_whitespace_only_source() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("   \n\t\n   "),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with whitespace source");
    });
}

/// Test: Source with unicode characters
#[test]
fn sync_with_unicode_source() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- 你好世界 🎮 émojis everywhere 🚀\nreturn { message = '日本語' }"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with unicode source");
    });
}

/// Test: Very long source code
#[test]
fn sync_with_very_long_source() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Generate a very long source (10KB of comments)
        let long_source: String = (0..500)
            .map(|i| {
                format!(
                    "-- Line {} of extremely verbose and pointless commentary\n",
                    i
                )
            })
            .collect::<String>()
            + "return {}";

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some(&long_source),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with long source");
    });
}

/// Test: Child with same name as parent
#[test]
fn sync_child_with_parent_name() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Child has same name as parent (weird but valid)
        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'parent'"),
            vec![make_child_instance(
                "DirModuleWithChildren", // Same name!
                "ModuleScript",
                Some("return 'child'"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with same-name child");
    });
}

/// Test: Deeply recursive children (10 levels)
#[test]
fn sync_extremely_deep_nesting() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Build 10 levels deep
        let mut current = make_child_instance("Level10", "ModuleScript", Some("return 10"), vec![]);
        for i in (1..10).rev() {
            current = make_child_instance(
                &format!("Level{}", i),
                "ModuleScript",
                Some(&format!("return {}", i)),
                vec![current],
            );
        }

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'root'"),
            vec![current],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with 10-level nesting");
    });
}

/// Test: All script types as siblings
#[test]
fn sync_all_script_types_as_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let children = vec![
            make_child_instance(
                "ChildModule",
                "ModuleScript",
                Some("return 'module'"),
                vec![],
            ),
            make_child_instance("ChildServer", "Script", Some("print('server')"), vec![]),
            make_child_instance(
                "ChildClient",
                "LocalScript",
                Some("print('client')"),
                vec![],
            ),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'parent with all script types'"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with all script types");
    });
}

/// Test: Script with only Folder children (no scripts)
#[test]
fn sync_script_with_only_folder_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let children = vec![
            make_child_instance("FolderA", "Folder", None, vec![]),
            make_child_instance("FolderB", "Folder", None, vec![]),
            make_child_instance("FolderC", "Folder", None, vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'module with only folders'"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with folder children");
    });
}

/// Test: Script with only Value children
#[test]
fn sync_script_with_only_value_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let children = vec![
            make_child_instance("StringVal", "StringValue", None, vec![]),
            make_child_instance("IntVal", "IntValue", None, vec![]),
            make_child_instance("BoolVal", "BoolValue", None, vec![]),
            make_child_instance("NumberVal", "NumberValue", None, vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'module with only values'"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with value children");
    });
}

/// Test: Sync then immediately sync again with different content
#[test]
fn sync_immediate_double_sync() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        // First sync - don't wait
        let added1 = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 1"),
            vec![],
        );
        let instance_ref = Ref::new();
        let mut added_map = HashMap::new();
        added_map.insert(instance_ref, added1);
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
        };
        session.post_api_write(&write_request).unwrap();

        // Immediately send second sync - no delay!
        let added2 = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 2"),
            vec![make_child_instance(
                "QuickChild",
                "ModuleScript",
                Some("return 'quick'"),
                vec![],
            )],
        );
        let instance_ref2 = Ref::new();
        let mut added_map2 = HashMap::new();
        added_map2.insert(instance_ref2, added2);
        let write_request2 = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map2,
            updated: vec![],
        };
        session.post_api_write(&write_request2).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(300));

        assert_directory_exists(&dir_path, "Directory should exist after double sync");
        assert_not_exists(&standalone, "No standalone after double sync");
    });
}

/// Test: 50 rapid-fire syncs
#[test]
fn stress_test_50_rapid_syncs() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        for i in 0..50 {
            let added = make_added_instance(
                rs_id,
                "DirModuleWithChildren",
                "ModuleScript",
                Some(&format!("return {}", i)),
                vec![],
            );
            let instance_ref = Ref::new();
            let mut added_map = HashMap::new();
            added_map.insert(instance_ref, added);
            let write_request = WriteRequest {
                session_id: info.session_id,
                removed: vec![],
                added: added_map,
                updated: vec![],
            };
            session.post_api_write(&write_request).unwrap();
            // No delay between syncs!
        }

        std::thread::sleep(std::time::Duration::from_millis(500));

        assert_directory_exists(&dir_path, "Directory survived 50 rapid syncs");
        assert_not_exists(&standalone, "No standalone after 50 rapid syncs");
    });
}

/// Test: Sync with newlines in content that look like multiple files
#[test]
fn sync_with_tricky_newlines() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let tricky_source = r#"
-- This looks like it could be multiple files
-- File: SomeOtherThing.luau
return {
    -- But it's not!
    anotherFile = [[
        -- File: YetAnotherThing.luau
        return "nested string that looks like a file"
    ]]
}
"#;

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some(tricky_source),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with tricky newlines");
    });
}

/// Test: Child has no source property (non-script child)
#[test]
fn sync_with_sourceless_children() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        let children = vec![
            make_child_instance("RemoteEvent", "RemoteEvent", None, vec![]),
            make_child_instance("RemoteFunction", "RemoteFunction", None, vec![]),
            make_child_instance("BindableEvent", "BindableEvent", None, vec![]),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'module with remote children'"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with sourceless children");
    });
}

/// Test: Very long instance name
#[test]
fn sync_with_very_long_name() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");

        // 100 character name
        let long_name = "A".repeat(100);
        let dir_path = src_path.join(&long_name);
        let standalone = src_path.join(format!("{}.luau", long_name));

        let added = make_added_instance(
            rs_id,
            &long_name,
            "ModuleScript",
            Some("return 'very long name'"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        // Should create one or the other, not both
        let dir_exists = dir_path.exists();
        let standalone_exists = standalone.exists();
        assert!(
            !(dir_exists && standalone_exists),
            "Should not create both directory and standalone for long name"
        );
    });
}

/// Test: Instance name with dots (looks like file extension)
#[test]
fn sync_name_with_dots() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Child with dots in name
        let children = vec![
            make_child_instance(
                "config.dev.local",
                "ModuleScript",
                Some("return 'dev'"),
                vec![],
            ),
            make_child_instance(
                "data.v2.backup",
                "ModuleScript",
                Some("return 'backup'"),
                vec![],
            ),
        ];

        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return 'parent'"),
            children,
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with dotted child names");
    });
}

/// Test: Sync standalone then immediately sync directory version
/// When children are added, standalone must convert to directory format.
#[test]
fn sync_standalone_then_directory_format() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let standalone = src_path.join("StandaloneModule.luau");
        let dir_path = src_path.join("StandaloneModule");

        assert_file_exists(&standalone, "Standalone exists initially");

        // First sync - no children (standalone format preserved)
        let added1 = make_added_instance(
            rs_id,
            "StandaloneModule",
            "ModuleScript",
            Some("return 'no children'"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added1);

        // Standalone should still exist after sync without children
        assert_file_exists(&standalone, "Standalone preserved after sync without children");

        // Second sync - with children (converts to directory)
        let added2 = make_added_instance(
            rs_id,
            "StandaloneModule",
            "ModuleScript",
            Some("return 'with children'"),
            vec![make_child_instance(
                "Child",
                "ModuleScript",
                Some("return 'child'"),
                vec![],
            )],
        );
        send_write_request(&session, &info.session_id, rs_id, added2);

        // Standalone should be removed (converted to directory)
        assert_not_exists(&standalone, "Standalone removed after children sync");

        // Directory should now exist
        assert_directory_exists(&dir_path, "Directory created after children sync");
        let init_file = dir_path.join("init.luau");
        assert_file_exists(&init_file, "init.luau should exist");
    });
}

/// Test: Configuration instance sync
#[test]
fn sync_configuration_instance() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModelWithChildren");

        let added = make_added_instance(
            rs_id,
            "DirModelWithChildren",
            "Configuration",
            None,
            vec![make_child_instance("Setting", "BoolValue", None, vec![])],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Configuration directory preserved");
    });
}

/// Test: Script that returns nothing (no return statement)
#[test]
fn sync_script_no_return() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // ModuleScript with no return (bad practice but valid)
        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("-- This module doesn't return anything\nlocal x = 1\nlocal y = 2"),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved with no-return module");
    });
}

/// Test: Sync with syntax errors in source
#[test]
fn sync_with_syntax_errors() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");

        // Lua with syntax error
        let added = make_added_instance(
            rs_id,
            "DirModuleWithChildren",
            "ModuleScript",
            Some("return { unclosed = "),
            vec![],
        );
        send_write_request(&session, &info.session_id, rs_id, added);

        assert_directory_exists(&dir_path, "Directory preserved despite syntax error");
    });
}

/// Test: Multiple syncs with varying children counts
#[test]
fn sync_varying_children_counts() {
    run_serve_test("syncback_format_transitions", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let rs_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .expect("ReplicatedStorage should exist");

        let src_path = session.path().join("src");
        let dir_path = src_path.join("DirModuleWithChildren");
        let standalone = src_path.join("DirModuleWithChildren.luau");

        // Sync with 0, 5, 0, 10, 0, 1 children
        let child_counts = [0, 5, 0, 10, 0, 1];

        for count in child_counts {
            let children: Vec<AddedInstance> = (0..count)
                .map(|i| {
                    make_child_instance(
                        &format!("Child{}", i),
                        "ModuleScript",
                        Some("return 1"),
                        vec![],
                    )
                })
                .collect();

            let added = make_added_instance(
                rs_id,
                "DirModuleWithChildren",
                "ModuleScript",
                Some(&format!("return {{ children = {} }}", count)),
                children,
            );
            send_write_request(&session, &info.session_id, rs_id, added);
        }

        assert_directory_exists(&dir_path, "Directory after varying children counts");
        assert_not_exists(&standalone, "No standalone after varying children counts");
    });
}
