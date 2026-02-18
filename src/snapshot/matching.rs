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

use std::cell::RefCell;
use std::collections::HashMap;

use rbx_dom_weak::types::Ref;
use rbx_dom_weak::Ustr;

use crate::variant_eq::variant_eq;

use super::{InstanceSnapshot, InstanceWithMeta, RojoTree};

const UNMATCHED_PENALTY: u32 = 10_000;

/// Maximum recursion depth for `compute_change_count`. Beyond this depth,
/// only flat property comparison is used (no subtree recursion). Prevents
/// O(n^k) explosion on deeply nested trees with many same-named instances.
const MAX_SCORING_DEPTH: u32 = 3;

/// Session-scoped cache for the matching algorithm. Caches
/// `compute_change_count` results so that recursive scoring work is
/// reused across calls at different tree levels.
pub struct MatchingSession {
    cost_cache: RefCell<HashMap<(Ref, Ref), u32>>,
}

impl Default for MatchingSession {
    fn default() -> Self {
        Self::new()
    }
}

impl MatchingSession {
    pub fn new() -> Self {
        Self {
            cost_cache: RefCell::new(HashMap::new()),
        }
    }
}

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
    session: &MatchingSession,
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
    let snap_len = snap_available.len();
    let tree_len = tree_children.len();
    let mut snap_matched: Vec<bool> = vec![false; snap_len];
    let mut tree_available: Vec<bool> = vec![true; tree_len];
    let mut matched: Vec<(usize, usize)> = Vec::with_capacity(snap_len.min(tree_len));

    // ================================================================
    // Fast-path: Group by (Name, ClassName) -- 1:1 groups instant-match
    // ================================================================
    let mut snap_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(snap_len);
    for (i, snap_opt) in snap_available.iter().enumerate() {
        if let Some(snap) = snap_opt {
            snap_by_key
                .entry((snap.name.to_string(), snap.class_name))
                .or_default()
                .push(i);
        }
    }

    let mut tree_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(tree_len);
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_key
                .entry((inst.name().to_string(), inst.class_name()))
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
        let mut pairs: Vec<(u32, usize, usize)> =
            Vec::with_capacity(avail_snap.len() * avail_tree.len());
        let mut best_so_far = u32::MAX;
        for &si in &avail_snap {
            let snap = match &snap_available[si] {
                Some(s) => s,
                None => continue,
            };
            for &ti in &avail_tree {
                let cost =
                    compute_change_count(snap, tree_children[ti], tree, best_so_far, 0, session);
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
    session: &MatchingSession,
) -> ScoringMatchResult {
    if snap_children.is_empty() && tree_children.is_empty() {
        return ScoringMatchResult {
            matched: Vec::new(),
            unmatched_snap: 0,
            unmatched_tree: 0,
        };
    }

    let snap_len = snap_children.len();
    let tree_len = tree_children.len();
    let mut snap_matched = vec![false; snap_len];
    let mut tree_matched = vec![false; tree_len];
    let mut matched: Vec<(usize, usize)> = Vec::with_capacity(snap_len.min(tree_len));

    // Group by (Name, ClassName)
    let mut snap_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(snap_len);
    for (i, snap) in snap_children.iter().enumerate() {
        snap_by_key
            .entry((snap.name.to_string(), snap.class_name))
            .or_default()
            .push(i);
    }

    let mut tree_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(tree_len);
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_key
                .entry((inst.name().to_string(), inst.class_name()))
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

        let mut pairs: Vec<(u32, usize, usize)> =
            Vec::with_capacity(avail_snap.len() * avail_tree.len());
        let mut best_so_far = u32::MAX;
        for &si in &avail_snap {
            for &ti in &avail_tree {
                let cost = compute_change_count(
                    &snap_children[si],
                    tree_children[ti],
                    tree,
                    best_so_far,
                    depth,
                    session,
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
    session: &MatchingSession,
) -> u32 {
    let cacheable = snap.snapshot_id.is_some() && tree_ref.is_some();
    if cacheable {
        let cache_key = (snap.snapshot_id, tree_ref);
        if let Some(&cached) = session.cost_cache.borrow().get(&cache_key) {
            return cached;
        }
    }

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
        if cacheable && cost < best_so_far {
            session
                .cost_cache
                .borrow_mut()
                .insert((snap.snapshot_id, tree_ref), cost);
        }
        return cost;
    }

    let scoring =
        match_children_for_scoring(snap_children, tree_children, tree, depth + 1, session);

    for &(si, ti) in &scoring.matched {
        let remaining = best_so_far.saturating_sub(cost);
        cost += compute_change_count(
            &snap_children[si],
            tree_children[ti],
            tree,
            remaining,
            depth + 1,
            session,
        );
        if cost >= best_so_far {
            return cost;
        }
    }

    cost = cost.saturating_add(
        (scoring.unmatched_snap + scoring.unmatched_tree) as u32 * UNMATCHED_PENALTY,
    );

    if cacheable && cost < best_so_far {
        session
            .cost_cache
            .borrow_mut()
            .insert((snap.snapshot_id, tree_ref), cost);
    }

    cost
}

/// Count own property diffs between a snapshot and a tree instance.
/// Each differing property = +1. Tags and Attributes counted granularly.
/// Children count diff = +1.
///
/// Syncback strips default-valued properties from model/meta files, so
/// filesystem snapshots may omit properties that the tree (populated from
/// Studio via two-way sync) has at their class defaults. To avoid inflating
/// match costs with phantom diffs, properties present on only one side are
/// skipped when their value matches the class default.
#[inline]
fn count_own_diffs(snap: &InstanceSnapshot, inst: &InstanceWithMeta) -> u32 {
    let mut cost: u32 = 0;

    let snap_props = &snap.properties;
    let inst_props = inst.properties();

    let class_data = rbx_reflection_database::get()
        .ok()
        .and_then(|db| db.classes.get(snap.class_name.as_str()));

    // Properties present on the snapshot side
    for (key, snap_val) in snap_props.iter() {
        if let Some(inst_val) = inst_props.get(key) {
            if !variant_eq(snap_val, inst_val) {
                cost += 1;
            }
        } else {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq(snap_val, default));
            if !is_default {
                cost += 1;
            }
        }
    }

    // Properties present only on the tree side
    for (key, inst_val) in inst_props.iter() {
        if !snap_props.contains_key(key) {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq(inst_val, default));
            if !is_default {
                cost += 1;
            }
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

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
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

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
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

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
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

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());
    }

    #[test]
    fn empty_both() {
        let snaps: Vec<InstanceSnapshot> = vec![];
        let (tree, children) = make_tree_with_children(&[]);

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
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

        let result = match_forward(snaps, &tree_children, &tree, &MatchingSession::new());

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

        let result = match_forward(snaps, &children, &tree, &MatchingSession::new());
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

        let result = match_forward(snaps, &tree_children, &tree, &MatchingSession::new());

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

    /// Helper: builds a RojoTree from child snapshots (with properties).
    fn make_tree_from_snapshots(children: Vec<InstanceSnapshot>) -> (RojoTree, Vec<Ref>) {
        let root_snap = InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: InstanceMetadata::default(),
            name: Cow::Borrowed("DataModel"),
            class_name: ustr("DataModel"),
            properties: Default::default(),
            children,
        };
        let tree = RojoTree::new(root_snap);
        let root_id = tree.get_root_id();
        let child_ids: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();
        (tree, child_ids)
    }

    // ================================================================
    // Default-omission matching tests
    //
    // These reproduce the scenario where syncback strips default-valued
    // properties from model files, creating an asymmetry between the
    // snapshot (sparse) and tree (full properties from Studio).
    //
    // The matching must still pair instances correctly despite some
    // snapshots missing properties that the tree has at their defaults.
    // ================================================================

    #[test]
    fn reflection_database_has_expected_defaults() {
        // Validates the reflection database has the defaults we rely on.
        use rbx_dom_weak::types::Variant;

        let db = rbx_reflection_database::get().expect("reflection database should load");
        let texture = db
            .classes
            .get("Texture")
            .expect("Texture class should exist");
        let face_default = texture
            .default_properties
            .get("Face")
            .expect("Texture should have Face default");
        match face_default {
            Variant::Enum(e) => assert_eq!(e.to_u32(), 5, "Face default should be Front (5)"),
            other => panic!("Face default should be Enum, got {:?}", other),
        }

        let part = db.classes.get("Part").expect("Part class should exist");
        let anchored_default = part
            .default_properties
            .get("Anchored")
            .expect("Part should have Anchored default");
        assert_eq!(
            anchored_default,
            &Variant::Bool(false),
            "Anchored default should be false"
        );
    }

    #[test]
    fn texture_face_six_instances_default_omitted() {
        // Exact reproduction of the Texture Face bug:
        // 6 Textures all named "Texture", each with a different Face.
        // Snapshot side: 5 have explicit Face, 1 omits Face (Front=default).
        // Tree side: all 6 have explicit Face (populated from Studio).
        //
        // Without the default-aware fix, the snapshot without Face scores
        // equally against all tree instances, steals one by iteration order,
        // and displaces the correct match.
        use rbx_dom_weak::types::{Enum, Variant};

        let faces = [
            ("Top", Enum::from_u32(1)),
            ("Right", Enum::from_u32(0)),
            ("Left", Enum::from_u32(3)),
            ("Bottom", Enum::from_u32(4)),
            ("Back", Enum::from_u32(2)),
        ];
        let face_front = Enum::from_u32(5);

        let shared_props = vec![("Transparency", Variant::Float32(0.65))];

        // Snapshots: 5 with explicit Face + 1 without (Front default)
        let mut snaps: Vec<InstanceSnapshot> = faces
            .iter()
            .map(|(_, face_enum)| {
                let mut props = shared_props.clone();
                props.push(("Face", Variant::Enum(*face_enum)));
                make_snapshot_with_props("Texture", "Texture", props)
            })
            .collect();
        // The Front one: NO Face property (default omitted by syncback)
        snaps.push(make_snapshot_with_props(
            "Texture",
            "Texture",
            shared_props.clone(),
        ));

        // Tree: all 6 with explicit Face (Studio always has all properties)
        let mut tree_children: Vec<InstanceSnapshot> = faces
            .iter()
            .map(|(_, face_enum)| {
                let mut props = shared_props.clone();
                props.push(("Face", Variant::Enum(*face_enum)));
                make_snapshot_with_props("Texture", "Texture", props)
            })
            .collect();
        tree_children.push(make_snapshot_with_props("Texture", "Texture", {
            let mut props = shared_props.clone();
            props.push(("Face", Variant::Enum(face_front)));
            props
        }));

        let (tree, tree_refs) = make_tree_from_snapshots(tree_children);
        let result = match_forward(snaps, &tree_refs, &tree, &MatchingSession::new());

        assert_eq!(result.matched.len(), 6);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());

        // Verify each pair has matching Face values.
        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            let snap_face = snap.properties.get(&ustr("Face"));
            let tree_face = inst.properties().get(&ustr("Face")).unwrap();

            match snap_face {
                Some(sv) => {
                    assert!(
                        variant_eq(sv, tree_face),
                        "Face mismatch: snap={:?}, tree={:?}",
                        sv,
                        tree_face
                    );
                }
                None => {
                    // Snapshot omitted Face = must match tree Face=Front (5)
                    assert!(
                        variant_eq(tree_face, &Variant::Enum(face_front)),
                        "Snapshot without Face should match Front, got {:?}",
                        tree_face
                    );
                }
            }
        }
    }

    #[test]
    fn texture_face_twelve_instances_two_groups() {
        // Exact reproduction of the user's real scenario:
        // 12 Textures split into two groups of 6 by TextureContent.
        // Each group covers all 6 faces, with Front omitted (default).
        // Tree has all 12 with explicit Face.
        use rbx_dom_weak::types::{Enum, Variant};

        let face_values: Vec<Enum> = vec![
            Enum::from_u32(1), // Top
            Enum::from_u32(0), // Right
            Enum::from_u32(3), // Left
            Enum::from_u32(4), // Bottom
            Enum::from_u32(2), // Back
        ];
        let face_front = Enum::from_u32(5);

        let mut snaps = Vec::new();
        let mut tree_children = Vec::new();

        for group_transparency in [0.6_f32, 0.65_f32] {
            // 5 with explicit Face
            for face in &face_values {
                snaps.push(make_snapshot_with_props(
                    "Texture",
                    "Texture",
                    vec![
                        ("Transparency", Variant::Float32(group_transparency)),
                        ("Face", Variant::Enum(*face)),
                    ],
                ));
                tree_children.push(make_snapshot_with_props(
                    "Texture",
                    "Texture",
                    vec![
                        ("Transparency", Variant::Float32(group_transparency)),
                        ("Face", Variant::Enum(*face)),
                    ],
                ));
            }
            // 1 without Face (Front default)
            snaps.push(make_snapshot_with_props(
                "Texture",
                "Texture",
                vec![("Transparency", Variant::Float32(group_transparency))],
            ));
            tree_children.push(make_snapshot_with_props(
                "Texture",
                "Texture",
                vec![
                    ("Transparency", Variant::Float32(group_transparency)),
                    ("Face", Variant::Enum(face_front)),
                ],
            ));
        }

        // Reverse tree order to make it harder
        tree_children.reverse();

        let (tree, tree_refs) = make_tree_from_snapshots(tree_children);
        let result = match_forward(snaps, &tree_refs, &tree, &MatchingSession::new());

        assert_eq!(result.matched.len(), 12);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());

        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            let snap_face = snap.properties.get(&ustr("Face"));
            let tree_face = inst.properties().get(&ustr("Face")).unwrap();

            // Transparency must match (distinguishes the two groups)
            let snap_t = snap.properties.get(&ustr("Transparency")).unwrap();
            let tree_t = inst.properties().get(&ustr("Transparency")).unwrap();
            assert!(
                variant_eq(snap_t, tree_t),
                "Group mismatch: snap Transparency={:?}, tree={:?}",
                snap_t,
                tree_t
            );

            match snap_face {
                Some(sv) => assert!(
                    variant_eq(sv, tree_face),
                    "Face mismatch: snap={:?}, tree={:?}",
                    sv,
                    tree_face
                ),
                None => assert!(
                    variant_eq(tree_face, &Variant::Enum(face_front)),
                    "Omitted Face should match Front, got {:?}",
                    tree_face
                ),
            }
        }
    }

    #[test]
    fn part_anchored_default_omitted() {
        // 4 Parts named "Block": 3 have Anchored=true (explicit),
        // 1 omits Anchored (false is the default). Tree has all 4.
        use rbx_dom_weak::types::Variant;

        let mut snaps = Vec::new();
        let mut tree_children = Vec::new();

        for i in 0..3 {
            let props = vec![
                ("Transparency", Variant::Float32(i as f32 * 0.1)),
                ("Anchored", Variant::Bool(true)),
            ];
            snaps.push(make_snapshot_with_props("Block", "Part", props.clone()));
            tree_children.push(make_snapshot_with_props("Block", "Part", props));
        }
        // The one with default Anchored=false: snapshot omits it
        snaps.push(make_snapshot_with_props(
            "Block",
            "Part",
            vec![("Transparency", Variant::Float32(0.3))],
        ));
        tree_children.push(make_snapshot_with_props(
            "Block",
            "Part",
            vec![
                ("Transparency", Variant::Float32(0.3)),
                ("Anchored", Variant::Bool(false)),
            ],
        ));

        tree_children.reverse();

        let (tree, tree_refs) = make_tree_from_snapshots(tree_children);
        let result = match_forward(snaps, &tree_refs, &tree, &MatchingSession::new());

        assert_eq!(result.matched.len(), 4);

        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            let snap_t = snap.properties.get(&ustr("Transparency")).unwrap();
            let tree_t = inst.properties().get(&ustr("Transparency")).unwrap();
            assert!(
                variant_eq(snap_t, tree_t),
                "Transparency mismatch: snap={:?}, tree={:?}",
                snap_t,
                tree_t
            );
        }
    }

    #[test]
    fn multiple_defaults_omitted_all_at_once() {
        // Tree instances have MANY extra properties all at their class defaults.
        // Snapshots omit all of them. The match should not be thrown off.
        use rbx_dom_weak::types::Variant;

        // Two sparse snapshots differing only by Transparency
        let snap_a = make_snapshot_with_props(
            "Wall",
            "Part",
            vec![("Transparency", Variant::Float32(0.0))],
        );
        let snap_b = make_snapshot_with_props(
            "Wall",
            "Part",
            vec![("Transparency", Variant::Float32(1.0))],
        );

        // Tree has Transparency + many known Part defaults from Studio.
        // These are all at their default values, so count_own_diffs should
        // skip them (not inflate match cost).
        let extra_defaults: Vec<(&str, Variant)> = vec![
            ("Anchored", Variant::Bool(false)),
            ("CastShadow", Variant::Bool(true)),
            ("CanCollide", Variant::Bool(true)),
            ("CanTouch", Variant::Bool(true)),
            ("Locked", Variant::Bool(false)),
            ("Massless", Variant::Bool(false)),
        ];

        let mut tree_props_a = vec![("Transparency", Variant::Float32(0.0))];
        tree_props_a.extend(extra_defaults.iter().cloned());
        let mut tree_props_b = vec![("Transparency", Variant::Float32(1.0))];
        tree_props_b.extend(extra_defaults.iter().cloned());

        // Tree in reversed order
        let (tree, tree_refs) = make_tree_from_snapshots(vec![
            make_snapshot_with_props("Wall", "Part", tree_props_b),
            make_snapshot_with_props("Wall", "Part", tree_props_a),
        ]);
        let result = match_forward(
            vec![snap_a, snap_b],
            &tree_refs,
            &tree,
            &MatchingSession::new(),
        );

        assert_eq!(result.matched.len(), 2, "Both should match");

        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();
            let snap_t = snap.properties.get(&ustr("Transparency")).unwrap();
            let tree_t = inst.properties().get(&ustr("Transparency")).unwrap();
            assert!(
                variant_eq(snap_t, tree_t),
                "Part with 6 extra default props mismatched: snap={:?}, tree={:?}",
                snap_t,
                tree_t
            );
        }
    }

    #[test]
    fn ten_ambiguous_textures_stress() {
        // 10 Textures: 8 with unique Face values (some repeated across
        // groups), 2 with Front omitted. Two distinguishing groups by
        // Transparency. The matcher must handle this without mis-pairing.
        use rbx_dom_weak::types::{Enum, Variant};

        let faces_group_a = [
            Some(Enum::from_u32(1)), // Top
            Some(Enum::from_u32(0)), // Right
            Some(Enum::from_u32(3)), // Left
            None,                    // Front (default, omitted)
            Some(Enum::from_u32(4)), // Bottom
        ];
        let faces_group_b = [
            Some(Enum::from_u32(2)), // Back
            Some(Enum::from_u32(4)), // Bottom
            None,                    // Front (default, omitted)
            Some(Enum::from_u32(1)), // Top
            Some(Enum::from_u32(3)), // Left
        ];
        let face_front = Enum::from_u32(5);

        let mut snaps = Vec::new();
        let mut tree_children = Vec::new();

        for (group_t, faces) in [(0.5_f32, &faces_group_a[..]), (0.8_f32, &faces_group_b[..])] {
            for face_opt in faces {
                let mut snap_props = vec![("Transparency", Variant::Float32(group_t))];
                let mut tree_props = vec![("Transparency", Variant::Float32(group_t))];

                if let Some(face) = face_opt {
                    snap_props.push(("Face", Variant::Enum(*face)));
                    tree_props.push(("Face", Variant::Enum(*face)));
                } else {
                    // Snapshot omits Face; tree has Front
                    tree_props.push(("Face", Variant::Enum(face_front)));
                }

                snaps.push(make_snapshot_with_props("Texture", "Texture", snap_props));
                tree_children.push(make_snapshot_with_props("Texture", "Texture", tree_props));
            }
        }

        // Shuffle tree order (reverse)
        tree_children.reverse();

        let (tree, tree_refs) = make_tree_from_snapshots(tree_children);
        let result = match_forward(snaps, &tree_refs, &tree, &MatchingSession::new());

        assert_eq!(result.matched.len(), 10);
        assert!(result.unmatched_snapshot.is_empty());
        assert!(result.unmatched_tree.is_empty());

        for (snap, tree_ref) in &result.matched {
            let inst = tree.get_instance(*tree_ref).unwrap();

            // Transparency must match (group discriminator)
            let snap_t = snap.properties.get(&ustr("Transparency")).unwrap();
            let tree_t = inst.properties().get(&ustr("Transparency")).unwrap();
            assert!(
                variant_eq(snap_t, tree_t),
                "Group mismatch: snap={:?}, tree={:?}",
                snap_t,
                tree_t
            );

            // Face must match
            let snap_face = snap.properties.get(&ustr("Face"));
            let tree_face = inst.properties().get(&ustr("Face")).unwrap();
            match snap_face {
                Some(sv) => assert!(
                    variant_eq(sv, tree_face),
                    "Face mismatch: snap={:?}, tree={:?}",
                    sv,
                    tree_face
                ),
                None => assert!(
                    variant_eq(tree_face, &Variant::Enum(face_front)),
                    "Omitted Face should match Front, got {:?}",
                    tree_face
                ),
            }
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
        let result = match_forward(snaps, &tree_children, &tree, &MatchingSession::new());
        let elapsed = start.elapsed();

        assert_eq!(result.matched.len(), 2);
        assert!(
            elapsed.as_secs() < 5,
            "Depth-limited matching took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn session_cache_consistent() {
        let snaps1 = vec![make_snapshot("X", "Folder"), make_snapshot("Y", "Folder")];
        let snaps2 = vec![make_snapshot("X", "Folder"), make_snapshot("Y", "Folder")];
        let (tree, children) = make_tree_with_children(&[("X", "Folder"), ("Y", "Folder")]);
        let session = MatchingSession::new();

        let r1 = match_forward(snaps1, &children, &tree, &session);
        let r2 = match_forward(snaps2, &children, &tree, &session);

        assert_eq!(r1.matched.len(), r2.matched.len());
        assert_eq!(r1.unmatched_snapshot.len(), r2.unmatched_snapshot.len());
    }
}
