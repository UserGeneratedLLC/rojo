---
name: Rewrite matching algorithm
overview: "Replace the multi-pass matching algorithm in all 3 implementations with a recursive change-count scoring function and greedy assignment. Sort key is a tuple: (change_count, child_index_A, child_index_B). Fast-paths for Ref pins and (Name, ClassName) grouping."
todos:
  - id: rewrite-lua-matching
    content: "Rewrite Lua matching.lua: recursive computeChangeCount + greedy assign with Ref pin and (Name, ClassName) fast-paths"
    status: completed
  - id: rewrite-rust-forward
    content: "Rewrite Rust forward sync matching.rs: recursive count_changes + greedy assign using variant_eq"
    status: completed
  - id: rewrite-rust-syncback
    content: "Rewrite Rust syncback matching.rs: recursive count_changes + greedy assign with hash fast-path"
    status: completed
  - id: verify-tests
    content: Run cargo test to verify all 714 tests still pass
    status: completed
isProject: false
---

# Rewrite Matching Algorithm

Replace all 3 matching implementations with one recursive scoring function and greedy assignment.

## matchChildren (entry point, called per parent)

```
matchChildren(sideA, sideB, parentA, parentB) -> (matched, unmatchedA, unmatchedB):

    // Fast-path 1: Ref pin (confirmed identity, highest priority)
    // Scan parentA and parentB for Ref properties (PrimaryPart, Value, etc.)
    // that point to children. If parentA.PrimaryPart = childX on sideA and
    // parentB.PrimaryPart = childY on sideB, pin (X, Y) as matched.
    // Remove pinned children from sideA/sideB.
    for each Ref property name shared by parentA and parentB:
        targetA = parentA[refProp]
        targetB = parentB[refProp]
        if targetA in sideA and targetB in sideB:
            match (targetA, targetB)
            remove both from sideA/sideB

    // Fast-path 2: Group remaining by (Name, ClassName)
    // 1:1 groups get instant-matched with zero scoring.
    groups = group sideA and sideB by (Name, ClassName)
    for each group:
        if exactly 1 on each side: instant match, remove both

    // Ambiguous groups: score + greedy assign
    for each group with multiple on at least one side:
        pairs = []
        bestSoFar = infinity
        for each (A, B) in cross-product of group:   // iteration order = child order
            cost = computeChangeCount(A, B, bestSoFar)  // RECURSIVE, early-exits
            pairs.append((cost, A, B))
            if cost < bestSoFar: bestSoFar = cost

        stable sort pairs by cost ascending
        // Stable sort preserves insertion order on ties = original child order
        greedy assign: pick first unmatched, remove both, repeat

    // Leftovers = new/deleted
```

The two fast-paths resolve the vast majority of children with zero scoring. Only truly ambiguous groups (same Name, same ClassName, multiple candidates) ever call `computeChangeCount`.

## computeChangeCount (recursive scoring function)

One function. Returns the total number of changes the reconciler would need to make to turn A into B, **including the entire subtree**.

```
computeChangeCount(A, B, bestSoFar) -> number:
    // Cheap: all own diffs in one shot (flat loops, no branching)
    cost = countOwnDiffs(A, B)   // properties + tags + attributes
    if cost >= bestSoFar: return cost

    // Expensive: recursive children matching
    childResult = matchChildren(A.children, B.children, A, B)  // RECURSE
    for each (childA, childB) in childResult.matched:
        cost += computeChangeCount(childA, childB, bestSoFar - cost)
        if cost >= bestSoFar: return cost
    cost += count(childResult.unmatched) * UNMATCHED_PENALTY

    return cost

countOwnDiffs(A, B) -> number:
    cost = 0
    for each property key in union(A.properties, B.properties):
        if values differ or key missing on one side: cost += 1
    for each tag in symmetricDifference(A.Tags, B.Tags): cost += 1
    for each attr key in union(A.Attributes, B.Attributes):
        if missing or value differs: cost += 1
    return cost
```

`computeChangeCount` and `matchChildren` are **mutually recursive**. Base case: leaf nodes (no children).

Name and ClassName are NOT in the change count -- the (Name, ClassName) grouping in `matchChildren` already guarantees they match for any pair that reaches `computeChangeCount`.

## Unmatched Child Penalty

Each unmatched child (create or delete an entire instance) adds a **large constant** (`UNMATCHED_PENALTY`) to the parent pair's cost. No need to recursively calculate the subtree -- creating/deleting an instance is categorically more expensive than tweaking properties, so a flat constant correctly prioritizes pairings that avoid creates/deletes.

`UNMATCHED_PENALTY` should be large enough that any number of property diffs on matched children is still cheaper than one unmatched child. A value like `10,000` works (no real instance has 10K properties).

## Sort Key and Stability

Sort key is `change_count`, ascending. On ties, original child order is preserved.

- **Rust:** `slice::sort_by` is stable natively. No extra work needed.
- **Lua:** `table.sort` is NOT stable (Luau uses non-stable quicksort). Use a simple ad-hoc stable sort: pairs are built with their insertion index, and the Lua sort comparator breaks ties by index. Something like `if a.cost == b.cost then return a.idx < b.idx end; return a.cost < b.cost`.

## Known Limitation: Greedy vs Optimal

The greedy algorithm (pick lowest-cost pair first) is NOT globally optimal. Edge cases exist where a locally-best pick forces a globally-worse assignment. The Hungarian algorithm (O(N^3)) would give optimal results, but ambiguous groups are typically small (2-10 instances) and the greedy works correctly for the common case. Flagged for revisiting if real-world games hit this edge case.

## What Counts as a Change (+1 each)

- **Each differing property:** iterate union of property keys on both sides. Each key where values differ or one side missing = +1. Includes CFrame, Size, Color3, Source, Material, Ref properties (PrimaryPart, Value), everything. Tags and Attributes are excluded from this loop (counted separately below).
- **Tags:** count individual tag operations. Each tag that needs to be added or removed = +1. If A has `{"Foo", "Bar"}` and B has `{"Bar", "Baz"}`, cost = 2 (remove "Foo", add "Baz").
- **Attributes:** count individual attribute operations. Each attribute that needs to be added, removed, or changed = +1. If A has `{X=1, Y=2}` and B has `{Y=3, Z=4}`, cost = 3 (remove X, change Y, add Z).
- **Recursive children cost:** total from `matchChildren` (matched pair costs + unmatched subtree costs).

Things NOT in the change count:

- Name -- grouping guarantees match.
- ClassName -- grouping guarantees match.
- Parent -- structural.

## Files to Rewrite

### 1. Lua -- `[plugin/src/Reconciler/matching.lua](plugin/src/Reconciler/matching.lua)`

Complete rewrite. Three functions: `matchChildren`, `computeChangeCount`, `costOfEntireInstance`. Property comparison via pcall reads from Studio instances vs virtual `.Properties`. Ref pin scans parent's Ref properties. (Name, ClassName) grouping as fast-path.

### 2. Rust forward sync -- `[src/snapshot/matching.rs](src/snapshot/matching.rs)`

Complete rewrite. Same three functions. Property comparison using `variant_eq()` from `[src/variant_eq.rs](src/variant_eq.rs)`. Operates on `InstanceSnapshot` vs `RojoTree` instances.

### 3. Rust syncback -- `[src/syncback/matching.rs](src/syncback/matching.rs)`

Complete rewrite. Same three functions. Hash-match fast-path: if `new_hash == old_hash`, return 0 without checking properties. Otherwise `variant_eq()` per property. Operates on `WeakDom` instances.

## What Gets Deleted

- All multi-pass infrastructure (Pass 1/2/3, `_pass1NameAndClass`, `_pass3Similarity`, etc.)
- `compute_similarity` / `compute_forward_similarity` / `_computeSimilarity`
- `collect_ref_property_targets`, `pass2_ref_discriminators`
- Replaced by: `matchChildren`, `computeChangeCount`, `costOfEntireInstance`

## Correctness

- Ref-pinned pairs are pulled out first -- confirmed identity, never wrong
- (Name, ClassName) grouping prevents cross-class or cross-name pairing
- Within ambiguous groups, recursive change counting picks the pair requiring fewest total reconciler operations across the entire subtree
- Child order tiebreaker ensures determinism on equal scores
- Unmatched subtree cost correctly penalizes pairings that cause cascading creates/deletes

