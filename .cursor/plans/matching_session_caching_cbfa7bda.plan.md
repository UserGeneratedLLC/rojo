---
name: Matching session caching
overview: Rewrite the Lua matching module for native codegen performance (--!strict, --!native, --!optimize 2) with session-based caching, clean 2-function mutual recursion, concrete types, and local functions for inlining. Add session caching to both Rust implementations for consistency.
todos:
  - id: lua-trueEquals
    content: "Rewrite trueEquals.lua: --!strict/--!native/--!optimize 2, single typeof dispatch, dedicated helper functions per type (no temp arrays), adopt DeepEqualsPureUnsafe pattern for table comparison (rawlen/rawget/next, no checkedKeys set), keep fuzzy equality + null-ref + EnumItem cross-type"
    status: completed
  - id: lua-rewrite
    content: "Rewrite matching.lua: --!strict/--!native/--!optimize 2, all functions as local function in dependency order (one forward declare for mutual recursion), 2-function mutual recursion (matchChildren + computePairCost), session caching, totalCost return, O(n) removeMatched, remove pcall in grouping"
    status: completed
  - id: lua-hydrate-threading
    content: Thread session through hydrate.lua, Reconciler/init.lua, and ServeSession.lua (create session before hydrate call at line 628)
    status: completed
  - id: lua-verify-codegen
    content: Run luau-compile --remarks on trueEquals.lua and matching.lua to verify inlining and native codegen quality, iterate until clean
    status: completed
  - id: lua-test
    content: Build plugin, test Lua changes in Studio (hydrate + matching works correctly with session caching) before proceeding to Rust
    status: completed
  - id: rust-forward-session
    content: "AFTER Lua passes: Add MatchingSession to src/snapshot/matching.rs with cost_cache, add total_cost to ForwardMatchResult, thread session through all functions and patch_compute.rs"
    status: completed
  - id: rust-syncback-session
    content: "AFTER Lua passes: Add MatchingSession to src/syncback/matching.rs with cost_cache, add total_cost to MatchResult, thread session through all functions and dir.rs"
    status: completed
  - id: rust-test-updates
    content: Update tests/tests/matching_fixtures.rs, run cargo test to verify Rust changes
    status: completed
isProject: false
---

# Matching Session Caching + Native Codegen Rewrite

## Problem

1. **Redundant computation**: Scoring recurses into children, then hydrate re-calls matchChildren on the same children. The intermediate `_computeChildrenCost` calls matchChildren, iterates matched pairs calling `_computeChangeCount`, which recurses again -- all discarded after scoring.
2. **Poor native codegen structure**: All functions are on the `Matching` table (table lookup per call, uninlineable). No type annotations. No `--!strict`/`--!native`/`--!optimize 2`. A `pcall` with closure allocation in the hot grouping loop.
3. **4-function recursion chain** (`matchChildren -> _computeChildrenCost -> _computeChangeCount -> _computeChildrenCost -> matchChildren`) is complex and hard for both the optimizer and the developer to reason about.

## Design

### Conceptual Model

`matchChildren` is fundamentally `computeSubtreeCost` -- it finds the cost-minimizing pairing of children and returns the total cost. The actual match assignments are a bonus output. The 2-function mutual recursion reflects this:

- `**matchChildren(session, vChildren, sChildren, virtualInstances, parentVId, parentSInst)`**: Given two child sets, find optimal pairing. Returns `MatchResult` with `totalCost`.
- `**computePairCost(session, virtualId, studioInstance, virtualInstances, bestSoFar)`**: Given one virtual and one studio instance, compute total cost including subtree. Calls `matchChildren` for children cost.

`_computeChildrenCost` and `_computeChangeCount` are eliminated. Their logic is absorbed into these two functions.

### MatchingSession

```lua
type MatchingSession = {
    matchCache: { [string]: { [Instance]: MatchResult } },
    costCache: { [string]: { [Instance]: number } },
}
```

- **matchCache**: keyed by `(parentVirtualId, parentStudioInstance)`. Parent uniquely determines children set.
- **costCache**: keyed by `(virtualId, studioInstance)`. Only stored when `cost < bestSoFar` (exact, not early-exited).

Caching replaces the need for a depth limit: each unique pair/children-set is computed at most once.

### bestSoFar Interaction

- **costCache**: only store when computation was complete (`cost < bestSoFar`). If cached, return immediately regardless of caller's `bestSoFar` (the cached value IS the exact cost).
- **matchCache**: `matchChildren` always completes (no `bestSoFar` on it), so results are always cacheable.

---

## Lua Plugin Rewrite: [matching.lua](plugin/src/Reconciler/matching.lua)

### Module Header

```lua
--!strict
--!native
--!optimize 2
```

### Type Definitions (all concrete, minimal `any`)

```lua
type MatchPair = {
    virtualId: string,
    studioInstance: Instance,
}

type MatchResult = {
    matched: { MatchPair },
    unmatchedVirtual: { string },
    unmatchedStudio: { Instance },
    totalCost: number,
}

type MatchingSession = {
    matchCache: { [string]: { [Instance]: MatchResult } },
    costCache: { [string]: { [Instance]: number } },
}

type VCache = {
    props: { [string]: any },
    extraProps: { string }?,
    tags: { [string]: boolean }?,
    attrs: { [string]: any }?,
    childCount: number,
}

type SCache = {
    instance: Instance,
    props: { [string]: any },
    tags: { [string]: boolean }?,
    attrs: { [string]: any }?,
    children: { Instance },
    childCount: number,
}
```

### All Functions as `local function` in Dependency Order

Every function is a `local function` defined in dependency order. No `Matching.foo()` table lookups. One forward declaration for the mutual recursion pair.

```lua
local UNMATCHED_PENALTY: number = 10000

local function newSession(): MatchingSession
    return { matchCache = {}, costCache = {} }
end

local function cacheVirtual(vInst: any, classKeys: any): VCache
    -- same logic as current _cacheVirtual, with return type annotation
end

local function cacheStudio(studioInstance: Instance, classKeys: any, extraProps: { string }?): SCache
    -- same logic as current _cacheStudio
    -- ADDITION: store sCache.instance = studioInstance
end

local function countOwnDiffs(vCache: VCache, sCache: SCache, classKeys: any): number
    -- same logic as current _countOwnDiffsCached
end

local function removeMatched(arr: { any }, matchedIndices: { [number]: boolean }): ()
    -- O(n) in-place compaction instead of O(n^2) table.remove loop
    local write = 1
    for read = 1, #arr do
        if not matchedIndices[read] then
            arr[write] = arr[read]
            write += 1
        end
    end
    for i = write, #arr do
        arr[i] = nil
    end
end
```

### Single Forward Declaration for Mutual Recursion

`computePairCost` calls `matchChildren` and vice versa. Define `matchChildren` first (forward-declared), then `computePairCost` as a `local function`, then assign `matchChildren`:

```lua
local matchChildren  -- forward declare (matchChildren is defined after computePairCost)

local function computePairCost(...)
    -- can reference matchChildren (captured as upvalue, assigned before first call)
end

matchChildren = function(...)
    -- can reference computePairCost (already defined above)
end
```

### computePairCost (replaces `_computeChangeCount` + `_computeChildrenCost`)

```lua
computePairCost = function(
    session: MatchingSession,
    virtualId: string,
    studioInstance: Instance,
    virtualInstances: { [string]: any },
    bestSoFar: number
): number
    -- 1. Cache lookup
    local vc = session.costCache[virtualId]
    if vc then
        local cached = vc[studioInstance]
        if cached ~= nil then return cached end
    end

    local vInst = virtualInstances[virtualId]
    if not vInst then return UNMATCHED_PENALTY end

    -- 2. Own diffs
    local classKeys = RbxDom.getClassComparisonKeys(vInst.ClassName)
    local vCache = cacheVirtual(vInst, classKeys)
    local sCache = cacheStudio(studioInstance, classKeys, vCache.extraProps)
    local cost: number = countOwnDiffs(vCache, sCache, classKeys)
    if cost >= bestSoFar then return cost end

    -- 3. Children cost (direct matchChildren call, no intermediate function)
    local vChildren = vInst.Children
    local studioKids = sCache.children

    if (not vChildren or #vChildren == 0) and #studioKids == 0 then
        -- leaf: no children on either side
    elseif not vChildren or #vChildren == 0 then
        cost += #studioKids * UNMATCHED_PENALTY
    elseif #studioKids == 0 then
        for _, childId in vChildren do
            if virtualInstances[childId] then cost += UNMATCHED_PENALTY end
        end
    else
        local validVChildren: { string } = {}
        for _, childId in vChildren do
            if virtualInstances[childId] then
                table.insert(validVChildren, childId)
            end
        end
        local childResult = matchChildren(
            session, validVChildren, studioKids, virtualInstances,
            virtualId, studioInstance
        )
        cost += childResult.totalCost
    end

    -- 4. Cache if exact (no early exit)
    if cost < bestSoFar then
        if not session.costCache[virtualId] then
            session.costCache[virtualId] = {}
        end
        session.costCache[virtualId][studioInstance] = cost
    end

    return cost
end
```

### matchChildren (restructured, returns totalCost)

```lua
matchChildren = function(
    session: MatchingSession,
    virtualChildren: { string },
    studioChildren: { Instance },
    virtualInstances: { [string]: any },
    parentVirtualId: string?,
    parentStudioInstance: Instance?
): MatchResult
    -- 1. Cache lookup
    if parentVirtualId and parentStudioInstance then
        local pc = session.matchCache[parentVirtualId]
        if pc then
            local cached = pc[parentStudioInstance]
            if cached then return cached end
        end
    end

    local matched: { MatchPair } = {}
    local remainingVirtual: { string } = table.clone(virtualChildren)
    local remainingStudio: { Instance } = table.clone(studioChildren)

    -- 2. Group by (Name, ClassName) -- NO pcall, direct property access
    local vByKey: { [string]: { number } } = {}
    for i, id in remainingVirtual do
        local vInst = virtualInstances[id]
        if vInst then
            local key = vInst.Name .. "\0" .. vInst.ClassName
            local group = vByKey[key]
            if not group then
                group = {}
                vByKey[key] = group
            end
            table.insert(group, i)
        end
    end

    local sByKey: { [string]: { number } } = {}
    for i, inst in remainingStudio do
        local key = inst.Name .. "\0" .. inst.ClassName
        local group = sByKey[key]
        if not group then
            group = {}
            sByKey[key] = group
        end
        table.insert(group, i)
    end

    -- 3. 1:1 instant match
    local matchedV: { [number]: boolean } = {}
    local matchedS: { [number]: boolean } = {}

    for key, vIndices in vByKey do
        local sIndices = sByKey[key]
        if sIndices and #vIndices == 1 and #sIndices == 1 then
            local vi, si = vIndices[1], sIndices[1]
            if not matchedV[vi] and not matchedS[si] then
                table.insert(matched, {
                    virtualId = remainingVirtual[vi],
                    studioInstance = remainingStudio[si],
                })
                matchedV[vi] = true
                matchedS[si] = true
            end
        end
    end

    removeMatched(remainingVirtual, matchedV)
    removeMatched(remainingStudio, matchedS)

    -- 4. Ambiguous groups (same structure as before but using computePairCost)
    if #remainingVirtual > 0 and #remainingStudio > 0 then
        -- Rebuild groups, pre-compute caches, N*M scoring...
        -- Score via: ownDiffs (from pre-computed caches) + childrenCost
        -- For childrenCost: call computePairCost (which calls matchChildren recursively)
        -- BUT: the ambiguous loop uses pre-computed caches for ownDiffs directly
        -- (cacheVirtual/cacheStudio/countOwnDiffs), then adds children cost via
        -- a direct matchChildren call (same as computePairCost but with caches already built)
        --
        -- The inner scoring for an ambiguous (vi, si) pair:
        --   cost = countOwnDiffs(vCaches[vi], sCaches[si], classKeys)
        --   if cost < bestSoFar then
        --       local vInst = virtualInstances[remainingVirtual[vi]]
        --       local studioKids = sCaches[si].children
        --       -- filter valid virtual children, call matchChildren, add totalCost
        --   end
        --
        -- Greedy assign sorted by cost ascending (tie-break by insertion index)
    end

    -- 5. Compute totalCost for ALL matched pairs (uses session cache)
    local totalCost: number = 0
    for _, pair in matched do
        totalCost += computePairCost(
            session, pair.virtualId, pair.studioInstance, virtualInstances, math.huge
        )
    end
    totalCost += (#remainingVirtual + #remainingStudio) * UNMATCHED_PENALTY

    local result: MatchResult = {
        matched = matched,
        unmatchedVirtual = remainingVirtual,
        unmatchedStudio = remainingStudio,
        totalCost = totalCost,
    }

    -- 6. Cache result
    if parentVirtualId and parentStudioInstance then
        if not session.matchCache[parentVirtualId] then
            session.matchCache[parentVirtualId] = {}
        end
        session.matchCache[parentVirtualId][parentStudioInstance] = result
    end

    return result
end
```

### pcall Removal in Studio Grouping

The current code wraps `inst.Name, inst.ClassName` in a `pcall(function() ... end)`. This allocates a closure per studio child and prevents inlining. During matching, studio children come from `:GetChildren()` on a live instance within a single-threaded sync frame -- they cannot be destroyed mid-operation. Replace with direct property access.

### O(n) removeMatched

Current `_removeMatched` uses `table.remove` in a reverse loop -- O(n^2) due to element shifting. Replace with single-pass in-place compaction (shown above).

### Module Export

```lua
return {
    newSession = newSession,
    matchChildren = matchChildren,
}
```

Only two symbols exported. All helpers are local (invisible outside module, inlineable by compiler).

---

## Lua Callers

### [hydrate.lua](plugin/src/Reconciler/hydrate.lua)

```lua
local Matching = require(script.Parent.matching)

local function hydrate(instanceMap, virtualInstances, rootId, rootInstance, session)
    -- ...
    local result = Matching.matchChildren(
        session, validVirtualIds, existingChildren, virtualInstances,
        rootId, rootInstance
    )
    for _, pair in result.matched do
        hydrate(instanceMap, virtualInstances, pair.virtualId, pair.studioInstance, session)
    end
    -- ...
end
```

### [Reconciler/init.lua](plugin/src/Reconciler/init.lua)

```lua
function Reconciler:hydrate(virtualInstances, rootId, rootInstance, session)
    Timer.start("Reconciler:hydrate")
    local result = hydrate(self.__instanceMap, virtualInstances, rootId, rootInstance, session)
    Timer.stop()
    return result
end
```

### [ServeSession.lua](plugin/src/ServeSession.lua)

Create session before hydrate at line 628:

```lua
local Matching = require(script.Parent.Reconciler.matching)
-- ...
local session = Matching.newSession()
self.__reconciler:hydrate(readResponseBody.instances, serverInfo.rootInstanceId, game, session)
```

---

## trueEquals Rewrite: [trueEquals.lua](plugin/src/Reconciler/trueEquals.lua)

### Module Header

```lua
--!strict
--!native
--!optimize 2
```

### Problems in Current Implementation

1. **Redundant type dispatch**: Every branch checks both `typeA` and `typeB` (`typeA == "CFrame" and typeB == "CFrame"`). Check type once, early-out if they differ.
2. **Heap allocations in hot path**: `{ a:GetComponents() }`, `{ a.X, a.Y, a.Z }`, `{ a.X, a.Y }` build temporary arrays for every CFrame/Vector3/Vector2 comparison.
3. `**checkedKeys` set**: Allocates a table for every table-vs-table comparison. Unnecessary -- second loop only needs to check for extra keys in `b`.
4. **No type annotations**, no `--!strict`, no `--!native`, no `--!optimize 2`.

### Structure: Single typeof Dispatch + Dedicated Helpers

Adopt the [DeepEqualsPureUnsafe](escape-tsunami reference) pattern: `rawequal` identity check, `rawlen` for quick array length, `rawget`/`next` for table traversal, no temporary allocations. Bake in fuzzy equality for the matching use case.

```lua
local EPSILON: number = 0.0001

local function fuzzyEq(a: number, b: number): boolean
    local diff = math.abs(a - b)
    local maxVal = math.max(math.abs(a), math.abs(b), 1)
    return diff < EPSILON or diff < maxVal * EPSILON
end

local function color3Eq(a: Color3, b: Color3): boolean
    return math.floor(a.R * 255) == math.floor(b.R * 255)
        and math.floor(a.G * 255) == math.floor(b.G * 255)
        and math.floor(a.B * 255) == math.floor(b.B * 255)
end

local function vector2Eq(a: Vector2, b: Vector2): boolean
    return fuzzyEq(a.X, b.X) and fuzzyEq(a.Y, b.Y)
end

local function vector3Eq(a: Vector3, b: Vector3): boolean
    return fuzzyEq(a.X, b.X) and fuzzyEq(a.Y, b.Y) and fuzzyEq(a.Z, b.Z)
end

local function cframeEq(a: CFrame, b: CFrame): boolean
    local ax, ay, az, aR00, aR01, aR02, aR10, aR11, aR12, aR20, aR21, aR22 = a:GetComponents()
    local bx, by, bz, bR00, bR01, bR02, bR10, bR11, bR12, bR20, bR21, bR22 = b:GetComponents()
    return fuzzyEq(ax, bx) and fuzzyEq(ay, by) and fuzzyEq(az, bz)
        and fuzzyEq(aR00, bR00) and fuzzyEq(aR01, bR01) and fuzzyEq(aR02, bR02)
        and fuzzyEq(aR10, bR10) and fuzzyEq(aR11, bR11) and fuzzyEq(aR12, bR12)
        and fuzzyEq(aR20, bR20) and fuzzyEq(aR21, bR21) and fuzzyEq(aR22, bR22)
end
```

All helpers are small `local function`s -- the compiler can inline them into `trueEquals`.

### Main Function: Single typeof Dispatch

```lua
local function trueEquals(a: any, b: any): boolean
    if rawequal(a, b) then return true end

    -- Null-ref equivalence: nil == {Ref = "000...0"}
    if a == nil then
        return type(b) == "table" and rawget(b, "Ref") == "00000000000000000000000000000000"
    end
    if b == nil then
        return type(a) == "table" and rawget(a, "Ref") == "00000000000000000000000000000000"
    end

    local t = typeof(a)
    if t ~= typeof(b) then
        -- EnumItem/number cross-type
        if t == "number" and typeof(b) == "EnumItem" then return a == (b :: EnumItem).Value end
        if t == "EnumItem" and typeof(b) == "number" then return (a :: EnumItem).Value == b end
        return false
    end

    -- Same-type dispatch (no redundant second type check)
    if t == "table" then
        -- DeepEqualsPureUnsafe pattern with fuzzy recursive equality
        if rawlen(a) ~= rawlen(b) then return false end
        for k, v in next, a do
            local ov = rawget(b, k)
            if ov == nil or not trueEquals(v, ov) then return false end
        end
        for k in next, b do
            if rawget(a, k) == nil then return false end
        end
        return true
    end

    if t == "number" then return fuzzyEq(a, b) end
    if t == "Color3" then return color3Eq(a, b) end
    if t == "Vector3" then return vector3Eq(a, b) end
    if t == "Vector2" then return vector2Eq(a, b) end
    if t == "CFrame" then return cframeEq(a, b) end

    -- NaN check (number case already handled above, this catches any remaining NaN-like values)
    if a ~= a and b ~= b then return true end

    return false
end
```

**Key improvements:**

- **Zero heap allocations**: No temporary arrays for CFrame/Vector3/Vector2. CFrame uses 12 direct `fuzzyEq` calls. Vector3/Vector2 use direct component access.
- **No `checkedKeys` set**: Second table loop only checks existence via `rawget`, doesn't compare values (already done in first loop).
- **Single `typeof` call path**: Check type of `a` once, verify `b` matches, then dispatch. No redundant `typeA == X and typeB == X` at every branch.
- `**rawequal` identity check first**: Catches same-reference case before any type checking.
- `**rawlen` fast-path for tables**: Different array lengths = not equal, no iteration needed.

---

## Codegen Verification

After each rewrite, run:

```bash
luau-compile --remarks plugin/src/Reconciler/trueEquals.lua
luau-compile --remarks plugin/src/Reconciler/matching.lua
```

**trueEquals.lua** -- check that:

- `fuzzyEq`, `color3Eq`, `vector2Eq`, `vector3Eq`, `cframeEq` are inlined into `trueEquals`
- No "can't inline" remarks on any helper

**matching.lua** -- check that:

- `countOwnDiffs`, `cacheVirtual`, `cacheStudio`, `removeMatched` are inlined at call sites
- `computePairCost` inlines or at minimum shows no optimization barriers
- No generic dispatch warnings for typed parameters

Iterate on any functions that fail to inline (split if too large, simplify control flow).

---

## Rust Forward Sync: [src/snapshot/matching.rs](src/snapshot/matching.rs)

Add session struct and thread through. Rust already has the clean 2-function structure (`match_forward` + `compute_change_count` / `match_children_for_scoring`).

```rust
pub struct MatchingSession {
    cost_cache: RefCell<HashMap<(Ref, Ref), u32>>,
}
```

- Add `total_cost: u32` to `ForwardMatchResult`
- Add `session: &MatchingSession` param to `match_forward`, `compute_change_count`, `match_children_for_scoring`
- Cache `compute_change_count` results by `(snap.snapshot_id, tree_ref)`, only when `cost < best_so_far`
- Create session in [patch_compute.rs](src/snapshot/patch_compute.rs) `compute_patch_set`, thread through `compute_patch_set_internal` and `compute_children_patches`

## Rust Syncback: [src/syncback/matching.rs](src/syncback/matching.rs)

Same pattern. Session struct, `total_cost` on `MatchResult`, session param on all functions.

- Create session in [dir.rs](src/snapshot_middleware/dir.rs) before `match_children` call

## Test Updates: [matching_fixtures.rs](tests/tests/matching_fixtures.rs)

Pass `&MatchingSession::new()` to `match_children` and `match_forward` calls.

---

## Summary of Optimizations


| Change                                                  | Impact                                                            |
| ------------------------------------------------------- | ----------------------------------------------------------------- |
| **trueEquals**: single typeof dispatch                  | Eliminates redundant type checks at every branch                  |
| **trueEquals**: dedicated type helpers                  | Zero heap allocations (no temp arrays for CFrame/Vector3/Vector2) |
| **trueEquals**: DeepEqualsPureUnsafe table pattern      | No checkedKeys set; rawlen/rawget/next                            |
| **trueEquals**: `--!strict`/`--!native`/`--!optimize 2` | Native codegen for the most-called function in matching           |
| **matching**: local functions (not table methods)       | Enables compiler inlining                                         |
| **matching**: `--!strict`/`--!native`/`--!optimize 2`   | Native codegen, type-guided optimization                          |
| **matching**: concrete types on all params/returns      | Specialized native code paths                                     |
| **matching**: remove pcall in grouping                  | Eliminates closure + pcall overhead per child                     |
| **matching**: 2-function recursion (from 4)             | Simpler for optimizer, easier to reason about                     |
| **matching**: session caching (matchCache + costCache)  | Eliminates redundant subtree scoring across recursion levels      |
| **matching**: totalCost on matchChildren                | Eliminates _computeChildrenCost re-iteration of matched pairs     |
| **matching**: O(n) removeMatched                        | Linear vs quadratic element removal                               |


