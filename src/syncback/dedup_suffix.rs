//! Dedup suffix lifecycle management.
//!
//! When instances have duplicate names, they receive `~N` suffixes on the
//! filesystem (e.g., `Foo`, `Foo~2`, `Foo~3`). This module provides shared
//! helpers for managing suffix cleanup when instances are deleted.
//!
//! Rules:
//! - **Gap-tolerant:** deleting a suffixed instance does NOT renumber remaining
//!   siblings. Gaps (`Foo`, `Foo~3` without `~2`) are harmless.
//! - **Base-name promotion:** when the base-name holder is deleted and multiple
//!   siblings remain, the lowest-numbered suffix is promoted to base name.
//! - **Group-to-1 cleanup:** when a deletion reduces the dedup group to exactly
//!   1 remaining instance, its suffix is removed entirely.

use std::path::{Path, PathBuf};

/// Result of analyzing the dedup group after a deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DedupCleanupAction {
    /// No rename needed. Either the deleted instance had a suffix and others
    /// remain, or the group is still large enough that no cleanup is required.
    None,

    /// The dedup group shrank to exactly 1 remaining instance.
    /// Its suffix must be removed (renamed to the clean base name).
    RemoveSuffix {
        /// The current path (with suffix) that should be renamed.
        from: PathBuf,
        /// The target path (clean, no suffix).
        to: PathBuf,
    },

    /// The base-name holder was deleted and the lowest-numbered suffix must be
    /// promoted to take the base name.
    PromoteLowest {
        /// The current path of the lowest-suffixed sibling.
        from: PathBuf,
        /// The base-name path (clean, no suffix).
        to: PathBuf,
    },
}

/// Parses a dedup suffix from a filesystem stem.
///
/// Given `"Foo~3"`, returns `Some(("Foo", 3))`.
/// Given `"Foo"`, returns `None`.
/// Given `"Foo~abc"` (non-numeric), returns `None`.
pub fn parse_dedup_suffix(stem: &str) -> Option<(&str, u32)> {
    if let Some(tilde_pos) = stem.rfind('~') {
        let base = &stem[..tilde_pos];
        let suffix_str = &stem[tilde_pos + 1..];
        if let Ok(n) = suffix_str.parse::<u32>() {
            if n > 0 {
                return Some((base, n));
            }
        }
    }
    None
}

/// Strips a `~N` dedup suffix from a filename stem, returning the base name.
///
/// Given `"Foo~2"`, returns `"Foo"`.
/// Given `"Foo"`, returns `"Foo"` unchanged.
/// Given `"Foo~0"` or `"Foo~abc"`, returns the input unchanged (not valid
/// dedup suffixes).
pub fn strip_dedup_suffix(name: &str) -> &str {
    parse_dedup_suffix(name).map_or(name, |(base, _)| base)
}

/// Builds a dedup'd filename from a base stem, optional suffix number, and
/// extension.
///
/// - `build_dedup_name("Foo", None, Some("server.luau"))` → `"Foo.server.luau"`
/// - `build_dedup_name("Foo", Some(2), Some("server.luau"))` → `"Foo~2.server.luau"`
/// - `build_dedup_name("Foo", Some(1), None)` → `"Foo~1"` (directory)
/// - `build_dedup_name("Foo", None, None)` → `"Foo"` (directory)
pub fn build_dedup_name(base_stem: &str, suffix: Option<u32>, extension: Option<&str>) -> String {
    let stem = match suffix {
        Some(n) => format!("{base_stem}~{n}"),
        None => base_stem.to_string(),
    };
    match extension {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem,
    }
}

/// Determines the cleanup action needed after removing an instance from a
/// dedup group.
///
/// `remaining_stems` contains the filesystem stems of all siblings that STILL
/// exist after the deletion (not including the deleted one). These should share
/// the same base stem (before `~N`). If the base-name holder was deleted, it
/// should NOT appear in `remaining_stems`.
///
/// `deleted_was_base` indicates whether the deleted instance held the base name
/// (no suffix).
///
/// Returns the appropriate cleanup action.
pub fn compute_cleanup_action(
    base_stem: &str,
    extension: Option<&str>,
    remaining_stems: &[String],
    deleted_was_base: bool,
    parent_dir: &Path,
) -> DedupCleanupAction {
    match remaining_stems.len() {
        0 => {
            // No siblings remain -- nothing to clean up.
            DedupCleanupAction::None
        }
        1 => {
            // Group shrank to 1: remove the survivor's suffix entirely.
            let survivor = &remaining_stems[0];
            let from_name = build_dedup_name(
                base_stem,
                parse_dedup_suffix(survivor).map(|(_, n)| n),
                extension,
            );
            let to_name = build_dedup_name(base_stem, None, extension);
            if from_name == to_name {
                // Survivor already has the clean name (was the base holder
                // and only one other was deleted).
                DedupCleanupAction::None
            } else {
                DedupCleanupAction::RemoveSuffix {
                    from: parent_dir.join(&from_name),
                    to: parent_dir.join(&to_name),
                }
            }
        }
        _ => {
            // Multiple siblings remain.
            if deleted_was_base {
                // The base-name holder was deleted. Promote the lowest suffix.
                let mut suffix_numbers: Vec<u32> = remaining_stems
                    .iter()
                    .filter_map(|s| parse_dedup_suffix(s).map(|(_, n)| n))
                    .collect();
                suffix_numbers.sort();

                if let Some(&lowest) = suffix_numbers.first() {
                    let from_name = build_dedup_name(base_stem, Some(lowest), extension);
                    let to_name = build_dedup_name(base_stem, None, extension);
                    DedupCleanupAction::PromoteLowest {
                        from: parent_dir.join(&from_name),
                        to: parent_dir.join(&to_name),
                    }
                } else {
                    // All remaining are base-name holders? Shouldn't happen.
                    DedupCleanupAction::None
                }
            } else {
                // A suffixed instance was deleted. Gap-tolerant: no rename.
                DedupCleanupAction::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_suffix_basic() {
        assert_eq!(parse_dedup_suffix("Foo~1"), Some(("Foo", 1)));
        assert_eq!(parse_dedup_suffix("Foo~2"), Some(("Foo", 2)));
        assert_eq!(parse_dedup_suffix("Foo~10"), Some(("Foo", 10)));
    }

    #[test]
    fn parse_suffix_none() {
        assert_eq!(parse_dedup_suffix("Foo"), None);
        assert_eq!(parse_dedup_suffix("Foo~0"), None); // 0 is not valid
        assert_eq!(parse_dedup_suffix("Foo~abc"), None);
        assert_eq!(parse_dedup_suffix("Foo~"), None);
    }

    #[test]
    fn parse_suffix_complex_stems() {
        assert_eq!(parse_dedup_suffix("A_B~3"), Some(("A_B", 3)));
        assert_eq!(parse_dedup_suffix("My Script~1"), Some(("My Script", 1)));
    }

    #[test]
    fn strip_suffix_basic() {
        assert_eq!(strip_dedup_suffix("Foo~1"), "Foo");
        assert_eq!(strip_dedup_suffix("Foo~2"), "Foo");
        assert_eq!(strip_dedup_suffix("Foo~10"), "Foo");
    }

    #[test]
    fn strip_suffix_no_op() {
        assert_eq!(strip_dedup_suffix("Foo"), "Foo");
        assert_eq!(strip_dedup_suffix("Foo~0"), "Foo~0");
        assert_eq!(strip_dedup_suffix("Foo~abc"), "Foo~abc");
        assert_eq!(strip_dedup_suffix("Foo~"), "Foo~");
        assert_eq!(strip_dedup_suffix(""), "");
    }

    #[test]
    fn build_name_file() {
        assert_eq!(
            build_dedup_name("Foo", None, Some("server.luau")),
            "Foo.server.luau"
        );
        assert_eq!(
            build_dedup_name("Foo", Some(1), Some("server.luau")),
            "Foo~1.server.luau"
        );
        assert_eq!(build_dedup_name("Foo", Some(2), Some("luau")), "Foo~2.luau");
    }

    #[test]
    fn build_name_dir() {
        assert_eq!(build_dedup_name("Foo", None, None), "Foo");
        assert_eq!(build_dedup_name("Foo", Some(1), None), "Foo~1");
    }

    #[test]
    fn cleanup_gap_tolerant() {
        // Delete ~1 from {Foo, Foo~1, Foo~2}: no rename needed.
        let remaining = vec!["Foo".to_string(), "Foo~2".to_string()];
        let action = compute_cleanup_action("Foo", None, &remaining, false, Path::new("/parent"));
        assert_eq!(action, DedupCleanupAction::None);
    }

    #[test]
    fn cleanup_group_to_one() {
        // Delete base from {Foo, Foo~1}: one remains, remove suffix.
        let remaining = vec!["Foo~1".to_string()];
        let action = compute_cleanup_action("Foo", None, &remaining, true, Path::new("/parent"));
        assert_eq!(
            action,
            DedupCleanupAction::RemoveSuffix {
                from: PathBuf::from("/parent/Foo~1"),
                to: PathBuf::from("/parent/Foo"),
            }
        );
    }

    #[test]
    fn cleanup_group_to_one_file() {
        // Delete base from {Foo.luau, Foo~1.luau}: one remains, remove suffix.
        let remaining = vec!["Foo~1".to_string()];
        let action =
            compute_cleanup_action("Foo", Some("luau"), &remaining, true, Path::new("/parent"));
        assert_eq!(
            action,
            DedupCleanupAction::RemoveSuffix {
                from: PathBuf::from("/parent/Foo~1.luau"),
                to: PathBuf::from("/parent/Foo.luau"),
            }
        );
    }

    #[test]
    fn cleanup_base_deleted_promote_lowest() {
        // Delete base from {Foo, Foo~1, Foo~2}: promote ~1 to base.
        let remaining = vec!["Foo~1".to_string(), "Foo~2".to_string()];
        let action = compute_cleanup_action("Foo", None, &remaining, true, Path::new("/parent"));
        assert_eq!(
            action,
            DedupCleanupAction::PromoteLowest {
                from: PathBuf::from("/parent/Foo~1"),
                to: PathBuf::from("/parent/Foo"),
            }
        );
    }

    #[test]
    fn cleanup_base_deleted_promote_with_gap() {
        // Delete base from {Foo, Foo~2, Foo~5} (gap at ~1): promote ~2 to base.
        let remaining = vec!["Foo~2".to_string(), "Foo~5".to_string()];
        let action = compute_cleanup_action("Foo", None, &remaining, true, Path::new("/parent"));
        assert_eq!(
            action,
            DedupCleanupAction::PromoteLowest {
                from: PathBuf::from("/parent/Foo~2"),
                to: PathBuf::from("/parent/Foo"),
            }
        );
    }

    #[test]
    fn cleanup_suffix_deleted_no_action() {
        // Delete ~1 from {Foo, Foo~1}: group shrinks to 1.
        // Since deleted_was_base=false and only 1 remains, remove its suffix.
        // Wait - if ~1 is deleted and only Foo remains, Foo has no suffix. No rename needed.
        let remaining = vec!["Foo".to_string()];
        let action = compute_cleanup_action("Foo", None, &remaining, false, Path::new("/parent"));
        // "Foo" is already the clean name, so no action.
        assert_eq!(action, DedupCleanupAction::None);
    }

    #[test]
    fn cleanup_no_remaining() {
        let action = compute_cleanup_action("Foo", None, &[], false, Path::new("/parent"));
        assert_eq!(action, DedupCleanupAction::None);
    }
}
