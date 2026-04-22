struct FluidParticle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    pressure: f32,
    force: vec3<f32>,
    next_index: i32,
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

@group(0) @binding(0) var<uniform> params: FluidParams;
@group(0) @binding(1) var<storage, read_write> particles: array<FluidParticle>;
@group(0) @binding(2) var<storage, read_write> grid: array<atomic<i32>>;
@group(0) @binding(3) var<storage, read> colliders: array<FluidCollider>;

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

// Poly6 Kernel for Density
fn W_poly6(r_sq: f32, h: f32) -> f32 {
    let h_sq = h * h;
    if (r_sq >= 0.0 && r_sq <= h_sq) {
        let max_val = h_sq - r_sq;
        return (315.0 / (64.0 * PI * pow(h, 9.0))) * max_val * max_val * max_val;
    }
    return 0.0;
}

// Spiky Kernel Gradient for Pressure
fn gradW_spiky(r: vec3<f32>, r_len: f32, h: f32) -> vec3<f32> {
    if (r_len > 0.0 && r_len <= h) {
        let max_val = h - r_len;
        let coeff = - (45.0 / (PI * pow(h, 6.0))) * max_val * max_val;
        return (r / r_len) * coeff;
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// Viscosity Kernel Laplacian
fn laplacianW_viscosity(r_len: f32, h: f32) -> f32 {
    if (r_len > 0.0 && r_len <= h) {
        return (45.0 / (PI * pow(h, 6.0))) * (h - r_len);
    }
    return 0.0;
}

// ---- PASS 0: CLEAR GRID ---- //
@compute @workgroup_size(64)
fn clear_grid(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    let total_cells = params.grid_size_x * params.grid_size_y * params.grid_size_z;
    if (index >= total_cells) { return; }
    
    atomicStore(&grid[index], -1);
}

// ---- PASS 1: HASH PARTICLES ---- //
@compute @workgroup_size(64)
fn hash_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pos = particles[index].position;
    let cell_coord = get_cell_coord(pos);
    let cell_idx = get_cell_index(cell_coord);
    
    if (cell_idx != -1) {
        let old_head = atomicExchange(&grid[cell_idx], i32(index));
        particles[index].next_index = old_head;
    } else {
        particles[index].next_index = -1;
    }
}

// ---- PASS 2: DENSITY & PRESSURE ---- //
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
                    var curr_idx = atomicLoad(&grid[neighbor_cell_idx]);
                    while (curr_idx != -1) {
                        let pj = particles[curr_idx];
                        let r = pos - pj.position;
                        let r_sq = dot(r, r);
                        if (r_sq < h_sq) {
                            density += params.mass * W_poly6(r_sq, h);
                        }
                        curr_idx = pj.next_index;
                    }
                }
            }
        }
    }
    
    // Self density is included because curr_idx will eventually be index, and r_sq = 0.
    
    // Prevent divide by zero
    density = max(density, 0.0001);
    particles[index].density = density;
    
    // Tait Equation / Pressure formulation
    // P = k * (density - rest_density)
    particles[index].pressure = max(params.gas_constant * (density - params.rest_density), 0.0);
}

// ---- PASS 3: FORCES ---- //
@compute @workgroup_size(64)
fn calc_forces(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pi = particles[index];
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
                    var curr_idx = atomicLoad(&grid[neighbor_cell_idx]);
                    while (curr_idx != -1) {
                        if (curr_idx != i32(index)) {
                            let pj = particles[curr_idx];
                            let r = pos - pj.position;
                            let r_len = length(r);
                            let rho_j = pj.density;
                            
                            if (r_len > 0.0 && r_len < h && rho_j > 0.0) {
                                // Pressure Force
                                let press_term = (press_i + pj.pressure) / (2.0 * rho_j);
                                f_press -= params.mass * press_term * gradW_spiky(r, r_len, h);
                                
                                // Viscosity Force
                                let vel_diff = pj.velocity - vel;
                                f_visc += params.viscosity * params.mass * (vel_diff / rho_j) * laplacianW_viscosity(r_len, h);
                            }
                        }
                        curr_idx = particles[curr_idx].next_index;
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
            // Apply push force along mouse vector direction + repel
            let push_force = params.mouse_dir * 5000.0 * rho_i;
            let repel_force = normalize(r_mouse) * 1000.0 * (params.mouse_radius - d_mouse) * rho_i;
            f_mouse = push_force + repel_force;
        }
    }
    
    particles[index].force = f_press + f_visc + f_grav + f_mouse;
}

// ---- PASS 4: INTEGRATE ---- //
@compute @workgroup_size(64)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var p = particles[index];
    
    let acceleration = p.force / p.density;
    p.velocity += acceleration * params.dt;
    p.position += p.velocity * params.dt;
    
    // Boundary collision with restitution
    let restitution = 0.5;
    let b_min = params.bounds_min;
    let b_max = params.bounds_max;
    
    if (p.position.x < b_min.x) { p.position.x = b_min.x + 0.001; p.velocity.x *= -restitution; }
    if (p.position.x > b_max.x) { p.position.x = b_max.x - 0.001; p.velocity.x *= -restitution; }
    
    if (p.position.y < b_min.y) { p.position.y = b_min.y + 0.001; p.velocity.y *= -restitution; }
    if (p.position.y > b_max.y) { p.position.y = b_max.y - 0.001; p.velocity.y *= -restitution; }
    
    if (p.position.z < b_min.z) { p.position.z = b_min.z + 0.001; p.velocity.z *= -restitution; }
    if (p.position.z > b_max.z) { p.position.z = b_max.z - 0.001; p.velocity.z *= -restitution; }
    
    // Dynamic Collision Objects (Balls thrown by user)
    for (var i = 0u; i < params.num_colliders; i = i + 1u) {
        let col = colliders[i];
        let diff = p.position - col.position;
        let dist = length(diff);
        if (dist > 0.0 && dist < col.radius) {
            let n = diff / dist;
            // Push particle out of ball
            p.position = col.position + n * (col.radius + 0.001);
            // Transfer momentum/velocity from ball to water! (Splash effect)
            let relative_vel = p.velocity - col.velocity;
            // Reflect velocity
            let v_dot_n = dot(relative_vel, n);
            if (v_dot_n < 0.0) {
                p.velocity = p.velocity - (1.0 + restitution) * v_dot_n * n;
            }
            // Add extra splash boost if hit parameter is fast
            p.velocity += col.velocity * 0.8; 
        }
    }

    particles[index] = p;
}
