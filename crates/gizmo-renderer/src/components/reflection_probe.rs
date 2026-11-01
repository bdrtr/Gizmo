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
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Whether the current cube is up to date. Cleared by [`request_capture`](Self::request_capture);
    /// not saved, so a loaded scene captures on its first frame.
    #[serde(skip)]
    pub captured: bool,
}

impl Default for ReflectionProbe {
    fn default() -> Self {
        Self { resolution: 128, intensity: 1.0, near: 0.05, far: 500.0, influence: None, blend_distance: 0.5, captured: false }
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
        self.captured = false;
    }
}
