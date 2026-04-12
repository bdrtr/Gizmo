//! Cascaded shadow maps (CSM) for directional lights: split the camera depth range into
//! several cascades, each rendered to a layer of a `depth2d_array` with its own light
//! orthographic projection (tighter texel density near the camera).

use gizmo_math::{Mat4, Vec3, Vec4};

/// Must match `texture_depth_2d_array` layer count and `SceneUniforms.light_view_proj` length.
pub const CASCADE_COUNT: usize = 4;

/// Resolution (width = height) of each cascade depth map.
pub const SHADOW_MAP_RES: u32 = 2048;

/// Logarithmic-linear split distances in **world units** along `cam_forward` from `cam_pos`.
/// `splits[i]` is the far distance of cascade `i` (inclusive range `[prev, splits[i]]`).
pub fn cascade_split_distances(z_near: f32, z_far: f32, lambda: f32) -> [f32; CASCADE_COUNT] {
    let mut s = [0.0f32; CASCADE_COUNT];
    let z_near = z_near.max(0.001);
    let z_far = z_far.max(z_near + 0.001);
    let n = CASCADE_COUNT as f32;
    for i in 0..CASCADE_COUNT {
        let p = (i + 1) as f32 / n;
        let log_d = z_near * (z_far / z_near).powf(p);
        let uni_d = z_near + (z_far - z_near) * p;
        s[i] = lambda * log_d + (1.0 - lambda) * uni_d;
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
        let corners = frustum_slice_corners(cam_pos, cam_forward, right, up, aspect, fov_y, prev_z, zf);
        let mid_dist = (prev_z + zf) * 0.5;
        let slice_center = cam_pos + cam_forward * mid_dist;
        let light_pos = slice_center - light_dir * 250.0;
        let light_view = Mat4::look_at_rh(light_pos, slice_center, Vec3::Y);

        let mut min_b = Vec3::splat(f32::MAX);
        let mut max_b = Vec3::splat(f32::MIN);
        for c in corners {
            let v = light_view * Vec4::new(c.x, c.y, c.z, 1.0);
            let p = Vec3::new(v.x, v.y, v.z) / v.w;
            min_b = min_b.min(p);
            max_b = max_b.max(p);
        }
        min_b.z -= 120.0;
        max_b.z += 280.0;

        // Light-space texel snap (reduces edge swimming)
        let world_units_per_texel_x = (max_b.x - min_b.x) / shadow_map_size as f32;
        let world_units_per_texel_y = (max_b.y - min_b.y) / shadow_map_size as f32;
        if world_units_per_texel_x > 1e-8 && world_units_per_texel_y > 1e-8 {
            min_b.x = (min_b.x / world_units_per_texel_x).floor() * world_units_per_texel_x;
            min_b.y = (min_b.y / world_units_per_texel_y).floor() * world_units_per_texel_y;
            max_b.x = min_b.x + world_units_per_texel_x * shadow_map_size as f32;
            max_b.y = min_b.y + world_units_per_texel_y * shadow_map_size as f32;
        }

        let ortho = Mat4::orthographic_rh(min_b.x, max_b.x, min_b.y, max_b.y, min_b.z, max_b.z);
        mats[i] = ortho * light_view;
        prev_z = zf;
    }
    mats
}
