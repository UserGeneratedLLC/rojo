//! Utilities for roundtrip and stress testing of syncback.
//!
//! Provides helpers for:
//! - Building projects to rbxl files
//! - Running syncback in clean mode
//! - Comparing directories for equality
//! - Applying filesystem mutations for stress testing

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::TempDir;

use super::io_util::ROJO_PATH;

/// Run `rojo build` on a project and return the output file path.
///
/// Returns a tuple of (TempDir handle, output path). The TempDir must be kept
/// alive for the output file to remain accessible.
pub fn run_rojo_build(project_path: &Path, output_name: &str) -> (TempDir, PathBuf) {
    let output_dir = tempfile::tempdir().expect("Failed to create temp dir for build output");
    let output_path = output_dir.path().join(output_name);

    let output = Command::new(ROJO_PATH)
        .args([
            "build",
            project_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run rojo build");

    if !output.status.success() {
        panic!(
            "rojo build failed for {:?}:\nstdout: {}\nstderr: {}",
            project_path,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    (output_dir, output_path)
}

/// Run `rojo syncback` in clean mode (default behavior).
///
/// Returns true if syncback succeeded, false otherwise.
pub fn run_rojo_syncback_clean(project_path: &Path, input_path: &Path) -> bool {
    // Verify input file exists before calling
    if !input_path.exists() {
        eprintln!(
            "ERROR: Input file does not exist: {}",
            input_path.display()
        );
        return false;
    }

    let output = Command::new(ROJO_PATH)
        .args([
            "syncback",
            project_path.to_str().unwrap(),
            "--input",
            input_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to run rojo syncback");

    if !output.status.success() {
        eprintln!(
            "rojo syncback failed (exit code {:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    output.status.success()
}

/// Ensure base project directories exist.
///
/// This reads the project file and creates any $path directories that don't exist.
/// Clean mode requires the base directory structure to be present - it can clean up
/// orphans and restore content, but it can't recreate directories from nothing.
pub fn ensure_project_dirs_exist(project_dir: &Path) {
    // Find the project file
    let project_file = project_dir.join("default.project.json5");
    if !project_file.exists() {
        return;
    }

    // Read and parse project file
    let content = fs::read_to_string(&project_file).expect("Failed to read project file");

    // Simple parsing - look for "$path" entries and extract the path values
    // This is a simplified parser that handles common cases
    for line in content.lines() {
        if let Some(path_start) = line.find("\"$path\"") {
            // Find the value after the colon
            let after_key = &line[path_start + 7..];
            if let Some(colon_pos) = after_key.find(':') {
                let after_colon = after_key[colon_pos + 1..].trim();
                // Extract the string value (remove quotes and trailing comma/brace)
                if after_colon.starts_with('"') {
                    let path_value = after_colon
                        .trim_start_matches('"')
                        .split('"')
                        .next()
                        .unwrap_or("");

                    if !path_value.is_empty() {
                        let full_path = project_dir.join(path_value);
                        // Only create if it doesn't exist and has no extension
                        // (directories typically don't have extensions)
                        if !full_path.exists() && full_path.extension().is_none() {
                            fs::create_dir_all(&full_path).ok();
                        }
                    }
                }
            }
        }
    }
}

/// Compare two directories recursively, asserting they have identical content.
///
/// This compares:
/// - The set of files (by relative path)
/// - The content of each file (byte-for-byte)
///
/// Panics with a detailed message if the directories differ.
pub fn assert_dirs_equal(dir_a: &Path, dir_b: &Path) {
    let files_a = collect_files(dir_a);
    let files_b = collect_files(dir_b);

    // Compare file sets
    let set_a: HashSet<_> = files_a.keys().collect();
    let set_b: HashSet<_> = files_b.keys().collect();

    let only_in_a: Vec<_> = set_a.difference(&set_b).collect();
    let only_in_b: Vec<_> = set_b.difference(&set_a).collect();

    if !only_in_a.is_empty() || !only_in_b.is_empty() {
        panic!(
            "Directory contents differ:\n  Only in {:?}: {:?}\n  Only in {:?}: {:?}",
            dir_a, only_in_a, dir_b, only_in_b
        );
    }

    // Compare file contents
    for (rel_path, content_a) in &files_a {
        let content_b = &files_b[rel_path];
        if content_a != content_b {
            // Try to show as text if possible for better error messages
            let text_a = String::from_utf8_lossy(content_a);
            let text_b = String::from_utf8_lossy(content_b);
            panic!(
                "File contents differ for {:?}:\n--- {:?}\n{}\n--- {:?}\n{}",
                rel_path, dir_a, text_a, dir_b, text_b
            );
        }
    }
}

/// Collect all files in a directory recursively.
///
/// Returns a map of relative paths to file contents.
fn collect_files(dir: &Path) -> HashMap<PathBuf, Vec<u8>> {
    let mut result = HashMap::new();
    if dir.exists() {
        collect_files_recursive(dir, dir, &mut result);
    }
    result
}

fn collect_files_recursive(base: &Path, current: &Path, result: &mut HashMap<PathBuf, Vec<u8>>) {
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Warning: Failed to read directory {:?}: {}", current, e);
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Warning: Failed to read directory entry: {}", e);
                continue;
            }
        };

        let path = entry.path();
        let rel_path = path
            .strip_prefix(base)
            .expect("Path should be under base")
            .to_path_buf();

        if path.is_dir() {
            collect_files_recursive(base, &path, result);
        } else {
            match fs::read(&path) {
                Ok(content) => {
                    result.insert(rel_path, content);
                }
                Err(e) => {
                    eprintln!("Warning: Failed to read file {:?}: {}", path, e);
                }
            }
        }
    }
}

/// Mutation types for stress testing clean mode.
///
/// Each mutation represents a way to "dirty" the filesystem that clean mode
/// should be able to fix.
#[derive(Debug, Clone)]
pub enum Mutation {
    /// Add an orphan file that doesn't exist in the rbxl
    AddOrphanFile {
        relative_path: &'static str,
        content: &'static str,
    },
    /// Delete a file that exists in the rbxl
    DeleteFile { relative_path: &'static str },
    /// Rename a file (creates orphan + missing file)
    RenameFile {
        from: &'static str,
        to: &'static str,
    },
    /// Change file extension (e.g., .luau -> .modulescript)
    ChangeExtension {
        from: &'static str,
        to: &'static str,
    },
    /// Add an orphan directory with content
    AddOrphanDirectory { relative_path: &'static str },
    /// Delete an entire directory
    #[allow(dead_code)]
    DeleteDirectory { relative_path: &'static str },
    /// Convert directory format to standalone file
    ConvertDirToFile {
        dir: &'static str,
        file_content: &'static str,
    },
    /// Convert standalone file to directory format
    ConvertFileToDir { file: &'static str },
    /// Corrupt a .meta.json5 file
    CorruptMetaFile { relative_path: &'static str },
    /// Modify file content to be wrong
    ModifyFileContent {
        relative_path: &'static str,
        new_content: &'static str,
    },
    /// Add a spurious .project.json5 file
    AddNestedProjectFile { relative_path: &'static str },
    /// Create duplicate with different extension
    DuplicateWithDifferentExtension {
        original: &'static str,
        duplicate_ext: &'static str,
    },
}

/// Apply a mutation to a directory.
///
/// This modifies the filesystem to simulate various "dirty" states that clean
/// mode syncback should be able to fix.
pub fn apply_mutation(dir: &Path, mutation: &Mutation) {
    match mutation {
        Mutation::AddOrphanFile {
            relative_path,
            content,
        } => {
            let path = dir.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&path, content).expect("Failed to write orphan file");
        }
        Mutation::DeleteFile { relative_path } => {
            let path = dir.join(relative_path);
            if path.exists() {
                fs::remove_file(&path).expect("Failed to delete file");
            }
        }
        Mutation::RenameFile { from, to } => {
            let from_path = dir.join(from);
            let to_path = dir.join(to);
            if let Some(parent) = to_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            if from_path.exists() {
                fs::rename(&from_path, &to_path).expect("Failed to rename file");
            }
        }
        Mutation::ChangeExtension { from, to } => {
            let from_path = dir.join(from);
            let to_path = dir.join(to);
            if from_path.exists() {
                fs::rename(&from_path, &to_path).expect("Failed to change extension");
            }
        }
        Mutation::AddOrphanDirectory { relative_path } => {
            let path = dir.join(relative_path);
            fs::create_dir_all(&path).expect("Failed to create orphan directory");
            // Add a file so the directory isn't empty
            fs::write(path.join("orphan_child.luau"), "-- orphan child").ok();
        }
        Mutation::DeleteDirectory { relative_path } => {
            let path = dir.join(relative_path);
            if path.exists() {
                fs::remove_dir_all(&path).expect("Failed to delete directory");
            }
        }
        Mutation::ConvertDirToFile { dir: dir_path, file_content } => {
            let path = dir.join(dir_path);
            if path.exists() && path.is_dir() {
                fs::remove_dir_all(&path).expect("Failed to remove directory");
            }
            // Create standalone file with same base name + .luau extension
            let file_path = path.with_extension("luau");
            fs::write(&file_path, file_content).expect("Failed to write file");
        }
        Mutation::ConvertFileToDir { file } => {
            let file_path = dir.join(file);
            let content = fs::read_to_string(&file_path).unwrap_or_default();
            if file_path.exists() {
                fs::remove_file(&file_path).expect("Failed to remove file");
            }
            // Create directory with init file
            let dir_path = file_path.with_extension("");
            fs::create_dir_all(&dir_path).expect("Failed to create directory");
            fs::write(dir_path.join("init.luau"), content).expect("Failed to write init");
        }
        Mutation::CorruptMetaFile { relative_path } => {
            let path = dir.join(relative_path);
            fs::write(&path, "{ this is not valid json5 {{{{")
                .expect("Failed to corrupt meta file");
        }
        Mutation::ModifyFileContent {
            relative_path,
            new_content,
        } => {
            let path = dir.join(relative_path);
            fs::write(&path, new_content).expect("Failed to modify content");
        }
        Mutation::AddNestedProjectFile { relative_path } => {
            let path = dir.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(
                &path,
                r#"{ "name": "SpuriousProject", "tree": { "$className": "Folder" } }"#,
            )
            .expect("Failed to add nested project");
        }
        Mutation::DuplicateWithDifferentExtension {
            original,
            duplicate_ext,
        } => {
            let orig_path = dir.join(original);
            let content = fs::read(&orig_path).unwrap_or_default();
            let dup_path = orig_path.with_extension(duplicate_ext);
            fs::write(&dup_path, content).expect("Failed to create duplicate");
        }
    }
}

/// Copy the full project directory from source to destination.
///
/// This copies the entire directory structure, which is required because
/// syncback needs the `$path` directories to exist for building the "old tree".
pub fn copy_project_dir(src: &Path, dst: &Path) {
    copy_dir_recursive(src, dst).expect("Failed to copy project directory");
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dst.join(entry.file_name());

        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}
