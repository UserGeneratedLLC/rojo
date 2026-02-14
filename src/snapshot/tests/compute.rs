use std::borrow::Cow;

use insta::assert_yaml_snapshot;

use rbx_dom_weak::{types::Ref, ustr, UstrMap};
use rojo_insta_ext::RedactionMap;

use crate::snapshot::{compute_patch_set, InstanceSnapshot, RojoTree};

#[test]
fn set_name_and_class_name() {
    let mut redactions = RedactionMap::default();

    let tree = empty_tree();
    redactions.intern(tree.get_root_id());

    let snapshot = InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: Default::default(),
        name: Cow::Borrowed("Some Folder"),
        class_name: ustr("Folder"),
        properties: Default::default(),
        children: Vec::new(),
    };

    let patch_set = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    let patch_value = redactions.redacted_yaml(patch_set);

    assert_yaml_snapshot!(patch_value);
}

#[test]
fn set_property() {
    let mut redactions = RedactionMap::default();

    let tree = empty_tree();
    redactions.intern(tree.get_root_id());

    let snapshot = InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: Default::default(),
        name: Cow::Borrowed("ROOT"),
        class_name: ustr("ROOT"),
        properties: UstrMap::from_iter([(ustr("PropertyName"), "Hello, world!".into())]),
        children: Vec::new(),
    };

    let patch_set = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    let patch_value = redactions.redacted_yaml(patch_set);

    assert_yaml_snapshot!(patch_value);
}

#[test]
fn remove_property() {
    let mut redactions = RedactionMap::default();

    let mut tree = empty_tree();
    redactions.intern(tree.get_root_id());

    {
        let root_id = tree.get_root_id();
        let mut root_instance = tree.get_instance_mut(root_id).unwrap();
        root_instance
            .properties_mut()
            .insert(ustr("Foo"), "This should be removed by the patch.".into());
    }

    let snapshot = InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: Default::default(),
        name: Cow::Borrowed("ROOT"),
        class_name: ustr("ROOT"),
        properties: Default::default(),
        children: Vec::new(),
    };

    let patch_set = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    let patch_value = redactions.redacted_yaml(patch_set);

    assert_yaml_snapshot!(patch_value);
}

#[test]
fn add_child() {
    let mut redactions = RedactionMap::default();

    let tree = empty_tree();
    redactions.intern(tree.get_root_id());

    let snapshot = InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: Default::default(),
        name: Cow::Borrowed("ROOT"),
        class_name: ustr("ROOT"),
        properties: Default::default(),
        children: vec![InstanceSnapshot {
            snapshot_id: Ref::none(),
            metadata: Default::default(),
            name: Cow::Borrowed("New"),
            class_name: ustr("Folder"),
            properties: Default::default(),
            children: Vec::new(),
        }],
    };

    let patch_set = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    let patch_value = redactions.redacted_yaml(patch_set);

    assert_yaml_snapshot!(patch_value);
}

#[test]
fn remove_child() {
    let mut redactions = RedactionMap::default();

    let mut tree = empty_tree();
    redactions.intern(tree.get_root_id());

    {
        let root_id = tree.get_root_id();
        let new_id = tree.insert_instance(
            root_id,
            InstanceSnapshot::new().name("Should not appear in snapshot"),
        );

        redactions.intern(new_id);
    }

    let snapshot = InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: Default::default(),
        name: Cow::Borrowed("ROOT"),
        class_name: ustr("ROOT"),
        properties: Default::default(),
        children: Vec::new(),
    };

    let patch_set = compute_patch_set(Some(snapshot), &tree, tree.get_root_id());
    let patch_value = redactions.redacted_yaml(patch_set);

    assert_yaml_snapshot!(patch_value);
}

fn empty_tree() -> RojoTree {
    RojoTree::new(InstanceSnapshot::new().name("ROOT").class_name("ROOT"))
}

// ---------------------------------------------------------------------------
// Rojo_Ref_* attribute resolution tests
// ---------------------------------------------------------------------------

use rbx_dom_weak::types::{Attributes, Variant};

/// Helper: build a tree with ROOT > Workspace > Part (to support path resolution).
fn tree_with_workspace_part() -> RojoTree {
    let snapshot = InstanceSnapshot::new()
        .name("ROOT")
        .class_name("DataModel")
        .children(vec![InstanceSnapshot::new()
            .name("Workspace")
            .class_name("Workspace")
            .children(vec![InstanceSnapshot::new()
                .name("Part")
                .class_name("Part")])]);
    RojoTree::new(snapshot)
}

/// Helper: build a tree with ROOT > Workspace > Model > Target + OtherPart.
fn tree_with_model_and_parts() -> RojoTree {
    let snapshot = InstanceSnapshot::new()
        .name("ROOT")
        .class_name("DataModel")
        .children(vec![InstanceSnapshot::new()
            .name("Workspace")
            .class_name("Workspace")
            .children(vec![InstanceSnapshot::new()
                .name("Model")
                .class_name("Model")
                .children(vec![
                    InstanceSnapshot::new().name("Target").class_name("Part"),
                    InstanceSnapshot::new().name("OtherPart").class_name("Part"),
                ])])]);
    RojoTree::new(snapshot)
}

#[test]
fn ref_attr_resolves_primary_part() {
    let tree = tree_with_model_and_parts();

    // Find the Model instance in the tree
    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();
    let model_id = tree
        .get_instance(workspace_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Create a snapshot of Model with Rojo_Ref_PrimaryPart attribute
    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_PrimaryPart".into(),
        Variant::String("Workspace/Model/Target".into()),
    );

    let snapshot = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]))
        .children(vec![
            InstanceSnapshot::new().name("Target").class_name("Part"),
            InstanceSnapshot::new().name("OtherPart").class_name("Part"),
        ]);

    let patch_set = compute_patch_set(Some(snapshot), &tree, model_id);

    // The patch should contain an update with PrimaryPart as a Variant::Ref
    let has_primary_part_ref = patch_set.updated_instances.iter().any(|u| {
        u.changed_properties
            .get(&ustr("PrimaryPart"))
            .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if r.is_some()))
    });
    assert!(
        has_primary_part_ref,
        "Rojo_Ref_PrimaryPart should resolve to a Variant::Ref in the patch"
    );
}

#[test]
fn ref_attr_resolves_value() {
    let tree = tree_with_workspace_part();

    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Add an ObjectValue child to Workspace in the tree
    let mut tree = tree;
    let objval_id = tree.insert_instance(
        workspace_id,
        InstanceSnapshot::new()
            .name("MyRef")
            .class_name("ObjectValue"),
    );

    // Create snapshot with Rojo_Ref_Value pointing to Workspace/Part
    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_Value".into(),
        Variant::String("Workspace/Part".into()),
    );

    let snapshot = InstanceSnapshot::new()
        .name("MyRef")
        .class_name("ObjectValue")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]));

    let patch_set = compute_patch_set(Some(snapshot), &tree, objval_id);

    let has_value_ref = patch_set.updated_instances.iter().any(|u| {
        u.changed_properties
            .get(&ustr("Value"))
            .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if r.is_some()))
    });
    assert!(
        has_value_ref,
        "Rojo_Ref_Value should resolve to a Variant::Ref in the patch"
    );
}

#[test]
fn ref_attr_multiple_on_same_instance() {
    let tree = tree_with_model_and_parts();

    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();
    let model_id = tree
        .get_instance(workspace_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Multiple Rojo_Ref_* attributes
    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_PrimaryPart".into(),
        Variant::String("Workspace/Model/Target".into()),
    );
    attrs.insert(
        "Rojo_Ref_CustomRef".into(),
        Variant::String("Workspace/Model/OtherPart".into()),
    );

    let snapshot = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]))
        .children(vec![
            InstanceSnapshot::new().name("Target").class_name("Part"),
            InstanceSnapshot::new().name("OtherPart").class_name("Part"),
        ]);

    let patch_set = compute_patch_set(Some(snapshot), &tree, model_id);

    let update = patch_set
        .updated_instances
        .iter()
        .find(|u| u.id == model_id);
    assert!(update.is_some(), "Should have an update for the Model");

    let update = update.unwrap();
    let has_pp = update
        .changed_properties
        .get(&ustr("PrimaryPart"))
        .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if r.is_some()));
    let has_custom = update
        .changed_properties
        .get(&ustr("CustomRef"))
        .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if r.is_some()));

    assert!(has_pp, "PrimaryPart ref should resolve");
    assert!(has_custom, "CustomRef ref should resolve");
}

#[test]
fn ref_attr_nonexistent_path_does_not_produce_valid_ref() {
    let tree = tree_with_workspace_part();
    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    let mut tree = tree;
    let model_id = tree.insert_instance(
        workspace_id,
        InstanceSnapshot::new().name("Model").class_name("Model"),
    );

    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_PrimaryPart".into(),
        Variant::String("Workspace/DoesNotExist".into()),
    );

    let snapshot = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]));

    let patch_set = compute_patch_set(Some(snapshot), &tree, model_id);

    // A non-existent path should NOT produce a valid (non-nil) Ref.
    // compute_ref_properties returns None for unresolvable paths, which
    // means "property value not set" -- not "set to nil Ref".
    let has_valid_ref = patch_set.updated_instances.iter().any(|u| {
        u.changed_properties
            .get(&ustr("PrimaryPart"))
            .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if r.is_some()))
    });
    assert!(
        !has_valid_ref,
        "Non-existent path should not produce a valid (non-nil) Ref"
    );
}

#[test]
fn ref_attr_with_regular_attributes_mixed() {
    let tree = tree_with_model_and_parts();
    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();
    let model_id = tree
        .get_instance(workspace_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Mix of Rojo_Ref_* and regular attributes
    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_PrimaryPart".into(),
        Variant::String("Workspace/Model/Target".into()),
    );
    attrs.insert("CustomAttribute".into(), Variant::String("hello".into()));
    attrs.insert("NumberAttr".into(), Variant::Float64(42.0));

    let snapshot = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]))
        .children(vec![
            InstanceSnapshot::new().name("Target").class_name("Part"),
            InstanceSnapshot::new().name("OtherPart").class_name("Part"),
        ]);

    let patch_set = compute_patch_set(Some(snapshot), &tree, model_id);

    // Should have PrimaryPart as Ref AND the regular attributes should
    // remain in the Attributes property (not be extracted as Ref properties).
    let has_pp = patch_set.updated_instances.iter().any(|u| {
        u.changed_properties
            .get(&ustr("PrimaryPart"))
            .is_some_and(|v| matches!(v, Some(Variant::Ref(_))))
    });
    assert!(has_pp, "PrimaryPart should be resolved from Rojo_Ref_*");
}

#[test]
fn ref_attr_empty_path_resolves_to_root() {
    let tree = tree_with_workspace_part();
    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    let mut tree = tree;
    let model_id = tree.insert_instance(
        workspace_id,
        InstanceSnapshot::new().name("Model").class_name("Model"),
    );

    let mut attrs = Attributes::new();
    attrs.insert("Rojo_Ref_PrimaryPart".into(), Variant::String("".into()));

    let snapshot = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]));

    let patch_set = compute_patch_set(Some(snapshot), &tree, model_id);

    // Empty path should resolve to root (RojoTree.get_instance_by_path("") returns root)
    let has_ref = patch_set.updated_instances.iter().any(|u| {
        u.changed_properties
            .get(&ustr("PrimaryPart"))
            .is_some_and(|v| matches!(v, Some(Variant::Ref(r)) if *r == root_id))
    });
    assert!(has_ref, "Empty path should resolve to root instance");
}

/// When both Rojo_Ref_PrimaryPart and Rojo_Target_PrimaryPart exist for the
/// same property, Rojo_Ref_* (path-based) must win because it is the preferred
/// system. BTreeMap iterates alphabetically ('R' < 'T'), so Rojo_Ref_* is
/// visited first. The Rojo_Target_* branch must skip insertion if Rojo_Ref_*
/// already set the property.
#[test]
fn ref_attr_priority_path_wins_over_target() {
    // Build tree: ROOT > Workspace > Model > Target, OtherPart
    let snapshot = InstanceSnapshot::new()
        .name("ROOT")
        .class_name("DataModel")
        .children(vec![InstanceSnapshot::new()
            .name("Workspace")
            .class_name("Workspace")
            .children(vec![InstanceSnapshot::new()
                .name("Model")
                .class_name("Model")
                .children(vec![
                    InstanceSnapshot::new().name("Target").class_name("Part"),
                    InstanceSnapshot::new().name("OtherPart").class_name("Part"),
                ])])]);
    let mut tree = RojoTree::new(snapshot);

    let root_id = tree.get_root_id();
    let workspace_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();
    let model_id = tree
        .get_instance(workspace_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();
    let model_children: Vec<_> = tree.get_instance(model_id).unwrap().children().to_vec();
    let target_id = model_children[0]; // Target
    let other_id = model_children[1]; // OtherPart

    // Give OtherPart a specified ID so Rojo_Target_* can resolve it
    let other_rojo_id = crate::RojoRef::new("other-rojo-id".to_string());
    tree.set_specified_id(other_id, other_rojo_id);

    // Create snapshot with BOTH attributes for the same property.
    // Rojo_Ref_PrimaryPart points to Target (via path).
    // Rojo_Target_PrimaryPart points to OtherPart (via ID).
    // Rojo_Ref_* should win.
    let mut attrs = Attributes::new();
    attrs.insert(
        "Rojo_Ref_PrimaryPart".into(),
        Variant::String("Workspace/Model/Target".into()),
    );
    attrs.insert(
        "Rojo_Target_PrimaryPart".into(),
        Variant::String("other-rojo-id".into()),
    );

    let snap = InstanceSnapshot::new()
        .name("Model")
        .class_name("Model")
        .properties(UstrMap::from_iter([(
            ustr("Attributes"),
            Variant::Attributes(attrs),
        )]))
        .children(vec![
            InstanceSnapshot::new().name("Target").class_name("Part"),
            InstanceSnapshot::new().name("OtherPart").class_name("Part"),
        ]);

    let patch_set = compute_patch_set(Some(snap), &tree, model_id);

    // PrimaryPart should point to Target (from Rojo_Ref_*), NOT OtherPart (from Rojo_Target_*)
    let pp_ref = patch_set
        .updated_instances
        .iter()
        .find_map(|u| u.changed_properties.get(&ustr("PrimaryPart")))
        .expect("PrimaryPart should be in the patch");

    match pp_ref {
        Some(Variant::Ref(r)) => {
            assert_eq!(
                *r, target_id,
                "Rojo_Ref_* should win over Rojo_Target_* -- PrimaryPart should point to Target, not OtherPart"
            );
        }
        other => panic!("PrimaryPart should be Some(Variant::Ref), got {:?}", other),
    }
}
