use std::fs;

use insta::{assert_snapshot, assert_yaml_snapshot, with_settings};
use tempfile::tempdir;

use crate::rojo_test::{
    internable::InternAndRedact,
    serve_util::{run_serve_test, serialize_to_xml_model, TestServeSession},
};

use librojo::web_api::SocketPacketType;

#[test]
fn empty() {
    run_serve_test("empty", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("empty_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "empty_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn scripts() {
    run_serve_test("scripts", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("scripts_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        with_settings!({ sort_maps => true }, {
            assert_yaml_snapshot!(
                "scripts_all",
                read_response.intern_and_redact(&mut redactions, root_id)
            );
        });

        let path = session.path().join("src/foo.luau");
        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&path, "Updated foo!").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "scripts_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        with_settings!({ sort_maps => true }, {
            assert_yaml_snapshot!(
                "scripts_all-2",
                read_response.intern_and_redact(&mut redactions, root_id)
            );
        });
    });
}

#[test]
fn add_folder() {
    run_serve_test("add_folder", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("add_folder_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "add_folder_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        fs::create_dir(session.path().join("src/my-new-folder")).unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "add_folder_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "add_folder_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn remove_file() {
    run_serve_test("remove_file", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("remove_file_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "remove_file_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let path = session.path().join("src/hello.txt");
        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "remove_file_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "remove_file_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn edit_init() {
    run_serve_test("edit_init", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("edit_init_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "edit_init_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let path = session.path().join("src/init.luau");
        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(&path, b"-- Edited contents").unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "edit_init_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "edit_init_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn move_folder_of_stuff() {
    run_serve_test("move_folder_of_stuff", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("move_folder_of_stuff_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "move_folder_of_stuff_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        // Create a directory full of stuff we can move in
        let src_dir = tempdir().unwrap();
        let stuff_path = src_dir.path().join("new-stuff");

        fs::create_dir(&stuff_path).unwrap();

        // Make a bunch of random files in our stuff folder
        for i in 0..10 {
            let file_name = stuff_path.join(format!("{}.txt", i));
            let file_contents = format!("File #{}", i);

            fs::write(file_name, file_contents).unwrap();
        }

        // We're hoping that this rename gets picked up as one event. This test
        // will fail otherwise.
        fs::rename(stuff_path, session.path().join("src/new-stuff")).unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "move_folder_of_stuff_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "move_folder_of_stuff_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn empty_json_model() {
    run_serve_test("empty_json_model", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("empty_json_model_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "empty_json_model_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        fs::write(
            session.path().join("src/test.model.json5"),
            r#"{"ClassName": "Model"}"#,
        )
        .unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "empty_json_model_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "empty_json_model_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
#[ignore = "Rojo does not watch missing, optional files for changes."]
fn add_optional_folder() {
    run_serve_test("add_optional_folder", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("add_optional_folder", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "add_optional_folder_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        fs::create_dir(session.path().join("create-later")).unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "add_optional_folder_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "add_optional_folder_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn sync_rule_alone() {
    run_serve_test("sync_rule_alone", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("sync_rule_alone_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "sync_rule_alone_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn sync_rule_complex() {
    run_serve_test("sync_rule_complex", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("sync_rule_complex_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "sync_rule_complex_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn sync_rule_no_extension() {
    run_serve_test("sync_rule_no_extension", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!(
            "sync_rule_no_extension_info",
            redactions.redacted_yaml(info)
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "sync_rule_no_extension_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn no_name_default_project() {
    run_serve_test("no_name_default_project", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!(
            "no_name_default_project_info",
            redactions.redacted_yaml(info)
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "no_name_default_project_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn no_name_project() {
    run_serve_test("no_name_project", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("no_name_project_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "no_name_project_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn no_name_top_level_project() {
    run_serve_test("no_name_top_level_project", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!(
            "no_name_top_level_project_info",
            redactions.redacted_yaml(info)
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "no_name_top_level_project_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let project_path = session.path().join("default.project.json5");
        let mut project_contents = fs::read_to_string(&project_path).unwrap();
        project_contents.push('\n');
        fs::write(&project_path, project_contents).unwrap();

        // The cursor shouldn't be changing so this snapshot is fine for testing
        // the response.
        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "no_name_top_level_project_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn sync_rule_no_name_project() {
    run_serve_test("sync_rule_no_name_project", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!(
            "sync_rule_no_name_project_info",
            redactions.redacted_yaml(info)
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "sync_rule_no_name_project_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

// On macOS, kqueue delivers a duplicate WRITE event for the same file
// modification. The second event re-snapshots against a tree whose Rojo
// transport attributes (Rojo_Id, Rojo_Target_*) were cleaned up by
// finalize_patch_application during the first event, producing spurious
// Attributes and metadata update messages that break the snapshot.
#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn ref_properties() {
    run_serve_test("ref_properties", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("ref_properties_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        fs::write(
            session.path().join("ModelTarget.model.json5"),
            r#"{
                "className": "Folder",
                "attributes": {
                    "Rojo_Id": "model target 2"
                },
                "children": [
                  {
                    "name": "ModelPointer",
                    "className": "Model",
                    "attributes": {
                      "Rojo_Target_PrimaryPart": "model target 2"
                    }
                  },
                  {
                    "name": "ProjectPointer",
                    "className": "Model",
                    "attributes": {
                      "Rojo_Target_PrimaryPart": "project target"
                    }
                  }
                ]
              }"#,
        )
        .unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "ref_properties_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

// On macOS, kqueue delivers an extra directory WRITE event after file
// removal that triggers a re-snapshot, clearing the stale Ref property
// and bumping messageCursor. This changes the expected snapshot output.
// The behavior is correct (kqueue is more thorough), but the snapshot
// was captured on Windows/Linux where only a single REMOVE event fires.
#[test]
#[cfg_attr(target_os = "macos", ignore)]
fn ref_properties_remove() {
    run_serve_test("ref_properties_remove", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("ref_properties_remove_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_remove_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let path = session.path().join("src/target.model.json5");
        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::remove_file(&path).unwrap();
            })
            .unwrap();
        assert_yaml_snapshot!(
            "ref_properties_remove_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_remove_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

/// When Ref properties were first implemented, a mistake was made that resulted
/// in Ref properties defined via attributes not being included in patch
/// computation, which resulted in subsequent patches setting those properties
/// to `nil`.
///
/// See: https://github.com/rojo-rbx/rojo/issues/929
#[test]
fn ref_properties_patch_update() {
    // Reusing ref_properties is fun and easy.
    run_serve_test("ref_properties", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!(
            "ref_properties_patch_update_info",
            redactions.redacted_yaml(info)
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_patch_update_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let target_path = session.path().join("ModelTarget.model.json5");

        // Inserting scale just to force the change processor to run
        fs::write(
            target_path,
            r#"{
            "id": "model target",
            "className": "Folder",
            "children": [
                {
                    "name": "ModelPointer",
                    "className": "Model",
                    "attributes": {
                        "Rojo_Target_PrimaryPart": "model target"
                    },
                    "properties": {
                        "Scale": 1
                    }
                }
            ]
        }"#,
        )
        .unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "ref_properties_patch_update_subscribe",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "ref_properties_patch_update_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn model_pivot_migration() {
    run_serve_test("pivot_migration", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("pivot_migration_info", redactions.redacted_yaml(info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "pivot_migration_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let project_path = session.path().join("default.project.json5");

        fs::write(
            project_path,
            r#"{
            "name": "pivot_migration",
            "tree": {
                "$className": "DataModel",
                "Workspace": {
                    "Model": {
                        "$className": "Model"
                    },
                    "Tool": {
                        "$path": "Tool.model.json5"
                    },
                    "Actor": {
                        "$className": "Actor"
                    }
                }
            }
        }"#,
        )
        .unwrap();

        let socket_packet = session
            .get_api_socket_packet(SocketPacketType::Messages, 0)
            .unwrap();
        assert_yaml_snapshot!(
            "model_pivot_migration_all",
            socket_packet.intern_and_redact(&mut redactions, ())
        );

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "model_pivot_migration_all-2",
            read_response.intern_and_redact(&mut redactions, root_id)
        );
    });
}

#[test]
fn meshpart_with_id() {
    run_serve_test("meshpart_with_id", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("meshpart_with_id_info", redactions.redacted_yaml(&info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "meshpart_with_id_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        // This is a bit awkward, but it's fine.
        let (meshpart, _) = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "MeshPart")
            .unwrap();
        let (objectvalue, _) = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ObjectValue")
            .unwrap();

        let serialize_response = session
            .get_api_serialize(&[*meshpart, *objectvalue])
            .unwrap();

        // We don't assert a snapshot on the SerializeResponse because the model includes the
        // Refs from the DOM as names, which means it will obviously be different every time
        // this code runs. Still, we ensure that the SessionId is right at least.
        assert_eq!(serialize_response.session_id, info.session_id);

        let model = serialize_to_xml_model(&serialize_response, &redactions);
        assert_snapshot!("meshpart_with_id_serialize_model", model);
    });
}

#[test]
fn forced_parent() {
    run_serve_test("forced_parent", |session, mut redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        assert_yaml_snapshot!("forced_parent_info", redactions.redacted_yaml(&info));

        let read_response = session.get_api_read(root_id).unwrap();
        assert_yaml_snapshot!(
            "forced_parent_all",
            read_response.intern_and_redact(&mut redactions, root_id)
        );

        let serialize_response = session.get_api_serialize(&[root_id]).unwrap();

        assert_eq!(serialize_response.session_id, info.session_id);

        let model = serialize_to_xml_model(&serialize_response, &redactions);
        assert_snapshot!("forced_parent_serialize_model", model);
    });
}

/// Test that plugin syncback via /api/write doesn't create duplicate files
/// when sending an AddedInstance that already exists on the filesystem.
///
/// This tests the fix for the issue where pulling an instance that appeared
/// as "to delete" (due to duplicate detection issues) would create additional
/// duplicate files instead of updating the existing ones.
///
/// The fix has two layers:
/// 1. Tree lookup: Check if instance exists in Rojo's tree, update in place
/// 2. Filesystem check: Detect existing file format and preserve it
#[test]
fn api_write_existing_instance() {
    use librojo::web_api::{AddedInstance, WriteRequest};
    use rbx_dom_weak::types::Variant;
    use std::collections::HashMap;

    run_serve_test("api_write_existing", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        // Read the tree to find ServerScriptService
        let read_response = session.get_api_read(root_id).unwrap();

        // Find the ServerScriptService instance
        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        // Verify initial state on filesystem
        let event_service_dir = session.path().join("src").join("EventService");
        let init_file = event_service_dir.join("init.server.luau");
        assert!(
            event_service_dir.exists(),
            "EventService directory should exist before test"
        );
        assert!(
            init_file.exists(),
            "init.server.luau should exist before test"
        );

        // This is the critical file we're checking - it should NOT be created
        let standalone_file = session.path().join("src").join("EventService.server.luau");
        assert!(
            !standalone_file.exists(),
            "EventService.server.luau should NOT exist initially"
        );

        // Simulate plugin sending an AddedInstance for EventService
        // This happens when user "pulls" an instance that appeared as "to delete"
        let mut properties = HashMap::new();
        properties.insert(
            "Source".to_string(),
            Variant::String("-- Updated EventService\nreturn {}".to_string()),
        );

        let added_instance = AddedInstance {
            parent: Some(sss_id),
            name: "EventService".to_string(),
            class_name: "Script".to_string(),
            properties,
            children: vec![AddedInstance {
                parent: None,
                name: "AcidRainEvent".to_string(),
                class_name: "ModuleScript".to_string(),
                properties: {
                    let mut p = HashMap::new();
                    p.insert(
                        "Source".to_string(),
                        Variant::String("-- Updated AcidRainEvent\nreturn {}".to_string()),
                    );
                    p
                },
                children: vec![],
            }],
        };

        let instance_ref = rbx_dom_weak::types::Ref::new();
        let mut added_map = HashMap::new();
        added_map.insert(instance_ref, added_instance);

        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: added_map,
            updated: vec![],
            stage_ids: Vec::new(),
        };

        session
            .post_api_write(&write_request)
            .expect("Write request should succeed");

        // Give the server time to process
        std::thread::sleep(std::time::Duration::from_millis(200));

        // CRITICAL ASSERTION: No duplicate standalone file should be created
        // The existing directory structure (EventService/init.server.luau) should be preserved
        assert!(
            !standalone_file.exists(),
            "EventService.server.luau should NOT be created - the existing directory structure must be preserved"
        );

        // The directory should still exist
        assert!(
            event_service_dir.exists(),
            "EventService directory should still exist after syncback"
        );
    });
}

// ===========================================================================
// Rojo_Ref_* path-based forward-sync tests
//
// These tests validate that Rojo_Ref_* attributes in meta/model files
// correctly resolve to Variant::Ref properties during forward sync
// (filesystem → server → plugin).
// ===========================================================================

/// Initial load: Model with Rojo_Ref_PrimaryPart attribute should have
/// PrimaryPart resolved as a Variant::Ref in the read response.
#[test]
fn ref_path_initial_load() {
    run_serve_test("ref_forward_sync", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();

        let read_response = session.get_api_read(info.root_instance_id).unwrap();
        let instances = &read_response.instances;

        // Find MyModel and check it has a PrimaryPart Ref property
        let model = instances
            .values()
            .find(|inst| inst.name == "MyModel")
            .expect("MyModel should exist in read response");

        let has_primary_part = model
            .properties
            .iter()
            .any(|(name, _)| name == "PrimaryPart");
        assert!(
            has_primary_part,
            "MyModel should have PrimaryPart property resolved from Rojo_Ref_PrimaryPart"
        );
    });
}

/// Add a Rojo_Ref_PrimaryPart attribute to an existing meta file.
/// The forward-sync patch should include a PrimaryPart Ref update.
#[test]
fn ref_path_add_attribute_to_existing_meta() {
    run_serve_test("ref_forward_sync", |session, mut redactions| {
        let _info = session.get_api_rojo().unwrap();

        // Modify MyModel's meta to change PrimaryPart to OtherPart
        let meta_path = session.path().join("src/Workspace/MyModel/init.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    &meta_path,
                    r#"{
                        "className": "Model",
                        "attributes": {
                            "Rojo_Ref_PrimaryPart": "@self/OtherPart"
                        }
                    }"#,
                )
                .unwrap();
            })
            .unwrap();

        // Verify we got a patch -- the PrimaryPart should be updated
        let redacted = socket_packet.intern_and_redact(&mut redactions, ());
        assert_yaml_snapshot!("ref_path_change_target_patch", redacted);
    });
}

/// Remove a Rojo_Ref_PrimaryPart attribute from a meta file.
/// The forward-sync patch should update PrimaryPart to nil.
#[test]
fn ref_path_remove_attribute() {
    run_serve_test("ref_forward_sync", |session, mut redactions| {
        let _info = session.get_api_rojo().unwrap();

        let meta_path = session.path().join("src/Workspace/MyModel/init.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                // Remove the Rojo_Ref_PrimaryPart attribute
                fs::write(
                    &meta_path,
                    r#"{
                        "className": "Model",
                        "attributes": {}
                    }"#,
                )
                .unwrap();
            })
            .unwrap();

        let redacted = socket_packet.intern_and_redact(&mut redactions, ());
        assert_yaml_snapshot!("ref_path_remove_attr_patch", redacted);
    });
}

/// Add a new model file with a Rojo_Ref_Value attribute.
/// The forward-sync patch should include the new instance with Value resolved.
#[test]
fn ref_path_new_file_with_ref_attr() {
    run_serve_test("ref_forward_sync", |session, mut redactions| {
        let _info = session.get_api_rojo().unwrap();

        let new_file = session
            .path()
            .join("src/Workspace/MyModel/RefValue.model.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    &new_file,
                    r#"{
                        "className": "ObjectValue",
                        "attributes": {
                            "Rojo_Ref_Value": "./Target"
                        }
                    }"#,
                )
                .unwrap();
            })
            .unwrap();

        let redacted = socket_packet.intern_and_redact(&mut redactions, ());
        assert_yaml_snapshot!("ref_path_new_file_patch", redacted);
    });
}

/// Rojo_Ref_* pointing to a non-existent path should not crash and
/// the property should not be set.
#[test]
fn ref_path_nonexistent_target_no_crash() {
    run_serve_test("ref_forward_sync", |session, mut redactions| {
        let _info = session.get_api_rojo().unwrap();

        let meta_path = session.path().join("src/Workspace/MyModel/init.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    &meta_path,
                    r#"{
                        "className": "Model",
                        "attributes": {
                            "Rojo_Ref_PrimaryPart": "./NonExistent/Part"
                        }
                    }"#,
                )
                .unwrap();
            })
            .unwrap();

        // Should receive a patch without crashing.
        // The PrimaryPart might be nil or missing (not a resolved Ref).
        let redacted = socket_packet.intern_and_redact(&mut redactions, ());
        assert_yaml_snapshot!("ref_path_nonexistent_patch", redacted);
    });
}

/// When a non-default project file is used (e.g. `named.project.json5`
/// instead of `default.project.json5`), tree validation must snapshot
/// through the project middleware — not walk the parent directory as a
/// generic Folder. Regression test for a bug where `check_tree_freshness`
/// and `reconcile_tree` used `folder_location()` instead of the project
/// file path, causing the entire repo root to be walked (including
/// unrelated directories with potentially malformed files).
#[test]
fn non_default_project_file_tree_validation() {
    let _ = tracing_subscriber::fmt::try_init();

    let mut session =
        TestServeSession::new_with_project_file("non_default_project", "named.project.json5");
    let _info = session.wait_to_come_online();

    session.assert_tree_fresh();
}

/// Multiple Rojo_Ref_* attributes on the same instance should all resolve.
#[test]
fn ref_path_multiple_attributes() {
    run_serve_test("ref_forward_sync", |session, mut redactions| {
        let _info = session.get_api_rojo().unwrap();

        let meta_path = session.path().join("src/Workspace/MyModel/init.meta.json5");

        let socket_packet = session
            .recv_socket_packet(SocketPacketType::Messages, 0, || {
                fs::write(
                    &meta_path,
                    r#"{
                        "className": "Model",
                        "attributes": {
                            "Rojo_Ref_PrimaryPart": "@self/Target",
                            "Rojo_Ref_CustomRef": "@self/OtherPart"
                        }
                    }"#,
                )
                .unwrap();
            })
            .unwrap();

        let redacted = socket_packet.intern_and_redact(&mut redactions, ());
        assert_yaml_snapshot!("ref_path_multiple_attrs_patch", redacted);
    });
}
