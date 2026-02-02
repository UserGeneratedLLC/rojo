---
name: Syncback Roundtrip Tests
overview: Add comprehensive build-syncback roundtrip tests and mutation stress tests to validate clean mode produces consistent results, complementing the existing 100+ syncback tests.
todos:
  - id: roundtrip-utils
    content: "Create `tests/rojo_test/roundtrip_util.rs` with helpers: `run_rojo_build`, `assert_dirs_equal`, `Mutation` enum and `apply_mutation`"
    status: completed
  - id: roundtrip-tests
    content: Create `tests/tests/syncback_roundtrip.rs` with build -> syncback -> rebuild roundtrip tests using existing build-test projects
    status: completed
  - id: clean-stress-tests
    content: Create `tests/tests/clean_mode_stress.rs` with clean-equals-fresh tests covering 12+ mutation scenarios
    status: completed
  - id: wire-modules
    content: Update `tests/rojo_test/mod.rs` and `tests/tests/mod.rs` to include new test modules
    status: in_progress
isProject: false
---

# Syncback Roundtrip and Clean Mode Stress Tests

## Vision and Philosophy

**Goal**: Make syncback bulletproof for complex games. No filesystem inconsistencies. The filesystem must be the source of truth.

**Testing Philosophy**:

- Tests exist to EXPOSE limitations, not just pass
- Tests that break the system are GOOD - they're the forcing function for quality
- Do NOT back down when tests fail - fix the underlying issue

**Code Quality Concerns**:

- The syncback code has been iterated by AI agents and may have accumulated "fix on fix" complexity
- If a subsystem needs repeated patches, step back and consider a ground-up redesign
- Example: the orphan file detection algorithm may need rethinking rather than more patches
- Use good judgment - don't rewrite everything, but don't just pile on fixes either

---

## Current Status: Bugs Found by Stress Tests

### Bug 1: Nested Orphan Files Not Removed (ACTIVE)

**Symptom**: Orphan files at the top level (`src/orphan.txt`) are removed, but nested orphan files (`src/level-1/orphan.luau`) are NOT removed.

**Debug output**: `Scanned 2 existing paths from filesystem` - but should be finding many more paths including nested orphan files.

**Root cause investigation needed**: The orphan scanning algorithm in `src/syncback/mod.rs` lines 220-292 may have a bug in:

1. How `dirs_to_scan` is populated from root children
2. How the recursive `scan_directory` function traverses the filesystem
3. Path normalization issues between scanned paths and added paths

### Bug 2: Duplicate File Removal Crash (FIXED)

**Symptom**: `failed to remove file: The system cannot find the file specified`

**Root cause**: Same file could be added to `removed_files` with both absolute and relative paths, causing double removal attempts.

**Fix applied**: In `fs_snapshot.rs`, gracefully handle "file not found" errors during removal since the file may have already been deleted.

---

## Context: The Syncback Work Done

This testing plan builds on significant syncback improvements:

### 1. Destructive/Clean Mode (Default)

From [destructive_syncback_mode plan](destructive_syncback_mode_acb8cb54.plan.md):

- Clean mode ignores existing file structure (`old = None`)
- Scans filesystem for ALL existing paths before syncback
- Removes orphaned files after syncback completes
- `--incremental` flag to opt into legacy structure-preserving behavior

Key implementation in `src/syncback/mod.rs`:

- Collects existing paths from filesystem (not just tree)
- Sets `old_ref = None` in clean mode so all children treated as new
- Removes orphaned paths after computing new structure

### 2. Plugin API Architecture Fix

From [architecture fix plan](rojo_syncback_architecture_fix_bc0c13ab.plan.md):

- Tree-based instance lookup before format decisions
- `detect_existing_script_format()` checks filesystem state
- Format preservation to prevent duplicate file creation
- Prevents the "duplicate creation cycle" bug

Key implementation in `src/web/api.rs`:

- `ExistingFileFormat` enum (None, Standalone, Directory)
- Checks if `Name/init.luau` or `Name.luau` exists before deciding format
- Plugin syncback respects existing file structure

---

## Current Test Coverage


| Test File                        | Count | Purpose                                                                |
| -------------------------------- | ----- | ---------------------------------------------------------------------- |
| `syncback.rs`                    | 25    | CLI syncback with pre-built rbxl/rbxm files (18 clean + 7 incremental) |
| `syncback_format_transitions.rs` | 47    | Plugin API format preservation via `/api/write`                        |
| `clean_mode.rs`                  | 6     | Clean vs incremental basic behavior                                    |
| `serve.rs`                       | 22    | General serve tests including write API                                |


**Total: ~100 syncback-related tests**

Test projects in `rojo-test/syncback-tests/` cover:

- Reference properties (6 tests)
- Ignore rules (5 tests)
- Nested projects (2 tests)
- File format/middleware (5 tests)
- Edge cases/conflicts (4 tests)

---

## Key Gaps

1. **No dynamic build-syncback roundtrip**: All tests use static pre-built `.rbxl`/`.rbxm` files, not dynamically built ones
2. **No mutation stress tests**: No tests that dirty the filesystem and verify clean mode fixes it
3. **No "clean == fresh" equivalence proof**: No tests proving clean mode on dirty dir equals fresh syncback

---

## Implementation Plan

### 1. Helper Utilities: `tests/rojo_test/roundtrip_util.rs`

```rust
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use super::io_util::ROJO_PATH;

/// Run `rojo build` and return (temp_dir_handle, output_path)
pub fn run_rojo_build(project_path: &Path, output_name: &str) -> (TempDir, PathBuf) {
    let output_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = output_dir.path().join(output_name);
    
    let output = Command::new(ROJO_PATH)
        .args(["build", project_path.to_str().unwrap(), "-o", output_path.to_str().unwrap()])
        .output()
        .expect("Failed to run rojo build");
    
    if !output.status.success() {
        panic!(
            "rojo build failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    
    (output_dir, output_path)
}

/// Run `rojo syncback` in clean mode (default)
pub fn run_rojo_syncback_clean(project_path: &Path, input_path: &Path) -> bool {
    Command::new(ROJO_PATH)
        .args([
            "syncback",
            project_path.to_str().unwrap(),
            "--input",
            input_path.to_str().unwrap(),
        ])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Compare two directories recursively, asserting they have identical content
pub fn assert_dirs_equal(dir_a: &Path, dir_b: &Path) {
    let files_a = collect_files(dir_a);
    let files_b = collect_files(dir_b);
    
    // Compare file sets
    let set_a: HashSet<_> = files_a.keys().collect();
    let set_b: HashSet<_> = files_b.keys().collect();
    
    let only_in_a: Vec<_> = set_a.difference(&set_b).collect();
    let only_in_b: Vec<_> = set_b.difference(&set_a).collect();
    
    assert!(
        only_in_a.is_empty() && only_in_b.is_empty(),
        "Directory contents differ:\n  Only in A: {:?}\n  Only in B: {:?}",
        only_in_a, only_in_b
    );
    
    // Compare file contents
    for (rel_path, content_a) in &files_a {
        let content_b = &files_b[rel_path];
        assert_eq!(
            content_a, content_b,
            "File contents differ for {:?}", rel_path
        );
    }
}

fn collect_files(dir: &Path) -> HashMap<PathBuf, Vec<u8>> {
    let mut result = HashMap::new();
    collect_files_recursive(dir, dir, &mut result);
    result
}

fn collect_files_recursive(base: &Path, current: &Path, result: &mut HashMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let rel_path = path.strip_prefix(base).unwrap().to_path_buf();
        
        if path.is_dir() {
            collect_files_recursive(base, &path, result);
        } else {
            let content = fs::read(&path).unwrap();
            result.insert(rel_path, content);
        }
    }
}

/// Mutation types for stress testing clean mode
pub enum Mutation {
    /// Add an orphan file that doesn't exist in the rbxl
    AddOrphanFile { relative_path: &'static str, content: &'static str },
    /// Delete a file that exists in the rbxl
    DeleteFile { relative_path: &'static str },
    /// Rename a file (creates orphan + missing file)
    RenameFile { from: &'static str, to: &'static str },
    /// Change file extension (e.g., .luau -> .modulescript)
    ChangeExtension { from: &'static str, to: &'static str },
    /// Add an orphan directory with content
    AddOrphanDirectory { relative_path: &'static str },
    /// Delete an entire directory
    DeleteDirectory { relative_path: &'static str },
    /// Convert directory format to standalone file
    ConvertDirToFile { dir: &'static str, file_content: &'static str },
    /// Convert standalone file to directory format
    ConvertFileToDir { file: &'static str },
    /// Corrupt a .meta.json5 file
    CorruptMetaFile { relative_path: &'static str },
    /// Modify file content to be wrong
    ModifyFileContent { relative_path: &'static str, new_content: &'static str },
    /// Add a spurious .project.json5 file
    AddNestedProjectFile { relative_path: &'static str },
    /// Create duplicate with different extension
    DuplicateWithDifferentExtension { original: &'static str, duplicate_ext: &'static str },
}

pub fn apply_mutation(dir: &Path, mutation: &Mutation) {
    match mutation {
        Mutation::AddOrphanFile { relative_path, content } => {
            let path = dir.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&path, content).expect("Failed to write orphan file");
        }
        Mutation::DeleteFile { relative_path } => {
            let path = dir.join(relative_path);
            fs::remove_file(&path).expect("Failed to delete file");
        }
        Mutation::RenameFile { from, to } => {
            let from_path = dir.join(from);
            let to_path = dir.join(to);
            fs::rename(&from_path, &to_path).expect("Failed to rename file");
        }
        Mutation::ChangeExtension { from, to } => {
            let from_path = dir.join(from);
            let to_path = dir.join(to);
            fs::rename(&from_path, &to_path).expect("Failed to change extension");
        }
        Mutation::AddOrphanDirectory { relative_path } => {
            let path = dir.join(relative_path);
            fs::create_dir_all(&path).expect("Failed to create orphan directory");
            // Add a file so the directory isn't empty
            fs::write(path.join("orphan_child.luau"), "-- orphan").ok();
        }
        Mutation::DeleteDirectory { relative_path } => {
            let path = dir.join(relative_path);
            fs::remove_dir_all(&path).expect("Failed to delete directory");
        }
        Mutation::ConvertDirToFile { dir: dir_path, file_content } => {
            let path = dir.join(dir_path);
            fs::remove_dir_all(&path).expect("Failed to remove directory");
            // Create standalone file with same base name
            let file_path = path.with_extension("luau");
            fs::write(&file_path, file_content).expect("Failed to write file");
        }
        Mutation::ConvertFileToDir { file } => {
            let file_path = dir.join(file);
            let content = fs::read_to_string(&file_path).unwrap_or_default();
            fs::remove_file(&file_path).expect("Failed to remove file");
            // Create directory with init file
            let dir_path = file_path.with_extension("");
            fs::create_dir_all(&dir_path).expect("Failed to create directory");
            fs::write(dir_path.join("init.luau"), content).expect("Failed to write init");
        }
        Mutation::CorruptMetaFile { relative_path } => {
            let path = dir.join(relative_path);
            fs::write(&path, "{ this is not valid json5 {{{{").expect("Failed to corrupt meta");
        }
        Mutation::ModifyFileContent { relative_path, new_content } => {
            let path = dir.join(relative_path);
            fs::write(&path, new_content).expect("Failed to modify content");
        }
        Mutation::AddNestedProjectFile { relative_path } => {
            let path = dir.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::write(&path, r#"{ "name": "SpuriousProject", "tree": { "$className": "Folder" } }"#)
                .expect("Failed to add nested project");
        }
        Mutation::DuplicateWithDifferentExtension { original, duplicate_ext } => {
            let orig_path = dir.join(original);
            let content = fs::read(&orig_path).unwrap_or_default();
            let dup_path = orig_path.with_extension(duplicate_ext);
            fs::write(&dup_path, content).expect("Failed to create duplicate");
        }
    }
}
```

### 2. Roundtrip Tests: `tests/tests/syncback_roundtrip.rs`

```rust
//! Build -> Syncback -> Rebuild roundtrip tests.
//!
//! These tests verify that:
//! 1. Building a project to rbxl
//! 2. Syncing back to a fresh directory
//! 3. Rebuilding from the syncback result
//! Produces semantically identical output.

use std::path::Path;
use tempfile::tempdir;

use crate::rojo_test::io_util::{copy_recursive, BUILD_TESTS_PATH};
use crate::rojo_test::roundtrip_util::{run_rojo_build, run_rojo_syncback_clean};

macro_rules! roundtrip_tests {
    ($($test_name:ident),* $(,)?) => {$(
        #[test]
        fn $test_name() {
            run_roundtrip_test(stringify!($test_name));
        }
    )*};
}

fn run_roundtrip_test(build_test_name: &str) {
    let _ = env_logger::try_init();
    
    let project_path = Path::new(BUILD_TESTS_PATH).join(build_test_name);
    
    // 1. Build original project -> rbxl
    let (_tmp1, original_rbxl) = run_rojo_build(&project_path, "original.rbxl");
    
    // 2. Create fresh syncback target with just project file
    let syncback_dir = tempdir().expect("Failed to create temp dir");
    copy_project_file(&project_path, syncback_dir.path());
    
    // 3. Syncback from rbxl to fresh directory
    assert!(
        run_rojo_syncback_clean(syncback_dir.path(), &original_rbxl),
        "Syncback should succeed"
    );
    
    // 4. Build from syncback result -> rbxl
    let (_tmp2, roundtrip_rbxl) = run_rojo_build(syncback_dir.path(), "roundtrip.rbxl");
    
    // 5. Compare both rbxl files (semantic comparison)
    assert_rbx_equal(&original_rbxl, &roundtrip_rbxl);
}

fn copy_project_file(src: &Path, dst: &Path) {
    // Copy just the project file (default.project.json5)
    for name in ["default.project.json5", "default.project.json"] {
        let src_file = src.join(name);
        if src_file.exists() {
            fs::copy(&src_file, dst.join(name)).unwrap();
            return;
        }
    }
    panic!("No project file found in {:?}", src);
}

fn assert_rbx_equal(file_a: &Path, file_b: &Path) {
    // Load both rbxl files and compare instance trees
    let dom_a = rbx_binary::from_reader(fs::File::open(file_a).unwrap()).unwrap();
    let dom_b = rbx_binary::from_reader(fs::File::open(file_b).unwrap()).unwrap();
    
    // Compare root children (skip DataModel itself)
    compare_instances(&dom_a, dom_a.root_ref(), &dom_b, dom_b.root_ref());
}

// Test ALL build-test projects that should roundtrip cleanly
roundtrip_tests! {
    attributes,
    client_in_folder,
    client_init,
    deep_nesting,
    init_meta_class_name,
    init_meta_properties,
    init_with_children,
    json_model_in_folder,
    json_model_legacy_name,
    module_in_folder,
    module_init,
    project_composed_default,
    project_composed_file,
    project_root_name,
    rbxm_in_folder,
    rbxmx_in_folder,
    server_in_folder,
    server_init,
    txt,
    txt_in_folder,
    sync_rule_alone,
    sync_rule_complex,
}
```

### 3. Clean Mode Stress Tests: `tests/tests/clean_mode_stress.rs`

```rust
//! Stress tests proving clean mode produces consistent results.
//!
//! Core invariant: syncing back to a DIRTY directory with clean mode
//! should produce IDENTICAL results to syncing back to an EMPTY directory.
//!
//! Tests apply various "mutations" to dirty up the filesystem, then verify
//! clean mode fixes everything.

use std::path::Path;
use tempfile::tempdir;

use crate::rojo_test::io_util::{copy_recursive, BUILD_TESTS_PATH};
use crate::rojo_test::roundtrip_util::{
    apply_mutation, assert_dirs_equal, run_rojo_build, run_rojo_syncback_clean, Mutation,
};

/// Core test: dirty syncback should equal fresh syncback
fn clean_equals_fresh(test_name: &str, mutations: &[Mutation]) {
    let _ = env_logger::try_init();
    
    let project_path = Path::new(BUILD_TESTS_PATH).join(test_name);
    
    // 1. Build rbxl from original project
    let (_tmp_rbxl, rbxl_path) = run_rojo_build(&project_path, "test.rbxl");
    
    // 2. Syncback to DIR_A (fresh start)
    let dir_a = tempdir().expect("Failed to create dir_a");
    copy_project_file(&project_path, dir_a.path());
    assert!(run_rojo_syncback_clean(dir_a.path(), &rbxl_path), "Initial syncback failed");
    
    // 3. Apply mutations to DIR_A (dirty it up)
    for mutation in mutations {
        apply_mutation(dir_a.path(), mutation);
    }
    
    // 4. Run clean syncback again on dirty DIR_A
    assert!(run_rojo_syncback_clean(dir_a.path(), &rbxl_path), "Clean syncback on dirty dir failed");
    
    // 5. Syncback to DIR_B (completely fresh)
    let dir_b = tempdir().expect("Failed to create dir_b");
    copy_project_file(&project_path, dir_b.path());
    assert!(run_rojo_syncback_clean(dir_b.path(), &rbxl_path), "Fresh syncback failed");
    
    // 6. CRITICAL ASSERTION: cleaned DIR_A == fresh DIR_B
    assert_dirs_equal(dir_a.path(), dir_b.path());
}

// ==========================================================================
// MUTATION SCENARIO TESTS
// ==========================================================================

#[test]
fn clean_removes_orphan_luau_files() {
    clean_equals_fresh("deep_nesting", &[
        Mutation::AddOrphanFile { 
            relative_path: "src/orphan_script.luau", 
            content: "-- I shouldn't exist" 
        },
        Mutation::AddOrphanFile { 
            relative_path: "src/inner/another_orphan.luau", 
            content: "return nil" 
        },
    ]);
}

#[test]
fn clean_removes_orphan_directories() {
    clean_equals_fresh("deep_nesting", &[
        Mutation::AddOrphanDirectory { 
            relative_path: "src/fake_service" 
        },
    ]);
}

#[test]
fn clean_restores_deleted_files() {
    clean_equals_fresh("module_init", &[
        Mutation::DeleteFile { relative_path: "folder/init.luau" },
    ]);
}

#[test]
fn clean_fixes_renamed_files() {
    clean_equals_fresh("module_in_folder", &[
        Mutation::RenameFile { 
            from: "folder/aModule.luau", 
            to: "folder/wrongName.luau" 
        },
    ]);
}

#[test]
fn clean_fixes_wrong_extensions() {
    clean_equals_fresh("server_in_folder", &[
        Mutation::ChangeExtension { 
            from: "folder/serverScript.server.luau", 
            to: "folder/serverScript.modulescript" 
        },
    ]);
}

#[test]
fn clean_restores_dir_from_file() {
    clean_equals_fresh("init_with_children", &[
        Mutation::ConvertDirToFile { 
            dir: "src", 
            file_content: "-- was a directory" 
        },
    ]);
}

#[test]
fn clean_restores_file_from_dir() {
    clean_equals_fresh("module_in_folder", &[
        Mutation::ConvertFileToDir { 
            file: "folder/aModule.luau"
        },
    ]);
}

#[test]
fn clean_fixes_corrupt_meta_files() {
    clean_equals_fresh("init_meta_properties", &[
        Mutation::CorruptMetaFile { 
            relative_path: "Lighting/init.meta.json5" 
        },
    ]);
}

#[test]
fn clean_restores_modified_content() {
    clean_equals_fresh("module_init", &[
        Mutation::ModifyFileContent { 
            relative_path: "folder/init.luau",
            new_content: "-- totally wrong content\nreturn 'hacked'"
        },
    ]);
}

#[test]
fn clean_removes_spurious_project_files() {
    clean_equals_fresh("deep_nesting", &[
        Mutation::AddNestedProjectFile { 
            relative_path: "src/fake.project.json5" 
        },
    ]);
}

#[test]
fn clean_removes_duplicate_extensions() {
    clean_equals_fresh("module_in_folder", &[
        Mutation::DuplicateWithDifferentExtension {
            original: "folder/aModule.luau",
            duplicate_ext: "lua"
        },
    ]);
}

#[test]
fn clean_handles_multiple_crazy_mutations() {
    clean_equals_fresh("deep_nesting", &[
        Mutation::AddOrphanFile { relative_path: "orphan1.luau", content: "x" },
        Mutation::AddOrphanFile { relative_path: "src/orphan2.txt", content: "y" },
        Mutation::AddOrphanDirectory { relative_path: "fake_dir" },
        Mutation::ModifyFileContent { 
            relative_path: "src/inner/init.luau",
            new_content: "-- wrong!"
        },
        Mutation::AddNestedProjectFile { relative_path: "src/bad.project.json5" },
    ]);
}
```

### 4. Wire Up Modules

`**tests/rojo_test/mod.rs`:**

```rust
pub mod roundtrip_util;
```

`**tests/tests/mod.rs`:**

```rust
mod syncback_roundtrip;
mod clean_mode_stress;
```

---

## Test Matrix


| Mutation Type        | Test Name                                | Verifies                 |
| -------------------- | ---------------------------------------- | ------------------------ |
| Orphan .luau files   | `clean_removes_orphan_luau_files`        | Orphan file removal      |
| Orphan directories   | `clean_removes_orphan_directories`       | Orphan directory removal |
| Deleted files        | `clean_restores_deleted_files`           | File restoration         |
| Renamed files        | `clean_fixes_renamed_files`              | Name correction          |
| Wrong extensions     | `clean_fixes_wrong_extensions`           | Extension normalization  |
| Dir to File          | `clean_restores_dir_from_file`           | Format restoration       |
| File to Dir          | `clean_restores_file_from_dir`           | Format restoration       |
| Corrupt meta         | `clean_fixes_corrupt_meta_files`         | Meta regeneration        |
| Modified content     | `clean_restores_modified_content`        | Content restoration      |
| Spurious projects    | `clean_removes_spurious_project_files`   | Nested project cleanup   |
| Duplicate extensions | `clean_removes_duplicate_extensions`     | Duplicate removal        |
| Combined chaos       | `clean_handles_multiple_crazy_mutations` | Stress test              |


---

## Files to Create/Modify


| File                                | Action                               |
| ----------------------------------- | ------------------------------------ |
| `tests/rojo_test/roundtrip_util.rs` | Create - helper utilities            |
| `tests/tests/syncback_roundtrip.rs` | Create - roundtrip tests             |
| `tests/tests/clean_mode_stress.rs`  | Create - mutation stress tests       |
| `tests/rojo_test/mod.rs`            | Modify - add `roundtrip_util` module |
| `tests/tests/mod.rs`                | Modify - add new test modules        |


---

## Success Criteria

1. All 20+ build-test projects can roundtrip successfully (build -> syncback -> rebuild -> compare)
2. All 12 mutation scenarios pass (clean == fresh invariant holds)
3. Tests run in CI without flakiness
4. Total added test runtime < 60 seconds

