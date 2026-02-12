---
name: Fix stale tree desync
overview: Add a tree validation mechanism that re-snapshots the project tree from the real filesystem when a plugin connects, ensuring the in-memory RojoTree never serves stale data even when VFS watcher events are missed.
todos:
  - id: validate-method
    content: Add `validate_tree()` method to ServeSession that re-snapshots from disk and patches the tree
    status: completed
  - id: trigger-on-connect
    content: Call `validate_tree()` in the WebSocket connection handler before serving data
    status: completed
  - id: broadcast-corrections
    content: Push any corrections from validation to the message queue for all connected plugins
    status: completed
  - id: test-validation
    content: "Test: start serve, externally delete/modify a tracked file, connect plugin, verify tree is corrected"
    status: completed
isProject: false
---

# Fix Stale Tree Desync on Plugin Connect

## Problem

When a plugin connects (especially in one-shot mode), the server's in-memory `RojoTree` can be stale if the VFS file watcher missed external filesystem changes. The tree is built once on startup and only updated via VFS events through `ChangeProcessor`. If events are missed (watcher buffer overflow, rapid batch operations like git, OS-level quirks), the tree silently drifts from reality. The plugin reads this stale tree via `GET /api/read/:id`, computes its diff against Studio, and sees "no changes" even though the filesystem is different.

Rebooting `rojo serve` forces a fresh read and fixes it, confirming the issue is purely stale in-memory state.

## Solution: Tree Validation on Plugin Connect

When a new plugin connection is established (WebSocket handshake), trigger a validation pass that re-snapshots the root project path from the real filesystem, diffs against the current tree, and applies any corrections. This is essentially what `ServeSession::new` does on startup, but incremental and without restart.

## Key Files

- `[src/serve_session.rs](src/serve_session.rs)` -- Owns the tree, VFS, and project. Add `validate_tree()` method here.
- `[src/change_processor.rs](src/change_processor.rs)` -- Has `apply_patches()` which re-snapshots a path and patches the tree. Reuse this pattern.
- `[src/web/api.rs](src/web/api.rs)` -- WebSocket connection handler. Call validation before serving the tree.
- `[src/snapshot/patch_compute.rs](src/snapshot/patch_compute.rs)` -- `compute_patch_set` used to diff snapshots.

## Implementation

### 1. Add `validate_tree()` to ServeSession

In `[src/serve_session.rs](src/serve_session.rs)`, add a public method that:

- Takes the root project's start path (`self.root_project.folder_location()`)
- Locks the VFS and tree
- Calls `snapshot_from_vfs(&instance_context, &vfs, start_path)` to get a fresh snapshot from disk
- Calls `compute_patch_set(snapshot, &tree, root_id)` to diff against the current tree
- If the patch is non-empty, calls `apply_patch_set(&mut tree, patch_set)` and pushes the result to the message queue

This reuses the same functions that `ServeSession::new` uses for initial tree construction. The VFS reads directly from the filesystem (no cache), so the snapshot will reflect the real state.

A compile-time constant at the top of `serve_session.rs` controls whether validation runs:

```rust
/// Set to `false` to skip tree validation on plugin connect (useful for testing).
const VALIDATE_TREE_ON_CONNECT: bool = true;
```

The method itself:

```rust
pub fn validate_tree(&self) -> Vec<AppliedPatchSet> {
    if !VALIDATE_TREE_ON_CONNECT {
        log::debug!("Tree validation skipped (VALIDATE_TREE_ON_CONNECT = false)");
        return Vec::new();
    }

    let start = std::time::Instant::now();
    let start_path = self.root_project.folder_location();
    let instance_context = InstanceContext::new();

    let snapshot = match snapshot_from_vfs(&instance_context, &self.vfs, start_path) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Tree validation snapshot error: {:?}", e);
            return Vec::new();
        }
    };

    let mut tree = self.tree.lock().unwrap();
    let root_id = tree.get_root_id();
    let patch_set = compute_patch_set(snapshot, &tree, root_id);

    if patch_set.is_empty() {
        log::info!("Tree validation complete (no corrections needed) in {:.1?}", start.elapsed());
        return Vec::new();
    }

    log::info!("Tree validation found stale state, applying corrections");
    let applied = apply_patch_set(&mut tree, patch_set);
    log::info!("Tree validation complete in {:.1?}", start.elapsed());
    vec![applied]
}
```

Note: The `Vfs` struct wraps the backend with a mutex internally (`self.inner.lock()`), so we access it via `self.vfs` (which is `Arc<Vfs>`) without needing an explicit lock. The exact locking pattern will follow the existing code style.

### 2. Trigger Validation on WebSocket Connect

In `[src/web/api.rs](src/web/api.rs)`, the WebSocket handler establishes a new connection. Before entering the message loop (or right after the handshake), call `validate_tree()` and broadcast any corrections:

```rust
// In the WebSocket handler, after handshake:
let corrections = self.serve_session.validate_tree();
if !corrections.is_empty() {
    self.serve_session.message_queue().push_messages(&corrections);
}
```

This ensures the tree is consistent with disk before the plugin starts receiving data.

### 3. Broadcast Corrections to Already-Connected Plugins

If multiple plugins are connected, the corrections are pushed to the message queue, which broadcasts to all connected WebSocket clients. This ensures all plugins get the updated state.

## Edge Cases

- **Large projects**: Full re-snapshot could add 100-500ms latency on connect for very large projects. This is acceptable for one-shot mode (happens once) and live mode (happens on reconnect). Can be optimized later with targeted path validation if needed.
- **Concurrent access**: The tree lock is held during validation. Other WebSocket handlers will block briefly. This matches the existing pattern in `ChangeProcessor::apply_patches`.
- **No changes**: If the tree is already fresh (common case), `compute_patch_set` returns an empty patch and no work is done. The overhead is one `snapshot_from_vfs` call (directory walk + file stats).
- **VFS consistency**: After validation, VFS events for the re-snapshotted paths will still fire (watches persist). The `compute_and_apply_changes` handler will process them as no-ops since the tree already matches.

## What This Fixes

After this change, "no changes appearing" on plugin reconnect should not happen regardless of whether VFS events were missed. The tree is always refreshed from disk on connect, matching the behavior of a fresh `rojo serve` startup.

## What This Does NOT Fix

The content "swap" issue (both directions firing during per-item selection) is a separate bug that needs further investigation. The stale tree fix ensures that even if a swap occurs, the developer can immediately reconnect and see the correct diff to recover.