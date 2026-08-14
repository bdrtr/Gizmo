//! What order blended geometry is painted in, decided once.
//!
//! # Why this exists
//!
//! The transparent pipeline writes no depth, so for anything blended the **draw order is the
//! result** — there is no depth test to fall back on. Both render paths know that, and each had
//! written its own answer:
//!
//! - The batch's representative depth was `batch_sort_depth` in the engine and
//!   `batch_centroid_depth` in `gizmo-studio`. Same computation to the character — the mean of the
//!   instance translations, distance from the camera — under two names, in two crates, with two
//!   tests.
//! - Sorting the instances **inside** a transparent batch was studio-only. The engine ordered its
//!   batches and then appended each batch's instances in collection order, so two overlapping
//!   transparent surfaces of the *same material* — a row of windows on one building, a stack of
//!   glass panes, any blended prop instanced more than once — composited in ECS iteration order in
//!   the game and back-to-front in the editor. Which one won flipped between runs.
//!
//! The second is the one that cost something, and it is the same shape as the first: nothing made
//! the two paths answer one question once.
//!
//! # What it does not decide
//!
//! Which *layer* a batch belongs to (backdrop before world before transparent) stays with the
//! engine's `DrawLayer`, because the two paths express it differently and deliberately: the engine
//! sorts one list with a comparator, the editor drains three separate maps in a fixed order. That
//! is a mechanism difference, not a disagreement about the answer.

use crate::gpu_types::InstanceRaw;
use gizmo_math::Vec3;

/// The distance the batch as a whole sorts by: the mean of its instances' world translations,
/// measured from the camera.
///
/// A mean rather than the nearest or furthest instance because it is the value that behaves for a
/// batch that straddles the camera — and because both paths already used it, which is a reason to
/// keep it rather than to reopen it.
#[must_use]
pub fn batch_depth(instances: &[InstanceRaw], cam_pos: Vec3) -> f32 {
    if instances.is_empty() {
        return 0.0;
    }
    let mut centroid = Vec3::ZERO;
    for inst in instances {
        // `InstanceRaw::model` is column-major; column 3 is the translation.
        centroid += Vec3::new(inst.model[3][0], inst.model[3][1], inst.model[3][2]);
    }
    centroid /= instances.len() as f32;
    cam_pos.distance(centroid)
}

/// Order one batch's instances far-to-near, which is the order blending needs.
///
/// Only meaningful for a batch that writes no depth. Calling it on an opaque batch is not wrong,
/// just wasted work — the depth buffer resolves those in any order.
pub fn sort_back_to_front(instances: &mut [InstanceRaw], cam_pos: Vec3) {
    instances.sort_by(|a, b| {
        let da = cam_pos.distance_squared(Vec3::new(a.model[3][0], a.model[3][1], a.model[3][2]));
        let db = cam_pos.distance_squared(Vec3::new(b.model[3][0], b.model[3][1], b.model[3][2]));
        // Descending: the farther instance is painted first.
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32, z: f32) -> InstanceRaw {
        InstanceRaw::new(
            [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, y, z, 1.0]],
            [1.0; 4],
            0.5,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            [0.0; 3],
            [0.0; 3],
        )
    }

    fn zs(instances: &[InstanceRaw]) -> Vec<f32> {
        instances.iter().map(|i| i.model[3][2]).collect()
    }

    #[test]
    fn batch_depth_is_the_distance_to_the_centroid() {
        let cam = Vec3::ZERO;
        assert!((batch_depth(&[at(0.0, 0.0, -10.0)], cam) - 10.0).abs() < 1e-3);
        // Centroid of (±3, 0, -4) is (0, 0, -4) → 4, not 5.
        let d = batch_depth(&[at(3.0, 0.0, -4.0), at(-3.0, 0.0, -4.0)], cam);
        assert!((d - 4.0).abs() < 1e-3, "got {d}");
        assert_eq!(batch_depth(&[], cam), 0.0, "an empty batch sorts at zero");
    }

    /// The property the engine was missing: the painted order must not depend on the order the
    /// instances happened to be collected in.
    #[test]
    fn sorting_is_independent_of_the_order_they_arrived_in() {
        let cam = Vec3::new(0.0, 0.0, 10.0);
        let mut near_first = vec![at(0.0, 0.0, 5.0), at(0.0, 0.0, -5.0), at(0.0, 0.0, 0.0)];
        let mut far_first = vec![at(0.0, 0.0, -5.0), at(0.0, 0.0, 0.0), at(0.0, 0.0, 5.0)];
        sort_back_to_front(&mut near_first, cam);
        sort_back_to_front(&mut far_first, cam);
        assert_eq!(zs(&near_first), vec![-5.0, 0.0, 5.0], "farthest painted first");
        assert_eq!(zs(&near_first), zs(&far_first), "the arrival order must not survive");
    }
}
