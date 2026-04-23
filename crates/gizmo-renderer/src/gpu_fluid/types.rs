#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FluidParticle {
    pub position: [f32; 3],
    pub density: f32,
    pub velocity: [f32; 3],
    pub pressure: f32,
    pub phase: u32,
    pub pad1: u32,
    pub pad2: u32,
    pub pad3: u32,
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
    pub pad1: f32,
    pub pad2: f32,
    pub pad3: f32,
}

pub const MAX_FLUID_COLLIDERS: usize = 64;
