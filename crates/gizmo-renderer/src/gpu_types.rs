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
    pub tangent: [f32; 4],
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
            tangent: [1.0, 0.0, 0.0, 1.0],
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
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 11]>() + std::mem::size_of::<[u32; 4]>() + std::mem::size_of::<[f32; 4]>())
                        as wgpu::BufferAddress,
                    shader_location: 6,
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
    // Active camera near/far, so DoF depth linearization matches the real projection
    // instead of hardcoded 0.1/1000 (miscalibrated CoC for any other far plane).
    pub cam_near: f32,
    pub cam_far: f32,
    pub _padding: f32,
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
    /// x = camera z_near, y = 1 / shadow map resolution (PCF texel size),
    /// z = elapsed time in seconds (fluid caustics/wave animation), w unused.
    pub cascade_params: [f32; 4],
    pub num_lights: u32,
    pub exposure: f32,
    pub _pre_align_pad: [u32; 2], // offset 1064-1071
    pub _align_pad: [u32; 3],     // offset 1072-1083
    pub environment_blend_t: f32, // offset 1084-1087
    pub environment_preset: u32,  // offset 1088-1091
    pub point_shadows_enabled: u32, // offset 1092-1095
    pub environment_preset_2: u32, // offset 1096-1099
    pub shading_mode: u32,        // offset 1100-1103
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

/// Per-material scalar parameters that accompany the textured-PBR bind group
/// (group 1, binding 6).  These carry the glTF factors that modulate the
/// sampled auxiliary maps so that an absent map falls back to the scalar value:
///
/// * `emissive` = emissiveFactor (× KHR_materials_emissive_strength) — multiplied
///   by the (white-default) emissive map, so absent map + zero factor = no emission.
/// * `normal_scale` = glTF normalTexture.scale — scales the tangent-space XY of the
///   (flat-default) normal map, so absent map = unperturbed geometric normal.
/// * `occlusion_strength` = glTF occlusionTexture.strength — lerps the (white-default)
///   AO map toward 1.0, so absent map = no occlusion.
/// * `uv` = KHR_texture_transform (offset / rotation / scale) applied to the UV
///   before every map is sampled; identity when the extension is absent.
///
/// std140 layout: three 16-byte vec4 slots → 48 bytes total.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MaterialParams {
    /// xyz = emissive factor (linear), w = normal-map scale.
    pub emissive_and_normal_scale: [f32; 4],
    /// x = occlusion (AO) strength; y = UV rotation (radians); zw = UV offset.
    pub occlusion_uv_rot_offset: [f32; 4],
    /// xy = UV scale; zw reserved (0.0).
    pub uv_scale: [f32; 4],
}

impl Default for MaterialParams {
    fn default() -> Self {
        // Neutral material: no emission, unit normal scale, unit AO strength,
        // identity UV transform (zero offset, zero rotation, unit scale).
        Self {
            emissive_and_normal_scale: [0.0, 0.0, 0.0, 1.0],
            occlusion_uv_rot_offset: [1.0, 0.0, 0.0, 0.0],
            uv_scale: [1.0, 1.0, 0.0, 0.0],
        }
    }
}

impl MaterialParams {
    pub fn new(
        emissive: [f32; 3],
        normal_scale: f32,
        occlusion_strength: f32,
        uv: UvTransform,
    ) -> Self {
        Self {
            emissive_and_normal_scale: [emissive[0], emissive[1], emissive[2], normal_scale],
            occlusion_uv_rot_offset: [occlusion_strength, uv.rotation, uv.offset[0], uv.offset[1]],
            uv_scale: [uv.scale[0], uv.scale[1], 0.0, 0.0],
        }
    }
}

/// A UV-coordinate transform from `KHR_texture_transform` (offset, rotation in
/// radians, scale). The renderer applies one transform per material (derived
/// from the base-colour texture) to every map's sampled UV — see
/// `asset::loaders`. Its [`Default`] is the identity (no transform).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct UvTransform {
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

impl Default for UvTransform {
    fn default() -> Self {
        Self { offset: [0.0, 0.0], rotation: 0.0, scale: [1.0, 1.0] }
    }
}

impl UvTransform {
    /// True when this transform leaves UVs unchanged (identity).
    pub fn is_identity(&self) -> bool {
        self.offset == [0.0, 0.0] && self.rotation == 0.0 && self.scale == [1.0, 1.0]
    }
}
