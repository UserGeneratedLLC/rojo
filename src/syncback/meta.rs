//! Shared helpers for reading and writing the `name` field in `.meta.json5` files.
//!
//! These are used by both `change_processor.rs` (two-way sync renames) and
//! `web/api.rs` (syncback added/removed instances) to keep meta file handling
//! DRY and consistent.

use anyhow::Context;
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

/// Upsert the `name` field inside a `.model.json5` / `.model.json` file.
///
/// Parses the existing JSON, sets/replaces the `name` key, and rewrites.
/// Unlike `upsert_meta_name`, this modifies the model file in-place.
pub fn upsert_model_name(model_path: &Path, real_name: &str) -> anyhow::Result<()> {
    let bytes = fs::read(model_path)?;
    let mut obj = match crate::json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(map)) => map,
        _ => anyhow::bail!("model file is not a JSON object: {}", model_path.display()),
    };
    obj.insert(
        "name".to_string(),
        serde_json::Value::String(real_name.to_string()),
    );
    let content = crate::json::to_vec_pretty_sorted(&serde_json::Value::Object(obj))?;
    fs::write(model_path, &content)?;
    Ok(())
}

/// Remove the `name` field from a `.model.json5` / `.model.json` file.
///
/// Unlike meta files, model files are never deleted when they become "empty"
/// (they always have at least `className`). Returns `RemoveNameOutcome` for
/// consistency with the meta helpers.
pub fn remove_model_name(model_path: &Path) -> anyhow::Result<RemoveNameOutcome> {
    if !model_path.exists() {
        return Ok(RemoveNameOutcome::NoOp);
    }
    let bytes = match fs::read(model_path) {
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
    let content = crate::json::to_vec_pretty_sorted(&serde_json::Value::Object(obj))?;
    fs::write(model_path, &content)?;
    Ok(RemoveNameOutcome::FieldRemoved)
}

/// Update `Rojo_Ref_*` attribute paths in a meta/model JSON5 file.
///
/// For each attribute whose key starts with `Rojo_Ref_` and whose string
/// value starts with `old_prefix`, replaces the prefix with `new_prefix`.
/// Returns true if any attribute was updated.
/// Update `Rojo_Ref_*` attributes in a meta/model file after a rename.
///
/// For each attribute, resolves the on-disk relative path to absolute using
/// `source_abs`, checks if the resolved absolute path is affected by the
/// rename (`old_prefix` → `new_prefix`), and if so, recomputes the relative
/// path from `source_abs` to the new absolute target.
pub fn update_ref_paths_in_file(
    file_path: &Path,
    old_prefix: &str,
    new_prefix: &str,
    source_abs: &str,
) -> anyhow::Result<bool> {
    use crate::REF_PATH_ATTRIBUTE_PREFIX;

    if !file_path.exists() {
        return Ok(false);
    }

    let bytes =
        fs::read(file_path).with_context(|| format!("Failed to read {}", file_path.display()))?;
    let mut val: serde_json::Value = crate::json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse JSON5 in {}", file_path.display()))?;
    if !val.is_object() {
        anyhow::bail!(
            "{} is not a JSON object, cannot update Rojo_Ref_* attributes",
            file_path.display()
        );
    }

    let old_prefix_slash = format!("{old_prefix}/");
    let mut updated = false;
    if let Some(attrs) = val.get_mut("attributes").and_then(|a| a.as_object_mut()) {
        for (key, value) in attrs.iter_mut() {
            if !key.starts_with(REF_PATH_ATTRIBUTE_PREFIX) {
                continue;
            }
            let Some(path_str) = value.as_str() else {
                continue;
            };
            let Some(resolved) =
                crate::resolve_ref_path_to_absolute(path_str, source_abs)
            else {
                continue;
            };
            if resolved == old_prefix || resolved.starts_with(&old_prefix_slash) {
                let new_abs =
                    format!("{new_prefix}{}", &resolved[old_prefix.len()..]);
                let new_relative =
                    crate::compute_relative_ref_path(source_abs, &new_abs);
                *value = serde_json::Value::String(new_relative);
                updated = true;
            }
        }
    }

    if updated {
        let content = crate::json::to_vec_pretty_sorted(&val)?;
        fs::write(file_path, &content)?;
    }

    Ok(updated)
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
