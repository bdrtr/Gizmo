#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    pub position: [f32; 3],
    pub life: f32,
    pub velocity: [f32; 3],
    pub max_life: f32,
    pub color: [f32; 4],
    pub size_start: f32,
    pub size_end: f32,
    pub _padding: [f32; 2],
}

impl GpuParticle {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuParticle>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x4,
                }, // pos + life
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                }, // vel + max_life
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                }, // color
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                }, // sizes + padding
            ],
        }
    }
}

/// The maximum number of obstacle spheres the particle-sim uniform can carry.
pub const MAX_PARTICLE_OBSTACLES: usize = 8;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleSimParams {
    pub dt: f32,
    pub global_gravity: f32,
    pub global_drag: f32,
    /// How many obstacles are active (0 = none → deflection OFF, the old behaviour).
    pub obstacle_count: f32,
    /// xyz = the nominal flow velocity (the relaxation target), w = the relaxation rate
    /// (0 = off). Particle velocities are drawn smoothly towards the target each frame, so the
    /// flow lines become parallel again downstream of an obstacle.
    pub flow_target: [f32; 4],
    /// x = turbulence strength (the divergence-free swirl amplitude added to the relaxation
    /// target, which gives smoke-like undulating filaments). yzw are reserved for future use.
    pub misc: [f32; 4],
    /// Obstacle spheres: xyz = the centre (world), w = the radius. The first `obstacle_count`
    /// are valid.
    pub obstacles: [[f32; 4]; MAX_PARTICLE_OBSTACLES],
}
