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
use std::collections::HashMap;

use blake3::Hash;
use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{Ustr, WeakDom};

use crate::variant_eq::variant_eq_disk;

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
        };
    }

    let new_len = new_children.len();
    let old_len = old_children.len();
    let mut new_matched = vec![false; new_len];
    let mut old_matched = vec![false; old_len];
    let mut matched: Vec<(usize, usize)> = Vec::with_capacity(new_len.min(old_len));

    // ================================================================
    // Fast-path: Group by (Name, ClassName) -- 1:1 groups instant-match
    // ================================================================
    let mut new_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(new_len);
    for (i, &r) in new_children.iter().enumerate() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_key
                .entry((inst.name.clone(), inst.class))
                .or_default()
                .push(i);
        }
    }

    let mut old_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(old_len);
    for (i, &r) in old_children.iter().enumerate() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_key
                .entry((inst.name.clone(), inst.class))
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

    let new_remaining = new_matched.iter().filter(|&&m| !m).count();
    let old_remaining = old_matched.iter().filter(|&&m| !m).count();

    if new_remaining == 0 || old_remaining == 0 {
        return build_result(
            new_children,
            old_children,
            &new_matched,
            &old_matched,
            matched,
        );
    }

    // ================================================================
    // Ambiguous groups: change-count scoring + greedy assignment
    // ================================================================
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

        // Score all (A, B) pairs using recursive change-count scoring
        let mut pairs: Vec<(u32, usize, usize)> =
            Vec::with_capacity(avail_new.len() * avail_old.len());
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
                    0,
                    session,
                );
                pairs.push((cost, ni, oi));
                if cost < best_so_far {
                    best_so_far = cost;
                }
            }
        }

        // Stable sort by cost ascending
        pairs.sort_by_key(|&(cost, _, _)| cost);

        // Greedy assign
        for &(_, ni, oi) in &pairs {
            if new_matched[ni] || old_matched[oi] {
                continue;
            }
            matched.push((ni, oi));
            new_matched[ni] = true;
            old_matched[oi] = true;
        }
    }

    build_result(
        new_children,
        old_children,
        &new_matched,
        &old_matched,
        matched,
    )
}

/// Convert index-based matching results back to Ref-based MatchResult.
fn build_result(
    new_children: &[Ref],
    old_children: &[Ref],
    new_matched: &[bool],
    old_matched: &[bool],
    mut matched_indices: Vec<(usize, usize)>,
) -> MatchResult {
    matched_indices.sort_by_key(|&(ni, _)| ni);
    let matched: Vec<(Ref, Ref)> = matched_indices
        .into_iter()
        .map(|(ni, oi)| (new_children[ni], old_children[oi]))
        .collect();

    let unmatched_new: Vec<Ref> = new_matched
        .iter()
        .enumerate()
        .filter(|(_, &m)| !m)
        .map(|(i, _)| new_children[i])
        .collect();

    let unmatched_old: Vec<Ref> = old_matched
        .iter()
        .enumerate()
        .filter(|(_, &m)| !m)
        .map(|(i, _)| old_children[i])
        .collect();

    MatchResult {
        matched,
        unmatched_new,
        unmatched_old,
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
#[allow(clippy::too_many_arguments)]
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
    if new_children.is_empty() && old_children.is_empty() {
        return ScoringMatchResult {
            matched: Vec::new(),
            unmatched_new: 0,
            unmatched_old: 0,
        };
    }

    let new_len = new_children.len();
    let old_len = old_children.len();
    let mut new_matched = vec![false; new_len];
    let mut old_matched = vec![false; old_len];
    let mut matched: Vec<(usize, usize)> = Vec::with_capacity(new_len.min(old_len));

    // Group by (Name, ClassName)
    let mut new_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(new_len);
    for (i, &r) in new_children.iter().enumerate() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_key
                .entry((inst.name.clone(), inst.class))
                .or_default()
                .push(i);
        }
    }

    let mut old_by_key: HashMap<(String, Ustr), Vec<usize>> = HashMap::with_capacity(old_len);
    for (i, &r) in old_children.iter().enumerate() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_key
                .entry((inst.name.clone(), inst.class))
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

        let mut pairs: Vec<(u32, usize, usize)> =
            Vec::with_capacity(avail_new.len() * avail_old.len());
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
/// Tags and Attributes counted granularly (per-tag and per-attribute diffs).
/// Ref properties compared by target Name+ClassName at UNMATCHED_PENALTY per
/// mismatch. Children count diff = +1.
/// Does NOT include hash fast-path (handled by `compute_change_count`).
///
/// Properties present on only one side are skipped when their value matches
/// the class default (see `src/snapshot/matching.rs` for full rationale).
#[inline]
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
            cost += diff_variant_pair(new_val, old_val, new_dom, old_dom);
        } else {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq_disk(new_val, default));
            if !is_default {
                cost += count_variant_one_sided(new_val);
            }
        }
    }

    for (key, old_val) in old_inst.properties.iter() {
        if !new_inst.properties.contains_key(key) {
            let is_default = class_data
                .and_then(|cd| cd.default_properties.get(key.as_str()))
                .is_some_and(|default| variant_eq_disk(old_val, default));
            if !is_default {
                cost += count_variant_one_sided(old_val);
            }
        }
    }

    if new_inst.children().len() != old_inst.children().len() {
        cost += 1;
    }

    cost
}

/// Compare two Variant values with type-aware scoring for syncback.
/// Ref targets resolved by Name+ClassName from their respective DOMs.
#[inline]
fn diff_variant_pair(a: &Variant, b: &Variant, a_dom: &WeakDom, b_dom: &WeakDom) -> u32 {
    match (a, b) {
        (Variant::Ref(ref_a), Variant::Ref(ref_b)) => {
            if ref_a == ref_b {
                return 0;
            }
            let a_none = !ref_a.is_some();
            let b_none = !ref_b.is_some();
            if a_none != b_none {
                return UNMATCHED_PENALTY;
            }
            if a_none && b_none {
                return 0;
            }
            let target_a = a_dom.get_by_ref(*ref_a);
            let target_b = b_dom.get_by_ref(*ref_b);
            match (target_a, target_b) {
                (Some(ta), Some(tb)) => {
                    if ta.name == tb.name && ta.class == tb.class {
                        0
                    } else {
                        UNMATCHED_PENALTY
                    }
                }
                (None, None) => 0,
                _ => UNMATCHED_PENALTY,
            }
        }
        (Variant::Tags(tags_a), Variant::Tags(tags_b)) => {
            use std::collections::HashSet;
            let set_a: HashSet<&str> = tags_a.iter().collect();
            let set_b: HashSet<&str> = tags_b.iter().collect();
            (set_a.difference(&set_b).count() + set_b.difference(&set_a).count()) as u32
        }
        (Variant::Attributes(attrs_a), Variant::Attributes(attrs_b)) => {
            let mut cost: u32 = 0;
            for (key, a_val) in attrs_a.iter() {
                match attrs_b.get(key.as_str()) {
                    Some(b_val) => {
                        if !variant_eq_disk(a_val, b_val) {
                            cost += 1;
                        }
                    }
                    None => cost += 1,
                }
            }
            for (key, _) in attrs_b.iter() {
                if attrs_a.get(key.as_str()).is_none() {
                    cost += 1;
                }
            }
            cost
        }
        _ => {
            if variant_eq_disk(a, b) {
                0
            } else {
                1
            }
        }
    }
}

/// Cost for a one-sided Variant (property present on only one side).
#[inline]
fn count_variant_one_sided(val: &Variant) -> u32 {
    match val {
        Variant::Ref(r) => {
            if r.is_some() {
                UNMATCHED_PENALTY
            } else {
                0
            }
        }
        Variant::Tags(tags) => tags.len() as u32,
        Variant::Attributes(attrs) => attrs.len() as u32,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::{ustr, InstanceBuilder};

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
    }

    #[test]
    fn ref_scoring_syncback() {
        use rbx_dom_weak::types::Variant;

        // 2 Models named "Weapon" in each DOM, each with a differently-named
        // child Part. A Ref property on each Model points to its own child.
        // Syncback matching must pair by Ref target Name+ClassName, not by
        // raw Ref identity (which always differs between DOMs).

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();

        let new_model_a =
            new_dom.insert(new_root, InstanceBuilder::new("Model").with_name("Weapon"));
        let new_handle = new_dom.insert(
            new_model_a,
            InstanceBuilder::new("Part").with_name("Handle"),
        );
        new_dom
            .get_by_ref_mut(new_model_a)
            .unwrap()
            .properties
            .insert("PrimaryPart".into(), Variant::Ref(new_handle));

        let new_model_b =
            new_dom.insert(new_root, InstanceBuilder::new("Model").with_name("Weapon"));
        let new_grip = new_dom.insert(new_model_b, InstanceBuilder::new("Part").with_name("Grip"));
        new_dom
            .get_by_ref_mut(new_model_b)
            .unwrap()
            .properties
            .insert("PrimaryPart".into(), Variant::Ref(new_grip));

        // Old DOM: reversed order (Grip-model first, Handle-model second)
        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();

        let old_model_b =
            old_dom.insert(old_root, InstanceBuilder::new("Model").with_name("Weapon"));
        let old_grip = old_dom.insert(old_model_b, InstanceBuilder::new("Part").with_name("Grip"));
        old_dom
            .get_by_ref_mut(old_model_b)
            .unwrap()
            .properties
            .insert("PrimaryPart".into(), Variant::Ref(old_grip));

        let old_model_a =
            old_dom.insert(old_root, InstanceBuilder::new("Model").with_name("Weapon"));
        let old_handle = old_dom.insert(
            old_model_a,
            InstanceBuilder::new("Part").with_name("Handle"),
        );
        old_dom
            .get_by_ref_mut(old_model_a)
            .unwrap()
            .properties
            .insert("PrimaryPart".into(), Variant::Ref(old_handle));

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
            let new_inst = new_dom.get_by_ref(*new_ref).unwrap();
            let old_inst = old_dom.get_by_ref(*old_ref).unwrap();

            let new_child_name = &new_dom.get_by_ref(new_inst.children()[0]).unwrap().name;
            let old_child_name = &old_dom.get_by_ref(old_inst.children()[0]).unwrap().name;
            assert_eq!(
                new_child_name, old_child_name,
                "Ref scoring failed: new model with child '{}' matched old model with child '{}'",
                new_child_name, old_child_name
            );
        }
    }

    // ================================================================
    // Large ambiguous group tests (50+ same-named instances)
    // ================================================================

    #[test]
    fn fifty_same_name_parts_five_groups_syncback() {
        use rbx_dom_weak::types::{Color3, Variant, Vector3};

        let groups: Vec<(Vector3, Color3)> = vec![
            (Vector3::new(0.0, 0.0, 0.0), Color3::new(1.0, 0.0, 0.0)),
            (Vector3::new(0.0, 5.0, 0.0), Color3::new(1.0, 0.0, 0.0)),
            (Vector3::new(0.0, 10.0, 0.0), Color3::new(0.0, 1.0, 0.0)),
            (Vector3::new(0.0, 0.0, 0.0), Color3::new(0.0, 1.0, 0.0)),
            (Vector3::new(0.0, 5.0, 0.0), Color3::new(0.0, 0.0, 1.0)),
        ];

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        for (pos, color) in &groups {
            for _ in 0..10 {
                new_dom.insert(
                    new_root,
                    InstanceBuilder::new("Part")
                        .with_name("Line")
                        .with_property("Position", Variant::Vector3(*pos))
                        .with_property("Color", Variant::Color3(*color)),
                );
            }
        }

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        for (pos, color) in groups.iter().rev() {
            for _ in 0..10 {
                old_dom.insert(
                    old_root,
                    InstanceBuilder::new("Part")
                        .with_name("Line")
                        .with_property("Position", Variant::Vector3(*pos))
                        .with_property("Color", Variant::Color3(*color)),
                );
            }
        }

        let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
        let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

        let start = std::time::Instant::now();
        let result = match_children(
            &new_children,
            &old_children,
            &new_dom,
            &old_dom,
            None,
            None,
            &MatchingSession::new(),
        );
        let elapsed = start.elapsed();

        assert_eq!(result.matched.len(), 50);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());
        assert!(elapsed.as_secs() < 5, "took too long: {:?}", elapsed);

        for (new_ref, old_ref) in &result.matched {
            let new_inst = new_dom.get_by_ref(*new_ref).unwrap();
            let old_inst = old_dom.get_by_ref(*old_ref).unwrap();

            let new_pos = new_inst.properties.get(&ustr("Position")).unwrap();
            let old_pos = old_inst.properties.get(&ustr("Position")).unwrap();
            assert!(
                variant_eq_disk(new_pos, old_pos),
                "Position mismatch: new={:?}, old={:?}",
                new_pos,
                old_pos
            );
            let new_color = new_inst.properties.get(&ustr("Color")).unwrap();
            let old_color = old_inst.properties.get(&ustr("Color")).unwrap();
            assert!(
                variant_eq_disk(new_color, old_color),
                "Color mismatch: new={:?}, old={:?}",
                new_color,
                old_color
            );
        }
    }

    #[test]
    fn fifty_parts_position_only_syncback() {
        use rbx_dom_weak::types::{Color3, Variant, Vector3};

        let shared_color = Color3::new(0.5, 0.5, 0.5);
        let positions = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
            Vector3::new(0.0, 10.0, 0.0),
            Vector3::new(0.0, 0.0, 10.0),
            Vector3::new(5.0, 5.0, 5.0),
        ];

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();

        for pos in &positions {
            for _ in 0..10 {
                new_dom.insert(
                    new_root,
                    InstanceBuilder::new("Part")
                        .with_name("Block")
                        .with_property("Position", Variant::Vector3(*pos))
                        .with_property("Color", Variant::Color3(shared_color)),
                );
            }
        }
        for pos in positions.iter().rev() {
            for _ in 0..10 {
                old_dom.insert(
                    old_root,
                    InstanceBuilder::new("Part")
                        .with_name("Block")
                        .with_property("Position", Variant::Vector3(*pos))
                        .with_property("Color", Variant::Color3(shared_color)),
                );
            }
        }

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

        assert_eq!(result.matched.len(), 50);
        for (new_ref, old_ref) in &result.matched {
            let new_pos = new_dom
                .get_by_ref(*new_ref)
                .unwrap()
                .properties
                .get(&ustr("Position"))
                .unwrap();
            let old_pos = old_dom
                .get_by_ref(*old_ref)
                .unwrap()
                .properties
                .get(&ustr("Position"))
                .unwrap();
            assert!(variant_eq_disk(new_pos, old_pos), "Position mismatch");
        }
    }

    #[test]
    fn fifty_parts_color_only_syncback() {
        use rbx_dom_weak::types::{Color3, Variant, Vector3};

        let shared_pos = Vector3::new(0.0, 0.0, 0.0);
        let colors = [
            Color3::new(1.0, 0.0, 0.0),
            Color3::new(0.0, 1.0, 0.0),
            Color3::new(0.0, 0.0, 1.0),
            Color3::new(1.0, 1.0, 0.0),
            Color3::new(0.0, 1.0, 1.0),
        ];

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();

        for color in &colors {
            for _ in 0..10 {
                new_dom.insert(
                    new_root,
                    InstanceBuilder::new("Part")
                        .with_name("Block")
                        .with_property("Position", Variant::Vector3(shared_pos))
                        .with_property("Color", Variant::Color3(*color)),
                );
            }
        }
        for color in colors.iter().rev() {
            for _ in 0..10 {
                old_dom.insert(
                    old_root,
                    InstanceBuilder::new("Part")
                        .with_name("Block")
                        .with_property("Position", Variant::Vector3(shared_pos))
                        .with_property("Color", Variant::Color3(*color)),
                );
            }
        }

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

        assert_eq!(result.matched.len(), 50);
        for (new_ref, old_ref) in &result.matched {
            let new_color = new_dom
                .get_by_ref(*new_ref)
                .unwrap()
                .properties
                .get(&ustr("Color"))
                .unwrap();
            let old_color = old_dom
                .get_by_ref(*old_ref)
                .unwrap()
                .properties
                .get(&ustr("Color"))
                .unwrap();
            assert!(variant_eq_disk(new_color, old_color), "Color mismatch");
        }
    }

    #[test]
    fn sixty_parts_near_float_syncback() {
        use rbx_dom_weak::types::Variant;

        let groups: Vec<f32> = vec![0.1, 0.100001, 0.5, 0.500001, 0.999999];
        let group_size = 12;

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();

        for &t in &groups {
            for _ in 0..group_size {
                new_dom.insert(
                    new_root,
                    InstanceBuilder::new("Part")
                        .with_name("Segment")
                        .with_property("Transparency", Variant::Float32(t)),
                );
            }
        }
        for &t in groups.iter().rev() {
            for _ in 0..group_size {
                old_dom.insert(
                    old_root,
                    InstanceBuilder::new("Part")
                        .with_name("Segment")
                        .with_property("Transparency", Variant::Float32(t)),
                );
            }
        }

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

        assert_eq!(result.matched.len(), 60);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());

        for (new_ref, old_ref) in &result.matched {
            let new_t = new_dom
                .get_by_ref(*new_ref)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
                .unwrap();
            let old_t = old_dom
                .get_by_ref(*old_ref)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
                .unwrap();
            assert!(
                variant_eq_disk(new_t, old_t),
                "Transparency mismatch: new={:?}, old={:?}",
                new_t,
                old_t
            );
        }
    }

    #[test]
    fn disk_representation_boundary_syncback() {
        use rbx_dom_weak::types::Variant;

        let a_val: f32 = 1.00005;
        let b_val: f32 = 1.00015;

        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Part")
                .with_name("Edge")
                .with_property("Transparency", Variant::Float32(a_val)),
        );
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Part")
                .with_name("Edge")
                .with_property("Transparency", Variant::Float32(b_val)),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Part")
                .with_name("Edge")
                .with_property("Transparency", Variant::Float32(b_val)),
        );
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Part")
                .with_name("Edge")
                .with_property("Transparency", Variant::Float32(a_val)),
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
            let new_t = new_dom
                .get_by_ref(*new_ref)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
                .unwrap();
            let old_t = old_dom
                .get_by_ref(*old_ref)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
                .unwrap();
            assert!(
                variant_eq_disk(new_t, old_t),
                "Boundary test failed: new={:?}, old={:?}",
                new_t,
                old_t
            );
        }
    }
}
