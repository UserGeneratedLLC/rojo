---
name: Consolidate filesystem_name_for
overview: Eliminate the duplicated InstigatingSource branching logic by having `ref_target_path_from_tree` call `tree.filesystem_name_for()` instead of reimplementing the same logic inline.
todos:
  - id: consolidate
    content: Replace inline InstigatingSource branching in ref_target_path_from_tree with tree.filesystem_name_for() call
    status: completed
  - id: verify
    content: Run cargo test and cargo clippy to confirm no regressions
    status: completed
isProject: false
---

# Consolidate `filesystem_name_for` Duplication

## Problem

`ref_target_path_from_tree` (`[src/rojo_ref.rs](src/rojo_ref.rs)` lines 140-191) reimplements the same `InstigatingSource` branching logic that `filesystem_name_for` (`[src/snapshot/tree.rs](src/snapshot/tree.rs)` lines 394-418) already provides. The [ref path audit](c:/Users/Joe/.cursor/plans/audit_ref_path_system_c21fc130.plan.md) confirmed these produce identical results for all branches.

## Approach

`ref_target_path_from_tree` already receives `&RojoTree` and `filesystem_name_for` is already `pub` on `RojoTree`. The fix is to replace the inline branching with a call to the existing method. No new functions, no signature changes, no new files.

## Change

**Single file: `[src/rojo_ref.rs](src/rojo_ref.rs)`** -- Replace the 25-line `let fs_name = if let Some(meta) ...` block (lines 158-183) inside `ref_target_path_from_tree` with a single call to `tree.filesystem_name_for(current)`:

Before (lines 140-191):

```rust
pub fn ref_target_path_from_tree(tree: &crate::snapshot::RojoTree, target_ref: Ref) -> String {
    let dom = tree.inner();
    let root_ref = dom.root_ref();
    let mut components: Vec<String> = Vec::new();
    let mut current = target_ref;

    loop {
        if current == root_ref || current.is_none() { break; }
        let inst = match dom.get_by_ref(current) {
            Some(i) => i,
            None => break,
        };

        // 25 lines of InstigatingSource branching (duplicated from tree.rs)
        let fs_name = if let Some(meta) = tree.get_metadata(current) {
            // ... Path branch ...
            // ... ProjectNode branch ...
            // ... no source fallback ...
        } else {
            inst.name.clone()
        };

        components.push(fs_name);
        current = inst.parent();
    }
    components.reverse();
    components.join("/")
}
```

After:

```rust
pub fn ref_target_path_from_tree(tree: &crate::snapshot::RojoTree, target_ref: Ref) -> String {
    let dom = tree.inner();
    let root_ref = dom.root_ref();
    let mut components: Vec<String> = Vec::new();
    let mut current = target_ref;

    loop {
        if current == root_ref || current.is_none() { break; }
        let inst = match dom.get_by_ref(current) {
            Some(i) => i,
            None => break,
        };
        components.push(tree.filesystem_name_for(current));
        current = inst.parent();
    }
    components.reverse();
    components.join("/")
}
```

## Scope

- **1 file** changed
- **~25 lines** removed (duplicated branching logic)
- **1 line** added (call to existing method)
- **0 call sites** affected (function signature unchanged)
- **0 test changes** needed (behavior is identical per the audit)

## Verification

- `cargo test` -- all existing ref path tests (100+ unit, 30+ integration) exercise this function
- `cargo clippy` -- no new warnings expected

