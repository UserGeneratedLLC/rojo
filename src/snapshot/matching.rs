//! Instance matching algorithm for forward sync.
//!
//! Pairs snapshot children (from filesystem re-snapshot) to tree children
//! (existing RojoTree instances) by minimizing total reconciler changes.
//!
//! Algorithm per parent:
//!   1. Group by (Name, ClassName) -- 1:1 groups instant-match
//!   2. Ambiguous groups: recursive change-count scoring + greedy assignment
//!
//! The change count = how many things the reconciler would need to touch
//! to turn instance A into instance B, including the entire subtree.

use std::collections::HashMap;

use rbx_dom_weak::types::Ref;

use crate::variant_eq::variant_eq;

use super::{InstanceSnapshot, InstanceWithMeta, RojoTree};

const UNMATCHED_PENALTY: u32 = 10_000;

/// Maximum recursion depth for `compute_change_count`. Beyond this depth,
/// only flat property comparison is used (no subtree recursion). Prevents
/// O(n^k) explosion on deeply nested trees with many same-named instances.
const MAX_SCORING_DEPTH: u32 = 3;

/// Result of the forward sync matching algorithm.
pub struct ForwardMatchResult {
    /// Pairs of (snapshot_child, tree_ref) that were matched.
    pub matched: Vec<(InstanceSnapshot, Ref)>,
    /// Snapshot children with no match in the tree (to be added).
    pub unmatched_snapshot: Vec<InstanceSnapshot>,
    /// Tree children with no match in the snapshot (to be removed).
    pub unmatched_tree: Vec<Ref>,
}

/// Match snapshot children to tree children, minimizing total changes.
pub fn match_forward(
    snapshot_children: Vec<InstanceSnapshot>,
    tree_children: &[Ref],
    tree: &RojoTree,
) -> ForwardMatchResult {
    if snapshot_children.is_empty() && tree_children.is_empty() {
        return ForwardMatchResult {
            matched: Vec::new(),
            unmatched_snapshot: Vec::new(),
            unmatched_tree: Vec::new(),
        };
    }

    let snap_available: Vec<Option<InstanceSnapshot>> =
        snapshot_children.into_iter().map(Some).collect();
    let mut snap_matched: Vec<bool> = vec![false; snap_available.len()];
    let mut tree_available: Vec<bool> = vec![true; tree_children.len()];
    let mut matched: Vec<(usize, usize)> = Vec::new();

    // ================================================================
    // Fast-path: Group by (Name, ClassName) -- 1:1 groups instant-match
    // ================================================================
    let mut snap_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, snap_opt) in snap_available.iter().enumerate() {
        if let Some(snap) = snap_opt {
            snap_by_key
                .entry((snap.name.to_string(), snap.class_name.to_string()))
                .or_default()
                .push(i);
        }
    }

    let mut tree_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_key
                .entry((inst.name().to_string(), inst.class_name().to_string()))
                .or_default()
                .push(i);
        }
    }

    // 1:1 groups: instant match
    for (key, snap_indices) in &snap_by_key {
        if let Some(tree_indices) = tree_by_key.get(key) {
            if snap_indices.len() == 1 && tree_indices.len() == 1 {
                let si = snap_indices[0];
                let ti = tree_indices[0];
                matched.push((si, ti));
                snap_matched[si] = true;
                tree_available[ti] = false;
            }
        }
    }

    // Early exit if nothing remains
    let snap_remaining = snap_matched.iter().filter(|&&m| !m).count();
    let tree_remaining = tree_available.iter().filter(|&&a| a).count();
    if snap_remaining == 0 || tree_remaining == 0 {
        return build_result(snap_available, tree_children, &tree_available, matched);
    }

    // ================================================================
    // Ambiguous groups: change-count scoring + greedy assignment
    // ================================================================
    for (key, snap_indices) in &snap_by_key {
        let Some(tree_indices) = tree_by_key.get(key) else {
            continue;
        };

        // Collect unmatched indices in this group
        let avail_snap: Vec<usize> = snap_indices
            .iter()
            .filter(|&&si| !snap_matched[si])
            .copied()
            .collect();
        let avail_tree: Vec<usize> = tree_indices
            .iter()
            .filter(|&&ti| tree_available[ti])
            .copied()
            .collect();

        if avail_snap.is_empty() || avail_tree.is_empty() {
            continue;
        }
        // 1:1 already handled above; skip if not truly ambiguous
        if avail_snap.len() <= 1 && avail_tree.len() <= 1 {
            if avail_snap.len() == 1 && avail_tree.len() == 1 {
                let si = avail_snap[0];
                let ti = avail_tree[0];
                matched.push((si, ti));
                snap_matched[si] = true;
                tree_available[ti] = false;
            }
            continue;
        }

        // Score all (A, B) pairs using recursive change-count scoring
        let mut pairs: Vec<(u32, usize, usize)> = Vec::new();
        let mut best_so_far = u32::MAX;
        for &si in &avail_snap {
            let snap = match &snap_available[si] {
                Some(s) => s,
                None => continue,
            };
            for &ti in &avail_tree {
                let cost = compute_change_count(snap, tree_children[ti], tree, best_so_far, 0);
                pairs.push((cost, si, ti));
                if cost < best_so_far {
                    best_so_far = cost;
                }
            }
        }

        // Stable sort by cost ascending (slice::sort_by is stable in Rust)
        pairs.sort_by_key(|&(cost, _, _)| cost);

        // Greedy assign
        for &(_, si, ti) in &pairs {
            if snap_matched[si] || !tree_available[ti] {
                continue;
            }
            matched.push((si, ti));
            snap_matched[si] = true;
            tree_available[ti] = false;
        }
    }

    build_result(snap_available, tree_children, &tree_available, matched)
}

/// Build the final result, consuming the snapshot children.
fn build_result(
    mut snap_available: Vec<Option<InstanceSnapshot>>,
    tree_children: &[Ref],
    tree_available: &[bool],
    matched_indices: Vec<(usize, usize)>,
) -> ForwardMatchResult {
    let mut matched = Vec::with_capacity(matched_indices.len());
    for (si, ti) in matched_indices {
        if let Some(snap) = snap_available[si].take() {
            matched.push((snap, tree_children[ti]));
        }
    }

    let unmatched_snapshot: Vec<InstanceSnapshot> = snap_available.into_iter().flatten().collect();

    let unmatched_tree: Vec<Ref> = tree_children
        .iter()
        .enumerate()
        .filter(|(i, _)| tree_available[*i])
        .map(|(_, r)| *r)
        .collect();

    ForwardMatchResult {
        matched,
        unmatched_snapshot,
        unmatched_tree,
    }
}

/// Lightweight matching result used during recursive scoring.
/// Returns index pairs (snap_index, tree_child_index) without consuming
/// the snapshots.
struct ScoringMatchResult {
    matched: Vec<(usize, usize)>,
    unmatched_snap: usize,
    unmatched_tree: usize,
}

/// Match children by reference for scoring purposes (non-consuming).
/// Groups by (Name, ClassName), instant-matches 1:1 groups, and scores
/// ambiguous groups using `compute_change_count` (mutually recursive).
fn match_children_for_scoring(
    snap_children: &[InstanceSnapshot],
    tree_children: &[Ref],
    tree: &RojoTree,
    depth: u32,
) -> ScoringMatchResult {
    let mut snap_matched = vec![false; snap_children.len()];
    let mut tree_matched = vec![false; tree_children.len()];
    let mut matched = Vec::new();

    if snap_children.is_empty() && tree_children.is_empty() {
        return ScoringMatchResult {
            matched,
            unmatched_snap: 0,
            unmatched_tree: 0,
        };
    }

    // Group by (Name, ClassName)
    let mut snap_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, snap) in snap_children.iter().enumerate() {
        snap_by_key
            .entry((snap.name.to_string(), snap.class_name.to_string()))
            .or_default()
            .push(i);
    }

    let mut tree_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_key
                .entry((inst.name().to_string(), inst.class_name().to_string()))
                .or_default()
                .push(i);
        }
    }

    // 1:1 groups: instant match
    for (key, snap_indices) in &snap_by_key {
        if let Some(tree_indices) = tree_by_key.get(key) {
            if snap_indices.len() == 1 && tree_indices.len() == 1 {
                let si = snap_indices[0];
                let ti = tree_indices[0];
                matched.push((si, ti));
                snap_matched[si] = true;
                tree_matched[ti] = true;
            }
        }
    }

    // Ambiguous groups: score + greedy assign
    for (key, snap_indices) in &snap_by_key {
        let Some(tree_indices) = tree_by_key.get(key) else {
            continue;
        };

        let avail_snap: Vec<usize> = snap_indices
            .iter()
            .filter(|&&si| !snap_matched[si])
            .copied()
            .collect();
        let avail_tree: Vec<usize> = tree_indices
            .iter()
            .filter(|&&ti| !tree_matched[ti])
            .copied()
            .collect();

        if avail_snap.is_empty() || avail_tree.is_empty() {
            continue;
        }
        if avail_snap.len() == 1 && avail_tree.len() == 1 {
            let si = avail_snap[0];
            let ti = avail_tree[0];
            matched.push((si, ti));
            snap_matched[si] = true;
            tree_matched[ti] = true;
            continue;
        }

        let mut pairs: Vec<(u32, usize, usize)> = Vec::new();
        let mut best_so_far = u32::MAX;
        for &si in &avail_snap {
            for &ti in &avail_tree {
                let cost = compute_change_count(
                    &snap_children[si],
                    tree_children[ti],
                    tree,
                    best_so_far,
                    depth,
                );
                pairs.push((cost, si, ti));
                if cost < best_so_far {
                    best_so_far = cost;
                }
            }
        }

        pairs.sort_by_key(|&(cost, _, _)| cost);

        for &(_, si, ti) in &pairs {
            if snap_matched[si] || tree_matched[ti] {
                continue;
            }
            matched.push((si, ti));
            snap_matched[si] = true;
            tree_matched[ti] = true;
        }
    }

    let unmatched_snap = snap_matched.iter().filter(|&&m| !m).count();
    let unmatched_tree = tree_matched.iter().filter(|&&m| !m).count();

    ScoringMatchResult {
        matched,
        unmatched_snap,
        unmatched_tree,
    }
}

/// Compute total change count between a snapshot and a tree instance,
/// including recursive subtree scoring. Returns the number of reconciler
/// operations needed to turn the tree instance into the snapshot.
///
/// Mutually recursive with `match_children_for_scoring`: this function
/// scores a single pair, while `match_children_for_scoring` groups and
/// assigns children pairs (calling this function for ambiguous scoring).
fn compute_change_count(
    snap: &InstanceSnapshot,
    tree_ref: Ref,
    tree: &RojoTree,
    best_so_far: u32,
    depth: u32,
) -> u32 {
    let inst = match tree.get_instance(tree_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };

    let mut cost = count_own_diffs(snap, &inst);
    if cost >= best_so_far || depth >= MAX_SCORING_DEPTH {
        return cost;
    }

    let snap_children = &snap.children;
    let tree_children = inst.children();

    if snap_children.is_empty() && tree_children.is_empty() {
        return cost;
    }

    let scoring = match_children_for_scoring(snap_children, tree_children, tree, depth + 1);

    for &(si, ti) in &scoring.matched {
        let remaining = best_so_far.saturating_sub(cost);
        cost += compute_change_count(
            &snap_children[si],
            tree_children[ti],
            tree,
            remaining,
            depth + 1,
        );
        if cost >= best_so_far {
            return cost;
        }
    }

    cost = cost.saturating_add(
        (scoring.unmatched_snap + scoring.unmatched_tree) as u32 * UNMATCHED_PENALTY,
    );

    cost
}

/// Count own property diffs between a snapshot and a tree instance.
/// Each differing property = +1. Tags and Attributes counted granularly.
/// Children count diff = +1.
fn count_own_diffs(snap: &InstanceSnapshot, inst: &InstanceWithMeta) -> u32 {
    let mut cost: u32 = 0;

    let snap_props = &snap.properties;
    let inst_props = inst.properties();

    // Properties present on the snapshot side
    for (key, snap_val) in snap_props.iter() {
        if let Some(inst_val) = inst_props.get(key) {
            if !variant_eq(snap_val, inst_val) {
                cost += 1;
            }
        } else {
            cost += 1; // Missing on tree side
        }
    }

    // Properties present only on the tree side
    for key in inst_props.keys() {
        if !snap_props.contains_key(key) {
            cost += 1;
        }
    }

    // Children count diff
    if snap.children.len() != inst.children().len() {
        cost += 1;
    }

    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{InstanceMetadata, InstanceSnapshot, RojoTree};
    use rbx_dom_weak::{ustr, HashMapExt as _};
    use std::borrow::Cow;

    fn make_snapshot(name: &str, class: &str) -> InstanceSnapshot {
        InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Owned(name.to_string()),
            class_name: ustr(class),
            properties: Default::default(),
            children: Vec::new(),
        }
    }

    fn make_tree_with_children(children: &[(&str, &str)]) -> (RojoTree, Vec<Ref>) {
        let child_snapshots: Vec<InstanceSnapshot> = children
            .iter()
            .map(|(name, class)| make_snapshot(name, class))
            .collect();
        let root_snap = InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Borrowed("DataModel"),
            class_name: ustr("DataModel"),
            properties: Default::default(),
            children: child_snapshots,
        };
        let tree = RojoTree::new(root_snap);
        let root_id = tree.get_root_id();
        let child_ids: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();
        (tree, child_ids)
    }

    #[test]
    fn unique_names_match() {
        let snaps = vec![
            make_snapshot("Alpha", "Folder"),
            make_snapshot("Beta", "Script"),
        ];
        let (tree, children) = make_tree_with_children(&[("Alpha", "Folder"), ("Beta", "Script")]);

        let result = match_forward(snaps, &children, &tree);
        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());
    }

    #[test]
    fn class_narrowing() {
        let snaps = vec![
            make_snapshot("Handler", "Script"),
            make_snapshot("Handler", "ModuleScript"),
        ];
        let (tree, children) =
            make_tree_with_children(&[("Handler", "ModuleScript"), ("Handler", "Script")]);

        let result = match_forward(snaps, &children, &tree);
        assert_eq!(result.matched.len(), 2);

        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            assert_eq!(snap.class_name.as_str(), inst.class_name().as_str());
        }
    }

    #[test]
    fn unmatched_both_sides() {
        let snaps = vec![
            make_snapshot("A", "Folder"),
            make_snapshot("NewOnly", "Folder"),
        ];
        let (tree, children) = make_tree_with_children(&[("A", "Folder"), ("OldOnly", "Folder")]);

        let result = match_forward(snaps, &children, &tree);
        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.unmatched_snapshot.len(), 1);
        assert_eq!(result.unmatched_tree.len(), 1);
        assert_eq!(result.unmatched_snapshot[0].name.as_ref(), "NewOnly");
    }

    #[test]
    fn duplicate_names_greedy() {
        let snaps = vec![
            make_snapshot("Data", "Folder"),
            make_snapshot("Data", "Folder"),
        ];
        let (tree, children) = make_tree_with_children(&[("Data", "Folder"), ("Data", "Folder")]);

        let result = match_forward(snaps, &children, &tree);
        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());
    }

    #[test]
    fn empty_both() {
        let snaps: Vec<InstanceSnapshot> = vec![];
        let (tree, children) = make_tree_with_children(&[]);

        let result = match_forward(snaps, &children, &tree);
        assert!(result.matched.is_empty());
    }

    fn make_snapshot_with_props(
        name: &str,
        class: &str,
        props: Vec<(&str, rbx_dom_weak::types::Variant)>,
    ) -> InstanceSnapshot {
        let mut properties = rbx_dom_weak::UstrMap::new();
        for (key, val) in props {
            properties.insert(ustr(key), val);
        }
        InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Owned(name.to_string()),
            class_name: ustr(class),
            properties,
            children: Vec::new(),
        }
    }

    #[test]
    fn many_same_name_parts_matched_by_properties() {
        // Simulate a Ladder model with 10 "Line" Parts, each with a different
        // Anchored value (as a simple distinguishing property).
        // The matching should pair each to its correct counterpart.
        use rbx_dom_weak::types::Variant;

        let count = 10;
        let snaps: Vec<InstanceSnapshot> = (0..count)
            .map(|i| {
                make_snapshot_with_props(
                    "Line",
                    "Part",
                    vec![("Transparency", Variant::Float32(i as f32 * 0.1))],
                )
            })
            .collect();

        // Build tree with same parts but in reversed order -- the matching
        // should still pair them correctly by property content, not position.
        let child_snapshots: Vec<InstanceSnapshot> = (0..count)
            .rev()
            .map(|i| {
                make_snapshot_with_props(
                    "Line",
                    "Part",
                    vec![("Transparency", Variant::Float32(i as f32 * 0.1))],
                )
            })
            .collect();
        let root_snap = InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Borrowed("DataModel"),
            class_name: ustr("DataModel"),
            properties: Default::default(),
            children: child_snapshots,
        };
        let tree = RojoTree::new(root_snap);
        let root_id = tree.get_root_id();
        let tree_children: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();

        let result = match_forward(snaps, &tree_children, &tree);

        assert_eq!(result.matched.len(), count);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());

        // Verify each pair has matching Transparency values (correct pairing)
        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            let snap_val = snap.properties.get(&ustr("Transparency"));
            let tree_val = inst.properties().get(&ustr("Transparency"));
            assert_eq!(
                snap_val, tree_val,
                "Mismatched Transparency: snap has {:?}, tree has {:?}",
                snap_val, tree_val
            );
        }
    }

    #[test]
    fn fifteen_same_name_identical_parts() {
        // 15 Parts all named "Line" with identical properties.
        // All pairs score the same -- greedy picks by stable child order.
        // Should match all 15 without panicking or cross-matching.
        let count = 15;
        let snaps: Vec<InstanceSnapshot> =
            (0..count).map(|_| make_snapshot("Line", "Part")).collect();
        let (tree, children) =
            make_tree_with_children(&(0..count).map(|_| ("Line", "Part")).collect::<Vec<_>>());

        let result = match_forward(snaps, &children, &tree);
        assert_eq!(result.matched.len(), count);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());
    }

    fn make_snapshot_with_children(
        name: &str,
        class: &str,
        children: Vec<InstanceSnapshot>,
    ) -> InstanceSnapshot {
        InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Owned(name.to_string()),
            class_name: ustr(class),
            properties: Default::default(),
            children,
        }
    }

    #[test]
    fn recursive_matching_distinguishes_by_children() {
        // Two Folders both named "Data" with identical top-level properties
        // but different children. The recursive scoring should pair each
        // Folder with the tree Folder that has matching children.
        use rbx_dom_weak::types::Variant;

        let snap_a = make_snapshot_with_children(
            "Data",
            "Folder",
            vec![make_snapshot_with_props(
                "Child",
                "Part",
                vec![("Transparency", Variant::Float32(0.0))],
            )],
        );
        let snap_b = make_snapshot_with_children(
            "Data",
            "Folder",
            vec![make_snapshot_with_props(
                "Child",
                "Part",
                vec![("Transparency", Variant::Float32(1.0))],
            )],
        );
        let snaps = vec![snap_a, snap_b];

        // Build tree with the two Folders in REVERSED order
        let tree_child_a = make_snapshot_with_children(
            "Data",
            "Folder",
            vec![make_snapshot_with_props(
                "Child",
                "Part",
                vec![("Transparency", Variant::Float32(1.0))],
            )],
        );
        let tree_child_b = make_snapshot_with_children(
            "Data",
            "Folder",
            vec![make_snapshot_with_props(
                "Child",
                "Part",
                vec![("Transparency", Variant::Float32(0.0))],
            )],
        );
        let root_snap = InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Borrowed("DataModel"),
            class_name: ustr("DataModel"),
            properties: Default::default(),
            children: vec![tree_child_a, tree_child_b],
        };
        let tree = RojoTree::new(root_snap);
        let root_id = tree.get_root_id();
        let tree_children: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();

        let result = match_forward(snaps, &tree_children, &tree);

        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());

        // Verify recursive matching paired by children content, not position.
        // snap[0] has child Transparency=0.0, should match tree[1] (also 0.0).
        // snap[1] has child Transparency=1.0, should match tree[0] (also 1.0).
        for (snap, tree_ref) in &result.matched {
            let tree_inst = tree.get_instance(*tree_ref).unwrap();
            let tree_child_ref = tree_inst.children()[0];
            let tree_child = tree.get_instance(tree_child_ref).unwrap();

            let snap_child = &snap.children[0];
            let snap_val = snap_child.properties.get(&ustr("Transparency"));
            let tree_val = tree_child.properties().get(&ustr("Transparency"));
            assert_eq!(
                snap_val, tree_val,
                "Recursive matching failed: snap child Transparency={:?}, tree child Transparency={:?}",
                snap_val, tree_val
            );
        }
    }

    #[test]
    fn depth_limit_completes_quickly() {
        // Deeply nested tree (depth=6) with same-named instances at each level.
        // Should complete without hanging thanks to the depth limit.
        fn make_deep_snap(depth: u32) -> InstanceSnapshot {
            if depth == 0 {
                return make_snapshot("Leaf", "Part");
            }
            make_snapshot_with_children(
                "Node",
                "Folder",
                vec![make_deep_snap(depth - 1), make_deep_snap(depth - 1)],
            )
        }

        let snaps = vec![make_deep_snap(6), make_deep_snap(6)];

        let tree_children_snaps = vec![make_deep_snap(6), make_deep_snap(6)];
        let root_snap = InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Borrowed("DataModel"),
            class_name: ustr("DataModel"),
            properties: Default::default(),
            children: tree_children_snaps,
        };
        let tree = RojoTree::new(root_snap);
        let root_id = tree.get_root_id();
        let tree_children: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();

        let start = std::time::Instant::now();
        let result = match_forward(snaps, &tree_children, &tree);
        let elapsed = start.elapsed();

        assert_eq!(result.matched.len(), 2);
        assert!(
            elapsed.as_secs() < 5,
            "Depth-limited matching took too long: {:?}",
            elapsed
        );
    }
}
