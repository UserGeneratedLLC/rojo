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

/// Checks if a path is unique in the DOM by verifying no duplicates exist
/// at ANY level of the path (target, parent, grandparent, etc.).
fn is_path_unique(dom: &WeakDom, target_ref: Ref) -> bool {
    let mut current_ref = target_ref;

    // Walk up the tree checking each level for duplicate names among siblings
    loop {
        let current = match dom.get_by_ref(current_ref) {
            Some(inst) => inst,
            None => return false,
        };

        let parent_ref = current.parent();
        if parent_ref.is_none() {
            // Reached root - path is unique at all levels
            return true;
        }

        let parent = match dom.get_by_ref(parent_ref) {
            Some(inst) => inst,
            None => return true,
        };

        // Check if any sibling has the same name as current
        let current_name = &current.name;
        let mut count = 0;
        for child_ref in parent.children() {
            if let Some(child) = dom.get_by_ref(*child_ref) {
                if &child.name == current_name {
                    count += 1;
                    if count > 1 {
                        // Duplicate found at this level - path is not unique
                        return false;
                    }
                }
            }
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
                // Target exists - check if path is unique
                if is_path_unique(dom, *target_ref) {
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
