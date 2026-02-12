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
    // messageCursor is non-deterministic: on macOS kqueue delivers extra
    // per-file vnode events that can bump the cursor beyond what the test
    // expects. The exact cursor value is irrelevant to snapshot correctness.
    settings.add_redaction(".messageCursor", "[message-cursor]");
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

    pub fn path(&self) -> &Path {
        &self.project_path
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

        loop {
            if start.elapsed() > timeout {
                return Err("Timeout waiting for packet from WebSocket".into());
            }

            match socket.read() {
                Ok(Message::Binary(binary)) => {
                    let packet: SocketPacket = deserialize_msgpack(&binary)?;
                    if packet.packet_type != packet_type {
                        continue;
                    }

                    // Close the WebSocket connection now that we got what we were waiting for
                    let _ = socket.close(None);
                    return Ok(packet);
                }
                Ok(Message::Close(_)) => {
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
        let body = reqwest::blocking::get(url)
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
        match reqwest::blocking::get(&url) {
            Ok(resp) => {
                if let Ok(body) = resp.bytes() {
                    if let Ok(server_info) =
                        deserialize_msgpack::<ServerInfoResponse>(&body)
                    {
                        info = Some(server_info);
                        break;
                    }
                }
            }
            Err(_) => {}
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
