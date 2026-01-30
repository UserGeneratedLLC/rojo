//! Implements iterating through an entire WeakDom and linking all Ref
//! properties using path-based attributes (preferred) or ID-based attributes
//! (fallback for non-unique paths).

use std::collections::{HashMap, HashSet, VecDeque};

use rbx_dom_weak::{
    types::{Attributes, Ref, UniqueId, Variant},
    ustr, Instance, Ustr, WeakDom,
};

use crate::{
    syncback::snapshot::inst_path, REF_ID_ATTRIBUTE_NAME, REF_PATH_ATTRIBUTE_PREFIX,
    REF_POINTER_ATTRIBUTE_PREFIX,
};

pub struct RefLinks {
    /// Refs that use path-based linking (path is unique).
    path_links: HashMap<Ref, Vec<PathRefLink>>,
    /// Refs that use ID-based linking (path is NOT unique - fallback).
    id_links: HashMap<Ref, Vec<IdRefLink>>,
    /// Target instances that need a Rojo_Id written (for ID-based system).
    targets_needing_id: HashSet<Ref>,
}

#[derive(PartialEq, Eq)]
struct PathRefLink {
    name: Ustr,
    path: String,
}

#[derive(PartialEq, Eq)]
struct IdRefLink {
    name: Ustr,
    target: Ref,
}

/// Collects all instance paths in a WeakDom before any pruning occurs.
/// Returns a map of Ref -> path for all instances.
pub fn collect_all_paths(dom: &WeakDom) -> HashMap<Ref, String> {
    let mut paths = HashMap::new();
    let mut queue = VecDeque::new();
    queue.push_back(dom.root_ref());

    while let Some(inst_ref) = queue.pop_front() {
        let inst = dom.get_by_ref(inst_ref).unwrap();
        queue.extend(inst.children().iter().copied());
        paths.insert(inst_ref, inst_path(dom, inst_ref));
    }

    paths
}

/// Pre-computes which instances have duplicate-named siblings.
/// Returns a HashSet of Refs that have at least one sibling with the same name.
///
/// This is O(N) where N is the number of instances, and allows subsequent
/// path uniqueness checks to be O(d) instead of O(d × s) where d=depth, s=siblings.
fn compute_refs_with_duplicate_siblings(dom: &WeakDom) -> HashSet<Ref> {
    let mut has_duplicate_siblings = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(dom.root_ref());

    while let Some(inst_ref) = queue.pop_front() {
        let inst = match dom.get_by_ref(inst_ref) {
            Some(i) => i,
            None => continue,
        };

        // Count children by name and collect their refs
        let mut name_to_refs: HashMap<&str, Vec<Ref>> = HashMap::new();
        for child_ref in inst.children() {
            if let Some(child) = dom.get_by_ref(*child_ref) {
                name_to_refs.entry(&child.name).or_default().push(*child_ref);
            }
            queue.push_back(*child_ref);
        }

        // Mark refs that share a name with siblings
        for (_name, refs) in name_to_refs {
            if refs.len() > 1 {
                for r in refs {
                    has_duplicate_siblings.insert(r);
                }
            }
        }
    }

    has_duplicate_siblings
}

/// Checks if a path is unique using pre-computed duplicate sibling info.
/// This is O(d) where d is the depth of the instance.
fn is_path_unique_with_cache(
    dom: &WeakDom,
    target_ref: Ref,
    has_duplicate_siblings: &HashSet<Ref>,
) -> bool {
    let mut current_ref = target_ref;

    // Walk up the tree checking each level using the pre-computed cache
    loop {
        // O(1) lookup instead of O(siblings) counting
        if has_duplicate_siblings.contains(&current_ref) {
            return false;
        }

        let current = match dom.get_by_ref(current_ref) {
            Some(inst) => inst,
            None => return false,
        };

        let parent_ref = current.parent();
        if parent_ref.is_none() {
            // Reached root - path is unique at all levels
            return true;
        }

        // Move up to parent and check the next level
        current_ref = parent_ref;
    }
}

/// Iterates through a WeakDom and collects referent properties.
/// Uses paths when unique, falls back to IDs when paths are ambiguous.
///
/// The `pre_prune_paths` parameter should contain paths for instances that may
/// have been pruned from the DOM. This allows references to instances outside
/// the sync tree to be preserved as path-based attributes.
pub fn collect_referents(dom: &WeakDom, pre_prune_paths: &HashMap<Ref, String>) -> RefLinks {
    let mut path_links: HashMap<Ref, Vec<PathRefLink>> = HashMap::new();
    let mut id_links: HashMap<Ref, Vec<IdRefLink>> = HashMap::new();
    let mut targets_needing_id: HashSet<Ref> = HashSet::new();

    // Pre-compute duplicate sibling info in O(N) - this makes all subsequent
    // path uniqueness checks O(d) instead of O(d × s) where d=depth, s=siblings
    let has_duplicate_siblings = compute_refs_with_duplicate_siblings(dom);

    // Cache path uniqueness results to avoid re-checking the same target
    let mut path_unique_cache: HashMap<Ref, bool> = HashMap::new();

    let mut queue = VecDeque::new();
    queue.push_back(dom.root_ref());

    while let Some(inst_ref) = queue.pop_front() {
        let pointer = dom.get_by_ref(inst_ref).unwrap();
        queue.extend(pointer.children().iter().copied());

        for (prop_name, prop_value) in &pointer.properties {
            let Variant::Ref(target_ref) = prop_value else {
                continue;
            };
            if target_ref.is_none() {
                continue;
            }

            // Check if target exists in current DOM
            if let Some(_target) = dom.get_by_ref(*target_ref) {
                // Target exists - check if path is unique (with caching)
                let is_unique = *path_unique_cache
                    .entry(*target_ref)
                    .or_insert_with(|| is_path_unique_with_cache(dom, *target_ref, &has_duplicate_siblings));

                if is_unique {
                    // Path is unique - use path-based system
                    let target_path = inst_path(dom, *target_ref);
                    path_links.entry(inst_ref).or_default().push(PathRefLink {
                        name: *prop_name,
                        path: target_path,
                    });
                } else {
                    // Path is NOT unique - fall back to ID-based system
                    log::debug!(
                        "Property {}.{} uses ID-based reference because target has duplicate-named siblings",
                        inst_path(dom, inst_ref),
                        prop_name
                    );
                    id_links.entry(inst_ref).or_default().push(IdRefLink {
                        name: *prop_name,
                        target: *target_ref,
                    });
                    targets_needing_id.insert(*target_ref);
                }
            } else if let Some(external_path) = pre_prune_paths.get(target_ref) {
                // Target was pruned - use path (we can't check uniqueness, assume it's fine)
                log::debug!(
                    "Property {}.{} points to pruned instance at '{}', storing as path reference",
                    inst_path(dom, inst_ref),
                    prop_name,
                    external_path
                );
                path_links.entry(inst_ref).or_default().push(PathRefLink {
                    name: *prop_name,
                    path: external_path.clone(),
                });
            } else {
                // Truly dangling reference
                log::warn!(
                    "Property {}.{} will be `nil` on disk because the referenced instance does not exist",
                    inst_path(dom, inst_ref),
                    prop_name
                );
            }
        }
    }

    RefLinks {
        path_links,
        id_links,
        targets_needing_id,
    }
}

/// Writes reference attributes to instances in the DOM.
/// Uses paths for unique references, IDs for non-unique (duplicate siblings).
pub fn link_referents(links: RefLinks, dom: &mut WeakDom) -> anyhow::Result<()> {
    // First, write Rojo_Id attributes to targets that need them (for ID-based refs)
    write_id_attributes(&links.targets_needing_id, dom)?;

    // Collect all instance IDs that need attributes updated
    let mut all_inst_ids: HashSet<Ref> = links.path_links.keys().copied().collect();
    all_inst_ids.extend(links.id_links.keys().copied());

    for inst_id in all_inst_ids {
        let mut path_attrs: Vec<(String, Variant)> = Vec::new();
        let mut id_attrs: Vec<(String, Variant)> = Vec::new();

        // Collect path-based refs
        if let Some(path_refs) = links.path_links.get(&inst_id) {
            for link in path_refs {
                path_attrs.push((
                    format!("{REF_PATH_ATTRIBUTE_PREFIX}{}", link.name),
                    Variant::String(link.path.clone()),
                ));
            }
        }

        // Collect ID-based refs
        if let Some(id_refs) = links.id_links.get(&inst_id) {
            for link in id_refs {
                let target = match dom.get_by_ref(link.target) {
                    Some(inst) => inst,
                    None => continue,
                };
                let id =
                    get_existing_id(target).expect("all ID-based targets should have an ID by now");
                id_attrs.push((
                    format!("{REF_POINTER_ATTRIBUTE_PREFIX}{}", link.name),
                    Variant::String(id.to_owned()),
                ));
            }
        }

        let inst = match dom.get_by_ref_mut(inst_id) {
            Some(inst) => inst,
            None => continue,
        };

        let mut attributes: Attributes = match inst.properties.remove(&ustr("Attributes")) {
            Some(Variant::Attributes(attrs)) => attrs,
            None => Attributes::new(),
            Some(value) => {
                anyhow::bail!(
                    "expected Attributes to be of type 'Attributes' but it was of type '{:?}'",
                    value.ty()
                );
            }
        }
        .into_iter()
        .filter(|(name, _)| {
            !name.starts_with(REF_PATH_ATTRIBUTE_PREFIX)
                && !name.starts_with(REF_POINTER_ATTRIBUTE_PREFIX)
        })
        .collect();

        // Add path-based refs
        for (attr_name, attr_value) in path_attrs {
            attributes.insert(attr_name, attr_value);
        }

        // Add ID-based refs
        for (attr_name, attr_value) in id_attrs {
            attributes.insert(attr_name, attr_value);
        }

        inst.properties
            .insert("Attributes".into(), attributes.into());
    }

    Ok(())
}

fn write_id_attributes(targets: &HashSet<Ref>, dom: &mut WeakDom) -> anyhow::Result<()> {
    for referent in targets {
        let inst = match dom.get_by_ref_mut(*referent) {
            Some(inst) => inst,
            None => continue,
        };

        // Skip if already has an ID
        if get_existing_id_from_props(&inst.properties).is_some() {
            continue;
        }

        let unique_id = match inst.properties.get(&ustr("UniqueId")) {
            Some(Variant::UniqueId(id)) => Some(*id),
            _ => None,
        }
        .unwrap_or_else(|| UniqueId::now().unwrap());

        let attributes = match inst.properties.get_mut(&ustr("Attributes")) {
            Some(Variant::Attributes(attrs)) => attrs,
            None => {
                inst.properties
                    .insert("Attributes".into(), Attributes::new().into());
                match inst.properties.get_mut(&ustr("Attributes")) {
                    Some(Variant::Attributes(attrs)) => attrs,
                    _ => unreachable!(),
                }
            }
            Some(value) => {
                anyhow::bail!(
                    "expected Attributes to be of type 'Attributes' but it was of type '{:?}'",
                    value.ty()
                );
            }
        };
        attributes.insert(
            REF_ID_ATTRIBUTE_NAME.into(),
            Variant::String(unique_id.to_string()),
        );
    }
    Ok(())
}

fn get_existing_id(inst: &Instance) -> Option<&str> {
    if let Variant::Attributes(attrs) = inst.properties.get(&ustr("Attributes"))? {
        let id = attrs.get(REF_ID_ATTRIBUTE_NAME)?;
        match id {
            Variant::String(str) => Some(str),
            Variant::BinaryString(bstr) => std::str::from_utf8(bstr.as_ref()).ok(),
            _ => None,
        }
    } else {
        None
    }
}

fn get_existing_id_from_props(props: &rbx_dom_weak::UstrMap<Variant>) -> Option<String> {
    if let Variant::Attributes(attrs) = props.get(&ustr("Attributes"))? {
        let id = attrs.get(REF_ID_ATTRIBUTE_NAME)?;
        match id {
            Variant::String(str) => Some(str.clone()),
            Variant::BinaryString(bstr) => std::str::from_utf8(bstr.as_ref())
                .ok()
                .map(|s| s.to_string()),
            _ => None,
        }
    } else {
        None
    }
}
