/// One GPU-simulated particle: 64 bytes that are stepped by a compute pass and then drawn, with
/// no copy in between, as the instance data [`GpuParticle::desc`] describes.
///
/// A particle is respawned rather than removed — the buffer's length never changes, and a dead
/// particle is one whose [`Self::life`] has run out. That is what keeps the whole system to one
/// dispatch and one draw with no CPU readback.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    /// World-space position.
    pub position: [f32; 3],
    /// Seconds of life left. At or below zero the particle is dead and eligible for respawn.
    pub life: f32,
    /// Velocity, m/s.
    pub velocity: [f32; 3],
    /// The life this particle was spawned with; `life / max_life` is the 1→0 age the shader fades
    /// and scales by.
    pub max_life: f32,
    /// Tint, RGBA.
    pub color: [f32; 4],
    /// Billboard size at spawn.
    pub size_start: f32,
    /// Billboard size at death; the shader interpolates between the two over the particle's age.
    pub size_end: f32,
    /// Padding to 64 bytes — and to a whole `vec4` attribute slot, since the sizes are read as
    /// one.
    pub _padding: [f32; 2],
}

impl GpuParticle {
    /// This struct as a per-instance vertex layout — four `vec4`s at locations 0..=3 covering the
    /// whole record, so the simulation buffer is bound directly as instance data.
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

/// The per-frame parameters the particle compute pass reads.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleSimParams {
    /// The timestep, in seconds.
    pub dt: f32,
    /// Downward acceleration applied to every particle, m/s².
    pub global_gravity: f32,
    /// Per-second velocity damping.
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
