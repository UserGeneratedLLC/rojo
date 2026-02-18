//! Implements iterating through an entire WeakDom and linking all Ref
//! properties using path-based attributes (preferred) or ID-based attributes
//! (fallback for non-unique paths).

use std::collections::{HashMap, HashSet, VecDeque};

use rbx_dom_weak::{
    types::{Attributes, Ref, UniqueId, Variant},
    ustr, Instance, Ustr, WeakDom,
};

use crate::{
    ref_attribute_name, ref_target_attribute_name,
    syncback::{name_needs_slugify, slugify_name},
    REF_ID_ATTRIBUTE_NAME, REF_PATH_ATTRIBUTE_PREFIX, REF_POINTER_ATTRIBUTE_PREFIX,
};

/// Public wrapper for `tentative_fs_path` used by the post-processing step
/// in `syncback_loop` to compare tentative vs final paths.
pub fn tentative_fs_path_public(dom: &WeakDom, target_ref: Ref) -> String {
    tentative_fs_path(dom, target_ref)
}

/// Compute a filesystem-name-compatible path for an instance in a bare WeakDom.
///
/// Each path segment is a **tentative filesystem name**: the slugified instance
/// name plus the extension that the syncback middleware would assign based on
/// class and children. This produces paths like `"Workspace/Hey_Bro.server.luau"`
/// that are compatible with `get_instance_by_path()` (which resolves using
/// filesystem names).
///
/// For directory-style instances (Folders, services, scripts with children),
/// the segment is just the slugified name (no extension).
fn tentative_fs_path(dom: &WeakDom, target_ref: Ref) -> String {
    let root_ref = dom.root_ref();
    let mut components: Vec<String> = Vec::new();
    let mut current = target_ref;

    loop {
        if current == root_ref || current.is_none() {
            break;
        }

        let inst = match dom.get_by_ref(current) {
            Some(i) => i,
            None => break,
        };

        let segment = tentative_fs_name(inst);
        components.push(segment);
        current = inst.parent();
    }

    components.reverse();
    components.join("/")
}

/// Compute the tentative filesystem name for a single instance based on its
/// class, RunContext property, and whether it has children.
///
/// This mirrors the logic in `get_best_middleware()` + `extension_for_middleware()`
/// but operates on a bare Instance without SyncbackSnapshot context.
fn tentative_fs_name(inst: &Instance) -> String {
    let slug = if name_needs_slugify(&inst.name) {
        slugify_name(&inst.name)
    } else {
        inst.name.clone()
    };

    let has_children = !inst.children().is_empty();

    // Directory-style classes never get extensions
    let is_container = matches!(
        inst.class.as_str(),
        "Folder" | "Configuration" | "Tool" | "ScreenGui" | "SurfaceGui" | "BillboardGui" | "AdGui"
    );

    if is_container || (has_children && is_script_class(inst.class.as_str())) {
        // Directory representation -- slug only, no extension
        return slug;
    }

    let extension = match inst.class.as_str() {
        "Script" => {
            if has_children {
                return slug; // directory
            }
            match inst.properties.get(&ustr("RunContext")) {
                Some(Variant::Enum(e)) => match e.to_u32() {
                    0 => "legacy.luau",
                    1 => "server.luau",
                    2 => "client.luau",
                    3 => "plugin.luau",
                    _ => "legacy.luau",
                },
                _ => "legacy.luau",
            }
        }
        "LocalScript" => {
            if has_children {
                return slug;
            }
            "local.luau"
        }
        "ModuleScript" => {
            if has_children {
                return slug;
            }
            "luau"
        }
        "StringValue" => {
            if has_children {
                return slug;
            }
            "txt"
        }
        "LocalizationTable" => {
            if has_children {
                return slug;
            }
            "csv"
        }
        _ => {
            if has_children {
                return slug; // directory
            }
            "model.json5"
        }
    };

    format!("{slug}.{extension}")
}

fn is_script_class(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

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
/// Returns a map of Ref -> filesystem-name-compatible path for all instances.
pub fn collect_all_paths(dom: &WeakDom) -> HashMap<Ref, String> {
    let mut paths = HashMap::new();
    let mut queue = VecDeque::new();
    queue.push_back(dom.root_ref());

    while let Some(inst_ref) = queue.pop_front() {
        let inst = dom.get_by_ref(inst_ref).unwrap();
        queue.extend(inst.children().iter().copied());
        paths.insert(inst_ref, tentative_fs_path(dom, inst_ref));
    }

    paths
}

/// Iterates through a WeakDom and collects referent properties.
/// Uses paths when unique, falls back to IDs when paths are ambiguous.
///
/// The `pre_prune_paths` parameter should contain paths for instances that may
/// have been pruned from the DOM. This allows references to instances outside
/// the sync tree to be preserved as path-based attributes.
///
/// The `final_paths` parameter, when provided, contains the definitive
/// filesystem-name-based paths assigned during the syncback walk (including
/// dedup suffixes like `~2`). These take priority over `tentative_fs_path()`.
pub fn collect_referents(
    dom: &WeakDom,
    pre_prune_paths: &HashMap<Ref, String>,
    final_paths: Option<&HashMap<Ref, String>>,
) -> RefLinks {
    let mut path_links: HashMap<Ref, Vec<PathRefLink>> = HashMap::new();
    let id_links: HashMap<Ref, Vec<IdRefLink>> = HashMap::new();
    let targets_needing_id: HashSet<Ref> = HashSet::new();

    // NOTE: tentative_fs_path() does not include dedup suffixes (~N). For
    // instances with duplicate names under the same parent, this produces
    // ambiguous paths. Resolution via get_instance_by_path() returns the
    // first matching child, which may be wrong.
    //
    // When `final_paths` is provided (from the syncback walk), it is used
    // instead of tentative_fs_path() to get the correct dedup'd path.
    // Otherwise, falls back to tentative_fs_path() (pre-dedup, potentially
    // ambiguous for duplicate-named instances).

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

            if dom.get_by_ref(*target_ref).is_some() {
                // Target exists in DOM -- use path-based system.
                // Prefer final_paths (dedup-aware) when available,
                // fall back to tentative_fs_path (pre-dedup).
                let target_path = final_paths
                    .and_then(|fp| fp.get(target_ref).cloned())
                    .unwrap_or_else(|| tentative_fs_path(dom, *target_ref));
                path_links.entry(inst_ref).or_default().push(PathRefLink {
                    name: *prop_name,
                    path: target_path,
                });
            } else if let Some(external_path) = pre_prune_paths.get(target_ref) {
                // Target was pruned -- use pre-prune path (already in
                // filesystem-name format from collect_all_paths)
                log::debug!(
                    "Property {}.{} points to pruned instance at '{}', storing as path reference",
                    tentative_fs_path(dom, inst_ref),
                    prop_name,
                    external_path
                );
                path_links.entry(inst_ref).or_default().push(PathRefLink {
                    name: *prop_name,
                    path: external_path.clone(),
                });
            } else {
                log::warn!(
                    "Property {}.{} will be `nil` on disk because the referenced instance does not exist",
                    tentative_fs_path(dom, inst_ref),
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
                    ref_attribute_name(&link.name),
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
                    ref_target_attribute_name(&link.name),
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
