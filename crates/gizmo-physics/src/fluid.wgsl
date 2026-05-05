// fluid.wgsl

struct Particle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    pressure: f32,
}

struct Params {
    dt: f32,
    gravity: vec3<f32>,
    particle_radius: f32,
    smoothing_radius: f32,
    target_density: f32,
    pressure_multiplier: f32,
    viscosity: f32,
    num_particles: u32,
    grid_size_x: u32,
    grid_size_y: u32,
    grid_size_z: u32,
    cell_size: f32,
}

@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1) var<storage, read_write> cell_counts: array<atomic<u32>>;
@group(0) @binding(2) var<storage, read_write> cell_offsets: array<u32>;
@group(0) @binding(3) var<storage, read_write> sorted_indices: array<u32>;
@group(0) @binding(4) var<storage, read_write> forces: array<vec3<f32>>;
@group(0) @binding(5) var<uniform> params: Params;

const PI: f32 = 3.14159265359;

// Calculate grid cell hash
fn get_cell_coords(pos: vec3<f32>) -> vec3<i32> {
    return vec3<i32>(floor(pos / params.cell_size));
}

fn get_cell_hash(coords: vec3<i32>) -> u32 {
    let cx = u32(clamp(coords.x, 0, i32(params.grid_size_x) - 1));
    let cy = u32(clamp(coords.y, 0, i32(params.grid_size_y) - 1));
    let cz = u32(clamp(coords.z, 0, i32(params.grid_size_z) - 1));
    return cx + cy * params.grid_size_x + cz * params.grid_size_x * params.grid_size_y;
}

// -----------------------------------------------------------------------------
// PASS 1: Clear counts & Compute Hashes
// -----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn clear_counts(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let cell_idx = global_id.x;
    let total_cells = params.grid_size_x * params.grid_size_y * params.grid_size_z;
    if (cell_idx < total_cells) {
        atomicStore(&cell_counts[cell_idx], 0u);
    }
}

@compute @workgroup_size(256)
fn count_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= params.num_particles) { return; }
    
    let p = particles[p_idx];
    let coords = get_cell_coords(p.position);
    let hash = get_cell_hash(coords);
    
    atomicAdd(&cell_counts[hash], 1u);
}

// -----------------------------------------------------------------------------
// PASS 2: Sort Particles into Grid
// Note: Prefix sum for cell_offsets is computed on CPU between passes
// -----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn sort_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= params.num_particles) { return; }
    
    let p = particles[p_idx];
    let coords = get_cell_coords(p.position);
    let hash = get_cell_hash(coords);
    
    let local_offset = atomicAdd(&cell_counts[hash], 1u);
    let sorted_idx = cell_offsets[hash] + local_offset;
    
    sorted_indices[sorted_idx] = p_idx;
}

// -----------------------------------------------------------------------------
// SPH Kernels
// -----------------------------------------------------------------------------
fn poly6_kernel(r: f32, h: f32) -> f32 {
    if (r < 0.0 || r > h) { return 0.0; }
    let q = h * h - r * r;
    return (315.0 / (64.0 * PI * pow(h, 9.0))) * q * q * q;
}

fn spiky_kernel_grad(r: f32, h: f32) -> f32 {
    if (r <= 0.0 || r > h) { return 0.0; }
    let q = h - r;
    return -(45.0 / (PI * pow(h, 6.0))) * q * q;
}

fn visc_kernel_laplacian(r: f32, h: f32) -> f32 {
    if (r <= 0.0 || r > h) { return 0.0; }
    return (45.0 / (PI * pow(h, 6.0))) * (h - r);
}

// -----------------------------------------------------------------------------
// PASS 3: Compute Density and Pressure
// -----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn compute_density(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= params.num_particles) { return; }
    
    let p_pos = particles[p_idx].position;
    let coords = get_cell_coords(p_pos);
    
    var density = 0.0;
    
    // Neighborhood search
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let n_coords = coords + vec3<i32>(x, y, z);
                
                // Bounds check
                if (n_coords.x < 0 || n_coords.x >= i32(params.grid_size_x) ||
                    n_coords.y < 0 || n_coords.y >= i32(params.grid_size_y) ||
                    n_coords.z < 0 || n_coords.z >= i32(params.grid_size_z)) {
                    continue;
                }
                
                let hash = get_cell_hash(n_coords);
                let start_idx = cell_offsets[hash];
                // Since count_particles re-added to cell_counts, we must use cell_offsets[hash+1] if available
                // Wait, sort_particles ALREADY advanced cell_counts!
                // So cell_counts[hash] NOW contains the exact offset for the END of the cell.
                let end_idx = cell_offsets[hash] + cell_counts[hash]; // NO WAIT!
                // Actually, sort_particles adds 1 for each particle.
                // So at the end of sort_particles, cell_counts[hash] equals the original count!
                // Yes, but we must zero it before sort_particles if we want that.
                // It's simpler to just pass `cell_counts` from CPU or zero it before sorting.
                // Let's assume cell_counts was zeroed by CPU before sort_particles, 
                // so after sort, cell_counts == original count.
                let count = cell_counts[hash];
                
                for (var i = 0u; i < count; i++) {
                    let neighbor_idx = sorted_indices[start_idx + i];
                    let n_pos = particles[neighbor_idx].position;
                    
                    let dist = distance(p_pos, n_pos);
                    density += poly6_kernel(dist, params.smoothing_radius);
                }
            }
        }
    }
    
    // Self-density (if not added above, but it IS added above since self is in the same cell)
    // Actually, we must ensure density is at least a small epsilon to avoid div by zero
    density = max(density, 0.001);
    
    particles[p_idx].density = density;
    
    // Tait equation for pressure
    particles[p_idx].pressure = params.pressure_multiplier * (pow(density / params.target_density, 7.0) - 1.0);
}

// -----------------------------------------------------------------------------
// PASS 4: Compute Forces
// -----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn compute_forces(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= params.num_particles) { return; }
    
    let p = particles[p_idx];
    let coords = get_cell_coords(p.position);
    
    var pressure_force = vec3<f32>(0.0);
    var visc_force = vec3<f32>(0.0);
    
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let n_coords = coords + vec3<i32>(x, y, z);
                
                if (n_coords.x < 0 || n_coords.x >= i32(params.grid_size_x) ||
                    n_coords.y < 0 || n_coords.y >= i32(params.grid_size_y) ||
                    n_coords.z < 0 || n_coords.z >= i32(params.grid_size_z)) {
                    continue;
                }
                
                let hash = get_cell_hash(n_coords);
                let start_idx = cell_offsets[hash];
                let count = cell_counts[hash];
                
                for (var i = 0u; i < count; i++) {
                    let n_idx = sorted_indices[start_idx + i];
                    if (n_idx == p_idx) { continue; }
                    
                    let n = particles[n_idx];
                    let dir = p.position - n.position;
                    let dist = length(dir);
                    
                    if (dist > 0.0 && dist < params.smoothing_radius) {
                        let dir_norm = dir / dist;
                        
                        // Pressure
                        let shared_pressure = (p.pressure + n.pressure) / 2.0;
                        pressure_force += -dir_norm * shared_pressure * spiky_kernel_grad(dist, params.smoothing_radius) / n.density;
                        
                        // Viscosity
                        let rel_vel = n.velocity - p.velocity;
                        visc_force += params.viscosity * rel_vel * visc_kernel_laplacian(dist, params.smoothing_radius) / n.density;
                    }
                }
            }
        }
    }
    
    forces[p_idx] = pressure_force + visc_force;
}

// -----------------------------------------------------------------------------
// PASS 5: Integrate
// -----------------------------------------------------------------------------
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let p_idx = global_id.x;
    if (p_idx >= params.num_particles) { return; }
    
    var p = particles[p_idx];
    
    let f = forces[p_idx];
    let accel = f / p.density + params.gravity;
    
    p.velocity += accel * params.dt;
    p.position += p.velocity * params.dt;
    
    // Bounds collision (simple box for now)
    let half_bounds = vec3<f32>(10.0, 10.0, 10.0);
    let damp = 0.5;
    
    if (p.position.y < 0.0) {
        p.position.y = 0.0;
        p.velocity.y *= -damp;
    }
    if (p.position.x < -half_bounds.x) {
        p.position.x = -half_bounds.x;
        p.velocity.x *= -damp;
    }
    if (p.position.x > half_bounds.x) {
        p.position.x = half_bounds.x;
        p.velocity.x *= -damp;
    }
    if (p.position.z < -half_bounds.z) {
        p.position.z = -half_bounds.z;
        p.velocity.z *= -damp;
    }
    if (p.position.z > half_bounds.z) {
        p.position.z = half_bounds.z;
        p.velocity.z *= -damp;
    }
    
    particles[p_idx] = p;
}
