use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
};

use anyhow::{bail, Context};
use memofs::Vfs;
use rbx_dom_weak::{
    types::{Attributes, Ref, Variant},
    ustr, HashMapExt as _, Instance, Ustr, UstrMap,
};
use rbx_reflection::ClassTag;

use crate::{
    project::{PathNode, Project, ProjectNode},
    resolution::UnresolvedValue,
    snapshot::{
        InstanceContext, InstanceMetadata, InstanceSnapshot, InstanceWithMeta, InstigatingSource,
        PathIgnoreRule, SyncRule,
    },
    snapshot_middleware::Middleware,
    syncback::{filter_properties, inst_path, FsSnapshot, SyncbackReturn, SyncbackSnapshot},
    variant_eq::variant_eq,
    RojoRef,
};

use super::snapshot_from_vfs;

/// Checks if a class transition is recoverable in clean mode.
///
/// Recoverable transitions:
/// - Folder -> Script (creates init.luau)
/// - Script -> Folder (removes init.luau)
/// - Script -> Script (changes init.server/client.luau type)
fn can_transition_class(old_class: &str, new_class: &str) -> bool {
    let is_script = |c: &str| matches!(c, "ModuleScript" | "Script" | "LocalScript");
    let is_folder = |c: &str| c == "Folder";

    // Folder can become a script (by creating init file)
    if is_folder(old_class) && is_script(new_class) {
        return true;
    }

    // Script can become Folder (by removing init file - handled by orphan removal)
    if is_script(old_class) && is_folder(new_class) {
        return true;
    }

    // Script can change type (e.g., ModuleScript -> LocalScript)
    if is_script(old_class) && is_script(new_class) {
        return true;
    }

    false
}

/// Determines the correct script-dir middleware for a Script based on its RunContext property.
///
/// Script class has a RunContext property that determines the correct middleware:
/// - Server → ServerScriptDir (init.server.luau)
/// - Client → ClientScriptDir (init.client.luau)
/// - Plugin → PluginScriptDir (init.plugin.luau)
/// - Legacy → LegacyScriptDir (init.legacy.luau)
///
/// If RunContext is not set or unrecognized, defaults to LegacyScriptDir.
fn middleware_for_script(inst: &Instance) -> Middleware {
    let run_context_enums = rbx_reflection_database::get()
        .ok()
        .and_then(|db| db.enums.get("RunContext"))
        .map(|e| &e.items);

    let run_context_value = inst.properties.get(&ustr("RunContext")).and_then(|v| match v {
        Variant::Enum(e) => Some(e.to_u32()),
        _ => None,
    });

    if let (Some(enums), Some(value)) = (run_context_enums, run_context_value) {
        for (name, &enum_value) in enums {
            if enum_value == value {
                return match name.as_ref() {
                    "Server" => Middleware::ServerScriptDir,
                    "Client" => Middleware::ClientScriptDir,
                    "Plugin" => Middleware::PluginScriptDir,
                    "Legacy" => Middleware::LegacyScriptDir,
                    _ => Middleware::LegacyScriptDir,
                };
            }
        }
    }

    // Default to LegacyScriptDir if no RunContext or unrecognized
    Middleware::LegacyScriptDir
}

pub fn snapshot_project(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let project = Project::load_exact(vfs, path, Some(name))
        .with_context(|| format!("File was not a valid Rojo project: {}", path.display()))?;
    let project_name = match project.name.as_deref() {
        Some(name) => name,
        None => panic!("Project is missing a name"),
    };

    let mut context = context.clone();
    context.clear_sync_rules();

    let rules = project.glob_ignore_paths.iter().map(|glob| PathIgnoreRule {
        glob: glob.clone(),
        base_path: project.folder_location().to_path_buf(),
    });

    let sync_rules = project.sync_rules.iter().map(|rule| SyncRule {
        base_path: project.folder_location().to_path_buf(),
        ..rule.clone()
    });

    context.add_sync_rules(sync_rules);
    context.add_path_ignore_rules(rules);

    match snapshot_project_node(&context, path, project_name, &project.tree, vfs, None)? {
        Some(found_snapshot) => {
            let mut snapshot = found_snapshot;
            // Setting the instigating source to the project file path is a little
            // coarse.
            //
            // Ideally, we'd only snapshot the project file if the project file
            // actually changed. Because Rojo only has the concept of one
            // relevant path -> snapshot path mapping per instance, we pick the more
            // conservative approach of snapshotting the project file if any
            // relevant paths changed.
            snapshot.metadata.instigating_source = Some(path.to_path_buf().into());

            // Mark this snapshot (the root node of the project file) as being
            // related to the project file.
            //
            // We SHOULD NOT mark the project file as a relevant path for any
            // nodes that aren't roots. They'll be updated as part of the project
            // file being updated.
            snapshot.metadata.relevant_paths.push(path.to_path_buf());

            Ok(Some(snapshot))
        }
        None => Ok(None),
    }
}

pub fn snapshot_project_node(
    context: &InstanceContext,
    project_path: &Path,
    instance_name: &str,
    node: &ProjectNode,
    vfs: &Vfs,
    parent_class: Option<&str>,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let project_folder = project_path.parent().unwrap();

    let mut class_name_from_path = None;

    let name = Cow::Owned(instance_name.to_owned());
    let mut properties = UstrMap::new();
    let mut children = Vec::new();
    let mut metadata = InstanceMetadata::new().context(context);

    if let Some(path_node) = &node.path {
        let path = path_node.path();

        // If the path specified in the project is relative, we assume it's
        // relative to the folder that the project is in, project_folder.
        let full_path = if path.is_relative() {
            Cow::Owned(project_folder.join(path))
        } else {
            Cow::Borrowed(path)
        };

        if let Some(snapshot) = snapshot_from_vfs(context, vfs, &full_path)? {
            class_name_from_path = Some(snapshot.class_name);

            // Properties from the snapshot are pulled in unchanged, and
            // overridden by properties set on the project node.
            properties.reserve(snapshot.properties.len());
            for (key, value) in snapshot.properties.into_iter() {
                properties.insert(key, value);
            }

            // The snapshot's children will be merged with the children defined
            // in the project node, if there are any.
            children.reserve(snapshot.children.len());
            for child in snapshot.children.into_iter() {
                children.push(child);
            }

            // Take the snapshot's metadata as-is, which will be mutated later
            // on.
            metadata = snapshot.metadata;
        }
    }

    let class_name_from_inference = infer_class_name(&name, parent_class);

    let class_name = match (
        node.class_name,
        class_name_from_path,
        class_name_from_inference,
        &node.path,
    ) {
        // These are the easy, happy paths!
        (Some(project), None, None, _) => project,
        (None, Some(path), None, _) => path,
        (None, None, Some(inference), _) => inference,

        // If the user specifies a class name, but there's an inferred class
        // name, we prefer the name listed explicitly by the user.
        (Some(project), None, Some(_), _) => project,

        // If the user has a $path pointing to a folder and we're able to infer
        // a class name, let's use the inferred name. If the path we're pointing
        // to isn't a folder, though, that's a user error.
        (None, Some(path), Some(inference), _) => {
            if path == "Folder" {
                inference
            } else {
                path
            }
        }

        (Some(project), Some(path), _, _) => {
            if path == "Folder" {
                project
            } else {
                bail!(
                    "ClassName for Instance \"{}\" was specified in both the project file (as \"{}\") and from the filesystem (as \"{}\").\n\
                     If $className and $path are both set, $path must refer to a Folder.
                     \n\
                     Project path: {}\n\
                     Filesystem path: {}\n",
                    instance_name,
                    project,
                    path,
                    project_path.display(),
                    node.path.as_ref().unwrap().path().display()
                );
            }
        }

        (None, None, None, Some(PathNode::Optional(_))) => {
            return Ok(None);
        }

        (_, None, _, Some(PathNode::Required(path))) => {
            anyhow::bail!(
                "Rojo project referred to a file using $path that could not be turned into a Roblox Instance by Rojo.\n\
                Check that the file exists and is a file type known by Rojo.\n\
                \n\
                Project path: {}\n\
                File $path: {}",
                project_path.display(),
                path.display(),
            );
        }

        (None, None, None, None) => {
            bail!(
                "Instance \"{}\" is missing some required information.\n\
                 One of the following must be true:\n\
                 - $className must be set to the name of a Roblox class\n\
                 - $path must be set to a path of an instance\n\
                 - The instance must be a known service, like ReplicatedStorage\n\
                 \n\
                 Project path: {}",
                instance_name,
                project_path.display(),
            );
        }
    };

    for (child_name, child_project_node) in &node.children {
        if let Some(child) = snapshot_project_node(
            context,
            project_path,
            child_name,
            child_project_node,
            vfs,
            Some(&class_name),
        )? {
            children.push(child);
        }
    }

    for (key, unresolved) in &node.properties {
        let value = unresolved
            .clone()
            .resolve(&class_name, key)
            .with_context(|| {
                format!(
                    "Unresolvable property in project at path {}",
                    project_path.display()
                )
            })?;

        match key.as_str() {
            "Name" | "Parent" => {
                log::warn!(
                    "Property '{}' cannot be set manually, ignoring. Attempted to set in '{}' at {}",
                    key,
                    instance_name,
                    project_path.display()
                );
                continue;
            }

            _ => {}
        }

        properties.insert(*key, value);
    }

    if !node.attributes.is_empty() {
        let mut attributes = Attributes::new();

        for (key, unresolved) in &node.attributes {
            let value = unresolved.clone().resolve_unambiguous().with_context(|| {
                format!(
                    "Unresolvable attribute in project at path {}",
                    project_path.display()
                )
            })?;

            attributes.insert(key.clone(), value);
        }

        properties.insert("Attributes".into(), attributes.into());
    }

    // If the user specified $ignoreUnknownInstances, overwrite the existing
    // value.
    //
    // If the user didn't specify it AND $path was not specified (meaning
    // there's no existing value we'd be stepping on from a project file or meta
    // file), set it to true.
    if let Some(ignore) = node.ignore_unknown_instances {
        metadata.ignore_unknown_instances = ignore;
    } else if node.path.is_none() {
        // TODO: Introduce a strict mode where $ignoreUnknownInstances is never
        // set implicitly.
        metadata.ignore_unknown_instances = true;
    }

    if let Some(id) = &node.id {
        metadata.specified_id = Some(RojoRef::new(id.clone()))
    }

    metadata.instigating_source = Some(InstigatingSource::ProjectNode {
        path: project_path.to_path_buf(),
        name: instance_name.to_string(),
        node: node.clone(),
        parent_class: parent_class.map(|name| name.to_owned()),
    });

    Ok(Some(InstanceSnapshot {
        snapshot_id: Ref::none(),
        name,
        class_name,
        properties,
        children,
        metadata,
    }))
}

pub fn syncback_project<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let old_inst = snapshot
        .old_inst()
        .expect("projects should always exist in both trees");
    // Generally, the path of a project is the first thing added to the relevant
    // paths. So, we take the last one.
    let project_path = old_inst
        .metadata()
        .relevant_paths
        .last()
        .expect("all projects should have a relevant path");
    let vfs = snapshot.vfs();

    log::debug!("Reloading project {} from vfs", project_path.display(),);
    let mut project = Project::load_exact(vfs, project_path, None)?;
    let base_path = project.folder_location().to_path_buf();

    // Sync rules for this project do not have their base rule set but it is
    // important when performing syncback on other projects.
    for rule in &mut project.sync_rules {
        rule.base_path.clone_from(&base_path)
    }

    let mut descendant_snapshots = Vec::new();
    let mut removed_descendants = Vec::new();

    let mut ref_to_path_map = HashMap::new();
    let mut old_child_map = HashMap::new();
    let mut new_child_map = HashMap::new();

    let mut node_changed_map = Vec::new();
    let mut node_queue = VecDeque::with_capacity(1);
    node_queue.push_back((&mut project.tree, old_inst, snapshot.new_inst()));

    while let Some((node, old_inst, new_inst)) = node_queue.pop_front() {
        log::debug!("Processing node {}", old_inst.name());
        if old_inst.class_name() != new_inst.class {
            // In clean mode, allow recoverable class transitions
            // (e.g., Folder -> ModuleScript by creating init.luau)
            let can_recover =
                can_transition_class(old_inst.class_name().as_str(), new_inst.class.as_str());
            if !snapshot.data.is_incremental() && can_recover {
                log::debug!(
                    "Clean mode: allowing class transition {} -> {} for {}",
                    old_inst.class_name(),
                    new_inst.class,
                    old_inst.name()
                );
            } else {
                anyhow::bail!(
                    "Cannot change the class of {} in project file {}.\n\
                    Current class is {}, it is a {} in the input file.",
                    old_inst.name(),
                    project_path.display(),
                    old_inst.class_name(),
                    new_inst.class
                );
            }
        }

        // TODO handle meta.json5 files in this branch. Right now, we perform
        // syncback if a node has `$path` set but the Middleware aren't aware
        // that the Instances they're running on originate in a project.json5.
        // As a result, the `meta.json5` syncback code is hardcoded to not work
        // if the Instance originates from a project file. However, we should
        // ideally use a .meta.json5 over the project node if it exists already.
        if node.path.is_some() {
            // Since the node has a path, we have to run syncback on it.
            let node_path = node.path.as_ref().map(PathNode::path).expect(
                "Project nodes with a path must have a path \
                If you see this message, something went seriously wrong. Please report it.",
            );
            let full_path = if node_path.is_absolute() {
                node_path.to_path_buf()
            } else {
                base_path.join(node_path)
            };

            let mut middleware = match Middleware::middleware_for_path(
                snapshot.vfs(),
                &project.sync_rules,
                &full_path,
            )? {
                Some(middleware) => middleware,
                None => {
                    // Path doesn't exist on filesystem. In clean mode, we can
                    // determine the middleware from the new instance and let
                    // syncback create the necessary files/directories.
                    if !snapshot.data.is_incremental() {
                        // Determine middleware based on new instance class (and RunContext for Scripts)
                        let inferred_middleware = match new_inst.class.as_str() {
                            "ModuleScript" => Middleware::ModuleScriptDir,
                            "Script" => middleware_for_script(new_inst),
                            "LocalScript" => Middleware::LocalScriptDir,
                            "Folder" => Middleware::Dir,
                            // For other classes, default to Dir
                            _ => Middleware::Dir,
                        };
                        log::debug!(
                            "Clean mode: path {} doesn't exist, inferring {:?} middleware for class {}",
                            full_path.display(),
                            inferred_middleware,
                            new_inst.class
                        );
                        inferred_middleware
                    } else {
                        anyhow::bail!(
                            "Rojo project referred to a file using $path that could not be turned \
                            into a Roblox Instance by Rojo.\n\
                            File $path: {}",
                            full_path.display()
                        )
                    }
                }
            };

            // In clean mode, we may need to override middleware when the filesystem
            // structure doesn't match the new instance type.
            if !snapshot.data.is_incremental() {
                // Case 1: Filesystem has a directory (Dir middleware) but new instance
                // is a script - use script-dir middleware to create the init file.
                if middleware == Middleware::Dir {
                    let script_middleware = match new_inst.class.as_str() {
                        "ModuleScript" => Some(Middleware::ModuleScriptDir),
                        "Script" => Some(middleware_for_script(new_inst)),
                        "LocalScript" => Some(Middleware::LocalScriptDir),
                        _ => None,
                    };
                    if let Some(new_middleware) = script_middleware {
                        log::debug!(
                            "Clean mode: overriding Dir->{:?} middleware for {} (class {})",
                            new_middleware,
                            full_path.display(),
                            new_inst.class
                        );
                        middleware = new_middleware;
                    }
                }

                // Case 2: Filesystem has a script-dir (init.luau exists) but new instance
                // is a Folder - use Dir middleware. The init file will be removed as orphan.
                let is_script_dir_middleware = matches!(
                    middleware,
                    Middleware::ModuleScriptDir
                        | Middleware::ServerScriptDir
                        | Middleware::ClientScriptDir
                        | Middleware::LocalScriptDir
                        | Middleware::PluginScriptDir
                        | Middleware::LegacyScriptDir
                );
                if is_script_dir_middleware && new_inst.class == "Folder" {
                    log::debug!(
                        "Clean mode: overriding {:?}->Dir middleware for {} (class Folder)",
                        middleware,
                        full_path.display()
                    );
                    middleware = Middleware::Dir;
                }

                // Case 3: Filesystem has a script-dir but new instance is a DIFFERENT script type.
                // e.g., ModuleScriptDir (init.luau) but new instance is Script → use correct script-dir.
                // Without this, changing a ModuleScript to Script in Studio would silently keep
                // the old init.luau file, causing the script to be read as ModuleScript on next build.
                // For Scripts, we also check RunContext to pick the correct middleware (Server/Client/Plugin/Legacy).
                if is_script_dir_middleware {
                    let correct_middleware = match new_inst.class.as_str() {
                        "ModuleScript" => Some(Middleware::ModuleScriptDir),
                        "Script" => Some(middleware_for_script(new_inst)),
                        "LocalScript" => Some(Middleware::LocalScriptDir),
                        _ => None,
                    };
                    if let Some(new_middleware) = correct_middleware {
                        if middleware != new_middleware {
                            log::debug!(
                                "Clean mode: overriding {:?}->{:?} middleware for {} (class {} changed)",
                                middleware,
                                new_middleware,
                                full_path.display(),
                                new_inst.class
                            );
                            middleware = new_middleware;
                        }
                    }
                }
            }

            descendant_snapshots.push(
                snapshot
                    .with_new_path(full_path.clone(), new_inst.referent(), Some(old_inst.id()))
                    .middleware(middleware),
            );

            ref_to_path_map.insert(new_inst.referent(), full_path);

            // We only want to set properties if it needs it.
            if !middleware.handles_own_properties() {
                project_node_property_syncback_path(snapshot, new_inst, node);
            }
        } else {
            project_node_property_syncback_no_path(snapshot, new_inst, node);
        }

        for child_ref in new_inst.children() {
            let child = snapshot
                .get_new_instance(*child_ref)
                .expect("all children of Instances should be in new DOM");
            if new_child_map.insert(&child.name, child).is_some() {
                let parent_path = inst_path(snapshot.new_tree(), new_inst.referent());
                anyhow::bail!(
                    "Instances that are direct children of an Instance that is made by a project file \
                    must have a unique name.\nThe child '{}' is duplicated in the place file.\n\
                    Full path: {}/{}", child.name, parent_path, child.name
                );
            }
        }
        for child_ref in old_inst.children() {
            let child = snapshot
                .get_old_instance(*child_ref)
                .expect("all children of Instances should be in old DOM");
            if old_child_map.insert(child.name(), child).is_some() {
                let parent_path = inst_path(snapshot.old_tree(), old_inst.id());
                anyhow::bail!(
                    "Instances that are direct children of an Instance that is made by a project file \
                    must have a unique name.\nThe child '{}' is duplicated on the file system.\n\
                    Full path: {}/{}", child.name(), parent_path, child.name()
                );
            }
        }

        // This loop does basic matching of Instance children to the node's
        // children. It ensures that `new_child_map` and `old_child_map` will
        // only contain Instances that don't belong to the project after this.
        for (child_name, child_node) in &mut node.children {
            // If a node's path is optional, we want to skip it if the path
            // doesn't exist since it isn't in the current old DOM.
            if let Some(path) = &child_node.path {
                if path.is_optional() {
                    let real_path = if path.path().is_absolute() {
                        path.path().to_path_buf()
                    } else {
                        base_path.join(path.path())
                    };
                    if !real_path.exists() {
                        log::warn!(
                            "Skipping node '{child_name}' of project because it is optional and not present on the disk.\n\
                            If this is not deliberate, please create a file or directory at {}", real_path.display()
                        );
                        continue;
                    }
                }
            }
            let new_equivalent = new_child_map.remove(child_name);
            let old_equivalent = old_child_map.remove(child_name.as_str());
            match (new_equivalent, old_equivalent) {
                (Some(new), Some(old)) => node_queue.push_back((child_node, old, new)),
                (_, None) => anyhow::bail!(
                    "The child '{child_name}' of Instance '{}' would be removed.\n\
                    Syncback cannot add or remove Instances from project {}",
                    old_inst.name(),
                    project_path.display()
                ),
                (None, _) => anyhow::bail!(
                    "The child '{child_name}' of Instance '{}' is present only in a project file,\n\
                    and not the provided file. Syncback cannot add or remove Instances from project:\n{}.",
                    old_inst.name(), project_path.display(),
                )
            }
        }

        // All of the children in this loop are by their nature not in the
        // project, so we just need to run syncback on them.
        for (name, new_child) in new_child_map.drain() {
            // Skip instances of ignored classes
            if snapshot.should_ignore_class(&new_child.class) {
                // Also remove from old_child_map so it won't be marked as removed
                old_child_map.remove(name.as_str());
                log::debug!(
                    "Skipping instance {} because its class {} is ignored",
                    new_child.name,
                    new_child.class
                );
                continue;
            }

            let parent_path = match ref_to_path_map.get(&new_child.parent()) {
                Some(path) => path.clone(),
                None => {
                    log::debug!("Skipping child {name} of node because it has no parent_path");
                    continue;
                }
            };

            // If a child also exists in the old tree, it will be caught in the
            // syncback on the project node path above (or is itself a node).
            // So the only things we need to run seperately is new children.
            if old_child_map.remove(name.as_str()).is_none() {
                let parent_middleware =
                    Middleware::middleware_for_path(vfs, &project.sync_rules, &parent_path)?
                        .expect("project nodes should have a middleware if they have children.");
                // If this node points directly to a project, it may still have
                // children but they'll be handled by syncback. This isn't a
                // concern with directories because they're singular things,
                // files that contain their own children.
                if parent_middleware != Middleware::Project {
                    descendant_snapshots.push(snapshot.with_base_path(
                        &parent_path,
                        new_child.referent(),
                        None,
                    )?);
                }
            }
        }
        // Filter out instances of ignored classes from removal
        removed_descendants.extend(old_child_map.drain().filter_map(|(_, inst)| {
            if snapshot.should_ignore_class(inst.class_name().as_str()) {
                log::debug!(
                    "Not removing instance {} because its class {} is ignored",
                    inst.name(),
                    inst.class_name()
                );
                None
            } else {
                Some(inst)
            }
        }));
        node_changed_map.push((&node.properties, &node.attributes, old_inst))
    }
    let mut fs_snapshot = FsSnapshot::new();

    for (node_properties, node_attributes, old_inst) in node_changed_map {
        if project_node_should_reserialize(node_properties, node_attributes, old_inst)? {
            fs_snapshot.add_file(project_path, crate::json::to_vec_pretty_sorted(&project)?);
            break;
        }
    }

    Ok(SyncbackReturn {
        fs_snapshot,
        children: descendant_snapshots,
        removed_children: removed_descendants,
    })
}

fn project_node_property_syncback(
    _snapshot: &SyncbackSnapshot,
    filtered_properties: UstrMap<&Variant>,
    new_inst: &Instance,
    node: &mut ProjectNode,
) {
    let properties = &mut node.properties;
    let mut attributes = BTreeMap::new();
    for (name, value) in filtered_properties {
        match value {
            Variant::Attributes(attrs) => {
                for (attr_name, attr_value) in attrs.iter() {
                    // We (probably) don't want to preserve internal attributes,
                    // only user defined ones.
                    if attr_name.starts_with("RBX") {
                        continue;
                    }
                    attributes.insert(
                        attr_name.clone(),
                        UnresolvedValue::from_variant_unambiguous(attr_value.clone()),
                    );
                }
            }
            _ => {
                properties.insert(
                    name,
                    UnresolvedValue::from_variant(value.clone(), &new_inst.class, &name),
                );
            }
        }
    }
    node.attributes = attributes;
}

fn project_node_property_syncback_path(
    snapshot: &SyncbackSnapshot,
    new_inst: &Instance,
    node: &mut ProjectNode,
) {
    let filtered_properties = snapshot
        .get_path_filtered_properties(new_inst.referent())
        .unwrap();
    project_node_property_syncback(snapshot, filtered_properties, new_inst, node)
}

fn project_node_property_syncback_no_path(
    snapshot: &SyncbackSnapshot,
    new_inst: &Instance,
    node: &mut ProjectNode,
) {
    let filtered_properties = filter_properties(snapshot.project(), new_inst);
    project_node_property_syncback(snapshot, filtered_properties, new_inst, node)
}

fn project_node_should_reserialize(
    node_properties: &BTreeMap<Ustr, UnresolvedValue>,
    node_attributes: &BTreeMap<String, UnresolvedValue>,
    instance: InstanceWithMeta,
) -> anyhow::Result<bool> {
    for (prop_name, unresolved_node_value) in node_properties {
        if let Some(inst_value) = instance.properties().get(prop_name) {
            let node_value = unresolved_node_value
                .clone()
                .resolve(&instance.class_name(), prop_name)?;
            if !variant_eq(inst_value, &node_value) {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }

    match instance.properties().get(&ustr("Attributes")) {
        Some(Variant::Attributes(inst_attributes)) => {
            // This will also catch if one is empty but the other isn't
            if node_attributes.len() != inst_attributes.len() {
                Ok(true)
            } else {
                for (attr_name, unresolved_node_value) in node_attributes {
                    if let Some(inst_value) = inst_attributes.get(attr_name.as_str()) {
                        let node_value = unresolved_node_value.clone().resolve_unambiguous()?;
                        if !variant_eq(inst_value, &node_value) {
                            return Ok(true);
                        }
                    } else {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
        Some(_) => Ok(true),
        None => {
            if !node_attributes.is_empty() {
                Ok(true)
            } else {
                Ok(false)
            }
        }
    }
}

fn infer_class_name(name: &str, parent_class: Option<&str>) -> Option<Ustr> {
    // If className wasn't defined from another source, we may be able
    // to infer one.

    let parent_class = parent_class?;

    if parent_class == "DataModel" {
        // Members of DataModel with names that match known services are
        // probably supposed to be those services.

        let descriptor = rbx_reflection_database::get().unwrap().classes.get(name)?;

        if descriptor.tags.contains(&ClassTag::Service) {
            return Some(ustr(name));
        }
    } else if parent_class == "StarterPlayer" {
        // StarterPlayer has two special members with their own classes.

        if name == "StarterPlayerScripts" || name == "StarterCharacterScripts" {
            return Some(ustr(name));
        }
    } else if parent_class == "Workspace" {
        // Workspace has a special Terrain class inside it
        if name == "Terrain" {
            return Some(ustr(name));
        }
    }

    None
}

// #[cfg(feature = "broken-tests")]
#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[ignore = "Functionality moved to root snapshot middleware"]
    #[test]
    fn project_from_folder() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "default.project.json5",
                VfsSnapshot::file(
                    r#"
                    {
                        "name": "indirect-project",
                        "tree": {
                            "$className": "Folder"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_from_direct_file() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "hello.project.json5",
                VfsSnapshot::file(
                    r#"
                    {
                        "name": "direct-project",
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/hello.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_resolved_properties() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json5",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "resolved-properties",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": {
                                    "String": "Hello, world!"
                                }
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_unresolved_properties() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json5",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "unresolved-properties",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": "Hi!"
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_children() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.project.json5",
            VfsSnapshot::file(
                r#"
                    {
                        "name": "children",
                        "tree": {
                            "$className": "Folder",

                            "Child": {
                                "$className": "Model"
                            }
                        }
                    }
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_txt() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-project",
                        "tree": {
                            "$path": "other.txt"
                        }
                    }
                "#,
                    ),
                ),
                ("other.txt", VfsSnapshot::file("Hello, world!")),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_project() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-project",
                        "tree": {
                            "$path": "other.project.json5"
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn project_with_path_to_project_with_children() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-child-project",
                        "tree": {
                            "$path": "other.project.json5"
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "Folder",

                            "SomeChild": {
                                "$className": "Model"
                            }
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    /// Ensures that if a property is defined both in the resulting instance
    /// from $path and also in $properties, that the $properties value takes
    /// precedence.
    #[test]
    fn project_path_property_overrides() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([
                (
                    "default.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "path-property-override",
                        "tree": {
                            "$path": "other.project.json5",
                            "$properties": {
                                "Value": "Changed"
                            }
                        }
                    }
                "#,
                    ),
                ),
                (
                    "other.project.json5",
                    VfsSnapshot::file(
                        r#"
                    {
                        "name": "other-project",
                        "tree": {
                            "$className": "StringValue",
                            "$properties": {
                                "Value": "Original"
                            }
                        }
                    }
                "#,
                    ),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json5"),
            "NOT_IN_SNAPSHOT",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn no_name_project() {
        let _ = env_logger::try_init();

        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([(
                "default.project.json5",
                VfsSnapshot::file(
                    r#"
                    {
                        "tree": {
                            "$className": "Model"
                        }
                    }
                "#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_project(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo/default.project.json5"),
            "no_name_project",
        )
        .expect("snapshot error")
        .expect("snapshot returned no instances");

        insta::assert_yaml_snapshot!(instance_snapshot);
    }
}
