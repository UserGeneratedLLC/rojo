use std::{fs, path::Path, process::Command};

use insta::assert_snapshot;
use tempfile::tempdir;

use crate::rojo_test::io_util::{get_working_dir_path, BUILD_TESTS_PATH, ROJO_PATH};

macro_rules! gen_build_tests {
    ( $($test_name: ident,)* ) => {
        $(
            paste::item! {
                #[test]
                fn [<build_ $test_name>]() {
                    let _ = env_logger::try_init();

                    run_build_test(stringify!($test_name));
                }
            }
        )*
    };
}

gen_build_tests! {
    init_csv_with_children,
    attributes,
    client_in_folder,
    client_init,
    csv_bug_145,
    csv_bug_147,
    csv_in_folder,
    deep_nesting,
    gitkeep,
    ignore_glob_inner,
    ignore_glob_nested,
    ignore_glob_spec,
    infer_service_name,
    infer_starter_player,
    init_meta_class_name,
    init_meta_properties,
    init_with_children,
    issue_546,
    json_as_lua,
    json_model_in_folder,
    json_model_legacy_name,
    module_in_folder,
    module_init,
    optional,
    project_composed_default,
    project_composed_file,
    project_root_name,
    rbxm_in_folder,
    rbxmx_in_folder,
    rbxmx_ref,
    script_meta_disabled,
    server_in_folder,
    server_init,
    txt,
    txt_in_folder,
    unresolved_values,
    weldconstraint,
    sync_rule_alone,
    sync_rule_complex,
    sync_rule_nested_projects,
    no_name_default_project,
    no_name_project,
    no_name_top_level_project,
    tilde_no_meta,
    meta_name_override,
    model_json_name_override,
    dedup_suffix_with_meta,
    dedup_suffix_auto_strip,
    init_meta_name_override,
    reserved_name_override,
}

fn run_build_test(test_name: &str) {
    let working_dir = get_working_dir_path();

    let input_path = Path::new(BUILD_TESTS_PATH).join(test_name);

    let output_dir = tempdir().expect("couldn't create temporary directory");
    let output_path = output_dir.path().join(format!("{}.rbxmx", test_name));

    let output = Command::new(ROJO_PATH)
        .args([
            "build",
            input_path.to_str().unwrap(),
            "-o",
            output_path.to_str().unwrap(),
        ])
        .env("RUST_LOG", "error")
        .current_dir(working_dir)
        .output()
        .expect("Couldn't start Rojo");

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    assert!(output.status.success(), "Rojo did not exit successfully");

    let contents = fs::read_to_string(&output_path).expect("Couldn't read output file");

    let mut settings = insta::Settings::new();

    let snapshot_path = Path::new(BUILD_TESTS_PATH)
        .parent()
        .unwrap()
        .join("build-test-snapshots");

    settings.set_snapshot_path(snapshot_path);

    settings.bind(|| {
        assert_snapshot!(test_name, contents);
    });
}

fn snapshot_debug(snap: &librojo::InstanceSnapshot) -> String {
    fn recurse(snap: &librojo::InstanceSnapshot, depth: usize, out: &mut String) {
        use std::fmt::Write;
        let indent = "  ".repeat(depth);
        writeln!(out, "{}{} [{}]", indent, snap.name, snap.class_name).unwrap();
        for child in &snap.children {
            recurse(child, depth + 1, out);
        }
    }
    let mut s = String::new();
    recurse(snap, 0, &mut s);
    s
}

#[test]
fn parallel_snapshot_determinism() {
    use crate::rojo_test::io_util::SERVE_TESTS_PATH;

    let _ = env_logger::try_init();

    let fixture_path = Path::new(SERVE_TESTS_PATH).join("connected_scripts");
    let vfs = memofs::Vfs::new_default();
    let ctx = librojo::InstanceContext::default();

    let first = librojo::snapshot_from_vfs(&ctx, &vfs, &fixture_path)
        .expect("snapshot failed")
        .expect("snapshot returned None");
    let baseline = snapshot_debug(&first);

    for i in 1..5 {
        let vfs = memofs::Vfs::new_default();
        let snap = librojo::snapshot_from_vfs(&ctx, &vfs, &fixture_path)
            .expect("snapshot failed")
            .expect("snapshot returned None");
        let current = snapshot_debug(&snap);
        assert_eq!(
            baseline, current,
            "Parallel snapshot produced different child order on iteration {i}"
        );
    }
}

#[test]
fn parallel_snapshot_determinism_stress() {
    let _ = env_logger::try_init();

    let dir = tempdir().expect("couldn't create temp dir");
    let root = dir.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    for i in 0..40 {
        fs::write(src.join(format!("mod_{i:03}.luau")), format!("return {i}")).unwrap();
    }
    for i in 0..10 {
        let sub = src.join(format!("pkg_{i:02}"));
        fs::create_dir(&sub).unwrap();
        for j in 0..5 {
            fs::write(sub.join(format!("sub_{j}.luau")), format!("return {j}")).unwrap();
        }
    }

    fs::write(
        root.join("default.project.json5"),
        r#"{ "name": "StressTest", "tree": { "$path": "src" } }"#,
    )
    .unwrap();

    let ctx = librojo::InstanceContext::default();
    let project_path = root.join("default.project.json5");

    let vfs = memofs::Vfs::new_default();
    let first = librojo::snapshot_from_vfs(&ctx, &vfs, &project_path)
        .expect("snapshot failed")
        .expect("snapshot returned None");
    let baseline = snapshot_debug(&first);

    for i in 1..50 {
        let vfs = memofs::Vfs::new_default();
        let snap = librojo::snapshot_from_vfs(&ctx, &vfs, &project_path)
            .expect("snapshot failed")
            .expect("snapshot returned None");
        let current = snapshot_debug(&snap);
        assert_eq!(
            baseline, current,
            "Stress: parallel snapshot diverged on iteration {i}"
        );
    }
}

#[test]
fn parallel_snapshot_with_prefetch_cache() {
    let _ = env_logger::try_init();

    let dir = tempdir().expect("couldn't create temp dir");
    let root = dir.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    for i in 0..20 {
        fs::write(src.join(format!("f{i}.luau")), format!("return {i}")).unwrap();
    }
    for i in 0..5 {
        let sub = src.join(format!("d{i}"));
        fs::create_dir(&sub).unwrap();
        for j in 0..4 {
            fs::write(sub.join(format!("g{j}.luau")), format!("return {j}")).unwrap();
        }
    }

    fs::write(
        root.join("default.project.json5"),
        r#"{ "name": "PrefetchTest", "tree": { "$path": "src" } }"#,
    )
    .unwrap();

    let ctx = librojo::InstanceContext::default();
    let project_path = root.join("default.project.json5");

    let vfs_no_cache = memofs::Vfs::new_default();
    let snap_no_cache = librojo::snapshot_from_vfs(&ctx, &vfs_no_cache, &project_path)
        .expect("no-cache snapshot failed")
        .expect("no-cache snapshot returned None");
    let baseline = snapshot_debug(&snap_no_cache);

    use std::collections::HashMap;
    let mut files = HashMap::new();
    let mut canonical = HashMap::new();
    for entry in walkdir::WalkDir::new(root).follow_links(true) {
        let entry = entry.unwrap();
        let path = entry.path().to_path_buf();
        if entry.file_type().is_file() {
            files.insert(path.clone(), fs::read(&path).unwrap());
        }
        if let Ok(c) = fs::canonicalize(&path) {
            canonical.insert(path, c);
        }
    }

    let vfs_cached = memofs::Vfs::new_default();
    vfs_cached.set_prefetch_cache(memofs::PrefetchCache {
        files,
        canonical,
        is_file: std::collections::HashMap::new(),
        children: std::collections::HashMap::new(),
    });

    let snap_cached = librojo::snapshot_from_vfs(&ctx, &vfs_cached, &project_path)
        .expect("cached snapshot failed")
        .expect("cached snapshot returned None");
    let cached_str = snapshot_debug(&snap_cached);

    assert_eq!(
        baseline, cached_str,
        "Prefetch-cached snapshot differs from non-cached snapshot"
    );
}

#[test]
fn parallel_snapshot_error_propagation() {
    let _ = env_logger::try_init();

    let dir = tempdir().expect("couldn't create temp dir");
    let root = dir.path();
    let src = root.join("src");
    fs::create_dir(&src).unwrap();

    for i in 0..5 {
        fs::write(src.join(format!("ok_{i}.luau")), format!("return {i}")).unwrap();
    }
    fs::write(src.join("bad.model.json"), "THIS IS NOT VALID JSON {{{").unwrap();

    fs::write(
        root.join("default.project.json5"),
        r#"{ "name": "ErrorTest", "tree": { "$path": "src" } }"#,
    )
    .unwrap();

    let ctx = librojo::InstanceContext::default();
    let project_path = root.join("default.project.json5");

    let vfs = memofs::Vfs::new_default();
    let result = librojo::snapshot_from_vfs(&ctx, &vfs, &project_path);
    assert!(
        result.is_err(),
        "Should propagate JSON parse error from parallel snapshot"
    );
}
