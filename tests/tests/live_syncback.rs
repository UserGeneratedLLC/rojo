use std::{fs, thread, time::Duration};

use rbx_dom_weak::InstanceBuilder;

use crate::rojo_test::{
    roundtrip_util::assert_dirs_equal,
    serve_util::{make_service_chunk, run_cli_syncback_on_chunks, run_serve_test},
};

use librojo::web_api::{ServiceChunk, SocketPacketType, PROTOCOL_VERSION};

fn assert_live_matches_cli(
    fixture_name: &str,
    chunks: &[ServiceChunk],
    place_id: Option<u64>,
) {
    run_serve_test(fixture_name, |session, _| {
        session.post_api_syncback(place_id, chunks.to_vec());
        session.wait_to_come_back_online();

        let (_cli_dir, cli_path) = run_cli_syncback_on_chunks(fixture_name, chunks);
        assert_dirs_equal(session.path(), &cli_path);
    });
}

// ── CLI parity tests ────────────────────────────────────────────

#[test]
fn parity_scripts() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![
            InstanceBuilder::new("ModuleScript")
                .with_name("Utils")
                .with_property("Source", rbx_dom_weak::types::Variant::String("return {}".into())),
            InstanceBuilder::new("Script")
                .with_name("Main")
                .with_property("Source", rbx_dom_weak::types::Variant::String("print('hi')".into()))
                .with_property(
                    "RunContext",
                    rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(1)),
                ),
        ],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_models_with_properties() {
    let chunks = vec![make_service_chunk(
        "Workspace",
        vec![InstanceBuilder::new("Part")
            .with_name("TestPart")
            .with_property("Anchored", rbx_dom_weak::types::Variant::Bool(true))
            .with_property(
                "Size",
                rbx_dom_weak::types::Variant::Vector3(rbx_dom_weak::types::Vector3::new(
                    4.0, 2.0, 6.0,
                )),
            )],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_special_names() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![
            InstanceBuilder::new("Folder").with_name("What?"),
            InstanceBuilder::new("Folder").with_name("Key:Value"),
            InstanceBuilder::new("Folder").with_name("A/B"),
        ],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_duplicate_names() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![
            InstanceBuilder::new("Folder").with_name("Data"),
            InstanceBuilder::new("Folder").with_name("Data"),
            InstanceBuilder::new("Folder").with_name("Data"),
        ],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_deep_hierarchy() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("Folder")
            .with_name("A")
            .with_children(vec![InstanceBuilder::new("Folder")
                .with_name("B")
                .with_children(vec![InstanceBuilder::new("Folder")
                    .with_name("C")
                    .with_children(vec![InstanceBuilder::new("Folder")
                        .with_name("D")
                        .with_children(vec![InstanceBuilder::new("ModuleScript")
                            .with_name("Leaf")
                            .with_property(
                                "Source",
                                rbx_dom_weak::types::Variant::String("return true".into()),
                            )])])])])],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_mixed_file_types() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![
            InstanceBuilder::new("StringValue")
                .with_name("MyString")
                .with_property("Value", rbx_dom_weak::types::Variant::String("hello world".into())),
            InstanceBuilder::new("Folder").with_name("Container"),
            InstanceBuilder::new("ModuleScript")
                .with_name("DataModule")
                .with_property(
                    "Source",
                    rbx_dom_weak::types::Variant::String("return {a=1}".into()),
                ),
        ],
    )];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_multi_service() {
    let chunks = vec![
        make_service_chunk(
            "ReplicatedStorage",
            vec![InstanceBuilder::new("ModuleScript")
                .with_name("SharedUtil")
                .with_property(
                    "Source",
                    rbx_dom_weak::types::Variant::String("return {}".into()),
                )],
        ),
        make_service_chunk(
            "ServerScriptService",
            vec![InstanceBuilder::new("Script")
                .with_name("ServerMain")
                .with_property(
                    "Source",
                    rbx_dom_weak::types::Variant::String("print('server')".into()),
                )
                .with_property(
                    "RunContext",
                    rbx_dom_weak::types::Variant::Enum(rbx_dom_weak::types::Enum::from_u32(1)),
                )],
        ),
        make_service_chunk(
            "Workspace",
            vec![InstanceBuilder::new("Part")
                .with_name("Floor")
                .with_property("Anchored", rbx_dom_weak::types::Variant::Bool(true))],
        ),
    ];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

#[test]
fn parity_empty_services_mixed() {
    let chunks = vec![
        make_service_chunk(
            "ReplicatedStorage",
            vec![InstanceBuilder::new("Folder").with_name("Stuff")],
        ),
        make_service_chunk("ServerScriptService", vec![]),
    ];
    assert_live_matches_cli("live_syncback", &chunks, None);
}

// ── Validation / rejection tests ─────────────────────────────────

#[test]
fn rejects_bad_protocol() {
    run_serve_test("live_syncback", |session, _| {
        let initial_info = session.get_api_rojo().unwrap();

        let request = serde_json::json!({
            "protocolVersion": 9999,
            "serverVersion": env!("CARGO_PKG_VERSION"),
            "placeId": null,
            "services": [],
        });

        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        serde::Serialize::serialize(&request, &mut serializer).unwrap();

        let response = session.post_api_syncback_raw(body);
        assert!(
            !response.status().is_success(),
            "Expected rejection for bad protocol, got {}",
            response.status()
        );

        let after_info = session.get_api_rojo().unwrap();
        assert_eq!(
            initial_info.session_id, after_info.session_id,
            "Server should not have restarted"
        );
    });
}

#[test]
fn rejects_bad_version() {
    run_serve_test("live_syncback", |session, _| {
        let initial_info = session.get_api_rojo().unwrap();

        let request = serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverVersion": "0.0.0",
            "placeId": null,
            "services": [],
        });

        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        serde::Serialize::serialize(&request, &mut serializer).unwrap();

        let response = session.post_api_syncback_raw(body);
        assert!(
            !response.status().is_success(),
            "Expected rejection for bad version, got {}",
            response.status()
        );

        let after_info = session.get_api_rojo().unwrap();
        assert_eq!(
            initial_info.session_id, after_info.session_id,
            "Server should not have restarted"
        );
    });
}

#[test]
fn rejects_blocked_place() {
    run_serve_test("live_syncback_place_ids", |session, _| {
        let initial_info = session.get_api_rojo().unwrap();

        let request = serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverVersion": env!("CARGO_PKG_VERSION"),
            "placeId": 999,
            "services": [],
        });

        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        serde::Serialize::serialize(&request, &mut serializer).unwrap();

        let response = session.post_api_syncback_raw(body);
        assert_eq!(
            response.status().as_u16(),
            403,
            "Expected 403 for blocked place"
        );

        let after_info = session.get_api_rojo().unwrap();
        assert_eq!(
            initial_info.session_id, after_info.session_id,
            "Server should not have restarted"
        );
    });
}

#[test]
fn rejects_unlisted_place() {
    run_serve_test("live_syncback_place_ids", |session, _| {
        let initial_info = session.get_api_rojo().unwrap();

        let request = serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverVersion": env!("CARGO_PKG_VERSION"),
            "placeId": 456,
            "services": [],
        });

        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        serde::Serialize::serialize(&request, &mut serializer).unwrap();

        let response = session.post_api_syncback_raw(body);
        assert_eq!(
            response.status().as_u16(),
            403,
            "Expected 403 for unlisted place"
        );

        let after_info = session.get_api_rojo().unwrap();
        assert_eq!(
            initial_info.session_id, after_info.session_id,
            "Server should not have restarted"
        );
    });
}

#[test]
fn allows_whitelisted_place() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("Folder").with_name("Allowed")],
    )];
    run_serve_test("live_syncback_place_ids", |session, _| {
        session.post_api_syncback(Some(123), chunks);
        let new_info = session.wait_to_come_back_online();
        assert!(
            !new_info.project_name.is_empty(),
            "Server should be back online with a project"
        );
    });
}

#[test]
fn allows_any_place_when_unrestricted() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("Folder").with_name("Any")],
    )];
    run_serve_test("live_syncback", |session, _| {
        session.post_api_syncback(Some(99999), chunks);
        session.wait_to_come_back_online();
    });
}

// ── Lifecycle / edge case tests ──────────────────────────────────

#[test]
fn server_comes_back_functional() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("ModuleScript")
            .with_name("TestModule")
            .with_property(
                "Source",
                rbx_dom_weak::types::Variant::String("return 42".into()),
            )],
    )];

    run_serve_test("live_syncback", |session, _| {
        session.post_api_syncback(None, chunks);
        let new_info = session.wait_to_come_back_online();

        let read = session.get_api_read(new_info.root_instance_id).unwrap();
        assert!(
            !read.instances.is_empty(),
            "Should be able to read instances from restarted server"
        );

        let module_path = session.path().join("src/shared/TestModule.luau");
        let socket_packet = session.recv_socket_packet(SocketPacketType::Messages, 0, || {
            fs::write(&module_path, "return 99").unwrap();
        });
        assert!(
            socket_packet.is_ok(),
            "WebSocket should receive patches after file modification"
        );
    });
}

#[test]
fn syncback_twice_different_data() {
    let chunks_a = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("Folder").with_name("DataA")],
    )];
    let chunks_b = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("Folder").with_name("DataB")],
    )];

    run_serve_test("live_syncback", |session, _| {
        session.post_api_syncback(None, chunks_a);
        session.wait_to_come_back_online();

        assert!(
            session.path().join("src/shared/DataA").exists(),
            "First syncback should create DataA"
        );

        session.post_api_syncback(None, chunks_b);
        session.wait_to_come_back_online();

        assert!(
            !session.path().join("src/shared/DataA").exists(),
            "DataA should be gone after second syncback (clean mode)"
        );
        assert!(
            session.path().join("src/shared/DataB").exists(),
            "Second syncback should create DataB"
        );
    });
}

#[test]
fn syncback_replaces_all_old_files() {
    let chunks = vec![make_service_chunk(
        "ReplicatedStorage",
        vec![InstanceBuilder::new("ModuleScript")
            .with_name("NewFile")
            .with_property(
                "Source",
                rbx_dom_weak::types::Variant::String("return 'new'".into()),
            )],
    )];

    run_serve_test("live_syncback", |session, _| {
        assert!(
            session.path().join("src/shared/OldModule.luau").exists(),
            "OldModule should exist before syncback"
        );

        session.post_api_syncback(None, chunks);
        session.wait_to_come_back_online();

        assert!(
            !session.path().join("src/shared/OldModule.luau").exists(),
            "OldModule should be gone after clean syncback"
        );
        assert!(
            session.path().join("src/shared/NewFile.luau").exists(),
            "NewFile should exist after syncback"
        );
    });
}

// ── Round-trip identity test ─────────────────────────────────────

#[test]
fn roundtrip_build_syncback_rebuild() {
    use crate::rojo_test::roundtrip_util::run_rojo_build;
    use std::io::Cursor;

    run_serve_test("live_syncback", |session, _| {
        let (_build_dir, rbxl_path_a) = run_rojo_build(session.path(), "build_a.rbxl");
        let rbxl_data_a = fs::read(&rbxl_path_a).unwrap();

        let dom_a = rbx_binary::from_reader(Cursor::new(&rbxl_data_a)).unwrap();
        let mut chunks = Vec::new();
        for &service_ref in dom_a.root().children() {
            let service = dom_a.get_by_ref(service_ref).unwrap();
            let child_refs: Vec<rbx_dom_weak::types::Ref> = service.children().to_vec();
            if child_refs.is_empty() {
                continue;
            }
            let mut buf = Vec::new();
            rbx_binary::to_writer(&mut buf, &dom_a, &child_refs).unwrap();
            chunks.push(ServiceChunk {
                class_name: service.class.to_string(),
                data: buf,
            });
        }

        session.post_api_syncback(None, chunks);
        session.wait_to_come_back_online();

        thread::sleep(Duration::from_millis(500));

        let (_build_dir_b, rbxl_path_b) = run_rojo_build(session.path(), "build_b.rbxl");
        let rbxl_data_b = fs::read(&rbxl_path_b).unwrap();
        let dom_b = rbx_binary::from_reader(Cursor::new(&rbxl_data_b)).unwrap();

        fn compare_trees(
            dom_a: &rbx_dom_weak::WeakDom,
            ref_a: rbx_dom_weak::types::Ref,
            dom_b: &rbx_dom_weak::WeakDom,
            ref_b: rbx_dom_weak::types::Ref,
            path: &str,
        ) {
            let inst_a = dom_a.get_by_ref(ref_a).unwrap();
            let inst_b = dom_b.get_by_ref(ref_b).unwrap();

            assert_eq!(
                inst_a.name, inst_b.name,
                "Name mismatch at {path}: {:?} vs {:?}",
                inst_a.name, inst_b.name
            );
            assert_eq!(
                inst_a.class, inst_b.class,
                "Class mismatch at {path}: {:?} vs {:?}",
                inst_a.class, inst_b.class
            );

            let mut children_a: Vec<_> = inst_a
                .children()
                .iter()
                .map(|&r| {
                    let c = dom_a.get_by_ref(r).unwrap();
                    (c.name.clone(), c.class.clone(), r)
                })
                .collect();
            children_a.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

            let mut children_b: Vec<_> = inst_b
                .children()
                .iter()
                .map(|&r| {
                    let c = dom_b.get_by_ref(r).unwrap();
                    (c.name.clone(), c.class.clone(), r)
                })
                .collect();
            children_b.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));

            assert_eq!(
                children_a.len(),
                children_b.len(),
                "Child count mismatch at {path}: {} vs {}",
                children_a.len(),
                children_b.len()
            );

            for (ca, cb) in children_a.iter().zip(children_b.iter()) {
                let child_path = format!("{path}/{}", ca.0);
                compare_trees(dom_a, ca.2, dom_b, cb.2, &child_path);
            }
        }

        compare_trees(&dom_a, dom_a.root_ref(), &dom_b, dom_b.root_ref(), "root");
    });
}
