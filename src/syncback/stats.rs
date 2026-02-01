//! Statistics tracking for syncback operations.
//!
//! This module provides a way to track and report various issues that occur
//! during syncback, including:
//! - Instances with duplicate names (indistinguishable paths)
//! - Instances that fell back to rbxm/rbxmx format
//! - Unknown classes not in the reflection database
//! - Unknown properties not in the reflection database

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Statistics collected during a syncback operation.
///
/// This struct is designed to be used in a single-threaded context during
/// the main syncback loop, but uses interior mutability for flexibility.
#[derive(Default)]
pub struct SyncbackStats {
    /// Count of instances skipped due to duplicate/indistinguishable names.
    duplicate_name_count: AtomicUsize,
    /// Count of instances that fell back to rbxm/rbxmx format.
    rbxm_fallback_count: AtomicUsize,
    /// Count of instances with unknown classes.
    unknown_class_count: AtomicUsize,
    /// Count of properties with unknown definitions.
    unknown_property_count: AtomicUsize,

    /// Set of unknown class names encountered (for reporting).
    unknown_classes: Mutex<HashSet<String>>,
    /// Set of unknown property names encountered (class.property format).
    unknown_properties: Mutex<HashSet<String>>,
}

impl SyncbackStats {
    /// Creates a new empty stats tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that an instance was skipped due to having a duplicate name.
    ///
    /// When debug logging is enabled, logs the individual instance path.
    pub fn record_duplicate_name(&self, inst_path: &str, name: &str) {
        self.duplicate_name_count.fetch_add(1, Ordering::Relaxed);

        // Only log individual instances at debug level to avoid spam
        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "Skipping instance due to duplicate name: '{}' at '{}'",
                name,
                inst_path
            );
        }
    }

    /// Records that multiple instances were skipped due to duplicate names at a path.
    ///
    /// This is more efficient when multiple duplicates are detected at once.
    pub fn record_duplicate_names_batch(
        &self,
        inst_path: &str,
        duplicate_names: &[&str],
        total_skipped: usize,
    ) {
        self.duplicate_name_count
            .fetch_add(total_skipped, Ordering::Relaxed);

        // Only log at debug level
        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "Skipping {} instance(s) with duplicate names at '{}': {:?}",
                total_skipped,
                inst_path,
                duplicate_names
            );
        }
    }

    /// Records that an instance fell back to rbxm/rbxmx format.
    pub fn record_rbxm_fallback(&self, inst_path: &str, reason: &str) {
        self.rbxm_fallback_count.fetch_add(1, Ordering::Relaxed);

        // Only log individual fallbacks at debug level
        if log::log_enabled!(log::Level::Debug) {
            log::debug!(
                "Instance '{}' fell back to binary model format: {}",
                inst_path,
                reason
            );
        }
    }

    /// Records that an unknown class was encountered.
    pub fn record_unknown_class(&self, class_name: &str) {
        self.unknown_class_count.fetch_add(1, Ordering::Relaxed);

        if let Ok(mut classes) = self.unknown_classes.lock() {
            if classes.insert(class_name.to_string()) {
                // Only log when we see a new unknown class
                if log::log_enabled!(log::Level::Debug) {
                    log::debug!(
                        "Encountered unknown class not in reflection database: '{}'",
                        class_name
                    );
                }
            }
        }
    }

    /// Records that an unknown property was encountered.
    pub fn record_unknown_property(&self, class_name: &str, property_name: &str) {
        self.unknown_property_count.fetch_add(1, Ordering::Relaxed);

        let key = format!("{}.{}", class_name, property_name);
        if let Ok(mut properties) = self.unknown_properties.lock() {
            if properties.insert(key.clone()) {
                // Only log when we see a new unknown property
                if log::log_enabled!(log::Level::Debug) {
                    log::debug!(
                        "Encountered unknown property not in reflection database: '{}'",
                        key
                    );
                }
            }
        }
    }

    /// Returns the count of instances skipped due to duplicate names.
    pub fn duplicate_name_count(&self) -> usize {
        self.duplicate_name_count.load(Ordering::Relaxed)
    }

    /// Returns the count of instances that fell back to rbxm format.
    pub fn rbxm_fallback_count(&self) -> usize {
        self.rbxm_fallback_count.load(Ordering::Relaxed)
    }

    /// Returns the count of unknown classes encountered.
    pub fn unknown_class_count(&self) -> usize {
        self.unknown_class_count.load(Ordering::Relaxed)
    }

    /// Returns the count of unknown properties encountered.
    pub fn unknown_property_count(&self) -> usize {
        self.unknown_property_count.load(Ordering::Relaxed)
    }

    /// Returns the unique unknown class names.
    pub fn unknown_classes(&self) -> Vec<String> {
        self.unknown_classes
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the unique unknown properties (as "Class.Property").
    pub fn unknown_properties(&self) -> Vec<String> {
        self.unknown_properties
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns true if any issues were recorded.
    pub fn has_issues(&self) -> bool {
        self.duplicate_name_count() > 0
            || self.rbxm_fallback_count() > 0
            || self.unknown_class_count() > 0
            || self.unknown_property_count() > 0
    }

    /// Returns true if there are unknown classes or properties that should be
    /// reported for reflection database updates.
    pub fn has_unknown_types(&self) -> bool {
        self.unknown_class_count() > 0 || self.unknown_property_count() > 0
    }

    /// Logs a summary of all recorded issues as warnings.
    ///
    /// This should be called at the end of a syncback operation.
    pub fn log_summary(&self) {
        let duplicate_count = self.duplicate_name_count();
        let rbxm_count = self.rbxm_fallback_count();
        let unknown_class_count = self.unknown_class_count();
        let unknown_prop_count = self.unknown_property_count();

        if !self.has_issues() {
            return;
        }

        log::warn!("Syncback completed with issues:");

        if duplicate_count > 0 {
            log::warn!(
                "  - {} instance(s) could not be synced due to duplicate/indistinguishable names",
                duplicate_count
            );
        }

        if rbxm_count > 0 {
            log::warn!(
                "  - {} instance(s) fell back to binary model format (rbxm/rbxmx)",
                rbxm_count
            );
        }

        if unknown_class_count > 0 {
            let classes = self.unknown_classes();
            log::warn!(
                "  - {} instance(s) have unknown classes not in reflection database",
                unknown_class_count
            );
            if log::log_enabled!(log::Level::Info) && !classes.is_empty() {
                log::info!("    Unknown classes: {:?}", classes);
            }
        }

        if unknown_prop_count > 0 {
            let properties = self.unknown_properties();
            log::warn!(
                "  - {} property reference(s) to unknown properties not in reflection database",
                unknown_prop_count
            );
            if log::log_enabled!(log::Level::Info) && !properties.is_empty() {
                // Only show first 20 to avoid spam
                let display: Vec<_> = properties.iter().take(20).collect();
                let remaining = properties.len().saturating_sub(20);
                if remaining > 0 {
                    log::info!(
                        "    Unknown properties (showing 20 of {}): {:?}",
                        properties.len(),
                        display
                    );
                } else {
                    log::info!("    Unknown properties: {:?}", display);
                }
            }
        }

        // Helpful hint about debug logging
        if duplicate_count > 0 || rbxm_count > 0 {
            log::warn!(
                "    Enable debug logging (RUST_LOG=debug) to see individual instance details"
            );
        }
    }

    /// Merges stats from another SyncbackStats instance into this one.
    pub fn merge(&self, other: &SyncbackStats) {
        self.duplicate_name_count.fetch_add(
            other.duplicate_name_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.rbxm_fallback_count.fetch_add(
            other.rbxm_fallback_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.unknown_class_count.fetch_add(
            other.unknown_class_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.unknown_property_count.fetch_add(
            other.unknown_property_count.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        if let (Ok(mut self_classes), Ok(other_classes)) =
            (self.unknown_classes.lock(), other.unknown_classes.lock())
        {
            self_classes.extend(other_classes.iter().cloned());
        }

        if let (Ok(mut self_props), Ok(other_props)) = (
            self.unknown_properties.lock(),
            other.unknown_properties.lock(),
        ) {
            self_props.extend(other_props.iter().cloned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_tracking() {
        let stats = SyncbackStats::new();

        assert!(!stats.has_issues());

        stats.record_duplicate_name("Root/Folder", "Script");
        stats.record_duplicate_name("Root/Folder", "Script");
        assert_eq!(stats.duplicate_name_count(), 2);

        stats.record_rbxm_fallback("Root/Model", "cannot represent as directory");
        assert_eq!(stats.rbxm_fallback_count(), 1);

        stats.record_unknown_class("MyCustomClass");
        stats.record_unknown_class("MyCustomClass"); // duplicate
        assert_eq!(stats.unknown_class_count(), 2);
        assert_eq!(stats.unknown_classes().len(), 1); // unique

        stats.record_unknown_property("Part", "CustomProp");
        assert_eq!(stats.unknown_property_count(), 1);

        assert!(stats.has_issues());
        assert!(stats.has_unknown_types());
    }

    #[test]
    fn test_batch_recording() {
        let stats = SyncbackStats::new();

        stats.record_duplicate_names_batch("Root/Folder", &["Script", "Model"], 4);
        assert_eq!(stats.duplicate_name_count(), 4);
    }

    #[test]
    fn test_merge() {
        let stats1 = SyncbackStats::new();
        let stats2 = SyncbackStats::new();

        stats1.record_duplicate_name("path1", "name1");
        stats2.record_duplicate_name("path2", "name2");
        stats2.record_unknown_class("Class1");

        stats1.merge(&stats2);

        assert_eq!(stats1.duplicate_name_count(), 2);
        assert_eq!(stats1.unknown_classes().len(), 1);
    }
}
