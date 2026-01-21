use std::path::Path;

use anyhow::Context;
use memofs::Vfs;
use rbx_dom_weak::{types::Ref, InstanceBuilder, WeakDom};

use crate::{
    glob::Glob,
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot},
    syncback::{inst_path, FsSnapshot, SyncbackReturn, SyncbackSnapshot},
};

#[profiling::function]
pub fn snapshot_rbxm(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let temp_tree = rbx_binary::from_reader(vfs.read(path)?.as_slice())
        .with_context(|| format!("Malformed rbxm file: {}", path.display()))?;

    let root_instance = temp_tree.root();
    let children = root_instance.children();

    if children.len() == 1 {
        let child = children[0];
        let snapshot = InstanceSnapshot::from_tree(temp_tree, child)
            .name(name)
            .metadata(
                InstanceMetadata::new()
                    .instigating_source(path)
                    .relevant_paths(vec![vfs.canonicalize(path)?])
                    .context(context),
            );

        Ok(Some(snapshot))
    } else {
        anyhow::bail!(
            "Rojo currently only supports model files with one top-level instance.\n\n \
             Check the model file at path {}",
            path.display()
        );
    }
}

pub fn syncback_rbxm<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let inst = snapshot.new_inst();
    let tree_globs = snapshot.compile_tree_globs();

    // If we have ignoreTrees patterns, filter the tree before serialization
    let serialized = if tree_globs.is_empty() {
        let mut serialized = Vec::new();
        rbx_binary::to_writer(&mut serialized, snapshot.new_tree(), &[inst.referent()])
            .context("failed to serialize new rbxm")?;
        serialized
    } else {
        // Clone the subtree, filtering out ignored instances
        let filtered_tree = clone_tree_filtered(
            snapshot.new_tree(),
            inst.referent(),
            &tree_globs,
        );
        let mut serialized = Vec::new();
        rbx_binary::to_writer(&mut serialized, &filtered_tree, &[filtered_tree.root_ref()])
            .context("failed to serialize filtered rbxm")?;
        serialized
    };

    Ok(SyncbackReturn {
        fs_snapshot: FsSnapshot::new().with_added_file(&snapshot.path, serialized),
        children: Vec::new(),
        removed_children: Vec::new(),
    })
}

/// Clones a subtree from the source WeakDom, filtering out instances that match
/// the provided ignoreTrees glob patterns.
pub fn clone_tree_filtered(source: &WeakDom, root_ref: Ref, ignore_globs: &[Glob]) -> WeakDom {
    let root_inst = source.get_by_ref(root_ref).expect("root ref should exist");

    // Create a new tree with the root instance
    let mut builder = InstanceBuilder::new(root_inst.class)
        .with_name(root_inst.name.clone())
        .with_properties(root_inst.properties.clone());

    // Recursively add children, filtering out ignored ones
    add_children_filtered(source, root_ref, &mut builder, ignore_globs);

    WeakDom::new(builder)
}

/// Recursively adds children to an InstanceBuilder, filtering out ignored instances.
fn add_children_filtered(
    source: &WeakDom,
    parent_ref: Ref,
    builder: &mut InstanceBuilder,
    ignore_globs: &[Glob],
) {
    let parent = source.get_by_ref(parent_ref).expect("parent ref should exist");

    for &child_ref in parent.children() {
        let child_path = inst_path(source, child_ref);

        // Check if this child should be ignored
        let should_ignore = ignore_globs.iter().any(|glob| glob.is_match(&child_path));

        if should_ignore {
            log::debug!("Filtering out {child_path} from rbxm due to ignoreTrees pattern");
            continue;
        }

        let child = source.get_by_ref(child_ref).expect("child ref should exist");

        let mut child_builder = InstanceBuilder::new(child.class)
            .with_name(child.name.clone())
            .with_properties(child.properties.clone());

        // Recursively add this child's children
        add_children_filtered(source, child_ref, &mut child_builder, ignore_globs);

        builder.add_child(child_builder);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn model_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.rbxm",
            VfsSnapshot::file(include_bytes!("../../assets/test-folder.rbxm").to_vec()),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_rbxm(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.rbxm"),
            "foo",
        )
        .unwrap()
        .unwrap();

        assert_eq!(instance_snapshot.name, "foo");
        assert_eq!(instance_snapshot.class_name, "Folder");
        assert_eq!(instance_snapshot.children, Vec::new());

        // We intentionally don't assert on properties. rbx_binary does not
        // distinguish between String and BinaryString. The sample model was
        // created by Roblox Studio and has an empty BinaryString "Tags"
        // property that currently deserializes incorrectly.
        // See: https://github.com/Roblox/rbx-dom/issues/49
    }
}
