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

impl Vertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress, shader_location: 1, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress, shader_location: 2, format: wgpu::VertexFormat::Float32x3 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress, shader_location: 3, format: wgpu::VertexFormat::Float32x2 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 11]>() as wgpu::BufferAddress, shader_location: 4, format: wgpu::VertexFormat::Uint32x4 },
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 11]>() + std::mem::size_of::<[u32; 4]>()) as wgpu::BufferAddress,
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
    pub position: [f32; 4],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PostProcessUniforms {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub chromatic_aberration: f32,
    pub vignette_intensity: f32,
    pub _padding: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub sun_direction: [f32; 4],
    pub sun_color: [f32; 4],
    pub lights: [LightData; 10],
    pub light_view_proj: [[f32; 4]; 4],
    pub num_lights: u32,
    pub _padding: [u32; 3],
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

