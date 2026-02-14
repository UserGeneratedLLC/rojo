use std::{borrow::Cow, fmt, sync::Arc};

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
}
