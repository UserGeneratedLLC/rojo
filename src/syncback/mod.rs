pub mod dedup_suffix;
mod file_names;
mod fs_snapshot;
mod hash;
pub mod matching;
pub mod meta;
mod property_filter;
mod ref_properties;
mod snapshot;
mod stats;

use anyhow::Context;
use indexmap::IndexMap;
use memofs::Vfs;
use rbx_dom_weak::{
    types::{Ref, Variant},
    ustr, Instance, Ustr, UstrSet, WeakDom,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{
    glob::Glob,
    snapshot::{InstanceWithMeta, RojoTree},
    snapshot_middleware::Middleware,
    syncback::ref_properties::{collect_all_paths, collect_referents, link_referents},
    Project,
};

pub use file_names::{
    adjacent_meta_path, deduplicate_name, extension_for_middleware, name_for_inst,
    name_needs_slugify, slugify_name, strip_middleware_extension, strip_script_suffix,
    validate_file_name,
};
pub use fs_snapshot::FsSnapshot;
pub use hash::*;
pub use property_filter::{
    filter_properties, filter_properties_preallocated, should_property_serialize,
};
pub use snapshot::{inst_path, SyncbackData, SyncbackSnapshot};
pub use stats::SyncbackStats;

/// Result of a syncback operation, containing everything needed for
/// post-processing (file writes, sourcemap generation, etc.).
pub struct SyncbackResult {
    /// Filesystem operations to perform (file writes, directory creation, removals).
    pub fs_snapshot: FsSnapshot,
    /// The new instance tree from the Roblox file, after pruning and filtering.
    /// Returned so callers can build sourcemaps without re-reading from disk.
    pub new_tree: WeakDom,
    /// Maps each instance Ref (in `new_tree`) to the file paths written for it.
    /// Used to generate sourcemaps from in-memory data.
    pub instance_paths: HashMap<Ref, Vec<PathBuf>>,
}

/// The name of an enviroment variable to use to override the behavior of
/// syncback on model files.
/// By default, syncback will use `Rbxm` for model files.
/// If this is set to `1`, it will instead use `Rbxmx`. If it is set to `2`,
/// it will use `JsonModel`.
///
/// This will **not** override existing `Rbxm` middleware. It will only impact
/// new files.
const DEBUG_MODEL_FORMAT_VAR: &str = "ROJO_SYNCBACK_DEBUG";

/// Services that are considered "visible" and will be included when
/// `ignoreHiddenServices` is enabled. All other services will be ignored.
pub const VISIBLE_SERVICES: &[&str] = &[
    "Lighting",
    "MaterialService",
    "ReplicatedFirst",
    "ReplicatedStorage",
    "ServerScriptService",
    "ServerStorage",
    "SoundService",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "Teams",
    "TextChatService",
    "VoiceChatService",
    "Workspace",
];

/// A glob that can be used to tell if a path contains a `.git` folder.
static GIT_IGNORE_GLOB: OnceLock<Glob> = OnceLock::new();

pub fn syncback_loop(
    vfs: &Vfs,
    old_tree: &mut RojoTree,
    new_tree: WeakDom,
    project: &Project,
    incremental: bool,
) -> anyhow::Result<SyncbackResult> {
    syncback_loop_with_stats(vfs, old_tree, new_tree, project, incremental, None)
}

/// Runs the syncback loop with an optional external stats tracker.
/// If no stats are provided, an internal one is created and logged at the end.
pub fn syncback_loop_with_stats(
    vfs: &Vfs,
    old_tree: &mut RojoTree,
    mut new_tree: WeakDom,
    project: &Project,
    incremental: bool,
    external_stats: Option<&SyncbackStats>,
) -> anyhow::Result<SyncbackResult> {
    // Create internal stats if not provided externally
    let internal_stats = SyncbackStats::new();
    let stats = external_stats.unwrap_or(&internal_stats);

    let ignore_patterns = project
        .syncback_rules
        .as_ref()
        .map(|rules| rules.compile_globs())
        .transpose()?;

    let tree_globs = project
        .syncback_rules
        .as_ref()
        .map(|rules| rules.compile_tree_globs())
        .transpose()?;

    let phase_timer = std::time::Instant::now();

    // Collect all instance paths BEFORE pruning so we can track external references
    // (references to instances that will be pruned, like SoundGroups in SoundService).
    log::debug!("Collecting instance paths before pruning...");
    let pre_prune_paths = collect_all_paths(&new_tree);
    log::debug!(
        "syncback: collect_all_paths in {:.1?}",
        phase_timer.elapsed()
    );

    // TODO: Add a better way to tell if the root of a project is a directory
    let skip_pruning = if let Some(path) = &project.tree.path {
        let resolved = if path.path().is_absolute() {
            path.path().to_path_buf()
        } else {
            project.folder_location().join(path.path())
        };
        let middleware =
            Middleware::middleware_for_path(vfs, &project.sync_rules, &resolved).unwrap();
        if let Some(middleware) = middleware {
            middleware.is_dir()
        } else {
            false
        }
    } else {
        false
    };
    let phase_timer = std::time::Instant::now();
    if !skip_pruning {
        strip_unknown_root_children(&mut new_tree, old_tree);
    }

    let ignore_hidden = project
        .ignore_hidden_services
        .or_else(|| {
            project
                .syncback_rules
                .as_ref()
                .map(|rules| rules.ignore_hidden_services())
        })
        .unwrap_or(true);
    if ignore_hidden {
        strip_hidden_services(&mut new_tree);
    }
    log::debug!("syncback: prune + filter in {:.1?}", phase_timer.elapsed());

    let phase_timer = std::time::Instant::now();
    let mut deferred_referents = collect_referents(&new_tree, &pre_prune_paths, None);
    let placeholder_map = std::mem::take(&mut deferred_referents.placeholder_to_source_and_target);
    log::debug!(
        "syncback: collect_referents in {:.1?}",
        phase_timer.elapsed()
    );

    let phase_timer = std::time::Instant::now();
    for referent in descendants(&new_tree, new_tree.root_ref()) {
        let new_inst = new_tree.get_by_ref_mut(referent).unwrap();
        if let Some(filter) = get_property_filter(project, new_inst) {
            for prop in filter {
                new_inst.properties.remove(&prop);
            }
        }
    }
    for referent in descendants(old_tree.inner(), old_tree.get_root_id()) {
        let mut old_inst_rojo = old_tree.get_instance_mut(referent).unwrap();
        let old_inst = old_inst_rojo.inner_mut();
        if let Some(filter) = get_property_filter(project, old_inst) {
            for prop in filter {
                old_inst.properties.remove(&prop);
            }
        }
    }

    // Handle removing the current camera.
    // syncCurrentCamera defaults to false, meaning we remove the camera by default
    let sync_current_camera = project
        .syncback_rules
        .as_ref()
        .and_then(|s| s.sync_current_camera)
        .unwrap_or(false);
    if !sync_current_camera {
        log::debug!("Removing CurrentCamera from new DOM");
        let mut workspace_ref = None;
        let mut camera_target = None;
        for child_ref in new_tree.root().children() {
            let inst = new_tree.get_by_ref(*child_ref).unwrap();
            if inst.class == "Workspace" {
                workspace_ref = Some(*child_ref);
                camera_target = inst.properties.get(&ustr("CurrentCamera")).cloned();
                break;
            }
        }
        if let (Some(ws_ref), Some(Variant::Ref(cam_ref))) = (workspace_ref, camera_target) {
            if new_tree.get_by_ref(cam_ref).is_some() {
                new_tree.destroy(cam_ref);
            }
            deferred_referents.remove_ref(ws_ref, "CurrentCamera");
        }
    }

    let ignore_referents = project
        .syncback_rules
        .as_ref()
        .and_then(|s| s.ignore_referents)
        .unwrap_or_default();
    if !ignore_referents {
        link_referents(deferred_referents, &mut new_tree)?;
    }
    log::debug!(
        "syncback: filter props + link refs in {:.1?}",
        phase_timer.elapsed()
    );

    new_tree.root_mut().name = old_tree.root().name().to_string();

    let phase_timer = std::time::Instant::now();
    let (old_hashes, new_hashes) = rayon::join(
        || hash_tree(project, old_tree.inner(), old_tree.get_root_id()),
        || hash_tree(project, &new_tree, new_tree.root_ref()),
    );
    log::debug!(
        "syncback: hash both trees (parallel) in {:.1?}",
        phase_timer.elapsed()
    );

    let project_path = project.folder_location();

    let phase_timer = std::time::Instant::now();
    let existing_paths: HashSet<PathBuf> = if !incremental {
        let mut paths = HashSet::new();

        // Get the source directories from the project's tree structure.
        // We need to collect ALL $path directories defined in the project,
        // not just instigating_source metadata (which may point to project file).
        let mut dirs_to_scan: Vec<PathBuf> = Vec::new();

        // Helper to recursively collect $path DIRECTORY entries from project tree
        // Only directories need to be scanned for orphan detection; single files
        // don't contain orphans that need to be removed.
        fn collect_paths_from_project(
            node: &crate::project::ProjectNode,
            base_path: &Path,
            paths: &mut Vec<PathBuf>,
        ) {
            // If this node has a $path that points to a directory, add it
            if let Some(ref path_node) = node.path {
                let resolved = base_path.join(path_node.path());
                // Only add directories - single files don't need orphan scanning
                if resolved.is_dir() && !paths.contains(&resolved) {
                    log::trace!("Found $path directory in project: {}", resolved.display());
                    paths.push(resolved);
                }
            }
            // Recursively check children
            for child in node.children.values() {
                collect_paths_from_project(child, base_path, paths);
            }
        }

        // Collect paths from the project tree definition
        collect_paths_from_project(&project.tree, project_path, &mut dirs_to_scan);

        // NOTE: We intentionally do NOT scan the project root folder itself.
        // Only directories explicitly referenced via $path should be scanned.
        // Scanning the root would delete unrelated files like .gitignore, README.md, etc.
        //
        // However, we DO need to collect alternate file representations of $path entries.
        // For example, if $path: "src" expects a directory, but "src.luau" exists as a file
        // (perhaps the user accidentally converted the directory to a file), that orphan
        // file should be removed. We collect these as individual files to check, not as
        // directories to scan recursively.
        let mut orphan_files_to_check: Vec<PathBuf> = Vec::new();

        // Helper to find alternate file extensions for a path
        fn collect_alternate_files(
            node: &crate::project::ProjectNode,
            base_path: &Path,
            files: &mut Vec<PathBuf>,
        ) {
            if let Some(ref path_node) = node.path {
                let path_str = path_node.path();
                let resolved = base_path.join(path_str);

                // If the $path points to a directory, check for file alternates
                if resolved.is_dir() {
                    // Common Rojo file extensions that could be alternates
                    let extensions = [
                        ".luau",
                        ".lua",
                        ".server.luau",
                        ".server.lua",
                        ".client.luau",
                        ".client.lua",
                        ".local.luau",
                        ".local.lua",
                    ];
                    for ext in extensions {
                        // Create alternate filename by appending extension to path name
                        let path_str_display = path_str.display().to_string();
                        let alt_file = base_path.join(format!("{}{}", path_str_display, ext));
                        if alt_file.exists() && alt_file.is_file() {
                            log::trace!(
                                "Found alternate file for $path '{}': {}",
                                path_str_display,
                                alt_file.display()
                            );
                            files.push(alt_file);
                        }
                    }
                }
            }
            for child in node.children.values() {
                collect_alternate_files(child, base_path, files);
            }
        }
        collect_alternate_files(&project.tree, project_path, &mut orphan_files_to_check);

        // Also check instance metadata for paths that might not be in project
        // (e.g., from nested project references)
        let root = old_tree.root();
        log::trace!(
            "Root instance: name={}, class={}",
            root.name(),
            root.class_name()
        );
        if let Some(source) = &root.metadata().instigating_source {
            let path = source.path();
            // Skip project files - we want source directories
            if !path.to_string_lossy().ends_with(".project.json5")
                && !path.to_string_lossy().ends_with(".project.json")
            {
                log::trace!("Root has instigating_source: {}", path.display());
                if path.exists() && !dirs_to_scan.contains(&path.to_path_buf()) {
                    dirs_to_scan.push(path.to_path_buf());
                }
            }
        }

        // Check children for additional paths
        for ref_id in old_tree.inner().root().children() {
            if let Some(inst) = old_tree.get_instance(*ref_id) {
                if let Some(source) = &inst.metadata().instigating_source {
                    let path = source.path();
                    if !path.to_string_lossy().ends_with(".project.json5")
                        && !path.to_string_lossy().ends_with(".project.json")
                        && path.exists()
                        && !dirs_to_scan.contains(&path.to_path_buf())
                    {
                        dirs_to_scan.push(path.to_path_buf());
                    }
                }
            }
        }

        if log::log_enabled!(log::Level::Trace) {
            let dirs_str: Vec<_> = dirs_to_scan
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            log::trace!("dirs_to_scan: {:?}", dirs_str);
        }

        for dir in &dirs_to_scan {
            if !dir.is_dir() {
                continue;
            }
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') && name != ".gitkeep" {
                    continue;
                }
            }
            for entry in walkdir::WalkDir::new(dir)
                .follow_links(true)
                .into_iter()
                .filter_entry(|e| {
                    if e.depth() == 0 {
                        return true;
                    }
                    e.file_name()
                        .to_str()
                        .is_none_or(|n| !n.starts_with('.') || n == ".gitkeep")
                })
                .flatten()
            {
                if entry.depth() == 0 {
                    continue;
                }
                let path = entry.path().to_path_buf();
                if !is_valid_path(&ignore_patterns, project_path, &path) {
                    continue;
                }
                paths.insert(path);
            }
        }

        // Also add any alternate file representations we found earlier
        // (e.g., src.luau when $path: "src" is a directory)
        for file in &orphan_files_to_check {
            if !is_valid_path(&ignore_patterns, project_path, file) {
                continue;
            }
            log::trace!("Adding alternate file to orphan check: {}", file.display());
            paths.insert(file.clone());
        }

        log::debug!("Scanned {} existing paths from filesystem", paths.len());
        paths
    } else {
        HashSet::new()
    };
    log::debug!(
        "syncback: orphan scan in {:.1?} ({} paths)",
        phase_timer.elapsed(),
        existing_paths.len()
    );

    let phase_timer = std::time::Instant::now();
    let ref_path_map = std::cell::RefCell::new(HashMap::new());
    let syncback_data = SyncbackData {
        vfs,
        old_tree,
        new_tree: &new_tree,
        project,
        incremental,
        stats,
        ref_path_map: &ref_path_map,
    };

    // Always start with old reference for the Project middleware.
    // In clean mode, child snapshots will have old=None (handled by SyncbackSnapshot methods).
    let mut snapshots = vec![SyncbackSnapshot {
        data: syncback_data,
        old: Some(old_tree.get_root_id()),
        new: new_tree.root_ref(),
        path: project.file_location.clone(),
        middleware: Some(Middleware::Project),
        needs_meta_name: false,
    }];

    let mut fs_snapshot = FsSnapshot::new();
    let mut instance_paths: HashMap<Ref, Vec<PathBuf>> = HashMap::new();

    'syncback: while let Some(snapshot) = snapshots.pop() {
        let inst_path = snapshot.get_new_inst_path(snapshot.new);
        // In incremental mode, we can quickly check that two subtrees are identical
        // and if they are, skip reconciling them. In clean mode, we always process
        // all instances to ensure fresh structure.
        if incremental {
            if let Some(old_ref) = snapshot.old {
                match (old_hashes.get(&old_ref), new_hashes.get(&snapshot.new)) {
                    (Some(old), Some(new)) => {
                        if old == new {
                            log::trace!(
                                "Skipping {inst_path} due to it being identically hashed as {old:?}"
                            );
                            continue;
                        }
                    }
                    _ => unreachable!("All Instances in both DOMs should have hashes"),
                }
            }
        }

        if !is_valid_path(&ignore_patterns, project_path, &snapshot.path) {
            log::debug!("Skipping {inst_path} because its path matches ignore pattern");
            continue;
        }
        // Check ignoreTrees with glob pattern support
        if let Some(ref globs) = tree_globs {
            for (glob, _pattern) in globs {
                if glob.is_match(&inst_path) {
                    log::debug!("Tree {inst_path} is blocked by ignoreTrees glob pattern");
                    continue 'syncback;
                }
            }
        }

        let middleware = get_best_middleware(&snapshot);

        log::trace!(
            "Middleware for {inst_path} is {:?} (path is {})",
            middleware,
            snapshot.path.display()
        );

        if matches!(middleware, Middleware::Json | Middleware::Toml) {
            log::warn!("Cannot syncback {middleware:?} at {inst_path}, skipping");
            continue;
        }

        let syncback = match middleware.syncback(&snapshot) {
            Ok(syncback) => syncback,
            Err(err) if middleware == Middleware::Dir => {
                let new_middleware = match env::var(DEBUG_MODEL_FORMAT_VAR) {
                    Ok(value) if value == "1" => Middleware::Rbxmx,
                    Ok(value) if value == "2" => Middleware::JsonModel,
                    _ => Middleware::Rbxm,
                };
                let file_name = snapshot
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .context("Directory middleware should have a name in its path")?;
                let mut path = snapshot.path.clone();
                path.set_file_name(format!(
                    "{file_name}.{}",
                    extension_for_middleware(new_middleware)
                ));
                let new_snapshot = snapshot.with_new_path(path, snapshot.new, snapshot.old);
                // Record the fallback in stats instead of warning directly
                stats.record_rbxm_fallback(&inst_path, &err.to_string());
                let new_syncback_result = new_middleware
                    .syncback(&new_snapshot)
                    .with_context(|| format!("Failed to syncback {inst_path}"));
                if new_syncback_result.is_ok() && snapshot.old_inst().is_some() {
                    // We need to remove the old FS representation if we're
                    // reserializing it as an rbxm.
                    fs_snapshot.remove_dir(&snapshot.path);
                }
                new_syncback_result?
            }
            Err(err) => anyhow::bail!("Failed to syncback {inst_path} because {err}"),
        };

        if !syncback.removed_children.is_empty() {
            log::debug!(
                "removed children for {inst_path}: {}",
                syncback.removed_children.len()
            );
            'remove: for inst in &syncback.removed_children {
                let path = inst.metadata().instigating_source.as_ref().unwrap().path();
                let inst_path = snapshot.get_old_inst_path(inst.id());
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') && name != ".gitkeep" {
                        log::debug!("Skipping removing {} (hidden path)", path.display());
                        continue;
                    }
                }
                if !is_valid_path(&ignore_patterns, project_path, path) {
                    log::debug!(
                        "Skipping removing {} because its matches an ignore pattern",
                        path.display()
                    );
                    continue;
                }
                // Check ignoreTrees with glob pattern support
                if let Some(ref globs) = tree_globs {
                    for (glob, _pattern) in globs {
                        if glob.is_match(&inst_path) {
                            log::debug!("Skipping removing {inst_path} because its path is blocked by ignoreTrees glob pattern");
                            continue 'remove;
                        }
                    }
                }
                if path.is_dir() {
                    fs_snapshot.remove_dir(path)
                } else {
                    fs_snapshot.remove_file(path)
                }
            }
        }

        // TODO provide replacement snapshots for e.g. two way sync

        // Collect file paths for this instance before merging into the main snapshot.
        // This builds the instance-to-path map used for sourcemap generation.
        let files_for_instance: Vec<PathBuf> = syncback
            .fs_snapshot
            .added_files()
            .into_iter()
            .map(|p| p.to_path_buf())
            .collect();
        if !files_for_instance.is_empty() {
            instance_paths.insert(snapshot.new, files_for_instance);
        }

        fs_snapshot.merge_with_filter(syncback.fs_snapshot, |path| {
            is_valid_path(&ignore_patterns, project_path, path)
        });

        snapshots.extend(syncback.children);
    }
    log::debug!("syncback: main walk loop in {:.1?}", phase_timer.elapsed());

    let phase_timer = std::time::Instant::now();
    {
        use ref_properties::tentative_fs_path_public;

        let final_map = ref_path_map.borrow();
        let mut substitutions: Vec<(String, String)> = Vec::new();
        for (placeholder, (source_ref, target_ref)) in &placeholder_map {
            let source_abs = final_map
                .get(source_ref)
                .cloned()
                .unwrap_or_else(|| tentative_fs_path_public(&new_tree, *source_ref));
            let target_abs = final_map
                .get(target_ref)
                .cloned()
                .unwrap_or_else(|| tentative_fs_path_public(&new_tree, *target_ref));
            let relative = crate::compute_relative_ref_path(&source_abs, &target_abs);
            substitutions.push((placeholder.clone(), relative));
        }
        log::debug!(
            "syncback: built {} ref substitutions in {:.1?}",
            substitutions.len(),
            phase_timer.elapsed()
        );

        let sub_timer = std::time::Instant::now();
        if !substitutions.is_empty() {
            fs_snapshot.fix_ref_paths(&substitutions);
        }
        log::debug!(
            "syncback: applied ref substitutions in {:.1?}",
            sub_timer.elapsed()
        );
    }

    let phase_timer = std::time::Instant::now();
    if !incremental && !existing_paths.is_empty() {
        log::debug!("Clean mode: checking for orphaned files to remove");

        let added_paths: HashSet<PathBuf> = fs_snapshot
            .added_paths()
            .into_iter()
            .map(|p| project_path.join(p))
            .collect();

        let mut added_dir_prefixes: HashSet<PathBuf> = HashSet::new();
        for p in &added_paths {
            let mut ancestor = p.clone();
            while ancestor.pop() {
                if !added_dir_prefixes.insert(ancestor.clone()) {
                    break;
                }
            }
        }

        let project_file = project.file_location.clone();

        // Collect ALL paths explicitly referenced via $path in the project.
        // These paths should NOT be removed during orphan cleanup because they
        // are explicitly part of the project structure.
        let mut protected_paths: HashSet<PathBuf> = HashSet::new();

        // Also build a mapping from filesystem path prefix to instance path prefix.
        // This is needed to convert filesystem paths to instance paths for ignoreTrees checking.
        // e.g., if project has "$path": "src" on "ReplicatedStorage", then
        // filesystem "src/Foo" maps to instance "ReplicatedStorage/Foo"
        let mut path_to_instance_prefix: Vec<(PathBuf, String)> = Vec::new();

        fn collect_all_path_refs_and_mappings(
            node: &crate::project::ProjectNode,
            base_path: &Path,
            instance_path: &str,
            protected: &mut HashSet<PathBuf>,
            mappings: &mut Vec<(PathBuf, String)>,
        ) {
            if let Some(ref path_node) = node.path {
                let resolved = base_path.join(path_node.path());
                protected.insert(resolved.clone());
                if !instance_path.is_empty() {
                    mappings.push((resolved, instance_path.to_string()));
                }
            }
            for (name, child) in &node.children {
                let child_inst_path = if instance_path.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", instance_path, name)
                };
                collect_all_path_refs_and_mappings(
                    child,
                    base_path,
                    &child_inst_path,
                    protected,
                    mappings,
                );
            }
        }
        collect_all_path_refs_and_mappings(
            &project.tree,
            project_path,
            "",
            &mut protected_paths,
            &mut path_to_instance_prefix,
        );
        if log::log_enabled!(log::Level::Trace) {
            log::trace!("Protected $path references: {:?}", protected_paths);
            log::trace!("Path to instance mappings: {:?}", path_to_instance_prefix);
        }

        // Helper to convert a filesystem path to an instance path using the mappings
        let fs_path_to_instance_path = |fs_path: &Path| -> Option<String> {
            for (fs_prefix, inst_prefix) in &path_to_instance_prefix {
                if let Ok(relative) = fs_path.strip_prefix(fs_prefix) {
                    let relative_str = relative
                        .components()
                        .filter_map(|c| c.as_os_str().to_str())
                        .collect::<Vec<_>>()
                        .join("/");
                    if relative_str.is_empty() {
                        return Some(inst_prefix.clone());
                    } else {
                        return Some(format!("{}/{}", inst_prefix, relative_str));
                    }
                }
            }
            None
        };

        let mut paths_to_remove: HashSet<PathBuf> = HashSet::new();
        for old_path in &existing_paths {
            let old_path_norm = old_path.clone();

            if old_path_norm == project_file {
                log::trace!("Skipping root project file: {}", old_path.display());
                continue;
            }

            // Never remove paths that are explicitly referenced via $path in the project
            if protected_paths.contains(&old_path_norm) {
                log::trace!(
                    "Skipping {} (explicitly referenced via $path)",
                    old_path.display()
                );
                continue;
            }

            // Check ignoreTrees with glob pattern support.
            // Convert filesystem path to instance path using the project structure mappings.
            if let Some(ref globs) = tree_globs {
                if let Some(inst_path_str) = fs_path_to_instance_path(&old_path_norm) {
                    let mut is_ignored = false;
                    for (glob, pattern_str) in globs {
                        // Check if this path matches the ignore pattern
                        if glob.is_match(&inst_path_str) {
                            log::trace!(
                                "Skipping {} (matches ignoreTrees pattern, inst_path: {})",
                                old_path.display(),
                                inst_path_str
                            );
                            is_ignored = true;
                            break;
                        }
                        // Also check if this is a directory that is an ANCESTOR of an ignored path.
                        // e.g., if "ReplicatedStorage/KeepMe" is ignored, then "ReplicatedStorage" (src/)
                        // should not be removed as it contains ignored content.
                        if old_path_norm.is_dir() {
                            // For literal patterns (no wildcards), use string prefix check
                            if !pattern_str.contains('*') && !pattern_str.contains('?') {
                                if pattern_str.starts_with(&inst_path_str)
                                    && pattern_str.len() > inst_path_str.len()
                                    && pattern_str.as_bytes().get(inst_path_str.len())
                                        == Some(&b'/')
                                {
                                    log::trace!(
                                        "Skipping {} (ancestor of literal ignoreTrees path, inst_path: {})",
                                        old_path.display(),
                                        inst_path_str
                                    );
                                    is_ignored = true;
                                    break;
                                }
                            } else {
                                // For glob patterns, check if ANY file inside this directory
                                // could match the pattern. We do this by checking all existing_paths
                                // that are inside this directory.
                                let dir_contains_match = existing_paths.iter().any(|child_path| {
                                    // Only check children of this directory
                                    if !child_path.starts_with(&old_path_norm)
                                        || child_path == &old_path_norm
                                    {
                                        return false;
                                    }
                                    // Convert child path to instance path and check glob match
                                    if let Some(child_inst_path) =
                                        fs_path_to_instance_path(child_path)
                                    {
                                        if glob.is_match(&child_inst_path) {
                                            log::trace!(
                                                "Directory {} contains ignored file {} (inst: {})",
                                                old_path.display(),
                                                child_path.display(),
                                                child_inst_path
                                            );
                                            return true;
                                        }
                                    }
                                    false
                                });
                                if dir_contains_match {
                                    log::trace!(
                                        "Skipping {} (contains files matching glob pattern)",
                                        old_path.display()
                                    );
                                    is_ignored = true;
                                    break;
                                }
                            }
                        }
                    }
                    if is_ignored {
                        continue;
                    }
                }
            }

            if added_paths.contains(&old_path_norm) {
                continue;
            }
            if old_path_norm.is_dir() && added_dir_prefixes.contains(&old_path_norm) {
                continue;
            }
            paths_to_remove.insert(old_path_norm);
        }

        // Second pass: only remove top-level orphaned paths.
        // Sorted path order guarantees all descendants appear consecutively
        // after their ancestor, so a single ancestor tracker suffices.
        let mut sorted_removals: Vec<_> = paths_to_remove.into_iter().collect::<Vec<_>>();
        sorted_removals.sort();

        let mut current_ancestor: Option<&PathBuf> = None;
        for old_path in &sorted_removals {
            if let Some(ancestor) = current_ancestor {
                if old_path.starts_with(ancestor) && old_path != ancestor {
                    continue;
                }
            }
            current_ancestor = Some(old_path);

            let relative_path = old_path.strip_prefix(project_path).unwrap_or(old_path);

            log::debug!("Removing orphaned path: {}", relative_path.display());
            if old_path.is_dir() {
                fs_snapshot.remove_dir(relative_path);
            } else {
                fs_snapshot.remove_file(relative_path);
            }
        }
    }

    log::debug!("syncback: orphan removal in {:.1?}", phase_timer.elapsed());

    if external_stats.is_none() {
        stats.log_summary();
    }

    Ok(SyncbackResult {
        fs_snapshot,
        new_tree,
        instance_paths,
    })
}

pub struct SyncbackReturn<'sync> {
    pub fs_snapshot: FsSnapshot,
    pub children: Vec<SyncbackSnapshot<'sync>>,
    pub removed_children: Vec<InstanceWithMeta<'sync>>,
}

pub fn get_best_middleware(snapshot: &SyncbackSnapshot) -> Middleware {
    let old_middleware = snapshot
        .old_inst()
        .and_then(|inst| inst.metadata().middleware);
    let inst = snapshot.new_inst();

    let mut middleware;

    if let Some(override_middleware) = snapshot.middleware {
        return override_middleware;
    } else if let Some(old_middleware) = old_middleware {
        // Use old middleware, but upgrade to *Dir variant if new instance has children
        // This handles cases where the old file was a single file (e.g., Csv)
        // but the new instance has children (needs CsvDir)
        middleware = old_middleware;
    } else {
        // Specific classes that need special middleware, everything else defaults to JsonModel
        middleware = match inst.class.as_str() {
            "Folder" | "Configuration" | "Tool" | "ScreenGui" | "SurfaceGui" | "BillboardGui"
            | "AdGui" => Middleware::Dir,
            "StringValue" => Middleware::Text,
            "Script" => {
                // Check RunContext to determine which middleware to use
                // RunContext enum values: Legacy = 0, Server = 1, Client = 2, Plugin = 3
                match inst.properties.get(&ustr("RunContext")) {
                    Some(Variant::Enum(e)) => match e.to_u32() {
                        0 => Middleware::LegacyScript,
                        1 => Middleware::ServerScript,
                        2 => Middleware::ClientScript,
                        3 => Middleware::PluginScript,
                        _ => Middleware::LegacyScript, // Unknown RunContext, default to Legacy
                    },
                    _ => Middleware::LegacyScript, // No RunContext property, default to Legacy
                }
            }
            "LocalScript" => Middleware::LocalScript,
            "ModuleScript" => Middleware::ModuleScript,
            "LocalizationTable" => Middleware::Csv,
            // Default: use JsonModel for everything else (becomes Dir if has children)
            _ => Middleware::JsonModel,
        }
    }

    if !inst.children().is_empty() {
        middleware = match middleware {
            Middleware::ServerScript => Middleware::ServerScriptDir,
            Middleware::ClientScript => Middleware::ClientScriptDir,
            Middleware::ModuleScript => Middleware::ModuleScriptDir,
            Middleware::PluginScript => Middleware::PluginScriptDir,
            Middleware::LegacyScript => Middleware::LegacyScriptDir,
            Middleware::LocalScript => Middleware::LocalScriptDir,
            Middleware::Csv => Middleware::CsvDir,
            Middleware::JsonModel | Middleware::Text => Middleware::Dir,
            _ => middleware,
        }
    }

    middleware
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncbackRules {
    /// A list of subtrees in a file that will be ignored by Syncback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ignore_trees: Vec<String>,
    /// A list of patterns to check against the path an Instance would serialize
    /// to. If a path matches one of these, the Instance won't be syncbacked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ignore_paths: Vec<String>,
    /// A map of classes to properties to ignore for that class when doing
    /// syncback.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    ignore_properties: IndexMap<Ustr, Vec<Ustr>>,
    /// A list of class names to ignore entirely during syncback.
    /// Instances of these classes will not be added, removed, or synced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    ignore_classes: Vec<String>,
    /// Whether or not the `CurrentCamera` of `Workspace` is included in the
    /// syncback or not. Defaults to `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_current_camera: Option<bool>,
    /// Whether or not to sync properties that cannot be modified via scripts.
    /// Defaults to `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_unscriptable: Option<bool>,
    /// Whether to skip serializing referent properties like `Model.PrimaryPart`
    /// during syncback. Defaults to `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore_referents: Option<bool>,
    /// Whether the globs specified in `ignore_paths` should be modified to also
    /// match directories. Defaults to `true`.
    ///
    /// If this is `true`, it'll take ignore globs that end in `/**` and convert
    /// them to also handle the directory they're referring to. This is
    /// generally a better UX.
    #[serde(skip_serializing_if = "Option::is_none")]
    create_ignore_dir_paths: Option<bool>,
    /// When enabled, only "visible" services will be synced back. This includes
    /// commonly used services like Workspace, ReplicatedStorage, ServerScriptService,
    /// etc., while ignoring internal/hidden services like Chat, HttpService, etc.
    /// Defaults to `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore_hidden_services: Option<bool>,
    /// Whether to emit warnings when duplicate child names are encountered during
    /// syncback. Duplicate names (case-insensitive) cannot be reliably synced to
    /// the file system, so those children are skipped.
    /// Defaults to `false` (warnings are suppressed).
    #[serde(skip_serializing_if = "Option::is_none")]
    warn_duplicate_names: Option<bool>,
}

impl SyncbackRules {
    pub fn compile_globs(&self) -> anyhow::Result<Vec<Glob>> {
        let mut globs = Vec::with_capacity(self.ignore_paths.len());
        let dir_ignore_paths = self.create_ignore_dir_paths.unwrap_or(true);

        for pattern in &self.ignore_paths {
            let glob = Glob::new(pattern)
                .with_context(|| format!("the pattern '{pattern}' is not a valid glob"))?;
            globs.push(glob);

            if dir_ignore_paths {
                if let Some(dir_pattern) = pattern.strip_suffix("/**") {
                    if let Ok(glob) = Glob::new(dir_pattern) {
                        globs.push(glob)
                    }
                }
            }
        }

        Ok(globs)
    }

    /// Compiles the ignoreTrees patterns into glob matchers.
    /// Supports glob patterns like `**/Abc/Script` for flexible matching.
    /// Returns both the compiled Glob and the original pattern string for each entry.
    pub fn compile_tree_globs(&self) -> anyhow::Result<Vec<(Glob, String)>> {
        let mut globs = Vec::with_capacity(self.ignore_trees.len());

        for pattern in &self.ignore_trees {
            let glob = Glob::new(pattern).with_context(|| {
                format!("the pattern '{pattern}' is not a valid ignoreTrees glob")
            })?;
            globs.push((glob, pattern.clone()));
        }

        Ok(globs)
    }

    /// Returns whether hidden/internal services should be ignored during
    /// syncback. When `true`, only visible services like Workspace,
    /// ReplicatedStorage, etc. will be synced. Defaults to `true`.
    #[inline]
    pub fn ignore_hidden_services(&self) -> bool {
        self.ignore_hidden_services.unwrap_or(true)
    }

    /// Returns whether to emit warnings when duplicate child names are
    /// encountered during syncback. Defaults to `false` (suppressed).
    #[inline]
    pub fn warn_duplicate_names(&self) -> bool {
        self.warn_duplicate_names.unwrap_or(false)
    }
}

fn is_valid_path(globs: &Option<Vec<Glob>>, base_path: &Path, path: &Path) -> bool {
    let git_glob = GIT_IGNORE_GLOB.get_or_init(|| Glob::new(".git/**").unwrap());
    let test_path = match path.strip_prefix(base_path) {
        Ok(suffix) => suffix,
        Err(_) => path,
    };
    if git_glob.is_match(test_path) {
        return false;
    }
    if let Some(ref ignore_paths) = globs {
        for glob in ignore_paths {
            if glob.is_match(test_path) {
                return false;
            }
        }
    }
    true
}

/// Returns a set of properties that should not be written with syncback if
/// one exists. This list is read directly from the Project and takes
/// inheritance into effect.
///
/// It **does not** handle properties that should not serialize for other
/// reasons, such as being defaults or being marked as not serializing in the
/// ReflectionDatabase.
fn get_property_filter(project: &Project, new_inst: &Instance) -> Option<UstrSet> {
    let filter = &project.syncback_rules.as_ref()?.ignore_properties;
    let mut set = UstrSet::default();

    let database = rbx_reflection_database::get().unwrap();
    let mut current_class_name = new_inst.class.as_str();

    loop {
        if let Some(list) = filter.get(&ustr(current_class_name)) {
            set.extend(list)
        }

        let class = database.classes.get(current_class_name)?;
        if let Some(super_class) = class.superclass.as_ref() {
            current_class_name = super_class;
        } else {
            break;
        }
    }

    Some(set)
}

/// Produces a list of descendants in the WeakDom such that all children come
/// before their parents.
fn descendants(dom: &WeakDom, root_ref: Ref) -> Vec<Ref> {
    let mut queue = VecDeque::new();
    let mut ordered = Vec::new();
    queue.push_front(root_ref);

    while let Some(referent) = queue.pop_front() {
        let inst = dom
            .get_by_ref(referent)
            .expect("Invariant: WeakDom had a Ref that wasn't inside it");
        ordered.push(referent);
        for child in inst.children() {
            queue.push_back(*child)
        }
    }

    ordered
}

/// Removes root children (services) that are not in the `VISIBLE_SERVICES` list.
/// This is used when `ignoreHiddenServices` is enabled to filter out internal
/// services like Chat, HttpService, etc.
///
/// This function only applies when the root is a DataModel (i.e., for place files).
/// For model files, the root children are regular instances, not services, so
/// filtering would incorrectly remove user content.
fn strip_hidden_services(dom: &mut WeakDom) {
    // Only apply service filtering when the root is a DataModel (place file)
    // For model files (rbxm), the root is typically not a DataModel and
    // children are regular instances, not services
    if dom.root().class != "DataModel" {
        log::trace!(
            "Skipping hidden services filter: root class is '{}', not 'DataModel'",
            dom.root().class
        );
        return;
    }

    let root_children = dom.root().children().to_vec();

    for child_ref in root_children {
        let child = dom
            .get_by_ref(child_ref)
            .expect("all children of the root should exist in the DOM");

        // Check if this service is in the visible services list
        if !VISIBLE_SERVICES.contains(&child.name.as_str()) {
            log::trace!(
                "Pruning hidden service {} of class {}",
                child.name,
                child.class
            );
            dom.destroy(child_ref);
        }
    }
}

/// Removes the children of `new`'s root that are not also children of `old`'s
/// root.
///
/// This does not care about duplicates, and only filters based on names and
/// class names.
fn strip_unknown_root_children(new: &mut WeakDom, old: &RojoTree) {
    let old_root = old.root();
    let old_root_children: HashMap<&str, InstanceWithMeta> = old_root
        .children()
        .iter()
        .map(|referent| {
            let inst = old
                .get_instance(*referent)
                .expect("all children of a DOM's root should exist");
            (inst.name(), inst)
        })
        .collect();

    let root_children = new.root().children().to_vec();

    for child_ref in root_children {
        let child = new
            .get_by_ref(child_ref)
            .expect("all children of the root should exist in the DOM");
        if let Some(old) = old_root_children.get(child.name.as_str()) {
            if old.class_name() == child.class {
                continue;
            }
        }
        log::trace!("Pruning root child {} of class {}", child.name, child.class);
        new.destroy(child_ref);
    }
}
