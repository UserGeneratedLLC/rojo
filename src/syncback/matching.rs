//! Instance matching algorithm for syncback.
//!
//! Pairs new children (from Roblox file) to old children (from filesystem)
//! by minimizing total reconciler changes.
//!
//! Algorithm per parent:
//!   1. Group by (Name, ClassName) -- 1:1 groups instant-match
//!   2. Ambiguous groups: change-count scoring + greedy assignment
//!      (with hash fast-path: hash equal = 0 cost)
//!
//! The change count = how many things the reconciler would need to touch
//! to turn instance A into instance B.

use std::collections::{HashMap, HashSet};

use blake3::Hash;
use rbx_dom_weak::{types::Ref, WeakDom};

use crate::variant_eq::variant_eq;

const UNMATCHED_PENALTY: u32 = 10_000;

/// Result of the matching algorithm.
#[derive(Debug)]
pub struct MatchResult {
    pub matched: Vec<(Ref, Ref)>,
    pub unmatched_new: Vec<Ref>,
    pub unmatched_old: Vec<Ref>,
}

/// Match new children to old children, minimizing total changes.
pub fn match_children(
    new_children: &[Ref],
    old_children: &[Ref],
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
) -> MatchResult {
    if new_children.is_empty() && old_children.is_empty() {
        return MatchResult {
            matched: Vec::new(),
            unmatched_new: Vec::new(),
            unmatched_old: Vec::new(),
        };
    }

    let mut matched: Vec<(Ref, Ref)> = Vec::new();
    let mut remaining_new: Vec<Ref> = new_children.to_vec();
    let mut remaining_old: Vec<Ref> = old_children.to_vec();

    // ================================================================
    // Fast-path: Group by (Name, ClassName) -- 1:1 groups instant-match
    // ================================================================
    let mut new_by_key: HashMap<(String, String), Vec<Ref>> = HashMap::new();
    for &r in &remaining_new {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_key
                .entry((inst.name.clone(), inst.class.to_string()))
                .or_default()
                .push(r);
        }
    }

    let mut old_by_key: HashMap<(String, String), Vec<Ref>> = HashMap::new();
    for &r in &remaining_old {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_key
                .entry((inst.name.clone(), inst.class.to_string()))
                .or_default()
                .push(r);
        }
    }

    let mut matched_new: HashSet<Ref> = HashSet::new();
    let mut matched_old: HashSet<Ref> = HashSet::new();

    // 1:1 groups: instant match
    // Sort keys for deterministic iteration
    let mut sorted_keys: Vec<_> = new_by_key.keys().cloned().collect();
    sorted_keys.sort();

    for key in &sorted_keys {
        let Some(new_refs) = new_by_key.get(key) else {
            continue;
        };
        let Some(old_refs) = old_by_key.get(key) else {
            continue;
        };
        if new_refs.len() == 1 && old_refs.len() == 1 {
            matched.push((new_refs[0], old_refs[0]));
            matched_new.insert(new_refs[0]);
            matched_old.insert(old_refs[0]);
        }
    }

    remaining_new.retain(|r| !matched_new.contains(r));
    remaining_old.retain(|r| !matched_old.contains(r));

    if remaining_new.is_empty() || remaining_old.is_empty() {
        return MatchResult {
            matched,
            unmatched_new: remaining_new,
            unmatched_old: remaining_old,
        };
    }

    // ================================================================
    // Ambiguous groups: change-count scoring + greedy assignment
    // ================================================================
    for key in &sorted_keys {
        let Some(new_refs) = new_by_key.get(key) else {
            continue;
        };
        let Some(old_refs) = old_by_key.get(key) else {
            continue;
        };

        let avail_new: Vec<Ref> = new_refs
            .iter()
            .filter(|r| !matched_new.contains(r))
            .copied()
            .collect();
        let avail_old: Vec<Ref> = old_refs
            .iter()
            .filter(|r| !matched_old.contains(r))
            .copied()
            .collect();

        if avail_new.is_empty() || avail_old.is_empty() {
            continue;
        }

        // Handle remaining 1:1 (from groups that had multiple but some were
        // already matched, leaving 1 on each side)
        if avail_new.len() == 1 && avail_old.len() == 1 {
            matched.push((avail_new[0], avail_old[0]));
            matched_new.insert(avail_new[0]);
            matched_old.insert(avail_old[0]);
            continue;
        }

        // Score all (A, B) pairs
        let mut pairs: Vec<(u32, Ref, Ref)> = Vec::new();
        let mut best_so_far = u32::MAX;

        for &new_ref in &avail_new {
            for &old_ref in &avail_old {
                let cost =
                    count_own_diffs(new_ref, old_ref, new_dom, old_dom, new_hashes, old_hashes);
                pairs.push((cost, new_ref, old_ref));
                if cost < best_so_far {
                    best_so_far = cost;
                }
            }
        }

        // Stable sort by cost ascending
        pairs.sort_by_key(|&(cost, _, _)| cost);

        // Greedy assign
        for &(_, new_ref, old_ref) in &pairs {
            if matched_new.contains(&new_ref) || matched_old.contains(&old_ref) {
                continue;
            }
            matched.push((new_ref, old_ref));
            matched_new.insert(new_ref);
            matched_old.insert(old_ref);
        }
    }

    remaining_new.retain(|r| !matched_new.contains(r));
    remaining_old.retain(|r| !matched_old.contains(r));

    MatchResult {
        matched,
        unmatched_new: remaining_new,
        unmatched_old: remaining_old,
    }
}

/// Count own property diffs between two WeakDom instances.
/// Hash fast-path: if hashes match, return 0 (identical, no property check needed).
/// Each differing property = +1. Children count diff = +1.
fn count_own_diffs(
    new_ref: Ref,
    old_ref: Ref,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
) -> u32 {
    // Hash fast-path: identical subtree = 0 cost
    if let (Some(nh), Some(oh)) = (new_hashes, old_hashes) {
        if let (Some(new_hash), Some(old_hash)) = (nh.get(&new_ref), oh.get(&old_ref)) {
            if new_hash == old_hash {
                return 0;
            }
        }
    }

    let new_inst = match new_dom.get_by_ref(new_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };
    let old_inst = match old_dom.get_by_ref(old_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };

    let mut cost: u32 = 0;

    // Properties present on new side
    for (key, new_val) in new_inst.properties.iter() {
        if let Some(old_val) = old_inst.properties.get(key) {
            if !variant_eq(new_val, old_val) {
                cost += 1;
            }
        } else {
            cost += 1;
        }
    }

    // Properties present only on old side
    for key in old_inst.properties.keys() {
        if !new_inst.properties.contains_key(key) {
            cost += 1;
        }
    }

    // Children count diff
    if new_inst.children().len() != old_inst.children().len() {
        cost += 1;
    }

    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    fn build_test_dom() -> (WeakDom, Ref, Ref, Ref) {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let root = dom.root_ref();
        let child_a = dom.insert(root, InstanceBuilder::new("Folder").with_name("Alpha"));
        let child_b = dom.insert(root, InstanceBuilder::new("Script").with_name("Beta"));
        (dom, root, child_a, child_b)
    }

    #[test]
    fn unique_names_match_instantly() {
        let (new_dom, new_root, _, _) = build_test_dom();
        let (old_dom, old_root, _, _) = build_test_dom();

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());

        for (new_ref, old_ref) in &result.matched {
            let new_name = &new_dom.get_by_ref(*new_ref).unwrap().name;
            let old_name = &old_dom.get_by_ref(*old_ref).unwrap().name;
            assert_eq!(new_name, old_name);
        }
    }

    #[test]
    fn class_name_grouping() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Script").with_name("Handler"),
        );
        new_dom.insert(
            new_root,
            InstanceBuilder::new("ModuleScript").with_name("Handler"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("ModuleScript").with_name("Handler"),
        );
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Script").with_name("Handler"),
        );

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 2);
        for (new_ref, old_ref) in &result.matched {
            let new_class = &new_dom.get_by_ref(*new_ref).unwrap().class;
            let old_class = &old_dom.get_by_ref(*old_ref).unwrap().class;
            assert_eq!(new_class, old_class);
        }
    }

    #[test]
    fn unmatched_new_and_old() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("Alpha"));
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("NewOnly"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("Alpha"));
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("OldOnly"),
        );

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.unmatched_new.len(), 1);
        assert_eq!(result.unmatched_old.len(), 1);
    }

    #[test]
    fn duplicate_names_matched() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("Data"));
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("Data"));

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("Data"));
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("Data"));

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);
        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());
    }

    #[test]
    fn empty_children() {
        let new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let result = match_children(&[], &[], &new_dom, &old_dom, None, None);
        assert!(result.matched.is_empty());
    }

    #[test]
    fn matching_stability() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("X"));
        new_dom.insert(new_root, InstanceBuilder::new("Script").with_name("Y"));

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("X"));
        old_dom.insert(old_root, InstanceBuilder::new("Script").with_name("Y"));

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let r1 = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);
        let r2 = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(r1.matched.len(), r2.matched.len());
        for (a, b) in r1.matched.iter().zip(r2.matched.iter()) {
            assert_eq!(a, b);
        }
    }
}
