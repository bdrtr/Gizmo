// ═══════════════════════════════════════════════════════════════════════
//  AAA Fluid GPU Bindings — Navier-Stokes / PBF / Vorticity Confinement
// ═══════════════════════════════════════════════════════════════════════

struct FluidParticle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    lambda: f32,
    predicted_position: vec3<f32>,
    phase: u32,
    // AAA: Vorticity field for curl computation (Vorticity Confinement)
    vorticity: vec3<f32>,
    // AAA: Padding to maintain 16-byte alignment
    _pad_vort: f32,
}

struct FluidCollider {
    position: vec3<f32>,
    radius: f32,
    velocity: vec3<f32>,
    shape_type: u32,
    half_extents: vec3<f32>,
    _pad: f32,
}

struct FluidParams {
    dt: f32,
    gravity: f32,
    rest_density: f32,
    gas_constant: f32,
    
    viscosity: f32,
    mass: f32,
    smoothing_radius: f32,
    num_particles: u32,
    
    grid_size_x: u32,
    grid_size_y: u32,
    grid_size_z: u32,
    cell_size: f32,

    bounds_min: vec3<f32>,
    bounds_padding1: f32,
    bounds_max: vec3<f32>,
    bounds_padding2: f32,

    mouse_pos: vec3<f32>,
    mouse_active: f32,
    mouse_dir: vec3<f32>,
    mouse_radius: f32,
    
    num_colliders: u32,
    cohesion: f32,
    time: f32,
    // AAA: Vorticity confinement strength (epsilon)
    vorticity_strength: f32,

    // AAA: Surface tension coefficient (gamma)
    surface_tension: f32,
    // AAA: Laplacian viscosity coefficient (mu)
    viscosity_laplacian: f32,
    // AAA: XSPH velocity smoothing factor (c)
    xsph_factor: f32,
    // AAA: Number of pressure solver iterations
    solver_iterations: u32,
}

struct ParticleHash {
    hash: u32,
    index: u32,
}

struct SortParams {
    j: u32,
    k: u32,
}

@group(0) @binding(0) var<uniform> params: FluidParams;
@group(0) @binding(1) var<storage, read_write> particles: array<FluidParticle>;
@group(0) @binding(2) var<storage, read_write> grid: array<u32>;
@group(0) @binding(3) var<storage, read> colliders: array<FluidCollider>;
@group(0) @binding(4) var<storage, read_write> sort_buffer: array<ParticleHash>;
@group(0) @binding(5) var<uniform> sort_params: SortParams;
