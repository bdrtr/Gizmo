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
    /// Radius of the light's **source**, in metres — a bulb's size, not its reach (that is
    /// [`radius`](Self::radius)). A sphere light's highlight on a glossy surface is as big as the
    /// sphere's reflection, not a pinpoint; `0` (the default) is the old point source. Specular
    /// uses the representative-point method (the point on the sphere nearest the reflected ray,
    /// energy-normalised), diffuse the centre. `#[serde(default)]`.
    #[serde(default)]
    pub source_radius: f32,
    /// Length of the light's source along the entity's **local X axis**, in metres — a tube
    /// (fluorescent, neon) of [`source_radius`](Self::source_radius) thickness instead of a sphere.
    /// A glossy surface shows a streak as long as the tube's reflection. `0` (the default) is a
    /// sphere, or a point. Specular takes the point on the tube nearest the reflected ray; diffuse
    /// averages the two ends. `#[serde(default)]`.
    #[serde(default)]
    pub source_length: f32,
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
            source_radius: 0.0,
            source_length: 0.0,
        }
    }

    /// This light as a tube `metres` long along the entity's local X axis (see
    /// [`source_length`](Self::source_length)).
    pub fn with_source_length(mut self, metres: f32) -> Self {
        self.source_length = if metres.is_finite() { metres.max(0.0) } else { 0.0 };
        self
    }

    /// This light as a sphere of `metres` radius (see [`source_radius`](Self::source_radius)).
    pub fn with_source_radius(mut self, metres: f32) -> Self {
        self.source_radius = if metres.is_finite() { metres.max(0.0) } else { 0.0 };
        self
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
    /// The sun's **angular radius**, in radians — how big its disc is in the sky, and so how soft
    /// the edges of the shadows it casts: a penumbra grows by `tan(angular_radius)` for every
    /// metre between the caster and the surface the shadow falls on, so a contact stays sharp and a
    /// tall caster's distant shadow blurs. The real sun is about 0.0046; an overcast sky, a few
    /// tenths. `0` (the default) keeps the engine's fixed near-crisp edge (PCSS limited to about
    /// one shadow-map texel), which is what scenes were made with. Only the `Sun` role casts.
    /// `#[serde(default)]`.
    #[serde(default)]
    pub angular_radius: f32,
    /// **Contact shadows**, in metres: how far toward the sun each pixel looks, in screen space,
    /// for something in the way. They catch what the shadow map cannot — a step, a coin, the line
    /// where an object meets the floor — anything shorter than the shadow map's bias (about 4 cm)
    /// or finer than its texels. `0` (the default) turns them off; 0.2–0.5 suits a human-scale
    /// scene. Only the `Sun` role. `#[serde(default)]`.
    #[serde(default)]
    pub contact_shadow_length: f32,
}

impl DirectionalLight {
    /// A directional light, with a negative intensity floored at zero.
    pub fn new(color: Vec3, intensity: f32, role: LightRole) -> Self {
        let intensity = intensity.max(0.0);
        Self {
            color,
            intensity,
            role,
            angular_radius: 0.0,
            contact_shadow_length: 0.0,
        }
    }

    /// This light with contact shadows reaching `metres` toward it (see
    /// [`contact_shadow_length`](Self::contact_shadow_length)), clamped to 0..=5.
    pub fn with_contact_shadows(mut self, metres: f32) -> Self {
        self.contact_shadow_length = if metres.is_finite() { metres.clamp(0.0, 5.0) } else { 0.0 };
        self
    }

    /// This light with soft shadows from a disc of `radians` angular radius (see
    /// [`angular_radius`](Self::angular_radius)), clamped to 0..=0.5.
    pub fn with_angular_radius(mut self, radians: f32) -> Self {
        self.angular_radius = if radians.is_finite() { radians.clamp(0.0, 0.5) } else { 0.0 };
        self
    }
}

/// A cone of light from the entity's position, aimed along its `Transform`'s forward axis.
///
/// Not `Copy` since it can name a [`cookie`](Self::cookie).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Radius of the light's **source**, in metres — a bulb's size, not its reach (that is
    /// [`radius`](Self::radius)). A sphere-sourced spot's highlight on a glossy surface is as big as the
    /// sphere's reflection, not a pinpoint; `0` (the default) is the old point source. Specular
    /// uses the representative-point method (the point on the sphere nearest the reflected ray,
    /// energy-normalised), diffuse the centre. `#[serde(default)]`.
    #[serde(default)]
    pub source_radius: f32,
    /// An image projected through the cone (a cookie, or gobo): its colour multiplies the light's,
    /// so black blocks it — a window frame, leaves, a flashlight's uneven beam. The image fills the
    /// outer cone; its up is world up as seen down the cone. A name registered with
    /// `Renderer::insert_light_cookie`, or on native a path read from disk on first use. `None`
    /// (the default) is a plain cone. See `gizmo_renderer::cookies`. `#[serde(default)]`.
    #[serde(default)]
    pub cookie: Option<String>,
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
            source_radius: 0.0,
            cookie: None,
        }
    }

    /// This light projecting the cookie `name` (see [`cookie`](Self::cookie)).
    pub fn with_cookie(mut self, name: impl Into<String>) -> Self {
        self.cookie = Some(name.into());
        self
    }

    /// This light with a spherical source of `metres` radius (see
    /// [`source_radius`](Self::source_radius)).
    pub fn with_source_radius(mut self, metres: f32) -> Self {
        self.source_radius = if metres.is_finite() { metres.max(0.0) } else { 0.0 };
        self
    }
}

/// A rectangular area light — a softbox, a window, a lit panel — emitting from one face.
///
/// The rectangle is centred on the entity, `width` along its local X, `height` along its local Y,
/// and it lights the side its **forward axis** (local −Z, as for [`SpotLight`]) faces; nothing
/// behind it is lit. Diffuse is the exact cosine-weighted solid angle of the rectangle (Lambert's
/// polygon formula), so a large panel close to a wall lights it as a panel, not as a bulb at its
/// centre. Specular takes the point on the rectangle nearest the reflected ray, energy-normalised:
/// a glossy floor shows the panel's reflection, not a pinpoint.
///
/// `intensity` is the on-axis intensity — the rectangle's radiance is `intensity / area` — so a
/// panel seen face on from far away lights like a [`PointLight`] of the same intensity, and
/// dimmer off axis, as a flat emitter does. No shadows: rectangle lights are not shadow casters.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RectLight {
    /// Linear RGB colour.
    pub color: Vec3,
    /// On-axis brightness (see the type's documentation).
    pub intensity: f32,
    /// The distance the falloff reaches zero at, as for [`PointLight::radius`] — and, likewise,
    /// the culling bound.
    pub radius: f32,
    /// Extent along the entity's local X, in metres.
    pub width: f32,
    /// Extent along the entity's local Y, in metres.
    pub height: f32,
}

impl RectLight {
    /// A rectangle light, with intensity floored at zero, the radius kept above 0.001 and each
    /// extent above 1 mm (a zero-area rectangle has no radiance to speak of).
    pub fn new(color: Vec3, intensity: f32, radius: f32, width: f32, height: f32) -> Self {
        let extent = |m: f32| if m.is_finite() { m.max(0.001) } else { 0.001 };
        Self {
            color,
            intensity: intensity.max(0.0),
            radius: radius.max(0.001),
            width: extent(width),
            height: extent(height),
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
