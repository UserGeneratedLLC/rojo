use std::{fmt, sync::Arc};

use rbx_dom_weak::{types::Ref, WeakDom};
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

/// Compute the path to a target instance for use in a `Rojo_Ref_*` attribute.
/// Returns the slash-separated path from the root (root name excluded).
///
/// Example: an instance at `DataModel > Workspace > Part1` returns `"Workspace/Part1"`.
pub fn ref_target_path(dom: &WeakDom, target_ref: Ref) -> String {
    dom.full_path_of(target_ref, "/")
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
}
