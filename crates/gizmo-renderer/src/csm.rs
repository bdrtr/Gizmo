//! Cascaded shadow maps (CSM) for directional lights: split the camera depth range into
//! several cascades, each rendered to a layer of a `depth2d_array` with its own light
//! orthographic projection (tighter texel density near the camera).

use gizmo_math::{Mat4, Vec3};

/// Must match `texture_depth_2d_array` layer count and `SceneUniforms.light_view_proj` length.
pub const CASCADE_COUNT: usize = 4;

/// Resolution (width = height) of each cascade depth map. 3072 (was 2048) so a
/// crisp ~1-texel PCF edge doesn't read as blocky on close-up geometry; the extra
/// VRAM (4 × 3072² × Depth32 ≈ 302 MB) is acceptable on a modern GPU.
pub const SHADOW_MAP_RES: u32 = 3072;

/// Maximum world distance the cascades cover, independent of the camera's far plane.
///
/// The camera far plane is often huge (e.g. 1500) so the sky/horizon isn't clipped,
/// but shadows only matter near the viewer. Feeding `cam_far` straight into the
/// cascade split would stretch cascade 0 across ~95 units for a far=1500 camera, so
/// a nearby object gets a handful of shadow texels and its shadow reads blocky and
/// blurry. Capping the shadow range packs the cascades onto what's actually near the
/// camera, giving crisp contact shadows. Fragments past this distance are unshadowed.
pub const SHADOW_DISTANCE: f32 = 100.0;

/// Blend between logarithmic (1.0) and uniform (0.0) cascade splits. 0.75 leans
/// logarithmic for denser near-camera texels while keeping the far cascade sane.
/// Single-sourced so the game and studio renderers can't pick different values.
pub const CASCADE_LAMBDA: f32 = 0.75;

/// Width of the band, as a fraction of the covered shadow range, over which the sampled
/// shadow term is faded back to "fully lit".
///
/// Without a fade the shadow term steps at the edge of the last cascade: inside it a fully
/// occluded fragment keeps whatever floor the shading applies (the baked-lit path floors it
/// at `1 - sun_share` = 0.55), one metre further out there is no cascade to sample so the
/// term is 1.0 — a 1/0.55 = 1.82x brightness jump along a line across the world. Fading
/// instead spends the last 15% of the range (15 m at the default [`SHADOW_DISTANCE`])
/// walking the term to 1.0, so the discontinuity becomes a gradient no edge detector — or
/// eye — can pick out.
///
/// The alternative fix is to push [`SHADOW_DISTANCE`] out past anything the camera can see.
/// That costs texel density everywhere: the same 4 x [`SHADOW_MAP_RES`]² texels would be
/// spread over a longer range, which is exactly the blocky near-camera shadow the
/// `SHADOW_DISTANCE` cap exists to prevent. The fade costs one `smoothstep` + one `mix` per
/// shadowed fragment and no memory.
pub const SHADOW_FADE_FRACTION: f32 = 0.15;

/// How far **above** a cascade's own slice, along the light, a shadow caster may sit and still be
/// rendered into that cascade's map, in metres.
///
/// It used to be 60, and 60 is not much. Measured against a caster rising above the middle of each
/// slice, the height at which it stopped casting was **65 m with the sun 75° up**, 90 m at 45°, and
/// 188 m at a low 20° — so a tall building simply loses its shadow as the sun climbs, which is
/// exactly backwards from what a player would expect. Nothing errors; the caster is clipped by the
/// shadow projection's near plane and never reaches the map.
///
/// **Raising it is close to free here and would not be under a perspective projection.** The
/// cascade projection is orthographic, so shadow-map depth is linear: `Depth32Float` over a
/// kilometre still resolves a tenth of a millimetre. The one real cost is that a depth bias
/// expressed in NDC units becomes a larger *world* distance as the range grows — at the old range
/// the shader's `0.0004` was 4.2 cm, and at this reach it would have been 22 cm of peter-panning.
/// That is why the shaders now express the bias in metres and convert it with the cascade's own z
/// gradient; this constant and that change belong together.
pub const CASTER_REACH: f32 = 500.0;

/// The matching margin on the far side of the slice, in metres — for receivers just past it rather
/// than casters in front of it, which is why it is much smaller and can afford to be.
pub const RECEIVER_MARGIN: f32 = 40.0;

/// How much of the sampled shadow term survives at `view_depth` (distance along the camera
/// forward axis, the same measure [`cascade_split_distances`] is expressed in).
///
/// `1.0` = use the cascade's sampled value verbatim; `0.0` = fully lit, which is what the
/// shaders fall back to anyway once a fragment projects outside the last cascade. Shaders
/// apply it as `mix(1.0, sampled, fade)`.
///
/// This is the CPU mirror of `shadow_distance_fade` in `shaders/baked_lit.wgsl` and
/// `shaders/deferred_lighting.wgsl` — the maths lives here so it can be tested without a
/// GPU, and `shader_shadow_fade_matches_the_rust_mirror` pins the shader copies to the same
/// constant.
pub fn shadow_distance_fade(view_depth: f32, shadow_far: f32) -> f32 {
    let far = shadow_far.max(1e-4);
    let band = (far * SHADOW_FADE_FRACTION).max(1e-4);
    let t = ((view_depth - (far - band)) / band).clamp(0.0, 1.0);
    // smoothstep(far - band, far, view_depth), inverted.
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The directional shadow cascades for one frame: the split distances and the
/// per-cascade light clip matrices, ready to upload.
pub struct ShadowCascades {
    /// The far distance of each cascade, along the camera's forward axis.
    pub splits: [f32; CASCADE_COUNT],
    /// World → light clip space for each cascade, in the same order as the shadow array's layers.
    pub view_projs: [Mat4; CASCADE_COUNT],
}

/// Compute the directional shadow cascades for a camera + light direction.
///
/// Wraps the shared cascade math (`SHADOW_DISTANCE` cap, [`CASCADE_LAMBDA`],
/// [`cascade_split_distances`], [`directional_cascade_view_projs`]) that the game
/// and studio render paths both need. The CALLER picks `light_dir` — the game
/// always uses the sun, the studio falls back to a point light when there's no
/// sun — so that legitimate difference stays at the call site while the
/// orchestration lives here once.
pub fn compute_directional_cascades(
    cam_pos: Vec3,
    cam_forward: Vec3,
    aspect: f32,
    fov_y: f32,
    cam_near: f32,
    cam_far: f32,
    light_dir: Vec3,
) -> ShadowCascades {
    let shadow_far = cam_far.min(SHADOW_DISTANCE);
    let splits = cascade_split_distances(cam_near, shadow_far, CASCADE_LAMBDA);
    let view_projs = directional_cascade_view_projs(
        cam_pos,
        cam_forward,
        aspect,
        fov_y,
        cam_near,
        &splits,
        light_dir,
        SHADOW_MAP_RES,
    );
    ShadowCascades { splits, view_projs }
}

/// Logarithmic-linear split distances in **world units** along `cam_forward` from `cam_pos`.
/// `splits[i]` is the far distance of cascade `i` (inclusive range `[prev, splits[i]]`).
pub fn cascade_split_distances(z_near: f32, z_far: f32, lambda: f32) -> [f32; CASCADE_COUNT] {
    let mut s = [0.0f32; CASCADE_COUNT];
    let z_near = z_near.max(0.001);
    let z_far = z_far.max(z_near + 0.001);
    let n = CASCADE_COUNT as f32;
    for (i, s) in s.iter_mut().enumerate() {
        let p = (i + 1) as f32 / n;
        let log_d = z_near * (z_far / z_near).powf(p);
        let uni_d = z_near + (z_far - z_near) * p;
        *s = lambda * log_d + (1.0 - lambda) * uni_d;
    }
    s[CASCADE_COUNT - 1] = z_far;
    s
}

fn camera_right_up(forward: Vec3) -> (Vec3, Vec3) {
    let forward = forward.normalize();
    let mut right = forward.cross(Vec3::Y);
    if right.length_squared() < 1e-10 {
        right = forward.cross(Vec3::X);
    }
    right = right.normalize();
    let up = right.cross(forward).normalize();
    (right, up)
}

fn frustum_slice_corners(
    cam_pos: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    aspect: f32,
    fov_y: f32,
    zn: f32,
    zf: f32,
) -> [Vec3; 8] {
    let th = (fov_y * 0.5).tan();
    let corners_2d = [(-1f32, -1f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];
    let mut out = [Vec3::ZERO; 8];
    let mut k = 0;
    for &(sx, sy) in &corners_2d {
        for &d in &[zn, zf] {
            let hh = d * th;
            let hw = hh * aspect;
            out[k] = cam_pos + forward * d + right * (sx * hw) + up * (sy * hh);
            k += 1;
        }
    }
    out
}

/// Builds one orthographic light clip matrix per cascade: `clip = ortho * light_view * world`.
pub fn directional_cascade_view_projs(
    cam_pos: Vec3,
    cam_forward: Vec3,
    aspect: f32,
    fov_y: f32,
    z_near: f32,
    splits: &[f32; CASCADE_COUNT],
    light_dir_world: Vec3,
    shadow_map_size: u32,
) -> [Mat4; CASCADE_COUNT] {
    let light_dir = light_dir_world.normalize();
    let (right, up) = camera_right_up(cam_forward);
    // The light basis needs an up vector that is not parallel to it. `Vec3::Y` unconditionally
    // meant a sun straight overhead — noon, the most ordinary configuration there is — produced a
    // degenerate `look_at_rh` and a cascade matrix of **NaN** end to end, so every shadow lookup
    // that frame sampled nothing. Measured, not guessed: `an_overhead_sun_does_not_produce_nan`.
    let light_up = if light_dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    // **One light basis for every cascade, fixed in the world.** It used to be rebuilt per cascade
    // around `slice_center`, which follows the camera — and a snap grid expressed in a space whose
    // origin moves with the camera moves with the camera too, which is most of why the snapping
    // below never worked. Only the rotation matters for that grid; the translation is arbitrary.
    let light_view = Mat4::look_at_rh(-light_dir, Vec3::ZERO, light_up);
    let mut prev_z = z_near;
    let mut mats = [Mat4::IDENTITY; CASCADE_COUNT];

    for i in 0..CASCADE_COUNT {
        let zf = splits[i];
        let corners =
            frustum_slice_corners(cam_pos, cam_forward, right, up, aspect, fov_y, prev_z, zf);

        // **Bounding sphere, not a bounding box.** The extent of the ortho box is the texel size,
        // and the texel size has to be constant or snapping to it is meaningless. An AABB of the
        // slice corners *in light space* changes shape as the camera rotates, so the texel size
        // changed every frame and the grid moved out from under the snap. A sphere's radius does
        // not depend on how the camera is turned.
        //
        // **It costs resolution and the figure is worth knowing.** A sphere circumscribing a
        // frustum slice is bigger than the slice's own box, so texels grow — measured at the
        // default settings, **1.44-1.52x** across the four cascades:
        //
        //     cascade 0   0.1- 6.7 m    4.3 mm/texel   (was 2.8)
        //     cascade 1   6.7-14.9 m    8.6 mm         (was 6.0)
        //     cascade 2  14.9-32.1 m   18.5 mm         (was 12.9)
        //     cascade 3  32.1-  100 m   59.3 mm        (was 40.8)
        //
        // That is the textbook trade and it is the right way round: a crawling shadow edge is far
        // more visible than a texel half again as wide, and the absolute figures stay fine. Do not
        // "recover" it by going back to the box — the box is what made the snapping decorative.
        let center = corners.iter().copied().fold(Vec3::ZERO, |a, c| a + c) / corners.len() as f32;
        let radius = corners
            .iter()
            .fold(0.0f32, |m, c| m.max((*c - center).length()));
        // Quantised so that float wobble in the corner set cannot nudge the texel size either.
        let radius = (radius * 16.0).ceil() / 16.0;

        let center_ls = light_view.transform_point3(center);
        let texel = 2.0 * radius / shadow_map_size as f32;

        // Now the snap does what it is for: the box is a fixed size on a fixed grid, so a static
        // world point keeps the same sub-texel phase while the camera moves, and shadow edges stop
        // crawling. `a_static_point_keeps_its_sub_texel_phase` measures exactly that.
        let min_x = ((center_ls.x - radius) / texel).floor() * texel;
        let min_y = ((center_ls.y - radius) / texel).floor() * texel;
        let (max_x, max_y) = (min_x + 2.0 * radius, min_y + 2.0 * radius);

        // Depth range from the same corners, in the same space. The margins keep casters that sit
        // behind the slice (and receivers just past it) inside the map.
        let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
        for c in corners {
            let p = light_view.transform_point3(c);
            min_z = min_z.min(p.z);
            max_z = max_z.max(p.z);
        }
        min_z -= RECEIVER_MARGIN;
        max_z += CASTER_REACH;

        let ortho = Mat4::orthographic_rh(min_x, max_x, min_y, max_y, -max_z, -min_z);
        mats[i] = ortho * light_view;
        prev_z = zf;
    }
    mats
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tall caster still casts, at any sun angle.
    ///
    /// It did not. With [`CASTER_REACH`] at its old 60 m, a caster rising above the middle of a
    /// slice stopped being rendered into the cascade at **65 m with the sun 75° up**, 90 m at 45°
    /// and 188 m at a low 20° — so a building lost its shadow as the sun climbed, which is the
    /// opposite of what anyone would expect. It is a clip, not an error: the caster falls outside
    /// the shadow projection's near plane and never reaches the map.
    ///
    /// The bar is 300 m, well inside the 500 m reach and well past any plausible building, and the
    /// steep sun is the case that matters because it is the one that used to fail first.
    #[test]
    fn a_tall_caster_still_casts_at_any_sun_angle() {
        use gizmo_math::Vec4;
        let splits = cascade_split_distances(0.1, SHADOW_DISTANCE, CASCADE_LAMBDA);
        for (name, light) in [
            ("alçak güneş 20°", Vec3::new(-0.94, -0.34, 0.0).normalize()),
            ("orta 45°", Vec3::new(-0.7, -0.7, 0.0).normalize()),
            ("tepeye yakın 75°", Vec3::new(-0.26, -0.97, 0.0).normalize()),
        ] {
            let (cam, fwd) = (Vec3::new(0.0, 2.0, 0.0), Vec3::X);
            let m = directional_cascade_view_projs(
                cam, fwd, 16.0 / 9.0, 0.785, 0.1, &splits, light, SHADOW_MAP_RES,
            );
            // Dilimin ortasındaki bir noktanın üstünde yükselen bir yayıcı.
            let mut limits = Vec::new();
            for ci in 0..CASCADE_COUNT {
                let d = if ci == 0 { splits[0] * 0.5 } else { (splits[ci - 1] + splits[ci]) * 0.5 };
                let base = cam + fwd * d;
                let mut last_ok = 0.0f32;
                for h_i in 0..400 {
                    let h = h_i as f32 * 1.0;
                    let p = Vec3::new(base.x, base.y + h, base.z);
                    let c = m[ci] * Vec4::new(p.x, p.y, p.z, 1.0);
                    let z = c.z / c.w;
                    if (0.0..=1.0).contains(&z) { last_ok = h; } else { break; }
                }
                limits.push(last_ok);
            }
            for (ci, limit) in limits.iter().enumerate() {
                assert!(
                    *limit >= 300.0,
                    "{name}: a caster above cascade {ci} stops casting at {limit} m — it is being \
                     clipped by the shadow projection's near plane, so tall geometry silently \
                     loses its shadow"
                );
            }
        }
    }

    /// Every point the camera can see inside the shadow distance is covered by the cascade that
    /// `select_cascade` picks for it.
    ///
    /// The two halves — which cascade a depth selects, and what that cascade's box contains — are
    /// computed in different places and in different languages: the split comparison is in
    /// `deferred_lighting.wgsl`, the fit is here. Nothing but a test keeps them agreeing, and a
    /// fragment that falls outside its cascade does not error — the shader's UV bounds check
    /// leaves it at "fully lit", so the failure looks like shadows quietly missing in a band of
    /// the view rather than like anything going wrong.
    ///
    /// Sampled over camera positions and orientations, depths across the whole range, and the
    /// frustum's corners as well as its centre. Measured at 0 of 7800 when the bounding-sphere fit
    /// landed, which is also what says the one-texel overhang the snap can leave at the far edge
    /// of the box is not reachable in practice.
    #[test]
    fn every_visible_point_lands_in_its_own_cascade() {
        use gizmo_math::Vec4;
        let light = Vec3::new(-0.4, -1.0, -0.3).normalize();
        let (aspect, fov) = (16.0f32 / 9.0, 0.785f32);
        let splits = cascade_split_distances(0.1, SHADOW_DISTANCE, CASCADE_LAMBDA);
        let mut outside = 0u32;
        let mut total = 0u32;
        // Kameranın gördüğü hacmi tara: derinlik, yatay ve dikey açı boyunca.
        for step in 0..40 {
            let a = step as f32 * 0.05;
            let (cam, fwd) = (Vec3::new(a, 2.0, 0.0), Vec3::new(a.cos(), 0.0, a.sin()));
            let (right, up) = camera_right_up(fwd);
            let m = directional_cascade_view_projs(
                cam, fwd, aspect, fov, 0.1, &splits, light, SHADOW_MAP_RES,
            );
            for d_i in 1..40 {
                let d = 0.1 + (SHADOW_DISTANCE - 0.1) * (d_i as f32 / 40.0);
                let ci = splits.iter().position(|&s| d < s).unwrap_or(CASCADE_COUNT - 1);
                let hh = d * (fov * 0.5).tan();
                let hw = hh * aspect;
                for &(sx, sy) in &[(0.0f32, 0.0f32), (0.98, 0.98), (-0.98, 0.98), (0.98, -0.98), (-0.98, -0.98)] {
                    let p = cam + fwd * d + right * (sx * hw) + up * (sy * hh);
                    let c = m[ci] * Vec4::new(p.x, p.y, p.z, 1.0);
                    let uv = ((c.x / c.w) * 0.5 + 0.5, (c.y / c.w) * -0.5 + 0.5);
                    total += 1;
                    if !(0.0..=1.0).contains(&uv.0) || !(0.0..=1.0).contains(&uv.1) {
                        outside += 1;
                    }
                }
            }
        }
        assert_eq!(
            outside, 0,
            "{outside} of {total} visible points fell outside the cascade selected for them — \
             those fragments read as fully lit, so shadows go missing in a band of the view"
        );
    }

    /// A sun straight overhead must not produce a NaN cascade matrix.
    ///
    /// It did. `Mat4::look_at_rh` was handed `Vec3::Y` as its up vector unconditionally, so a
    /// light direction of `(0, -1, 0)` — noon, the most ordinary lighting there is — made the
    /// basis degenerate and every one of the sixteen matrix entries NaN. Nothing crashed; the
    /// shadow lookup simply sampled a NaN UV and the shadow term became undefined.
    #[test]
    fn an_overhead_sun_does_not_produce_nan() {
        let splits = cascade_split_distances(0.1, SHADOW_DISTANCE, CASCADE_LAMBDA);
        for light in [
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.001, -1.0, 0.0).normalize(),
        ] {
            let mats = directional_cascade_view_projs(
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::X,
                16.0 / 9.0,
                0.785,
                0.1,
                &splits,
                light,
                SHADOW_MAP_RES,
            );
            for (i, m) in mats.iter().enumerate() {
                assert!(
                    m.to_cols_array().iter().all(|v| v.is_finite()),
                    "cascade {i} is not finite for light {light:?}"
                );
            }
        }
    }

    /// A world point that does not move keeps the same **sub-texel phase** in the shadow map while
    /// the camera does.
    ///
    /// This is the whole purpose of texel snapping and it was not happening. The cascade follows
    /// the camera, so a static point is expected to travel across the shadow map — but in *whole
    /// texels*, so that the sampling grid stays fixed relative to the world. Measured before the
    /// fix, the fractional part of the texel coordinate changed on every step (.188, .872, .554,
    /// .234, …) and the steps themselves were fractional (−2.3162, −2.3179, …). That is shadow
    /// edges crawling as the camera moves.
    ///
    /// Two things had to change for this to hold, and the test covers both because it exercises
    /// translation and rotation: the ortho extent comes from a bounding **sphere** (an AABB of the
    /// same corners changes size as the camera turns, so the texel size changed with it), and the
    /// light basis is fixed in the world rather than rebuilt around a camera-following centre (a
    /// grid expressed in a moving space moves).
    #[test]
    fn a_static_point_keeps_its_sub_texel_phase() {
        use gizmo_math::Vec4;
        let point = Vec3::new(3.0, 0.0, 0.0);
        let light = Vec3::new(-0.4, -1.0, -0.3).normalize();
        let splits = cascade_split_distances(0.1, SHADOW_DISTANCE, CASCADE_LAMBDA);

        let phase_at = |cam: Vec3, fwd: Vec3| -> (f32, f32) {
            let m = directional_cascade_view_projs(
                cam,
                fwd,
                16.0 / 9.0,
                0.785,
                0.1,
                &splits,
                light,
                SHADOW_MAP_RES,
            );
            let c = m[0] * Vec4::new(point.x, point.y, point.z, 1.0);
            let uv = (
                (c.x / c.w) * 0.5 + 0.5,
                (c.y / c.w) * -0.5 + 0.5,
            );
            let res = SHADOW_MAP_RES as f32;
            ((uv.0 * res).fract(), (uv.1 * res).fract())
        };

        // Distance between two phases on the unit circle — 0.999 and 0.001 are adjacent, not
        // opposite, and a naive difference would call that a whole-texel error.
        let apart = |a: f32, b: f32| {
            let d = (a - b).abs();
            d.min(1.0 - d)
        };

        let base = phase_at(Vec3::new(-10.0, 2.0, 0.0), Vec3::X);
        for k in 1..8 {
            let slid = phase_at(Vec3::new(-10.0 + k as f32 * 0.01, 2.0, 0.0), Vec3::X);
            assert!(
                apart(slid.0, base.0) < 0.01 && apart(slid.1, base.1) < 0.01,
                "sliding the camera moved the sub-texel phase from {base:?} to {slid:?} — the \
                 snap grid is not fixed in the world, so shadow edges crawl"
            );
            let a = k as f32 * 0.002;
            let turned = phase_at(Vec3::new(-10.0, 2.0, 0.0), Vec3::new(a.cos(), 0.0, a.sin()));
            assert!(
                apart(turned.0, base.0) < 0.01 && apart(turned.1, base.1) < 0.01,
                "turning the camera moved the sub-texel phase from {base:?} to {turned:?} — the \
                 texel size is changing with the camera's orientation"
            );
        }
    }


    // Pure, deterministic, GPU-free coverage of the CSM cascade math (the CPU core of the
    // directional-shadow path). Complements the headless golden render test, which can't
    // reliably frame a shadow, and the compose/exposure tests, without any adapter.
    #[test]
    fn cascade_splits_are_monotonic_and_bounded() {
        let splits = cascade_split_distances(0.1, 100.0, CASCADE_LAMBDA);
        for i in 1..CASCADE_COUNT {
            assert!(splits[i] > splits[i - 1], "splits must strictly increase: {splits:?}");
        }
        assert!(splits[0] > 0.1, "first split must be beyond the near plane: {splits:?}");
        assert!(
            (splits[CASCADE_COUNT - 1] - 100.0).abs() < 1e-3,
            "last split must equal the shadow far distance: {splits:?}"
        );
        assert!(splits.iter().all(|s| s.is_finite()), "splits must be finite: {splits:?}");
    }

    #[test]
    fn cascade_splits_handle_degenerate_range() {
        // far <= near must be clamped (near + epsilon), never NaN/inf or a panic.
        let splits = cascade_split_distances(1.0, 0.5, CASCADE_LAMBDA);
        assert!(
            splits.iter().all(|s| s.is_finite()),
            "degenerate range produced non-finite splits: {splits:?}"
        );
        for i in 1..CASCADE_COUNT {
            assert!(splits[i] >= splits[i - 1], "splits must stay non-decreasing when clamped");
        }
    }

    #[test]
    fn directional_cascades_produce_finite_matrices() {
        // SHADOW_DISTANCE caps the covered range even for a huge camera far plane.
        let c = compute_directional_cascades(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -1.0),
            16.0 / 9.0,
            std::f32::consts::FRAC_PI_4,
            0.1,
            1500.0,
            Vec3::new(0.3, -1.0, 0.2),
        );
        for (i, m) in c.view_projs.iter().enumerate() {
            assert!(
                m.to_cols_array().iter().all(|v| v.is_finite()),
                "cascade {i} light-view-proj has non-finite entries"
            );
        }
        assert!(c.splits.iter().all(|s| s.is_finite()));
        // Shadow range is capped at SHADOW_DISTANCE, not the 1500 camera far plane.
        assert!(
            c.splits[CASCADE_COUNT - 1] <= SHADOW_DISTANCE + 1e-3,
            "cascades must not stretch past SHADOW_DISTANCE: {:?}",
            c.splits
        );
    }

    #[test]
    fn uniform_lambda_gives_evenly_spaced_splits() {
        // lambda = 0 → pure uniform: split[i] = near + (far−near)·(i+1)/N.
        let s = cascade_split_distances(1.0, 5.0, 0.0);
        assert!((s[0] - 2.0).abs() < 1e-4, "{s:?}");
        assert!((s[1] - 3.0).abs() < 1e-4, "{s:?}");
        assert!((s[2] - 4.0).abs() < 1e-4, "{s:?}");
        assert!((s[3] - 5.0).abs() < 1e-4, "{s:?}");
    }

    #[test]
    fn logarithmic_lambda_packs_splits_toward_the_near_plane() {
        // lambda = 1 → pure log split: denser near the camera than a uniform split.
        let log = cascade_split_distances(1.0, 100.0, 1.0);
        let uni = cascade_split_distances(1.0, 100.0, 0.0);
        // First cascade covers less distance under the log scheme.
        assert!(log[0] < uni[0], "log near split should be tighter: {log:?} vs {uni:?}");
        // Log splits grow geometrically: ratio between successive splits is ~constant.
        let r0 = log[1] / log[0];
        let r1 = log[2] / log[1];
        assert!((r0 - r1).abs() < 1e-3, "log splits not geometric: {log:?}");
    }

    // ── Shadow-distance fade (the cure for the hard brightness step at SHADOW_DISTANCE) ──
    //
    // The step is visual and this crate cannot render, so what is tested here is the maths the
    // shaders evaluate: `shadow_distance_fade` is the CPU mirror of the WGSL function, and
    // `baked_lit_shadow_term` below reproduces the exact expression `baked_lit.wgsl` builds
    // from it. Continuity of THAT expression is the property the bug report is about.

    /// The multiplier `baked_lit.wgsl` applies to the baked colour, for a fragment the sun
    /// cannot see at all (`vis` = 0 — the worst case, and the one that produced the band).
    ///
    /// Mirrors the shader exactly:
    ///   `vis_faded = mix(1.0, vis, fade)` then `1 - sun_share + sun_share * vis_faded`.
    fn baked_lit_shadow_term(view_depth: f32, shadow_far: f32, vis: f32) -> f32 {
        const SUN_SHARE: f32 = 0.45; // baked_lit.wgsl `sun_share`
        let sampled = if view_depth <= shadow_far { vis } else { 1.0 };
        let fade = shadow_distance_fade(view_depth, shadow_far);
        let vis_faded = 1.0 + (sampled - 1.0) * fade;
        1.0 - SUN_SHARE + SUN_SHARE * vis_faded
    }

    #[test]
    fn shadow_fade_is_inert_until_the_last_stretch_of_the_range() {
        let far = SHADOW_DISTANCE;
        // Everything up to (1 - SHADOW_FADE_FRACTION) of the range samples the cascade verbatim.
        for d in [0.0f32, 1.0, 25.0, 50.0, 84.9] {
            assert_eq!(
                shadow_distance_fade(d, far),
                1.0,
                "fade must not touch the shadow term at {d} m (band starts at \
                 {})",
                far * (1.0 - SHADOW_FADE_FRACTION)
            );
        }
        // …and nothing survives at or past the end of the covered range.
        for d in [far, far + 1.0, far * 10.0] {
            assert_eq!(shadow_distance_fade(d, far), 0.0, "fade must be spent by {d} m");
        }
    }

    #[test]
    fn shadow_fade_is_monotonic_and_bounded() {
        let far = SHADOW_DISTANCE;
        let mut prev = shadow_distance_fade(0.0, far);
        let mut d = 0.0f32;
        while d <= far * 1.2 {
            let f = shadow_distance_fade(d, far);
            assert!((0.0..=1.0).contains(&f), "fade out of range at {d} m: {f}");
            assert!(f <= prev + 1e-6, "fade must never increase with distance ({d} m: {prev} → {f})");
            prev = f;
            d += 0.05;
        }
    }

    // THE regression test for the reported band. Before the fade, `baked_lit_shadow_term` was
    // 0.55 for every shadowed fragment inside the range and 1.0 for every one outside it: a
    // 1.82x brightness jump across a single boundary, with no distance falloff anywhere in
    // between. Sweeping the term in 5 cm steps and bounding the largest adjacent difference
    // catches that step (0.45) and anything like it.
    #[test]
    fn shadowed_brightness_has_no_step_at_the_shadow_distance() {
        let far = SHADOW_DISTANCE;
        let step_m = 0.05f32;
        let mut worst_delta = 0.0f32;
        let mut worst_at = 0.0f32;
        let mut d = 0.0f32;
        while d < far * 1.2 {
            let a = baked_lit_shadow_term(d, far, 0.0);
            let b = baked_lit_shadow_term(d + step_m, far, 0.0);
            let delta = (b - a).abs();
            if delta > worst_delta {
                worst_delta = delta;
                worst_at = d;
            }
            d += step_m;
        }
        // A 0.45 jump (the pre-fix step) is ten times this bound; the fade spreads the same
        // 0.45 over SHADOW_FADE_FRACTION x SHADOW_DISTANCE = 15 m, so no 5 cm slice moves far.
        assert!(
            worst_delta < 0.005,
            "shadow term still steps: {worst_delta} over {step_m} m at {worst_at} m \
             (pre-fix this was 0.45 at {far} m)"
        );
        // And the endpoints are still the values the shading intends.
        assert!(
            (baked_lit_shadow_term(0.0, far, 0.0) - 0.55).abs() < 1e-6,
            "a fully shadowed fragment in front of the camera must keep the 0.55 floor"
        );
        assert!(
            (baked_lit_shadow_term(far + 5.0, far, 0.0) - 1.0).abs() < 1e-6,
            "past the covered range the term must be fully lit"
        );
    }

    #[test]
    fn shadow_fade_leaves_a_lit_fragment_untouched() {
        // `vis` = 1 (nothing occluding) must stay 1 at every distance: the fade may only ever
        // move the term TOWARD lit, never away from it.
        let far = SHADOW_DISTANCE;
        for d in [0.0f32, 50.0, 90.0, 99.9, 100.0, 250.0] {
            assert!(
                (baked_lit_shadow_term(d, far, 1.0) - 1.0).abs() < 1e-6,
                "unoccluded fragment darkened at {d} m"
            );
        }
    }

    #[test]
    fn shadow_fade_survives_a_degenerate_range() {
        // A camera whose far plane is tiny caps shadow_far below SHADOW_DISTANCE; a zero or
        // negative one must not divide by zero.
        for far in [0.0f32, -1.0, 1e-6, 0.5] {
            for d in [0.0f32, 0.25, 10.0] {
                let f = shadow_distance_fade(d, far);
                assert!(f.is_finite(), "fade not finite for far={far}, d={d}: {f}");
                assert!((0.0..=1.0).contains(&f), "fade out of range for far={far}, d={d}: {f}");
            }
        }
    }

    // The WGSL copies of the fade are text, so nothing else can catch them drifting from the
    // Rust mirror above. Adapter-free: it reads the shader sources, it does not compile them.
    #[test]
    fn shader_shadow_fade_matches_the_rust_mirror() {
        let shaders = [
            ("baked_lit.wgsl", include_str!("shaders/baked_lit.wgsl")),
            ("deferred_lighting.wgsl", include_str!("shaders/deferred_lighting.wgsl")),
        ];
        let expected = format!("const SHADOW_FADE_FRACTION: f32 = {SHADOW_FADE_FRACTION:?};");
        for (name, src) in shaders {
            assert!(
                src.contains(&expected),
                "{name} must declare `{expected}` — the shader fade band has drifted from \
                 csm::SHADOW_FADE_FRACTION"
            );
            assert!(
                src.contains("fn shadow_distance_fade("),
                "{name} lost its shadow_distance_fade mirror"
            );
            assert!(
                src.contains("shadow_distance_fade(view_depth"),
                "{name} declares the fade but never applies it to the sampled shadow term"
            );
        }
    }

    #[test]
    fn cascade_computation_is_deterministic() {
        let build = || {
            compute_directional_cascades(
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(0.0, 0.0, -1.0),
                16.0 / 9.0,
                std::f32::consts::FRAC_PI_4,
                0.1,
                200.0,
                Vec3::new(0.3, -1.0, 0.2),
            )
        };
        let a = build();
        let b = build();
        assert_eq!(a.splits, b.splits, "splits must be reproducible");
        for i in 0..CASCADE_COUNT {
            assert_eq!(
                a.view_projs[i].to_cols_array(),
                b.view_projs[i].to_cols_array(),
                "cascade {i} matrix must be reproducible (texel snap is stable)"
            );
        }
    }
}
