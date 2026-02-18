//! Integration tests for the matching algorithm using real-world fixtures.
//!
//! Uses the UFOWave rbxm (full Studio properties) and its syncback'd
//! filesystem representation (defaults stripped) to verify that all
//! matching algorithms correctly pair instances despite asymmetric
//! property sets.
//!
//! The key scenario: 12 Textures under TsunamiWave, each with a unique
//! Face value. Two groups of 6 (by TextureContent). In each group, the
//! Face=Front texture has Face omitted from its model.json5 (it's the
//! class default). The matching must still pair it to the correct Studio
//! instance rather than stealing another Face's match.

use std::collections::HashMap;
use std::path::Path;

use rbx_dom_weak::types::{Ref, Variant};
use rbx_dom_weak::{ustr, WeakDom};

use crate::rojo_test::io_util::SYNCBACK_TESTS_PATH;

const FIXTURE_DIR: &str = "UFOWave_matching";

/// Parse the input.rbxm fixture into a WeakDom.
fn load_input_rbxm() -> WeakDom {
    let path = Path::new(SYNCBACK_TESTS_PATH)
        .join(FIXTURE_DIR)
        .join("input.rbxm");
    let data =
        std::fs::read(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
    rbx_binary::from_reader(data.as_slice())
        .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path.display(), e))
}

/// Build a WeakDom from the expected/ directory by building it through
/// the snapshot pipeline (the same path `atlas build` uses).
fn load_expected_dom() -> WeakDom {
    let expected_path = Path::new(SYNCBACK_TESTS_PATH)
        .join(FIXTURE_DIR)
        .join("expected");

    let vfs = memofs::Vfs::new_default();
    let ctx = librojo::InstanceContext::default();
    let snapshot = librojo::snapshot_from_vfs(&ctx, &vfs, &expected_path)
        .unwrap_or_else(|e| panic!("Failed to snapshot expected/: {}", e))
        .expect("snapshot_from_vfs returned None for expected/");

    // Convert InstanceSnapshot tree into a WeakDom for syncback matching
    let tree = librojo::RojoTree::new(snapshot);
    rojo_tree_to_weak_dom(&tree)
}

/// Convert a RojoTree into a WeakDom (for syncback matching which needs
/// two WeakDom instances).
fn rojo_tree_to_weak_dom(tree: &librojo::RojoTree) -> WeakDom {
    let root_id = tree.get_root_id();
    let root = tree.get_instance(root_id).unwrap();

    let mut dom = WeakDom::new(
        rbx_dom_weak::InstanceBuilder::new(root.class_name().as_str()).with_name(root.name()),
    );
    let dom_root = dom.root_ref();

    // Copy properties to root
    for (key, val) in root.properties() {
        dom.get_by_ref_mut(dom_root)
            .unwrap()
            .properties
            .insert(*key, val.clone());
    }

    // Recursively copy children
    fn copy_children(
        tree: &librojo::RojoTree,
        parent_ref: Ref,
        dom: &mut WeakDom,
        dom_parent: Ref,
    ) {
        let children: Vec<Ref> = tree.get_instance(parent_ref).unwrap().children().to_vec();
        for child_ref in children {
            let child = tree.get_instance(child_ref).unwrap();
            let builder = rbx_dom_weak::InstanceBuilder::new(child.class_name().as_str())
                .with_name(child.name());
            let new_ref = dom.insert(dom_parent, builder);
            for (key, val) in child.properties() {
                dom.get_by_ref_mut(new_ref)
                    .unwrap()
                    .properties
                    .insert(*key, val.clone());
            }
            copy_children(tree, child_ref, dom, new_ref);
        }
    }

    copy_children(tree, root_id, &mut dom, dom_root);
    dom
}

/// Recursively find a descendant by name in a WeakDom.
fn find_descendant(dom: &WeakDom, parent: Ref, name: &str) -> Option<Ref> {
    let parent_inst = dom.get_by_ref(parent)?;
    for &child in parent_inst.children() {
        let child_inst = dom.get_by_ref(child)?;
        if child_inst.name == name {
            return Some(child);
        }
        if let Some(found) = find_descendant(dom, child, name) {
            return Some(found);
        }
    }
    None
}

/// Recursively find a descendant by name in a RojoTree.
fn find_descendant_by_name(tree: &librojo::RojoTree, parent: Ref, name: &str) -> Option<Ref> {
    let parent_inst = tree.get_instance(parent)?;
    for &child in parent_inst.children() {
        let child_inst = tree.get_instance(child)?;
        if child_inst.name() == name {
            return Some(child);
        }
        if let Some(found) = find_descendant_by_name(tree, child, name) {
            return Some(found);
        }
    }
    None
}

/// Get all Texture children under a parent in a WeakDom.
fn get_texture_children(dom: &WeakDom, parent: Ref) -> Vec<Ref> {
    let parent_inst = dom.get_by_ref(parent).unwrap();
    parent_inst
        .children()
        .iter()
        .filter(|&&r| {
            dom.get_by_ref(r)
                .map_or(false, |i| i.class.as_str() == "Texture")
        })
        .copied()
        .collect()
}

/// Extract the Face enum value from an instance (returns None if not set).
fn get_face(dom: &WeakDom, inst_ref: Ref) -> Option<u32> {
    let inst = dom.get_by_ref(inst_ref)?;
    match inst.properties.get(&ustr("Face"))? {
        Variant::Enum(e) => Some(e.to_u32()),
        _ => None,
    }
}

/// Build a map of Face value → count for a set of Texture refs.
fn face_distribution(dom: &WeakDom, refs: &[Ref]) -> HashMap<Option<u32>, usize> {
    let mut map = HashMap::new();
    for &r in refs {
        *map.entry(get_face(dom, r)).or_insert(0) += 1;
    }
    map
}

// ================================================================
// Syncback matching (WeakDom vs WeakDom)
// ================================================================

#[test]
fn syncback_matching_ufowave_textures() {
    let _ = env_logger::try_init();

    let input_dom = load_input_rbxm();
    let expected_dom = load_expected_dom();

    // Find TsunamiWave in both trees
    let input_root = input_dom.root_ref();
    let input_tsunami = find_descendant(&input_dom, input_root, "TsunamiWave")
        .expect("TsunamiWave not found in input.rbxm");

    let expected_root = expected_dom.root_ref();
    let expected_tsunami = find_descendant(&expected_dom, expected_root, "TsunamiWave")
        .expect("TsunamiWave not found in expected/");

    let input_textures = get_texture_children(&input_dom, input_tsunami);
    let expected_textures = get_texture_children(&expected_dom, expected_tsunami);

    assert!(
        !input_textures.is_empty(),
        "No Texture children in input.rbxm TsunamiWave"
    );
    assert_eq!(
        input_textures.len(),
        expected_textures.len(),
        "Texture count mismatch: input has {}, expected has {}",
        input_textures.len(),
        expected_textures.len()
    );

    // Verify both sides have all 6 faces per group
    let input_faces = face_distribution(&input_dom, &input_textures);
    let expected_faces = face_distribution(&expected_dom, &expected_textures);
    eprintln!("Input face distribution: {:?}", input_faces);
    eprintln!("Expected face distribution: {:?}", expected_faces);

    // Run syncback matching
    let result = librojo::syncback::matching::match_children(
        &input_textures,
        &expected_textures,
        &input_dom,
        &expected_dom,
        None,
        None,
    );

    assert_eq!(
        result.matched.len(),
        input_textures.len(),
        "All Textures should be matched"
    );
    assert!(result.unmatched_new.is_empty(), "No unmatched input");
    assert!(result.unmatched_old.is_empty(), "No unmatched expected");

    // Verify correct Face pairing for every matched pair
    for (new_ref, old_ref) in &result.matched {
        let new_face = get_face(&input_dom, *new_ref);
        let old_face = get_face(&expected_dom, *old_ref);

        // If expected has no Face, it's the default (Front = 5)
        let effective_old_face = old_face.or(Some(5));

        assert_eq!(
            new_face,
            effective_old_face,
            "Face mismatch: input {:?} (name={}) matched to expected {:?} (name={})",
            new_face,
            input_dom.get_by_ref(*new_ref).unwrap().name,
            old_face,
            expected_dom.get_by_ref(*old_ref).unwrap().name,
        );
    }
}

// ================================================================
// Forward sync matching (InstanceSnapshot vs RojoTree)
// ================================================================

#[test]
fn forward_matching_ufowave_textures() {
    let _ = env_logger::try_init();

    // For forward sync: snapshots come from filesystem (sparse),
    // tree comes from Studio (full properties, simulated by rbxm).

    // Build snapshots from the expected/ directory
    let expected_path = Path::new(SYNCBACK_TESTS_PATH)
        .join(FIXTURE_DIR)
        .join("expected");
    let vfs = memofs::Vfs::new_default();
    let ctx = librojo::InstanceContext::default();
    let snapshot = librojo::snapshot_from_vfs(&ctx, &vfs, &expected_path)
        .expect("Failed to snapshot expected/")
        .expect("snapshot_from_vfs returned None");

    // Build the tree from the rbxm (simulating Studio-populated tree).
    // from_tree takes ownership, so we load a fresh copy.
    let input_dom = load_input_rbxm();
    let root_ref = input_dom.root_ref();
    let input_snapshot = librojo::InstanceSnapshot::from_tree(input_dom, root_ref);
    let tree = librojo::RojoTree::new(input_snapshot);
    let tree_root = tree.get_root_id();

    // Find TsunamiWave in the snapshot children
    let snap_tsunami = snapshot
        .children
        .iter()
        .find(|c| c.name == "TsunamiWave")
        .expect("TsunamiWave not found in snapshot");

    // Find TsunamiWave in the tree (may be nested under root → UFOWave → TsunamiWave)
    let tree_tsunami = find_descendant_by_name(&tree, tree_root, "TsunamiWave")
        .expect("TsunamiWave not found in tree");

    // Extract Texture children from snapshot (filesystem side, sparse)
    let snap_textures: Vec<librojo::InstanceSnapshot> = snap_tsunami
        .children
        .iter()
        .filter(|c| c.class_name.as_str() == "Texture")
        .cloned()
        .collect();

    // Extract Texture children from tree (Studio side, full properties)
    let tree_textures: Vec<Ref> = tree
        .get_instance(tree_tsunami)
        .unwrap()
        .children()
        .iter()
        .filter(|&&r| {
            tree.get_instance(r)
                .map_or(false, |i| i.class_name().as_str() == "Texture")
        })
        .copied()
        .collect();

    assert_eq!(
        snap_textures.len(),
        tree_textures.len(),
        "Texture count mismatch"
    );
    assert!(!snap_textures.is_empty(), "No Texture children found");

    // Run forward sync matching
    let result = librojo::snapshot::matching::match_forward(snap_textures, &tree_textures, &tree);

    assert_eq!(
        result.matched.len(),
        tree_textures.len(),
        "All Textures should be matched"
    );
    assert!(result.unmatched_snapshot.is_empty());
    assert!(result.unmatched_tree.is_empty());

    // Verify correct Face pairing
    let face_front = Variant::Enum(rbx_dom_weak::types::Enum::from_u32(5));

    for (snap, tree_ref) in &result.matched {
        let tree_inst = tree.get_instance(*tree_ref).unwrap();
        let tree_face = tree_inst.properties().get(&ustr("Face"));

        let snap_face = snap.properties.get(&ustr("Face"));

        match (snap_face, tree_face) {
            (Some(sv), Some(tv)) => {
                assert!(
                    librojo::variant_eq::variant_eq(sv, tv),
                    "Face mismatch: snap={:?}, tree={:?}",
                    sv,
                    tv
                );
            }
            (None, Some(tv)) => {
                // Snapshot omitted Face (default = Front)
                assert!(
                    librojo::variant_eq::variant_eq(tv, &face_front),
                    "Omitted Face should match Front, got {:?}",
                    tv
                );
            }
            (None, None) => {
                // Both omitted: both at default, fine
            }
            (Some(sv), None) => {
                panic!("Snap has Face={:?} but tree has none", sv);
            }
        }
    }
}
