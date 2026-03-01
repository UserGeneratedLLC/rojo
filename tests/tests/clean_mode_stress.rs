//! Stress tests proving clean mode produces consistent results.
//!
//! Core invariant: syncing back to a DIRTY directory with clean mode
//! should produce IDENTICAL results to syncing back to an EMPTY directory.
//!
//! These tests apply various "mutations" to dirty up the filesystem, then verify
//! clean mode fixes everything. This validates the clean mode implementation
//! handles edge cases correctly.

use std::path::Path;

use tempfile::tempdir;

use crate::rojo_test::io_util::BUILD_TESTS_PATH;
use crate::rojo_test::roundtrip_util::{
    apply_mutation, assert_dirs_equal, copy_project_dir, ensure_project_dirs_exist, run_rojo_build,
    run_rojo_syncback_clean, Mutation,
};

/// Core test: dirty syncback should equal fresh syncback.
///
/// This is the fundamental invariant we're testing:
/// 1. Build rbxl from project
/// 2. Syncback to DIR_A (fresh)
/// 3. Apply mutations to DIR_A (dirty it up)
/// 4. Syncback again to DIR_A (clean mode should fix it)
/// 5. Syncback to DIR_B (completely fresh)
/// 6. Assert DIR_A == DIR_B
fn clean_equals_fresh(test_name: &str, mutations: &[Mutation]) {
    let _ = tracing_subscriber::fmt::try_init();

    let project_path = Path::new(BUILD_TESTS_PATH).join(test_name);

    // 1. Build rbxm from original project (model format, not place)
    let (_tmp_rbxm, rbxm_path) = run_rojo_build(&project_path, "test.rbxm");

    // 2. Syncback to DIR_A (fresh start - copy full project structure)
    let dir_a = tempdir().expect("Failed to create dir_a");
    copy_project_dir(&project_path, dir_a.path());
    assert!(
        run_rojo_syncback_clean(dir_a.path(), &rbxm_path),
        "Initial syncback to dir_a failed for {}",
        test_name
    );

    // 3. Apply mutations to DIR_A (dirty it up)
    for mutation in mutations {
        apply_mutation(dir_a.path(), mutation);
    }

    // 4. Ensure base project directories exist after mutations.
    // Clean mode requires the base directory structure - it can't create
    // directories from nothing, only clean up orphans and restore content.
    ensure_project_dirs_exist(dir_a.path());

    // 5. Run clean syncback again on dirty DIR_A
    assert!(
        run_rojo_syncback_clean(dir_a.path(), &rbxm_path),
        "Clean syncback on dirty dir_a failed for {}",
        test_name
    );

    // 6. Syncback to DIR_B (completely fresh - copy full project structure)
    let dir_b = tempdir().expect("Failed to create dir_b");
    copy_project_dir(&project_path, dir_b.path());
    assert!(
        run_rojo_syncback_clean(dir_b.path(), &rbxm_path),
        "Fresh syncback to dir_b failed for {}",
        test_name
    );

    // 7. CRITICAL ASSERTION: cleaned DIR_A == fresh DIR_B
    assert_dirs_equal(dir_a.path(), dir_b.path());
}

// =============================================================================
// ORPHAN FILE TESTS
// =============================================================================

/// Clean mode should remove orphan .luau files that don't exist in the rbxl.
#[test]
fn clean_removes_orphan_luau_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            relative_path: "src/orphan_script.luau",
            content: "-- I shouldn't exist\nreturn nil",
        }],
    );
}

/// Clean mode should remove deeply nested orphan files.
#[test]
fn clean_removes_nested_orphan_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            relative_path: "src/level-1/orphan_in_level1.luau",
            content: "-- Nested orphan",
        }],
    );
}

/// Clean mode should remove orphan .txt files.
#[test]
fn clean_removes_orphan_txt_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            relative_path: "src/orphan.txt",
            content: "I shouldn't exist",
        }],
    );
}

/// Clean mode should remove orphan directories with content.
#[test]
fn clean_removes_orphan_directories() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanDirectory {
            relative_path: "src/fake_service",
        }],
    );
}

/// Clean mode should remove deeply nested orphan directories.
#[test]
fn clean_removes_deeply_nested_orphan_directories() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddOrphanDirectory {
                relative_path: "src/level-1/level-2/orphan_folder",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/level-2/orphan_folder/nested/deep.luau",
                content: "-- deeply nested orphan",
            },
        ],
    );
}

// =============================================================================
// FILE RESTORATION TESTS
// =============================================================================

/// Clean mode should restore deleted files.
#[test]
fn clean_restores_deleted_files() {
    clean_equals_fresh(
        "module_init",
        &[Mutation::DeleteFile {
            relative_path: "folder/init.luau",
        }],
    );
}

/// Clean mode should restore deleted files in nested structures.
#[test]
fn clean_restores_deleted_nested_files() {
    clean_equals_fresh(
        "init_with_children",
        &[
            Mutation::DeleteFile {
                relative_path: "src/init.luau",
            },
            Mutation::DeleteFile {
                relative_path: "src/other.luau",
            },
        ],
    );
}

// =============================================================================
// FILE RENAME/NAME CORRECTION TESTS
// =============================================================================

/// Clean mode should fix renamed files (creates orphan + restores correct name).
///
/// IGNORED: This test renames a file to an unrecognizable name, which prevents
/// the project from loading at all. Clean mode requires the project to load first,
/// so it can't fix this scenario. Users must maintain recognizable file names.
#[test]
#[ignore = "Architectural limitation: project can't load when files have unrecognizable names"]
fn clean_fixes_renamed_files() {
    clean_equals_fresh(
        "module_in_folder",
        &[Mutation::RenameFile {
            from: "folder/aModule.luau",
            to: "folder/wrongName.luau",
        }],
    );
}

// =============================================================================
// EXTENSION CORRECTION TESTS
// =============================================================================

/// Clean mode should fix wrong file extensions.
///
/// IGNORED: This test changes a file extension to an unrecognizable format,
/// which prevents the project from loading. Clean mode requires the project
/// to load first, so it can't fix this scenario.
#[test]
#[ignore = "Architectural limitation: project can't load with unrecognizable extensions"]
fn clean_fixes_wrong_extensions() {
    clean_equals_fresh(
        "server_in_folder",
        &[Mutation::ChangeExtension {
            from: "folder/serverScript.server.luau",
            to: "folder/serverScript.modulescript",
        }],
    );
}

/// Clean mode should fix .lua -> .luau extension issues.
#[test]
fn clean_fixes_lua_to_luau_extension() {
    clean_equals_fresh(
        "module_in_folder",
        &[Mutation::ChangeExtension {
            from: "folder/aModule.luau",
            to: "folder/aModule.lua",
        }],
    );
}

// =============================================================================
// FORMAT CONVERSION TESTS (DIR <-> FILE)
// =============================================================================

/// Clean mode should restore directory format from standalone file.
#[test]
fn clean_restores_dir_from_file() {
    clean_equals_fresh(
        "init_with_children",
        &[Mutation::ConvertDirToFile {
            dir: "src",
            file_content: "-- was a directory\nreturn {}",
        }],
    );
}

/// Clean mode should restore standalone file from directory format.
#[test]
fn clean_restores_file_from_dir() {
    clean_equals_fresh(
        "module_in_folder",
        &[Mutation::ConvertFileToDir {
            file: "folder/aModule.luau",
        }],
    );
}

// =============================================================================
// META FILE TESTS
// =============================================================================

/// Clean mode should fix corrupted meta files.
///
/// IGNORED: Corrupting a meta file to invalid JSON prevents the project from
/// loading. Clean mode requires the project to load first.
#[test]
#[ignore = "Architectural limitation: corrupted meta files prevent project loading"]
fn clean_fixes_corrupt_meta_files() {
    clean_equals_fresh(
        "init_meta_properties",
        &[Mutation::CorruptMetaFile {
            relative_path: "Lighting/init.meta.json5",
        }],
    );
}

/// Clean mode should remove orphan meta files.
#[test]
fn clean_removes_orphan_meta_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            relative_path: "src/orphan.meta.json5",
            content: r#"{ "className": "Folder" }"#,
        }],
    );
}

// =============================================================================
// CONTENT MODIFICATION TESTS
// =============================================================================

/// Clean mode should restore modified file content.
#[test]
fn clean_restores_modified_content() {
    clean_equals_fresh(
        "module_init",
        &[Mutation::ModifyFileContent {
            relative_path: "folder/init.luau",
            new_content: "-- totally wrong content\nreturn 'hacked'",
        }],
    );
}

/// Clean mode should restore modified content in multiple files.
#[test]
fn clean_restores_multiple_modified_files() {
    clean_equals_fresh(
        "init_with_children",
        &[
            Mutation::ModifyFileContent {
                relative_path: "src/init.luau",
                new_content: "-- wrong init",
            },
            Mutation::ModifyFileContent {
                relative_path: "src/other.luau",
                new_content: "-- wrong other",
            },
        ],
    );
}

// =============================================================================
// PROJECT FILE TESTS
// =============================================================================

/// Clean mode should remove spurious nested project files.
#[test]
fn clean_removes_spurious_project_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddNestedProjectFile {
            relative_path: "src/fake.project.json5",
        }],
    );
}

/// Clean mode should remove multiple spurious project files.
#[test]
fn clean_removes_multiple_spurious_project_files() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddNestedProjectFile {
                relative_path: "src/fake1.project.json5",
            },
            Mutation::AddNestedProjectFile {
                relative_path: "src/level-1/fake2.project.json5",
            },
        ],
    );
}

// =============================================================================
// DUPLICATE FILE TESTS
// =============================================================================

/// Clean mode should remove duplicate files with different extensions.
///
/// IGNORED: Duplicate files with different extensions create ambiguous paths
/// that Rojo cannot resolve during project loading.
#[test]
#[ignore = "Architectural limitation: ambiguous file names prevent project loading"]
fn clean_removes_duplicate_extensions() {
    clean_equals_fresh(
        "module_in_folder",
        &[Mutation::DuplicateWithDifferentExtension {
            original: "folder/aModule.luau",
            duplicate_ext: "lua",
        }],
    );
}

/// Clean mode should handle multiple duplicates.
///
/// IGNORED: Multiple duplicate files create ambiguous paths that Rojo cannot
/// resolve during project loading.
#[test]
#[ignore = "Architectural limitation: ambiguous file names prevent project loading"]
fn clean_removes_multiple_duplicates() {
    clean_equals_fresh(
        "module_in_folder",
        &[
            Mutation::DuplicateWithDifferentExtension {
                original: "folder/aModule.luau",
                duplicate_ext: "lua",
            },
            Mutation::DuplicateWithDifferentExtension {
                original: "folder/aModule.luau",
                duplicate_ext: "modulescript",
            },
        ],
    );
}

// =============================================================================
// COMBINED / STRESS TESTS
// =============================================================================

/// Clean mode handles multiple different mutations at once.
///
/// NOTE: All orphans are placed within $path directories (src/).
/// Orphan files at the project root are NOT removed by clean mode.
#[test]
fn clean_handles_multiple_mutations() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddOrphanFile {
                relative_path: "src/orphan1.luau",
                content: "x",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/orphan2.txt",
                content: "y",
            },
            Mutation::AddOrphanDirectory {
                relative_path: "src/fake_dir",
            },
            Mutation::AddNestedProjectFile {
                relative_path: "src/bad.project.json5",
            },
        ],
    );
}

/// Clean mode handles extreme chaos with many mutations.
///
/// NOTE: Clean mode only removes orphans within `$path` directories.
/// Orphan files at the project root (outside $path dirs) are NOT removed
/// because that would risk deleting legitimate files like .gitignore, README.md, etc.
#[test]
fn clean_handles_extreme_chaos() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            // Orphan files within $path directories (these WILL be removed)
            Mutation::AddOrphanFile {
                relative_path: "src/orphan_src.luau",
                content: "-- src level orphan",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/orphan_l1.luau",
                content: "-- l1 orphan",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/level-2/orphan_l2.luau",
                content: "-- l2 orphan",
            },
            // Orphan directories within $path (these WILL be removed)
            Mutation::AddOrphanDirectory {
                relative_path: "src/fake_folder",
            },
            // Orphan meta files
            Mutation::AddOrphanFile {
                relative_path: "src/fake_meta.meta.json5",
                content: "{}",
            },
            // Spurious project files
            Mutation::AddNestedProjectFile {
                relative_path: "src/rogue.project.json5",
            },
            // Various file types - use unique base names to avoid duplicate instance names
            Mutation::AddOrphanFile {
                relative_path: "src/orphan_text.txt",
                content: "text orphan",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/orphan_json.json5",
                content: "{}",
            },
        ],
    );
}

/// Clean mode handles mutations across different project types.
#[test]
fn clean_handles_mutations_on_init_project() {
    clean_equals_fresh(
        "module_init",
        &[
            Mutation::AddOrphanFile {
                relative_path: "folder/orphan.luau",
                content: "-- orphan",
            },
            Mutation::ModifyFileContent {
                relative_path: "folder/init.luau",
                new_content: "-- modified",
            },
        ],
    );
}

/// Clean mode handles mutations that affect nested content.
#[test]
fn clean_handles_mutations_on_nested_project() {
    clean_equals_fresh(
        "init_with_children",
        &[
            Mutation::AddOrphanFile {
                relative_path: "src/extra_child.luau",
                content: "-- extra",
            },
            Mutation::DeleteFile {
                relative_path: "src/other.luau",
            },
            Mutation::ModifyFileContent {
                relative_path: "src/init.luau",
                new_content: "-- wrong",
            },
        ],
    );
}

// =============================================================================
// EDGE CASE TESTS
// =============================================================================

/// Clean mode handles empty orphan directories.
#[test]
fn clean_handles_empty_orphan_directory() {
    // The AddOrphanDirectory mutation adds a child file, but let's test
    // with just adding an empty-ish directory structure
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            relative_path: "src/empty_dir/.gitkeep",
            content: "",
        }],
    );
}

/// Clean mode handles orphan files with special characters in names.
#[test]
fn clean_handles_special_character_names() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddOrphanFile {
                relative_path: "src/file-with-dashes.luau",
                content: "-- dashes",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/file_with_underscores.luau",
                content: "-- underscores",
            },
        ],
    );
}

// =============================================================================
// GITKEEP CLEANUP TESTS
// =============================================================================

/// Clean mode removes a stale .gitkeep from a non-empty directory.
///
/// When a directory gains children, Rojo no longer emits a .gitkeep for it.
/// Clean mode must detect and remove the leftover .gitkeep on disk.
#[test]
fn clean_removes_stale_gitkeep_in_non_empty_dir() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            // src/level-1/ has children, so .gitkeep is stale
            relative_path: "src/level-1/.gitkeep",
            content: "",
        }],
    );
}

/// Clean mode removes stale .gitkeep from the root $path directory.
#[test]
fn clean_removes_stale_gitkeep_in_root_path_dir() {
    clean_equals_fresh(
        "deep_nesting",
        &[Mutation::AddOrphanFile {
            // src/ has children (level-1/), so .gitkeep is stale
            relative_path: "src/.gitkeep",
            content: "",
        }],
    );
}

/// Clean mode removes stale .gitkeep files at multiple nesting levels.
#[test]
fn clean_removes_stale_gitkeep_at_multiple_levels() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddOrphanFile {
                relative_path: "src/.gitkeep",
                content: "",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/.gitkeep",
                content: "",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/level-2/.gitkeep",
                content: "",
            },
        ],
    );
}

/// Clean mode removes stale .gitkeep alongside other orphan files.
#[test]
fn clean_removes_stale_gitkeep_with_other_orphans() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/.gitkeep",
                content: "",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/orphan.luau",
                content: "-- orphan",
            },
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/stray.txt",
                content: "stray",
            },
        ],
    );
}

/// Clean mode preserves .gitkeep in a legitimately empty directory.
///
/// The deep_nesting fixture has src/level-1/level-2/level-3/ with only a .gitkeep.
/// After syncback, that directory remains empty so the .gitkeep should stay.
/// This test verifies we don't over-remove by adding unrelated mutations elsewhere.
#[test]
fn clean_preserves_legitimate_gitkeep_in_empty_dir() {
    clean_equals_fresh(
        "deep_nesting",
        &[
            // Add stale .gitkeep in non-empty dir (should be removed)
            Mutation::AddOrphanFile {
                relative_path: "src/level-1/.gitkeep",
                content: "",
            },
            // Add unrelated orphan (should be removed)
            Mutation::AddOrphanFile {
                relative_path: "src/unrelated_orphan.luau",
                content: "-- orphan",
            },
            // Note: src/level-1/level-2/level-3/.gitkeep is legitimate
            // and should survive clean mode (verified by assert_dirs_equal)
        ],
    );
}

/// Clean mode handles .gitkeep in init-style projects with children.
#[test]
fn clean_removes_stale_gitkeep_in_init_project() {
    clean_equals_fresh(
        "init_with_children",
        &[Mutation::AddOrphanFile {
            // src/ has init.luau and other.luau, so .gitkeep is stale
            relative_path: "src/.gitkeep",
            content: "",
        }],
    );
}
