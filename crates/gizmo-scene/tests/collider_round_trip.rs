//! A collider that survives a save/load has to come back with everything it needs, including the
//! parts a scene file does not store.
//!
//! Two shapes keep derived state beside their data — the triangle mesh's BVH and the
//! heightfield's cached bounds — and both are `#[serde(skip)]`, rebuilt on load. That is the kind
//! of field whose loss is invisible: the shape deserialises, the inspector shows the right
//! numbers, and the body collides with nothing because its broadphase box is a point at the
//! origin. This test lives here because `gizmo-scene` is the layer that actually serialises a
//! collider, and it is the only crate below the facade with a serialiser to test through.

use gizmo_physics_core::{Collider, ColliderShape};
use gizmo_math::Vec3;

#[test]
fn a_heightfield_comes_back_with_the_bounds_a_scene_file_does_not_store() {
    let collider =
        Collider::heightfield(vec![0.0, 1.0, 2.0, 3.0], 2, 2, Vec3::new(1.0, 4.0, 1.0));

    let text = ron::ser::to_string(&collider).expect("a collider serialises");
    let back: Collider = ron::from_str(&text).expect("and comes back");

    let ColliderShape::Heightfield(hf) = &back.shape else {
        panic!("expected a heightfield, got {:?}", back.shape)
    };
    assert_eq!(hf.rows, 2);
    assert_eq!(hf.cols, 2);
    assert_eq!(hf.height_at(1, 1), 12.0, "3 × scale.y");
    assert_eq!(
        hf.local_aabb.max.y, 12.0,
        "the bounds are `serde(skip)` and must be measured again on load"
    );
    assert_eq!(hf.local_aabb.min.y, 0.0);
    assert_eq!(hf.cell_counts(), (1, 1), "and the lattice still makes a cell");
}

/// A file can hold anything. A sample count that disagrees with the lattice must not be indexed
/// into: it loads as a field that collides with nothing, and says so in a warning.
#[test]
fn a_malformed_heightfield_in_a_file_loads_as_nothing_rather_than_panicking() {
    // Written by serialising a good one and then losing a sample, which is what a hand-edited or
    // truncated file looks like — rather than a hand-typed literal that would also have to keep
    // up with every other field of a `Collider`.
    let good = Collider::heightfield(vec![0.0, 1.0, 2.0, 3.0], 2, 2, Vec3::ONE);
    let text = ron::ser::to_string(&good)
        .expect("serialise")
        .replace("[0.0,1.0,2.0,3.0]", "[0.0,1.0,2.0]");
    assert!(text.contains("[0.0,1.0,2.0]"), "the corruption landed: {text}");
    let back: Collider = ron::from_str(&text).expect("a malformed field still parses");
    let ColliderShape::Heightfield(hf) = &back.shape else {
        panic!("expected a heightfield")
    };
    assert_eq!(hf.cell_counts(), (0, 0), "three samples cannot fill a 2×2 lattice");
}
