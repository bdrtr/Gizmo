//! [`ReflectionProbe`] — where the scene is captured for specular reflections.

/// Captures the scene from this entity's position into a cube that metals and glossy surfaces
/// reflect, in place of the procedural sky. See [`crate::reflection`] for how.
///
/// **Captured once**, on the first frame it exists, and again after
/// [`request_capture`](Self::request_capture) — six full renders is a cost for a level load, not
/// for every frame. Move the probe or change the room, then ask for a recapture.
///
/// One probe at a time (the first found), looked up by direction only: reflections are right near
/// the probe and drift with distance from it. Particles and fluid are not captured.
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
    /// Whether the current cube is up to date. Cleared by [`request_capture`](Self::request_capture);
    /// not saved, so a loaded scene captures on its first frame.
    #[serde(skip)]
    pub captured: bool,
}

impl Default for ReflectionProbe {
    fn default() -> Self {
        Self { resolution: 128, intensity: 1.0, near: 0.05, far: 500.0, captured: false }
    }
}

impl ReflectionProbe {
    /// A probe with the defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recaptures on the next frame.
    pub fn request_capture(&mut self) {
        self.captured = false;
    }
}
