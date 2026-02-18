---
name: Live syncback fixes
overview: "Fix five issues: lost Workspace attributes, spurious ClockTime, Rojo_Ref_CurrentCamera leak, and float precision noise via f32 truncation."
todos:
  - id: fix-attributes-encoding
    content: Add explicit Attributes/Tags encoding to encodeService.lua (mirroring encodeInstance.lua)
    status: completed
  - id: fix-clocktime
    content: Filter non-serializable properties server-side in build_dom_from_chunks using the Rust reflection DB
    status: completed
  - id: fix-currentcamera-ref
    content: "Stop Rojo_Ref_CurrentCamera from leaking: filter deferred_referents after camera destruction in syncback_loop"
    status: completed
  - id: fix-float-rounding
    content: Truncate all float values to f32 in UnresolvedValue::from_variant for rbxl parity
    status: completed
isProject: false
---

# Live Syncback Fixes

## Issue 1: Workspace Attributes/Properties Lost

**Root cause:** `encodeService.lua` iterates `classDescriptor.properties` but the Luau RbxDom database may not list `Attributes` and `Tags` as enumerable properties. `encodeInstance.lua` has explicit `encodeAttributes()` / `encodeTags()` helpers using `instance:GetAttributes()` / `instance:GetTags()` directly.

**Fix:** Add explicit Attributes and Tags encoding to `[plugin/src/ChangeBatcher/encodeService.lua](plugin/src/ChangeBatcher/encodeService.lua)` after the property loop. Add `if propertyName == "Attributes" or propertyName == "Tags" then continue end` to the main loop to avoid double-encoding.

## Issue 2: Lighting.ClockTime Appearing

**Root cause:** Luau DB marks ClockTime as serializable; Rust DB marks it as DoesNotSerialize. Plugin sends it, CLI syncback filters it.

**Fix:** In `[src/cli/serve.rs](src/cli/serve.rs)` `build_dom_from_chunks`, filter each plugin-sent property through `should_property_serialize` from the Rust reflection DB before applying it to the builder.

## Issue 3: Rojo_Ref_CurrentCamera Leaking (pre-existing bug)

**Root cause:** `collect_referents` captures `Workspace.CurrentCamera` -> Camera ref before camera is destroyed. `link_referents` writes `Rojo_Ref_CurrentCamera` without checking target still exists.

**Fix:** In `[src/syncback/mod.rs](src/syncback/mod.rs)`, after camera destruction (line 222), filter `deferred_referents.path_links` to remove entries whose target ref no longer exists.

## Issue 4: Float Precision

**Root cause:** Live syncback sends Float64 from Lua; rbxl stores Float32. Tiny diffs cause properties to fail default comparison.

**Fix:** In `[src/resolution.rs](src/resolution.rs)` `UnresolvedValue::from_variant`, truncate ALL float values to f32 precision. This is the single chokepoint for json5 writes:

- **Float32(n)** (line 67): already f32, `n as f64` is fine (no change needed)
- **Float64(n)** (line 68): change to `(n as f32) as f64` to truncate
- **Vector2** (line 82): `(vector.x as f32) as f64` etc
- **Vector3** (line 85): same
- **CFrame** (lines 90-103): all 12 components
- **Color3** (line 88): already f32, fine
- **UDim/UDim2**: fall through to `FullyQualified` -- add a `round_variant` helper that truncates float components to f32 before wrapping

Helper:

```rust
fn truncate_to_f32(v: f64) -> f64 {
    (v as f32) as f64
}
```

## Deferred: Matching Parity

Matching differences between live and CLI syncback are likely downstream of the above fixes (extra properties inflating scores, missing attributes, float diffs). Revisit after these four fixes land. If matching is still off, consider reusing the previously-served tree instead of creating a fresh `ServeSession::new_oneshot` in `run_live_syncback`.

