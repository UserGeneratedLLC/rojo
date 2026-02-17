//! 3-pass instance matching algorithm for syncback.
//!
//! Pairs new children (from Roblox file) to old children (from filesystem)
//! using progressively more expensive matching signals:
//!
//! - **Pass 1** (O(n)): unique name matching + ClassName narrowing within
//!   same-name groups.
//! - **Pass 2**: Ref property discriminators. If a Ref target is already
//!   matched in Pass 1, use it to differentiate candidates.
//! - **Pass 3**: pairwise similarity scoring using precomputed subtree
//!   hashes. Greedy best-match-first with DOM child order as tiebreaker.
//!
//! The algorithm is designed for the syncback context where both "new" and
//! "old" sides are WeakDom instances.

use std::collections::{HashMap, HashSet};

use blake3::Hash;
use rbx_dom_weak::{
    types::{Ref, Variant},
    ustr, WeakDom,
};

/// Result of the matching algorithm.
#[derive(Debug)]
pub struct MatchResult {
    /// Pairs of (new_ref, old_ref) that were matched.
    pub matched: Vec<(Ref, Ref)>,
    /// New children with no match on the old side (to be created).
    pub unmatched_new: Vec<Ref>,
    /// Old children with no match on the new side (to be deleted).
    pub unmatched_old: Vec<Ref>,
}

/// Run the 3-pass matching algorithm.
///
/// `new_children` and `old_children` are the Refs of children under the same
/// parent in their respective WeakDoms. `new_hashes` and `old_hashes` are
/// optional precomputed subtree hashes (from `hash_tree()`).
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

    // ---- Pass 1: unique name matching + ClassName narrowing ----
    pass1_name_and_class(
        &mut remaining_new,
        &mut remaining_old,
        new_dom,
        old_dom,
        &mut matched,
    );

    // Fast path: if all matched, skip Passes 2 and 3.
    if remaining_new.is_empty() || remaining_old.is_empty() {
        return MatchResult {
            matched,
            unmatched_new: remaining_new,
            unmatched_old: remaining_old,
        };
    }

    // ---- Pass 2: Ref property discriminators ----
    // Snapshot the already-matched pairs so we can borrow matched mutably.
    let matched_snapshot: Vec<(Ref, Ref)> = matched.clone();
    pass2_ref_discriminators(
        &mut remaining_new,
        &mut remaining_old,
        new_dom,
        old_dom,
        &matched_snapshot,
        &mut matched,
    );

    if remaining_new.is_empty() || remaining_old.is_empty() {
        return MatchResult {
            matched,
            unmatched_new: remaining_new,
            unmatched_old: remaining_old,
        };
    }

    // ---- Pass 3: similarity scoring ----
    pass3_similarity(
        &mut remaining_new,
        &mut remaining_old,
        new_dom,
        old_dom,
        new_hashes,
        old_hashes,
        new_children,
        old_children,
        &mut matched,
    );

    MatchResult {
        matched,
        unmatched_new: remaining_new,
        unmatched_old: remaining_old,
    }
}

/// Pass 1: Match instances with unique names, then narrow same-name groups by
/// ClassName.
fn pass1_name_and_class(
    remaining_new: &mut Vec<Ref>,
    remaining_old: &mut Vec<Ref>,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    matched: &mut Vec<(Ref, Ref)>,
) {
    // Build name→refs maps for both sides.
    let mut new_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_new.iter() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_name
                .entry(inst.name.clone())
                .or_default()
                .push(r);
        }
    }
    let mut old_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_old.iter() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_name
                .entry(inst.name.to_string())
                .or_default()
                .push(r);
        }
    }

    let mut matched_new: HashSet<Ref> = HashSet::new();
    let mut matched_old: HashSet<Ref> = HashSet::new();

    // Sort names for deterministic iteration order.
    let mut sorted_names: Vec<&String> = new_by_name.keys().collect();
    sorted_names.sort();

    for name in sorted_names {
        let new_refs = &new_by_name[name];
        let Some(old_refs) = old_by_name.get(name.as_str()) else {
            continue;
        };

        // Case 1: exactly one on each side → instant match.
        if new_refs.len() == 1 && old_refs.len() == 1 {
            matched.push((new_refs[0], old_refs[0]));
            matched_new.insert(new_refs[0]);
            matched_old.insert(old_refs[0]);
            continue;
        }

        // Case 2: multiple on at least one side → try ClassName narrowing.
        let mut new_by_class: HashMap<&str, Vec<Ref>> = HashMap::new();
        for &r in new_refs {
            if let Some(inst) = new_dom.get_by_ref(r) {
                new_by_class.entry(&inst.class).or_default().push(r);
            }
        }
        let mut old_by_class: HashMap<&str, Vec<Ref>> = HashMap::new();
        for &r in old_refs {
            if let Some(inst) = old_dom.get_by_ref(r) {
                old_by_class.entry(&inst.class).or_default().push(r);
            }
        }

        for (class_name, new_class_refs) in &new_by_class {
            if let Some(old_class_refs) = old_by_class.get(class_name) {
                if new_class_refs.len() == 1 && old_class_refs.len() == 1 {
                    matched.push((new_class_refs[0], old_class_refs[0]));
                    matched_new.insert(new_class_refs[0]);
                    matched_old.insert(old_class_refs[0]);
                }
            }
        }
    }

    remaining_new.retain(|r| !matched_new.contains(r));
    remaining_old.retain(|r| !matched_old.contains(r));
}

/// Pass 2: Use pre-resolved Ref properties to differentiate instances within
/// same-name groups.
fn pass2_ref_discriminators(
    remaining_new: &mut Vec<Ref>,
    remaining_old: &mut Vec<Ref>,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    already_matched: &[(Ref, Ref)],
    matched: &mut Vec<(Ref, Ref)>,
) {
    if remaining_new.is_empty() || remaining_old.is_empty() {
        return;
    }

    // Build bidirectional match maps: new→old and old→new.
    let new_to_old: HashMap<Ref, Ref> = already_matched.iter().copied().collect();
    let old_to_new: HashMap<Ref, Ref> = already_matched
        .iter()
        .map(|&(n, o)| (o, n))
        .collect();

    // Collect Ref property targets for each remaining instance.
    let new_ref_targets: HashMap<Ref, Vec<Ref>> = remaining_new
        .iter()
        .map(|&r| (r, collect_ref_property_targets(new_dom, r)))
        .collect();
    let old_ref_targets: HashMap<Ref, Vec<Ref>> = remaining_old
        .iter()
        .map(|&r| (r, collect_ref_property_targets(old_dom, r)))
        .collect();

    // Group remaining by name for matching within groups.
    let mut new_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_new.iter() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_name
                .entry(inst.name.clone())
                .or_default()
                .push(r);
        }
    }
    let mut old_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_old.iter() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_name
                .entry(inst.name.to_string())
                .or_default()
                .push(r);
        }
    }

    let mut matched_new: HashSet<Ref> = HashSet::new();
    let mut matched_old: HashSet<Ref> = HashSet::new();

    for (name, new_refs) in &new_by_name {
        let Some(old_refs) = old_by_name.get(name) else {
            continue;
        };

        for &new_ref in new_refs {
            if matched_new.contains(&new_ref) {
                continue;
            }
            let new_targets = &new_ref_targets[&new_ref];
            if new_targets.is_empty() {
                continue;
            }
            let translated: Vec<Ref> = new_targets
                .iter()
                .filter_map(|t| new_to_old.get(t).copied())
                .collect();
            if translated.is_empty() {
                continue;
            }

            let mut candidates: Vec<Ref> = Vec::new();
            for &old_ref in old_refs {
                if matched_old.contains(&old_ref) {
                    continue;
                }
                let old_targets = &old_ref_targets[&old_ref];
                let old_translated: Vec<Ref> = old_targets
                    .iter()
                    .filter_map(|t| old_to_new.get(t).copied())
                    .collect();

                if translated.iter().any(|t| old_targets.contains(t))
                    || old_translated.iter().any(|t| new_targets.contains(t))
                {
                    candidates.push(old_ref);
                }
            }

            if candidates.len() == 1 {
                matched.push((new_ref, candidates[0]));
                matched_new.insert(new_ref);
                matched_old.insert(candidates[0]);
            }
        }
    }

    remaining_new.retain(|r| !matched_new.contains(r));
    remaining_old.retain(|r| !matched_old.contains(r));
}

/// Pass 3: Similarity scoring using subtree hashes and greedy assignment.
#[allow(clippy::too_many_arguments)]
fn pass3_similarity(
    remaining_new: &mut Vec<Ref>,
    remaining_old: &mut Vec<Ref>,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
    original_new_children: &[Ref],
    original_old_children: &[Ref],
    matched: &mut Vec<(Ref, Ref)>,
) {
    if remaining_new.is_empty() || remaining_old.is_empty() {
        return;
    }

    // Group remaining by name.
    let mut new_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_new.iter() {
        if let Some(inst) = new_dom.get_by_ref(r) {
            new_by_name
                .entry(inst.name.clone())
                .or_default()
                .push(r);
        }
    }
    let mut old_by_name: HashMap<String, Vec<Ref>> = HashMap::new();
    for &r in remaining_old.iter() {
        if let Some(inst) = old_dom.get_by_ref(r) {
            old_by_name
                .entry(inst.name.to_string())
                .or_default()
                .push(r);
        }
    }

    let mut matched_new: HashSet<Ref> = HashSet::new();
    let mut matched_old: HashSet<Ref> = HashSet::new();

    for (name, new_refs) in &new_by_name {
        let Some(old_refs) = old_by_name.get(name) else {
            continue;
        };

        let mut pairs: Vec<(Ref, Ref, u32)> = Vec::new();
        for &new_ref in new_refs {
            if matched_new.contains(&new_ref) {
                continue;
            }
            for &old_ref in old_refs {
                if matched_old.contains(&old_ref) {
                    continue;
                }
                let score = compute_similarity(
                    new_ref, old_ref, new_dom, old_dom, new_hashes, old_hashes,
                );
                pairs.push((new_ref, old_ref, score));
            }
        }

        pairs.sort_by(|a, b| {
            b.2.cmp(&a.2).then_with(|| {
                let a_new_idx = original_new_children
                    .iter()
                    .position(|&r| r == a.0)
                    .unwrap_or(usize::MAX);
                let b_new_idx = original_new_children
                    .iter()
                    .position(|&r| r == b.0)
                    .unwrap_or(usize::MAX);
                a_new_idx.cmp(&b_new_idx).then_with(|| {
                    let a_old_idx = original_old_children
                        .iter()
                        .position(|&r| r == a.1)
                        .unwrap_or(usize::MAX);
                    let b_old_idx = original_old_children
                        .iter()
                        .position(|&r| r == b.1)
                        .unwrap_or(usize::MAX);
                    a_old_idx.cmp(&b_old_idx)
                })
            })
        });

        for (new_ref, old_ref, _score) in &pairs {
            if matched_new.contains(new_ref) || matched_old.contains(old_ref) {
                continue;
            }
            matched.push((*new_ref, *old_ref));
            matched_new.insert(*new_ref);
            matched_old.insert(*old_ref);
        }
    }

    remaining_new.retain(|r| !matched_new.contains(r));
    remaining_old.retain(|r| !matched_old.contains(r));
}

/// Compute a similarity score between two instances.
/// Higher = more similar. Signals checked in priority order:
/// ClassName (100), Tags (50), Attributes (30), Hash equality (1000),
/// Properties overlap (10 per match).
fn compute_similarity(
    new_ref: Ref,
    old_ref: Ref,
    new_dom: &WeakDom,
    old_dom: &WeakDom,
    new_hashes: Option<&HashMap<Ref, Hash>>,
    old_hashes: Option<&HashMap<Ref, Hash>>,
) -> u32 {
    let new_inst = match new_dom.get_by_ref(new_ref) {
        Some(i) => i,
        None => return 0,
    };
    let old_inst = match old_dom.get_by_ref(old_ref) {
        Some(i) => i,
        None => return 0,
    };

    let mut score: u32 = 0;

    // ClassName match is a strong signal.
    if new_inst.class == old_inst.class {
        score += 100;
    }

    // Hash equality: very likely identical content.
    if let (Some(nh), Some(oh)) = (new_hashes, old_hashes) {
        if let (Some(new_hash), Some(old_hash)) = (nh.get(&new_ref), oh.get(&old_ref)) {
            if new_hash == old_hash {
                score += 1000;
            }
        }
    }

    // Tags comparison.
    if let (Some(Variant::Tags(new_tags)), Some(Variant::Tags(old_tags))) = (
        new_inst.properties.get(&ustr("Tags")),
        old_inst.properties.get(&ustr("Tags")),
    ) {
        if new_tags == old_tags {
            score += 50;
        }
    }

    // Attributes comparison.
    if let (Some(Variant::Attributes(new_attrs)), Some(Variant::Attributes(old_attrs))) = (
        new_inst.properties.get(&ustr("Attributes")),
        old_inst.properties.get(&ustr("Attributes")),
    ) {
        if new_attrs == old_attrs {
            score += 30;
        }
    }

    // Children count similarity (cheap structural signal).
    let new_child_count = new_inst.children().len();
    let old_child_count = old_inst.children().len();
    if new_child_count == old_child_count {
        score += 20;
    }

    score
}

/// Collect all Ref-valued property targets for an instance.
fn collect_ref_property_targets(dom: &WeakDom, inst_ref: Ref) -> Vec<Ref> {
    let inst = match dom.get_by_ref(inst_ref) {
        Some(i) => i,
        None => return Vec::new(),
    };
    let mut targets = Vec::new();
    for value in inst.properties.values() {
        if let Variant::Ref(target) = value {
            if !target.is_none() {
                targets.push(*target);
            }
        }
    }
    targets
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
        let (new_dom, new_root, new_a, new_b) = build_test_dom();
        let (old_dom, old_root, old_a, old_b) = build_test_dom();

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();
        let old_children: Vec<Ref> = old_dom
            .get_by_ref(old_root)
            .unwrap()
            .children()
            .to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());

        // Alpha matched to Alpha, Beta to Beta
        for (new_ref, old_ref) in &result.matched {
            let new_name = &new_dom.get_by_ref(*new_ref).unwrap().name;
            let old_name = &old_dom.get_by_ref(*old_ref).unwrap().name;
            assert_eq!(new_name, old_name);
        }
    }

    #[test]
    fn class_name_narrowing() {
        // Two instances named "Handler" but different classes
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        let new_a = new_dom.insert(
            new_root,
            InstanceBuilder::new("Script").with_name("Handler"),
        );
        let new_b = new_dom.insert(
            new_root,
            InstanceBuilder::new("ModuleScript").with_name("Handler"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        let old_a = old_dom.insert(
            old_root,
            InstanceBuilder::new("ModuleScript").with_name("Handler"),
        );
        let old_b = old_dom.insert(
            old_root,
            InstanceBuilder::new("Script").with_name("Handler"),
        );

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();
        let old_children: Vec<Ref> = old_dom
            .get_by_ref(old_root)
            .unwrap()
            .children()
            .to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 2);
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());

        // Verify Script matched to Script, ModuleScript to ModuleScript.
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
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("Alpha"),
        );
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("NewOnly"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("Alpha"),
        );
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("OldOnly"),
        );

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();
        let old_children: Vec<Ref> = old_dom
            .get_by_ref(old_root)
            .unwrap()
            .children()
            .to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.unmatched_new.len(), 1);
        assert_eq!(result.unmatched_old.len(), 1);

        let unmatched_new_name = &new_dom
            .get_by_ref(result.unmatched_new[0])
            .unwrap()
            .name;
        assert_eq!(unmatched_new_name, "NewOnly");

        let unmatched_old_name = &old_dom
            .get_by_ref(result.unmatched_old[0])
            .unwrap()
            .name;
        assert_eq!(unmatched_old_name, "OldOnly");
    }

    #[test]
    fn duplicate_names_fall_to_pass3() {
        // Two Folders named "Data" on each side, same class.
        // Pass 1 can't distinguish, Pass 3 uses greedy assignment.
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("Data"),
        );
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("Data"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("Data"),
        );
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("Data"),
        );

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();
        let old_children: Vec<Ref> = old_dom
            .get_by_ref(old_root)
            .unwrap()
            .children()
            .to_vec();

        let result = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        // Both should be matched (greedy assignment).
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
        assert!(result.unmatched_new.is_empty());
        assert!(result.unmatched_old.is_empty());
    }

    #[test]
    fn all_new_no_old() {
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("A"),
        );

        let old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();

        let result = match_children(&new_children, &[], &new_dom, &old_dom, None, None);

        assert!(result.matched.is_empty());
        assert_eq!(result.unmatched_new.len(), 1);
        assert!(result.unmatched_old.is_empty());
    }

    #[test]
    fn matching_stability() {
        // Run the same matching twice; verify identical results.
        let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let new_root = new_dom.root_ref();
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Folder").with_name("X"),
        );
        new_dom.insert(
            new_root,
            InstanceBuilder::new("Script").with_name("Y"),
        );

        let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let old_root = old_dom.root_ref();
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Folder").with_name("X"),
        );
        old_dom.insert(
            old_root,
            InstanceBuilder::new("Script").with_name("Y"),
        );

        let new_children: Vec<Ref> = new_dom
            .get_by_ref(new_root)
            .unwrap()
            .children()
            .to_vec();
        let old_children: Vec<Ref> = old_dom
            .get_by_ref(old_root)
            .unwrap()
            .children()
            .to_vec();

        let r1 = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);
        let r2 = match_children(&new_children, &old_children, &new_dom, &old_dom, None, None);

        assert_eq!(r1.matched.len(), r2.matched.len());
        for (a, b) in r1.matched.iter().zip(r2.matched.iter()) {
            assert_eq!(a, b);
        }
    }
}
