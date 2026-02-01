use std::ffi::OsStr;

use insta::assert_snapshot;

use crate::rojo_test::syncback_util::{
    run_syncback_test, run_syncback_test_incremental, snapshot_rbxm,
};

macro_rules! syncback_tests {
    ($($test_name:ident => $list:expr$(,)?),*) => {$(
        #[test]
        fn $test_name() {
            run_syncback_test(stringify!($test_name), |path| {
                for name in $list {
                    let snapshot_name = format!(concat!(stringify!($test_name), "-{}"), name);
                    let new = path.join::<&str>(name);
                    if !new.exists() {
                        panic!("the path stub '{}' does not exist after syncback runs. consider double checking for typos.", name);
                    }
                    if let Some("rbxm") = new.extension().and_then(OsStr::to_str) {
                        let content = fs_err::read(new).unwrap();
                        snapshot_rbxm(&snapshot_name, content, name);
                    } else {
                        let content = fs_err::read_to_string(new).unwrap();
                        assert_snapshot!(snapshot_name, content, name);
                    }
                }
            });
        }
    )*};
}

/// Tests that need to run in incremental mode (preserving existing structure)
macro_rules! syncback_tests_incremental {
    ($($test_name:ident => $list:expr$(,)?),*) => {$(
        #[test]
        fn $test_name() {
            run_syncback_test_incremental(stringify!($test_name), |path| {
                for name in $list {
                    let snapshot_name = format!(concat!(stringify!($test_name), "-{}"), name);
                    let new = path.join::<&str>(name);
                    if !new.exists() {
                        panic!("the path stub '{}' does not exist after syncback runs. consider double checking for typos.", name);
                    }
                    if let Some("rbxm") = new.extension().and_then(OsStr::to_str) {
                        let content = fs_err::read(new).unwrap();
                        snapshot_rbxm(&snapshot_name, content, name);
                    } else {
                        let content = fs_err::read_to_string(new).unwrap();
                        assert_snapshot!(snapshot_name, content, name);
                    }
                }
            });
        }
    )*};
}

// Tests that run in clean mode (default)
syncback_tests! {
    // Ensures that there's only one copy written to disk if navigating a
    // project file might yield two copies
    child_but_not => ["OnlyOneCopy/child_of_one.luau", "ReplicatedStorage/child_replicated_storage.luau"],
    // Ensures that if a RojoId is duplicated somewhere in the project, it's
    // rewritten rather than synced back as a conflict
    duplicate_rojo_id => ["container.model.json5"],
    // Ensures that the `ignorePaths` setting works for additions (new files get .json5)
    ignore_paths_adding => ["src/int_value.model.json5", "src/subfolder/string_value.txt"],
    // Ensures that the `ignorePaths` setting works for removals (new file gets .json5)
    ignore_paths_removing => ["src/Message.model.json5"],
    // Ensures that `ignoreTrees` works for additions
    ignore_trees_adding => [],
    // Ensures that `ignoreTrees` works for removals
    ignore_trees_removing => [],
    // Ensures projects that refer to other projects work as expected
    nested_projects => ["nested.project.json5", "string_value.txt"],
    // Ensures files that are ignored by nested projects are picked up if
    // they're included in second project. Unusual but perfectly workable
    // pattern that syncback has to support.
    nested_projects_weird => ["src/modules/ClientModule.luau", "src/modules/ServerModule.luau"],
    // Ensures that projects respect `init` files when they're directly referenced from a node
    project_init => ["src/init.luau"],
    // Ensures that projects can be reserialized by syncback and that
    // default.project.json5 doesn't change unexpectedly
    project_reserialize => ["attribute_mismatch.luau", "property_mismatch.project.json5"],
    // Confirms that Instances that cannot serialize as directories serialize as rbxms
    rbxm_fallback => [],
    // Ensures that ref properties are linked when no attributes are manually
    // set in the DataModel (new files get .json5)
    ref_properties_blank => ["src/pointer.model.json5", "src/target.txt"],
    // Ensures that having multiple pointers that are aimed at the same target doesn't trigger ref rewrites.
    ref_properties_duplicate => [],
    // Ensures that ref properties that point to nothing after the prune both
    // do not leave any trace of themselves (new files get .json5)
    ref_properties_pruned => ["src/Pointer1.model.json5", "src/Pointer2.model.json5", "src/Pointer3.model.json5"],
    // Ensures that the `$schema` field roundtrips with syncback
    schema_roundtrip => ["default.project.json5", "src/model.model.json5", "src/init/init.meta.json5", "src/adjacent.meta.json5"],
    // Ensures that StringValues inside project files are written to the
    // project file, but only if they don't have `$path` set
    string_value_project => ["default.project.json5"],
    // Ensures that the `syncUnscriptable` setting works
    unscriptable_properties => ["default.project.json5"],
}

// Tests that run in incremental mode (preserving existing structure)
syncback_tests_incremental! {
    // Ensures that syncback works with CSVs (preserves existing CSV structure)
    csv => ["src/csv_init/init.csv", "src/csv.csv"],
    // Ensures that the `ignorePaths` setting works for `init` files
    ignore_paths_init => ["src/non-init.luau", "src/init-file/init.luau"],
    // Ensures that all of the JSON middlewares are handled as expected
    json_middlewares => ["src/dir_with_meta/init.meta.json5", "src/model_json.model.json5", "src/project_json.project.json5"],
    // Ensures that ref properties are linked properly on the file system
    ref_properties => ["src/pointer.model.json5", "src/target.model.json5"],
    // Ensures that if there is a conflict in RojoRefs, one of them is rewritten
    ref_properties_conflict => ["src/Pointer_2.model.json5", "src/Target_2.model.json5"],
    // Ensures that the old middleware is respected during syncback (incremental mode only)
    respect_old_middleware => ["default.project.json5", "src/model_json.model.json5", "src/rbxm.rbxm", "src/rbxmx.rbxmx"],
    // Ensures that sync rules are respected (incremental mode only - uses old paths when possible)
    sync_rules => ["src/module.modulescript", "src/text.text"],
}
