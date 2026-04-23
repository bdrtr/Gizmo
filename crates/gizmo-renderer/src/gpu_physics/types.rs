use crate::gpu_types::Vertex;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuBox {
    pub position: [f32; 3],
    pub mass: f32,
    pub velocity: [f32; 3],
    pub state: u32,
    pub rotation: [f32; 4],
    pub angular_velocity: [f32; 3],
    pub sleep_counter: u32,
    pub color: [f32; 4],
    pub half_extents: [f32; 3],
    pub _pad: u32,
}

impl GpuBox {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuBox>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 80,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuCollider {
    pub shape_type: u32, // 0 = AABB, 1 = Plane
    pub _pad1: [u32; 3],
    pub data1: [f32; 4], // AABB: min, Plane: normal
    pub data2: [f32; 4], // AABB: max, Plane: [d, pad, pad, pad]
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PhysicsSimParams {
    // WGSL vec3<f32> → 16-byte alignment. Toplam struct: 64 bytes.
    pub dt: f32,                 // offset 0
    pub _pad0: [u32; 3],         // offset 4-15  (WGSL implicit padding — vec3 align 16)
    pub _pad1: [f32; 3],         // offset 16-27 (WGSL _pad1: vec3<f32>)
    pub _pad1b: u32,             // offset 28-31 (WGSL implicit padding — vec3 align 16)
    pub gravity: [f32; 3],       // offset 32-43 (WGSL gravity: vec3<f32>)
    pub damping: f32,            // offset 44-47
    pub num_boxes: u32,          // offset 48-51
    pub num_colliders: u32,      // offset 52-55
    pub _pad2: [u32; 2],         // offset 56-63 (WGSL _pad2: vec2<u32>)
}
