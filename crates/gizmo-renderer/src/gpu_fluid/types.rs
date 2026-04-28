// ═══════════════════════════════════════════════════════════════════════
//  AAA Fluid GPU Types — Rust ↔ WGSL mirrored structs
// ═══════════════════════════════════════════════════════════════════════

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParticle {
    pub position: [f32; 3],
    pub density: f32,
    pub velocity: [f32; 3],
    pub lambda: f32,
    pub predicted_position: [f32; 3],
    pub phase: u32,
    // AAA: Vorticity field (curl of velocity) for Vorticity Confinement
    pub vorticity: [f32; 3],
    pub _pad_vort: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParticleHash {
    pub hash: u32,
    pub index: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SortParams {
    pub j: u32,
    pub k: u32,
    pub _pad0: u32,
    pub _pad1: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidCollider {
    pub position: [f32; 3],
    pub radius: f32,
    pub velocity: [f32; 3],
    pub padding: f32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParams {
    pub dt: f32,
    pub gravity: f32,
    pub rest_density: f32,
    pub gas_constant: f32,
    pub viscosity: f32,
    pub mass: f32,
    pub smoothing_radius: f32,
    pub num_particles: u32,
    pub grid_size_x: u32,
    pub grid_size_y: u32,
    pub grid_size_z: u32,
    pub cell_size: f32,
    pub bounds_min: [f32; 3],
    pub bounds_padding1: f32,
    pub bounds_max: [f32; 3],
    pub bounds_padding2: f32,

    pub mouse_pos: [f32; 3],
    pub mouse_active: f32,
    pub mouse_dir: [f32; 3],
    pub mouse_radius: f32,

    pub num_colliders: u32,
    pub cohesion: f32,
    pub time: f32,
    // AAA: Vorticity confinement strength (epsilon)
    pub vorticity_strength: f32,

    // AAA: Surface tension coefficient (gamma)
    pub surface_tension: f32,
    // AAA: Laplacian viscosity coefficient (mu)
    pub viscosity_laplacian: f32,
    // AAA: XSPH velocity smoothing factor (c)
    pub xsph_factor: f32,
    // AAA: Number of pressure solver iterations
    pub solver_iterations: u32,
}

pub const MAX_FLUID_COLLIDERS: usize = 64;
