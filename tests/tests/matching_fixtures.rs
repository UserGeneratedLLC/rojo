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
use rbx_dom_weak::{ustr, HashMapExt as _, WeakDom};

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
                .is_some_and(|i| i.class.as_str() == "Texture")
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
    let session = librojo::syncback::matching::MatchingSession::new();
    let result = librojo::syncback::matching::match_children(
        &input_textures,
        &expected_textures,
        &input_dom,
        &expected_dom,
        None,
        None,
        &session,
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
                .is_some_and(|i| i.class_name().as_str() == "Texture")
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
    let session = librojo::snapshot::matching::MatchingSession::new();
    let result =
        librojo::snapshot::matching::match_forward(snap_textures, &tree_textures, &tree, &session);

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
                assert!(
                    librojo::variant_eq::variant_eq(tv, &face_front),
                    "Omitted Face should match Front, got {:?}",
                    tv
                );
            }
            (None, None) => {}
            (Some(sv), None) => {
                panic!("Snap has Face={:?} but tree has none", sv);
            }
        }
    }
}

// ================================================================
// Parity: forward sync vs syncback produce same pairings
// ================================================================

#[test]
fn parity_forward_vs_syncback() {
    use rbx_dom_weak::types::Variant;
    use rbx_dom_weak::InstanceBuilder;
    use std::borrow::Cow;

    let transparency_values = [0.0_f32, 0.3, 0.6, 0.9];

    // Build "new" WeakDom (for syncback) with parts in forward order
    let mut new_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let new_root = new_dom.root_ref();
    for &t in &transparency_values {
        let builder = InstanceBuilder::new("Part")
            .with_name("Line")
            .with_property("Transparency", Variant::Float32(t));
        new_dom.insert(new_root, builder);
    }

    // Build "old" WeakDom (for syncback) with parts in REVERSED order
    let mut old_dom = WeakDom::new(InstanceBuilder::new("DataModel"));
    let old_root = old_dom.root_ref();
    for &t in transparency_values.iter().rev() {
        let builder = InstanceBuilder::new("Part")
            .with_name("Line")
            .with_property("Transparency", Variant::Float32(t));
        old_dom.insert(old_root, builder);
    }

    let new_children: Vec<Ref> = new_dom.get_by_ref(new_root).unwrap().children().to_vec();
    let old_children: Vec<Ref> = old_dom.get_by_ref(old_root).unwrap().children().to_vec();

    // Run syncback matching
    let syncback_session = librojo::syncback::matching::MatchingSession::new();
    let syncback_result = librojo::syncback::matching::match_children(
        &new_children,
        &old_children,
        &new_dom,
        &old_dom,
        None,
        None,
        &syncback_session,
    );

    // Build equivalent InstanceSnapshots for forward sync
    let snap_children: Vec<librojo::InstanceSnapshot> = transparency_values
        .iter()
        .map(|&t| {
            let mut properties = rbx_dom_weak::UstrMap::new();
            properties.insert(ustr("Transparency"), Variant::Float32(t));
            librojo::InstanceSnapshot {
                snapshot_id: Ref::none(),
                metadata: librojo::snapshot::InstanceMetadata::default(),
                name: Cow::Borrowed("Line"),
                class_name: ustr("Part"),
                properties,
                children: Vec::new(),
            }
        })
        .collect();

    // Build RojoTree from old_dom data (reversed order)
    let tree_snap_children: Vec<librojo::InstanceSnapshot> = transparency_values
        .iter()
        .rev()
        .map(|&t| {
            let mut properties = rbx_dom_weak::UstrMap::new();
            properties.insert(ustr("Transparency"), Variant::Float32(t));
            librojo::InstanceSnapshot {
                snapshot_id: Ref::none(),
                metadata: librojo::snapshot::InstanceMetadata::default(),
                name: Cow::Borrowed("Line"),
                class_name: ustr("Part"),
                properties,
                children: Vec::new(),
            }
        })
        .collect();

    let root_snap = librojo::InstanceSnapshot {
        snapshot_id: Ref::none(),
        metadata: librojo::snapshot::InstanceMetadata::default(),
        name: Cow::Borrowed("DataModel"),
        class_name: ustr("DataModel"),
        properties: Default::default(),
        children: tree_snap_children,
    };
    let tree = librojo::RojoTree::new(root_snap);
    let tree_root = tree.get_root_id();
    let tree_children_refs: Vec<Ref> = tree.get_instance(tree_root).unwrap().children().to_vec();

    // Run forward sync matching
    let forward_session = librojo::snapshot::matching::MatchingSession::new();
    let forward_result = librojo::snapshot::matching::match_forward(
        snap_children,
        &tree_children_refs,
        &tree,
        &forward_session,
    );

    // Both should match all 4
    assert_eq!(syncback_result.matched.len(), 4);
    assert_eq!(forward_result.matched.len(), 4);

    // Verify both produce the same pairing (by Transparency value)
    let mut syncback_pairs: Vec<(f32, f32)> = syncback_result
        .matched
        .iter()
        .map(|(nr, or)| {
            let nt = match new_dom
                .get_by_ref(*nr)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
            {
                Some(Variant::Float32(t)) => *t,
                _ => panic!("Missing Transparency on new"),
            };
            let ot = match old_dom
                .get_by_ref(*or)
                .unwrap()
                .properties
                .get(&ustr("Transparency"))
            {
                Some(Variant::Float32(t)) => *t,
                _ => panic!("Missing Transparency on old"),
            };
            (nt, ot)
        })
        .collect();
    syncback_pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let mut forward_pairs: Vec<(f32, f32)> = forward_result
        .matched
        .iter()
        .map(|(snap, tree_ref)| {
            let st = match snap.properties.get(&ustr("Transparency")) {
                Some(Variant::Float32(t)) => *t,
                _ => panic!("Missing Transparency on snap"),
            };
            let tt = match tree
                .get_instance(*tree_ref)
                .unwrap()
                .properties()
                .get(&ustr("Transparency"))
            {
                Some(Variant::Float32(t)) => *t,
                _ => panic!("Missing Transparency on tree"),
            };
            (st, tt)
        })
        .collect();
    forward_pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    assert_eq!(
        syncback_pairs, forward_pairs,
        "Syncback and forward sync produced different pairings"
    );

    // Verify correct pairing: each pair should have matching Transparency
    for (nt, ot) in &syncback_pairs {
        assert_eq!(
            nt, ot,
            "Incorrect pairing: new Transparency={} matched to old Transparency={}",
            nt, ot
        );
    }
}

// ================================================================
// Round-trip: syncback matching + forward sync matching agree
// ================================================================

#[test]
fn round_trip_matching_identity() {
    let _ = env_logger::try_init();

    // Load the input rbxm (simulates Studio with full properties)
    let input_dom = load_input_rbxm();
    let input_root = input_dom.root_ref();
    let input_tsunami = find_descendant(&input_dom, input_root, "TsunamiWave")
        .expect("TsunamiWave not found in input.rbxm");
    let input_textures = get_texture_children(&input_dom, input_tsunami);
    assert_eq!(input_textures.len(), 12, "Expected 12 Textures in input");

    // Load the expected/ filesystem as a WeakDom (syncback output)
    let expected_dom = load_expected_dom();
    let expected_root = expected_dom.root_ref();
    let expected_tsunami = find_descendant(&expected_dom, expected_root, "TsunamiWave")
        .expect("TsunamiWave not found in expected/");
    let expected_textures = get_texture_children(&expected_dom, expected_tsunami);
    assert_eq!(
        expected_textures.len(),
        12,
        "Expected 12 Textures in expected/"
    );

    // Step 1: Syncback matching (input rbxm vs expected filesystem)
    let syncback_session = librojo::syncback::matching::MatchingSession::new();
    let syncback_result = librojo::syncback::matching::match_children(
        &input_textures,
        &expected_textures,
        &input_dom,
        &expected_dom,
        None,
        None,
        &syncback_session,
    );
    assert_eq!(
        syncback_result.matched.len(),
        12,
        "Syncback should match all 12"
    );

    // Step 2: Forward sync matching (expected/ snapshots vs input-as-tree)
    let expected_path = Path::new(SYNCBACK_TESTS_PATH)
        .join(FIXTURE_DIR)
        .join("expected");
    let vfs = memofs::Vfs::new_default();
    let ctx = librojo::InstanceContext::default();
    let full_snapshot = librojo::snapshot_from_vfs(&ctx, &vfs, &expected_path)
        .expect("Failed to snapshot expected/")
        .expect("snapshot_from_vfs returned None");
    let snap_tsunami = full_snapshot
        .children
        .iter()
        .find(|c| c.name == "TsunamiWave")
        .expect("TsunamiWave not found in snapshot");
    let snap_textures: Vec<librojo::InstanceSnapshot> = snap_tsunami
        .children
        .iter()
        .filter(|c| c.class_name.as_str() == "Texture")
        .cloned()
        .collect();
    assert_eq!(snap_textures.len(), 12, "Expected 12 snapshot Textures");

    let input_dom2 = load_input_rbxm();
    let root_ref2 = input_dom2.root_ref();
    let input_snapshot = librojo::InstanceSnapshot::from_tree(input_dom2, root_ref2);
    let tree = librojo::RojoTree::new(input_snapshot);
    let tree_root = tree.get_root_id();
    let tree_tsunami = find_descendant_by_name(&tree, tree_root, "TsunamiWave")
        .expect("TsunamiWave not found in tree");
    let tree_textures: Vec<Ref> = tree
        .get_instance(tree_tsunami)
        .unwrap()
        .children()
        .iter()
        .filter(|&&r| {
            tree.get_instance(r)
                .is_some_and(|i| i.class_name().as_str() == "Texture")
        })
        .copied()
        .collect();
    assert_eq!(tree_textures.len(), 12, "Expected 12 tree Textures");

    let forward_session = librojo::snapshot::matching::MatchingSession::new();
    let forward_result = librojo::snapshot::matching::match_forward(
        snap_textures,
        &tree_textures,
        &tree,
        &forward_session,
    );
    assert_eq!(
        forward_result.matched.len(),
        12,
        "Forward sync should match all 12"
    );

    // Step 3: Verify both directions paired by the same Face value.
    // Syncback pairs: (input_face, expected_face)
    let face_front = 5_u32;
    let mut syncback_faces: Vec<(u32, u32)> = syncback_result
        .matched
        .iter()
        .map(|(input_ref, expected_ref)| {
            let iface = get_face(&input_dom, *input_ref).unwrap_or(face_front);
            let eface = get_face(&expected_dom, *expected_ref).unwrap_or(face_front);
            (iface, eface)
        })
        .collect();
    syncback_faces.sort();

    // Forward pairs: (snap_face, tree_face)
    let mut forward_faces: Vec<(u32, u32)> = forward_result
        .matched
        .iter()
        .map(|(snap, tree_ref)| {
            let sface = snap
                .properties
                .get(&ustr("Face"))
                .and_then(|v| match v {
                    Variant::Enum(e) => Some(e.to_u32()),
                    _ => None,
                })
                .unwrap_or(face_front);
            let tface = tree
                .get_instance(*tree_ref)
                .and_then(|i| i.properties().get(&ustr("Face")))
                .and_then(|v| match v {
                    Variant::Enum(e) => Some(e.to_u32()),
                    _ => None,
                })
                .unwrap_or(face_front);
            (sface, tface)
        })
        .collect();
    forward_faces.sort();

    // Both directions must produce identical (face, face) pairs
    assert_eq!(
        syncback_faces, forward_faces,
        "Round-trip mismatch: syncback and forward sync disagree on Face pairings"
    );

    // Every pair must have matching Face values
    for (a, b) in &syncback_faces {
        assert_eq!(a, b, "Face mismatch in round-trip: {} vs {}", a, b);
    }
}
