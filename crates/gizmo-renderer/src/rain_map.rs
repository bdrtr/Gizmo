//! The rain map: the scene's depth seen from straight above, around the camera — what keeps the ground
//! under a bridge, a roof or a parked car dry in the rain (`Renderer::wetness`).
//!
//! A fragment the map has something above is under cover, and takes no rain (`rain_exposure` in
//! `common.wgsl`, which `gizmo::surface::rain_on` is handed); one at the top of the map is out in the
//! open. The map is a layer of the sun's shadow array past the cascades (`csm::RAIN_LAYER`), drawn by
//! the shadow pass's pipelines with a matrix of its own, every frame the scene is wet, so that what moves
//! — a car, a door — shelters where it stands, and what streams in shelters as it arrives. It is drawn
//! around the frame's own camera: a second view (split screen, a texture camera) far from it sees open
//! sky past the map's edge.
//!
//! Desktop only: the forward-only path's sun array has one layer, and its lit shaders read the rain
//! without cover.

use gizmo_math::{Mat4, Vec3};

/// How far the map reaches, metres a side, the camera at its middle: what a street's reflections show
/// and then some.
pub const RAIN_MAP_EXTENT: f32 = 256.0;

/// How deep the map sees, metres, the camera half way: a tower's top above it and a valley below.
pub const RAIN_MAP_DEPTH: f32 = 2000.0;

/// How far below the top of the map a fragment may lie and still count as at the top, metres: its own
/// surface, a texel's slope away, and the depth's precision — not a ledge.
pub const RAIN_MAP_BIAS: f32 = 0.25;

/// The rain pass's group (`ShadowVsUniform` in the shadow pass's layout): the map's matrix, which the
/// shadow pipelines read as a cascade's.
pub struct RainMap {
    buffer: wgpu::Buffer,
    /// Bound at group 0 of the rain pass, as a cascade's group is in its pass.
    pub bind_group: wgpu::BindGroup,
}

impl RainMap {
    /// The pass's buffer and group, over `layout` (`SceneState::shadow_pass_bind_group_layout`).
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rain map pass"),
            size: std::mem::size_of::<crate::gpu_types::ShadowVsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rain map pass bg"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        Self { buffer, bind_group }
    }

    /// Writes this frame's matrix (`view_proj`) for the pass, with the frame's camera and clock, which a
    /// surface that cuts itself by them reads (`ShadowVsUniform`).
    pub fn write(&self, queue: &wgpu::Queue, view_proj: Mat4, camera: Vec3, clock: f32) {
        let uniform = crate::gpu_types::ShadowVsUniform {
            light_view_proj: view_proj.to_cols_array_2d(),
            camera_position: camera.extend(0.0).to_array(),
            params: [clock, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// The map's world → clip matrix around `camera`, for a map `res` texels a side: looking straight
    /// down from half its depth above the camera, its middle snapped to whole texels, so that a moving
    /// camera does not drag the edges of cover across the ground a fraction of a texel a frame.
    pub fn view_proj(camera: Vec3, res: u32) -> Mat4 {
        let texel = RAIN_MAP_EXTENT / res.max(1) as f32;
        let middle = Vec3::new((camera.x / texel).round() * texel, camera.y, (camera.z / texel).round() * texel);
        let eye = middle + Vec3::Y * (RAIN_MAP_DEPTH * 0.5);
        // Up on the map is −Z: any direction but the one looked along.
        let view = Mat4::look_at_rh(eye, middle, Vec3::NEG_Z);
        let half = RAIN_MAP_EXTENT * 0.5;
        Mat4::orthographic_rh(-half, half, -half, half, 0.0, RAIN_MAP_DEPTH) * view
    }

    /// What the scene uniform carries for the lit shaders' lookup (`SceneUniforms::rain_params`): `x` 1
    /// while there is a map, `y` a texel's width in the map's coordinates, `z` [`RAIN_MAP_BIAS`] in its
    /// depth.
    pub fn params(res: u32) -> [f32; 4] {
        [1.0, 1.0 / res.max(1) as f32, RAIN_MAP_BIAS / RAIN_MAP_DEPTH, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where `point` lands in the map: its coordinate (0 – 1 across, top-left first, as a texture's) and
    /// its depth (0 at the top of the map).
    fn lands(view_proj: Mat4, point: Vec3) -> (f32, f32, f32) {
        let clip = view_proj * point.extend(1.0);
        (clip.x * 0.5 + 0.5, 0.5 - clip.y * 0.5, clip.z / clip.w)
    }

    /// The camera's ground is the map's middle, what is above it is nearer the top, and the map reaches
    /// half its extent each way and half its depth up and down — the lookup's `[0, 1]` throughout.
    #[test]
    fn the_map_looks_down_on_the_camera() {
        let camera = Vec3::new(10.0, 5.0, -20.0);
        let m = RainMap::view_proj(camera, 4096);
        let (u, v, ground) = lands(m, camera);
        assert!((u - 0.5).abs() < 1e-3 && (v - 0.5).abs() < 1e-3, "the camera is the middle: {u} {v}");
        let (_, _, roof) = lands(m, camera + Vec3::Y * 4.0);
        assert!(roof < ground, "what is above is nearer the top of the map: {roof} against {ground}");
        assert!(((ground - roof) - 4.0 / RAIN_MAP_DEPTH).abs() < 1e-5, "and by its height, linearly");
        let (u, _, _) = lands(m, camera + Vec3::X * (RAIN_MAP_EXTENT * 0.5 - 0.01));
        assert!((0.99..=1.0).contains(&u), "half the extent to the right is the right edge: {u}");
        let (_, _, top) = lands(m, camera + Vec3::Y * (RAIN_MAP_DEPTH * 0.5 - 0.01));
        let (_, _, bottom) = lands(m, camera - Vec3::Y * (RAIN_MAP_DEPTH * 0.5 - 0.01));
        assert!((0.0..1e-4).contains(&top) && (1.0 - 1e-4..=1.0).contains(&bottom), "half the depth up and down: {top} {bottom}");
    }

    /// The map's middle moves in whole texels: a camera moving a fraction of one leaves the matrix as it
    /// was, and one moving a texel moves the map a texel.
    #[test]
    fn the_map_moves_in_whole_texels() {
        let res = 1024;
        let texel = RAIN_MAP_EXTENT / res as f32;
        let at = Vec3::new(3.0 * texel, 0.0, 0.0);
        let still = RainMap::view_proj(at, res);
        assert_eq!(RainMap::view_proj(at + Vec3::X * (0.3 * texel), res), still, "a fraction of a texel moves nothing");
        let moved = RainMap::view_proj(at + Vec3::X * texel, res);
        let (u0, _, _) = lands(still, Vec3::ZERO);
        let (u1, _, _) = lands(moved, Vec3::ZERO);
        assert!(((u0 - u1) - 1.0 / res as f32).abs() < 1e-5, "a texel moves the map a texel: {u0} → {u1}");
    }
}
