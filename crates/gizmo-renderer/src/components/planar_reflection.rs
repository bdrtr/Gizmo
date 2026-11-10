//! [`PlanarReflection`] — a flat surface that reflects the scene.

use gizmo_math::{Mat4, Vec3, Vec4};

/// Makes this entity's flat mesh a true reflection of the scene: a mirror, a polished floor, a pool.
///
/// Unlike screen-space reflections it shows what the camera cannot see, because the scene is drawn
/// again from the camera reflected in the plane ([`crate::planar`]). **That is what it costs: one more
/// full draw of the scene per mirror in view**, shadows included, so a game wants one or two. A mirror
/// out of view, or seen from behind, costs nothing that frame.
///
/// Give the entity an [`unlit`](crate::components::Material::with_unlit) material: its colour is what
/// shows where the surface does not reflect, and it keeps the mirror out of the deferred and shadow
/// passes. Or a [`water`](crate::components::Material::with_water) one with [`water`](Self::water): the
/// waves then break the reflection up, and Fresnel decides how much of it shows. (The water's own
/// texture is not read while it reflects; its colour is.) Seen from underwater it has no capture and
/// draws as plain water. Mirrors do not appear in each other, in their own reflection or in a reflection probe, and
/// there is no blur: every mirror is perfectly sharp. Every view captures its own — a split-screen
/// view and a camera drawing into a texture reflect for their own camera, one more scene draw each —
/// and in the editor the surface draws as its plain colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanarReflection {
    /// The surface's outward normal in the mesh's local space (default +Y, a floor or a pool).
    pub local_normal: Vec3,
    /// The surface's distance from the local origin along that normal (0 for a plane mesh; half its
    /// thickness for a thin box).
    pub local_offset: f32,
    /// How much it reflects head-on, `0..=1`. 1 is a mirror; 0.02 is water or glass, which still
    /// reflects everything at a grazing angle.
    pub reflectance: f32,
}

impl Default for PlanarReflection {
    fn default() -> Self {
        Self { local_normal: Vec3::Y, local_offset: 0.0, reflectance: 1.0 }
    }
}

impl PlanarReflection {
    /// A perfect mirror facing local +Y through the local origin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// For a [`Material::with_water`](crate::components::Material::with_water) surface: facing local
    /// +Y, reflecting 2 % head-on and everything at a grazing angle, as water does.
    #[must_use]
    pub fn water() -> Self {
        Self { reflectance: 0.02, ..Self::default() }
    }

    /// Faces `normal` in local space, `offset` along it from the local origin.
    #[must_use]
    pub fn facing(mut self, normal: Vec3, offset: f32) -> Self {
        self.local_normal = normal;
        self.local_offset = offset;
        self
    }

    /// Replaces the head-on reflectance, clamped to `0..=1`.
    #[must_use]
    pub fn with_reflectance(mut self, reflectance: f32) -> Self {
        self.reflectance = reflectance.clamp(0.0, 1.0);
        self
    }

    /// The world-space plane as `(normal, d)`, with `normal · p + d > 0` on the reflecting side.
    /// `None` for a degenerate transform or normal.
    #[must_use]
    pub fn world_plane(&self, model: &Mat4) -> Option<Vec4> {
        let local = self.local_normal.try_normalize()?;
        let normal = model.inverse().transpose().transform_vector3(local).try_normalize()?;
        let point = model.transform_point3(local * self.local_offset);
        let plane = normal.extend(-normal.dot(point));
        plane.is_finite().then_some(plane)
    }
}

gizmo_core::impl_component!(PlanarReflection);
