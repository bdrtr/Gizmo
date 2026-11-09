//! [`ReflectionProbe`] — where the scene is captured for specular reflections.

/// Captures the scene from this entity's position into a cube that metals and glossy surfaces
/// reflect, in place of the procedural sky. See [`crate::reflection`] for how.
///
/// **Captured once**, on the first frame it exists, and again after
/// [`request_capture`](Self::request_capture) — six full renders is a cost for a level load, not
/// for every frame. Move the probe or change the room, then ask for a recapture.
///
/// Up to four at a time (the first four by entity order). A probe **with** an
/// [`influence`](Self::influence) box — the room's inner bounds — is local: it applies only inside
/// the box, fading in over [`blend_distance`](Self::blend_distance) from its walls, and traces each
/// reflected ray to those walls first (box-projected parallax), so a mirror on the far side of the
/// room reflects the part of the wall it faces. Where boxes overlap, they blend. A probe
/// **without** a box is the background: it fills whatever the boxed probes leave uncovered, looked
/// up by direction; with no such probe, that is the procedural sky. Particles and fluid are not
/// captured.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ReflectionProbe {
    /// Texels per cube edge, rounded to a power of two in 8..=1024. 128 by default.
    pub resolution: u32,
    /// Multiplies what the probe reflects. 1 by default.
    pub intensity: f32,
    /// Near clip of the capture, in metres — keep it inside the smallest gap around the probe.
    pub near: f32,
    /// Far clip of the capture.
    pub far: f32,
    /// Half-extents of the box, centred on the probe and aligned to the world axes, that the
    /// captured scene is projected onto — usually the room the probe sits in. `None` (the default)
    /// looks reflections up by direction alone. A surface outside the box also falls back to that.
    pub influence: Option<gizmo_math::Vec3>,
    /// How far inside its box, in metres, the probe fades in from nothing to full weight — the
    /// width of the blend at a doorway. 0.5 by default.
    pub blend_distance: f32,
    /// Which capture this probe asks for — bumped by [`request_capture`](Self::request_capture).
    ///
    /// **Every new probe starts at a value no other probe in the process has had** (`fresh_request`),
    /// not at 0. The renderer keys what it captured by entity and request, and an entity handle can
    /// come back bit-identical: after `World::clear_entities` the first spawn is `Entity(0, gen 0)`
    /// again, so a level restarted in place would have its probes taken for the ones it replaced and
    /// keep the old level's cubes. A component put on in place of another, on the same entity, is
    /// new the same way. A copy (`Clone`, a render world's extraction) keeps the value, as it should.
    /// Not saved: a loaded probe is a new one.
    #[serde(skip, default = "fresh_request")]
    capture_request: u64,
}

/// A request value no probe in this process has held: the start of a new probe.
fn fresh_request() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Requests advance by one per `request_capture`, so new probes are spaced far apart.
    static NEXT: AtomicU64 = AtomicU64::new(1 << 32);
    NEXT.fetch_add(1 << 32, Ordering::Relaxed)
}

impl Default for ReflectionProbe {
    fn default() -> Self {
        Self {
            resolution: 128,
            intensity: 1.0,
            near: 0.05,
            far: 500.0,
            influence: None,
            blend_distance: 0.5,
            capture_request: fresh_request(),
        }
    }
}

/// What the game authors, compared: the capture request is the renderer's business, so two probes
/// built alike are equal whatever has been asked of either.
impl PartialEq for ReflectionProbe {
    fn eq(&self, other: &Self) -> bool {
        self.resolution == other.resolution
            && self.intensity == other.intensity
            && self.near == other.near
            && self.far == other.far
            && self.influence == other.influence
            && self.blend_distance == other.blend_distance
    }
}

impl ReflectionProbe {
    /// A probe with the defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// This probe with box-projected parallax over a box of `half_extents` around it.
    pub fn with_influence(mut self, half_extents: gizmo_math::Vec3) -> Self {
        self.influence = Some(half_extents.abs());
        self
    }

    /// Recaptures on the next frame.
    pub fn request_capture(&mut self) {
        self.capture_request = self.capture_request.wrapping_add(1);
    }

    /// Which capture this probe asks for. The renderer keeps the request each probe was last
    /// captured at (`ReflectionProbeGpu::captured`) and captures again when this differs — so a
    /// request is seen and honoured once without the draw writing anything back into the
    /// component, which is what lets it draw from a copy of the game world.
    #[must_use]
    pub fn capture_request(&self) -> u64 {
        self.capture_request
    }
}

#[cfg(test)]
mod tests {
    use super::ReflectionProbe;

    /// A request is a new value the renderer has not captured at — each one distinct from the
    /// last — and a new probe holds a value no earlier probe had, so it is captured on sight even
    /// where it takes an earlier probe's exact entity handle.
    #[test]
    fn a_request_is_a_new_capture_request() {
        let mut probe = ReflectionProbe::new();
        let first = probe.capture_request();
        probe.request_capture();
        assert_ne!(probe.capture_request(), first);
        let asked = probe.capture_request();
        probe.request_capture();
        assert_ne!(probe.capture_request(), asked);

        let replacement = ReflectionProbe::new();
        for seen in [first, asked, probe.capture_request()] {
            assert_ne!(replacement.capture_request(), seen, "a new probe looks like an old one to the renderer");
        }
        assert_eq!(probe.clone().capture_request(), probe.capture_request(), "a copy is the same probe");
        assert_eq!(replacement, ReflectionProbe::new(), "probes built alike compare equal");
    }
}
