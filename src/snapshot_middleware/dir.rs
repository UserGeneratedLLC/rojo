use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::Context;
use memofs::{DirEntry, Vfs};

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot, InstigatingSource},
    syncback::{
        hash_instance, name_needs_slugify, slugify_name, FsSnapshot, SyncbackReturn,
        SyncbackSnapshot,
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
    // We skip duplicates instead of failing, tracking them in stats.
    let mut child_name_counts: HashMap<String, usize> = HashMap::new();
    for child_ref in new_inst.children() {
        let child = snapshot.get_new_instance(*child_ref).unwrap();
        let lower_name = child.name.to_lowercase();
        *child_name_counts.entry(lower_name).or_insert(0) += 1;
    }

    let duplicate_names: HashSet<String> = child_name_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect();

    // Record duplicate names in stats tracker
    if !duplicate_names.is_empty() {
        let inst_path = crate::syncback::inst_path(snapshot.new_tree(), snapshot.new);
        // Count total instances being skipped (sum of all duplicates)
        let mut total_skipped = 0;
        for child_ref in new_inst.children() {
            let child = snapshot.get_new_instance(*child_ref).unwrap();
            if duplicate_names.contains(&child.name.to_lowercase()) {
                total_skipped += 1;
            }
        }
        let duplicate_list: Vec<&str> = duplicate_names.iter().map(|s| s.as_str()).collect();
        snapshot
            .stats()
            .record_duplicate_names_batch(&inst_path, &duplicate_list, total_skipped);
    }

    if let Some(old_inst) = snapshot.old_inst() {
        let mut old_child_map = HashMap::with_capacity(old_inst.children().len());
        for child in old_inst.children() {
            let inst = snapshot.get_old_instance(*child).unwrap();
            old_child_map.insert(inst.name(), inst);
        }

        // Pre-seed taken_names from old children's slugified instance names.
        // This ensures new-only children that happen to slugify to the same
        // bare slug as an existing sibling will be deduplicated (e.g., new
        // "A/B" → slug "A_B" won't collide with existing "A_B").
        // Matches the approach used in api.rs (lines 902-920).
        for (_, inst) in &old_child_map {
            let name = inst.name();
            let slug = if name_needs_slugify(name) {
                slugify_name(name).to_lowercase()
            } else {
                name.to_lowercase()
            };
            taken_names.insert(slug);
        }

        for new_child_ref in new_inst.children() {
            let new_child = snapshot.get_new_instance(*new_child_ref).unwrap();

            // Skip children with duplicate names - cannot reliably sync them
            if duplicate_names.contains(&new_child.name.to_lowercase()) {
                old_child_map.remove(new_child.name.as_str());
                continue;
            }

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
        // Filter out instances of ignored classes and duplicates from removal.
        removed_children.extend(old_child_map.into_values().filter(|inst| {
            // Don't remove duplicates - we're skipping them entirely
            if duplicate_names.contains(&inst.name().to_lowercase()) {
                return false;
            }
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
        for new_child_ref in new_inst.children() {
            let new_child = snapshot.get_new_instance(*new_child_ref).unwrap();

            // Skip children with duplicate names - cannot reliably sync them
            if duplicate_names.contains(&new_child.name.to_lowercase()) {
                continue;
            }

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
