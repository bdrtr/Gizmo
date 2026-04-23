// ---- PASS 1: PREDICT ---- //
@compute @workgroup_size(64)
fn predict(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    
    // Apply gravity
    var total_force = vec3<f32>(0.0, -params.gravity, 0.0) * params.mass;
    
    // Jitter force to prevent perfect crystallization (Numerical Crystallization fix)
    let jitter_vec = vec3<f32>(
        fract(f32(index) * 13.37) - 0.5,
        fract(f32(index) * 42.19) - 0.5,
        fract(f32(index) * 7.91) - 0.5
    );
    total_force += jitter_vec * 0.5 * params.mass; // Tiny random force
    
    // Initialize newly spawned particles at the mouse position
    if (pi.phase == 0xFFFFFFFFu) {
        pi.phase = 0u; // Mark as initialized
        
        let jitter = vec3<f32>(
            fract(f32(index) * 13.37) - 0.5,
            fract(f32(index) * 42.19) - 0.5,
            fract(f32(index) * 7.91) - 0.5
        );
        
        if (params.mouse_active > 0.5) {
            pi.position = params.mouse_pos + jitter * 0.5;
            pi.velocity = params.mouse_dir * 5.0; // Shoot out from the mouse
        } else {
            pi.position = vec3<f32>(0.0, 8.0, 0.0) + jitter * 2.0; // Fallback position
            pi.velocity = vec3<f32>(0.0);
        }
        
        pi.predicted_position = pi.position;
        particles[index] = pi;
        return; // Skip normal physics for this frame since it was just initialized
    }
    
    // Mouse interaction (gentle push for existing active particles)
    if (params.mouse_active > 0.5) {
        let r_mouse = pi.position - params.mouse_pos;
        let d_mouse = length(r_mouse);
        if (d_mouse > 0.0 && d_mouse < params.mouse_radius) {
            let push_force = params.mouse_dir * 3000.0 * params.mass;
            total_force += push_force;
        }
    }
    
    let accel = total_force / params.mass;
    pi.velocity += accel * params.dt;
    pi.predicted_position = pi.position + pi.velocity * params.dt;
    
    particles[index] = pi;
}

// ---- PASS 2: CALC LAMBDA ---- //
@compute @workgroup_size(64)
fn calc_lambda(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pi = particles[index];
    let pos_i = pi.predicted_position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    
    var density = params.mass * W_poly6(0.0, h);
    var grad_c_sum_sq = 0.0;
    var grad_c_i = vec3<f32>(0.0);
    
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
                                let r = pos_i - pj.predicted_position;
                                let r_sq = dot(r, r);
                                
                                if (r_sq < h_sq) {
                                    density += params.mass * W_poly6(r_sq, h);
                                    let r_len = max(sqrt(r_sq), 0.00001);
                                    let gradW = gradW_spiky(r, r_len, h);
                                    let grad_c_j = -(params.mass / params.rest_density) * gradW;
                                    grad_c_sum_sq += dot(grad_c_j, grad_c_j);
                                    grad_c_i -= grad_c_j;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    grad_c_sum_sq += dot(grad_c_i, grad_c_i);
    
    // Density constraint C_i = rho_i / rho_0 - 1
    // max(..., 0.0) ensures it only pushes particles apart (fluid is incompressible, not extensible)
    let constraint_C = max((density / params.rest_density) - 1.0, 0.0);
    
    // Industry standard relaxation parameter epsilon to prevent rigid bouncing at boundaries
    let eps = 2000.0; 
    particles[index].lambda = -constraint_C / (grad_c_sum_sq + eps);
}

// ---- PASS 3: APPLY DELTA P ---- //
@compute @workgroup_size(64)
fn apply_delta_p(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    let pos_i = pi.predicted_position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    
    var delta_p = vec3<f32>(0.0);
    
    // Removed Tensile Instability Correction because k=0.1 assumes a different lambda scale.
    let dq = 0.07;
    let w_dq = W_poly6(dq * dq, h);
    
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
                                let r = pos_i - pj.predicted_position;
                                let r_sq = dot(r, r);
                                
                                if (r_sq < h_sq) {
                                    let r_len = max(sqrt(r_sq), 0.00001);
                                    let gradW = gradW_spiky(r, r_len, h);
                                    let lambda_sum = pi.lambda + pj.lambda;
                                    
                                    // Removed artificial pressure (s_corr) because it adds chaotic kinetic energy (boiling)
                                    // and requires heavy damping to control.
                                    
                                    delta_p += lambda_sum * gradW;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    delta_p *= (params.mass / params.rest_density);
    
    // Under-relaxation (Omega) to prevent Jacobi non-linear overshoot
    let omega = 0.5;
    delta_p *= omega;
    
    // CFL Limit: tightly bound position correction to prevent rubber-band bouncing
    let max_delta = h * 0.1;
    let dp_len = length(delta_p);
    if (dp_len > max_delta) {
        delta_p = (delta_p / dp_len) * max_delta;
    }
    
    var new_pos = pos_i + delta_p;
    
    // Boundary collision (Muller 2013: project onto boundary)
    let b_min = params.bounds_min;
    let b_max = params.bounds_max;
    let padding = params.smoothing_radius * 0.5;
    let jitter = fract(f32(index) * 13.37) * 0.001;
    
    if (new_pos.x < b_min.x + padding) { new_pos.x = b_min.x + padding + jitter; }
    if (new_pos.x > b_max.x - padding) { new_pos.x = b_max.x - padding - jitter; }
    if (new_pos.y < b_min.y + padding) { new_pos.y = b_min.y + padding + jitter; }
    if (new_pos.y > b_max.y - padding) { new_pos.y = b_max.y - padding - jitter; }
    if (new_pos.z < b_min.z + padding) { new_pos.z = b_min.z + padding + jitter; }
    if (new_pos.z > b_max.z - padding) { new_pos.z = b_max.z - padding - jitter; }
    
    pi.predicted_position = new_pos;
    particles[index] = pi;
}

// ---- PASS 4: UPDATE VELOCITY ---- //
@compute @workgroup_size(64)
fn update_velocity(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    
    // Update velocity v = (p* - p) / dt
    pi.velocity = (pi.predicted_position - pi.position) / params.dt;
    
    // Industry standard XSPH Viscosity (Muller 2013)
    let pos_i = pi.predicted_position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    
    var v_visc = vec3<f32>(0.0);
    
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
                                let r = pos_i - pj.predicted_position;
                                let r_sq = dot(r, r);
                                
                                if (r_sq < h_sq) {
                                    let w_val = W_poly6(r_sq, h);
                                    let vel_diff = pj.velocity - pi.velocity;
                                    // VERY IMPORTANT: MUST scale by mass / rest_density
                                    // Otherwise W sums to 1000/0.457 = ~2000, causing a 2000x velocity explosion!
                                    v_visc += vel_diff * w_val * (params.mass / params.rest_density);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Apply weaker XSPH Viscosity for a more liquid/splashy feel (was 0.1)
    pi.velocity += 0.02 * v_visc; 
    
    // Boundary collision friction / restitution (absorbs shockwaves at corners)
    let b_min = params.bounds_min;
    let b_max = params.bounds_max;
    let padding = h * 0.5;
    if (pos_i.y <= b_min.y + padding + 0.05) {
        pi.velocity.x *= 0.95;
        pi.velocity.z *= 0.95;
        pi.velocity.y *= 0.8; // Absorb bounce energy less to allow splashing
        if (pi.velocity.y < 0.0) { pi.velocity.y = 0.0; }
    }
    if (pos_i.x <= b_min.x + padding + 0.01 || pos_i.x >= b_max.x - padding - 0.01) {
        pi.velocity.y *= 0.95;
        pi.velocity.z *= 0.95;
    }
    if (pos_i.z <= b_min.z + padding + 0.01 || pos_i.z >= b_max.z - padding - 0.01) {
        pi.velocity.x *= 0.95;
        pi.velocity.y *= 0.95;
    }
    
    // Global atmospheric drag / damping reduced drastically to let water ripple longer
    pi.velocity *= 0.999;
    
    // Final speed limit for general engine stability
    let speed = length(pi.velocity);
    let max_speed = 15.0; // Allowed slightly faster movement
    if (speed > max_speed) {
        pi.velocity = (pi.velocity / speed) * max_speed;
    }
    
    pi.position = pi.predicted_position;
    particles[index] = pi;
}
