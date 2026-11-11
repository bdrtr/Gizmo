//! **A shadow pass draws the casters in its own frustum**, not every caster of the frame.
//!
//! The cascades and every local light's layers each draw the casters; instanced batches are too
//! large to cull whole — a field of a thousand rocks is one batch whose box is the field — so a
//! batch's instances are put in spatial order
//! ([`spatial_order`](crate::shadow_culling::spatial_order)) and cut into runs of at most
//! [`SHADOW_CHUNK`](crate::shadow_culling::SHADOW_CHUNK) neighbours, each with the box around it
//! ([`chunk`](crate::shadow_culling::chunk)). A pass draws the runs its frustum meets
//! ([`visible_ranges`](crate::shadow_culling::visible_ranges)), neighbouring ones as one draw.
//!
//! Until 2026-11-11 every pass drew every caster: a spot lighting three metres of floor drew the
//! whole level into its layer. Measured on 2 500 objects (3.8 million triangles) under 16 shadowed
//! spots: 26 096 shadow instances and 15.3 ms a frame then, 5 698 and 10.7 ms with this (9.1 ms with
//! no shadows); with the sun's cascades as well, 50 020 and 23.0 ms then, 13 607 and 14.1 ms now —
//! every frame byte for byte the one drawn without culling.
//!
//! Both render paths build their chunks and draw their passes through these functions, so the
//! editor's viewport culls exactly as the game does.

use gizmo_math::{Aabb, Frustum};
use std::ops::Range;

/// The most instances a chunk holds. Small enough that a lamp lighting a corner of a level draws
/// a few of them; large enough that a frame of a hundred thousand casters has a few thousand boxes
/// to test a pass against, not a hundred thousand.
pub const SHADOW_CHUNK: usize = 64;

/// A run of a batch's instances, contiguous in the instance buffer, and the box around them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShadowChunk {
    /// The instances, as indices into the frame's instance buffer.
    pub range: Range<u32>,
    /// Every one of their world boxes, merged.
    pub bounds: Aabb,
}

/// The order to put instances in so that each run of [`SHADOW_CHUNK`] of them is compact: by the
/// Morton code of each box's centre within the union of them all, 10 bits an axis. Stable — equal
/// codes keep their order — so it is a function of the boxes alone.
#[must_use]
pub fn spatial_order(bounds: &[Aabb]) -> Vec<u32> {
    let mut order: Vec<u32> = (0..bounds.len() as u32).collect();
    if bounds.len() <= SHADOW_CHUNK {
        return order;
    }
    let all = bounds.iter().fold(Aabb::empty(), |a, b| a.merge(*b));
    let size = (all.max - all.min).max(gizmo_math::Vec3A::splat(1e-6));
    let key = |b: &Aabb| -> u32 {
        let t = ((b.center() - all.min) / size).clamp(gizmo_math::Vec3A::ZERO, gizmo_math::Vec3A::ONE) * 1023.0;
        // A non-finite centre sorts first rather than poisoning the order.
        let q = |v: f32| if v.is_finite() { v as u32 } else { 0 };
        spread(q(t.x)) | (spread(q(t.y)) << 1) | (spread(q(t.z)) << 2)
    };
    let keys: Vec<u32> = bounds.iter().map(key).collect();
    order.sort_by_key(|&i| keys[i as usize]);
    order
}

/// The ten low bits of `v` with two zero bits between each: one axis of a 30-bit Morton code.
fn spread(v: u32) -> u32 {
    let mut x = v & 0x3ff;
    x = (x | (x << 16)) & 0x0300_00ff;
    x = (x | (x << 8)) & 0x0300_f00f;
    x = (x | (x << 4)) & 0x030c_30c3;
    (x | (x << 2)) & 0x0924_9249
}

/// `bounds` — already in [`spatial_order`], their instances at `first..` in the buffer — cut into
/// runs of [`SHADOW_CHUNK`], appended to `out`.
pub fn chunk(bounds: &[Aabb], first: u32, out: &mut Vec<ShadowChunk>) {
    for (i, run) in bounds.chunks(SHADOW_CHUNK).enumerate() {
        let start = first + (i * SHADOW_CHUNK) as u32;
        out.push(ShadowChunk {
            range: start..start + run.len() as u32,
            bounds: run.iter().fold(Aabb::empty(), |a, b| a.merge(*b)),
        });
    }
}

/// The instance ranges of `chunks` a pass through `frustum` draws: the chunks it meets, a run of
/// neighbours as one range, each clamped to the `uploaded` instances that reached the buffer.
pub fn visible_ranges<'a>(
    chunks: &'a [ShadowChunk],
    frustum: &'a Frustum,
    uploaded: u32,
) -> impl Iterator<Item = Range<u32>> + 'a {
    let mut seen = chunks.iter().filter(|c| frustum.intersects_aabb(c.bounds)).map(|c| c.range.clone()).peekable();
    std::iter::from_fn(move || {
        let mut range = seen.next()?;
        while let Some(next) = seen.peek() {
            if next.start != range.end {
                break;
            }
            range.end = next.end;
            seen.next();
        }
        Some(range.start.min(uploaded)..range.end.min(uploaded))
    })
    .filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Mat4, Vec3};

    fn cube_at(x: f32, z: f32) -> Aabb {
        Aabb::new(Vec3::new(x - 0.5, 0.0, z - 0.5), Vec3::new(x + 0.5, 1.0, z + 0.5))
    }

    /// A 64 × 64 grid of boxes, spawned row by row.
    fn grid() -> Vec<Aabb> {
        (0..64 * 64).map(|i| cube_at((i % 64) as f32 * 2.0, (i / 64) as f32 * 2.0)).collect()
    }

    /// In spatial order, a chunk is a patch of the grid, not a strip across it: no chunk's box is
    /// wider than 16 m on any side (an 8 × 8 patch is 15 m), where in spawn order every chunk is a
    /// whole row, 127 m long.
    #[test]
    fn spatial_order_makes_compact_chunks() {
        let bounds = grid();
        let order = spatial_order(&bounds);
        let mut sorted: Vec<u32> = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..bounds.len() as u32).collect::<Vec<_>>(), "a permutation");
        let ordered: Vec<Aabb> = order.iter().map(|&i| bounds[i as usize]).collect();
        let mut chunks = Vec::new();
        chunk(&ordered, 100, &mut chunks);
        assert_eq!(chunks.len(), 64);
        assert_eq!(chunks[0].range, 100..164);
        assert_eq!(chunks.last().unwrap().range.end, 100 + 64 * 64);
        for c in &chunks {
            let size = c.bounds.max - c.bounds.min;
            assert!(size.x <= 16.0 && size.z <= 16.0, "a chunk {size:?} across");
        }
        let mut unsorted = Vec::new();
        chunk(&bounds, 0, &mut unsorted);
        assert!((unsorted[0].bounds.max.x - unsorted[0].bounds.min.x) > 100.0, "spawn order: a row");
    }

    /// A batch no larger than a chunk keeps its order: there is nothing to cull inside it.
    #[test]
    fn a_small_batch_keeps_its_order() {
        let bounds: Vec<Aabb> = (0..10).rev().map(|i| cube_at(i as f32, 0.0)).collect();
        assert_eq!(spatial_order(&bounds), (0..10).collect::<Vec<u32>>());
    }

    /// A frustum looking at one corner of the grid draws the chunks there and no others, and
    /// neighbouring chunks come out as one range, clamped to what reached the buffer.
    #[test]
    fn a_pass_draws_the_chunks_its_frustum_meets() {
        let bounds = grid();
        let order = spatial_order(&bounds);
        let ordered: Vec<Aabb> = order.iter().map(|&i| bounds[i as usize]).collect();
        let mut chunks = Vec::new();
        chunk(&ordered, 0, &mut chunks);
        // A spot 4 m over the grid's first corner, looking down, 40° half-angle, 12 m reach.
        let view_proj = Mat4::perspective_rh(80f32.to_radians(), 1.0, 0.05, 12.0)
            * Mat4::look_to_rh(Vec3::new(3.0, 4.0, 3.0), Vec3::NEG_Y, Vec3::Z);
        let frustum = Frustum::from_matrix(&view_proj);
        let ranges: Vec<Range<u32>> = visible_ranges(&chunks, &frustum, u32::MAX).collect();
        let drawn: u32 = ranges.iter().map(|r| r.end - r.start).sum();
        assert!(drawn > 0 && drawn <= 4 * SHADOW_CHUNK as u32, "{drawn} of 4096 instances drawn");
        for pair in ranges.windows(2) {
            assert!(pair[0].end < pair[1].start, "neighbours merged: {ranges:?}");
        }
        // Every box the frustum meets is in a drawn range.
        for (slot, b) in ordered.iter().enumerate() {
            if frustum.intersects_aabb(*b) {
                assert!(ranges.iter().any(|r| r.contains(&(slot as u32))), "box {slot} meets the frustum and is not drawn");
            }
        }
        // Clamped to the buffer.
        let clamped: Vec<Range<u32>> = visible_ranges(&chunks, &frustum, 10).collect();
        assert!(clamped.iter().all(|r| r.end <= 10));
    }

    /// A frustum that takes in everything draws everything, as one range.
    #[test]
    fn a_pass_over_everything_draws_one_range() {
        let bounds = grid();
        let mut chunks = Vec::new();
        chunk(&bounds, 7, &mut chunks);
        let frustum = Frustum::from_matrix(&(Mat4::orthographic_rh(-1000.0, 1000.0, -1000.0, 1000.0, -1000.0, 1000.0)));
        let ranges: Vec<Range<u32>> = visible_ranges(&chunks, &frustum, u32::MAX).collect();
        assert_eq!(ranges, vec![7..7 + 64 * 64]);
    }
}
