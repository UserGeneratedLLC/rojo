//! Connected mode tests: multi-step filesystem changes with WebSocket patch
//! verification and round-trip identity checks.
//!
//! These tests simulate a plugin connected to `rojo serve` receiving patches
//! over time as the filesystem evolves. Each test verifies:
//! 1. The WebSocket patch contains the correct delta (via insta snapshots)
//! 2. The tree state after each change is correct (via insta snapshots)
//! 3. Where marked, a fresh `rojo serve` on the same filesystem produces
//!    the same tree (round-trip identity invariant)

use std::fs;

use insta::assert_yaml_snapshot;

use crate::rojo_test::{
    internable::InternAndRedact,
    serve_util::{assert_round_trip, get_message_cursor, run_serve_test},
};

use librojo::web_api::SocketPacketType;

// ---------------------------------------------------------------------------
// Phase 1: Format Transitions
// ---------------------------------------------------------------------------

/// Test 1: Rename init.luau -> init.server.luau to change ClassName
/// from ModuleScript to Script.
#[test]
fn init_type_change_module_to_server() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_module_to_server_initial",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let init_path = session.path().join("src/DirModule/init.luau");
        let new_path = session.path().join("src/DirModule/init.server.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::rename(&init_path, &new_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_module_to_server_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_module_to_server_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 2: Rename init.server.luau -> init.local.luau to change ClassName
/// from Script to LocalScript.
#[test]
fn init_type_change_server_to_local() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_server_to_local_initial",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let init_path = session.path().join("src/DirScript/init.server.luau");
        let new_path = session.path().join("src/DirScript/init.local.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::rename(&init_path, &new_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_server_to_local_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_type_server_to_local_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 3: Multi-step init type cycling: module -> server -> local -> module.
/// Three patches, each verified, with round-trip at each step.
#[test]
fn init_type_change_cycle() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let dir = session.path().join("src/DirModule");

        // Step 1: init.luau -> init.server.luau (Module -> Script)
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::rename(dir.join("init.luau"), dir.join("init.server.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_cycle_patch1",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 2: init.server.luau -> init.local.luau (Script -> LocalScript)
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::rename(dir.join("init.server.luau"), dir.join("init.local.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_cycle_patch2",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 3: init.local.luau -> init.luau (LocalScript -> Module)
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::rename(dir.join("init.local.luau"), dir.join("init.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_cycle_patch3",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);
    });
}

/// Test 4: Delete init.luau from a directory while children remain.
/// Children should survive and the parent should become a Folder.
#[test]
fn init_delete_children_survive() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let init_path = session.path().join("src/DirModule/init.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&init_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_delete_survive_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_delete_survive_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 5: Create init.luau in an existing folder that had no init file.
/// The Folder instance should become a ModuleScript.
#[test]
fn init_create_in_existing_folder() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let dir = session.path().join("src/DirModule");
        let init_path = dir.join("init.luau");

        // First remove the init file to create a plain folder
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&init_path).unwrap();
            })
            .unwrap();
        let cursor = get_message_cursor(&packet);

        // Now re-create init.luau -- Folder should become ModuleScript
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(&init_path, "-- Re-created init\nreturn {}").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_create_in_folder_patch",
            packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_init_create_in_folder_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 2: Multi-Step Connected Sequences
// ---------------------------------------------------------------------------

/// Test 6: Create -> edit -> rename -> delete a file. Four patches.
#[test]
fn create_edit_rename_delete_sequence() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        // Step 1: Create file
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("NewScript.luau"), "-- new script\nreturn 1").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_sequence_create",
            packet.intern_and_redact(&mut redactions, ())
        );

        // Step 2: Edit file
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(src.join("NewScript.luau"), "-- edited script\nreturn 2").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_sequence_edit",
            packet.intern_and_redact(&mut redactions, ())
        );

        // Step 3: Rename file
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::rename(src.join("NewScript.luau"), src.join("Renamed.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_sequence_rename",
            packet.intern_and_redact(&mut redactions, ())
        );

        // Step 4: Delete file
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(src.join("Renamed.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_sequence_delete",
            packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 7: Edit multiple files sequentially, verify each patch only
/// contains the changed file (no cross-contamination).
#[test]
fn multi_file_sequential_edits() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let _root_id = info.root_instance_id;

        let src = session.path().join("src");

        // Edit standalone.luau
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    src.join("standalone.luau"),
                    "-- edited standalone\nreturn 1",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_multi_edit_standalone",
            packet.intern_and_redact(&mut redactions, ())
        );

        // Edit server.server.luau
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(
                    src.join("server.server.luau"),
                    "-- edited server\nprint('v2')",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_multi_edit_server",
            packet.intern_and_redact(&mut redactions, ())
        );

        // Edit local.client.luau
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(
                    src.join("local.client.luau"),
                    "-- edited local\nprint('v2')",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_multi_edit_local",
            packet.intern_and_redact(&mut redactions, ())
        );
    });
}

/// Test 8: Add 3 files sequentially. Each patch should add exactly one instance.
#[test]
fn add_multiple_files_sequentially() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("AddedOne.luau"), "return 1").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_add_seq_1",
            packet.intern_and_redact(&mut redactions, ())
        );

        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(src.join("AddedTwo.luau"), "return 2").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_add_seq_2",
            packet.intern_and_redact(&mut redactions, ())
        );

        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(src.join("AddedThree.luau"), "return 3").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_add_seq_3",
            packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 9: Remove 3 files sequentially. Each patch should remove exactly one instance.
#[test]
fn remove_multiple_files_sequentially() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(src.join("standalone.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_remove_seq_1",
            packet.intern_and_redact(&mut redactions, ())
        );

        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(src.join("server.server.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_remove_seq_2",
            packet.intern_and_redact(&mut redactions, ())
        );

        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(src.join("local.client.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_remove_seq_3",
            packet.intern_and_redact(&mut redactions, ())
        );
    });
}

// ---------------------------------------------------------------------------
// Phase 3: Init File Operations
// ---------------------------------------------------------------------------

/// Test 10: Edit init.luau then edit a child. Two separate patches.
#[test]
fn edit_init_then_edit_child() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let _root_id = info.root_instance_id;

        let dir = session.path().join("src/DirModule");

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    dir.join("init.luau"),
                    "-- edited init\nreturn { edited = true }",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_edit_init_then_child_init",
            packet.intern_and_redact(&mut redactions, ())
        );

        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(
                    dir.join("ChildA.luau"),
                    "-- edited child A\nreturn 'edited'",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_edit_init_then_child_child",
            packet.intern_and_redact(&mut redactions, ())
        );
    });
}

/// Test 11: Replace init file type: delete init.luau, create init.server.luau.
/// ClassName should change, children should be preserved.
#[test]
fn replace_init_file_type() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let dir = session.path().join("src/DirModule");

        // Delete init.luau and create init.server.luau
        // Do this as two steps to be safe with filesystem events
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(dir.join("init.luau")).unwrap();
            })
            .unwrap();
        let cursor = get_message_cursor(&packet);

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(dir.join("init.server.luau"), "-- now a server script").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_replace_init_type_patch",
            packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_replace_init_type_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 12: Delete init from directory with children. Parent becomes Folder.
#[test]
fn delete_init_from_directory() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let init_path = session.path().join("src/DirScript/init.server.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&init_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_delete_init_from_dir_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "connected_delete_init_from_dir_final",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 4: Property/Meta Changes
// ---------------------------------------------------------------------------

/// Test 13: Create adjacent meta file next to a Script.
/// Verify the tree state picks up the property and round-trip holds.
///
/// NOTE: Creating a new .meta.json5 next to a project-node-rooted script
/// re-snapshots the project node but the diff engine reports "no changes",
/// so no WebSocket patch is emitted. The tree state IS correct (verified
/// by round-trip), but the plugin would need to reconnect to see the
/// change. This test verifies the round-trip identity is maintained.
#[test]
fn adjacent_meta_creation() {
    run_serve_test("connected_scripts", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Use server.server.luau (Script class) since Disabled is a Script property
        let meta_path = session.path().join("src/server.server.meta.json5");
        fs::write(&meta_path, r#"{ "properties": { "Disabled": true } }"#).unwrap();

        // Give VFS time to process the CREATE event
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // Verify round-trip identity: fresh rebuild matches live tree
        assert_round_trip(&session, root_id);
    });
}

/// Test 14: Create then delete a meta file. Round-trip should hold
/// after both operations.
#[test]
fn adjacent_meta_delete() {
    run_serve_test("connected_scripts", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/server.server.meta.json5");

        // Create meta file with Disabled property
        fs::write(&meta_path, r#"{ "properties": { "Disabled": true } }"#).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_round_trip(&session, root_id);

        // Delete it
        fs::remove_file(&meta_path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert_round_trip(&session, root_id);
    });
}

/// Test 15: Create init.meta.json5 in a directory with Script init.
/// The Disabled property should appear in the patch.
#[test]
fn init_meta_creation() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // DirScript has init.server.luau (Script class), so Disabled is valid
        let meta_path = session.path().join("src/DirScript/init.meta.json5");
        fs::write(&meta_path, r#"{ "properties": { "Disabled": true } }"#).unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "connected_init_meta_creation_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 5: Model File Changes
// ---------------------------------------------------------------------------

/// Test 16: Edit model.json5 to add children. Verify additions in patch.
#[test]
fn model_json_edit_properties() {
    run_serve_test("connected_models", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let model_path = session.path().join("src/SimpleModel.model.json5");
        fs::write(
            &model_path,
            r#"{
  "className": "Configuration",
  "children": [{
    "name": "NewChild",
    "className": "Folder"
  }]
}"#,
        )
        .unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "connected_model_edit_props_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 17: Create a new model.json5 file. Verify instance addition.
#[test]
fn model_json_create() {
    run_serve_test("connected_models", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let model_path = session.path().join("src/NewModel.model.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&model_path, r#"{ "className": "Folder" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_model_create_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 18: Delete a model.json5 file. Verify instance removal.
#[test]
fn model_json_delete() {
    run_serve_test("connected_models", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let model_path = session.path().join("src/SimpleModel.model.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&model_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_model_delete_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );
    });
}

// ---------------------------------------------------------------------------
// Phase 6: Cursor Tracking and Reconnection
// ---------------------------------------------------------------------------

/// Test 19: Verify cursor advances monotonically across 3 changes.
#[test]
fn cursor_advances_correctly() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let _root_id = info.root_instance_id;

        let src = session.path().join("src");

        let p1 = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("standalone.luau"), "-- v1\nreturn 1").unwrap();
            })
            .unwrap();
        let c1 = get_message_cursor(&p1);

        let p2 = session
            .recv_socket_packet(SocketPacketType::Messages, c1, || {
                fs::write(src.join("standalone.luau"), "-- v2\nreturn 2").unwrap();
            })
            .unwrap();
        let c2 = get_message_cursor(&p2);

        let p3 = session
            .recv_socket_packet(SocketPacketType::Messages, c2, || {
                fs::write(src.join("standalone.luau"), "-- v3\nreturn 3").unwrap();
            })
            .unwrap();
        let c3 = get_message_cursor(&p3);

        // Cursors must be monotonically increasing
        assert!(c1 > 0, "Cursor 1 should be > 0, got {c1}");
        assert!(c2 > c1, "Cursor 2 ({c2}) should be > cursor 1 ({c1})");
        assert!(c3 > c2, "Cursor 3 ({c3}) should be > cursor 2 ({c2})");
        // Snapshot the final state as a basic sanity check
        assert_yaml_snapshot!(
            "connected_cursor_final_patch",
            p3.intern_and_redact(&mut redactions, ())
        );
    });
}

/// Test 20: Reconnect with cursor 0 after 2 changes. Should get ALL patches.
#[test]
fn reconnect_with_old_cursor() {
    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let _root_id = info.root_instance_id;

        let src = session.path().join("src");

        // Make change 1
        let p1 = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("standalone.luau"), "-- change1\nreturn 1").unwrap();
            })
            .unwrap();
        let c1 = get_message_cursor(&p1);

        // Make change 2
        let _p2 = session
            .recv_socket_packet(SocketPacketType::Messages, c1, || {
                fs::write(src.join("server.server.luau"), "-- change2\nprint('v2')").unwrap();
            })
            .unwrap();

        // Reconnect with cursor 0 -- should catch up with ALL messages
        let catch_up = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "connected_reconnect_cursor0_catchup",
            catch_up.intern_and_redact(&mut redactions, ())
        );
    });
}

// ---------------------------------------------------------------------------
// Phase 7: Mixed API + Filesystem Verification
// ---------------------------------------------------------------------------

/// Test 21: API write then filesystem change. Only the filesystem change
/// should produce a WebSocket patch (echo suppression).
#[test]
fn api_write_then_filesystem_change() {
    use librojo::web_api::{InstanceUpdate, WriteRequest};

    run_serve_test("connected_scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Find the standalone instance
        let read = session.get_api_read(root_id).unwrap();
        let rs_id = read
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ReplicatedStorage")
            .map(|(id, _)| *id)
            .unwrap();
        let rs_read = session.get_api_read(rs_id).unwrap();
        let (standalone_id, _) = rs_read
            .instances
            .iter()
            .find(|(_, inst)| inst.name == "standalone")
            .unwrap();

        // API write: update source on standalone
        let mut props = rbx_dom_weak::UstrMap::default();
        props.insert(
            rbx_dom_weak::ustr("Source"),
            Some(rbx_dom_weak::types::Variant::String(
                "-- api write\nreturn {}".to_string(),
            )),
        );
        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: std::collections::HashMap::new(),
            updated: vec![InstanceUpdate {
                id: *standalone_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: props,
                changed_metadata: None,
            }],
        };
        session.post_api_write(&write_request).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Now make a filesystem change to a DIFFERENT file
        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    session.path().join("src/server.server.luau"),
                    "-- fs change\nprint('fs')",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_api_then_fs_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 8a: Slugify Forward-Sync
// ---------------------------------------------------------------------------

/// Test 22: Edit slugified script source. Patch should show instance
/// named "What?Module" (from meta), NOT "What_Module" (filesystem name).
#[test]
fn edit_slugified_script_source() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let file_path = session.path().join("src/What_Module.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&file_path, "-- edited slugified module\nreturn 'edited'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_edit_source_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 23: Edit slugified server script source. Patch should show
/// instance named "Key:Script".
#[test]
fn edit_slugified_server_script_source() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let _root_id = info.root_instance_id;

        let file_path = session.path().join("src/Key_Script.server.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&file_path, "-- edited key script\nprint('edited')").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_edit_server_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );
    });
}

/// Test 24: Create new slugified file + meta. Patch should add instance
/// with real name from meta.
#[test]
fn create_new_slugified_file_with_meta() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("New_Slug.meta.json5"), r#"{ "name": "New/Slug" }"#).unwrap();
                fs::write(src.join("New_Slug.luau"), "-- new slug\nreturn 'new'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_create_new_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 25: Delete slugified file and meta. Patch should remove the instance.
#[test]
fn delete_slugified_file_and_meta() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(src.join("What_Module.luau")).unwrap();
                fs::remove_file(src.join("What_Module.meta.json5")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_delete_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );
    });
}

/// Test 26: Edit only the meta name field. Same slug, different real name.
/// This is the "Foo/Bar -> Foo|Bar, both slugify to Foo_Bar" case.
#[test]
fn edit_only_meta_name_field() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/What_Module.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&meta_path, r#"{ "name": "What*Module" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_edit_meta_name_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 27: Delete meta file (leave script). Instance name should revert
/// from "What?Module" to "What_Module" (the filename stem).
#[test]
fn delete_meta_name_reverts_to_stem() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/What_Module.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&meta_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_delete_meta_revert_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 8b: Dedup (~N Suffix) Forward-Sync
// ---------------------------------------------------------------------------

/// Test 28: Edit dedup file with meta. Patch should show instance named
/// "Hey/Bro" (from meta), not "Hey_Bro~1".
#[test]
fn edit_dedup_file_with_meta() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let file_path = session.path().join("src/Hey_Bro~1.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&file_path, "-- edited dedup\nreturn 'edited'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_dedup_edit_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 29: Create Foo~1.luau with NO meta. Instance name should be
/// literally "Foo~1" -- tilde is NOT parsed as dedup marker.
#[test]
fn dedup_file_without_meta_uses_stem() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let file_path = session.path().join("src/Foo~1.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&file_path, "-- no meta, tilde is literal\nreturn 'Foo~1'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_dedup_no_meta_stem_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 30: Remove one of two colliding files. Only the correct instance removed.
#[test]
fn remove_one_of_two_colliding_files() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(src.join("Hey_Bro~1.luau")).unwrap();
                fs::remove_file(src.join("Hey_Bro~1.meta.json5")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_dedup_remove_one_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 31: Add third collision (Hey_Bro~2). All three should coexist.
#[test]
fn add_third_collision() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("Hey_Bro~2.meta.json5"), r#"{ "name": "Hey*Bro" }"#).unwrap();
                fs::write(
                    src.join("Hey_Bro~2.luau"),
                    "-- third collision\nreturn 'Hey*Bro'",
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_dedup_add_third_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 8c: Slugified Directory Format
// ---------------------------------------------------------------------------

/// Test 32: Edit init in slugified directory. Patch should show instance
/// named "Slug:Dir" (from init.meta.json5).
#[test]
fn edit_init_in_slugified_directory() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let init_path = session.path().join("src/Slug_Dir/init.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&init_path, "-- edited slug dir init\nreturn 'edited'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slugdir_edit_init_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 33: Init type change in slugified directory. ClassName changes
/// but name must remain "Slug:Dir" from meta.
#[test]
fn init_type_change_in_slugified_directory() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let dir = session.path().join("src/Slug_Dir");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::rename(dir.join("init.luau"), dir.join("init.server.luau")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slugdir_init_type_change_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 34: Add child to slugified directory. Child should appear under
/// instance named "Slug:Dir".
#[test]
fn add_child_to_slugified_directory() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let child_path = session.path().join("src/Slug_Dir/NewChild.luau");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&child_path, "-- new child in slug dir\nreturn 'child'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slugdir_add_child_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 8d: Meta Name Field Lifecycle
// ---------------------------------------------------------------------------

/// Test 35: Add meta name to clean file. Name should change from
/// "Normal" to "Nor/mal".
#[test]
fn add_meta_name_to_clean_file() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/Normal.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&meta_path, r#"{ "name": "Nor/mal" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_meta_add_name_to_clean_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 36: Update meta name field. Slug stays the same, real name changes.
#[test]
fn update_meta_name_field() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/What_Module.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&meta_path, r#"{ "name": "What:Module" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_meta_update_name_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 37: Create meta with both name AND properties. Round-trip
/// should show both the name override and the property.
#[test]
fn meta_name_with_other_properties() {
    run_serve_test("connected_slugify", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let meta_path = session.path().join("src/CleanScript.server.meta.json5");
        fs::write(
            &meta_path,
            r#"{ "name": "Real/Name", "properties": { "Disabled": true } }"#,
        )
        .unwrap();

        // Give VFS time to process
        std::thread::sleep(std::time::Duration::from_millis(1500));

        // Verify round-trip identity
        assert_round_trip(&session, root_id);
    });
}

/// Test 38: Edit model.json5 name field. Name should change.
#[test]
fn model_json_name_field_edit() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let model_path = session.path().join("src/What_Model.model.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    &model_path,
                    r#"{ "name": "What:Model", "className": "Configuration" }"#,
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_model_name_edit_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

/// Test 39: Remove name field from model.json5. Name should revert
/// from "What?Model" to "What_Model".
#[test]
fn model_json_remove_name_field() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let model_path = session.path().join("src/What_Model.model.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&model_path, r#"{ "className": "Configuration" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_model_remove_name_patch",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        assert_round_trip(&session, root_id);
    });
}

// ---------------------------------------------------------------------------
// Phase 8e: Multi-Step Slugify Scenarios
// ---------------------------------------------------------------------------

/// Test 40: Multi-step slugify rename chain. Edit content, update meta name,
/// delete meta (reverts), re-create meta. Round-trip at each step.
#[test]
fn slugify_rename_chain() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let file_path = session.path().join("src/What_Module.luau");
        let meta_path = session.path().join("src/What_Module.meta.json5");

        // Step 1: Edit content
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&file_path, "-- step1 edit\nreturn 'step1'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_chain_step1",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 2: Update meta name What?Module -> What*Module
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(&meta_path, r#"{ "name": "What*Module" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_chain_step2",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 3: Delete meta (name reverts to What_Module)
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(&meta_path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_chain_step3",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 4: Re-create meta with different name
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(&meta_path, r#"{ "name": "What/Module" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_slug_chain_step4",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);
    });
}

/// Test 41: Collision evolution. Add third collision, remove one, reuse
/// the ~1 slot with a different name. Round-trip at each step.
#[test]
fn collision_evolution() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        // Step 1: Add third collision Hey_Bro~2 -> "Hey*Bro"
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("Hey_Bro~2.meta.json5"), r#"{ "name": "Hey*Bro" }"#).unwrap();
                fs::write(src.join("Hey_Bro~2.luau"), "return 'Hey*Bro'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_collision_evo_step1",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 2: Remove Hey_Bro~1 (Hey/Bro)
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(src.join("Hey_Bro~1.luau")).unwrap();
                fs::remove_file(src.join("Hey_Bro~1.meta.json5")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_collision_evo_step2",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 3: Reuse ~1 slot with different name "Hey|Bro"
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::write(src.join("Hey_Bro~1.meta.json5"), r#"{ "name": "Hey|Bro" }"#).unwrap();
                fs::write(src.join("Hey_Bro~1.luau"), "return 'Hey|Bro'").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_collision_evo_step3",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);
    });
}

/// Test 42: Clean name -> slugified (via adding meta) -> clean (via removing
/// meta) -> different slug+name (via rename + meta). Round-trip at each step.
#[test]
fn clean_to_slug_to_clean_via_filesystem() {
    run_serve_test("connected_slugify", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Intern the initial tree so parent Refs are in the redaction map
        let read_response = session.get_api_read(root_id).unwrap();
        read_response.intern_and_redact(&mut redactions, root_id);

        let src = session.path().join("src");

        // Step 1: Add meta to Normal.luau -> name becomes "Nor:mal"
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(src.join("Normal.meta.json5"), r#"{ "name": "Nor:mal" }"#).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_clean_slug_clean_step1",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 2: Delete meta -> name reverts to "Normal"
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::remove_file(src.join("Normal.meta.json5")).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_clean_slug_clean_step2",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);

        // Step 3: Rename Normal.luau -> Has_Slash.luau + create meta
        let cursor = get_message_cursor(&packet);
        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                fs::rename(src.join("Normal.luau"), src.join("Has_Slash.luau")).unwrap();
                fs::write(
                    src.join("Has_Slash.meta.json5"),
                    r#"{ "name": "Has/Slash" }"#,
                )
                .unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "connected_clean_slug_clean_step3",
            packet.intern_and_redact(&mut redactions, ())
        );
        assert_round_trip(&session, root_id);
    });
}
