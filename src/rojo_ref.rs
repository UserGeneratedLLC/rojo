use std::{fmt, sync::Arc};

use serde::{Deserialize, Serialize};

/// Legacy: ID attribute on target instances (kept for backwards compatibility)
pub const REF_ID_ATTRIBUTE_NAME: &str = "Rojo_Id";
/// Legacy: Pointer attribute prefix using IDs (kept for backwards compatibility)
pub const REF_POINTER_ATTRIBUTE_PREFIX: &str = "Rojo_Target_";
/// Prefix for path-based reference attributes. This is the preferred system
/// that stores the path to the target instance (e.g., "SoundService/Effects").
/// No modification to the target instance is needed.
pub const REF_PATH_ATTRIBUTE_PREFIX: &str = "Rojo_Ref_";

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
