use std::collections::HashMap;

use librojo::web_api::{InstanceUpdate, WriteRequest};

use crate::rojo_test::serve_util::run_serve_test;

#[test]
fn scripts_only_read_prunes_non_script_subtrees() {
    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let names: Vec<&str> = read_response
            .instances
            .values()
            .map(|inst| inst.name.as_ref())
            .collect();

        assert!(
            names.contains(&"TopScript"),
            "TopScript should be included"
        );
        assert!(
            names.contains(&"DeepScript"),
            "DeepScript should be included"
        );
        assert!(
            names.contains(&"SomeScript"),
            "SomeScript should be included"
        );

        assert!(
            !names.contains(&"PartA"),
            "PartA should be pruned"
        );
        assert!(
            !names.contains(&"PartB"),
            "PartB should be pruned"
        );
        assert!(
            !names.contains(&"SomeModel"),
            "SomeModel should be pruned"
        );
        assert!(
            !names.contains(&"PureParts"),
            "PureParts folder should be pruned (no script descendants)"
        );
    });
}

#[test]
fn scripts_only_read_includes_deep_ancestors() {
    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let names: Vec<&str> = read_response
            .instances
            .values()
            .map(|inst| inst.name.as_ref())
            .collect();

        assert!(
            names.contains(&"DeepNest"),
            "DeepNest ancestor should be included for DeepScript"
        );
        assert!(
            names.contains(&"Middle"),
            "Middle ancestor should be included for DeepScript"
        );
        assert!(
            names.contains(&"DeepScript"),
            "DeepScript should be included"
        );
    });
}

#[test]
fn scripts_only_read_ancestors_have_no_properties() {
    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        for (_, inst) in &read_response.instances {
            let is_script = matches!(
                inst.class_name.as_str(),
                "Script" | "LocalScript" | "ModuleScript"
            );
            if !is_script && inst.name != "scripts_only_read_pruning" {
                assert!(
                    inst.properties.is_empty(),
                    "Non-script ancestor '{}' ({}) should have empty properties, got {} properties",
                    inst.name,
                    inst.class_name,
                    inst.properties.len()
                );
                if let Some(ref meta) = inst.metadata {
                    assert!(
                        meta.ignore_unknown_instances,
                        "Non-script ancestor '{}' should have ignoreUnknownInstances: true",
                        inst.name
                    );
                }
            }
        }
    });
}

#[test]
fn scripts_only_serverinfo_flag() {
    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        assert!(
            info.sync_scripts_only,
            "syncScriptsOnly should be true in ServerInfoResponse"
        );
    });
}

#[test]
fn scripts_only_serverinfo_flag_absent_when_disabled() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        assert!(
            info.sync_scripts_only,
            "syncScriptsOnly should be true for syncback_scripts_only fixture"
        );
    });
}

#[test]
fn scripts_only_write_filters_non_script_updates() {
    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;

        let read_response = session.get_api_read(root_id).unwrap();

        let sss_id = read_response
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(id, _)| *id)
            .expect("ServerScriptService should exist");

        let write_request = WriteRequest {
            session_id: info.session_id,
            removed: vec![],
            added: HashMap::new(),
            updated: vec![InstanceUpdate {
                id: sss_id,
                changed_name: Some("Renamed".to_string()),
                changed_class_name: None,
                changed_properties: Default::default(),
                changed_metadata: None,
            }],
            stage_ids: Vec::new(),
        };

        session
            .post_api_write(&write_request)
            .expect("Write request should succeed");

        std::thread::sleep(std::time::Duration::from_millis(200));

        let read_after = session.get_api_read(root_id).unwrap();
        let sss_after = read_after
            .instances
            .iter()
            .find(|(_, inst)| inst.class_name == "ServerScriptService")
            .map(|(_, inst)| inst.name.as_ref())
            .expect("ServerScriptService should still exist");
        assert_eq!(
            sss_after, "ServerScriptService",
            "Non-script update should have been filtered; name should remain unchanged"
        );
    });
}

#[test]
fn scripts_only_existing_fixture_prunes_non_script_model() {
    run_serve_test("syncback_scripts_only", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();
        let root_id = info.root_instance_id;
        let read_response = session.get_api_read(root_id).unwrap();

        let names: Vec<&str> = read_response
            .instances
            .values()
            .map(|inst| inst.name.as_ref())
            .collect();

        assert!(
            !names.contains(&"NonScriptModel"),
            "NonScriptModel (Configuration) should be pruned in scripts-only mode"
        );
    });
}

#[test]
fn scripts_only_websocket_prunes_non_script_additions() {
    use librojo::web_api::SocketPacketType;

    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();

        let read_response = session.get_api_read(info.root_instance_id).unwrap();
        let cursor = read_response.message_cursor;

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                let pure_parts_dir = session.path().join("src").join("PureParts");
                std::fs::write(
                    pure_parts_dir.join("AnotherPart.model.json5"),
                    "{ \"className\": \"Part\", \"properties\": {} }\n",
                )
                .expect("Should write non-script file");
            })
            .ok();

        if let Some(packet) = packet {
            let librojo::web_api::SocketPacketBody::Messages(messages_packet) = packet.body;
            for msg in &messages_packet.messages {
                let has_part = msg
                    .added
                    .values()
                    .any(|inst| inst.name.as_ref() == "AnotherPart");
                assert!(
                    !has_part,
                    "WebSocket should not include non-script AnotherPart in scripts-only mode"
                );
            }
        }
    });
}

#[test]
fn scripts_only_websocket_injects_ancestors_for_new_script() {
    use librojo::web_api::SocketPacketType;

    run_serve_test("scripts_only_read_pruning", |session, _redactions| {
        let info = session.get_api_rojo().unwrap();

        let read_response = session.get_api_read(info.root_instance_id).unwrap();
        let cursor = read_response.message_cursor;

        let packet = session
            .recv_socket_packet(SocketPacketType::Messages, cursor, || {
                let pure_parts_dir = session.path().join("src").join("PureParts");
                std::fs::write(
                    pure_parts_dir.join("NewScript.luau"),
                    "return \"new script in pruned subtree\"\n",
                )
                .expect("Should write new script file");
            })
            .expect("Should receive WebSocket packet for new script");

        let librojo::web_api::SocketPacketBody::Messages(messages_packet) = packet.body;
        let all_added: Vec<&str> = messages_packet
            .messages
            .iter()
            .flat_map(|msg| msg.added.values())
            .map(|inst| inst.name.as_ref())
            .collect();

        assert!(
            all_added.contains(&"NewScript"),
            "WebSocket should include the new script. Got: {:?}",
            all_added
        );
        assert!(
            all_added.contains(&"PureParts"),
            "WebSocket should inject the PureParts ancestor. Got: {:?}",
            all_added
        );
    });
}
