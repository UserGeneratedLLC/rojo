use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use hyper_tungstenite::tungstenite::{connect, Message};
use rbx_dom_weak::types::Ref;

use serde::Deserialize;
use tempfile::{tempdir, TempDir};

use librojo::web_api::{
    ReadResponse, SerializeResponse, ServerInfoResponse, SocketPacket, SocketPacketBody,
    SocketPacketType,
};
use rojo_insta_ext::RedactionMap;

use crate::rojo_test::io_util::{
    copy_recursive, get_working_dir_path, KillOnDrop, ROJO_PATH, SERVE_TESTS_PATH,
};

/// Convenience method to run a `rojo serve` test.
///
/// Test projects should be defined in the `serve-tests` folder; their filename
/// should be given as the first parameter.
///
/// The passed in callback is where the actual test body should go. Setup and
/// cleanup happens automatically.
pub fn run_serve_test(test_name: &str, callback: impl FnOnce(TestServeSession, RedactionMap)) {
    let _ = env_logger::try_init();

    let mut redactions = RedactionMap::default();

    let mut session = TestServeSession::new(test_name);
    let info = session.wait_to_come_online();

    redactions.intern(info.session_id);
    redactions.intern(info.root_instance_id);

    let mut settings = insta::Settings::new();

    let snapshot_path = Path::new(SERVE_TESTS_PATH)
        .parent()
        .unwrap()
        .join("serve-test-snapshots");

    settings.set_snapshot_path(snapshot_path);
    settings.set_sort_maps(true);
    settings.add_redaction(".serverVersion", "[server-version]");
    // messageCursor is non-deterministic: event batching varies across
    // platforms (macOS kqueue, Windows ReadDirectoryChanges) and can bump
    // the cursor beyond what the test expects. The exact cursor value is
    // irrelevant to snapshot correctness.
    // .messageCursor covers ReadResponse (root level), .body.messageCursor
    // covers SocketPacket (nested under body).
    settings.add_redaction(".messageCursor", "[message-cursor]");
    settings.add_redaction(".body.messageCursor", "[message-cursor]");
    settings.bind(move || callback(session, redactions));
}

/// Represents a running Rojo serve session running in a temporary directory.
pub struct TestServeSession {
    // Drop order is important here: we want the process to be killed before the
    // directory it's operating on is destroyed.
    rojo_process: KillOnDrop,
    _dir: TempDir,

    port: usize,
    project_path: PathBuf,
}

impl TestServeSession {
    pub fn new(name: &str) -> Self {
        let working_dir = get_working_dir_path();

        let source_path = Path::new(SERVE_TESTS_PATH).join(name);
        let dir = tempdir().expect("Couldn't create temporary directory");
        let project_path = dir
            .path()
            .canonicalize()
            .expect("Couldn't canonicalize temporary directory path")
            .join(name);

        let source_is_file = fs::metadata(&source_path).unwrap().is_file();

        if source_is_file {
            fs::copy(&source_path, &project_path).expect("couldn't copy project file");
        } else {
            fs::create_dir(&project_path).expect("Couldn't create temporary project subdirectory");

            copy_recursive(&source_path, &project_path)
                .expect("Couldn't copy project to temporary directory");
        };

        // This is an ugly workaround for FSEvents sometimes reporting events
        // for the above copy operations, similar to this Stack Overflow question:
        // https://stackoverflow.com/questions/47679298/howto-avoid-receiving-old-events-in-fseventstream-callback-fsevents-framework-o
        // We'll hope that 100ms is enough for FSEvents to get whatever it is
        // out of its system.
        // TODO: find a better way to avoid processing these spurious events.
        #[cfg(target_os = "macos")]
        std::thread::sleep(Duration::from_millis(100));

        let port = get_port_number();
        let port_string = port.to_string();

        let rojo_process = Command::new(ROJO_PATH)
            .args([
                "serve",
                project_path.to_str().unwrap(),
                "--port",
                port_string.as_str(),
            ])
            .current_dir(working_dir)
            .stderr(Stdio::piped())
            .spawn()
            .expect("Couldn't start Rojo");

        TestServeSession {
            rojo_process: KillOnDrop(rojo_process),
            _dir: dir,
            port,
            project_path,
        }
    }

    /// Creates a test session with a git repo initialized in the project dir.
    /// The `setup` callback runs after the fixture is copied and git is initialized
    /// but BEFORE the serve process starts. Use it to commit initial files and
    /// make modifications that should appear as git changes.
    pub fn new_with_git(name: &str, setup: impl FnOnce(&Path)) -> Self {
        let working_dir = get_working_dir_path();

        let source_path = Path::new(SERVE_TESTS_PATH).join(name);
        let dir = tempdir().expect("Couldn't create temporary directory");
        let project_path = dir
            .path()
            .canonicalize()
            .expect("Couldn't canonicalize temporary directory path")
            .join(name);

        fs::create_dir(&project_path).expect("Couldn't create temporary project subdirectory");
        copy_recursive(&source_path, &project_path)
            .expect("Couldn't copy project to temporary directory");

        // Initialize git repo and run setup callback
        Command::new("git")
            .args(["init"])
            .current_dir(&project_path)
            .output()
            .expect("git init failed");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&project_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&project_path)
            .output()
            .unwrap();

        setup(&project_path);

        #[cfg(target_os = "macos")]
        std::thread::sleep(Duration::from_millis(100));

        let port = get_port_number();
        let port_string = port.to_string();

        let rojo_process = Command::new(ROJO_PATH)
            .args([
                "serve",
                project_path.to_str().unwrap(),
                "--port",
                port_string.as_str(),
            ])
            .current_dir(working_dir)
            .stderr(Stdio::piped())
            .spawn()
            .expect("Couldn't start Rojo");

        TestServeSession {
            rojo_process: KillOnDrop(rojo_process),
            _dir: dir,
            port,
            project_path,
        }
    }

    pub fn path(&self) -> &Path {
        &self.project_path
    }

    #[allow(dead_code)]
    pub fn port(&self) -> usize {
        self.port
    }

    /// Waits for the `rojo serve` server to come online with expontential
    /// backoff.
    pub fn wait_to_come_online(&mut self) -> ServerInfoResponse {
        const BASE_DURATION_MS: f32 = 30.0;
        const EXP_BACKOFF_FACTOR: f32 = 1.3;
        const MAX_TRIES: u32 = 5;

        for i in 1..=MAX_TRIES {
            match self.rojo_process.0.try_wait() {
                Ok(Some(status)) => {
                    let mut stderr_output = String::new();
                    if let Some(mut stderr) = self.rojo_process.0.stderr.take() {
                        let _ = stderr.read_to_string(&mut stderr_output);
                    }
                    panic!(
                        "Rojo process exited with status {}\nstderr:\n{}",
                        status, stderr_output
                    );
                }
                Ok(None) => { /* The process is still running, as expected */ }
                Err(err) => panic!("Failed to wait on Rojo process: {}", err),
            }

            let info = match self.get_api_rojo() {
                Ok(info) => info,
                Err(err) => {
                    let retry_time_ms = BASE_DURATION_MS * (i as f32).powf(EXP_BACKOFF_FACTOR);
                    let retry_time = Duration::from_millis(retry_time_ms as u64);

                    log::info!("Server error, retrying in {:?}: {}", retry_time, err);
                    thread::sleep(retry_time);
                    continue;
                }
            };

            log::info!("Got session info: {:?}", info);

            return info;
        }

        panic!("Rojo server did not respond after {} tries.", MAX_TRIES);
    }

    pub fn get_api_rojo(&self) -> Result<ServerInfoResponse, reqwest::Error> {
        let url = format!("http://localhost:{}/api/rojo", self.port);
        let body = reqwest::blocking::get(url)?.bytes()?;

        Ok(deserialize_msgpack(&body).expect("Server returned malformed response"))
    }

    pub fn get_api_read(&self, id: Ref) -> Result<ReadResponse<'_>, reqwest::Error> {
        let url = format!("http://localhost:{}/api/read/{}", self.port, id);
        let body = reqwest::blocking::get(url)?.bytes()?;

        Ok(deserialize_msgpack(&body).expect("Server returned malformed response"))
    }

    pub fn get_api_socket_packet(
        &self,
        packet_type: SocketPacketType,
        cursor: u32,
    ) -> Result<SocketPacket<'static>, Box<dyn std::error::Error>> {
        self.recv_socket_packet(packet_type, cursor, || {})
    }

    /// Start listening on the WebSocket, then run the provided action (e.g. a
    /// file modification), and wait for the expected packet.
    ///
    /// This avoids race conditions where the file watcher hasn't processed a
    /// change before the WebSocket connects: by connecting first, the listener
    /// is guaranteed to be in place when the change is detected.
    ///
    /// After receiving the first matching packet, continues collecting for a
    /// short settle period to handle cases where a single filesystem operation
    /// (e.g., rename + meta write) produces multiple WebSocket messages on
    /// platforms where events aren't batched (notably Windows).
    pub fn recv_socket_packet(
        &self,
        packet_type: SocketPacketType,
        cursor: u32,
        action: impl FnOnce(),
    ) -> Result<SocketPacket<'static>, Box<dyn std::error::Error>> {
        let url = format!("ws://localhost:{}/api/socket/{}", self.port, cursor);

        let (mut socket, _response) = connect(url)?;

        // Set a read timeout on the underlying TCP stream to prevent blocking forever.
        // Without this, socket.read() blocks indefinitely if no data arrives.
        let timeout = Duration::from_secs(10);
        if let hyper_tungstenite::tungstenite::stream::MaybeTlsStream::Plain(ref stream) =
            socket.get_ref()
        {
            stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        }

        // Now that the WebSocket is connected and listening, perform the action
        // that should trigger the change (e.g. writing/deleting a file).
        action();

        let start = std::time::Instant::now();
        let mut collected: Option<SocketPacket<'static>> = None;
        let mut last_received: Option<std::time::Instant> = None;

        // After receiving the first message, keep collecting for this duration
        // after the LAST received message. This handles split events on Windows
        // where rename = REMOVE + CREATE may produce separate messages.
        // Must be shorter than the 200ms reconciliation timer to avoid
        // capturing tree-correction messages as part of the same batch.
        let settle = Duration::from_millis(100);

        loop {
            // Hard timeout: no messages at all within 10 seconds
            if start.elapsed() > timeout && collected.is_none() {
                return Err("Timeout waiting for packet from WebSocket".into());
            }

            // Settle timeout: we have at least one message, and no new
            // messages arrived for `settle` duration — return collected.
            if let Some(last) = last_received {
                if last.elapsed() >= settle {
                    let _ = socket.close(None);
                    return Ok(collected.unwrap());
                }
            }

            match socket.read() {
                Ok(Message::Binary(binary)) => {
                    let packet: SocketPacket = deserialize_msgpack(&binary)?;
                    if packet.packet_type != packet_type {
                        continue;
                    }

                    match collected.as_mut() {
                        Some(existing) => {
                            merge_socket_packets(existing, packet);
                        }
                        None => {
                            collected = Some(packet);
                        }
                    }
                    last_received = Some(std::time::Instant::now());
                }
                Ok(Message::Close(_)) => {
                    if let Some(packet) = collected {
                        return Ok(packet);
                    }
                    return Err("WebSocket closed before receiving messages".into());
                }
                Ok(_) => {
                    // Ignore other message types (ping, pong, text)
                    continue;
                }
                Err(hyper_tungstenite::tungstenite::Error::Io(e))
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut
                        || e.kind() == std::io::ErrorKind::Interrupted =>
                {
                    // No data available yet, read timed out, or interrupted by signal - try again
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    pub fn get_api_serialize(&self, ids: &[Ref]) -> Result<SerializeResponse, reqwest::Error> {
        let mut id_list = String::with_capacity(ids.len() * 33);
        for id in ids {
            write!(id_list, "{id},").unwrap();
        }
        id_list.pop();

        let url = format!("http://localhost:{}/api/serialize/{}", self.port, id_list);

        let body = reqwest::blocking::get(url)?.bytes()?;

        Ok(deserialize_msgpack(&body).expect("Server returned malformed response"))
    }

    /// Assert the in-memory tree exactly matches the filesystem.
    /// Calls the read-only `/api/validate-tree` endpoint which re-snapshots
    /// from disk and diffs against the current tree without applying corrections.
    /// Panics with drift counts if any discrepancy is found.
    pub fn assert_tree_fresh(&self) {
        let url = format!("http://localhost:{}/api/validate-tree", self.port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to build reqwest client");
        let body = client
            .get(&url)
            .send()
            .expect("Failed to call /api/validate-tree")
            .bytes()
            .expect("Failed to read validate-tree response body");
        let report: librojo::TreeFreshnessReport =
            deserialize_msgpack(&body).expect("Failed to deserialize TreeFreshnessReport");

        assert!(
            report.is_fresh,
            "Tree does not match filesystem: {} added, {} removed, {} updated \
             (validated in {:.1}ms)",
            report.added, report.removed, report.updated, report.elapsed_ms
        );
    }

    /// Post to /api/write to simulate plugin syncback operations.
    /// Uses the library's WriteRequest type for proper serialization.
    /// Uses human-readable msgpack format to match server expectations.
    pub fn post_api_write(
        &self,
        request: &librojo::web_api::WriteRequest,
    ) -> Result<(), reqwest::Error> {
        use serde::Serialize;

        let url = format!("http://localhost:{}/api/write", self.port);

        // Serialize with human-readable mode to match server expectations
        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        request
            .serialize(&mut serializer)
            .expect("Failed to serialize WriteRequest");

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to build reqwest client");
        let response = client.post(url).body(body).send()?;

        if !response.status().is_success() {
            panic!(
                "Write request failed with status {}: {}",
                response.status(),
                response.text().unwrap_or_default()
            );
        }

        Ok(())
    }
}

fn deserialize_msgpack<'a, T: Deserialize<'a>>(
    input: &'a [u8],
) -> Result<T, rmp_serde::decode::Error> {
    let mut deserializer = rmp_serde::Deserializer::new(input).with_human_readable();

    T::deserialize(&mut deserializer)
}

/// Obtain a free port by asking the OS to assign an ephemeral one.
///
/// Binds a temporary TcpListener to port 0 (the OS picks a free port),
/// reads back the assigned port, then drops the listener. The brief
/// TOCTOU window before Rojo rebinds is negligible on localhost.
fn get_port_number() -> usize {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port for test");
    let port = listener.local_addr().unwrap().port() as usize;
    drop(listener);
    port
}

/// Build a ServiceChunk by serializing child instances into rbxm format.
pub fn make_service_chunk(
    class_name: &str,
    children: Vec<rbx_dom_weak::InstanceBuilder>,
) -> librojo::web_api::ServiceChunk {
    use rbx_dom_weak::WeakDom;

    let mut dom = WeakDom::new(rbx_dom_weak::InstanceBuilder::new("DataModel"));
    let root = dom.root_ref();
    for child in children {
        dom.insert(root, child);
    }
    let refs: Vec<Ref> = dom.root().children().to_vec();
    let mut buf = Vec::new();
    rbx_binary::to_writer(&mut buf, &dom, &refs).unwrap();
    librojo::web_api::ServiceChunk {
        class_name: class_name.to_string(),
        data: buf,
    }
}

/// Build a complete rbxl file from service chunks, mirroring the same data
/// that live syncback receives. Used for CLI parity testing.
pub fn make_rbxl_from_chunks(chunks: &[librojo::web_api::ServiceChunk]) -> Vec<u8> {
    use rbx_dom_weak::WeakDom;
    use std::collections::HashMap;
    use std::io::Cursor;

    let mut dom = WeakDom::new(rbx_dom_weak::InstanceBuilder::new("DataModel"));
    let root_ref = dom.root_ref();

    for chunk in chunks {
        let chunk_dom = rbx_binary::from_reader(Cursor::new(&chunk.data))
            .unwrap_or_else(|e| panic!("Failed to parse rbxm for {}: {e}", chunk.class_name));

        let service_ref =
            dom.insert(root_ref, rbx_dom_weak::InstanceBuilder::new(&chunk.class_name));

        let mut ref_map: HashMap<Ref, Ref> = HashMap::new();
        for &child_ref in chunk_dom.root().children() {
            deep_clone_into_test(&chunk_dom, &mut dom, child_ref, service_ref, &mut ref_map);
        }

        let all_refs: Vec<Ref> = ref_map.values().copied().collect();
        for inst_ref in all_refs {
            let props_to_fix: Vec<(String, Ref)> = {
                let inst = dom.get_by_ref(inst_ref).unwrap();
                inst.properties
                    .iter()
                    .filter_map(|(key, value)| {
                        if let rbx_dom_weak::types::Variant::Ref(r) = value {
                            ref_map.get(r).map(|&mapped| (key.to_string(), mapped))
                        } else {
                            None
                        }
                    })
                    .collect()
            };
            if !props_to_fix.is_empty() {
                let inst = dom.get_by_ref_mut(inst_ref).unwrap();
                for (key, new_ref) in props_to_fix {
                    inst.properties
                        .insert(key.into(), rbx_dom_weak::types::Variant::Ref(new_ref));
                }
            }
        }
    }

    let service_refs: Vec<Ref> = dom.root().children().to_vec();
    let mut buf = Vec::new();
    rbx_binary::to_writer(&mut buf, &dom, &service_refs).unwrap();
    buf
}

fn deep_clone_into_test(
    source: &rbx_dom_weak::WeakDom,
    target: &mut rbx_dom_weak::WeakDom,
    source_ref: Ref,
    target_parent: Ref,
    ref_map: &mut std::collections::HashMap<Ref, Ref>,
) {
    let inst = source.get_by_ref(source_ref).unwrap();
    let mut builder =
        rbx_dom_weak::InstanceBuilder::new(inst.class.as_str()).with_name(inst.name.as_str());

    for (key, value) in &inst.properties {
        builder = builder.with_property(key.as_str(), value.clone());
    }

    let new_ref = target.insert(target_parent, builder);
    ref_map.insert(source_ref, new_ref);

    for &child_ref in inst.children() {
        deep_clone_into_test(source, target, child_ref, new_ref, ref_map);
    }
}

/// Run CLI syncback on a fresh copy of a fixture using the same data
/// that was sent to live syncback. Returns the temp dir (keep alive) and
/// project path.
pub fn run_cli_syncback_on_chunks(
    fixture_name: &str,
    chunks: &[librojo::web_api::ServiceChunk],
) -> (TempDir, PathBuf) {
    use crate::rojo_test::io_util::{copy_recursive, ROJO_PATH, SERVE_TESTS_PATH};

    let source_path = Path::new(SERVE_TESTS_PATH).join(fixture_name);
    let dir = tempdir().expect("Couldn't create temp dir for CLI syncback");
    let project_path = dir.path().join(fixture_name);
    fs::create_dir(&project_path).expect("Couldn't create project subdirectory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy fixture");

    let rbxl_data = make_rbxl_from_chunks(chunks);
    let rbxl_path = dir.path().join("input.rbxl");
    fs::write(&rbxl_path, rbxl_data).expect("Failed to write rbxl");

    let output = std::process::Command::new(ROJO_PATH)
        .args([
            "syncback",
            project_path.to_str().unwrap(),
            "--input",
            rbxl_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run atlas syncback");

    if !output.status.success() {
        panic!(
            "atlas syncback failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    (dir, project_path)
}

impl TestServeSession {
    /// Post to /api/syncback with raw bytes. Returns the full response
    /// for status code inspection.
    pub fn post_api_syncback_raw(&self, body: Vec<u8>) -> reqwest::blocking::Response {
        let url = format!("http://localhost:{}/api/syncback", self.port);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build reqwest client");
        client
            .post(url)
            .body(body)
            .send()
            .expect("Failed to send syncback request")
    }

    /// Post to /api/syncback with correct version/protocol info.
    /// Panics if the response is not 200.
    pub fn post_api_syncback(
        &self,
        place_id: Option<u64>,
        services: Vec<librojo::web_api::ServiceChunk>,
    ) {
        use serde::Serialize;

        let request = serde_json::json!({
            "protocolVersion": librojo::web_api::PROTOCOL_VERSION,
            "serverVersion": env!("CARGO_PKG_VERSION"),
            "placeId": place_id,
            "services": services,
        });

        let mut body = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut body)
            .with_human_readable()
            .with_struct_map();
        request
            .serialize(&mut serializer)
            .expect("Failed to serialize syncback request");

        let response = self.post_api_syncback_raw(body);
        assert!(
            response.status().is_success(),
            "Syncback request failed with status {}: {}",
            response.status(),
            response.text().unwrap_or_default()
        );
    }

    /// Wait for the server to come back online after a syncback-triggered
    /// restart. Uses longer timeouts than initial startup since syncback
    /// runs between teardown and restart.
    pub fn wait_to_come_back_online(&self) -> ServerInfoResponse {
        const BASE_DURATION_MS: f32 = 200.0;
        const EXP_BACKOFF_FACTOR: f32 = 1.3;
        const MAX_TRIES: u32 = 20;

        for i in 1..=MAX_TRIES {
            let info = match self.get_api_rojo() {
                Ok(info) => info,
                Err(_) => {
                    let retry_time_ms = BASE_DURATION_MS * (i as f32).powf(EXP_BACKOFF_FACTOR);
                    let retry_time = Duration::from_millis(retry_time_ms as u64);
                    thread::sleep(retry_time);
                    continue;
                }
            };

            return info;
        }

        panic!(
            "Server did not come back online after syncback ({} tries)",
            MAX_TRIES
        );
    }
}

/// Extract the message cursor from a SocketPacket for use in subsequent
/// WebSocket subscriptions when chaining multi-step tests.
pub fn get_message_cursor(packet: &SocketPacket) -> u32 {
    match &packet.body {
        SocketPacketBody::Messages(msg) => msg.message_cursor,
    }
}

/// A normalized instance representation for structural tree comparison.
/// Two separate `rojo serve` sessions produce different `Ref` values,
/// so we need a Ref-free representation for round-trip identity checks.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct NormalizedInstance {
    pub name: String,
    pub class_name: String,
    pub properties: BTreeMap<String, String>,
    pub children: Vec<NormalizedInstance>,
}

/// Normalize a ReadResponse into a NormalizedInstance tree rooted at the given ID.
/// Strips Refs, sorts children by (name, class_name) for deterministic comparison.
pub fn normalize_read_response(read: &ReadResponse, root_id: Ref) -> NormalizedInstance {
    fn normalize_recursive(
        instances: &std::collections::HashMap<Ref, librojo::web_api::Instance>,
        id: Ref,
    ) -> NormalizedInstance {
        let inst = &instances[&id];
        let mut properties: BTreeMap<String, String> = BTreeMap::new();
        for (key, value) in &inst.properties {
            // Skip Ref properties (non-deterministic)
            match &**value {
                rbx_dom_weak::types::Variant::Ref(_) => continue,
                _ => {
                    properties.insert(key.to_string(), format!("{value:?}"));
                }
            }
        }
        let mut children: Vec<NormalizedInstance> = inst
            .children
            .iter()
            .map(|child_id| normalize_recursive(instances, *child_id))
            .collect();
        children.sort_by(|a, b| (&a.name, &a.class_name).cmp(&(&b.name, &b.class_name)));
        NormalizedInstance {
            name: inst.name.to_string(),
            class_name: inst.class_name.to_string(),
            properties,
            children,
        }
    }
    normalize_recursive(&read.instances, root_id)
}

/// Start a fresh `rojo serve` on the given project path, read the full tree,
/// and return a NormalizedInstance tree.
///
/// This verifies the round-trip invariant: the filesystem state written by
/// the live session, when read from scratch, must produce the same tree.
pub fn fresh_rebuild_read(project_path: &Path) -> NormalizedInstance {
    let project_file = if project_path.join("default.project.json5").exists() {
        project_path.join("default.project.json5")
    } else if project_path.join("default.project.json").exists() {
        project_path.join("default.project.json")
    } else {
        panic!(
            "No default project file found in {}",
            project_path.display()
        );
    };

    let port = get_port_number();
    let port_string = port.to_string();
    let working_dir = get_working_dir_path();

    let process = Command::new(ROJO_PATH)
        .args([
            "serve",
            project_file.to_str().unwrap(),
            "--port",
            port_string.as_str(),
        ])
        .current_dir(working_dir)
        .stderr(Stdio::piped())
        .spawn()
        .expect("Couldn't start fresh Rojo for round-trip check");

    let mut kill_guard = KillOnDrop(process);

    // Wait for fresh server to come online with same backoff
    const BASE_DURATION_MS: f32 = 30.0;
    const EXP_BACKOFF_FACTOR: f32 = 1.3;
    const MAX_TRIES: u32 = 5;

    let mut info = None;
    for i in 1..=MAX_TRIES {
        match kill_guard.0.try_wait() {
            Ok(Some(status)) => {
                let mut stderr_output = String::new();
                if let Some(mut stderr) = kill_guard.0.stderr.take() {
                    let _ = stderr.read_to_string(&mut stderr_output);
                }
                panic!(
                    "Fresh Rojo process exited with status {}\nstderr:\n{}",
                    status, stderr_output
                );
            }
            Ok(None) => {}
            Err(err) => panic!("Failed to wait on fresh Rojo process: {}", err),
        }

        let url = format!("http://localhost:{}/api/rojo", port);
        if let Ok(resp) = reqwest::blocking::get(&url) {
            if let Ok(body) = resp.bytes() {
                if let Ok(server_info) = deserialize_msgpack::<ServerInfoResponse>(&body) {
                    info = Some(server_info);
                    break;
                }
            }
        }
        let retry_time_ms = BASE_DURATION_MS * (i as f32).powf(EXP_BACKOFF_FACTOR);
        thread::sleep(Duration::from_millis(retry_time_ms as u64));
    }

    let info = info.expect("Fresh Rojo server did not respond");
    let root_id = info.root_instance_id;

    let url = format!("http://localhost:{}/api/read/{}", port, root_id);
    let body = reqwest::blocking::get(&url)
        .expect("Failed to read from fresh server")
        .bytes()
        .expect("Failed to get bytes from fresh server");
    let read: ReadResponse =
        deserialize_msgpack(&body).expect("Fresh server returned malformed response");

    normalize_read_response(&read, root_id)
}

/// Assert that the live session tree matches a fresh rebuild from the filesystem.
/// This is the core round-trip identity check.
pub fn assert_round_trip(session: &TestServeSession, root_id: Ref) {
    // Give VFS events time to settle before checking
    thread::sleep(Duration::from_millis(500));

    let live_read = session.get_api_read(root_id).unwrap();
    let live_tree = normalize_read_response(&live_read, root_id);
    let fresh_tree = fresh_rebuild_read(session.path());

    assert_eq!(
        live_tree, fresh_tree,
        "Round-trip identity violation: live tree and fresh rebuild differ. \
         The filesystem state does not faithfully represent the instance tree."
    );
}

/// Merge a second SocketPacket's messages into the first packet.
///
/// Used to combine multiple WebSocket messages from split filesystem events
/// (e.g., rename = REMOVE + CREATE on Windows) into a single logical packet
/// for snapshot comparison. Takes the highest message cursor and combines
/// all added/removed/updated entries into one message.
pub fn merge_socket_packets(base: &mut SocketPacket<'static>, other: SocketPacket<'static>) {
    match (&mut base.body, other.body) {
        (SocketPacketBody::Messages(base_msgs), SocketPacketBody::Messages(other_msgs)) => {
            base_msgs.message_cursor = base_msgs.message_cursor.max(other_msgs.message_cursor);
            for msg in other_msgs.messages {
                if let Some(last) = base_msgs.messages.last_mut() {
                    last.removed.extend(msg.removed);
                    last.added.extend(msg.added);
                    last.updated.extend(msg.updated);
                } else {
                    base_msgs.messages.push(msg);
                }
            }
        }
    }
}

/// Takes a SerializeResponse and creates an XML model out of the response.
///
/// Since the provided structure intentionally includes unredacted referents,
/// some post-processing is done to ensure they don't show up in the model.
pub fn serialize_to_xml_model(response: &SerializeResponse, redactions: &RedactionMap) -> String {
    let mut dom = rbx_binary::from_reader(response.model_contents.as_slice()).unwrap();
    // This makes me realize that maybe we need a `descendants_mut` iter.
    let ref_list: Vec<Ref> = dom.descendants().map(|inst| inst.referent()).collect();
    for referent in ref_list {
        let inst = dom.get_by_ref_mut(referent).unwrap();
        if let Some(id) = redactions.get_id_for_value(&inst.name) {
            inst.name = format!("id-{id}");
        }
    }

    let mut data = Vec::new();
    rbx_xml::to_writer_default(&mut data, &dom, dom.root().children()).unwrap();
    String::from_utf8(data).expect("rbx_xml should never produce invalid utf-8")
}
