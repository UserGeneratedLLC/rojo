//! 3-pass instance matching algorithm for forward sync.
//!
//! Pairs snapshot children (from filesystem re-snapshot) to tree children
//! (existing RojoTree instances). Same algorithm pattern as
//! `syncback::matching` but operating on different data types:
//!
//! - **Snapshot side**: `InstanceSnapshot` with name, class_name, properties
//! - **Tree side**: `RojoTree` instances with name(), class_name(), properties()

use std::collections::HashMap;

use rbx_dom_weak::types::Ref;

use super::{InstanceSnapshot, InstanceWithMeta, RojoTree};

/// Result of the forward sync matching algorithm.
pub struct ForwardMatchResult {
    /// Pairs of (snapshot_child, tree_ref) that were matched.
    pub matched: Vec<(InstanceSnapshot, Ref)>,
    /// Snapshot children with no match in the tree (to be added).
    pub unmatched_snapshot: Vec<InstanceSnapshot>,
    /// Tree children with no match in the snapshot (to be removed).
    pub unmatched_tree: Vec<Ref>,
}

/// Run the 3-pass matching algorithm for forward sync.
///
/// Takes ownership of `snapshot_children` since matched snapshots are moved
/// into the result.
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

    // Index snapshot children for later consumption.
    let mut snap_available: Vec<Option<InstanceSnapshot>> =
        snapshot_children.into_iter().map(Some).collect();
    // Track which indices are matched (separate from snap_available to avoid
    // consuming snapshots before build_result).
    let mut snap_matched: Vec<bool> = vec![false; snap_available.len()];
    let mut tree_available: Vec<bool> = vec![true; tree_children.len()];

    let mut matched: Vec<(usize, usize)> = Vec::new(); // (snap_idx, tree_idx)

    // ---- Pass 1: unique name matching + ClassName narrowing ----
    pass1_name_and_class(
        &snap_available,
        tree_children,
        &tree_available,
        tree,
        &mut matched,
    );

    // Mark matched items as unavailable for subsequent passes.
    for &(si, ti) in &matched {
        snap_matched[si] = true;
        tree_available[ti] = false;
    }

    // Fast path: if nothing remains on either side, skip Passes 2 and 3.
    let snap_remaining = snap_matched.iter().filter(|&&m| !m).count();
    let tree_remaining = tree_available.iter().filter(|&&a| a).count();
    if snap_remaining == 0 || tree_remaining == 0 {
        return build_result(snap_available, tree_children, &tree_available, matched);
    }

    // ---- Pass 2: Ref property discriminators (placeholder) ----
    // TODO: Implement ref-based matching using Rojo_Ref_* attributes on
    // snapshot side and resolved Refs on tree side.

    // ---- Pass 3: similarity scoring ----
    pass3_similarity(
        &snap_available,
        tree_children,
        &mut tree_available,
        tree,
        &mut matched,
    );

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

    let unmatched_snapshot: Vec<InstanceSnapshot> =
        snap_available.into_iter().flatten().collect();

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

/// Pass 1: Match instances with unique names, then narrow by ClassName.
fn pass1_name_and_class(
    snap_available: &[Option<InstanceSnapshot>],
    tree_children: &[Ref],
    tree_available: &[bool],
    tree: &RojoTree,
    matched: &mut Vec<(usize, usize)>,
) {
    // Build name→indices maps for both sides.
    let mut snap_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, snap_opt) in snap_available.iter().enumerate() {
        if let Some(snap) = snap_opt {
            snap_by_name.entry(&snap.name).or_default().push(i);
        }
    }

    let mut tree_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if !tree_available[i] {
            continue;
        }
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_name
                .entry(inst.name().to_string())
                .or_default()
                .push(i);
        }
    }

    for (name, snap_indices) in &snap_by_name {
        let Some(tree_indices) = tree_by_name.get(*name) else {
            continue;
        };

        // Case 1: exactly one on each side → instant match.
        if snap_indices.len() == 1 && tree_indices.len() == 1 {
            matched.push((snap_indices[0], tree_indices[0]));
            continue;
        }

        // Case 2: try ClassName narrowing.
        let mut snap_by_class: HashMap<&str, Vec<usize>> = HashMap::new();
        for &si in snap_indices {
            if let Some(snap) = &snap_available[si] {
                snap_by_class.entry(&snap.class_name).or_default().push(si);
            }
        }

        let mut tree_by_class: HashMap<String, Vec<usize>> = HashMap::new();
        for &ti in tree_indices {
            if let Some(inst) = tree.get_instance(tree_children[ti]) {
                tree_by_class
                    .entry(inst.class_name().to_string())
                    .or_default()
                    .push(ti);
            }
        }

        for (class, snap_class_indices) in &snap_by_class {
            if let Some(tree_class_indices) = tree_by_class.get(*class) {
                if snap_class_indices.len() == 1 && tree_class_indices.len() == 1 {
                    matched.push((snap_class_indices[0], tree_class_indices[0]));
                }
            }
        }
    }
}

/// Pass 3: Pairwise similarity scoring within same-name groups.
fn pass3_similarity(
    snap_available: &[Option<InstanceSnapshot>],
    tree_children: &[Ref],
    tree_available: &mut [bool],
    tree: &RojoTree,
    matched: &mut Vec<(usize, usize)>,
) {
    // Track which snap indices have been matched in this pass.
    let mut snap_matched: Vec<bool> = vec![false; snap_available.len()];

    // Group remaining by name.
    let mut snap_by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, snap_opt) in snap_available.iter().enumerate() {
        if let Some(snap) = snap_opt {
            snap_by_name.entry(&snap.name).or_default().push(i);
        }
    }

    let mut tree_by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, &child_ref) in tree_children.iter().enumerate() {
        if !tree_available[i] {
            continue;
        }
        if let Some(inst) = tree.get_instance(child_ref) {
            tree_by_name
                .entry(inst.name().to_string())
                .or_default()
                .push(i);
        }
    }

    for (name, snap_indices) in &snap_by_name {
        let Some(tree_indices) = tree_by_name.get(*name) else {
            continue;
        };

        // Build pairs with similarity scores.
        let mut pairs: Vec<(usize, usize, u32)> = Vec::new();
        for &si in snap_indices {
            if snap_matched[si] {
                continue;
            }
            let snap = match &snap_available[si] {
                Some(s) => s,
                None => continue,
            };
            for &ti in tree_indices {
                if !tree_available[ti] {
                    continue;
                }
                let inst = match tree.get_instance(tree_children[ti]) {
                    Some(i) => i,
                    None => continue,
                };
                let score = compute_forward_similarity(snap, &inst);
                pairs.push((si, ti, score));
            }
        }

        // Sort descending by score, tiebreak by original order.
        pairs.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0).then(a.1.cmp(&b.1))));

        // Greedy assignment.
        for (si, ti, _score) in pairs {
            if snap_matched[si] || !tree_available[ti] {
                continue;
            }
            matched.push((si, ti));
            snap_matched[si] = true;
            tree_available[ti] = false;
        }
    }
}

/// Compute similarity between a snapshot child and a tree child.
fn compute_forward_similarity(snap: &InstanceSnapshot, inst: &InstanceWithMeta) -> u32 {
    let mut score: u32 = 0;

    if snap.class_name.as_str() == inst.class_name().as_str() {
        score += 100;
    }

    // Children count similarity.
    if snap.children.len() == inst.children().len() {
        score += 20;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{InstanceMetadata, InstanceSnapshot, RojoTree};
    use rbx_dom_weak::ustr;
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
        let snaps = vec![make_snapshot("Alpha", "Folder"), make_snapshot("Beta", "Script")];
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
        let snaps = vec![make_snapshot("A", "Folder"), make_snapshot("NewOnly", "Folder")];
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
}
