use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use rbx_dom_weak::{
    types::{Ref, Variant},
    WeakDom,
};
use serde::{Deserialize, Serialize};

/// Legacy: ID attribute on target instances (kept for backwards compatibility)
pub const REF_ID_ATTRIBUTE_NAME: &str = "Rojo_Id";
/// Legacy: Pointer attribute prefix using IDs (kept for backwards compatibility)
pub const REF_POINTER_ATTRIBUTE_PREFIX: &str = "Rojo_Target_";
/// Prefix for path-based reference attributes. This is the preferred system
/// that stores the path to the target instance (e.g., "SoundService/Effects").
/// No modification to the target instance is needed.
pub const REF_PATH_ATTRIBUTE_PREFIX: &str = "Rojo_Ref_";

/// Compute the `Rojo_Ref_*` attribute name for a given property name.
///
/// Example: `ref_attribute_name("PrimaryPart")` returns `"Rojo_Ref_PrimaryPart"`.
pub fn ref_attribute_name(prop_name: &str) -> String {
    format!("{REF_PATH_ATTRIBUTE_PREFIX}{prop_name}")
}

/// Compute the `Rojo_Target_*` attribute name for a given property name.
///
/// Example: `ref_target_attribute_name("Value")` returns `"Rojo_Target_Value"`.
pub fn ref_target_attribute_name(prop_name: &str) -> String {
    format!("{REF_POINTER_ATTRIBUTE_PREFIX}{prop_name}")
}

/// Escape "/" in an instance name for use in ref paths.
/// Uses "\/" as escape sequence. Also escapes existing backslashes to "\\"
/// so the escaping is unambiguous.
pub fn escape_ref_path_segment(name: &str) -> Cow<'_, str> {
    if name.contains('/') || name.contains('\\') {
        Cow::Owned(name.replace('\\', "\\\\").replace('/', "\\/"))
    } else {
        Cow::Borrowed(name)
    }
}

/// Unescape a ref path segment back to an instance name.
/// Reverses the escaping done by `escape_ref_path_segment`.
pub fn unescape_ref_path_segment(segment: &str) -> Cow<'_, str> {
    if segment.contains('\\') {
        let mut result = String::with_capacity(segment.len());
        let mut chars = segment.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('/') => result.push('/'),
                    Some('\\') => result.push('\\'),
                    Some(other) => {
                        result.push('\\');
                        result.push(other);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }
        Cow::Owned(result)
    } else {
        Cow::Borrowed(segment)
    }
}

/// Split a ref path string into segments, handling escaped "/" characters.
/// Splits on unescaped "/" only, then unescapes each segment.
pub fn split_ref_path(path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Escaped character -- keep both the backslash and next char
            current.push(c);
            if let Some(&next) = chars.peek() {
                current.push(next);
                chars.next();
            }
        } else if c == '/' {
            // Unescaped separator
            segments.push(unescape_ref_path_segment(&current).into_owned());
            current = String::new();
        } else {
            current.push(c);
        }
    }

    // Push the last segment
    segments.push(unescape_ref_path_segment(&current).into_owned());

    segments
}

/// Compute the path to a target instance for use in a `Rojo_Ref_*` attribute.
/// Returns the slash-separated path from the root (root name excluded).
/// Instance names containing "/" are escaped as "\/" to prevent ambiguity.
///
/// **Legacy version:** Uses raw instance names with escaping. New code should
/// use `ref_target_path_from_tree()` which uses filesystem names.
///
/// Example: an instance at `DataModel > Workspace > Part1` returns `"Workspace/Part1"`.
/// An instance named `"A/B"` at Workspace returns `"Workspace/A\/B"`.
pub fn ref_target_path(dom: &WeakDom, target_ref: Ref) -> String {
    let root_ref = dom.root_ref();
    let mut components: Vec<Cow<'_, str>> = dom
        .ancestors_of(target_ref)
        .filter(|inst| inst.referent() != root_ref)
        .map(|inst| escape_ref_path_segment(&inst.name))
        .collect();
    components.reverse();
    components.join("/")
}

/// Compute the filesystem-name-based path to a target instance for use in
/// `Rojo_Ref_*` attributes.
///
/// Each path segment is the **full filesystem name** of the instance:
/// - `Folder "Foo"` → `"Foo"` (directory name)
/// - `ModuleScript "Foo"` → `"Foo.luau"` (slug + extension)
/// - `Script "Foo"` (server) → `"Foo.server.luau"`
/// - Dedup'd `ModuleScript "Foo"` → `"Foo~2.luau"`
///
/// Filesystem names are derived from `instigating_source` paths (the actual
/// file/dir on disk). For instances without filesystem backing (inside .rbxm
/// files, newly added), falls back to the instance name.
///
/// Since filesystem names can't contain `/` (slugified to `_`), path splitting
/// is a simple `split('/')` -- no escaping needed.
pub fn ref_target_path_from_tree(tree: &crate::snapshot::RojoTree, target_ref: Ref) -> String {
    let dom = tree.inner();
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

        // Derive the filesystem name for this instance.
        let fs_name = if let Some(meta) = tree.get_metadata(current) {
            if let Some(source) = &meta.instigating_source {
                match source {
                    crate::snapshot::InstigatingSource::Path(_) => {
                        // File-backed instance: use the filename from disk.
                        source
                            .path()
                            .file_name()
                            .and_then(|f| f.to_str())
                            .unwrap_or(&inst.name)
                            .to_string()
                    }
                    crate::snapshot::InstigatingSource::ProjectNode { .. } => {
                        // Project-sourced instance: use instance name, not the
                        // project file path (which would be e.g. "default.project.json5").
                        inst.name.clone()
                    }
                }
            } else {
                // No instigating source (newly added) -- use name.
                inst.name.clone()
            }
        } else {
            // No metadata at all -- use instance name.
            inst.name.clone()
        };

        components.push(fs_name);
        current = inst.parent();
    }

    components.reverse();
    components.join("/")
}

/// Extract a string value from a Variant that may be String or BinaryString.
/// Returns None with a warning for non-string types or invalid UTF-8.
/// Used by compute_ref_properties and defer_ref_properties for parsing
/// Rojo_Ref_* and Rojo_Target_* attribute values.
pub fn variant_as_str<'a>(value: &'a Variant, attr_name: &str) -> Option<&'a str> {
    match value {
        Variant::String(s) => Some(s.as_str()),
        Variant::BinaryString(bytes) => match std::str::from_utf8(bytes.as_ref()) {
            Ok(s) => Some(s),
            Err(_) => {
                log::warn!("Attribute {attr_name} contains invalid UTF-8 BinaryString");
                None
            }
        },
        _ => {
            log::warn!(
                "Attribute {attr_name} is of type {:?} when it was expected to be a String",
                value.ty()
            );
            None
        }
    }
}

// TODO add an internment strategy for RojoRefs
// Something like what rbx-dom does for SharedStrings probably works

#[derive(Debug, Default, PartialEq, Hash, Clone, Serialize, Deserialize, Eq)]
pub struct RojoRef(Arc<String>);

impl RojoRef {
    #[inline]
    pub fn new(id: String) -> Self {
        Self(Arc::from(id))
    }

    #[inline]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for RojoRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Index of meta/model files that contain `Rojo_Ref_*` attributes.
///
/// Maps the `Rojo_Ref_*` attribute VALUE (the instance path string) to the
/// set of filesystem paths (meta/model files) where that attribute appears.
/// This allows `update_ref_paths_after_rename` to find affected files in
/// O(affected_files) instead of O(tree_size).
#[derive(Debug, Default)]
pub struct RefPathIndex {
    paths_to_files: HashMap<String, HashSet<PathBuf>>,
}

impl RefPathIndex {
    pub fn new() -> Self {
        Self {
            paths_to_files: HashMap::new(),
        }
    }

    /// Record that `meta_file` contains a `Rojo_Ref_*` attribute with value
    /// `ref_path` (the instance path string, e.g., "Workspace/Model/Part1").
    pub fn add(&mut self, ref_path: &str, meta_file: &Path) {
        self.paths_to_files
            .entry(ref_path.to_string())
            .or_default()
            .insert(meta_file.to_path_buf());
    }

    /// Remove the record that `meta_file` contains a `Rojo_Ref_*` attribute
    /// with value `ref_path`. Called when an attribute is removed or its value
    /// changes.
    pub fn remove(&mut self, ref_path: &str, meta_file: &Path) {
        if let Some(files) = self.paths_to_files.get_mut(ref_path) {
            files.remove(meta_file);
            if files.is_empty() {
                self.paths_to_files.remove(ref_path);
            }
        }
    }

    /// Remove `meta_file` from ALL entries in the index.
    /// Used when re-indexing a file: first remove all old entries, then
    /// re-add entries for the attributes that remain.
    pub fn remove_all_for_file(&mut self, meta_file: &Path) {
        let mut empty_keys = Vec::new();
        for (path, files) in &mut self.paths_to_files {
            files.remove(meta_file);
            if files.is_empty() {
                empty_keys.push(path.clone());
            }
        }
        for key in empty_keys {
            self.paths_to_files.remove(&key);
        }
    }

    /// Find all meta/model files that contain a `Rojo_Ref_*` attribute whose
    /// value equals `prefix` or starts with `prefix/`. These are the files
    /// that need updating when an instance at `prefix` is renamed.
    pub fn find_by_prefix(&self, prefix: &str) -> Vec<PathBuf> {
        let prefix_with_sep = format!("{prefix}/");
        let mut result = Vec::new();
        for (path, files) in &self.paths_to_files {
            if path == prefix || path.starts_with(&prefix_with_sep) {
                result.extend(files.iter().cloned());
            }
        }
        // Deduplicate (a single file may have multiple Rojo_Ref_* attrs
        // matching the prefix)
        result.sort();
        result.dedup();
        result
    }

    /// Rename a file in all index entries (update the filesystem path).
    /// Called when a directory is renamed and the meta files move to new paths.
    pub fn rename_file(&mut self, old_file: &Path, new_file: &Path) {
        for files in self.paths_to_files.values_mut() {
            if files.remove(old_file) {
                files.insert(new_file.to_path_buf());
            }
        }
    }

    /// Update all index entries after a rename: replace `old_prefix` with
    /// `new_prefix` in all matching path keys.
    pub fn update_prefix(&mut self, old_prefix: &str, new_prefix: &str) {
        let old_with_sep = format!("{old_prefix}/");
        let mut to_rename: Vec<(String, String)> = Vec::new();
        for path in self.paths_to_files.keys() {
            if path == old_prefix {
                to_rename.push((path.clone(), new_prefix.to_string()));
            } else if path.starts_with(&old_with_sep) {
                let new_path = format!("{new_prefix}{}", &path[old_prefix.len()..]);
                to_rename.push((path.clone(), new_path));
            }
        }
        for (old_key, new_key) in to_rename {
            if let Some(files) = self.paths_to_files.remove(&old_key) {
                self.paths_to_files
                    .entry(new_key)
                    .or_default()
                    .extend(files);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbx_dom_weak::InstanceBuilder;

    // -----------------------------------------------------------------------
    // ref_attribute_name tests
    // -----------------------------------------------------------------------

    #[test]
    fn ref_attr_name_primary_part() {
        assert_eq!(ref_attribute_name("PrimaryPart"), "Rojo_Ref_PrimaryPart");
    }

    #[test]
    fn ref_attr_name_value() {
        assert_eq!(ref_attribute_name("Value"), "Rojo_Ref_Value");
    }

    #[test]
    fn ref_attr_name_part0() {
        assert_eq!(ref_attribute_name("Part0"), "Rojo_Ref_Part0");
    }

    #[test]
    fn ref_attr_name_attachment0() {
        assert_eq!(ref_attribute_name("Attachment0"), "Rojo_Ref_Attachment0");
    }

    #[test]
    fn ref_attr_name_adornee() {
        assert_eq!(ref_attribute_name("Adornee"), "Rojo_Ref_Adornee");
    }

    // -----------------------------------------------------------------------
    // ref_target_path tests
    // -----------------------------------------------------------------------

    #[test]
    fn ref_target_path_root_child() {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let child = dom.insert(dom.root_ref(), InstanceBuilder::new("Workspace"));
        assert_eq!(ref_target_path(&dom, child), "Workspace");
    }

    #[test]
    fn ref_target_path_deeply_nested() {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let a = dom.insert(dom.root_ref(), InstanceBuilder::new("A"));
        let b = dom.insert(a, InstanceBuilder::new("B"));
        let c = dom.insert(b, InstanceBuilder::new("C"));
        assert_eq!(ref_target_path(&dom, c), "A/B/C");
    }

    #[test]
    fn ref_target_path_root_is_empty() {
        let dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        assert_eq!(ref_target_path(&dom, dom.root_ref()), "");
    }

    // -----------------------------------------------------------------------
    // Escape/unescape tests
    // -----------------------------------------------------------------------

    #[test]
    fn escape_name_without_slash() {
        assert_eq!(escape_ref_path_segment("Part1"), Cow::Borrowed("Part1"));
    }

    #[test]
    fn escape_name_with_slash() {
        let escaped = escape_ref_path_segment("A/B");
        assert_eq!(escaped.as_ref(), "A\\/B");
    }

    #[test]
    fn escape_name_with_backslash() {
        let escaped = escape_ref_path_segment("A\\B");
        assert_eq!(escaped.as_ref(), "A\\\\B");
    }

    #[test]
    fn escape_name_with_both() {
        let escaped = escape_ref_path_segment("A/B\\C");
        assert_eq!(escaped.as_ref(), "A\\/B\\\\C");
    }

    #[test]
    fn unescape_plain_segment() {
        assert_eq!(unescape_ref_path_segment("Part1"), Cow::Borrowed("Part1"));
    }

    #[test]
    fn unescape_escaped_slash() {
        assert_eq!(unescape_ref_path_segment("A\\/B").as_ref(), "A/B");
    }

    #[test]
    fn unescape_escaped_backslash() {
        assert_eq!(unescape_ref_path_segment("A\\\\B").as_ref(), "A\\B");
    }

    #[test]
    fn escape_unescape_round_trip() {
        let original = "A/B\\C/D";
        let escaped = escape_ref_path_segment(original);
        let unescaped = unescape_ref_path_segment(&escaped);
        assert_eq!(unescaped.as_ref(), original);
    }

    // -----------------------------------------------------------------------
    // split_ref_path tests
    // -----------------------------------------------------------------------

    #[test]
    fn split_simple_path() {
        let segments = split_ref_path("Workspace/Model/Part");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], "Workspace");
        assert_eq!(segments[1], "Model");
        assert_eq!(segments[2], "Part");
    }

    #[test]
    fn split_path_with_escaped_slash() {
        let segments = split_ref_path("Workspace/A\\/B/Part");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], "Workspace");
        assert_eq!(segments[1], "A/B"); // unescaped
        assert_eq!(segments[2], "Part");
    }

    #[test]
    fn split_empty_path() {
        let segments = split_ref_path("");
        assert!(segments.is_empty());
    }

    #[test]
    fn split_single_segment() {
        let segments = split_ref_path("Workspace");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0], "Workspace");
    }

    // -----------------------------------------------------------------------
    // ref_target_path with "/" in instance name
    // -----------------------------------------------------------------------

    #[test]
    fn ref_target_path_name_with_slash() {
        let mut dom = WeakDom::new(InstanceBuilder::new("DataModel"));
        let ws = dom.insert(dom.root_ref(), InstanceBuilder::new("Workspace"));
        let child = dom.insert(ws, InstanceBuilder::new("A/B"));
        let path = ref_target_path(&dom, child);
        assert_eq!(path, "Workspace/A\\/B");

        // Verify round-trip: split_ref_path should produce the correct segments
        let segments = split_ref_path(&path);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0], "Workspace");
        assert_eq!(segments[1], "A/B");
    }

    // -----------------------------------------------------------------------
    // ref_target_attribute_name tests
    // -----------------------------------------------------------------------

    #[test]
    fn ref_target_attr_name_value() {
        assert_eq!(ref_target_attribute_name("Value"), "Rojo_Target_Value");
    }

    // -----------------------------------------------------------------------
    // RefPathIndex tests
    // -----------------------------------------------------------------------

    #[test]
    fn ref_path_index_add_and_find() {
        let mut index = RefPathIndex::new();
        index.add(
            "Workspace/Model/Part1",
            Path::new("/project/init.meta.json5"),
        );
        let results = index.find_by_prefix("Workspace/Model/Part1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], PathBuf::from("/project/init.meta.json5"));
    }

    #[test]
    fn ref_path_index_find_by_prefix_children() {
        let mut index = RefPathIndex::new();
        index.add("Workspace/Model/Part1", Path::new("/a.meta.json5"));
        index.add("Workspace/Model/Part2", Path::new("/b.meta.json5"));
        index.add("Workspace/Other", Path::new("/c.meta.json5"));

        // "Workspace/Model" should match both Part1 and Part2
        let results = index.find_by_prefix("Workspace/Model");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn ref_path_index_find_exact_match() {
        let mut index = RefPathIndex::new();
        index.add("Workspace/Model", Path::new("/a.meta.json5"));
        let results = index.find_by_prefix("Workspace/Model");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn ref_path_index_no_partial_match() {
        let mut index = RefPathIndex::new();
        index.add("Workspace/ModelExtra", Path::new("/a.meta.json5"));
        // "Workspace/Model" should NOT match "Workspace/ModelExtra"
        let results = index.find_by_prefix("Workspace/Model");
        assert!(results.is_empty());
    }

    #[test]
    fn ref_path_index_remove() {
        let mut index = RefPathIndex::new();
        let path = Path::new("/a.meta.json5");
        index.add("Workspace/Part", path);
        assert_eq!(index.find_by_prefix("Workspace/Part").len(), 1);

        index.remove("Workspace/Part", path);
        assert!(index.find_by_prefix("Workspace/Part").is_empty());
    }

    #[test]
    fn ref_path_index_remove_nonexistent_is_noop() {
        let mut index = RefPathIndex::new();
        // Should not panic
        index.remove("Workspace/Missing", Path::new("/x.meta.json5"));
    }

    #[test]
    fn ref_path_index_remove_all_for_file() {
        let mut index = RefPathIndex::new();
        let file = Path::new("/a.meta.json5");
        index.add("Workspace/Part1", file);
        index.add("Workspace/Part2", file);

        index.remove_all_for_file(file);
        assert!(index.find_by_prefix("Workspace/Part1").is_empty());
        assert!(index.find_by_prefix("Workspace/Part2").is_empty());
    }

    #[test]
    fn ref_path_index_remove_all_for_file_preserves_other_files() {
        let mut index = RefPathIndex::new();
        let file_a = Path::new("/a.meta.json5");
        let file_b = Path::new("/b.meta.json5");
        // Both files share the same ref path entry
        index.add("Workspace/Part1", file_a);
        index.add("Workspace/Part1", file_b);
        // file_a also has another ref
        index.add("Workspace/Part2", file_a);

        index.remove_all_for_file(file_a);

        // file_b's entry should be preserved
        let results = index.find_by_prefix("Workspace/Part1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], PathBuf::from("/b.meta.json5"));

        // file_a's other entry should be gone
        assert!(index.find_by_prefix("Workspace/Part2").is_empty());
    }

    #[test]
    fn ref_path_index_update_prefix() {
        let mut index = RefPathIndex::new();
        index.add("Workspace/OldModel/Part1", Path::new("/a.meta.json5"));
        index.add("Workspace/OldModel/Part2", Path::new("/b.meta.json5"));
        index.add("Workspace/Other", Path::new("/c.meta.json5"));

        index.update_prefix("Workspace/OldModel", "Workspace/NewModel");

        assert!(index.find_by_prefix("Workspace/OldModel").is_empty());
        assert_eq!(index.find_by_prefix("Workspace/NewModel").len(), 2);
        // "Workspace/Other" should be unchanged
        assert_eq!(index.find_by_prefix("Workspace/Other").len(), 1);
    }

    #[test]
    fn ref_path_index_update_prefix_exact() {
        let mut index = RefPathIndex::new();
        index.add("Workspace/Part", Path::new("/a.meta.json5"));

        index.update_prefix("Workspace/Part", "Workspace/RenamedPart");

        assert!(index.find_by_prefix("Workspace/Part").is_empty());
        assert_eq!(index.find_by_prefix("Workspace/RenamedPart").len(), 1);
    }

    #[test]
    fn ref_path_index_deduplicate_results() {
        let mut index = RefPathIndex::new();
        let file = Path::new("/shared.meta.json5");
        // Same file has two Rojo_Ref_* attrs both under the same prefix
        index.add("Workspace/Model/Part1", file);
        index.add("Workspace/Model/Part2", file);

        let results = index.find_by_prefix("Workspace/Model");
        // Should be deduplicated to just one entry
        assert_eq!(results.len(), 1);
    }
}
