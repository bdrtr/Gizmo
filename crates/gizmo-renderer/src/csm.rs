//! Cascaded shadow maps (CSM) for directional lights: split the camera depth range into
//! several cascades, each rendered to a layer of a `depth2d_array` with its own light
//! orthographic projection (tighter texel density near the camera).

use gizmo_math::{Mat4, Vec3, Vec4};

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

/// The directional shadow cascades for one frame: the split distances and the
/// per-cascade light clip matrices, ready to upload.
pub struct ShadowCascades {
    pub splits: [f32; CASCADE_COUNT],
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
    let mut prev_z = z_near;
    let mut mats = [Mat4::IDENTITY; CASCADE_COUNT];

    for i in 0..CASCADE_COUNT {
        let zf = splits[i];
        let corners =
            frustum_slice_corners(cam_pos, cam_forward, right, up, aspect, fov_y, prev_z, zf);
        let mid_dist = (prev_z + zf) * 0.5;
        let slice_center = cam_pos + cam_forward * mid_dist;
        let light_pos = slice_center - light_dir * 250.0;
        let light_view = Mat4::look_at_rh(light_pos, slice_center, Vec3::Y);

        let mut min_b = Vec3::splat(f32::MAX);
        let mut max_b = Vec3::splat(f32::MIN);
        for c in corners {
            let v = light_view * Vec4::new(c.x, c.y, c.z, 1.0);
            debug_assert!(v.w.abs() > 1e-6, "CSM corner projection: v.w ≈ 0 — degenerate light view matrix");
            let p = Vec3::new(v.x, v.y, v.z) / v.w;
            min_b = min_b.min(p);
            max_b = max_b.max(p);
        }
        min_b.z -= 40.0;
        max_b.z += 60.0;

        // Light-space texel snap (reduces edge swimming)
        let world_units_per_texel_x = (max_b.x - min_b.x) / shadow_map_size as f32;
        let world_units_per_texel_y = (max_b.y - min_b.y) / shadow_map_size as f32;
        if world_units_per_texel_x > 1e-8 && world_units_per_texel_y > 1e-8 {
            min_b.x = (min_b.x / world_units_per_texel_x).floor() * world_units_per_texel_x;
            min_b.y = (min_b.y / world_units_per_texel_y).floor() * world_units_per_texel_y;
            max_b.x = min_b.x + world_units_per_texel_x * shadow_map_size as f32;
            max_b.y = min_b.y + world_units_per_texel_y * shadow_map_size as f32;
        }

        let ortho = Mat4::orthographic_rh(min_b.x, max_b.x, min_b.y, max_b.y, -max_b.z, -min_b.z);
        mats[i] = ortho * light_view;
        prev_z = zf;
    }
    mats
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
