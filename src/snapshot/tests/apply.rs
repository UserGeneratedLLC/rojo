use insta::assert_yaml_snapshot;

use rbx_dom_weak::{ustr, UstrMap};
use rojo_insta_ext::RedactionMap;

use crate::{
    snapshot::{apply_patch_set, InstanceSnapshot, PatchSet, PatchUpdate, RojoTree},
    tree_view::{intern_tree, view_tree},
};

#[test]
fn set_name_and_class_name() {
    let mut redactions = RedactionMap::default();

    let mut tree = empty_tree();
    intern_tree(&tree, &mut redactions);

    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: tree.get_root_id(),
            changed_name: Some("Hello, world!".to_owned()),
            changed_class_name: Some(ustr("Folder")),
            changed_properties: Default::default(),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied_patch_set = apply_patch_set(&mut tree, patch_set);

    let tree_view = view_tree(&tree, &mut redactions);
    assert_yaml_snapshot!(tree_view);

    let applied_patch_value = redactions.redacted_yaml(applied_patch_set);
    assert_yaml_snapshot!(applied_patch_value);
}

#[test]
fn add_property() {
    let mut redactions = RedactionMap::default();

    let mut tree = empty_tree();
    intern_tree(&tree, &mut redactions);

    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: tree.get_root_id(),
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(ustr("Foo"), Some("Value of Foo".into()))]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied_patch_set = apply_patch_set(&mut tree, patch_set);

    let tree_view = view_tree(&tree, &mut redactions);
    assert_yaml_snapshot!(tree_view);

    let applied_patch_value = redactions.redacted_yaml(applied_patch_set);
    assert_yaml_snapshot!(applied_patch_value);
}

#[test]
fn remove_property() {
    let mut redactions = RedactionMap::default();

    let mut tree = empty_tree();
    intern_tree(&tree, &mut redactions);

    {
        let root_id = tree.get_root_id();
        let mut root_instance = tree.get_instance_mut(root_id).unwrap();

        root_instance
            .properties_mut()
            .insert(ustr("Foo"), "Should be removed".into());
    }

    let tree_view = view_tree(&tree, &mut redactions);
    assert_yaml_snapshot!("remove_property_initial", tree_view);

    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: tree.get_root_id(),
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(ustr("Foo"), None)]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied_patch_set = apply_patch_set(&mut tree, patch_set);

    let tree_view = view_tree(&tree, &mut redactions);
    assert_yaml_snapshot!("remove_property_after_patch", tree_view);

    let applied_patch_value = redactions.redacted_yaml(applied_patch_set);
    assert_yaml_snapshot!("remove_property_appied_patch", applied_patch_value);
}

fn empty_tree() -> RojoTree {
    RojoTree::new(InstanceSnapshot::new().name("ROOT").class_name("ROOT"))
}

// ---------------------------------------------------------------------------
// Ref property patch application tests
// ---------------------------------------------------------------------------

use rbx_dom_weak::types::{Ref, Variant};

#[test]
fn apply_ref_property_update() {
    let mut redactions = RedactionMap::default();

    // Build a tree: ROOT > ChildA, ChildB
    let mut tree = RojoTree::new(
        InstanceSnapshot::new()
            .name("ROOT")
            .class_name("DataModel")
            .children(vec![
                InstanceSnapshot::new().name("ChildA").class_name("Part"),
                InstanceSnapshot::new().name("ChildB").class_name("Model"),
            ]),
    );
    intern_tree(&tree, &mut redactions);

    let root_id = tree.get_root_id();
    let children: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();
    let child_a_id = children[0];
    let child_b_id = children[1];

    // Apply a patch that sets a Ref property on ChildB pointing to ChildA
    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: child_b_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(
                ustr("PrimaryPart"),
                Some(Variant::Ref(child_a_id)),
            )]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied = apply_patch_set(&mut tree, patch_set);
    assert_eq!(applied.updated.len(), 1);

    // Verify the Ref property was set
    let child_b = tree.get_instance(child_b_id).unwrap();
    let pp = child_b.properties().get(&ustr("PrimaryPart"));
    assert!(
        matches!(pp, Some(Variant::Ref(r)) if *r == child_a_id),
        "PrimaryPart should point to ChildA"
    );
}

#[test]
fn apply_nil_ref_skips_in_forward_sync() {
    // In the forward-sync path, Ref::none() is a sentinel for "unresolved ref"
    // and apply_update_child skips it (line 281-282 of patch_apply.rs).
    // This test verifies that behavior is preserved.
    let mut tree = RojoTree::new(
        InstanceSnapshot::new()
            .name("ROOT")
            .class_name("DataModel")
            .children(vec![InstanceSnapshot::new()
                .name("Model")
                .class_name("Model")]),
    );

    let root_id = tree.get_root_id();
    let model_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Set PrimaryPart to something
    {
        let mut model = tree.get_instance_mut(model_id).unwrap();
        model
            .properties_mut()
            .insert(ustr("PrimaryPart"), Variant::Ref(root_id));
    }

    // In forward-sync, Ref::none() means "unresolved" and is skipped.
    // The property should remain unchanged.
    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: model_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(
                ustr("PrimaryPart"),
                Some(Variant::Ref(Ref::none())),
            )]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied = apply_patch_set(&mut tree, patch_set);
    assert_eq!(applied.updated.len(), 1);

    let model = tree.get_instance(model_id).unwrap();
    let pp = model.properties().get(&ustr("PrimaryPart"));
    assert!(
        matches!(pp, Some(Variant::Ref(r)) if *r == root_id),
        "Forward-sync: PrimaryPart should remain unchanged when Ref::none() is applied"
    );
}

#[test]
fn apply_ref_property_removal() {
    // In the two-way sync path, nil Refs are converted to property removals
    // (None) in handle_api_write before reaching apply_patch_set. This test
    // verifies that None correctly removes the property.
    let mut tree = RojoTree::new(
        InstanceSnapshot::new()
            .name("ROOT")
            .class_name("DataModel")
            .children(vec![InstanceSnapshot::new()
                .name("Model")
                .class_name("Model")]),
    );

    let root_id = tree.get_root_id();
    let model_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Set PrimaryPart to something
    {
        let mut model = tree.get_instance_mut(model_id).unwrap();
        model
            .properties_mut()
            .insert(ustr("PrimaryPart"), Variant::Ref(root_id));
    }

    // In two-way sync, nil Refs become None (property removal) before
    // reaching the patch system. Verify that None actually removes it.
    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: model_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(ustr("PrimaryPart"), None)]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    let applied = apply_patch_set(&mut tree, patch_set);
    assert_eq!(applied.updated.len(), 1);

    let model = tree.get_instance(model_id).unwrap();
    assert!(
        model.properties().get(&ustr("PrimaryPart")).is_none(),
        "PrimaryPart should be removed after applying None (two-way sync nil Ref)"
    );
}

#[test]
fn apply_multiple_ref_property_updates() {
    let mut tree = RojoTree::new(
        InstanceSnapshot::new()
            .name("ROOT")
            .class_name("DataModel")
            .children(vec![
                InstanceSnapshot::new().name("PartA").class_name("Part"),
                InstanceSnapshot::new().name("PartB").class_name("Part"),
                InstanceSnapshot::new().name("Model").class_name("Model"),
            ]),
    );

    let root_id = tree.get_root_id();
    let children: Vec<Ref> = tree.get_instance(root_id).unwrap().children().to_vec();
    let part_a = children[0];
    let part_b = children[1];
    let model_id = children[2];

    // Apply multiple Ref property updates in the same patch
    let patch_set = PatchSet {
        updated_instances: vec![
            PatchUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: UstrMap::from_iter([(
                    ustr("PrimaryPart"),
                    Some(Variant::Ref(part_a)),
                )]),
                changed_metadata: None,
            },
            PatchUpdate {
                id: model_id,
                changed_name: None,
                changed_class_name: None,
                changed_properties: UstrMap::from_iter([(
                    ustr("CustomRef"),
                    Some(Variant::Ref(part_b)),
                )]),
                changed_metadata: None,
            },
        ],
        ..Default::default()
    };

    let applied = apply_patch_set(&mut tree, patch_set);
    assert_eq!(applied.updated.len(), 2);

    let model = tree.get_instance(model_id).unwrap();
    assert!(matches!(
        model.properties().get(&ustr("PrimaryPart")),
        Some(Variant::Ref(r)) if *r == part_a
    ));
    assert!(matches!(
        model.properties().get(&ustr("CustomRef")),
        Some(Variant::Ref(r)) if *r == part_b
    ));
}

#[test]
fn apply_ref_to_nonexistent_instance() {
    let mut tree = RojoTree::new(
        InstanceSnapshot::new()
            .name("ROOT")
            .class_name("DataModel")
            .children(vec![InstanceSnapshot::new()
                .name("Model")
                .class_name("Model")]),
    );

    let root_id = tree.get_root_id();
    let model_id = tree
        .get_instance(root_id)
        .unwrap()
        .children()
        .first()
        .copied()
        .unwrap();

    // Create a Ref to an ID that doesn't exist in the tree
    let fake_ref = Ref::new();

    let patch_set = PatchSet {
        updated_instances: vec![PatchUpdate {
            id: model_id,
            changed_name: None,
            changed_class_name: None,
            changed_properties: UstrMap::from_iter([(
                ustr("PrimaryPart"),
                Some(Variant::Ref(fake_ref)),
            )]),
            changed_metadata: None,
        }],
        ..Default::default()
    };

    // Should not crash -- the Ref is set on the tree instance even if the
    // target doesn't exist (dangling refs are allowed in WeakDom)
    let applied = apply_patch_set(&mut tree, patch_set);
    assert_eq!(applied.updated.len(), 1);
}
