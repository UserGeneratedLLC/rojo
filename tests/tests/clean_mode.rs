//! Tests for clean mode (default) vs incremental mode behavior.
//!
//! These tests verify that:
//! 1. Clean mode removes orphaned files
//! 2. Clean mode doesn't remove files that are being written to
//! 3. Clean mode doesn't remove parent directories of files being written
//! 4. Incremental mode preserves existing file structure

use std::path::Path;

use tempfile::tempdir;

use crate::rojo_test::io_util::{atlas_command, copy_recursive};

const SYNCBACK_TESTS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/rojo-test/syncback-tests");

/// Helper to run syncback and return success status
fn run_syncback(project_path: &Path, input_path: &Path, incremental: bool) -> bool {
    let mut args = vec![
        "--color",
        "never",
        "syncback",
        project_path.to_str().unwrap(),
        "--input",
        input_path.to_str().unwrap(),
    ];

    if incremental {
        args.push("--incremental");
    }

    let output = atlas_command()
        .args(args)
        .output()
        .expect("Couldn't spawn syncback process");

    if !output.status.success() {
        eprintln!("Syncback failed!");
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    output.status.success()
}

/// Test that clean mode removes orphaned files that don't exist in the input.
/// Uses the sync_rules test case which has custom extensions (.modulescript, .text)
/// that get replaced with standard extensions (.luau, .txt) in clean mode.
#[test]
fn clean_mode_removes_orphaned_files() {
    let _ = tracing_subscriber::fmt::try_init();

    // Use the sync_rules test case - it has files with custom extensions
    // that become orphaned when replaced with standard extensions
    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input.rbxm");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    // The original files with custom extensions should exist
    let modulescript_file = project_path.join("src").join("module.modulescript");
    let text_file = project_path.join("src").join("text.text");
    assert!(
        modulescript_file.exists(),
        "module.modulescript should exist before syncback"
    );
    assert!(text_file.exists(), "text.text should exist before syncback");

    // Run syncback in clean mode (default)
    assert!(
        run_syncback(&project_path, &input_file, false),
        "Syncback should succeed"
    );

    // The old custom extension files should be removed in clean mode
    // (they are orphaned because new files with standard extensions replace them)
    assert!(
        !modulescript_file.exists(),
        "module.modulescript should be removed in clean mode"
    );
    assert!(
        !text_file.exists(),
        "text.text should be removed in clean mode"
    );

    // The new files with standard extensions should exist
    let module_file = project_path.join("src").join("module.luau");
    let txt_file = project_path.join("src").join("text.txt");
    assert!(
        module_file.exists(),
        "module.luau should be created by syncback"
    );
    assert!(txt_file.exists(), "text.txt should be created by syncback");
}

/// Test that clean mode doesn't remove files that are being written to
#[test]
fn clean_mode_preserves_written_files() {
    let _ = tracing_subscriber::fmt::try_init();

    // Use the child_but_not test case
    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("child_but_not")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("child_but_not")
        .join("input.rbxl");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    // Run syncback in clean mode
    assert!(
        run_syncback(&project_path, &input_file, false),
        "Syncback should succeed"
    );

    // The directories being written to should exist
    let only_one_copy = project_path.join("OnlyOneCopy");
    let replicated_storage = project_path.join("ReplicatedStorage");

    assert!(
        only_one_copy.exists(),
        "OnlyOneCopy directory should exist (it's being written to)"
    );
    assert!(
        replicated_storage.exists(),
        "ReplicatedStorage directory should exist (it's being written to)"
    );

    // The files inside should exist
    assert!(
        only_one_copy.join("child_of_one.luau").exists(),
        "child_of_one.luau should exist"
    );
    assert!(
        replicated_storage
            .join("child_replicated_storage.luau")
            .exists(),
        "child_replicated_storage.luau should exist"
    );
}

/// Test that incremental mode preserves existing file structure
#[test]
fn incremental_mode_preserves_structure() {
    let _ = tracing_subscriber::fmt::try_init();

    // Use the sync_rules test case - it has custom extensions
    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input.rbxm");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    // Verify the custom-extension files exist before syncback
    let modulescript = project_path.join("src").join("module.modulescript");
    let text = project_path.join("src").join("text.text");
    assert!(
        modulescript.exists(),
        "module.modulescript should exist before syncback"
    );
    assert!(text.exists(), "text.text should exist before syncback");

    // Run syncback in incremental mode
    assert!(
        run_syncback(&project_path, &input_file, true),
        "Syncback should succeed"
    );

    // In incremental mode, the custom extensions should be preserved
    assert!(
        modulescript.exists(),
        "module.modulescript should be preserved in incremental mode"
    );
    assert!(
        text.exists(),
        "text.text should be preserved in incremental mode"
    );
}

/// Test that clean mode uses fresh file extensions (not preserving old ones)
#[test]
fn clean_mode_uses_fresh_extensions() {
    let _ = tracing_subscriber::fmt::try_init();

    // Use the sync_rules test case - it has custom extensions
    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input.rbxm");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    // Run syncback in clean mode (default)
    assert!(
        run_syncback(&project_path, &input_file, false),
        "Syncback should succeed"
    );

    // In clean mode, the files should use standard extensions
    let modulescript_old = project_path.join("src").join("module.modulescript");
    let text_old = project_path.join("src").join("text.text");
    let module_new = project_path.join("src").join("module.luau");
    let text_new = project_path.join("src").join("text.txt");

    // Old extensions should NOT exist
    assert!(
        !modulescript_old.exists(),
        "module.modulescript should NOT exist in clean mode"
    );
    assert!(
        !text_old.exists(),
        "text.text should NOT exist in clean mode"
    );

    // New standard extensions SHOULD exist
    assert!(
        module_new.exists(),
        "module.luau should exist in clean mode"
    );
    assert!(text_new.exists(), "text.txt should exist in clean mode");
}

/// Test that clean mode doesn't remove the project file
#[test]
fn clean_mode_preserves_project_file() {
    let _ = tracing_subscriber::fmt::try_init();

    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("ignore_trees_adding")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("ignore_trees_adding")
        .join("input.rbxl");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    let project_file = project_path.join("default.project.json5");
    assert!(
        project_file.exists(),
        "Project file should exist before syncback"
    );

    // Run syncback in clean mode
    assert!(
        run_syncback(&project_path, &input_file, false),
        "Syncback should succeed"
    );

    // Project file should still exist
    assert!(
        project_file.exists(),
        "Project file should be preserved in clean mode"
    );
}

/// Test that running clean mode twice produces identical results
#[test]
fn clean_mode_is_idempotent() {
    let _ = tracing_subscriber::fmt::try_init();

    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("child_but_not")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("child_but_not")
        .join("input.rbxl");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    // Run syncback twice in clean mode
    assert!(
        run_syncback(&project_path, &input_file, false),
        "First syncback should succeed"
    );

    // Capture state after first run
    let file1 = project_path.join("OnlyOneCopy").join("child_of_one.luau");
    let content1 = fs_err::read_to_string(&file1).unwrap();

    assert!(
        run_syncback(&project_path, &input_file, false),
        "Second syncback should succeed"
    );

    // File should still exist with same content
    assert!(
        file1.exists(),
        "File should still exist after second syncback"
    );
    let content2 = fs_err::read_to_string(&file1).unwrap();
    assert_eq!(
        content1, content2,
        "File content should be identical after second syncback"
    );
}

/// Test that clean mode does NOT delete hidden directories or their contents
/// inside $path source directories. Hidden directories like .vscode/ or
/// .cursor/ should be left untouched by the orphan scanner.
#[test]
fn clean_mode_preserves_hidden_directories() {
    let _ = tracing_subscriber::fmt::try_init();

    let source_path = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input-project");
    let input_file = Path::new(SYNCBACK_TESTS_PATH)
        .join("sync_rules")
        .join("input.rbxm");

    let test_dir = tempdir().expect("Couldn't create temporary directory");
    let project_path = test_dir.path().join("test_project");

    fs_err::create_dir(&project_path).expect("Couldn't create project directory");
    copy_recursive(&source_path, &project_path).expect("Couldn't copy project");

    let src_dir = project_path.join("src");
    let hidden_dir = src_dir.join(".hidden");
    fs_err::create_dir_all(&hidden_dir).expect("Couldn't create hidden directory");
    fs_err::write(hidden_dir.join("keep.txt"), "do not delete").unwrap();
    fs_err::write(hidden_dir.join("config.txt"), "some config").unwrap();

    let nested_hidden = hidden_dir.join("nested");
    fs_err::create_dir_all(&nested_hidden).unwrap();
    fs_err::write(nested_hidden.join("deep.txt"), "also keep").unwrap();

    assert!(
        run_syncback(&project_path, &input_file, false),
        "Syncback should succeed"
    );

    assert!(
        hidden_dir.exists(),
        ".hidden directory should survive clean mode syncback"
    );
    assert!(
        hidden_dir.join("keep.txt").exists(),
        ".hidden/keep.txt should survive"
    );
    assert!(
        hidden_dir.join("config.txt").exists(),
        ".hidden/config.txt should survive"
    );
    assert!(
        nested_hidden.join("deep.txt").exists(),
        ".hidden/nested/deep.txt should survive"
    );
}
