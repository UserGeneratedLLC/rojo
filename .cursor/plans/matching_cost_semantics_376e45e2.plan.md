---
name: matching cost semantics
overview: "Split `variant_eq` into two variants: keep fuzzy comparison for Studio-facing operations (patch_compute, api, property_filter), and add a new disk-representation comparison for both Rust matching files where cost should reflect actual file changes."
todos:
  - id: expose-fns
    content: Make format_f32/format_f64 pub(crate) in json.rs and cleanup_f32 pub(crate) in resolution.rs
    status: completed
  - id: disk-eq-helpers
    content: Add disk_eq_f32, disk_eq_f32_cleaned, disk_eq_f64 zero-allocation helpers to variant_eq.rs
    status: completed
  - id: variant-eq-disk
    content: Implement variant_eq_disk in variant_eq.rs dispatching per Variant type (Float32 standalone vs composite cleanup)
    status: completed
  - id: update-fwd-matching
    content: Replace variant_eq with variant_eq_disk in src/snapshot/matching.rs (count_own_diffs, diff_variant_pair, count_attributes_diff)
    status: completed
  - id: update-sb-matching
    content: Replace variant_eq with variant_eq_disk in src/syncback/matching.rs (count_own_diffs, diff_variant_pair, attribute diff)
    status: completed
  - id: tests-veq
    content: "Add variant_eq_disk unit tests: fuzzy-vs-disk divergence, cleanup normalization, NaN/Inf/-0, all composite types"
    status: completed
  - id: tests-fwd-large
    content: Add 50+ same-named instance matching tests to src/snapshot/matching.rs with 5 variation groups
    status: completed
  - id: tests-sb-large
    content: Add 50+ same-named instance matching tests to src/syncback/matching.rs with 5 variation groups
    status: completed
  - id: tests-disk-boundary
    content: Add tests with values at format_f32 boundaries (close-but-different-on-disk, different-but-same-on-disk)
    status: completed
  - id: tests-update-existing
    content: Update existing matching test assertions to match variant_eq_disk semantics
    status: completed
isProject: false
---

# Matching Cost Semantics Fix

## Problem

`variant_eq` currently uses fuzzy epsilon comparison (absolute + relative, `EPSILON = 0.0001`) in both Rust matching files. This is wrong because:

- **Rust cost = "will this value result in a file change?"** Two values that format identically on disk should score 0 cost, even if their raw f32 bits differ. Conversely, two values within epsilon but with different disk representations should score 1.
- **Lua cost = "will this change be applied in Studio?"** Fuzzy epsilon is correct here -- it prevents oscillation from Studio float precision rounding.

The fuzzy epsilon can diverge from disk reality in both directions:

- `format_f32(1.00005)` = `"1.00005"`, `format_f32(1.00015)` = `"1.00015"` -- different on disk, but `fuzzy_eq_f32` says "equal" (diff = 0.0001)
- `format_f32(0.123456)` could round both `0.1234561` and `0.1234559` to the same string -- same on disk, but `fuzzy_eq_f32` might say "different"

## Data Flow Analysis

### Disk serialization pipeline (how floats reach files)

```
Variant value
  |
  v
resolution.rs::from_variant()
  |-- Standalone Float32 → AmbiguousValue::Number32(v)        [NO cleanup]
  |-- Standalone Float64 → AmbiguousValue::Number(v as f64)   [NO cleanup]
  |-- Composite types (Vector3, Color3, CFrame, UDim, etc.)
  |     → cleanup_f32(component) then into Array/FullyQualified
  v
json.rs::Json5ValueSerializer
  |-- serialize_f32 → format_f32(v)  [6 significant digits, lexical-write-float]
  |-- serialize_f64 → format_f64(v)  [full precision]
  v
.meta.json5 / .model.json5 on disk
```

`**cleanup_f32` matters for composites.** It rounds to 6 decimal places (via `format!("{:.6}")` round-trip), which zeros out very small values. Example: `cleanup_f32(0.0000001)` = `0.0`, so `format_f32(0.0)` = `"0"` vs `format_f32(0.0000001)` = `"1e-7"` -- completely different disk output.

### Where values come from in each matching file

`**[src/syncback/matching.rs](src/syncback/matching.rs)`:**

- `new_dom` = from rbxl/rbxm binary parse. Raw Studio values, NOT through resolution.rs
- `old_dom` = from filesystem build. Values already in disk-normalized form

`**[src/snapshot/matching.rs](src/snapshot/matching.rs)`:**

- `snap` = from filesystem re-snapshot. Values read from disk, already in disk form
- `tree` = from RojoTree. Could be from initial forward sync (disk form) or two-way sync push (raw Studio values)

In both cases, at least one side may not have been through `cleanup_f32 + format_f32`. The comparison must normalize both sides through the full disk pipeline.

### Other callers of `variant_eq` (KEEP fuzzy -- different semantics)


| File                                                    | Purpose                             | Why fuzzy is correct                                           |
| ------------------------------------------------------- | ----------------------------------- | -------------------------------------------------------------- |
| `[patch_compute.rs](src/snapshot/patch_compute.rs)`     | Compute patches to send to plugin   | Must agree with Lua `diff.lua` (which uses fuzzy `trueEquals`) |
| `[api.rs](src/web/api.rs)`                              | Filter defaults during two-way sync | "Is this at default?" -- not a disk question                   |
| `[property_filter.rs](src/syncback/property_filter.rs)` | Filter defaults during syncback     | Same rationale                                                 |
| `[hash/mod.rs](src/syncback/hash/mod.rs)`               | Exclude defaults from content hash  | Same rationale                                                 |
| `[project.rs](src/snapshot_middleware/project.rs)`      | Check project node value drift      | Same rationale                                                 |


## Implementation

### 1. Expose formatting functions

- `**[src/json.rs](src/json.rs)`:** Make `format_f32`, `format_f64` and the `Options` constants `pub(crate)`
- `**[src/resolution.rs](src/resolution.rs)`:** Make `cleanup_f32` `pub(crate)`

### 2. Add disk-representation comparison to `[src/variant_eq.rs](src/variant_eq.rs)`

Add zero-allocation comparison helpers that write to stack buffers:

```rust
fn disk_eq_f32(a: f32, b: f32) -> bool {
    if a.to_bits() == b.to_bits() { return true; }
    // format both into stack buffers, compare byte slices
}

fn disk_eq_f32_cleaned(a: f32, b: f32) -> bool {
    disk_eq_f32(cleanup_f32(a), cleanup_f32(b))
}

fn disk_eq_f64(a: f64, b: f64) -> bool {
    if a.to_bits() == b.to_bits() { return true; }
    // format both into stack buffers, compare byte slices
}
```

Then `pub fn variant_eq_disk(a: &Variant, b: &Variant) -> bool` that dispatches:


| Variant type                                    | Comparison                                         |
| ----------------------------------------------- | -------------------------------------------------- |
| Float32                                         | `disk_eq_f32` (no cleanup -- standalone)           |
| Float64                                         | `disk_eq_f64` (no cleanup)                         |
| Vector3, Color3, CFrame components              | `disk_eq_f32_cleaned` (cleanup -- composite)       |
| UDim scale / UDim2 scales                       | `disk_eq_f32_cleaned` (cleanup)                    |
| NumberRange, Rect min/max                       | `disk_eq_f32_cleaned` (cleanup)                    |
| NumberSequence, ColorSequence keypoint floats   | `disk_eq_f32_cleaned` (cleanup)                    |
| PhysicalProperties floats                       | `disk_eq_f32_cleaned` (cleanup)                    |
| Ray, Region3 components                         | `disk_eq_f32_cleaned` (cleanup)                    |
| Non-float types (Bool, Int, String, Enum, etc.) | Exact equality (no epsilon)                        |
| Tags                                            | Sorted set comparison (same as now)                |
| Attributes                                      | Recursive with `variant_eq_disk` for nested values |


### 3. Update both matching files

`**[src/snapshot/matching.rs](src/snapshot/matching.rs)`:** Replace all `variant_eq` calls in `count_own_diffs`, `diff_variant_pair`, and `count_attributes_diff` with `variant_eq_disk`.

`**[src/syncback/matching.rs](src/syncback/matching.rs)`:** Same replacements in `count_own_diffs`, `diff_variant_pair`, and attribute comparison.

### 4. Lua side -- no changes needed (verified)

The plugin's cost model is correct as-is:

- `trueEquals` is shared by both `matching.lua` (scoring) and `diff.lua` (patch generation), so they cannot disagree -- if matching says "equal", diff will also say "equal", and the change won't appear in the patch visualizer
- The fuzzy epsilon prevents Studio float-precision oscillation (set 0.5 -> Studio stores 0.49999999 -> reads back as ~0.5 -> trueEquals says "equal" -> no phantom re-diff)
- `matching.lua`'s `countOwnDiffs` fills defaults from `classKeys.defaults` for missing virtual properties, matching what diff/hydrate will do

The Lua side's cost is correctly "will this change be flagged in the patch visualizer / applied to Studio."

### 5. Testing Suite

Existing `variant_eq` tests stay unchanged (they test the fuzzy function which is still used by patch_compute, api, etc.). All new tests below are additive.

#### 5a. `variant_eq_disk` unit tests (in `src/variant_eq.rs`)

**Fuzzy-vs-disk divergence tests** -- these are the cases that prove fuzzy is wrong for file-change detection:

```rust
// Fuzzy says "equal" but disk would differ
assert!(fuzzy_eq_f32(1.00005, 1.00015));           // within epsilon
assert!(!disk_eq_f32(1.00005, 1.00015));            // format_f32 gives different strings

// Fuzzy says "different" but disk would be identical
assert!(!fuzzy_eq_f32(0.1234561, 0.1234559));       // outside relative epsilon
assert!(disk_eq_f32(0.1234561, 0.1234559));          // format_f32 rounds both the same
```

**cleanup_f32 normalization tests** -- composite types get cleaned before formatting:

```rust
// Very small values zeroed by cleanup
assert!(disk_eq_f32_cleaned(0.0000001, 0.0));        // cleanup makes both 0.0
assert!(!disk_eq_f32(0.0000001, 0.0));                // without cleanup, different

// Full Vector3 comparison
assert!(variant_eq_disk(
    &Variant::Vector3(Vector3::new(1.0, 2.0, 0.0000001)),
    &Variant::Vector3(Vector3::new(1.0, 2.0, 0.0)),
));  // cleanup zeros the tiny z component
```

**Edge cases:**

- NaN == NaN (both format to "NaN")
- Inf == Inf (both format to "Infinity")
- -0.0 == 0.0 (both format to "0")
- All composite types: Vector2, Vector3, Color3, CFrame, UDim, UDim2, Rect, Ray, Region3, NumberRange, NumberSequence, ColorSequence, PhysicalProperties
- Non-float types: exact equality (Bool, Int32, Int64, String, Enum, BrickColor)
- Tags: sorted set comparison
- Attributes: recursive disk comparison of nested Variants

#### 5b. Large ambiguous group matching -- forward sync (`src/snapshot/matching.rs`)

**Test: `fifty_same_name_parts_five_variation_groups`**

50 Parts all named `"Line"` with 5 variation groups of 10 exact duplicates each:


| Group | Position     | Color               | Count |
| ----- | ------------ | ------------------- | ----- |
| A     | `[0, 0, 0]`  | `[1, 0, 0]` (red)   | 10    |
| B     | `[0, 5, 0]`  | `[1, 0, 0]` (red)   | 10    |
| C     | `[0, 10, 0]` | `[0, 1, 0]` (green) | 10    |
| D     | `[0, 0, 0]`  | `[0, 1, 0]` (green) | 10    |
| E     | `[0, 5, 0]`  | `[0, 0, 1]` (blue)  | 10    |


- Groups A vs D: same position, different color (color-only diff)
- Groups A vs B: same color, different position (position-only diff)
- Groups B vs E: same position, different color
- Within each group: 10 exact duplicates (any pairing is correct)

Build snapshots in order A,B,C,D,E. Build tree in **reversed** order E,D,C,B,A.

**Assertions:**

1. All 50 matched, 0 unmatched on either side
2. For every matched pair: `Position` values match (within disk equality)
3. For every matched pair: `Color` values match (within disk equality)
4. Completes in < 5 seconds (depth limit prevents exponential blowup)

**Test: `fifty_parts_position_only_variations`**

50 Parts named `"Block"` -- same Color on all, 5 distinct Position values (10 duplicates each). Tree reversed. Verify all 50 pair correctly by Position.

**Test: `fifty_parts_color_only_variations`**

50 Parts named `"Block"` -- same Position on all, 5 distinct Color values (10 duplicates each). Tree reversed. Verify all 50 pair correctly by Color.

**Test: `sixty_parts_with_near_float_values`**

60 Parts named `"Segment"` with Transparency values chosen at `format_f32` boundaries:


| Group | Transparency | Count | Notes                                            |
| ----- | ------------ | ----- | ------------------------------------------------ |
| F     | `0.100000`   | 12    | disk repr: `"0.1"`                               |
| G     | `0.100001`   | 12    | disk repr: `"0.100001"` -- different from F      |
| H     | `0.500000`   | 12    | disk repr: `"0.5"`                               |
| I     | `0.500001`   | 12    | disk repr differs from H                         |
| J     | `0.999999`   | 12    | disk repr rounds to same or different from `1.0` |


The exact Transparency values will be chosen by testing `format_f32` output during implementation to find values that:

- Are within fuzzy epsilon of each other but format differently (must NOT cross-pair)
- Are outside fuzzy epsilon but format the same (must cross-pair)

This test specifically catches the bug where fuzzy equality would match Group F with Group G instances, causing 12 unnecessary file rewrites.

#### 5c. Large ambiguous group matching -- syncback (`src/syncback/matching.rs`)

Same test matrix as 5b but using `WeakDom` + `InstanceBuilder`:

**Test: `fifty_same_name_parts_five_groups_syncback`**

Same 5 variation groups (A-E) of 10 Parts each. `new_dom` in order, `old_dom` reversed.

```rust
let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
let new_root = new_dom.root_ref();
for (pos, color) in &groups {
    for _ in 0..10 {
        new_dom.insert(new_root, InstanceBuilder::new("Part")
            .with_name("Line")
            .with_property("Position", Variant::Vector3(*pos))
            .with_property("Color", Variant::Color3(*color)));
    }
}
```

Assertions identical to 5b: all 50 paired, Position+Color match on every pair.

**Test: `sixty_parts_near_float_syncback`**

Same boundary test as 5b's `sixty_parts_with_near_float_values`, but in syncback context.

#### 5d. Disk boundary stress test

**Test: `disk_representation_boundary_matching`** (in both matching files)

Explicitly construct instances where fuzzy and disk comparison disagree, and verify matching picks the disk-correct pairing:

2 Parts named `"Edge"`:

- Snap/New A: Transparency = `1.00005` (format_f32 = `"1.00005"`)
- Snap/New B: Transparency = `1.00015` (format_f32 = `"1.00015"`)

Tree/Old (reversed):

- Tree/Old X: Transparency = `1.00015`
- Tree/Old Y: Transparency = `1.00005`

With fuzzy equality: both (A,X) and (A,Y) score 0 (within epsilon) -- random pairing, 50% chance of wrong.
With disk equality: (A,Y) scores 0 and (A,X) scores 1 -- deterministically correct.

Verify A matches Y and B matches X.

#### 5e. Update existing tests

Existing matching tests that assert `variant_eq` behavior in matching contexts will continue to pass because:

- Most use exact integer/enum values (not affected by fuzzy vs disk)
- Float tests use values that are either identical or far apart

Tests that use float values at fuzzy epsilon boundaries may need value adjustments if the old assertion relied on fuzzy tolerance. Specifically review:

- `many_same_name_parts_matched_by_properties` (uses `i * 0.1` increments -- safe, these format distinctly)
- `ten_ambiguous_textures_stress` (uses Enum values -- safe, not float)
- `variant_eq_float32_with_new_epsilon` (tests `variant_eq` not `variant_eq_disk` -- unchanged)

No existing test should break, but verify during implementation.