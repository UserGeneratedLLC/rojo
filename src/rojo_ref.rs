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
/// Prefix for path-based reference attributes using Luau require-by-string
/// style paths (e.g., `@self/Handle.model.json5`, `./Sibling.luau`,
/// `../Uncle/Part.model.json5`, `@game/ReplicatedStorage/Module.luau`).
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

/// Escape "/" in an instance name for use in legacy instance-name paths.
/// Only used by `ref_target_path` (for debug/log output via `inst_path`).
/// `Rojo_Ref_*` attributes use filesystem names which can't contain `/`.
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

/// Compute a human-readable path to an instance using raw instance names.
/// Used for debug/log output via `inst_path()`. NOT used for `Rojo_Ref_*`
/// attributes (those use `compute_relative_ref_path` with filesystem names).
///
/// Instance names containing "/" are escaped as "\/" to prevent ambiguity.
/// Example: `DataModel > Workspace > Part1` returns `"Workspace/Part1"`.
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

/// Compute the **absolute** filesystem-name-based path to an instance.
///
/// Returns a slash-separated path from DataModel's children downward.
/// Used as input to `compute_relative_ref_path()` and for RefPathIndex
/// indexing. Not written to disk directly -- `compute_relative_ref_path`
/// converts absolute paths to the Luau-style relative format.
///
/// Each path segment is the **full filesystem name** of the instance:
/// - `Folder "Foo"` → `"Foo"` (directory name)
/// - `ModuleScript "Foo"` → `"Foo.luau"` (slug + extension)
/// - `Script "Foo"` (server) → `"Foo.server.luau"`
/// - Dedup'd `ModuleScript "Foo"` → `"Foo~2.luau"`
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

        components.push(tree.filesystem_name_for(current));
        current = inst.parent();
    }

    components.reverse();
    components.join("/")
}

/// Compute a Luau require-by-string style path from a source instance to a
/// target instance. Both inputs are **absolute** ref paths (slash-separated
/// filesystem names from DataModel's children downward).
///
/// Returns a prefixed path following Roblox's require-by-string semantics:
/// - `@self` when target == source
/// - `@self/...` when target is a descendant of source
/// - `./...` when source and target share the same parent (1 level up)
/// - `../...` chains for navigation within the same service (2+ levels up)
/// - `@game/...` for cross-service refs or when target is an ancestor of source
///
/// Reference: <https://create.roblox.com/docs/reference/engine/globals/LuaGlobals#require>
pub fn compute_relative_ref_path(source_abs: &str, target_abs: &str) -> String {
    if source_abs == target_abs {
        return "@self".to_string();
    }

    let source_prefix = format!("{source_abs}/");
    if target_abs.starts_with(&source_prefix) {
        return format!("@self/{}", &target_abs[source_prefix.len()..]);
    }

    let source_parts: Vec<&str> = source_abs.split('/').collect();
    let target_parts: Vec<&str> = target_abs.split('/').collect();

    let common_len = source_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    if common_len == 0 {
        return format!("@game/{target_abs}");
    }

    let ups = source_parts.len() - common_len;
    let remaining_parts = &target_parts[common_len..];

    if remaining_parts.is_empty() {
        return format!("@game/{target_abs}");
    }

    let remaining = remaining_parts.join("/");

    if ups == 1 {
        format!("./{remaining}")
    } else {
        let mut result = String::with_capacity(3 * (ups - 1) + remaining.len());
        for _ in 0..(ups - 1) {
            result.push_str("../");
        }
        result.push_str(&remaining);
        result
    }
}

/// Resolve a relative (Luau-style) ref path back to an absolute ref path
/// using only string manipulation. `source_abs` is the absolute ref path of
/// the instance that owns the attribute.
///
/// Returns `None` if the path navigates above the root.
pub fn resolve_ref_path_to_absolute(path: &str, source_abs: &str) -> Option<String> {
    if let Some(rest) = path.strip_prefix("@game/") {
        return Some(rest.to_string());
    }
    if path == "@game" {
        return Some(String::new());
    }
    if path == "@self" {
        return Some(source_abs.to_string());
    }

    let (mut parts, rest) = if let Some(rest) = path.strip_prefix("@self/") {
        (source_abs.split('/').collect::<Vec<_>>(), rest)
    } else if let Some(rest) = path.strip_prefix("./") {
        let mut p: Vec<&str> = source_abs.split('/').collect();
        p.pop();
        (p, rest)
    } else if let Some(rest) = path.strip_prefix("../") {
        let mut p: Vec<&str> = source_abs.split('/').collect();
        p.pop();
        p.pop()?;
        (p, rest)
    } else {
        return Some(path.to_string());
    };

    for segment in rest.split('/') {
        if segment == ".." {
            parts.pop()?;
        } else if !segment.is_empty() {
            parts.push(segment);
        }
    }

    Some(parts.join("/"))
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

    /// Scan all `.meta.json5`, `.model.json5`, `.meta.json`, `.model.json`
    /// files under `root` for existing `Rojo_Ref_*` attributes and index them.
    ///
    /// Relative paths (prefixed with `@self/`, `./`, `../`) are resolved to
    /// absolute using `tree` so the index always stores absolute target paths.
    /// This ensures prefix-based lookup works correctly for rename updates.
    pub fn populate_from_dir(&mut self, root: &Path, tree: &crate::snapshot::RojoTree) {
        use rayon::prelude::*;
        use walkdir::WalkDir;

        fn is_meta_or_model(name: &str) -> bool {
            name.ends_with(".meta.json5")
                || name.ends_with(".model.json5")
                || name.ends_with(".meta.json")
                || name.ends_with(".model.json")
        }

        let meta_paths: Vec<std::path::PathBuf> = WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_map(|e: Result<walkdir::DirEntry, _>| e.ok())
            .filter(|e: &walkdir::DirEntry| {
                e.file_type().is_file() && e.file_name().to_str().is_some_and(is_meta_or_model)
            })
            .map(|e: walkdir::DirEntry| e.into_path())
            .collect();

        let entries: Vec<(String, std::path::PathBuf)> = meta_paths
            .par_iter()
            .flat_map(|path| {
                let source_abs = tree
                    .get_ids_at_path(path)
                    .first()
                    .map(|&id| crate::ref_target_path_from_tree(tree, id))
                    .unwrap_or_default();

                let mut results = Vec::new();
                if let Ok(bytes) = std::fs::read(path) {
                    if let Ok(val) = crate::json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(attrs) = val.get("attributes").and_then(|a| a.as_object()) {
                            for (key, value) in attrs {
                                if key.starts_with(crate::REF_PATH_ATTRIBUTE_PREFIX) {
                                    if let Some(path_str) = value.as_str() {
                                        let resolved = crate::resolve_ref_path_to_absolute(
                                            path_str,
                                            &source_abs,
                                        )
                                        .unwrap_or_else(|| path_str.to_string());
                                        results.push((resolved, path.clone()));
                                    }
                                }
                            }
                        }
                    }
                }
                results
            })
            .collect();

        let count = entries.len();
        for (resolved, path) in entries {
            self.add(&resolved, &path);
        }

        if count > 0 {
            log::info!(
                "RefPathIndex: populated {} Rojo_Ref_* entries from existing meta/model files",
                count
            );
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

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- @self
    // -----------------------------------------------------------------------

    #[test]
    fn relative_self_reference() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model"),
            "@self"
        );
    }

    #[test]
    fn relative_self_direct_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/Handle.model.json5"),
            "@self/Handle.model.json5"
        );
    }

    #[test]
    fn relative_self_nested_descendant() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Model",
                "Workspace/Model/SubFolder/Part.model.json5"
            ),
            "@self/SubFolder/Part.model.json5"
        );
    }

    #[test]
    fn relative_self_deeply_nested() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/A/B/C/D.luau"),
            "@self/A/B/C/D.luau"
        );
    }

    #[test]
    fn relative_self_tool_handle() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Tool", "Workspace/Tool/Handle.model.json5"),
            "@self/Handle.model.json5"
        );
    }

    #[test]
    fn relative_self_init_style_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/Init"),
            "@self/Init"
        );
    }

    #[test]
    fn relative_self_nested_sound() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Gun", "Workspace/Gun/Sounds/Fire.model.json5"),
            "@self/Sounds/Fire.model.json5"
        );
    }

    #[test]
    fn relative_self_slugified_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/Hey_Bro.server.luau"),
            "@self/Hey_Bro.server.luau"
        );
    }

    #[test]
    fn relative_self_dedup_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/Data~2"),
            "@self/Data~2"
        );
    }

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- ./
    // -----------------------------------------------------------------------

    #[test]
    fn relative_dot_sibling_script() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/ScriptA.server.luau",
                "Workspace/Folder/ScriptB.server.luau"
            ),
            "./ScriptB.server.luau"
        );
    }

    #[test]
    fn relative_dot_sibling_module() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/ScriptA.server.luau",
                "Workspace/Folder/Config.luau"
            ),
            "./Config.luau"
        );
    }

    #[test]
    fn relative_dot_sibling_directory() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/ScriptA.server.luau",
                "Workspace/Folder/SubFolder"
            ),
            "./SubFolder"
        );
    }

    #[test]
    fn relative_dot_sibling_then_descend() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/ScriptA.server.luau",
                "Workspace/Folder/SubFolder/Deep.luau"
            ),
            "./SubFolder/Deep.luau"
        );
    }

    #[test]
    fn relative_dot_object_value_sibling() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/ObjectValue.model.json5",
                "Workspace/Folder/Target.model.json5"
            ),
            "./Target.model.json5"
        );
    }

    #[test]
    fn relative_dot_beam_attachment0() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Beams/Beam.model.json5",
                "Workspace/Beams/Att1.model.json5"
            ),
            "./Att1.model.json5"
        );
    }

    #[test]
    fn relative_dot_beam_attachment1() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Beams/Beam.model.json5",
                "Workspace/Beams/Att2.model.json5"
            ),
            "./Att2.model.json5"
        );
    }

    #[test]
    fn relative_dot_slugified_sibling() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/A.luau",
                "Workspace/Folder/Hey_Bro.server.luau"
            ),
            "./Hey_Bro.server.luau"
        );
    }

    #[test]
    fn relative_dot_dedup_sibling_folder() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Folder/A.luau", "Workspace/Folder/Data~2"),
            "./Data~2"
        );
    }

    #[test]
    fn relative_dot_dedup_sibling_model() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Folder/A.luau",
                "Workspace/Folder/Data~3.model.json5"
            ),
            "./Data~3.model.json5"
        );
    }

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- ../
    // -----------------------------------------------------------------------

    #[test]
    fn relative_dotdot_2_ups() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/Script.server.luau",
                "Workspace/B/Part.model.json5"
            ),
            "../B/Part.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_2_ups_directory() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/Script.server.luau", "Workspace/B"),
            "../B"
        );
    }

    #[test]
    fn relative_dotdot_3_ups() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/Script.server.luau",
                "Workspace/C/Part.model.json5"
            ),
            "../../C/Part.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_4_ups() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/C/Script.luau",
                "Workspace/D/E/Part.model.json5"
            ),
            "../../../D/E/Part.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_5_ups() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/C/D/Script.luau",
                "Workspace/E/Part.model.json5"
            ),
            "../../../../E/Part.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_peer_model() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Models/Car/Body.model.json5",
                "Workspace/Models/Truck/Body.model.json5"
            ),
            "../Truck/Body.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_cousin() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/Deep/Script.luau", "Workspace/A/Other.luau"),
            "../Other.luau"
        );
    }

    #[test]
    fn relative_dotdot_beam_uncle_folder() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/Beam.model.json5",
                "Workspace/A/C/Att.model.json5"
            ),
            "../C/Att.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_cross_system() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Systems/Combat/Hitbox.server.luau",
                "Workspace/Systems/Audio/HitSound.model.json5"
            ),
            "../Audio/HitSound.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_same_zone() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Map/Zone1/Spawns/Spawn1.model.json5",
                "Workspace/Map/Zone1/Props/Tree.model.json5"
            ),
            "../Props/Tree.model.json5"
        );
    }

    #[test]
    fn relative_dotdot_cross_zone() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Map/Zone1/Spawns/Spawn1.model.json5",
                "Workspace/Map/Zone2/Flag.model.json5"
            ),
            "../../Zone2/Flag.model.json5"
        );
    }

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- @game/
    // -----------------------------------------------------------------------

    #[test]
    fn relative_game_cross_service_classic() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/Script.server.luau",
                "ReplicatedStorage/Module.luau"
            ),
            "@game/ReplicatedStorage/Module.luau"
        );
    }

    #[test]
    fn relative_game_to_server_storage() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Script.server.luau", "ServerStorage/Data.luau"),
            "@game/ServerStorage/Data.luau"
        );
    }

    #[test]
    fn relative_game_sss_to_rs() {
        assert_eq!(
            compute_relative_ref_path(
                "ServerScriptService/Main.server.luau",
                "ReplicatedStorage/Shared/Utils.luau"
            ),
            "@game/ReplicatedStorage/Shared/Utils.luau"
        );
    }

    #[test]
    fn relative_game_client_to_rs() {
        assert_eq!(
            compute_relative_ref_path(
                "StarterGui/ScreenGui/Button.client.luau",
                "ReplicatedStorage/UI/Theme.luau"
            ),
            "@game/ReplicatedStorage/UI/Theme.luau"
        );
    }

    #[test]
    fn relative_game_to_service_itself() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/B/C/Script.luau", "Lighting"),
            "@game/Lighting"
        );
    }

    #[test]
    fn relative_game_ancestor_own_service() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/B/C/Script.luau", "Workspace"),
            "@game/Workspace"
        );
    }

    #[test]
    fn relative_game_ancestor_grandparent() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/B/Script.luau", "Workspace/A"),
            "@game/Workspace/A"
        );
    }

    #[test]
    fn relative_game_ancestor_great_grandparent() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A/B/C/Script.luau", "Workspace/A/B"),
            "@game/Workspace/A/B"
        );
    }

    #[test]
    fn relative_game_ancestor_own_service_single_level() {
        assert_eq!(
            compute_relative_ref_path("ReplicatedStorage/A.luau", "ReplicatedStorage"),
            "@game/ReplicatedStorage"
        );
    }

    #[test]
    fn relative_game_to_sound_service() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Script.luau", "SoundService/BGM.model.json5"),
            "@game/SoundService/BGM.model.json5"
        );
    }

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- special names
    // -----------------------------------------------------------------------

    #[test]
    fn relative_special_windows_reserved() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Folder/A.luau", "Workspace/Other/CON_.luau"),
            "../Other/CON_.luau"
        );
    }

    #[test]
    fn relative_special_into_dedup_folder() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A.luau", "Workspace/Data~2/Child.luau"),
            "./Data~2/Child.luau"
        );
    }

    #[test]
    fn relative_special_init_style_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Model", "Workspace/Model/Scripts"),
            "@self/Scripts"
        );
    }

    #[test]
    fn relative_special_init_style_both() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Scripts", "Workspace/Scripts/Foo"),
            "@self/Foo"
        );
    }

    // -----------------------------------------------------------------------
    // compute_relative_ref_path tests -- tricky edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn relative_edge_service_own_child() {
        assert_eq!(
            compute_relative_ref_path("Workspace", "Workspace/Model.model.json5"),
            "@self/Model.model.json5"
        );
    }

    #[test]
    fn relative_edge_service_to_service() {
        assert_eq!(
            compute_relative_ref_path("Workspace", "ReplicatedStorage/Module.luau"),
            "@game/ReplicatedStorage/Module.luau"
        );
    }

    #[test]
    fn relative_edge_service_self_ref() {
        assert_eq!(compute_relative_ref_path("Workspace", "Workspace"), "@self");
    }

    #[test]
    fn relative_edge_service_to_different_service() {
        assert_eq!(
            compute_relative_ref_path("Workspace", "Lighting"),
            "@game/Lighting"
        );
    }

    #[test]
    fn relative_edge_segment_prefix_ambiguity() {
        assert_eq!(
            compute_relative_ref_path("Workspace/A", "Workspace/AB"),
            "./AB"
        );
    }

    #[test]
    fn relative_edge_same_stem_different_ext() {
        assert_eq!(
            compute_relative_ref_path("Workspace/Foo.server.luau", "Workspace/Foo.luau"),
            "./Foo.luau"
        );
    }

    #[test]
    fn relative_edge_deep_close_cousins() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/C/D/E/F/Script.luau",
                "Workspace/A/B/C/D/E/G/Target.luau"
            ),
            "../G/Target.luau"
        );
    }

    #[test]
    fn relative_edge_deep_far_apart() {
        assert_eq!(
            compute_relative_ref_path(
                "Workspace/A/B/C/D/E/F/Script.luau",
                "Workspace/X/Target.luau"
            ),
            "../../../../../../X/Target.luau"
        );
    }

    // -----------------------------------------------------------------------
    // resolve_ref_path_to_absolute tests
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_abs_game_prefix() {
        assert_eq!(
            resolve_ref_path_to_absolute("@game/ReplicatedStorage/Module.luau", "Workspace/Script"),
            Some("ReplicatedStorage/Module.luau".to_string())
        );
    }

    #[test]
    fn resolve_abs_self_no_path() {
        assert_eq!(
            resolve_ref_path_to_absolute("@self", "Workspace/Model"),
            Some("Workspace/Model".to_string())
        );
    }

    #[test]
    fn resolve_abs_self_with_child() {
        assert_eq!(
            resolve_ref_path_to_absolute("@self/Handle.model.json5", "Workspace/Model"),
            Some("Workspace/Model/Handle.model.json5".to_string())
        );
    }

    #[test]
    fn resolve_abs_self_nested() {
        assert_eq!(
            resolve_ref_path_to_absolute("@self/A/B/C", "Workspace/Model"),
            Some("Workspace/Model/A/B/C".to_string())
        );
    }

    #[test]
    fn resolve_abs_dot_sibling() {
        assert_eq!(
            resolve_ref_path_to_absolute("./Sibling.luau", "Workspace/Folder/Script.luau"),
            Some("Workspace/Folder/Sibling.luau".to_string())
        );
    }

    #[test]
    fn resolve_abs_dot_sibling_descend() {
        assert_eq!(
            resolve_ref_path_to_absolute("./Sub/Deep.luau", "Workspace/Folder/Script.luau"),
            Some("Workspace/Folder/Sub/Deep.luau".to_string())
        );
    }

    #[test]
    fn resolve_abs_dotdot_2_ups() {
        assert_eq!(
            resolve_ref_path_to_absolute("../B/Part.model.json5", "Workspace/A/Script.luau"),
            Some("Workspace/B/Part.model.json5".to_string())
        );
    }

    #[test]
    fn resolve_abs_dotdot_3_ups() {
        assert_eq!(
            resolve_ref_path_to_absolute("../../C/Part.model.json5", "Workspace/A/B/Script.luau"),
            Some("Workspace/C/Part.model.json5".to_string())
        );
    }

    #[test]
    fn resolve_abs_dotdot_cross_service_via_root() {
        assert_eq!(
            resolve_ref_path_to_absolute(
                "../../ReplicatedStorage/Module.luau",
                "Workspace/A/Script.luau"
            ),
            Some("ReplicatedStorage/Module.luau".to_string())
        );
    }

    #[test]
    fn resolve_abs_bare_path_legacy() {
        assert_eq!(
            resolve_ref_path_to_absolute("Workspace/Model/Handle.model.json5", "anything/here"),
            Some("Workspace/Model/Handle.model.json5".to_string())
        );
    }

    #[test]
    fn resolve_abs_dotdot_mid_path() {
        assert_eq!(
            resolve_ref_path_to_absolute("@self/Sub/../Handle.model.json5", "Workspace/Model"),
            Some("Workspace/Model/Handle.model.json5".to_string())
        );
    }

    #[test]
    fn resolve_abs_dotdot_above_root_returns_none() {
        assert_eq!(resolve_ref_path_to_absolute("../../X", "Workspace"), None);
    }

    #[test]
    fn resolve_then_compute_round_trip() {
        let source = "Workspace/A/B/Script.luau";
        let target = "Workspace/C/Part.model.json5";
        let relative = compute_relative_ref_path(source, target);
        assert_eq!(relative, "../../C/Part.model.json5");
        let resolved = resolve_ref_path_to_absolute(&relative, source).unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn resolve_then_compute_round_trip_self() {
        let source = "Workspace/Model";
        let target = "Workspace/Model/Handle.model.json5";
        let relative = compute_relative_ref_path(source, target);
        assert_eq!(relative, "@self/Handle.model.json5");
        let resolved = resolve_ref_path_to_absolute(&relative, source).unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn resolve_then_compute_round_trip_game() {
        let source = "Workspace/Script.luau";
        let target = "ReplicatedStorage/Module.luau";
        let relative = compute_relative_ref_path(source, target);
        assert_eq!(relative, "@game/ReplicatedStorage/Module.luau");
        let resolved = resolve_ref_path_to_absolute(&relative, source).unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn resolve_then_compute_round_trip_dot() {
        let source = "Workspace/Folder/A.luau";
        let target = "Workspace/Folder/B.luau";
        let relative = compute_relative_ref_path(source, target);
        assert_eq!(relative, "./B.luau");
        let resolved = resolve_ref_path_to_absolute(&relative, source).unwrap();
        assert_eq!(resolved, target);
    }
}
