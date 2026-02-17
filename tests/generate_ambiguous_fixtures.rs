///! One-time fixture generator for ambiguous container tests.
///! Run with: cargo test --test generate_ambiguous_fixtures -- --ignored
///!
///! This creates all the .rbxm input files and project directories needed
///! for the ambiguous container test suite.
///!
///! For .rbxm input, `process_model_dom` in syncback.rs takes the single
///! root instance from the rbxm as the project tree root. Its children
///! become the content of `src/`. Duplicate names CANNOT be at the root
///! level (project.rs rejects them) — they must be deeper in the tree.
mod rojo_test;

use rbx_dom_weak::{
    types::{Tags, Variant},
    InstanceBuilder, WeakDom,
};
use std::path::Path;

use rojo_test::fixture_gen::*;

fn syncback_base() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rojo-test/syncback-tests"
    ))
}

fn build_base() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rojo-test/build-tests"
    ))
}

fn serve_base() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/rojo-test/serve-tests"
    ))
}

/// Create a syncback fixture: project dir, src/, project file, and input.rbxm.
/// `root_builder` should be the SINGLE root instance (becomes project tree root).
/// Its children become the contents of src/.
fn create_syncback_fixture(name: &str, root_builder: InstanceBuilder) {
    let base = syncback_base().join(name);
    let project_dir = base.join("input-project");
    let src_dir = project_dir.join("src");

    ensure_dir(&src_dir);

    let project = standard_project_json5(name);
    std::fs::write(project_dir.join("default.project.json5"), project)
        .expect("Failed to write project file");

    std::fs::write(src_dir.join(".gitkeep"), "").expect("Failed to write .gitkeep");

    // Wrap in a WeakDom and serialize the single root child
    let dom = WeakDom::new(root_builder);
    write_rbxm(&base.join("input.rbxm"), &dom);
}

/// Create a syncback incremental fixture with existing filesystem state.
fn create_syncback_incremental_fixture(
    name: &str,
    root_builder: InstanceBuilder,
    setup_project: impl FnOnce(&Path),
) {
    let base = syncback_base().join(name);
    let project_dir = base.join("input-project");
    let src_dir = project_dir.join("src");

    ensure_dir(&src_dir);

    let project = standard_project_json5(name);
    std::fs::write(project_dir.join("default.project.json5"), project)
        .expect("Failed to write project file");

    setup_project(&project_dir);

    let dom = WeakDom::new(root_builder);
    write_rbxm(&base.join("input.rbxm"), &dom);
}

/// Helper to build an rbxm blob from a single root builder (for use as
/// "old state" in incremental fixtures).
fn build_rbxm_bytes(root_builder: InstanceBuilder) -> Vec<u8> {
    let dom = WeakDom::new(root_builder);
    let root = dom.root_ref();
    let mut buf = Vec::new();
    rbx_binary::to_writer(&mut buf, &dom, &[root]).expect("Failed to serialize rbxm");
    buf
}

// ============================================================
// SYNCBACK CLEAN-MODE FIXTURES
// ============================================================

#[test]
#[ignore]
fn gen_ambiguous_basic() {
    // Root has child "Parent" which has two "Child" duplicates.
    // Parent should become an rbxm container.
    create_syncback_fixture(
        "ambiguous_basic",
        folder("Root").with_child(
            folder("Parent")
                .with_child(folder("Child"))
                .with_child(folder("Child")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_deepest_level() {
    // Root/Outer/Inner where only Inner has duplicate children.
    // Inner becomes rbxm; Outer stays a directory.
    create_syncback_fixture(
        "ambiguous_deepest_level",
        folder("Root").with_child(
            folder("Outer").with_child(
                folder("Inner")
                    .with_child(folder("Dup"))
                    .with_child(folder("Dup")),
            ),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_multiple_groups() {
    // Parent has Alpha(x2), Beta(x2), and Unique.
    // All captured in one rbxm.
    create_syncback_fixture(
        "ambiguous_multiple_groups",
        folder("Root").with_child(
            folder("Parent")
                .with_child(folder("Alpha"))
                .with_child(folder("Alpha"))
                .with_child(folder("Beta"))
                .with_child(folder("Beta"))
                .with_child(folder("Unique")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_with_unique_siblings() {
    // Parent has Dup(x2) + Solo. All three inside rbxm.
    create_syncback_fixture(
        "ambiguous_with_unique_siblings",
        folder("Root").with_child(
            folder("Parent")
                .with_child(folder("Dup"))
                .with_child(folder("Dup"))
                .with_child(module_script("Solo", "return 'solo'")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_script_container() {
    // ModuleScript with Source + two duplicate children.
    create_syncback_fixture(
        "ambiguous_script_container",
        folder("Root").with_child(
            module_script("MyModule", "return {}")
                .with_child(folder("Child"))
                .with_child(folder("Child")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_slugified_name() {
    // Container named "What?Folder" (forbidden char ?).
    create_syncback_fixture(
        "ambiguous_slugified_name",
        folder("Root").with_child(
            folder("What?Folder")
                .with_child(folder("Dup"))
                .with_child(folder("Dup")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_dedup_collision() {
    // Two siblings: "Test" (unique children) and "Te/st" (has duplicates,
    // slugifies to "Test"). The rbxm should get a ~1 dedup suffix.
    create_syncback_fixture(
        "ambiguous_dedup_collision",
        folder("Root")
            .with_child(folder("Test").with_child(folder("NormalChild")))
            .with_child(
                folder("Te/st")
                    .with_child(folder("Dup"))
                    .with_child(folder("Dup")),
            ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_deep_nesting() {
    // Container with 5 levels of nesting inside.
    create_syncback_fixture(
        "ambiguous_deep_nesting",
        folder("Root").with_child(
            folder("Deep")
                .with_child(folder("Dup"))
                .with_child(folder("Dup"))
                .with_child(folder("Level1").with_child(folder("Level2").with_child(
                    folder("Level3").with_child(folder("Level4").with_child(folder("Level5"))),
                ))),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_tags_and_attributes() {
    use rbx_dom_weak::types::Attributes as RbxAttributes;
    // Container with children that have Tags and Attributes properties.
    // Verifies these properties survive the rbxm round-trip.
    let mut attrs = RbxAttributes::new();
    attrs.insert("Health".to_string(), Variant::Float64(100.0));
    attrs.insert(
        "DisplayName".to_string(),
        Variant::String("Hero".to_string()),
    );
    attrs.insert("IsNPC".to_string(), Variant::Bool(true));

    let tagged_child = InstanceBuilder::new("Folder")
        .with_name("Child")
        .with_property(
            "Tags",
            Variant::Tags(["Collectible", "Rare"].iter().copied().collect::<Tags>()),
        )
        .with_property("Attributes", Variant::Attributes(attrs));

    let child_with_attrs_only = {
        let mut a2 = RbxAttributes::new();
        a2.insert("Priority".to_string(), Variant::Float64(5.0));
        InstanceBuilder::new("Folder")
            .with_name("Child")
            .with_property("Attributes", Variant::Attributes(a2))
    };

    create_syncback_fixture(
        "ambiguous_tags_and_attributes",
        folder("Root").with_child(
            folder("Parent")
                .with_child(tagged_child)
                .with_child(child_with_attrs_only),
        ),
    );
}

// ============================================================
// SYNCBACK INCREMENTAL FIXTURES (expansion tests)
// ============================================================

#[test]
#[ignore]
fn gen_ambiguous_expansion_resolved() {
    // Input: duplicates resolved (one renamed from "Child" to "ChildB").
    // Existing state: Parent is an rbxm container.
    create_syncback_incremental_fixture(
        "ambiguous_expansion_resolved",
        // New state: no more duplicates
        folder("Root").with_child(
            folder("Parent")
                .with_child(folder("Child"))
                .with_child(folder("ChildB")),
        ),
        |project_dir| {
            let src = project_dir.join("src");
            // Old state: Parent.rbxm + meta
            let old_rbxm = build_rbxm_bytes(
                folder("Parent")
                    .with_child(folder("Child"))
                    .with_child(folder("Child")),
            );
            std::fs::write(src.join("Parent.rbxm"), old_rbxm).unwrap();
            std::fs::write(
                src.join("Parent.meta.json5"),
                "{\n  ambiguousContainer: true,\n}\n",
            )
            .unwrap();
        },
    );
}

#[test]
#[ignore]
fn gen_ambiguous_expansion_still_ambiguous() {
    // Input: 3x "Child" → 1 renamed, but 2x "Child" remain.
    create_syncback_incremental_fixture(
        "ambiguous_expansion_still_ambiguous",
        folder("Root").with_child(
            folder("Parent")
                .with_child(folder("Child"))
                .with_child(folder("Child"))
                .with_child(folder("ChildC")),
        ),
        |project_dir| {
            let src = project_dir.join("src");
            let old_rbxm = build_rbxm_bytes(
                folder("Parent")
                    .with_child(folder("Child"))
                    .with_child(folder("Child"))
                    .with_child(folder("Child")),
            );
            std::fs::write(src.join("Parent.rbxm"), old_rbxm).unwrap();
            std::fs::write(
                src.join("Parent.meta.json5"),
                "{\n  ambiguousContainer: true,\n}\n",
            )
            .unwrap();
        },
    );
}

#[test]
#[ignore]
fn gen_ambiguous_user_rbxm_not_expanded() {
    // User rbxm (no ambiguousContainer flag) with unique children.
    create_syncback_incremental_fixture(
        "ambiguous_user_rbxm_not_expanded",
        // New state: same unique children
        folder("Root").with_child(
            model("UserModel")
                .with_child(part("Handle"))
                .with_child(part("Blade")),
        ),
        |project_dir| {
            let src = project_dir.join("src");
            // Old state: UserModel.rbxm WITHOUT ambiguousContainer flag
            let old_rbxm = build_rbxm_bytes(
                model("UserModel")
                    .with_child(part("Handle"))
                    .with_child(part("Blade")),
            );
            std::fs::write(src.join("UserModel.rbxm"), old_rbxm).unwrap();
            // NO meta file with ambiguousContainer
        },
    );
}

// ============================================================
// EDGE CASE FIXTURES
// ============================================================

#[test]
#[ignore]
fn gen_ambiguous_case_insensitive() {
    // Two children "child" and "Child" (case only).
    create_syncback_fixture(
        "ambiguous_case_insensitive",
        folder("Root").with_child(
            folder("CaseTest")
                .with_child(folder("child"))
                .with_child(folder("Child")),
        ),
    );
}

#[test]
#[ignore]
fn gen_ambiguous_windows_invalid_chars() {
    // Container with many forbidden Windows chars.
    create_syncback_fixture(
        "ambiguous_windows_invalid_chars",
        folder("Root").with_child(
            folder("What<>:\"|?*Name")
                .with_child(folder("Dup"))
                .with_child(folder("Dup")),
        ),
    );
}

// ============================================================
// BUILD-TEST FIXTURES
// ============================================================

#[test]
#[ignore]
fn gen_build_ambiguous_container() {
    let base = build_base().join("ambiguous_container");
    let src = base.join("src");
    ensure_dir(&src);

    std::fs::write(
        base.join("default.project.json5"),
        standard_project_json5("ambiguous_container"),
    )
    .unwrap();

    // Container rbxm with duplicate-named children
    let container_rbxm = build_rbxm_bytes(
        folder("Container")
            .with_child(folder("Dup"))
            .with_child(folder("Dup"))
            .with_child(module_script("Script", "return 'inside container'")),
    );
    std::fs::write(src.join("Container.rbxm"), container_rbxm).unwrap();
    std::fs::write(
        src.join("Container.meta.json5"),
        "{\n  ambiguousContainer: true,\n}\n",
    )
    .unwrap();

    // Normal sibling
    std::fs::write(src.join("Normal.luau"), "return 'normal sibling'").unwrap();
}

#[test]
#[ignore]
fn gen_build_ambiguous_tags_and_attributes() {
    use rbx_dom_weak::types::Attributes as RbxAttributes;

    let base = build_base().join("ambiguous_tags_and_attributes");
    let src = base.join("src");
    ensure_dir(&src);

    std::fs::write(
        base.join("default.project.json5"),
        standard_project_json5("ambiguous_tags_and_attributes"),
    )
    .unwrap();

    let mut attrs = RbxAttributes::new();
    attrs.insert("Health".to_string(), Variant::Float64(100.0));
    attrs.insert(
        "DisplayName".to_string(),
        Variant::String("Hero".to_string()),
    );
    attrs.insert("IsNPC".to_string(), Variant::Bool(true));

    let tagged_child = InstanceBuilder::new("Folder")
        .with_name("Child")
        .with_property(
            "Tags",
            Variant::Tags(["Collectible", "Rare"].iter().copied().collect::<Tags>()),
        )
        .with_property("Attributes", Variant::Attributes(attrs));

    let child_with_attrs_only = {
        let mut a2 = RbxAttributes::new();
        a2.insert("Priority".to_string(), Variant::Float64(5.0));
        InstanceBuilder::new("Folder")
            .with_name("Child")
            .with_property("Attributes", Variant::Attributes(a2))
    };

    let container_rbxm = build_rbxm_bytes(
        folder("Container")
            .with_child(tagged_child)
            .with_child(child_with_attrs_only),
    );
    std::fs::write(src.join("Container.rbxm"), container_rbxm).unwrap();
    std::fs::write(
        src.join("Container.meta.json5"),
        "{\n  ambiguousContainer: true,\n}\n",
    )
    .unwrap();
}

// ============================================================
// SERVE-TEST FIXTURE
// ============================================================

#[test]
#[ignore]
fn gen_serve_ambiguous_container() {
    let base = serve_base().join("ambiguous_container");
    let src = base.join("src");
    ensure_dir(&src);

    std::fs::write(
        base.join("default.project.json5"),
        r#"{
  "name": "ambiguous_container",
  "tree": {
    "$className": "DataModel",
    "ReplicatedStorage": {
      "$className": "ReplicatedStorage",
      "$path": "src"
    }
  }
}"#,
    )
    .unwrap();

    std::fs::write(src.join("ScriptA.luau"), "return 'A'").unwrap();
    std::fs::write(src.join("ScriptB.luau"), "return 'B'").unwrap();
}

// ============================================================
// PROJECT NODE PARENT TEST FIXTURE
// ============================================================

#[test]
#[ignore]
fn gen_ambiguous_project_node_parent() {
    // Root with duplicate-named direct children (these are direct children
    // of a ProjectNode, so they can't become rbxm containers).
    // Also includes a unique child to verify it syncs normally.
    create_syncback_fixture(
        "ambiguous_project_node_parent",
        folder("Root")
            .with_child(folder("Dup"))
            .with_child(folder("Dup"))
            .with_child(module_script("UniqueScript", "return 'unique'")),
    );
}
