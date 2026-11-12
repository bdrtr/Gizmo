//! **Local light shadows**: the depth maps point, spot and rectangle lights cast through.
//!
//! Every one of them lives in one depth texture array, a light taking the layers its shape needs
//! from one pool:
//!
//! - a **spot** whose cone is no wider than
//!   [`SPOT_FRUSTUM_MAX_ANGLE`](crate::local_shadows::SPOT_FRUSTUM_MAX_ANGLE) takes one layer, a
//!   perspective projection down its cone;
//! - a **point** or **rectangle** light, or a wider spot, takes six — a cube around the light, one
//!   face a layer, the face chosen in the shader by the direction from the light
//!   (`local_shadow_layer` in `common.wgsl`). A rectangle casts from its centre.
//!
//! Every layer draws its casters by both faces, and the lookup softens a light's edge by its
//! source's size (`local_shadow_visibility`, `light_source_radius` in `common.wgsl`).
//!
//! The layers go to the lights that ask for them (`casts_shadows` on the light) in the frame's rank
//! order — the order `collect_scene_lights` put them in, nearest influence first — until the pool
//! of [`MAX_LOCAL_SHADOW_LAYERS`](crate::local_shadows::MAX_LOCAL_SHADOW_LAYERS) is spent. A light
//! that does not fit lights without a shadow; one further down that does fit still gets its own.
//! Which lights cast is therefore a function of the world and the camera, the same on every frame
//! they are the same.
//!
//! Until 2026-11-11 the engine had one cube map, cast by the first point light while a
//! renderer-wide flag was set, and looked up by the deferred pass alone: spot and rectangle lights
//! cast nothing, a second point light nothing, and nothing drawn forward — glass, the editor's
//! viewport — saw any of it.
//!
//! The layers are 2-D rather than a cube array so that one lookup serves every shape: the CPU
//! builds each layer's matrix, the shadow pass draws through it and the shader projects through
//! the same one, so the three cannot disagree about a face's orientation. Each face reaches
//! [`LOCAL_SHADOW_MARGIN_TEXELS`](crate::local_shadows::LOCAL_SHADOW_MARGIN_TEXELS) past its
//! quarter of the sphere, so a filter tap beside a face's edge reads the scene rather than a
//! clamped border.

use crate::frame_uniforms::MAX_LIGHTS;
use gizmo_math::{Mat4, Vec3};

/// Texels along a side of each layer.
pub const LOCAL_SHADOW_RES: u32 = 1024;

/// How many layers one frame's lights may share: five point lights and two spots, or thirty-two
/// spots. Each is [`LOCAL_SHADOW_RES`]² of 32-bit depth, 4 MiB, allocated only once a frame uses
/// it ([`crate::pipeline::SceneState::ensure_local_shadow_layers`]).
pub const MAX_LOCAL_SHADOW_LAYERS: usize = 32;

/// The near plane of every layer, metres. A caster nearer its light than this is not in its
/// shadow — a lamp's own shade, typically, which is better off not casting at all.
pub const LOCAL_SHADOW_NEAR: f32 = 0.05;

/// The widest cone, as a half-angle, that a spot casts through a single layer: 80°, where Unreal caps
/// a spot's cone too. One projection spreads its texels thinner over the cone's middle the wider
/// it is — a texel there is `tan(angle)` times a cube face's, 1.7× at 60° and 5.7× at 80° — and
/// past 80° that runs away (57× at 89°), so a wider spot casts through six layers instead. A
/// floodlight that wide is soft-edged anyway; six passes for every one of them would not be.
pub const SPOT_FRUSTUM_MAX_ANGLE: f32 = 80.0 * std::f32::consts::PI / 180.0;

/// Texels each layer reaches past the region it is looked up for: the lookup's normal offset
/// moves its point up to two texels and its filter reads one more on each side.
pub const LOCAL_SHADOW_MARGIN_TEXELS: f32 = 4.0;

/// Set on a light's entry ([`LocalShadows::entry`]) when its shadow is a cube of six layers.
pub const LOCAL_SHADOW_CUBE: u32 = 1 << 31;

/// The bits of a light's entry that hold its first layer + 1 (0: it casts none).
pub const LOCAL_SHADOW_FIRST_MASK: u32 = 0xFF;

/// Where a light's contact-shadow length sits in its entry: millimetres, in bits 8 to 23 (up to
/// 65.5 m; the lights clamp it to 5). See [`LocalShadows::with_contact_shadows`].
pub const CONTACT_SHADOW_SHIFT: u32 = 8;

/// The faces of a light's cube, in layer order: +X, −X, +Y, −Y, +Z, −Z. `local_shadow_layer` in
/// `common.wgsl` numbers them the same way; `up` is any axis square to the face's.
const CUBE_FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// What a light that asks for a shadow casts it over.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum LocalShadowCaster {
    /// A cone from `position` along `direction` (unit), `outer_angle` the half-angle where its
    /// light reaches zero, out to `radius`.
    Spot {
        /// Where the cone starts.
        position: Vec3,
        /// Which way it points, unit length.
        direction: Vec3,
        /// Its half-angle, radians.
        outer_angle: f32,
        /// How far it reaches, metres.
        radius: f32,
    },
    /// Every direction around `position`, out to `radius` — a point light, or a rectangle light from
    /// its centre.
    Omni {
        /// The light's centre.
        position: Vec3,
        /// How far it reaches, metres.
        radius: f32,
    },
}

/// One layer of the local shadow array: the projection it is drawn and looked up through.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LocalShadowLayer {
    /// World → the layer's clip space.
    pub view_proj: Mat4,
    /// The near plane, metres ([`LOCAL_SHADOW_NEAR`]).
    pub near: f32,
    /// The far plane, metres: the light's reach.
    pub far: f32,
    /// The width of one texel per metre of depth along the layer's axis — what the lookup's normal
    /// offset is measured in.
    pub texel_per_metre: f32,
}

impl Default for LocalShadowLayer {
    fn default() -> Self {
        Self { view_proj: Mat4::IDENTITY, near: LOCAL_SHADOW_NEAR, far: 1.0, texel_per_metre: 0.0 }
    }
}

impl LocalShadowLayer {
    /// A layer at `position` looking along `forward`, `half_extent` the tangent of the half-angle
    /// it must cover; the margin is added here.
    fn new(position: Vec3, forward: Vec3, up: Vec3, half_extent: f32, radius: f32) -> Self {
        let res = LOCAL_SHADOW_RES as f32;
        let t = half_extent * res / (res - 2.0 * LOCAL_SHADOW_MARGIN_TEXELS);
        let near = LOCAL_SHADOW_NEAR;
        let far = radius.max(2.0 * near);
        let proj = Mat4::perspective_rh(2.0 * t.atan(), 1.0, near, far);
        Self {
            view_proj: proj * Mat4::look_to_rh(position, forward, up),
            near,
            far,
            texel_per_metre: 2.0 * t / res,
        }
    }
}

/// One frame's local shadows: the layers it draws and which light each belongs to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalShadows {
    layers: [LocalShadowLayer; MAX_LOCAL_SHADOW_LAYERS],
    layer_count: usize,
    /// Per light slot: its first layer + 1 in the low byte (0 = casts none), with
    /// [`LOCAL_SHADOW_CUBE`] for six, and its contact-shadow length in millimetres above
    /// ([`CONTACT_SHADOW_SHIFT`]).
    entries: [u32; MAX_LIGHTS],
}

impl Default for LocalShadows {
    /// No light casts.
    fn default() -> Self {
        Self { layers: [LocalShadowLayer::default(); MAX_LOCAL_SHADOW_LAYERS], layer_count: 0, entries: [0; MAX_LIGHTS] }
    }
}

impl LocalShadows {
    /// Lays the layers out for `casters` — `(light slot, what it casts over)` in rank order, best
    /// first — giving each the layers its shape needs while they last. A caster with a non-finite
    /// position or reach, a direction that is not one, a cone that is not one (no wider than 0.0001
    /// rad, or not a number: it lights nothing), or a slot past [`MAX_LIGHTS`] is skipped.
    #[must_use]
    pub fn plan(casters: impl IntoIterator<Item = (usize, LocalShadowCaster)>) -> Self {
        let mut out = Self::default();
        for (slot, caster) in casters {
            if slot >= MAX_LIGHTS || out.entries[slot] & LOCAL_SHADOW_FIRST_MASK != 0 {
                continue;
            }
            let free = MAX_LOCAL_SHADOW_LAYERS - out.layer_count;
            let first = out.layer_count;
            match caster {
                LocalShadowCaster::Spot { outer_angle, .. } if outer_angle.is_nan() || outer_angle <= 1e-4 => continue,
                LocalShadowCaster::Spot { position, direction, outer_angle, radius }
                    if outer_angle <= SPOT_FRUSTUM_MAX_ANGLE =>
                {
                    let direction = direction.normalize_or_zero();
                    if free < 1 || !position.is_finite() || direction == Vec3::ZERO || !radius.is_finite() {
                        continue;
                    }
                    // Any axis square to the cone's will do; this one only has to not be parallel.
                    let up = if direction.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
                    out.layers[first] =
                        LocalShadowLayer::new(position, direction, up, outer_angle.tan(), radius);
                    out.layer_count += 1;
                    out.entries[slot] = first as u32 + 1;
                }
                LocalShadowCaster::Spot { position, radius, .. } | LocalShadowCaster::Omni { position, radius } => {
                    if free < CUBE_FACES.len() || !position.is_finite() || !radius.is_finite() {
                        continue;
                    }
                    for (face, (forward, up)) in CUBE_FACES.iter().enumerate() {
                        out.layers[first + face] = LocalShadowLayer::new(position, *forward, *up, 1.0, radius);
                    }
                    out.layer_count += CUBE_FACES.len();
                    out.entries[slot] = (first as u32 + 1) | LOCAL_SHADOW_CUBE;
                }
            }
        }
        out
    }

    /// This frame's shadows with each light in `lights` — `(light slot, metres)` — looking that far
    /// toward its light for a contact shadow, whether it casts a shadow map or not: millimetres in
    /// bits 8 to 23 of its entry (`light_contact_length` in `common.wgsl` reads them back). A slot past
    /// [`MAX_LIGHTS`], or a length that is not a positive number, is left as it is.
    #[must_use]
    pub fn with_contact_shadows(mut self, lights: impl IntoIterator<Item = (usize, f32)>) -> Self {
        for (slot, metres) in lights {
            let Some(entry) = self.entries.get_mut(slot) else { continue };
            if metres.is_nan() || metres <= 0.0 || metres.is_infinite() {
                continue;
            }
            let millimetres = (metres * 1000.0).round().min(65_535.0) as u32;
            *entry = (*entry & !(0xFFFF << CONTACT_SHADOW_SHIFT)) | (millimetres << CONTACT_SHADOW_SHIFT);
        }
        self
    }

    /// The layers this frame draws, in array order.
    #[must_use]
    pub fn layers(&self) -> &[LocalShadowLayer] {
        &self.layers[..self.layer_count]
    }

    /// Light slot `light`'s entry: its first layer + 1 in the low byte (0 when it casts none), with
    /// [`LOCAL_SHADOW_CUBE`] set when the shadow is six layers, and its contact-shadow length in
    /// millimetres from [`CONTACT_SHADOW_SHIFT`].
    #[must_use]
    pub fn entry(&self, light: usize) -> u32 {
        self.entries.get(light).copied().unwrap_or(0)
    }

    /// Every light slot's entry, as [`entry`](Self::entry) gives them.
    #[must_use]
    pub fn entries(&self) -> &[u32; MAX_LIGHTS] {
        &self.entries
    }
}

/// The local shadow array on the GPU: the texture, a view of every layer for the lookups, and one
/// view a layer for the passes that draw them.
///
/// It holds as many layers as the most a frame has asked for — one until a light casts — and grows
/// when a frame asks for more ([`crate::pipeline::SceneState::ensure_local_shadow_layers`]). It does
/// not shrink: a scene that once had four shadowed point lights is likely to have them again.
#[non_exhaustive]
pub struct LocalShadowMaps {
    /// The depth array itself.
    pub texture: wgpu::Texture,
    /// Every layer, for sampling — the shadow group's binding 2.
    pub view: wgpu::TextureView,
    /// One view a layer, for rendering into.
    pub layer_views: Vec<wgpu::TextureView>,
}

impl LocalShadowMaps {
    /// An array of `layers` layers (at least one, so there is always something to bind). A texel a
    /// layer on the forward-only path, which draws no shadow maps (see `build_shadow_resources`).
    #[must_use]
    pub fn new(device: &wgpu::Device, layers: u32) -> Self {
        let layers = layers.max(1);
        let res = if cfg!(gizmo_forward_only) { 1 } else { LOCAL_SHADOW_RES };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("local_shadow_layers"),
            size: wgpu::Extent3d { width: res, height: res, depth_or_array_layers: layers },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("local_shadow_layers_array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let layer_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("local_shadow_layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    aspect: wgpu::TextureAspect::DepthOnly,
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        Self { texture, view, layer_views }
    }

    /// How many layers it holds.
    #[must_use]
    pub fn layers(&self) -> usize {
        self.layer_views.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(angle_deg: f32) -> LocalShadowCaster {
        LocalShadowCaster::Spot {
            position: Vec3::new(1.0, 4.0, -2.0),
            direction: Vec3::NEG_Y,
            outer_angle: angle_deg.to_radians(),
            radius: 12.0,
        }
    }

    fn omni() -> LocalShadowCaster {
        LocalShadowCaster::Omni { position: Vec3::new(0.0, 3.0, 0.0), radius: 10.0 }
    }

    /// Where `p` lands in `layer`: uv in 0..1 across the layer, and NDC depth.
    fn project(layer: &LocalShadowLayer, p: Vec3) -> (f32, f32, f32) {
        let c = layer.view_proj * p.extend(1.0);
        let n = c.truncate() / c.w;
        (n.x * 0.5 + 0.5, 0.5 - n.y * 0.5, n.z)
    }

    /// The face `local_shadow_layer` (`common.wgsl`) picks for a point `d` from the light — its rule,
    /// written out to check the CPU's faces against.
    fn shader_face(d: Vec3) -> usize {
        let a = d.abs();
        if a.x >= a.y && a.x >= a.z {
            if d.x >= 0.0 { 0 } else { 1 }
        } else if a.y >= a.z {
            if d.y >= 0.0 { 2 } else { 3 }
        } else if d.z >= 0.0 {
            4
        } else {
            5
        }
    }

    /// A spot within the single-layer limit takes one layer; a point light, and a spot past it, six.
    #[test]
    fn each_shape_takes_the_layers_it_needs() {
        let plan = LocalShadows::plan([(0, spot(40.0)), (1, omni()), (2, spot(85.0)), (3, spot(79.0))]);
        assert_eq!(plan.layers().len(), 1 + 6 + 6 + 1);
        assert_eq!(plan.entry(0), 1, "the spot's one layer is layer 0");
        assert_eq!(plan.entry(1), 2 | LOCAL_SHADOW_CUBE, "the point light's cube starts at layer 1");
        assert_eq!(plan.entry(2), 8 | LOCAL_SHADOW_CUBE, "a spot past 80° casts through a cube too");
        assert_eq!(plan.entry(3), 14, "one just inside it through one layer");
        assert_eq!(plan.entry(4), 0, "a light that asked for nothing casts nothing");
    }

    /// The pool is shared in rank order: a light that does not fit is skipped, and a smaller one
    /// further down still gets its layer.
    #[test]
    fn layers_go_in_rank_order_until_they_run_out() {
        let mut casters: Vec<(usize, LocalShadowCaster)> = (0..5).map(|i| (i, omni())).collect();
        casters.push((5, omni()));
        casters.push((6, spot(30.0)));
        casters.push((7, spot(30.0)));
        casters.push((8, spot(30.0)));
        let plan = LocalShadows::plan(casters);
        for i in 0..5 {
            assert_eq!(plan.entry(i), (6 * i as u32 + 1) | LOCAL_SHADOW_CUBE);
        }
        assert_eq!(plan.entry(5), 0, "a sixth cube does not fit in the two layers left");
        assert_eq!(plan.entry(6), 31, "the spot after it takes layer 30");
        assert_eq!(plan.entry(7), 32, "and the next one layer 31");
        assert_eq!(plan.entry(8), 0, "and then there are none");
        assert_eq!(plan.layers().len(), MAX_LOCAL_SHADOW_LAYERS);
    }

    /// A caster that cannot be projected is skipped rather than given a layer of NaN.
    #[test]
    fn a_caster_that_cannot_be_projected_casts_nothing() {
        let bad = [
            (0, LocalShadowCaster::Omni { position: Vec3::NAN, radius: 5.0 }),
            (1, LocalShadowCaster::Spot { position: Vec3::ZERO, direction: Vec3::ZERO, outer_angle: 0.5, radius: 5.0 }),
            (2, LocalShadowCaster::Spot { position: Vec3::ZERO, direction: Vec3::Y, outer_angle: 0.5, radius: f32::INFINITY }),
            (3, LocalShadowCaster::Spot { position: Vec3::ZERO, direction: Vec3::Y, outer_angle: 0.0, radius: 5.0 }),
            (4, LocalShadowCaster::Spot { position: Vec3::ZERO, direction: Vec3::Y, outer_angle: f32::NAN, radius: 5.0 }),
            (MAX_LIGHTS, omni()),
        ];
        let plan = LocalShadows::plan(bad);
        assert!(plan.layers().is_empty());
        assert!(plan.entries().iter().all(|&e| e == 0));
    }

    /// A spot's layer holds its whole cone: the axis lands in the middle, the cone's edge inside
    /// the layer by the margin, and depth grows with distance from the light.
    #[test]
    fn a_spot_layer_holds_its_cone() {
        let angle = 40.0_f32.to_radians();
        let plan = LocalShadows::plan([(0, spot(40.0))]);
        let layer = plan.layers()[0];
        let apex = Vec3::new(1.0, 4.0, -2.0);
        let (u, v, near_z) = project(&layer, apex + Vec3::NEG_Y * 2.0);
        assert!((u - 0.5).abs() < 1e-5 && (v - 0.5).abs() < 1e-5, "the axis lands at ({u}, {v})");
        let (_, _, far_z) = project(&layer, apex + Vec3::NEG_Y * 8.0);
        assert!(0.0 < near_z && near_z < far_z && far_z < 1.0, "depths {near_z} then {far_z}");
        // On the cone's edge, 3 m out: inside, the margin short of the border.
        let edge = apex + 3.0 * (Vec3::NEG_Y * angle.cos() + Vec3::X * angle.sin());
        let (u, v, _) = project(&layer, edge);
        let reach = (u - 0.5).abs().max((v - 0.5).abs());
        let margin = LOCAL_SHADOW_MARGIN_TEXELS / LOCAL_SHADOW_RES as f32;
        assert!((reach - (0.5 - margin)).abs() < margin * 0.01, "the cone's edge lands at ({u}, {v})");
    }

    /// For every direction around a point light, the face the shader picks is a layer that holds
    /// it: inside, by the margin, and in front of the light.
    #[test]
    fn the_face_the_shader_picks_holds_the_point() {
        let plan = LocalShadows::plan([(0, omni())]);
        let centre = Vec3::new(0.0, 3.0, 0.0);
        let margin = LOCAL_SHADOW_MARGIN_TEXELS / LOCAL_SHADOW_RES as f32 * 0.99;
        let mut seen = [false; 6];
        // A spiral over the sphere, edges and corners of the cube included.
        let n = 4000;
        for i in 0..n {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - y * y).sqrt();
            let phi = i as f32 * 2.399_963;
            let d = Vec3::new(r * phi.cos(), y, r * phi.sin()) * 4.0;
            let face = shader_face(d);
            seen[face] = true;
            let (u, v, z) = project(&plan.layers()[face], centre + d);
            assert!(
                (margin..=1.0 - margin).contains(&u) && (margin..=1.0 - margin).contains(&v) && (0.0..1.0).contains(&z),
                "direction {d} picks face {face}, where it lands at ({u}, {v}, {z})"
            );
        }
        assert!(seen.iter().all(|&s| s), "every face was reached: {seen:?}");
    }

    /// The texel width the lookup offsets by is what a texel actually covers: at 5 m along a cube
    /// face's axis, two points one texel width apart land one texel apart.
    #[test]
    fn a_layers_texel_width_is_one_texel() {
        let plan = LocalShadows::plan([(0, omni())]);
        let layer = plan.layers()[0]; // +X
        let centre = Vec3::new(0.0, 3.0, 0.0);
        let width = layer.texel_per_metre * 5.0;
        let (a, _, _) = project(&layer, centre + Vec3::new(5.0, 0.0, 0.0));
        let (b, _, _) = project(&layer, centre + Vec3::new(5.0, 0.0, width));
        let texels = (a - b).abs() * LOCAL_SHADOW_RES as f32;
        assert!((texels - 1.0).abs() < 1e-3, "one texel width moved {texels} texels");
    }

    /// A light's contact-shadow length rides its entry above its layers, and leaves them as they are;
    /// a light that casts no map gets one too.
    #[test]
    fn a_contact_shadow_length_rides_the_entry_above_the_layers() {
        let plan = LocalShadows::plan([(1, LocalShadowCaster::Omni { position: Vec3::ZERO, radius: 5.0 })])
            .with_contact_shadows([(1, 0.25), (3, 1.5), (4, -1.0), (5, f32::NAN), (MAX_LIGHTS, 1.0)]);
        assert_eq!(plan.entry(1) & LOCAL_SHADOW_FIRST_MASK, 1, "the cube's first layer, as it was");
        assert_ne!(plan.entry(1) & LOCAL_SHADOW_CUBE, 0, "still a cube");
        assert_eq!((plan.entry(1) >> CONTACT_SHADOW_SHIFT) & 0xFFFF, 250, "25 cm in millimetres");
        assert_eq!(plan.entry(3), 1500 << CONTACT_SHADOW_SHIFT, "a light with no map: no layers, a length");
        assert_eq!(plan.entry(4), 0, "a negative length is none");
        assert_eq!(plan.entry(5), 0, "and not a number is none");
    }

}
