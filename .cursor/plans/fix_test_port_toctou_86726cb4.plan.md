---
name: Fix test port TOCTOU
overview: Two-layer fix for port-in-use race conditions. (1) Server-side -- retry bind with backoff when a known port is busy. (2) Test-side -- retry with a new random port when the serve process dies from AddrInUse.
todos:
  - id: server-bind-retry
    content: Add bind retry with backoff in `LiveServer::start()` for known ports
    status: completed
  - id: respawn-method
    content: Add `respawn_with_new_port()` method to `TestServeSession`
    status: completed
  - id: wait-retry
    content: Modify `wait_to_come_online()` to detect AddrInUse and retry via respawn
    status: completed
  - id: fresh-rebuild-retry
    content: Add port-retry logic to `fresh_rebuild_read()`
    status: completed
isProject: false
---

# Fix Port-in-Use Race Conditions

## Problem

Two failure modes:

1. **Tests (random ports):** `get_port_number()` in `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)` binds port 0, reads the assigned port, drops the listener, then passes the port to the spawned serve process. Another test can grab the same port in that TOCTOU window.
2. **Server (known ports):** `TcpListener::bind(address).await.unwrap()` in `[src/web/mod.rs:80](src/web/mod.rs)` panics immediately if the port is in use, with no retry. A previous server instance shutting down or a transient conflict gives no grace period.

## Layer 1: Server-side bind retry (`src/web/mod.rs`)

Replace the `.unwrap()` on `TcpListener::bind()` at line 80 with a retry loop:

- On `AddrInUse`: retry up to 5 times with exponential backoff (200ms, 400ms, 800ms, 1600ms, 3200ms -- ~6s total)
- On any other bind error, or after retries exhausted: return `anyhow::Error` (propagates up cleanly instead of panicking)

```rust
let listener = {
    const MAX_BIND_RETRIES: u32 = 5;
    const BASE_BACKOFF_MS: u64 = 200;
    let mut last_err = None;
    for attempt in 0..MAX_BIND_RETRIES {
        match TcpListener::bind(address).await {
            Ok(listener) => { last_err = None; break; }
            Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
                let delay = BASE_BACKOFF_MS * 2u64.pow(attempt);
                log::warn!(
                    "Port {} in use, retrying in {}ms (attempt {}/{})",
                    address.port(), delay, attempt + 1, MAX_BIND_RETRIES
                );
                tokio::time::sleep(Duration::from_millis(delay)).await;
                last_err = Some(err);
            }
            Err(err) => { last_err = Some(err); break; }
        }
    }
    // unwrap or propagate
};
```

This helps both tests (brief TOCTOU window) and production (previous process releasing a port).

## Layer 2: Test-side retry with new random port (`tests/rojo_test/serve_util.rs`)

If the serve process still dies after the server-side retries (e.g., the port is held long-term by another test), the test infrastructure retries with a completely new port.

### 2a. Add `respawn_with_new_port()` to `TestServeSession`

Kills the current process, picks a new ephemeral port, and respawns:

```rust
fn respawn_with_new_port(&mut self) {
    let port = get_port_number();
    let port_string = port.to_string();
    let working_dir = get_working_dir_path();

    let rojo_process = Command::new(ROJO_PATH)
        .args(["serve", self.project_path.to_str().unwrap(), "--port", &port_string])
        .current_dir(working_dir)
        .stderr(Stdio::piped())
        .spawn()
        .expect("Couldn't start Rojo");

    self.rojo_process = KillOnDrop(rojo_process);
    self.port = port;
}
```

### 2b. Modify `wait_to_come_online()`

In the `Ok(Some(status))` branch (line ~266-276), detect `AddrInUse` in stderr and retry:

- Add a `port_retries` counter (max 3 attempts)
- On `AddrInUse`: call `self.respawn_with_new_port()`, reset the backoff counter, continue
- On other errors: panic as before

### 2c. Apply same pattern to `fresh_rebuild_read()`

`fresh_rebuild_read()` (line ~929) has its own inline wait loop. Wrap the spawn + wait in a retry loop that detects `AddrInUse` and retries with a new port.

## Files changed

- `[src/web/mod.rs](src/web/mod.rs)` -- bind retry with backoff (Layer 1)
- `[tests/rojo_test/serve_util.rs](tests/rojo_test/serve_util.rs)` -- respawn with new port (Layer 2)

