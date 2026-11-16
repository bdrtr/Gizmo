//! **How often a skeleton is posed: by how big it was drawn.**
//!
//! Sampling a skeleton's clips, composing its joints and uploading its skin is paid per skeleton
//! per frame, whether the character fills the screen, is a dot on a minimap or stands behind the
//! camera. In `dunya` 127 animated characters cost 3.5–4 ms of every frame's preparation on a Ryzen
//! 7 7840HS (measured 2026-11-16), most of them far away or out of sight.
//!
//! The draw records, for each skeleton it drew — in the camera or only into a shadow map — how
//! tall it was on screen, in pixels, the largest of the frame's views ([`SkeletonsSeen::record`],
//! called by the game path's batching). The animation systems then pose a skeleton
//! ([`SkeletonsSeen::poses`]):
//!
//! - every frame when it was drawn at least [`EVERY_FRAME_PIXELS`] tall;
//! - every 2nd, 4th or 8th frame as it gets smaller (a sheep at 100 m, a dot on a map), staggered
//!   by entity so the work does not bunch into one frame;
//! - not at all when no view drew it in the last two frames — its playhead still advances, so it
//!   is where it should be in its clip when it is seen again; the first frame it is drawn shows
//!   the pose it was last given, and the next frame its own.
//!
//! A skeleton is posed regardless when something other than the clock moved it — never posed yet,
//! its clip or state switched, its playhead set by hand (an editor scrubbing a paused game, a
//! scene reopened half-way through a clip): only the frames where the driver's own clock is all
//! that changed are thinned out. Where nothing has recorded anything — a
//! headless step, the editor, whose draw is its own — every skeleton is posed every frame, as it
//! was before this existed. A player that must be posed whatever is seen (a hitbox read off a
//! bone, a camera on a head) says so with `AnimationUpdate::Always`.

use std::collections::HashMap;

use crate::components::Skeleton;
use gizmo_animation::skeletal::AnimationUpdate;

/// Pixels tall from which a skeleton is posed every frame.
pub const EVERY_FRAME_PIXELS: f32 = 120.0;

/// Frames between poses for a skeleton drawn `pixels` tall: 1, 2, 4 or 8.
#[must_use]
pub fn pose_interval(pixels: f32) -> u32 {
    if pixels >= EVERY_FRAME_PIXELS {
        1
    } else if pixels >= 50.0 {
        2
    } else if pixels >= 20.0 {
        4
    } else {
        8
    }
}

/// The skeletons the last frames drew and how big — see the [module](self).
#[derive(Default)]
pub struct SkeletonsSeen {
    /// This frame's, as the draw records them: skeleton → its largest height in pixels.
    drawing: HashMap<usize, f32>,
    /// The frame before's.
    drawn: HashMap<usize, f32>,
    /// Frames begun ([`begin_frame`](Self::begin_frame)); the stagger counts them.
    frame: u64,
    /// Whether any frame has recorded anything: until one does, every skeleton is posed.
    recording: bool,
}

/// A skeleton's identity: its skin's bind group, which every entity drawing that skin shares.
fn key(skeleton: &Skeleton) -> usize {
    std::sync::Arc::as_ptr(&skeleton.bind_group) as usize
}

impl SkeletonsSeen {
    /// Starts a frame's record: the last one becomes the frame before's.
    pub fn begin_frame(&mut self) {
        self.drawn = std::mem::take(&mut self.drawing);
        self.frame += 1;
    }

    /// The draw drew `skeleton` `pixels` tall in one of its views.
    pub fn record(&mut self, skeleton: &Skeleton, pixels: f32) {
        self.recording = true;
        let tallest = self.drawing.entry(key(skeleton)).or_insert(0.0);
        *tallest = tallest.max(pixels);
    }

    /// How tall `skeleton` was drawn in the last two frames, or `None` where no view drew it.
    #[must_use]
    pub fn pixels(&self, skeleton: &Skeleton) -> Option<f32> {
        let k = key(skeleton);
        match (self.drawing.get(&k), self.drawn.get(&k)) {
            (None, None) => None,
            (a, b) => Some(a.copied().unwrap_or(0.0).max(b.copied().unwrap_or(0.0))),
        }
    }

    /// Whether the skeleton of `entity` (its id, for the stagger) is posed this frame. `moved`: its
    /// clip, state or playhead was changed by something other than the driver's clock since the
    /// driver last left it (or it was never driven) — posed then whatever its size.
    #[must_use]
    pub fn poses(&self, skeleton: &Skeleton, entity: u32, update: AnimationUpdate, moved: bool) -> bool {
        if !self.recording || moved || update == AnimationUpdate::Always {
            return true;
        }
        match self.pixels(skeleton) {
            None => false,
            Some(pixels) => (self.frame + u64::from(entity)).is_multiple_of(u64::from(pose_interval(pixels))),
        }
    }

    /// Every skeleton posed every frame: what a caller with no draw passes.
    #[must_use]
    pub fn everything() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interval_grows_as_the_skeleton_shrinks() {
        assert_eq!(pose_interval(400.0), 1);
        assert_eq!(pose_interval(EVERY_FRAME_PIXELS), 1);
        assert_eq!(pose_interval(80.0), 2);
        assert_eq!(pose_interval(30.0), 4);
        assert_eq!(pose_interval(3.0), 8);
    }
}
