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

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use blake3::Hash;
use rbx_dom_weak::{types::Ref, WeakDom};

use crate::variant_eq::variant_eq;

const UNMATCHED_PENALTY: u32 = 10_000;

/// Maximum recursion depth for `compute_change_count`. Beyond this depth,
/// only flat property comparison is used (no subtree recursion).
const MAX_SCORING_DEPTH: u32 = 3;

/// Session-scoped cache for the syncback matching algorithm.
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

/// Result of the matching algorithm.
#[derive(Debug)]
pub struct MatchResult {
    pub matched: Vec<(Ref, Ref)>,
    pub unmatched_new: Vec<Ref>,
    pub unmatched_old: Vec<Ref>,
    pub total_cost: u32,
}

/// Match new children to old children, minimizing total changes.
pub fn match_children(
    new_children: &[Ref],
    old_children: &[Ref],
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
    session: &MatchingSession,
) -> MatchResult {
    if new_children.is_empty() && old_children.is_empty() {
        return MatchResult {
            matched: Vec::new(),
            unmatched_new: Vec::new(),
            unmatched_old: Vec::new(),
            total_cost: 0,
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
        let mut total_cost: u32 = 0;
        for &(new_ref, old_ref) in &matched {
            total_cost = total_cost.saturating_add(compute_change_count(
                new_ref,
                old_ref,
                new_dom,
                old_dom,
                new_hashes,
                old_hashes,
                u32::MAX,
                0,
                session,
            ));
        }
        total_cost = total_cost
            .saturating_add((remaining_new.len() + remaining_old.len()) as u32 * UNMATCHED_PENALTY);
        return MatchResult {
            matched,
            unmatched_new: remaining_new,
            unmatched_old: remaining_old,
            total_cost,
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

        // Score all (A, B) pairs using recursive change-count scoring
        let mut pairs: Vec<(u32, Ref, Ref)> = Vec::new();
        let mut best_so_far = u32::MAX;

        for &new_ref in &avail_new {
            for &old_ref in &avail_old {
                let cost = compute_change_count(
                    new_ref,
                    old_ref,
                    new_dom,
                    old_dom,
                    new_hashes,
                    old_hashes,
                    best_so_far,
                    0,
                    session,
                );
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

    let mut total_cost: u32 = 0;
    for &(new_ref, old_ref) in &matched {
        total_cost = total_cost.saturating_add(compute_change_count(
            new_ref,
            old_ref,
            new_dom,
            old_dom,
            new_hashes,
            old_hashes,
            u32::MAX,
            0,
            session,
        ));
    }
    total_cost = total_cost
        .saturating_add((remaining_new.len() + remaining_old.len()) as u32 * UNMATCHED_PENALTY);

    MatchResult {
        matched,
        unmatched_new: remaining_new,
        unmatched_old: remaining_old,
        total_cost,
    }
}

/// Lightweight matching result used during recursive scoring.
struct ScoringMatchResult {
    matched: Vec<(usize, usize)>,
    unmatched_new: usize,
    unmatched_old: usize,
}

/// Match children by reference for scoring purposes (non-consuming).
/// Groups by (Name, ClassName), instant-matches 1:1 groups, and scores
/// ambiguous groups using `compute_change_count` (mutually recursive).
fn match_children_for_scoring(
    new_children: &[Ref],
    old_children: &[Ref],
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
    depth: u32,
    session: &MatchingSession,
) -> ScoringMatchResult {
    let mut new_matched = vec![false; new_children.len()];
    let mut old_matched = vec![false; old_children.len()];
    let mut matched = Vec::new();

    if new_children.is_empty() && old_children.is_empty() {
        return ScoringMatchResult {
            matched,
            unmatched_new: 0,
            unmatched_old: 0,
        };
    }

    // Group by (Name, ClassName)
    let mut new_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, &r) in new_children.iter().enumerate() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_key
                .entry((inst.name.clone(), inst.class.to_string()))
                .or_default()
                .push(i);
        }
    }

    let mut old_by_key: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, &r) in old_children.iter().enumerate() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_key
                .entry((inst.name.clone(), inst.class.to_string()))
                .or_default()
                .push(i);
        }
    }

    // 1:1 groups: instant match
    for (key, new_indices) in &new_by_key {
        if let Some(old_indices) = old_by_key.get(key) {
            if new_indices.len() == 1 && old_indices.len() == 1 {
                let ni = new_indices[0];
                let oi = old_indices[0];
                matched.push((ni, oi));
                new_matched[ni] = true;
                old_matched[oi] = true;
            }
        }
    }

    // Ambiguous groups: score + greedy assign
    for (key, new_indices) in &new_by_key {
        let Some(old_indices) = old_by_key.get(key) else {
            continue;
        };

        let avail_new: Vec<usize> = new_indices
            .iter()
            .filter(|&&ni| !new_matched[ni])
            .copied()
            .collect();
        let avail_old: Vec<usize> = old_indices
            .iter()
            .filter(|&&oi| !old_matched[oi])
            .copied()
            .collect();

        if avail_new.is_empty() || avail_old.is_empty() {
            continue;
        }
        if avail_new.len() == 1 && avail_old.len() == 1 {
            let ni = avail_new[0];
            let oi = avail_old[0];
            matched.push((ni, oi));
            new_matched[ni] = true;
            old_matched[oi] = true;
            continue;
        }

        let mut pairs: Vec<(u32, usize, usize)> = Vec::new();
        let mut best_so_far = u32::MAX;
        for &ni in &avail_new {
            for &oi in &avail_old {
                let cost = compute_change_count(
                    new_children[ni],
                    old_children[oi],
                    new_dom,
                    old_dom,
                    new_hashes,
                    old_hashes,
                    best_so_far,
                    depth,
                    session,
                );
                pairs.push((cost, ni, oi));
                if cost < best_so_far {
                    best_so_far = cost;
                }
            }
        }

        pairs.sort_by_key(|&(cost, _, _)| cost);

        for &(_, ni, oi) in &pairs {
            if new_matched[ni] || old_matched[oi] {
                continue;
            }
            matched.push((ni, oi));
            new_matched[ni] = true;
            old_matched[oi] = true;
        }
    }

    let unmatched_new = new_matched.iter().filter(|&&m| !m).count();
    let unmatched_old = old_matched.iter().filter(|&&m| !m).count();

    ScoringMatchResult {
        matched,
        unmatched_new,
        unmatched_old,
    }
}

/// Compute total change count between two WeakDom instances, including
/// recursive subtree scoring. Hash fast-path: if hashes match, return 0.
///
/// Mutually recursive with `match_children_for_scoring`.
#[allow(clippy::too_many_arguments)]
fn compute_change_count(
    new_ref: Ref,
    old_ref: Ref,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
    best_so_far: u32,
    depth: u32,
    session: &MatchingSession,
) -> u32 {
    let cacheable = new_ref.is_some() && old_ref.is_some();
    if cacheable {
        if let Some(&cached) = session.cost_cache.borrow().get(&(new_ref, old_ref)) {
            return cached;
        }
    }

    // Hash fast-path: identical subtree = 0 cost
    if let (Some(nh), Some(oh)) = (new_hashes, old_hashes) {
        if let (Some(new_hash), Some(old_hash)) = (nh.get(&new_ref), oh.get(&old_ref)) {
            if new_hash == old_hash {
                if cacheable {
                    session
                        .cost_cache
                        .borrow_mut()
                        .insert((new_ref, old_ref), 0);
                }
                return 0;
            }
        }
    }

    let mut cost = count_own_diffs(new_ref, old_ref, new_dom, old_dom);
    if cost >= best_so_far || depth >= MAX_SCORING_DEPTH {
        return cost;
    }

    let new_inst = match new_dom.get_by_ref(new_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };
    let old_inst = match old_dom.get_by_ref(old_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };

    let new_children = new_inst.children();
    let old_children = old_inst.children();

    if new_children.is_empty() && old_children.is_empty() {
        if cacheable && cost < best_so_far {
            session
                .cost_cache
                .borrow_mut()
                .insert((new_ref, old_ref), cost);
        }
        return cost;
    }

    let scoring = match_children_for_scoring(
        new_children,
        old_children,
        new_dom,
        old_dom,
        new_hashes,
        old_hashes,
        depth + 1,
        session,
    );

    for &(ni, oi) in &scoring.matched {
        let remaining = best_so_far.saturating_sub(cost);
        cost += compute_change_count(
            new_children[ni],
            old_children[oi],
            new_dom,
            old_dom,
            new_hashes,
            old_hashes,
            remaining,
            depth + 1,
            session,
        );
        if cost >= best_so_far {
            return cost;
        }
    }

    cost = cost
        .saturating_add((scoring.unmatched_new + scoring.unmatched_old) as u32 * UNMATCHED_PENALTY);

    if cacheable && cost < best_so_far {
        session
            .cost_cache
            .borrow_mut()
            .insert((new_ref, old_ref), cost);
    }

    cost
}

/// Count own property diffs between two WeakDom instances (flat, non-recursive).
/// Each differing property = +1. Children count diff = +1.
/// Does NOT include hash fast-path (handled by `compute_change_count`).
///
/// Properties present on only one side are skipped when their value matches
/// the class default (see `src/snapshot/matching.rs` for full rationale).
fn count_own_diffs(new_ref: Ref, old_ref: Ref, new_dom: &WeakDom, old_dom: &WeakDom) -> u32 {
    let new_inst = match new_dom.get_by_ref(new_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };
    let old_inst = match old_dom.get_by_ref(old_ref) {
        Some(i) => i,
        None => return UNMATCHED_PENALTY,
    };

    let mut cost: u32 = 0;

    let class_data = rbx_reflection_database::get()
        .ok()
        .and_then(|db| db.classes.get(new_inst.class.as_str()));

    for (key, new_val) in new_inst.properties.iter() {
        if let Some(old_val) = old_inst.properties.get(key) {
            if !variant_eq(new_val, old_val) {
                cost += 1;
            }
        } else {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq(new_val, default));
            if !is_default {
                cost += 1;
            }
        }
    }

    for (key, old_val) in old_inst.properties.iter() {
        if !new_inst.properties.contains_key(key) {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq(old_val, default));
            if !is_default {
                cost += 1;
            }
        }
    }

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

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );

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

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );

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

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );

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

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());
    }

    #[test]
    fn empty_children() {
        let new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let result = match_children(
            &[],
            &[],
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
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

        let r1 = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        let r2 = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );

        assert_eq!(r1.matched.len(), r2.matched.len());
        for (a, b) in r1.matched.iter().zip(r2.matched.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn total_cost_zero_identical() {
        let (new_dom, new_root, _, _) = build_test_dom();
        let (old_dom, old_root, _, _) = build_test_dom();

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        assert_eq!(result.matched.len(), 2);
        assert_eq!(
            result.total_cost, 0,
            "Identical instances should have zero cost"
        );
    }

    #[test]
    fn total_cost_nonzero_different() {
        use rbx_dom_weak::types::Variant;

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Part")
                .with_name("P")
                .with_property("Transparency", Variant::Float32(0.5)),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Part")
                .with_name("P")
                .with_property("Transparency", Variant::Float32(0.0)),
        );

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        assert_eq!(result.matched.len(), 1);
        assert!(
            result.total_cost > 0,
            "Different properties should produce nonzero cost, got {}",
            result.total_cost
        );
    }

    #[test]
    fn total_cost_includes_unmatched() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("A"));
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("B"));
        new_dom.insert(new_root, InstanceBuilder::new("Folder").with_name("C"));

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("A"));
        old_dom.insert(old_root, InstanceBuilder::new("Folder").with_name("B"));

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        assert_eq!(result.matched.len(), 2);
        assert_eq!(result.unmatched_new.len(), 1);
        assert!(
            result.total_cost >= UNMATCHED_PENALTY,
            "Should include penalty for unmatched, got {}",
            result.total_cost
        );
    }

    #[test]
    fn session_cache_consistent_syncback() {
        let (new_dom, new_root, _, _) = build_test_dom();
        let (old_dom, old_root, _, _) = build_test_dom();

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();
        let session = MatchingSession::new();

        let r1 = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &session,
        );
        let r2 = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &session,
        );
        assert_eq!(r1.matched.len(), r2.matched.len());
        assert_eq!(r1.total_cost, r2.total_cost);
    }
}
