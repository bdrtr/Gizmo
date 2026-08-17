use gizmo_math::Vec3;

/// A light radiating equally in all directions from the entity's position.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    /// Linear RGB colour.
    pub color: Vec3,
    /// Brightness. Zero is a light that is present but contributes nothing.
    pub intensity: f32,
    /// The distance at which the falloff reaches zero. It is also the culling bound — a light is
    /// only considered for surfaces inside this radius, so an over-large radius costs
    /// performance and a too-small one clips the light off visibly.
    pub radius: f32,
}

impl PointLight {
    /// A point light, with a negative intensity floored at zero and the radius kept above 0.001 —
    /// a zero radius divides by zero in the falloff.
    pub fn new(color: Vec3, intensity: f32, radius: f32) -> Self {
        let intensity = intensity.max(0.0);
        let radius = radius.max(0.001);
        Self {
            color,
            intensity,
            radius,
        }
    }
}

/// What a [`DirectionalLight`] is *for* — which one of several is the scene's sun.
///
/// The distinction is load-bearing: only the sun drives the cascaded shadow maps and the sky
/// gradient, and there is one set of cascades. A scene with two suns would have them fight over
/// those cascades, so the extra directional lights are [`Generic`](Self::Generic) fill and shade
/// nothing but the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LightRole {
    /// The scene's sun: it drives the shadow cascades and the sky.
    Sun,
    /// An ordinary directional light — fill or rim, casting no shadows.
    Generic,
}

/// A light from an infinitely distant source: parallel rays, no falloff. Its direction comes
/// from the entity's `Transform`, not from a field here.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionalLight {
    /// Linear RGB colour.
    pub color: Vec3,
    /// Brightness.
    pub intensity: f32,
    /// Whether this is the scene's sun — see [`LightRole`].
    pub role: LightRole,
}

impl DirectionalLight {
    /// A directional light, with a negative intensity floored at zero.
    pub fn new(color: Vec3, intensity: f32, role: LightRole) -> Self {
        let intensity = intensity.max(0.0);
        Self {
            color,
            intensity,
            role,
        }
    }
}

/// A cone of light from the entity's position, aimed along its `Transform`'s forward axis.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpotLight {
    /// Linear RGB colour.
    pub color: Vec3,
    /// Brightness.
    pub intensity: f32,
    /// The distance the falloff reaches zero at, as for [`PointLight::radius`].
    pub radius: f32,
    /// Half-angle of the cone's fully-lit core, in radians.
    pub inner_angle: f32,
    /// Half-angle at which the light reaches zero. Between the two the edge falls off smoothly;
    /// equal angles give a hard-edged cone.
    pub outer_angle: f32,
}

impl SpotLight {
    /// A spot light, with intensity floored at zero, the radius kept above 0.001, and an inner
    /// angle clamped to no more than the outer one — an inner cone wider than its outer cone
    /// inverts the falloff and lights the outside of the cone instead.
    pub fn new(
        color: Vec3,
        intensity: f32,
        radius: f32,
        inner_angle: f32,
        outer_angle: f32,
    ) -> Self {
        let intensity = intensity.max(0.0);
        let radius = radius.max(0.001);
        let inner_angle = inner_angle.min(outer_angle);
        Self {
            color,
            intensity,
            radius,
            inner_angle,
            outer_angle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_light_clamps_negative_intensity_and_radius() {
        let l = PointLight::new(Vec3::new(1.0, 0.0, 0.0), -5.0, -1.0);
        assert_eq!(l.intensity, 0.0, "negative intensity clamped to 0");
        assert_eq!(l.radius, 0.001, "non-positive radius clamped to the 0.001 floor");
        // Valid values pass through untouched.
        let ok = PointLight::new(Vec3::ONE, 3.0, 2.0);
        assert_eq!(ok.intensity, 3.0);
        assert_eq!(ok.radius, 2.0);
    }

    #[test]
    fn directional_light_clamps_intensity_but_keeps_role() {
        let l = DirectionalLight::new(Vec3::ONE, -2.0, LightRole::Sun);
        assert_eq!(l.intensity, 0.0);
        assert_eq!(l.role, LightRole::Sun);
        let ok = DirectionalLight::new(Vec3::ONE, 4.0, LightRole::Generic);
        assert_eq!(ok.intensity, 4.0);
        assert_eq!(ok.role, LightRole::Generic);
    }

    #[test]
    fn spot_light_inner_angle_never_exceeds_outer() {
        // inner > outer must be pulled down to outer (avoids an inverted cone falloff).
        let l = SpotLight::new(Vec3::ONE, 2.0, 1.0, 1.5, 0.5);
        assert!(l.inner_angle <= l.outer_angle);
        assert_eq!(l.inner_angle, 0.5);
        assert_eq!(l.outer_angle, 0.5);
        // A well-ordered pair is preserved.
        let ok = SpotLight::new(Vec3::ONE, 1.0, 2.0, 0.2, 0.6);
        assert_eq!(ok.inner_angle, 0.2);
        assert_eq!(ok.outer_angle, 0.6);
    }

    #[test]
    fn spot_light_clamps_intensity_and_radius() {
        let l = SpotLight::new(Vec3::ONE, -1.0, 0.0, 0.2, 0.4);
        assert_eq!(l.intensity, 0.0);
        assert_eq!(l.radius, 0.001);
    }

    #[test]
    fn lights_survive_a_serde_roundtrip() {
        let p = PointLight::new(Vec3::new(0.25, 0.5, 0.75), 1.5, 3.0);
        let back: PointLight = ron::from_str(&ron::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);

        let d = DirectionalLight::new(Vec3::new(1.0, 1.0, 0.5), 2.0, LightRole::Generic);
        let back: DirectionalLight = ron::from_str(&ron::to_string(&d).unwrap()).unwrap();
        assert_eq!(d, back);

        let s = SpotLight::new(Vec3::ONE, 1.0, 2.0, 0.25, 0.5);
        let back: SpotLight = ron::from_str(&ron::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
