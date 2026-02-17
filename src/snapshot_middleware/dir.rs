use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::Context;
use memofs::{DirEntry, Vfs};
use rbx_dom_weak::types::Ref;

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot, InstigatingSource},
    syncback::{hash_instance, FsSnapshot, SyncbackReturn, SyncbackSnapshot},
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
    use crate::syncback::matching::match_children;

    let new_inst = snapshot.new_inst();

    let mut children = Vec::new();
    let mut removed_children = Vec::new();

    // taken_names tracks claimed full filesystem name components (slug +
    // extension for files, directory name for dirs), lowercased.
    let mut taken_names: HashSet<String> = HashSet::new();

    // Filter new children: exclude ignored classes and ignoreTrees.
    let eligible_new: Vec<Ref> = new_inst
        .children()
        .iter()
        .copied()
        .filter(|r| {
            let child = snapshot.get_new_instance(*r).unwrap();
            if snapshot.should_ignore_class(&child.class) {
                log::debug!(
                    "Skipping instance {} because its class {} is ignored",
                    child.name,
                    child.class
                );
                return false;
            }
            if snapshot.should_ignore_tree(*r) {
                return false;
            }
            true
        })
        .collect();

    if let Some(old_inst) = snapshot.old_inst() {
        // Build old_ref → InstanceWithMeta lookup for metadata access.
        let mut old_meta_map: HashMap<Ref, _> = HashMap::new();
        for &child_ref in old_inst.children() {
            let inst = snapshot.get_old_instance(child_ref).unwrap();
            old_meta_map.insert(child_ref, inst);
        }

        // Filter old children: exclude those without disk presence, from
        // project files, or with ignored classes.
        let eligible_old: Vec<Ref> = old_inst
            .children()
            .iter()
            .copied()
            .filter(|r| {
                let inst = old_meta_map.get(r).unwrap();
                if inst.metadata().relevant_paths.is_empty() {
                    log::debug!(
                        "Excluding old instance {} from matching (no disk presence)",
                        inst.name()
                    );
                    return false;
                }
                if matches!(
                    inst.metadata().instigating_source,
                    Some(InstigatingSource::ProjectNode { .. })
                ) {
                    log::debug!(
                        "Excluding old instance {} from matching (project file)",
                        inst.name()
                    );
                    return false;
                }
                if snapshot.should_ignore_class(inst.class_name().as_str()) {
                    return false;
                }
                true
            })
            .collect();

        // Run the 3-pass matching algorithm to pair new ↔ old children.
        let match_result = match_children(
            &eligible_new,
            &eligible_old,
            snapshot.new_tree(),
            snapshot.old_tree(),
            None, // TODO: pass precomputed hashes for better similarity
            None,
        );

        // Pre-seed taken_names from ALL matched old children's filesystem
        // names so that unmatched new children correctly dedup.
        if snapshot.data.is_incremental() {
            for &(_, old_ref) in &match_result.matched {
                if let Some(inst) = old_meta_map.get(&old_ref) {
                    if let Some(path) = inst.metadata().relevant_paths.first() {
                        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                            taken_names.insert(filename.to_lowercase());
                        }
                    }
                }
            }
            // Also seed from unmatched old (they still occupy filesystem names).
            for &old_ref in &match_result.unmatched_old {
                if let Some(inst) = old_meta_map.get(&old_ref) {
                    if let Some(path) = inst.metadata().relevant_paths.first() {
                        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                            taken_names.insert(filename.to_lowercase());
                        }
                    }
                }
            }
        }

        // Sort matched pairs by new child name for deterministic ordering.
        let mut sorted_matched = match_result.matched;
        sorted_matched.sort_by(|(a, _), (b, _)| {
            let a_name = &snapshot.get_new_instance(*a).unwrap().name;
            let b_name = &snapshot.get_new_instance(*b).unwrap().name;
            a_name.cmp(b_name)
        });

        // Process matched pairs: preserve existing filesystem assignment.
        for (new_ref, old_ref) in &sorted_matched {
            let (child_snap, _needs_meta, dedup_key) = snapshot.with_joined_path(
                *new_ref,
                Some(*old_ref),
                &taken_names,
            )?;
            taken_names.insert(dedup_key.to_lowercase());
            children.push(child_snap);
        }

        // Sort unmatched new by name for deterministic dedup ordering.
        let mut sorted_unmatched_new = match_result.unmatched_new;
        sorted_unmatched_new.sort_by(|a, b| {
            let a_name = &snapshot.get_new_instance(*a).unwrap().name;
            let b_name = &snapshot.get_new_instance(*b).unwrap().name;
            a_name.cmp(b_name)
        });

        // Process unmatched new children: generate new filenames.
        for new_ref in &sorted_unmatched_new {
            let (child_snap, _needs_meta, dedup_key) =
                snapshot.with_joined_path(*new_ref, None, &taken_names)?;
            taken_names.insert(dedup_key.to_lowercase());
            children.push(child_snap);
        }

        // Unmatched old children are removed (instance was deleted).
        // Filter out ignored classes from removal.
        for old_ref in match_result.unmatched_old {
            if let Some(inst) = old_meta_map.get(&old_ref) {
                if !snapshot.should_ignore_class(inst.class_name().as_str()) {
                    removed_children.push(*inst);
                } else {
                    log::debug!(
                        "Not removing instance {} because its class {} is ignored",
                        inst.name(),
                        inst.class_name()
                    );
                }
            }
        }
    } else {
        // No old instance (clean mode). All children are new.
        // Sort alphabetically for deterministic dedup ordering.
        let mut sorted_new = eligible_new;
        sorted_new.sort_by(|a, b| {
            let a_name = &snapshot.get_new_instance(*a).unwrap().name;
            let b_name = &snapshot.get_new_instance(*b).unwrap().name;
            a_name.cmp(b_name)
        });

        for new_child_ref in &sorted_new {
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
