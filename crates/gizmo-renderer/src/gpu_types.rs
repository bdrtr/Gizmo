use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub normal: [f32; 3],
    pub tex_coords: [f32; 2],
    pub joint_indices: [u32; 4],
    pub joint_weights: [f32; 4],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            color: [1.0; 3],
            normal: [0.0, 1.0, 0.0],
            tex_coords: [0.0; 2],
            joint_indices: [0; 4],
            joint_weights: [0.0; 4],
        }
    }
}

impl Vertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 11]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint32x4,
                },
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 11]>() + std::mem::size_of::<[u32; 4]>())
                        as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LightData {
    pub position: [f32; 4],  // xyz=pos, w=intensity
    pub color: [f32; 4],     // rgb=color, a=radius
    pub direction: [f32; 4], // xyz=direction (spot), w=inner_cutoff_cos
    pub params: [f32; 4], // x=outer_cutoff_cos, y=light_type (0=point,1=spot,2=directional), zw=unused
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PostProcessUniforms {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub chromatic_aberration: f32,
    pub vignette_intensity: f32,
    pub film_grain_intensity: f32,
    pub dof_focus_dist: f32,
    pub dof_focus_range: f32,
    pub dof_blur_size: f32,
    pub _padding: [f32; 3],
}

/// Uniform block for the shadow pass vertex shader only (one cascade matrix per draw).
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct ShadowVsUniform {
    pub light_view_proj: [[f32; 4]; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub sun_direction: [f32; 4],
    pub sun_color: [f32; 4],
    pub lights: [LightData; 10],
    /// Directional CSM: world → light clip space per cascade (same order as shadow array layers).
    pub light_view_proj: [[[f32; 4]; 4]; 4],
    /// Far distance (along `camera_forward`) for cascades 0..3; `w` is always camera far plane.
    pub cascade_splits: [f32; 4],
    /// xyz = normalized camera forward in world space (for view-depth cascade selection).
    pub camera_forward: [f32; 4],
    /// x = camera z_near, y = 1 / shadow map resolution (PCF texel size), zw unused.
    pub cascade_params: [f32; 4],
    pub num_lights: u32,
    // WGSL: _align_pad: vec3<u32> has alignment 16.
    // After num_lights at offset 1060, WGSL inserts 12 bytes implicit padding → offset 1072
    pub _pre_align_pad: [u32; 3], // offset 1060-1071 (WGSL implicit padding before vec3)
    pub _align_pad: [u32; 3],     // offset 1072-1083 (WGSL _align_pad: vec3<u32>)
    pub _post_align_pad: u32,     // offset 1084-1087 (WGSL implicit padding, next vec3 align 16)
    pub _pad_scene: [u32; 3],     // offset 1088-1099 (WGSL _pad_scene: vec3<u32>)
    pub shading_mode: u32, // offset 1100-1103 (WGSL shading_mode: u32) 0=Lit, 1=Normals, 2=Albedo
                           // Total: 1104 bytes
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    pub albedo_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub _padding: f32,
}
