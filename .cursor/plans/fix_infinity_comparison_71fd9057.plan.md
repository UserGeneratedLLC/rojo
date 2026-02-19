---
name: Fix infinity comparison
overview: Make `fuzzyEq` self-contained for all IEEE 754 special values (infinity and NaN), eliminating phantom diffs on properties like `UISizeConstraint.MaxSize` and fixing NaN component comparison in compound types.
todos:
  - id: fix-lua-fuzzyeq
    content: Add exact-equality and NaN checks to `fuzzyEq` in trueEquals.lua
    status: completed
  - id: add-tests
    content: Add infinity and NaN test cases to trueEquals.spec.lua
    status: completed
isProject: false
---

# Fix Infinity and NaN Comparison in fuzzyEq

## Root Cause

Two bugs in `fuzzyEq`:

**1. Infinity:** `fuzzyEq(inf, inf)` returns `false` because `inf - inf = NaN`, and `NaN < x` is always `false`. Affects `UISizeConstraint.MaxSize` which defaults to `Vector2(inf, inf)`.

**2. NaN in compound types:** NaN equality is handled in the `trueEquals` number branch (line 93), but `fuzzyEq` is called directly from `vector2Eq`, `vector3Eq`, `cframeEq`, etc. A Vector2/Vector3/CFrame with a NaN component silently returns `false` instead of `true` because `fuzzyEq` doesn't check for NaN.

## Fix

Make `fuzzyEq` self-contained by adding exact-equality and NaN checks at the top:

```lua
local function fuzzyEq(a: number, b: number): boolean
    if a == b then
        return true
    end
    if a ~= a then
        return b ~= b
    end
    local diff = math.abs(a - b)
    local maxVal = math.max(math.abs(a), math.abs(b), 1)
    return diff < EPSILON or diff < maxVal * EPSILON
end
```

Behavior:

- `inf == inf` -> `true` (caught by `a == b`)
- `-inf == -inf` -> `true` (caught by `a == b`)
- `inf == -inf` -> `false` (falls through, `inf - (-inf) = inf`, fails comparison)
- `NaN == NaN` -> `true` (`a == b` is false for NaN, then `a ~= a` catches it)
- `NaN == 5` -> `false` (`a ~= a` is true but `b ~= b` is false)

Remove the now-redundant NaN check in `trueEquals` line 93 (the number branch `if a ~= a then return b ~= b end`) since `fuzzyEq` handles it directly.

## Files

### 1. [plugin/src/Reconciler/trueEquals.lua](plugin/src/Reconciler/trueEquals.lua) (line 15)

Replace `fuzzyEq` with the version above.

### 2. [plugin/src/Reconciler/trueEquals.spec.lua](plugin/src/Reconciler/trueEquals.spec.lua)

Add test cases for:

- `trueEquals(math.huge, math.huge)` -> true
- `trueEquals(-math.huge, -math.huge)` -> true
- `trueEquals(math.huge, -math.huge)` -> false
- `trueEquals(Vector2.new(math.huge, math.huge), Vector2.new(math.huge, math.huge))` -> true
- `trueEquals(Vector3.new(0/0, 1, 2), Vector3.new(0/0, 1, 2))` -> true (NaN component)
- `trueEquals(Vector2.new(0/0, 0/0), Vector2.new(0/0, 0/0))` -> true (NaN components)

