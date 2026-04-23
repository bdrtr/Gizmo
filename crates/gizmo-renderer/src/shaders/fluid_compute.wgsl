struct FluidParticle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    pressure: f32,
    phase: u32,
    pad1: u32,
    pad2: u32,
    pad3: u32,
}

struct FluidCollider {
    position: vec3<f32>,
    radius: f32,
    velocity: vec3<f32>,
    padding: f32,
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
    pad1: f32,
    pad2: f32,
    pad3: f32,
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

// Math constants
const PI: f32 = 3.14159265359;

// ---- Helpers ----
fn get_cell_coord(pos: vec3<f32>) -> vec3<i32> {
    let local_pos = pos - params.bounds_min;
    return vec3<i32>(
        i32(local_pos.x / params.cell_size),
        i32(local_pos.y / params.cell_size),
        i32(local_pos.z / params.cell_size)
    );
}

fn get_cell_index(coord: vec3<i32>) -> i32 {
    if (coord.x < 0 || coord.y < 0 || coord.z < 0) { return -1; }
    if (coord.x >= i32(params.grid_size_x) || coord.y >= i32(params.grid_size_y) || coord.z >= i32(params.grid_size_z)) { return -1; }
    return coord.x + coord.y * i32(params.grid_size_x) + coord.z * i32(params.grid_size_x * params.grid_size_y);
}

// Poly6 Kernel for Density (3D Formulation)
fn W_poly6(r_sq: f32, h: f32) -> f32 {
    let h_sq = h * h;
    if (r_sq >= 0.0 && r_sq <= h_sq) {
        let diff = h_sq - r_sq;
        // 315 / (64 * pi * h^9) = 1.56668147106 / h^9
        let coeff = 1.56668147106 / (h * h_sq * h_sq * h_sq * h_sq);
        return coeff * diff * diff * diff;
    }
    return 0.0;
}

// Spiky Kernel Gradient for Pressure (3D Formulation)
fn gradW_spiky(r: vec3<f32>, r_len: f32, h: f32) -> vec3<f32> {
    if (r_len > 0.0 && r_len <= h) {
        let diff = h - r_len;
        // -45 / (pi * h^6) = -14.3239448783 / h^6
        let coeff = -14.3239448783 / (h * h * h * h * h * h);
        return (r / r_len) * coeff * diff * diff;
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// Viscosity Kernel Laplacian (3D Formulation)
fn laplacianW_viscosity(r_len: f32, h: f32) -> f32 {
    if (r_len > 0.0 && r_len <= h) {
        // 45 / (pi * h^6) = 14.3239448783 / h^6
        let coeff = 14.3239448783 / (h * h * h * h * h * h);
        return coeff * (h - r_len);
    }
    return 0.0;
}

// ---- PASS 0: CLEAR GRID ---- //
@compute @workgroup_size(64)
fn clear_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    let total_cells = params.grid_size_x * params.grid_size_y * params.grid_size_z;
    if (index >= total_cells) { return; }
    
    grid[index * 2u] = 0xFFFFFFFFu; // start_index
    grid[index * 2u + 1u] = 0xFFFFFFFFu; // end_index
}

// ---- PASS 1: HASH PARTICLES ---- //
@compute @workgroup_size(64)
fn hash_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pos = particles[index].position;
    let cell_coord = get_cell_coord(pos);
    let cell_idx = get_cell_index(cell_coord);
    
    var hash: u32 = 0xFFFFFFFFu;
    if (cell_idx != -1) {
        hash = u32(cell_idx);
    }
    
    sort_buffer[index].hash = hash;
    sort_buffer[index].index = index;
}

// ---- PASS 2: BITONIC SORT ---- //
@compute @workgroup_size(64)
fn bitonic_sort_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let i = global_id.x;
    let j = sort_params.j;
    let k = sort_params.k;
    
    let ixj = i ^ j;
    if (ixj > i) {
        let p1 = sort_buffer[i];
        let p2 = sort_buffer[ixj];
        
        let up = (i & k) == 0u;
        
        if ((up && p1.hash > p2.hash) || (!up && p1.hash < p2.hash)) {
            sort_buffer[i] = p2;
            sort_buffer[ixj] = p1;
        }
    }
}

// ---- PASS 3: GRID OFFSETS ---- //
@compute @workgroup_size(64)
fn grid_offsets_pass(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let hash = sort_buffer[index].hash;
    if (hash == 0xFFFFFFFFu) { return; }
    
    let prev_hash = select(0xFFFFFFFFu, sort_buffer[index - 1u].hash, index > 0u);
    if (hash != prev_hash) {
        grid[hash * 2u] = index; // start_index
    }
    
    let next_hash = select(0xFFFFFFFFu, sort_buffer[index + 1u].hash, index < params.num_particles - 1u);
    if (hash != next_hash) {
        grid[hash * 2u + 1u] = index; // end_index
    }
}

// ---- PASS 4: DENSITY & PRESSURE ---- //
@compute @workgroup_size(64)
fn calc_density(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pi = particles[index];
    let pos = pi.position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    
    let cell_coord = get_cell_coord(pos);
    var density: f32 = 0.0;
    
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let neighbor_coord = vec3<i32>(cell_coord.x + x, cell_coord.y + y, cell_coord.z + z);
                let neighbor_cell_idx = get_cell_index(neighbor_coord);
                if (neighbor_cell_idx != -1) {
                    let start_idx = grid[u32(neighbor_cell_idx) * 2u];
                    if (start_idx != 0xFFFFFFFFu) {
                        let end_idx = grid[u32(neighbor_cell_idx) * 2u + 1u];
                        for (var i = start_idx; i <= end_idx; i = i + 1u) {
                            let particle_idx = sort_buffer[i].index;
                            let pj = particles[particle_idx];
                            let r = pos - pj.position;
                            let r_sq = dot(r, r);
                            if (r_sq < h_sq) {
                                density += params.mass * W_poly6(r_sq, h);
                            }
                        }
                    }
                }
            }
        }
    }
    
    density = max(density, 0.0001);
    particles[index].density = density;
    
    // 3D Tait Equation: P = k * ((rho / rho0)^7 - 1)
    let ratio = density / params.rest_density;
    let ratio2 = ratio * ratio;
    let ratio4 = ratio2 * ratio2;
    let ratio7 = ratio4 * ratio2 * ratio;
    
    particles[index].pressure = max(params.gas_constant * (ratio7 - 1.0), 0.0);
}

// ---- PASS 5: FORCES & INTEGRATE ---- //
@compute @workgroup_size(64)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    let pos = pi.position;
    let vel = pi.velocity;
    let rho_i = pi.density;
    let press_i = pi.pressure;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    
    let cell_coord = get_cell_coord(pos);
    
    var f_press = vec3<f32>(0.0);
    var f_visc = vec3<f32>(0.0);
    
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let neighbor_coord = vec3<i32>(cell_coord.x + x, cell_coord.y + y, cell_coord.z + z);
                let neighbor_cell_idx = get_cell_index(neighbor_coord);
                if (neighbor_cell_idx != -1) {
                    let start_idx = grid[u32(neighbor_cell_idx) * 2u];
                    if (start_idx != 0xFFFFFFFFu) {
                        let end_idx = grid[u32(neighbor_cell_idx) * 2u + 1u];
                        for (var i = start_idx; i <= end_idx; i = i + 1u) {
                            let particle_idx = sort_buffer[i].index;
                            if (particle_idx != index) {
                                let pj = particles[particle_idx];
                                let r = pos - pj.position;
                                let r_len = length(r);
                                let rho_j = pj.density;
                                
                                if (r_len > 0.0 && r_len < h && rho_j > 0.0) {
                                    let press_term = (press_i + pj.pressure) / (2.0 * rho_j);
                                    f_press -= params.mass * press_term * gradW_spiky(r, r_len, h);
                                    
                                    let vel_diff = pj.velocity - vel;
                                    f_visc += params.viscosity * params.mass * (vel_diff / rho_j) * laplacianW_viscosity(r_len, h);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    let f_grav = vec3<f32>(0.0, -params.gravity, 0.0) * rho_i; 
    
    // Mouse Interaction
    var f_mouse = vec3<f32>(0.0);
    if (params.mouse_active > 0.5) {
        let r_mouse = pos - params.mouse_pos;
        let d_mouse = length(r_mouse);
        if (d_mouse > 0.0 && d_mouse < params.mouse_radius) {
            let push_force = params.mouse_dir * 5000.0 * rho_i;
            let repel_force = normalize(r_mouse) * 1000.0 * (params.mouse_radius - d_mouse) * rho_i;
            f_mouse = push_force + repel_force;
        }
    }
    
    let total_force = f_press + f_visc + f_grav + f_mouse;
    let acceleration = total_force / rho_i;
    
    pi.velocity += acceleration * params.dt;
    pi.position += pi.velocity * params.dt;
    
    // Boundary collision with restitution
    let restitution = 0.1;
    let b_min = params.bounds_min;
    let b_max = params.bounds_max;
    
    if (pi.position.x < b_min.x) { pi.position.x = b_min.x + 0.001; pi.velocity.x *= -restitution; }
    if (pi.position.x > b_max.x) { pi.position.x = b_max.x - 0.001; pi.velocity.x *= -restitution; }
    
    if (pi.position.y < b_min.y) { pi.position.y = b_min.y + 0.001; pi.velocity.y *= -restitution; }
    if (pi.position.y > b_max.y) { pi.position.y = b_max.y - 0.001; pi.velocity.y *= -restitution; }
    
    if (pi.position.z < b_min.z) { pi.position.z = b_min.z + 0.001; pi.velocity.z *= -restitution; }
    if (pi.position.z > b_max.z) { pi.position.z = b_max.z - 0.001; pi.velocity.z *= -restitution; }
    
    // Dynamic Collision Objects
    for (var i = 0u; i < params.num_colliders; i = i + 1u) {
        let col = colliders[i];
        let diff = pi.position - col.position;
        let dist = length(diff);
        if (dist > 0.0 && dist < col.radius) {
            let n = diff / dist;
            pi.position = col.position + n * (col.radius + 0.001);
            let relative_vel = pi.velocity - col.velocity;
            let v_dot_n = dot(relative_vel, n);
            if (v_dot_n < 0.0) {
                pi.velocity = pi.velocity - (1.0 + restitution) * v_dot_n * n;
            }
            pi.velocity += col.velocity * 0.8; 
        }
    }

    particles[index] = pi;
}
