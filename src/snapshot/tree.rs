use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

use rbx_dom_weak::{
    types::{Ref, Variant},
    ustr, Instance, InstanceBuilder, Ustr, UstrMap, WeakDom,
};

use crate::{multimap::MultiMap, RojoRef};

use super::{InstanceMetadata, InstanceSnapshot, InstigatingSource};

#[inline]
pub(crate) fn is_script_class(class_name: &str) -> bool {
    matches!(class_name, "Script" | "LocalScript" | "ModuleScript")
}

/// An expanded variant of rbx_dom_weak's `WeakDom` that tracks additional
/// metadata per instance that's Rojo-specific.
///
/// This tree is also optimized for doing fast incremental updates and patches.
#[derive(Debug)]
pub struct RojoTree {
    /// Contains the instances without their Rojo-specific metadata.
    inner: WeakDom,

    /// Metadata associated with each instance that is kept up-to-date with the
    /// set of actual instances.
    metadata_map: HashMap<Ref, InstanceMetadata>,

    /// A multimap from source paths to all of the root instances that were
    /// constructed from that path.
    ///
    /// Descendants of those instances should not be contained in the set, the
    /// value portion of the map is also a set in order to support the same path
    /// appearing multiple times in the same Rojo project. This is sometimes
    /// called "path aliasing" in various Rojo documentation.
    path_to_ids: MultiMap<PathBuf, Ref>,

    /// A map of specified RojoRefs to underlying Refs they represent.
    /// This field is a MultiMap to allow for the possibility of the user specifying
    /// the same RojoRef for multiple different instances. An entry containing
    /// multiple elements is an error condition that should be raised to the user.
    specified_id_to_refs: MultiMap<RojoRef, Ref>,

    /// Refs of all Script, LocalScript, and ModuleScript instances in the tree.
    /// Maintained incrementally for fast scripts-only mode filtering in the
    /// serve API without walking the entire tree.
    script_refs: HashSet<Ref>,
}

impl RojoTree {
    pub fn new(snapshot: InstanceSnapshot) -> RojoTree {
        let root_builder = InstanceBuilder::new(snapshot.class_name)
            .with_name(snapshot.name)
            .with_properties(snapshot.properties);

        let mut tree = RojoTree {
            inner: WeakDom::new(root_builder),
            metadata_map: HashMap::new(),
            path_to_ids: MultiMap::new(),
            specified_id_to_refs: MultiMap::new(),
            script_refs: HashSet::new(),
        };

        let root_ref = tree.inner.root_ref();

        tree.insert_metadata(root_ref, snapshot.metadata);

        if is_script_class(snapshot.class_name.as_ref()) {
            tree.script_refs.insert(root_ref);
        }

        for child in snapshot.children {
            tree.insert_instance(root_ref, child);
        }

        tree
    }

    pub fn inner(&self) -> &WeakDom {
        &self.inner
    }

    pub fn get_root_id(&self) -> Ref {
        self.inner.root_ref()
    }

    /// Returns the root Instance of this tree.
    #[inline]
    pub fn root(&self) -> InstanceWithMeta<'_> {
        self.get_instance(self.get_root_id())
            .expect("RojoTrees should have a root")
    }

    pub fn get_instance(&self, id: Ref) -> Option<InstanceWithMeta<'_>> {
        if let Some(instance) = self.inner.get_by_ref(id) {
            let metadata = self.metadata_map.get(&id).unwrap();

            Some(InstanceWithMeta { instance, metadata })
        } else {
            None
        }
    }

    pub fn get_instance_mut(&mut self, id: Ref) -> Option<InstanceWithMetaMut<'_>> {
        if let Some(instance) = self.inner.get_by_ref_mut(id) {
            let metadata = self.metadata_map.get_mut(&id).unwrap();

            Some(InstanceWithMetaMut { instance, metadata })
        } else {
            None
        }
    }

    pub fn insert_instance(&mut self, parent_ref: Ref, snapshot: InstanceSnapshot) -> Ref {
        // !!!!!!!!!! UGLY HACK !!!!!!!!!!
        // ! If you are going to change this, go change it in patch_compute/compute_property_patches too
        //
        // This is a set of special cases working around a more general problem upstream
        // in rbx-dom that causes pivots to not build to file correctly, described in
        // github.com/rojo-rbx/rojo/issues/628 and github.com/UserGeneratedLLC/rbx-dom/issues/385
        //
        // We need to insert the NeedsPivotMigration property with a value of false on
        // every instance that inherits from Model for pivots to build correctly.
        let hack_needs_pivot_migration = match snapshot.class_name.as_ref() {
            // This is not a future proof way to do this but the last time a
            // descendant of Model was added was in 2020 so it's probably fine.
            "Model" | "Actor" | "Tool" | "HopperBin" | "Flag" | "WorldModel" | "Workspace"
            | "Status"
                if !snapshot
                    .properties
                    .contains_key(&ustr("NeedsPivotMigration")) =>
            {
                vec![("NeedsPivotMigration", Variant::Bool(false))]
            }
            _ => Vec::new(),
        };

        let is_script = is_script_class(snapshot.class_name.as_ref());

        let builder = InstanceBuilder::empty()
            .with_class(snapshot.class_name)
            .with_name(snapshot.name.into_owned())
            .with_properties(hack_needs_pivot_migration)
            .with_properties(snapshot.properties);

        let referent = self.inner.insert(parent_ref, builder);
        self.insert_metadata(referent, snapshot.metadata);

        if is_script {
            self.script_refs.insert(referent);
        }

        for child in snapshot.children {
            self.insert_instance(referent, child);
        }

        referent
    }

    pub fn remove(&mut self, id: Ref) {
        if self.inner.get_by_ref(id).is_none() {
            return;
        }

        let mut to_move = VecDeque::new();
        to_move.push_back(id);

        while let Some(id) = to_move.pop_front() {
            self.remove_metadata(id);
            self.script_refs.remove(&id);

            if let Some(instance) = self.inner.get_by_ref(id) {
                to_move.extend(instance.children().iter().copied());
            }
        }

        self.inner.destroy(id);
    }

    /// Replaces the metadata associated with the given instance ID.
    pub fn update_metadata(&mut self, id: Ref, metadata: InstanceMetadata) {
        use std::collections::hash_map::Entry;

        match self.metadata_map.entry(id) {
            Entry::Occupied(mut entry) => {
                let existing_metadata = entry.get();

                // If this instance's source path changed, we need to update our
                // path associations so that file changes will trigger updates
                // to this instance correctly.
                if existing_metadata.relevant_paths != metadata.relevant_paths {
                    for existing_path in &existing_metadata.relevant_paths {
                        self.path_to_ids.remove(existing_path, id);
                    }

                    for new_path in &metadata.relevant_paths {
                        self.path_to_ids.insert(new_path.clone(), id);
                    }
                }
                if existing_metadata.specified_id != metadata.specified_id {
                    // We need to uphold the invariant that each ID can only map
                    // to one referent.
                    if let Some(new) = &metadata.specified_id {
                        if !self.specified_id_to_refs.get(new).is_empty() {
                            log::error!("Duplicate user-specified referent '{new}'");
                        }

                        self.specified_id_to_refs.insert(new.clone(), id);
                    }
                    if let Some(old) = &existing_metadata.specified_id {
                        self.specified_id_to_refs.remove(old, id);
                    }
                }

                entry.insert(metadata);
            }
            Entry::Vacant(entry) => {
                entry.insert(metadata);
            }
        }
    }

    pub fn descendants(&self, id: Ref) -> RojoDescendants<'_> {
        let mut queue = VecDeque::new();
        queue.push_back(id);

        RojoDescendants { queue, tree: self }
    }

    pub fn get_ids_at_path(&self, path: &Path) -> &[Ref] {
        self.path_to_ids.get(path)
    }

    pub fn get_metadata(&self, id: Ref) -> Option<&InstanceMetadata> {
        self.metadata_map.get(&id)
    }

    /// Get the backing Ref of the given RojoRef. If the RojoRef maps to exactly
    /// one Ref, this method returns Some. Otherwise, it returns None.
    pub fn get_specified_id(&self, specified: &RojoRef) -> Option<Ref> {
        match self.specified_id_to_refs.get(specified)[..] {
            [referent] => Some(referent),
            _ => None,
        }
    }

    pub fn set_specified_id(&mut self, id: Ref, specified: RojoRef) {
        if let Some(metadata) = self.metadata_map.get_mut(&id) {
            if let Some(old) = metadata.specified_id.replace(specified.clone()) {
                self.specified_id_to_refs.remove(&old, id);
            }
        }
        self.specified_id_to_refs.insert(specified, id);
    }

    pub fn script_refs(&self) -> &HashSet<Ref> {
        &self.script_refs
    }

    pub fn update_script_tracking(&mut self, id: Ref, old_class: &str, new_class: &str) {
        let was_script = is_script_class(old_class);
        let is_script = is_script_class(new_class);
        if was_script && !is_script {
            self.script_refs.remove(&id);
        } else if !was_script && is_script {
            self.script_refs.insert(id);
        }
    }

    /// Looks up an instance by its filesystem-name path in the tree.
    ///
    /// Each segment is matched against the **filesystem name** of child
    /// instances (derived from `instigating_source` path), falling back to
    /// the instance name for instances without filesystem backing.
    ///
    /// Path format: `"Workspace/Model.model.json5/Attachment~2.luau"`
    /// Segments are separated by `/` (no escaping needed since filesystem
    /// names can't contain `/`).
    pub fn get_instance_by_path(&self, path: &str) -> Option<Ref> {
        if path.is_empty() {
            return Some(self.get_root_id());
        }

        let segments: Vec<&str> = path.split('/').collect();
        let mut current_ref = self.get_root_id();

        for segment in &segments {
            let current = self.inner.get_by_ref(current_ref)?;
            let children = current.children();

            let mut found = false;
            for &child_ref in children {
                let fs_name = self.filesystem_name_for(child_ref);
                if fs_name.eq_ignore_ascii_case(segment) {
                    current_ref = child_ref;
                    found = true;
                    break;
                }
            }

            if !found {
                // Fallback: try matching by instance name (for backward
                // compatibility with legacy ref paths and instances without
                // filesystem backing). Case-insensitive to match the primary
                // lookup semantics.
                for &child_ref in children {
                    let child = self.inner.get_by_ref(child_ref)?;
                    if child.name.eq_ignore_ascii_case(segment) {
                        current_ref = child_ref;
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                return None;
            }
        }

        Some(current_ref)
    }

    /// Resolve a Luau require-by-string style ref path to a target instance.
    ///
    /// `source_ref` is the instance that owns the `Rojo_Ref_*` attribute.
    /// Prefix semantics (per Roblox require-by-string):
    /// - `@game/rest` -- resolve from DataModel root
    /// - `@self` -- return source_ref
    /// - `@self/rest` -- resolve from source_ref
    /// - `./rest` -- resolve from source_ref's parent
    /// - `../rest` -- resolve from source_ref's grandparent
    /// - bare path (no prefix) -- legacy fallback, resolve like `@game/`
    ///
    /// During segment walk, `..` navigates to the parent.
    pub fn resolve_ref_path(&self, path: &str, source_ref: Ref) -> Option<Ref> {
        if let Some(rest) = path.strip_prefix("@game/") {
            return self.walk_segments(self.get_root_id(), rest);
        }
        if path == "@game" {
            return Some(self.get_root_id());
        }
        if path == "@self" {
            return Some(source_ref);
        }
        if let Some(rest) = path.strip_prefix("@self/") {
            return self.walk_segments(source_ref, rest);
        }
        if let Some(rest) = path.strip_prefix("./") {
            let parent = self.inner.get_by_ref(source_ref)?.parent();
            if parent.is_none() {
                return None;
            }
            return self.walk_segments(parent, rest);
        }
        if let Some(rest) = path.strip_prefix("../") {
            let parent = self.inner.get_by_ref(source_ref)?.parent();
            if parent.is_none() {
                return None;
            }
            let grandparent = self.inner.get_by_ref(parent)?.parent();
            if grandparent.is_none() {
                return None;
            }
            return self.walk_segments(grandparent, rest);
        }

        // Bare path (no prefix) -- legacy backward compatibility
        self.walk_segments(self.get_root_id(), path)
    }

    /// Walk `/`-separated segments from a starting Ref, handling `..` as
    /// "go to parent". Each non-`..` segment matches a child by filesystem
    /// name (case-insensitive), falling back to instance name.
    fn walk_segments(&self, start: Ref, rest: &str) -> Option<Ref> {
        if rest.is_empty() {
            return Some(start);
        }

        let mut current_ref = start;
        for segment in rest.split('/') {
            if segment.is_empty() {
                continue;
            }
            if segment == ".." {
                let inst = self.inner.get_by_ref(current_ref)?;
                let parent = inst.parent();
                if parent.is_none() {
                    return None;
                }
                current_ref = parent;
                continue;
            }

            let current = self.inner.get_by_ref(current_ref)?;
            let children = current.children();

            let mut found = false;
            for &child_ref in children {
                let fs_name = self.filesystem_name_for(child_ref);
                if fs_name.eq_ignore_ascii_case(segment) {
                    current_ref = child_ref;
                    found = true;
                    break;
                }
            }

            if !found {
                for &child_ref in children {
                    let child = self.inner.get_by_ref(child_ref)?;
                    if child.name.eq_ignore_ascii_case(segment) {
                        current_ref = child_ref;
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                return None;
            }
        }

        Some(current_ref)
    }

    /// Returns the filesystem name for an instance (the filename or directory
    /// name on disk). Derived from `instigating_source` if available, otherwise
    /// falls back to the instance name.
    pub fn filesystem_name_for(&self, id: Ref) -> String {
        if let Some(meta) = self.metadata_map.get(&id) {
            if let Some(source) = &meta.instigating_source {
                match source {
                    InstigatingSource::Path(_) => {
                        if let Some(name) = source.path().file_name().and_then(|f| f.to_str()) {
                            return name.to_string();
                        }
                    }
                    InstigatingSource::ProjectNode { .. } => {
                        // Project-sourced instance: use instance name, not the
                        // project file path (which would be e.g. "default.project.json5").
                        if let Some(inst) = self.inner.get_by_ref(id) {
                            return inst.name.clone();
                        }
                    }
                }
            }
        }
        // Fallback: use instance name
        self.inner
            .get_by_ref(id)
            .map(|inst| inst.name.clone())
            .unwrap_or_default()
    }

    fn insert_metadata(&mut self, id: Ref, metadata: InstanceMetadata) {
        for path in &metadata.relevant_paths {
            self.path_to_ids.insert(path.clone(), id);
        }

        if let Some(specified_id) = &metadata.specified_id {
            if !self.specified_id_to_refs.get(specified_id).is_empty() {
                log::error!("Duplicate user-specified referent '{specified_id}'");
            }

            self.set_specified_id(id, specified_id.clone());
        }

        self.metadata_map.insert(id, metadata);
    }

    /// Moves the Rojo metadata from the instance with the given ID from this
    /// tree into some loose maps.
    fn remove_metadata(&mut self, id: Ref) {
        let Some(metadata) = self.metadata_map.remove(&id) else {
            return;
        };

        if let Some(specified) = metadata.specified_id {
            self.specified_id_to_refs.remove(&specified, id);
        }

        for path in &metadata.relevant_paths {
            self.path_to_ids.remove(path, id);
        }
    }
}

pub struct RojoDescendants<'a> {
    queue: VecDeque<Ref>,
    tree: &'a RojoTree,
}

impl<'a> Iterator for RojoDescendants<'a> {
    type Item = InstanceWithMeta<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.queue.pop_front()?;

        let instance = self
            .tree
            .inner
            .get_by_ref(id)
            .expect("Instance did not exist");

        let metadata = self
            .tree
            .get_metadata(instance.referent())
            .expect("Metadata did not exist for instance");

        self.queue.extend(instance.children().iter().copied());

        Some(InstanceWithMeta { instance, metadata })
    }
}

/// RojoTree's equivalent of `&'a Instance`.
///
/// This has to be a value type for RojoTree because the instance and metadata
/// are stored in different places. The mutable equivalent is
/// `InstanceWithMetaMut`.
#[derive(Debug, Clone, Copy)]
pub struct InstanceWithMeta<'a> {
    instance: &'a Instance,
    metadata: &'a InstanceMetadata,
}

impl<'a> InstanceWithMeta<'a> {
    pub fn id(&self) -> Ref {
        self.instance.referent()
    }

    pub fn parent(&self) -> Ref {
        self.instance.parent()
    }

    pub fn name(&self) -> &'a str {
        &self.instance.name
    }

    pub fn class_name(&self) -> Ustr {
        self.instance.class
    }

    pub fn properties(&self) -> &'a UstrMap<Variant> {
        &self.instance.properties
    }

    pub fn children(&self) -> &'a [Ref] {
        self.instance.children()
    }

    pub fn metadata(&self) -> &'a InstanceMetadata {
        self.metadata
    }

    pub fn inner(&self) -> &Instance {
        self.instance
    }
}

/// RojoTree's equivalent of `&'a mut Instance`.
///
/// This has to be a value type for RojoTree because the instance and metadata
/// are stored in different places. The immutable equivalent is
/// `InstanceWithMeta`.
#[derive(Debug)]
pub struct InstanceWithMetaMut<'a> {
    instance: &'a mut Instance,
    metadata: &'a mut InstanceMetadata,
}

impl InstanceWithMetaMut<'_> {
    pub fn id(&self) -> Ref {
        self.instance.referent()
    }

    pub fn name(&self) -> &str {
        &self.instance.name
    }

    pub fn name_mut(&mut self) -> &mut String {
        &mut self.instance.name
    }

    pub fn class_name(&self) -> &str {
        &self.instance.class
    }

    pub fn set_class_name<'a, S: Into<&'a str>>(&mut self, new_class: S) {
        self.instance.class = ustr(new_class.into());
    }

    pub fn properties(&self) -> &UstrMap<Variant> {
        &self.instance.properties
    }

    pub fn properties_mut(&mut self) -> &mut UstrMap<Variant> {
        &mut self.instance.properties
    }

    pub fn children(&self) -> &[Ref] {
        self.instance.children()
    }

    pub fn metadata(&self) -> &InstanceMetadata {
        self.metadata
    }

    pub fn inner(&self) -> &Instance {
        self.instance
    }

    pub fn inner_mut(&mut self) -> &mut Instance {
        self.instance
    }
}

#[cfg(test)]
mod test {
    use crate::{
        snapshot::{InstanceMetadata, InstanceSnapshot},
        RojoRef,
    };

    use super::RojoTree;

    #[test]
    fn update_script_tracking_adds_and_removes() {
        let snapshot = InstanceSnapshot::new().name("Root").class_name("DataModel");
        let child = InstanceSnapshot::new()
            .name("TestFolder")
            .class_name("Folder");

        let mut tree = RojoTree::new(snapshot);
        let root = tree.get_root_id();
        let folder_id = tree.insert_instance(root, child);

        assert!(!tree.script_refs().contains(&folder_id));

        tree.update_script_tracking(folder_id, "Folder", "ModuleScript");
        assert!(tree.script_refs().contains(&folder_id));

        tree.update_script_tracking(folder_id, "ModuleScript", "Script");
        assert!(tree.script_refs().contains(&folder_id));

        tree.update_script_tracking(folder_id, "Script", "Folder");
        assert!(!tree.script_refs().contains(&folder_id));
    }

    #[test]
    fn swap_duped_specified_ids() {
        let custom_ref = RojoRef::new("MyCoolRef".into());
        let snapshot = InstanceSnapshot::new()
            .metadata(InstanceMetadata::new().specified_id(Some(custom_ref.clone())));
        let mut tree = RojoTree::new(InstanceSnapshot::new());

        let original = tree.insert_instance(tree.get_root_id(), snapshot.clone());
        assert_eq!(tree.get_specified_id(&custom_ref.clone()), Some(original));

        let duped = tree.insert_instance(tree.get_root_id(), snapshot.clone());
        assert_eq!(tree.get_specified_id(&custom_ref.clone()), None);

        tree.remove(original);
        assert_eq!(tree.get_specified_id(&custom_ref.clone()), Some(duped));
    }
}
