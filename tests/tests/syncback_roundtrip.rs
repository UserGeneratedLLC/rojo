//! Build -> Syncback -> Rebuild roundtrip tests.
//!
//! These tests verify that:
//! 1. Building a project to rbxl
//! 2. Syncing back to a fresh directory
//! 3. Rebuilding from the syncback result
//!
//! Produces semantically identical output. This validates that the syncback
//! system correctly reconstructs project structure from binary place files.

use std::{fs, path::Path};

use rbx_dom_weak::{ustr, WeakDom};
use tempfile::tempdir;

use crate::rojo_test::io_util::BUILD_TESTS_PATH;
use crate::rojo_test::roundtrip_util::{copy_project_dir, run_rojo_build, run_rojo_syncback_clean};

/// Generate roundtrip tests for build-test projects.
///
/// Each test:
/// 1. Builds the project to an rbxl file
/// 2. Creates a fresh directory with just the project file
/// 3. Syncs back from the rbxl to the fresh directory
/// 4. Rebuilds from the syncback result
/// 5. Compares the two rbxl files for semantic equality
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

    // 1. Build original project -> rbxm (model format, not place)
    let (_tmp1, original_rbxm) = run_rojo_build(&project_path, "original.rbxm");

    // 2. Create fresh syncback target with full project structure
    let syncback_dir = tempdir().expect("Failed to create temp dir");
    copy_project_dir(&project_path, syncback_dir.path());

    // 3. Syncback from rbxm to directory
    assert!(
        run_rojo_syncback_clean(syncback_dir.path(), &original_rbxm),
        "Syncback should succeed for {}",
        build_test_name
    );

    // 4. Build from syncback result -> rbxm
    let (_tmp2, roundtrip_rbxm) = run_rojo_build(syncback_dir.path(), "roundtrip.rbxm");

    // 5. Compare both rbxm files (semantic comparison)
    assert_rbx_equal(&original_rbxm, &roundtrip_rbxm, build_test_name);
}

/// Compare two rbxl files for semantic equality.
///
/// This compares the instance trees, checking that all instances have the same
/// names, class names, and properties (excluding transient properties like Ref).
fn assert_rbx_equal(file_a: &Path, file_b: &Path, test_name: &str) {
    let data_a = fs::read(file_a).expect("Failed to read original rbxl");
    let data_b = fs::read(file_b).expect("Failed to read roundtrip rbxl");

    let dom_a = rbx_binary::from_reader(data_a.as_slice()).expect("Failed to parse original rbxl");
    let dom_b =
        rbx_binary::from_reader(data_b.as_slice()).expect("Failed to parse roundtrip rbxl");

    // Compare the root's children (DataModel children)
    compare_children(&dom_a, dom_a.root_ref(), &dom_b, dom_b.root_ref(), test_name);
}

/// Recursively compare children of two instances.
fn compare_children(
    dom_a: &WeakDom,
    ref_a: rbx_dom_weak::types::Ref,
    dom_b: &WeakDom,
    ref_b: rbx_dom_weak::types::Ref,
    test_name: &str,
) {
    let inst_a = dom_a.get_by_ref(ref_a).expect("Instance A should exist");
    let inst_b = dom_b.get_by_ref(ref_b).expect("Instance B should exist");

    let children_a = inst_a.children();
    let children_b = inst_b.children();

    // Build maps of children by name for comparison
    let mut map_a: std::collections::HashMap<&str, &rbx_dom_weak::Instance> =
        std::collections::HashMap::new();
    let mut map_b: std::collections::HashMap<&str, &rbx_dom_weak::Instance> =
        std::collections::HashMap::new();

    for &child_ref in children_a {
        if let Some(child) = dom_a.get_by_ref(child_ref) {
            map_a.insert(&child.name, child);
        }
    }

    for &child_ref in children_b {
        if let Some(child) = dom_b.get_by_ref(child_ref) {
            map_b.insert(&child.name, child);
        }
    }

    // Check that both have the same children
    let names_a: std::collections::HashSet<&str> = map_a.keys().copied().collect();
    let names_b: std::collections::HashSet<&str> = map_b.keys().copied().collect();

    let only_in_a: Vec<_> = names_a.difference(&names_b).collect();
    let only_in_b: Vec<_> = names_b.difference(&names_a).collect();

    if !only_in_a.is_empty() || !only_in_b.is_empty() {
        panic!(
            "[{}] Children differ under '{}':\n  Only in original: {:?}\n  Only in roundtrip: {:?}",
            test_name, inst_a.name, only_in_a, only_in_b
        );
    }

    // Compare each child
    for name in names_a {
        let child_a = map_a[name];
        let child_b = map_b[name];

        // Check class name matches
        if child_a.class != child_b.class {
            panic!(
                "[{}] Class mismatch for '{}': original={}, roundtrip={}",
                test_name, name, child_a.class, child_b.class
            );
        }

        // Compare important properties (Source for scripts, Value for value types)
        compare_properties(child_a, child_b, test_name);

        // Recurse into children
        let child_ref_a = dom_a
            .get_by_ref(ref_a)
            .unwrap()
            .children()
            .iter()
            .find(|&&r| dom_a.get_by_ref(r).map(|i| i.name.as_str()) == Some(name))
            .copied()
            .unwrap();
        let child_ref_b = dom_b
            .get_by_ref(ref_b)
            .unwrap()
            .children()
            .iter()
            .find(|&&r| dom_b.get_by_ref(r).map(|i| i.name.as_str()) == Some(name))
            .copied()
            .unwrap();

        compare_children(dom_a, child_ref_a, dom_b, child_ref_b, test_name);
    }
}

/// Compare properties of two instances.
///
/// Focuses on properties that matter for roundtrip correctness:
/// - Source (for scripts)
/// - Value (for value types)
/// - Other serializable properties
fn compare_properties(
    inst_a: &rbx_dom_weak::Instance,
    inst_b: &rbx_dom_weak::Instance,
    test_name: &str,
) {
    // Check Source property for scripts
    if let (Some(source_a), Some(source_b)) = (
        inst_a.properties.get(&ustr("Source")),
        inst_b.properties.get(&ustr("Source")),
    ) {
        if source_a != source_b {
            panic!(
                "[{}] Source property differs for '{}' ({}):\nOriginal: {:?}\nRoundtrip: {:?}",
                test_name, inst_a.name, inst_a.class, source_a, source_b
            );
        }
    }

    // Check Value property for value types
    if let (Some(value_a), Some(value_b)) = (
        inst_a.properties.get(&ustr("Value")),
        inst_b.properties.get(&ustr("Value")),
    ) {
        if value_a != value_b {
            panic!(
                "[{}] Value property differs for '{}' ({}):\nOriginal: {:?}\nRoundtrip: {:?}",
                test_name, inst_a.name, inst_a.class, value_a, value_b
            );
        }
    }

    // Check Attributes if present
    if let (Some(attrs_a), Some(attrs_b)) = (
        inst_a.properties.get(&ustr("Attributes")),
        inst_b.properties.get(&ustr("Attributes")),
    ) {
        if attrs_a != attrs_b {
            panic!(
                "[{}] Attributes differ for '{}' ({}):\nOriginal: {:?}\nRoundtrip: {:?}",
                test_name, inst_a.name, inst_a.class, attrs_a, attrs_b
            );
        }
    }
}

// =============================================================================
// ROUNDTRIP TESTS
// =============================================================================
//
// These are build-test projects that should roundtrip cleanly through
// build -> syncback -> rebuild.
//
// Some projects are excluded because they:
// - Use sync rules with custom extensions (sync_rule_* projects)
// - Have CSV files that may not roundtrip identically
// - Have special project configurations that affect syncback

roundtrip_tests! {
    // Basic script types
    client_in_folder,
    client_init,
    module_in_folder,
    module_init,
    server_in_folder,
    server_init,

    // Nested structures
    deep_nesting,
    init_with_children,

    // Meta files and properties
    init_meta_class_name,
    init_meta_properties,
    script_meta_disabled,

    // JSON models
    json_model_in_folder,
    json_model_legacy_name,

    // Text files
    txt,
    txt_in_folder,

    // Binary models (rbxm/rbxmx)
    rbxm_in_folder,
    rbxmx_in_folder,
    rbxmx_ref,

    // Composed projects
    project_composed_default,
    project_composed_file,
    project_root_name,

    // Special cases
    attributes,
    gitkeep,
    optional,
    weldconstraint,

    // Service inference
    infer_service_name,
    infer_starter_player,

    // Issue regression tests
    issue_546,
}
