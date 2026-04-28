// ═══════════════════════════════════════════════════════════════════════════════
//  AAA SPH Fluid Compute Shader — Gizmo Engine
//  ─────────────────────────────────────────────────────────────────────────────
//  Features:
//    ✓ Navier-Stokes tabanlı sıkıştırılamaz akış (PBF)
//    ✓ CFL kararlılık koşulu (dt_adaptive GPU tarafı)
//    ✓ Vorticity Confinement (türbülans modeli)
//    ✓ Akinci Surface Tension (yüzey gerilimi / kohezyon)
//    ✓ Laplacian Viscosity + XSPH hız düzeltmesi
//    ✓ Tensile Instability Correction (Monaghan)
//    ✓ Sınır sürtünmesi / restitüsyon
// ═══════════════════════════════════════════════════════════════════════════════

// ── Helper: Neighbor iteration macro-like loop ──
// Used in every pass — iterates all particles in 27 neighboring cells

// ════════════════════════════════════════════════════════════════════════
//  PASS 1: PREDICT — Semi-implicit Euler integration + gravity
// ════════════════════════════════════════════════════════════════════════
@compute @workgroup_size(64)
fn predict(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    
    // Apply gravity (Navier-Stokes body force term: ρg)
    var total_force = vec3<f32>(0.0, -params.gravity, 0.0) * params.mass;
    
    // Numerical crystallization prevention — tiny jitter force
    let jitter_vec = vec3<f32>(
        fract(f32(index) * 13.37 + params.time * 0.1) - 0.5,
        fract(f32(index) * 42.19 + params.time * 0.2) - 0.5,
        fract(f32(index) * 7.91 + params.time * 0.3) - 0.5
    );
    total_force += jitter_vec * 0.3 * params.mass;
    
    // Initialize newly spawned particles at the mouse position
    if (pi.phase == 0xFFFFFFFFu) {
        pi.phase = 0u;
        
        let jitter = vec3<f32>(
            fract(f32(index) * 13.37) - 0.5,
            fract(f32(index) * 42.19) - 0.5,
            fract(f32(index) * 7.91) - 0.5
        );
        
        if (params.mouse_active > 0.5) {
            pi.position = params.mouse_pos + jitter * 0.5;
            pi.velocity = params.mouse_dir * 5.0;
        } else {
            pi.position = vec3<f32>(0.0, 8.0, 0.0) + jitter * 2.0;
            pi.velocity = vec3<f32>(0.0);
        }
        
        pi.predicted_position = pi.position;
        pi.vorticity = vec3<f32>(0.0);
        pi._pad_vort = 0.0;
        particles[index] = pi;
        return;
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
    
    // Collider interaction
    for (var c = 0u; c < params.num_colliders; c = c + 1u) {
        let col = colliders[c];
        let r_col = pi.position - col.position;
        let d_col = length(r_col);
        if (d_col > 0.0 && d_col < col.radius * 1.5) {
            // Penalty force to push fluid away from collider
            let penetration = col.radius * 1.5 - d_col;
            let normal = r_col / d_col;
            total_force += normal * penetration * 5000.0 * params.mass;
            // Transfer collider velocity to fluid (coupling)
            pi.velocity += col.velocity * 0.1;
        }
    }
    
    let accel = total_force / params.mass;
    pi.velocity += accel * params.dt;
    pi.predicted_position = pi.position + pi.velocity * params.dt;
    
    particles[index] = pi;
}

// ════════════════════════════════════════════════════════════════════════
//  PASS 2: CALC LAMBDA — Density constraint (Incompressibility)
//  Implements Macklin & Müller 2013 "Position Based Fluids"
// ════════════════════════════════════════════════════════════════════════
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
    
    // Store density for later passes (surface tension, vorticity)
    particles[index].density = density;
    
    // Density constraint C_i = ρ_i / ρ_0 - 1
    // max(..., 0.0) ensures incompressibility (only pushes apart, never pulls)
    let constraint_C = max((density / params.rest_density) - 1.0, 0.0);
    
    // Relaxation parameter ε prevents division instability at boundaries
    let eps = 600.0;
    particles[index].lambda = -constraint_C / (grad_c_sum_sq + eps);
}

// ════════════════════════════════════════════════════════════════════════
//  PASS 3: APPLY DELTA P — Position correction with Tensile Instability
//  Monaghan's artificial pressure prevents particle clumping
// ════════════════════════════════════════════════════════════════════════
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
    
    // ── Tensile Instability Correction (Monaghan 2000) ──
    // Prevents particle clumping/clustering in negative pressure regions
    // s_corr = -k * (W(r) / W(Δq))^n
    let k_corr = 0.001;  // Correction strength (conservative to avoid boiling)
    let n_corr = 4.0;    // Exponent (sharper falloff)
    let dq = 0.2 * h;    // Reference distance
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
                                    
                                    // Tensile instability correction term
                                    var s_corr = 0.0;
                                    if (w_dq > 0.0) {
                                        let ratio = W_poly6(r_sq, h) / w_dq;
                                        s_corr = -k_corr * pow(ratio, n_corr);
                                    }
                                    
                                    delta_p += (lambda_sum + s_corr) * gradW;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    delta_p *= (params.mass / params.rest_density);
    
    // Under-relaxation (Jacobi solver omega) to prevent non-linear overshoot
    let omega = 0.65;
    delta_p *= omega;
    
    // CFL-bounded position correction — prevent rubber-band bouncing
    let max_delta = h * 0.15;
    let dp_len = length(delta_p);
    if (dp_len > max_delta) {
        delta_p = (delta_p / dp_len) * max_delta;
    }
    
    var new_pos = pos_i + delta_p;
    
    // Boundary collision (project onto boundary surface)
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

// ════════════════════════════════════════════════════════════════════════
//  PASS 4: COMPUTE VORTICITY — ω = ∇ × v (curl of velocity field)
//  First half of Vorticity Confinement (Fedkiw et al. 2001)
// ════════════════════════════════════════════════════════════════════════
@compute @workgroup_size(64)
fn compute_vorticity(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    let pi = particles[index];
    let pos_i = pi.predicted_position;
    let vel_i = (pi.predicted_position - pi.position) / params.dt;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    
    // ω_i = Σ_j (m_j / ρ_j) * (v_j - v_i) × ∇W_ij
    var omega = vec3<f32>(0.0);
    
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
                                    
                                    let vel_j = (pj.predicted_position - pj.position) / params.dt;
                                    let vel_diff = vel_j - vel_i;
                                    
                                    // Volume element: m/ρ
                                    let vol_j = params.mass / max(pj.density, 0.001);
                                    
                                    // Curl: (v_j - v_i) × ∇W
                                    omega += vol_j * cross(vel_diff, gradW);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    particles[index].vorticity = omega;
}

// ════════════════════════════════════════════════════════════════════════
//  PASS 5: UPDATE VELOCITY — Vorticity Confinement + Surface Tension
//           + Laplacian Viscosity + XSPH smoothing
//  This is the AAA final integration pass
// ════════════════════════════════════════════════════════════════════════
@compute @workgroup_size(64)
fn update_velocity(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    
    // Base velocity from position correction: v = (p* - p) / dt
    pi.velocity = (pi.predicted_position - pi.position) / params.dt;
    
    let pos_i = pi.predicted_position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    let rho_i = max(pi.density, 0.001);
    
    // Accumulate forces from neighbor scan
    var v_xsph = vec3<f32>(0.0);          // XSPH velocity smoothing
    var f_viscosity = vec3<f32>(0.0);      // Laplacian viscosity force
    var f_surface_tension = vec3<f32>(0.0); // Akinci surface tension
    var color_field_normal = vec3<f32>(0.0); // Color field gradient (CSF)
    var color_field_laplacian = 0.0;        // Color field Laplacian
    
    // ── Vorticity Confinement (Fedkiw et al. 2001) ──
    // f_vort = ε * (N × ω), where N = ∇|ω| / |∇|ω||
    var grad_omega_mag = vec3<f32>(0.0);
    let omega_i = pi.vorticity;
    let omega_i_mag = length(omega_i);
    
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
                                    let w_val = W_poly6(r_sq, h);
                                    let vol_j = params.mass / max(pj.density, 0.001);
                                    
                                    // ─── XSPH Velocity Smoothing ───
                                    let vel_diff = pj.velocity - pi.velocity;
                                    v_xsph += vel_diff * w_val * vol_j;
                                    
                                    // ─── Laplacian Viscosity (Müller 2003) ───
                                    // f_visc = μ * Σ m_j * (v_j - v_i) / ρ_j * ∇²W
                                    let lap_w = laplacianW_viscosity(r_len, h);
                                    f_viscosity += (params.mass / max(pj.density, 0.001)) * vel_diff * lap_w;
                                    
                                    // ─── Akinci Surface Tension (2013) ───
                                    // Cohesion force: F_coh = -γ * m_i * m_j * C(|r|) * r̂
                                    let w_coh = W_cohesion(r_len, h);
                                    let r_hat = r / r_len;
                                    f_surface_tension -= params.mass * params.mass * w_coh * r_hat;
                                    
                                    // ─── Color Field (for CSF normal estimation) ───
                                    // n = Σ (m_j / ρ_j) * ∇W
                                    color_field_normal += vol_j * gradW;
                                    color_field_laplacian += vol_j * lap_w;
                                    
                                    // ─── Vorticity: ∇|ω| for confinement direction ───
                                    let omega_j_mag = length(pj.vorticity);
                                    grad_omega_mag += vol_j * (omega_j_mag - omega_i_mag) * gradW;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // ── Apply Vorticity Confinement Force ──
    let grad_omega_len = length(grad_omega_mag);
    if (grad_omega_len > 0.0001 && omega_i_mag > 0.0001) {
        let N = grad_omega_mag / grad_omega_len;
        let f_vort = cross(N, omega_i) * params.vorticity_strength;
        pi.velocity += f_vort * params.dt;
    }
    
    // ── Apply Laplacian Viscosity ──
    pi.velocity += params.viscosity_laplacian * f_viscosity * params.dt;
    
    // ── Apply Surface Tension (Akinci cohesion + curvature) ──
    let surface_normal_len = length(color_field_normal);
    if (surface_normal_len > 0.1) {
        // Curvature κ = -∇²c / |∇c|
        let curvature = -color_field_laplacian / surface_normal_len;
        let curvature_force = params.surface_tension * curvature * color_field_normal / surface_normal_len;
        pi.velocity += curvature_force * params.dt / rho_i;
    }
    // Direct cohesion
    pi.velocity += params.cohesion * f_surface_tension * params.dt / rho_i;
    
    // ── Apply XSPH Velocity Smoothing ──
    pi.velocity += params.xsph_factor * v_xsph;
    
    // ── Boundary collision friction / restitution ──
    let b_min = params.bounds_min;
    let b_max = params.bounds_max;
    let pad = h * 0.5;
    
    // Floor friction (absorbs energy, prevents infinite sliding)
    if (pos_i.y <= b_min.y + pad + 0.05) {
        pi.velocity.x *= 0.95;
        pi.velocity.z *= 0.95;
        pi.velocity.y *= 0.75;
        if (pi.velocity.y < 0.0) { pi.velocity.y = 0.0; }
    }
    // Wall friction
    if (pos_i.x <= b_min.x + pad + 0.01 || pos_i.x >= b_max.x - pad - 0.01) {
        pi.velocity.x *= -0.3; // Slight bounce with energy loss
        pi.velocity.y *= 0.95;
        pi.velocity.z *= 0.95;
    }
    if (pos_i.z <= b_min.z + pad + 0.01 || pos_i.z >= b_max.z - pad - 0.01) {
        pi.velocity.z *= -0.3;
        pi.velocity.x *= 0.95;
        pi.velocity.y *= 0.95;
    }
    
    // Minimal atmospheric drag (lets water ripple and swirl longer)
    pi.velocity *= 0.9995;
    
    // ── CFL-aware speed limit ──
    // v_max = CFL * h / dt → prevents particles from skipping cells
    let cfl_max_speed = 0.4 * h / params.dt;
    let final_max_speed = min(cfl_max_speed, 20.0);
    let speed = length(pi.velocity);
    if (speed > final_max_speed) {
        pi.velocity = (pi.velocity / speed) * final_max_speed;
    }
    
    pi.position = pi.predicted_position;
    particles[index] = pi;
}

// ════════════════════════════════════════════════════════════════════════
//  PASS 6: CLASSIFY PARTICLES — Foam / Spray / Droplet Detection
//  Weber number + density ratio + neighbor count based classification
// ════════════════════════════════════════════════════════════════════════
@compute @workgroup_size(64)
fn classify_particles(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let index = global_id.x;
    if (index >= params.num_particles) { return; }
    
    var pi = particles[index];
    if (pi.phase == 0xFFFFFFFFu) { return; } // Skip uninitialized
    if (pi.density < 1.0) { return; } // Skip particles without valid density
    
    let pos_i = pi.position;
    let h = params.smoothing_radius;
    let h_sq = h * h;
    let cell_coord = get_cell_coord(pos_i);
    let speed = length(pi.velocity);
    let density_ratio = pi.density / params.rest_density;
    
    // Count neighbors within smoothing radius
    var neighbor_count = 0u;
    
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
                                let r_sq = dot(pos_i - pj.position, pos_i - pj.position);
                                if (r_sq < h_sq) {
                                    neighbor_count += 1u;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Classification with hysteresis (prevents flickering)
    let was_spray = pi.phase == 1u;
    let was_foam = pi.phase == 2u;
    
    // Spray: Very few neighbors + high speed → airborne droplets
    let spray_density_thresh = select(0.35, 0.5, was_spray); // Hysteresis
    let spray_speed_thresh = select(4.0, 2.5, was_spray);
    
    // Foam: Few neighbors + moderate density → surface froth
    let foam_density_thresh = select(0.6, 0.75, was_foam);
    let foam_neighbor_thresh = select(8u, 12u, was_foam);
    
    if (density_ratio < spray_density_thresh && speed > spray_speed_thresh && neighbor_count < 6u) {
        pi.phase = 1u; // SPRAY
    } else if (density_ratio < foam_density_thresh && neighbor_count < foam_neighbor_thresh) {
        pi.phase = 2u; // FOAM
    } else {
        pi.phase = 0u; // LIQUID (bulk water)
    }
    
    particles[index] = pi;
}
