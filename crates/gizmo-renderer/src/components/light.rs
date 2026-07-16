use gizmo_math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

impl PointLight {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum LightRole {
    Sun,
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub intensity: f32,
    pub role: LightRole,
}

impl DirectionalLight {
    pub fn new(color: Vec3, intensity: f32, role: LightRole) -> Self {
        let intensity = intensity.max(0.0);
        Self {
            color,
            intensity,
            role,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpotLight {
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

impl SpotLight {
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
