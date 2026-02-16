use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::Context;
use memofs::{DirEntry, Vfs};

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot, InstigatingSource},
    snapshot_middleware::Middleware,
    syncback::{
        hash_instance, name_needs_slugify, slugify_name, strip_middleware_extension, FsSnapshot,
        SyncbackReturn, SyncbackSnapshot,
    },
};

use super::{meta_file::DirectoryMetadata, snapshot_from_vfs};

const EMPTY_DIR_KEEP_NAME: &str = ".gitkeep";

pub fn snapshot_dir(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let mut snapshot = match snapshot_dir_no_meta(context, vfs, path, name)? {
        Some(snapshot) => snapshot,
        None => return Ok(None),
    };

    DirectoryMetadata::read_and_apply_all(vfs, path, &mut snapshot)?;

    Ok(Some(snapshot))
}

/// Snapshot a directory without applying meta files; useful for if the
/// directory's ClassName will change before metadata should be applied. For
/// example, this can happen if the directory contains an `init.client.luau`
/// file.
pub fn snapshot_dir_no_meta(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let passes_filter_rules = |child: &DirEntry| {
        context
            .path_ignore_rules
            .iter()
            .all(|rule| rule.passes(child.path()))
    };

    let mut snapshot_children = Vec::new();

    for entry in vfs.read_dir(path)? {
        let entry = entry?;

        if !passes_filter_rules(&entry) {
            continue;
        }

        if let Some(child_snapshot) = snapshot_from_vfs(context, vfs, entry.path())? {
            snapshot_children.push(child_snapshot);
        }
    }

    let normalized_path = vfs.canonicalize(path)?;
    let relevant_paths = vec![
        normalized_path.clone(),
        // TODO: We shouldn't need to know about Lua existing in this
        // middleware. Should we figure out a way for that function to add
        // relevant paths to this middleware?
        // Modern extensions (preferred)
        normalized_path.join("init.luau"),
        normalized_path.join("init.server.luau"),
        normalized_path.join("init.client.luau"),
        normalized_path.join("init.local.luau"),
        normalized_path.join("init.legacy.luau"),
        normalized_path.join("init.csv"),
        // Legacy extensions (for backwards compatibility)
        normalized_path.join("init.lua"),
        normalized_path.join("init.server.lua"),
        normalized_path.join("init.client.lua"),
    ];

    let snapshot = InstanceSnapshot::new()
        .name(name)
        .class_name("Folder")
        .children(snapshot_children)
        .metadata(
            InstanceMetadata::new()
                .instigating_source(path)
                .relevant_paths(relevant_paths)
                .context(context),
        );

    Ok(Some(snapshot))
}

pub fn syncback_dir<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();

    let mut dir_syncback = syncback_dir_no_meta(snapshot)?;

    let mut meta = DirectoryMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?;
    if let Some(meta) = &mut meta {
        if new_inst.class != "Folder" {
            meta.class_name = Some(new_inst.class);
        }

        if !meta.is_empty() {
            dir_syncback.fs_snapshot.add_file(
                snapshot.path.join("init.meta.json5"),
                crate::json::to_vec_pretty_sorted(&meta)
                    .context("could not serialize new init.meta.json5")?,
            );
        }
    }

    let metadata_empty = meta
        .as_ref()
        .map(DirectoryMetadata::is_empty)
        .unwrap_or_default();
    if new_inst.children().is_empty() && metadata_empty {
        dir_syncback
            .fs_snapshot
            .add_file(snapshot.path.join(EMPTY_DIR_KEEP_NAME), Vec::new())
    }

    Ok(dir_syncback)
}

pub fn syncback_dir_no_meta<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();

    let mut children = Vec::new();
    let mut removed_children = Vec::new();

    // taken_names tracks claimed bare slugs (without extensions) for dedup.
    // Pre-seeded from old tree children's slugified instance names so that
    // new-only children correctly dedup against existing siblings, regardless
    // of iteration order (see plan: fix_stem-level_dedup §2).
    let mut taken_names: HashSet<String> = HashSet::new();

    // Detect duplicate child names (case-insensitive for file system safety).
    // Instead of skipping duplicates, return an error to trigger the rbxm
    // container fallback in the main syncback loop.
    if crate::syncback::has_duplicate_children(snapshot.new_tree(), snapshot.new) {
        let inst_path = crate::syncback::inst_path(snapshot.new_tree(), snapshot.new);
        anyhow::bail!(
            "directory has duplicate-named children at {inst_path}, converting to rbxm container"
        );
    }

    if let Some(old_inst) = snapshot.old_inst() {
        let mut old_child_map = HashMap::with_capacity(old_inst.children().len());
        for child in old_inst.children() {
            let inst = snapshot.get_old_instance(*child).unwrap();
            old_child_map.insert(inst.name(), inst);
        }

        // Pre-seed taken_names from old children's actual filesystem dedup keys
        // so that new-only children correctly dedup against existing siblings.
        // We derive keys from relevant_paths (the real filename on disk) rather
        // than slugifying instance names, because an old instance may already
        // have a tilde suffix (e.g. A_B~1.luau from prior dedup) that the bare
        // slug wouldn't capture.
        // Only in incremental mode: in clean mode, old_ref is forced to None
        // for all children, so every child is treated as new and pre-seeding
        // would cause false collisions (spurious ~1 suffixes).
        if snapshot.data.is_incremental() {
            for inst in old_child_map.values() {
                if let Some(path) = inst.metadata().relevant_paths.first() {
                    if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                        let middleware = inst.metadata().middleware.unwrap_or(Middleware::Dir);
                        let dedup_key =
                            strip_middleware_extension(filename, middleware).to_lowercase();
                        taken_names.insert(dedup_key);
                    }
                } else {
                    // Fallback (shouldn't happen for old instances with disk presence)
                    let name = inst.name();
                    let slug = if name_needs_slugify(name) {
                        slugify_name(name).to_lowercase()
                    } else {
                        name.to_lowercase()
                    };
                    taken_names.insert(slug);
                }
            }
        }

        // Sort children alphabetically for deterministic dedup ordering.
        // Without this, DOM iteration order determines which sibling gets
        // the base slug vs ~1, causing non-deterministic output across
        // different serializations of the same place.
        let mut sorted_children: Vec<_> = new_inst
            .children()
            .iter()
            .map(|r| (*r, snapshot.get_new_instance(*r).unwrap().name.clone()))
            .collect();
        sorted_children.sort_by(|(_, a), (_, b)| a.cmp(b));

        for (new_child_ref, _) in &sorted_children {
            let new_child = snapshot.get_new_instance(*new_child_ref).unwrap();

            // Skip instances of ignored classes
            if snapshot.should_ignore_class(&new_child.class) {
                // Also remove from old_child_map so it won't be marked as removed
                old_child_map.remove(new_child.name.as_str());
                log::debug!(
                    "Skipping instance {} because its class {} is ignored",
                    new_child.name,
                    new_child.class
                );
                continue;
            }
            // Skip instances matching ignoreTrees
            if snapshot.should_ignore_tree(*new_child_ref) {
                old_child_map.remove(new_child.name.as_str());
                continue;
            }
            if let Some(old_child) = old_child_map.remove(new_child.name.as_str()) {
                if old_child.metadata().relevant_paths.is_empty() {
                    log::debug!(
                        "Skipping instance {} because it doesn't exist on the disk",
                        old_child.name()
                    );
                    continue;
                } else if matches!(
                    old_child.metadata().instigating_source,
                    Some(InstigatingSource::ProjectNode { .. })
                ) {
                    log::debug!(
                        "Skipping instance {} because it originates in a project file",
                        old_child.name()
                    );
                    continue;
                }
                // This child exists in both doms. Pass it on.
                let (child_snap, _needs_meta, dedup_key) = snapshot.with_joined_path(
                    *new_child_ref,
                    Some(old_child.id()),
                    &taken_names,
                )?;
                taken_names.insert(dedup_key.to_lowercase());
                children.push(child_snap);
            } else {
                // The child only exists in the the new dom
                let (child_snap, _needs_meta, dedup_key) =
                    snapshot.with_joined_path(*new_child_ref, None, &taken_names)?;
                taken_names.insert(dedup_key.to_lowercase());
                children.push(child_snap);
            }
        }
        // Any children that are in the old dom but not the new one are removed.
        // Filter out instances of ignored classes from removal.
        removed_children.extend(old_child_map.into_values().filter(|inst| {
            if snapshot.should_ignore_class(inst.class_name().as_str()) {
                log::debug!(
                    "Not removing instance {} because its class {} is ignored",
                    inst.name(),
                    inst.class_name()
                );
                false
            } else {
                true
            }
        }));
    } else {
        // There is no old instance. Just add every child.
        // Sort alphabetically for deterministic dedup ordering.
        let mut sorted_children: Vec<_> = new_inst
            .children()
            .iter()
            .map(|r| (*r, snapshot.get_new_instance(*r).unwrap().name.clone()))
            .collect();
        sorted_children.sort_by(|(_, a), (_, b)| a.cmp(b));

        for (new_child_ref, _) in &sorted_children {
            let new_child = snapshot.get_new_instance(*new_child_ref).unwrap();

            // Skip instances of ignored classes
            if snapshot.should_ignore_class(&new_child.class) {
                log::debug!(
                    "Skipping instance {} because its class {} is ignored",
                    new_child.name,
                    new_child.class
                );
                continue;
            }
            // Skip instances matching ignoreTrees
            if snapshot.should_ignore_tree(*new_child_ref) {
                continue;
            }
            let (child_snap, _needs_meta, dedup_key) =
                snapshot.with_joined_path(*new_child_ref, None, &taken_names)?;
            taken_names.insert(dedup_key.to_lowercase());
            children.push(child_snap);
        }
    }
    let mut fs_snapshot = FsSnapshot::new();

    if let Some(old_ref) = snapshot.old {
        let new_hash = hash_instance(snapshot.project(), snapshot.new_tree(), snapshot.new)
            .expect("new Instance should be hashable");
        let old_hash = hash_instance(snapshot.project(), snapshot.old_tree(), old_ref)
            .expect("old Instance should be hashable");

        if old_hash != new_hash {
            fs_snapshot.add_dir(&snapshot.path);
        } else {
            log::debug!(
                "Skipping reserializing directory {} because old and new tree hash the same",
                new_inst.name
            );
        }
    } else {
        fs_snapshot.add_dir(&snapshot.path);
    }

    Ok(SyncbackReturn {
        fs_snapshot,
        children,
        removed_children,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn empty_folder() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot("/foo", VfsSnapshot::empty_dir())
            .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot =
            snapshot_dir(&InstanceContext::default(), &vfs, Path::new("/foo"), "foo")
                .unwrap()
                .unwrap();

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn folder_in_folder() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo",
            VfsSnapshot::dir([("Child", VfsSnapshot::empty_dir())]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot =
            snapshot_dir(&InstanceContext::default(), &vfs, Path::new("/foo"), "foo")
                .unwrap()
                .unwrap();

        insta::assert_yaml_snapshot!(instance_snapshot);
    }
}
