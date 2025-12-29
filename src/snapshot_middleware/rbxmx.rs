use std::path::Path;

use anyhow::Context;
use memofs::Vfs;
use rbx_xml::EncodeOptions;

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot},
    syncback::{FsSnapshot, SyncbackReturn, SyncbackSnapshot},
};

use super::rbxm::clone_tree_filtered;

pub fn snapshot_rbxmx(
    context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let options = rbx_xml::DecodeOptions::new()
        .property_behavior(rbx_xml::DecodePropertyBehavior::ReadUnknown);

    let temp_tree = rbx_xml::from_reader(vfs.read(path)?.as_slice(), options)
        .with_context(|| format!("Malformed rbxmx file: {}", path.display()))?;

    let root_instance = temp_tree.root();
    let children = root_instance.children();

    if children.len() == 1 {
        let child = children[0];
        let snapshot = InstanceSnapshot::from_tree(temp_tree, child)
            .name(name)
            .metadata(
                InstanceMetadata::new()
                    .instigating_source(path)
                    .relevant_paths(vec![path.to_path_buf()])
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

pub fn syncback_rbxmx<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let inst = snapshot.new_inst();
    let tree_globs = snapshot.compile_tree_globs();

    let options =
        EncodeOptions::new().property_behavior(rbx_xml::EncodePropertyBehavior::WriteUnknown);

    // If we have ignoreTrees patterns, filter the tree before serialization
    let serialized = if tree_globs.is_empty() {
        let mut serialized = Vec::new();
        rbx_xml::to_writer(
            &mut serialized,
            snapshot.new_tree(),
            &[inst.referent()],
            options,
        )
        .context("failed to serialize new rbxmx")?;
        serialized
    } else {
        // Clone the subtree, filtering out ignored instances
        let filtered_tree = clone_tree_filtered(
            snapshot.new_tree(),
            inst.referent(),
            &tree_globs,
        );
        let mut serialized = Vec::new();
        rbx_xml::to_writer(
            &mut serialized,
            &filtered_tree,
            &[filtered_tree.root_ref()],
            options,
        )
        .context("failed to serialize filtered rbxmx")?;
        serialized
    };

    Ok(SyncbackReturn {
        fs_snapshot: FsSnapshot::new().with_added_file(&snapshot.path, serialized),
        children: Vec::new(),
        removed_children: Vec::new(),
    })
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn plain_folder() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.rbxmx",
            VfsSnapshot::file(
                r#"
                    <roblox version="4">
                        <Item class="Folder" referent="0">
                            <Properties>
                                <string name="Name">THIS NAME IS IGNORED</string>
                            </Properties>
                        </Item>
                    </roblox>
                "#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_rbxmx(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.rbxmx"),
            "foo",
        )
        .unwrap()
        .unwrap();

        assert_eq!(instance_snapshot.name, "foo");
        assert_eq!(instance_snapshot.class_name, "Folder");
        assert_eq!(instance_snapshot.properties, Default::default());
        assert_eq!(instance_snapshot.children, Vec::new());
    }
}
