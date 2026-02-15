---
name: Fix Ref Audit Issues
overview: "Fix all 4 critical issues and 2 correctness concerns from the Ref two-way sync audit: ServeSession missing instanceMap, stale paths after rename, instance names with / in paths, compute_ref_properties priority conflict, encodeInstance Ref leak, and filter_properties_for_meta implicit Ref handling."
todos:
  - id: fix-serve-session
    content: Fix ServeSession.lua:738 -- add self.__instanceMap as 4th arg to encodePatchUpdate, grep for other missing callers
    status: completed
  - id: fix-priority
    content: Fix compute_ref_properties priority -- swap processing order so Rojo_Ref_* is processed after Rojo_Target_* (path-based wins)
    status: completed
  - id: fix-encode-instance
    content: Fix encodeInstance.lua Ref leak -- add explicit Ref skip before encodeProperty call
    status: completed
  - id: fix-filter-explicit
    content: Fix filter_properties_for_meta -- add explicit Variant::Ref skip alongside UniqueId
    status: completed
  - id: fix-slash-escape
    content: Add escape/unescape helpers for / in instance names, update ref_target_path, get_instance_by_path, inst_path
    status: completed
  - id: fix-stale-paths
    content: Add update_ref_paths_after_rename to ChangeProcessor -- scan tree for stale Rojo_Ref_* paths and update them after rename
    status: completed
  - id: fix-stale-meta
    content: Add update_ref_paths_in_meta helper to rewrite Rojo_Ref_* attributes in meta/model files on disk
    status: completed
  - id: dry-target-helper
    content: "DRY: Extract Rojo_Target_* attribute name formatting into shared helper in rojo_ref.rs (matches ref_attribute_name pattern)"
    status: completed
  - id: dry-binarystring
    content: "DRY: Extract BinaryString -> UTF-8 attribute value parsing into shared helper, used by compute_ref_properties and defer_ref_properties"
    status: completed
  - id: test-parity
    content: "Test: Add CLI syncback parity test -- verify two-way sync and CLI syncback produce identical Rojo_Ref_* output for same input"
    status: cancelled
  - id: test-concurrent
    content: "Test: Add concurrent Ref changes test -- multiple Ref properties changed in same batch on same instance"
    status: cancelled
  - id: test-edge-cases
    content: "Test: Add edge case tests -- Ref to ProjectNode instance, Ref to nested project instance, Ref property name not in reflection"
    status: cancelled
  - id: fix-ambiguous-warning
    content: Add ambiguous path warning in syncback_updated_properties when target path has duplicate-named siblings at any ancestor level
    status: completed
  - id: verify-regression
    content: Run regression tests and verify expected pass/fail outcomes match predictions
    status: completed
isProject: false
---

# Fix All Ref Audit Issues

## Issue 1: ServeSession.lua:738 Missing instanceMap (CRITICAL)

**One-line fix.** [ServeSession.lua](plugin/src/ServeSession.lua) line 738:

```lua
-- Before:
local update = encodePatchUpdate(instance, change.id, propertiesToSync)
-- After:
local update = encodePatchUpdate(instance, change.id, propertiesToSync, self.__instanceMap)
```

Also grep for any OTHER callers of `encodePatchUpdate` that may be missing the 4th arg.

---

## Issue 2: Stale Rojo_Ref_* Paths After Rename (CRITICAL)

When an instance is renamed, all `Rojo_Ref_`* attributes across the tree that reference the old path must be updated to the new path.

### Design

When `handle_tree_event` processes a rename:

1. Compute the **old path** and **new path** of the renamed instance (using `full_path_of` before and after rename)
2. Scan ALL descendants of the tree root for `Rojo_Ref_`* attributes
3. For each attribute whose path **starts with** the old path (prefix match handles children too), replace the old prefix with the new prefix
4. Update the in-memory tree's Attributes property
5. Write the updated meta/model file to disk (with `suppress_path`)

### Implementation

**File:** [change_processor.rs](src/change_processor.rs)

Add a new method `update_ref_paths_after_rename(&self, tree, old_path, new_path)` that:

- Iterates `tree.descendants(root_id)`
- For each instance, checks `properties.get("Attributes")` for `Variant::Attributes`
- For each attribute starting with `REF_PATH_ATTRIBUTE_PREFIX`, checks if the path value starts with `old_path`
- If match: replaces the prefix with `new_path`
- Updates the instance's Attributes property in the tree
- Finds the instance's meta/model file via `InstigatingSource::Path` and rewrites the `Rojo_Ref_`* attribute on disk

Call this method from the rename handler, AFTER the filesystem rename is complete and the tree path has been updated. Both the directory-format rename path (line ~885) and the regular file rename path (line ~973) need it.

**Important:** The old path must be computed BEFORE the rename happens. Capture it early:

```rust
let old_path = tree.inner().full_path_of(instance_id, "/");
// ... do the rename ...
let new_path = tree.inner().full_path_of(instance_id, "/");
update_ref_paths_after_rename(&tree, &old_path, &new_path);
```

The prefix match handles all descendants: if `"Workspace/TestModel"` is renamed to `"Workspace/RenamedModel"`, then `"Workspace/TestModel/Part1"` becomes `"Workspace/RenamedModel/Part1"`.

### Meta file rewriting

Use the existing pattern from `upsert_meta_name_field`:

1. Read the JSON5 file
2. Parse as `serde_json::Value`
3. Navigate to `attributes` object
4. Update matching `Rojo_Ref_*` values
5. Write back with `suppress_path`

Extract a helper in [syncback/meta.rs](src/syncback/meta.rs) (or [rojo_ref.rs](src/rojo_ref.rs)):

```rust
pub fn update_ref_paths_in_meta(
    meta_path: &Path,
    old_prefix: &str,
    new_prefix: &str,
) -> anyhow::Result<bool>  // returns true if any attribute was updated
```

---

## Issue 3: Instance Names Containing "/" Corrupt Ref Paths (CRITICAL)

`full_path_of` joins names with "/" separator. `get_instance_by_path` splits on "/". If an instance name contains "/", the path becomes ambiguous.

### Design

Escape "/" in instance names when building paths. Unescape after splitting when resolving paths. Use `\/` as the escape sequence (backslash-slash).

### Implementation

**File:** [rojo_ref.rs](src/rojo_ref.rs)

Add escape/unescape helpers:

```rust
/// Escape "/" in an instance name for use in ref paths.
/// Uses "\/" as escape sequence.
pub fn escape_ref_path_segment(name: &str) -> Cow<'_, str> {
    if name.contains('/') {
        Cow::Owned(name.replace('/', "\\/"))
    } else {
        Cow::Borrowed(name)
    }
}

/// Unescape "\/" back to "/" in a path segment.
pub fn unescape_ref_path_segment(segment: &str) -> Cow<'_, str> {
    if segment.contains("\\/") {
        Cow::Owned(segment.replace("\\/", "/"))
    } else {
        Cow::Borrowed(segment)
    }
}
```

**File:** [rojo_ref.rs](src/rojo_ref.rs) -- Update `ref_target_path`:

Instead of calling `dom.full_path_of(target_ref, "/")`, build the path manually with escaping:

```rust
pub fn ref_target_path(dom: &WeakDom, target_ref: Ref) -> String {
    let root_ref = dom.root_ref();
    let mut components: Vec<String> = dom
        .ancestors_of(target_ref)
        .filter(|inst| inst.referent() != root_ref)
        .map(|inst| escape_ref_path_segment(&inst.name).into_owned())
        .collect();
    components.reverse();
    components.join("/")
}
```

**File:** [tree.rs](src/snapshot/tree.rs) -- Update `get_instance_by_path`:

Split on unescaped "/" only, then unescape each segment:

```rust
pub fn get_instance_by_path(&self, path: &str) -> Option<Ref> {
    if path.is_empty() {
        return Some(self.get_root_id());
    }
    // Split on "/" but not "\/"
    let segments = split_ref_path(path);
    // ... walk tree using unescaped segment names ...
}
```

Add a path splitting function to `rojo_ref.rs` that splits on unescaped "/" only:

```rust
pub fn split_ref_path(path: &str) -> Vec<String> {
    // Split on "/" that is NOT preceded by "\"
    // Then unescape each segment
}
```

**Also update:** `inst_path` in [syncback/snapshot.rs](src/syncback/snapshot.rs) to use the escaped path builder for CLI syncback consistency.

---

## Issue 4: compute_ref_properties Priority Conflict (CRITICAL)

If both `Rojo_Ref_PrimaryPart` and `Rojo_Target_PrimaryPart` exist, `Rojo_Target_`* silently overwrites `Rojo_Ref_*` because it's processed second.

### Fix

**File:** [patch_compute.rs](src/snapshot/patch_compute.rs) `compute_ref_properties`

Swap the processing order: process `Rojo_Target_`* FIRST, then `Rojo_Ref_*`. Since `map.insert` overwrites, the last insert wins. Processing `Rojo_Ref_*` second means the path-based system has priority (which is correct since it's the preferred system).

```rust
for (attr_name, attr_value) in attributes.iter() {
    // Process legacy Rojo_Target_* FIRST (lower priority)
    if let Some(prop_name) = attr_name.strip_prefix(REF_POINTER_ATTRIBUTE_PREFIX) {
        // ... resolve via ID ...
        map.insert(ustr(prop_name), ...);
        continue;
    }

    // Process Rojo_Ref_* SECOND (higher priority, overwrites if both exist)
    if let Some(prop_name) = attr_name.strip_prefix(REF_PATH_ATTRIBUTE_PREFIX) {
        // ... resolve via path ...
        map.insert(ustr(prop_name), ...);
        continue;
    }
}
```

---

## Issue 5: encodeInstance.lua Ref Leak (Correctness)

**File:** [encodeInstance.lua](plugin/src/ChangeBatcher/encodeInstance.lua)

Ref properties pass through `UNENCODABLE_DATA_TYPES` check and reach `encodeProperty` where they fail via pcall. Add an explicit skip:

```lua
-- After the UNENCODABLE_DATA_TYPES check, before encodeProperty:
if descriptor.dataType == "Ref" then
    -- Ref properties cannot be encoded during instance addition because
    -- the target instance has no server ID yet. They are handled separately
    -- by encodePatchUpdate.lua during property update encoding.
    continue
end
```

---

## Issue 6: filter_properties_for_meta Implicit Ref Handling (Correctness)

**File:** [api.rs](src/web/api.rs) `filter_properties_for_meta`

Add explicit skip with comment for Variant::Ref alongside the UniqueId skip:

```rust
// Skip UniqueId and Ref - they don't serialize to JSON.
// Ref properties are handled upstream in syncback_updated_properties()
// where they are converted to Rojo_Ref_* path-based attributes.
if matches!(value, Variant::UniqueId(_) | Variant::Ref(_)) {
    continue;
}
```

---

## Issue 6b: Ambiguous Path Warning in Two-Way Sync

**File:** [api.rs](src/web/api.rs) `syncback_updated_properties()`

When computing a Ref target path, the two-way sync path does NOT check whether the path is ambiguous (duplicate-named siblings at any ancestor level). CLI syncback has `compute_refs_with_duplicate_siblings` for this. The two-way sync should at minimum log a warning.

After computing the path via `ref_target_path`, check for path ambiguity:

```rust
let path = crate::ref_target_path(tree.inner(), *target_ref);

// Warn if the path is ambiguous (duplicate-named siblings)
if !is_ref_path_unique(tree, *target_ref) {
    log::warn!(
        "Ref property '{}' for instance {:?} has an ambiguous path '{}' \
         (duplicate-named siblings exist). The ref may resolve to the wrong \
         target on rebuild.",
        key, update.id, path
    );
}
```

Add a helper `is_ref_path_unique(tree, target_ref)` that walks ancestors checking for duplicate-named siblings -- reuse the logic from `is_path_unique_with_cache` in [ref_properties.rs](src/syncback/ref_properties.rs) adapted for `RojoTree` instead of `WeakDom`.

---

## Issue 7: DRY -- Rojo_Target_* Attribute Name Formatting

**File:** [ref_properties.rs](src/syncback/ref_properties.rs) line 251

The ID-based `Rojo_Target_`* system uses inline `format!("{REF_POINTER_ATTRIBUTE_PREFIX}{}", link.name)` while the path-based system uses the shared `ref_attribute_name()` helper. Add a matching helper:

```rust
// In rojo_ref.rs:
pub fn ref_target_attribute_name(prop_name: &str) -> String {
    format!("{REF_POINTER_ATTRIBUTE_PREFIX}{prop_name}")
}
```

Update `ref_properties.rs` line 251 to use it.

---

## Issue 8: DRY -- BinaryString to UTF-8 Attribute Value Parsing

**Files:** [patch_compute.rs](src/snapshot/patch_compute.rs) and [patch_apply.rs](src/snapshot/patch_apply.rs)

Both `compute_ref_properties` and `defer_ref_properties` have identical logic for parsing attribute values that may be `Variant::String` or `Variant::BinaryString`:

```rust
let path = match attr_value {
    Variant::String(str) => str.as_str(),
    Variant::BinaryString(bytes) => {
        if let Ok(str) = std::str::from_utf8(bytes.as_ref()) {
            str
        } else {
            log::warn!("...");
            continue;
        }
    }
    _ => {
        log::warn!("...");
        continue;
    }
};
```

Extract to a shared helper in [rojo_ref.rs](src/rojo_ref.rs):

```rust
/// Extract a string value from a Variant that may be String or BinaryString.
/// Returns None with a warning for non-string types or invalid UTF-8.
pub fn variant_as_str<'a>(value: &'a Variant, attr_name: &str) -> Option<&'a str> {
    match value {
        Variant::String(s) => Some(s.as_str()),
        Variant::BinaryString(bytes) => std::str::from_utf8(bytes.as_ref()).ok().or_else(|| {
            log::warn!("Attribute {attr_name} contains invalid UTF-8");
            None
        }),
        _ => {
            log::warn!("Attribute {attr_name} is {:?}, expected String", value.ty());
            None
        }
    }
}
```

---

## Regression Tests

The audit already added 5 regression tests (section 10a-10d). After implementing these fixes:

- `ref_stale_path_after_target_rename` should PASS
- `ref_stale_path_after_parent_rename` should PASS
- `ref_to_instance_added_in_same_request` should still FAIL (separate issue: WeakDom custom IDs)
- Lua spec tests 10c (untracked ref) should still FAIL (by design: untracked instances can't be encoded)
- Lua spec test 10d (missing instanceMap) should still FAIL (test deliberately omits instanceMap arg; fix 1 fixes ServeSession.lua:738 which is a different call site)

### New tests needed:

- Instance name with "/" in Ref path: set PrimaryPart to an instance named "A/B", verify path is escaped and resolves back
- Priority conflict: file with both Rojo_Ref_* and Rojo_Target_* for same property, verify Rojo_Ref_* wins
- CLI syncback parity: build an rbxl with Ref properties, run both CLI syncback and two-way sync, compare output files
- Concurrent Ref changes: set Part0 and Part1 on a WeldConstraint in the same batch, verify both Rojo_Ref_* attributes appear
- Edge cases: Ref targeting a ProjectNode instance, Ref targeting an instance in a nested project, Ref with a property name not in rbx_reflection

