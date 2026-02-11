//! Shared helpers for reading and writing the `name` field in `.meta.json5` files.
//!
//! These are used by both `change_processor.rs` (two-way sync renames) and
//! `web/api.rs` (syncback added/removed instances) to keep meta file handling
//! DRY and consistent.

use std::fs;
use std::path::Path;

/// Upsert the `name` field in a `.meta.json5` file.
///
/// If the file exists, parses it and merges the `name` key (preserving other
/// fields like `ignoreUnknownInstances`). If it doesn't exist, creates a new
/// file with just the `name` key.
///
/// Returns `Ok(true)` if the file was written, `Ok(false)` if serialization
/// failed (logged), or `Err` on I/O failure.
pub fn upsert_meta_name(meta_path: &Path, real_name: &str) -> anyhow::Result<()> {
    let mut obj = if meta_path.exists() {
        match fs::read(meta_path) {
            Ok(bytes) => match crate::json::from_slice::<serde_json::Value>(&bytes) {
                Ok(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            },
            Err(_) => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };
    obj.insert(
        "name".to_string(),
        serde_json::Value::String(real_name.to_string()),
    );
    let content = crate::json::to_vec_pretty_sorted(&serde_json::Value::Object(obj))?;
    fs::write(meta_path, &content)?;
    Ok(())
}

/// Outcome of attempting to remove the `name` field from a meta file.
pub enum RemoveNameOutcome {
    /// The meta file didn't exist or had no `name` field -- nothing changed.
    NoOp,
    /// The `name` field was removed; other fields remain. File was rewritten.
    FieldRemoved,
    /// The `name` field was the only field. The file was deleted entirely.
    FileDeleted,
}

/// Remove the `name` field from a `.meta.json5` file.
///
/// If the file becomes an empty object after removal, deletes it entirely.
/// Returns the outcome so callers can manage filesystem event suppression.
pub fn remove_meta_name(meta_path: &Path) -> anyhow::Result<RemoveNameOutcome> {
    if !meta_path.exists() {
        return Ok(RemoveNameOutcome::NoOp);
    }
    let bytes = match fs::read(meta_path) {
        Ok(b) => b,
        Err(_) => return Ok(RemoveNameOutcome::NoOp),
    };
    let mut obj = match crate::json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => return Ok(RemoveNameOutcome::NoOp),
    };
    if obj.remove("name").is_none() {
        return Ok(RemoveNameOutcome::NoOp);
    }
    if obj.is_empty() {
        fs::remove_file(meta_path)?;
        Ok(RemoveNameOutcome::FileDeleted)
    } else {
        let content = crate::json::to_vec_pretty_sorted(&serde_json::Value::Object(obj))?;
        fs::write(meta_path, &content)?;
        Ok(RemoveNameOutcome::FieldRemoved)
    }
}
