// =========================================================================
// FEM (Finite Element Method) Soft Body Physics Compute Shader
// Neo-Hookean Hyperelastic Material Model
// =========================================================================

struct SoftBodyNode {
    position_mass: vec4<f32>,
    velocity_fixed: vec4<f32>, // w is u32 cast to f32
    force_x: atomic<i32>,
    force_y: atomic<i32>,
    force_z: atomic<i32>,
    _pad: i32,
}

struct Tetrahedron {
    indices: vec4<u32>,
    inv_rest_col0: vec4<f32>,
    inv_rest_col1: vec4<f32>,
    inv_rest_col2: vec4<f32>,
    rest_volume_pad: vec4<f32>,
}

struct FEMParams {
    properties: vec4<f32>, // dt, mu, lambda, damping
    gravity: vec4<f32>,
    counts: vec4<u32>, // num_nodes, num_elements, _, _
}

struct GpuCollider {
    shape_type: u32, // 0=Plane, 1=Sphere
    radius: f32,
    _pad0: u32,
    _pad1: u32,
    position: vec4<f32>,
    normal: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: FEMParams;
@group(0) @binding(1) var<storage, read_write> nodes: array<SoftBodyNode>;
@group(0) @binding(2) var<storage, read> elements: array<Tetrahedron>;
@group(0) @binding(3) var<storage, read> colliders: array<GpuCollider>;

// Encode a force component into the fixed-point i32 accumulator, clamping to just
// inside the i32 range first. WGSL i32() of an out-of-range float is undefined and
// the accumulator saturates near ±2.1e9, so an unclamped stiff-material force could
// wrap it (flipping sign, injecting energy). In-range values are bit-identical.
fn enc(value: f32, scale: f32) -> i32 {
    return i32(clamp(value * scale, -2.0e9, 2.0e9));
}

// Pass 1: Clear forces
@compute @workgroup_size(256)
fn clear_forces(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.counts.x) { return; }
    
    // Yalnızca dış kuvvetleri sıfırla (gravity ekle)
    let gravity_force = params.gravity.xyz * nodes[idx].position_mass.w;

    atomicStore(&nodes[idx].force_x, enc(gravity_force.x, 100000.0));
    atomicStore(&nodes[idx].force_y, enc(gravity_force.y, 100000.0));
    atomicStore(&nodes[idx].force_z, enc(gravity_force.z, 100000.0));
}

// Pass 2: Calculate Piola-Kirchhoff Stress & Apply Element Forces
// atomicFetchAddFloat is not standard WGSL yet, but we will use an atomic array 
// for thread-safe accumulation if necessary. For now, since Gizmo handles 
// massive scale physics via iterative solver, we'll implement the stress calculation.
// (In a true parallel GPU FEM, you either color the mesh to avoid write-conflicts 
// or use atomic floats if the extension is supported).
@compute @workgroup_size(256)
fn compute_stress(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let elem_idx = global_id.x;
    if (elem_idx >= params.counts.y) { return; }
    
    let elem = elements[elem_idx];
    
    let i0 = elem.indices.x;
    let i1 = elem.indices.y;
    let i2 = elem.indices.z;
    let i3 = elem.indices.w;
    
    let p0 = nodes[i0].position_mass.xyz;
    let p1 = nodes[i1].position_mass.xyz;
    let p2 = nodes[i2].position_mass.xyz;
    let p3 = nodes[i3].position_mass.xyz;
    
    // Deformed Shape Matrix (Ds)
    let ds = mat3x3<f32>(
        p1 - p0,
        p2 - p0,
        p3 - p0
    );
    
    // Inverse Rest Shape Matrix (Dm^-1)
    let inv_dm = mat3x3<f32>(
        elem.inv_rest_col0.xyz,
        elem.inv_rest_col1.xyz,
        elem.inv_rest_col2.xyz
    );
    
    // Deformation Gradient (F = Ds * Dm^-1)
    let F = ds * inv_dm;
    
    // Cross products of columns for J and Cofactor matrix
    let c0 = F[0];
    let c1 = F[1];
    let c2 = F[2];
    
    let J = dot(c0, cross(c1, c2)); // Volume change ratio
    // Sign-PRESERVING clamp away from the singular J=0. `max(J, 0.01)` used to
    // force J positive, which flipped the sign of F^-T = cofactor(F)/J for
    // inverted (J < 0) elements, so their Piola stress pushed the wrong way and
    // they could never un-invert. Keep |J| >= 0.01 but preserve the sign.
    let J_safe = select(min(J, -0.01), max(J, 0.01), J >= 0.0);

    // F^-T is exactly Cofactor(F) / J
    let F_inv_T = mat3x3<f32>(
        cross(c1, c2),
        cross(c2, c0),
        cross(c0, c1)
    ) * (1.0 / J_safe);

    // 1st Piola-Kirchhoff Stress Tensor (P)
    // P = mu * (F - F^-T) + lambda * ln(J) * F^-T
    // ln|J| (abs) keeps the volumetric term defined for inverted elements.
    let ln_J = log(abs(J_safe));
    let P = params.properties.y * (F - F_inv_T) + (params.properties.z * ln_J) * F_inv_T;
    
    // Element Force Matrix (H = -V0 * P * Dm^-T)
    let H = -elem.rest_volume_pad.x * (P * transpose(inv_dm));
    
    // Extract forces for nodes 1, 2, 3
    let f1 = H[0];
    let f2 = H[1];
    let f3 = H[2];
    
    // f0 = -(f1 + f2 + f3) to maintain equilibrium
    let f0 = -(f1 + f2 + f3);
    
    // Write forces back to nodes using atomic operations.
    // The forces are accumulated in a fixed-point i32 (scale 1e5). WGSL i32() of
    // an out-of-range float is undefined, and the accumulator saturates near
    // ±2.1e9, so a stiff material / large deformation could wrap it — flipping the
    // force sign and injecting energy. `enc()` clamps to just inside i32 range
    // before the cast; in-range forces are unaffected (bit-identical).
    let force_scale = 100000.0;

    atomicAdd(&nodes[i0].force_x, enc(f0.x, force_scale));
    atomicAdd(&nodes[i0].force_y, enc(f0.y, force_scale));
    atomicAdd(&nodes[i0].force_z, enc(f0.z, force_scale));

    atomicAdd(&nodes[i1].force_x, enc(f1.x, force_scale));
    atomicAdd(&nodes[i1].force_y, enc(f1.y, force_scale));
    atomicAdd(&nodes[i1].force_z, enc(f1.z, force_scale));

    atomicAdd(&nodes[i2].force_x, enc(f2.x, force_scale));
    atomicAdd(&nodes[i2].force_y, enc(f2.y, force_scale));
    atomicAdd(&nodes[i2].force_z, enc(f2.z, force_scale));

    atomicAdd(&nodes[i3].force_x, enc(f3.x, force_scale));
    atomicAdd(&nodes[i3].force_y, enc(f3.y, force_scale));
    atomicAdd(&nodes[i3].force_z, enc(f3.z, force_scale));
}

// Pass 3: Integration (Symplectic Euler)
@compute @workgroup_size(256)
fn integrate(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.counts.x) { return; }

    let is_fixed = u32(nodes[idx].velocity_fixed.w);
    if (is_fixed == 1u) { return; }
    
    let fx = f32(atomicLoad(&nodes[idx].force_x)) / 100000.0;
    let fy = f32(atomicLoad(&nodes[idx].force_y)) / 100000.0;
    let fz = f32(atomicLoad(&nodes[idx].force_z)) / 100000.0;
    let total_force = vec3<f32>(fx, fy, fz);
    
    let mass = nodes[idx].position_mass.w;
    var velocity = nodes[idx].velocity_fixed.xyz;
    var position = nodes[idx].position_mass.xyz;
    
    let dt = params.properties.x;
    
    // Integrate Velocity (a = F / m)
    let accel = total_force / mass;
    velocity += accel * dt;
    
    // Default Global Damping (Energy loss)
    velocity *= params.properties.w;
    var future_pos = position + velocity * dt;

    // Advanced GPU-Side Collision Detection
    let num_colliders = params.counts.z;
    for (var i = 0u; i < num_colliders; i = i + 1u) {
        let col = colliders[i];
        
        if (col.shape_type == 0u) {
            // Plane Collision
            // col.position.xyz is a point on the plane
            // col.normal.xyz is the plane normal
            let plane_normal = col.normal.xyz;
            let to_node = future_pos - col.position.xyz;
            let dist = dot(to_node, plane_normal);
            
            if (dist < 0.0) {
                // Resolve interpenetration
                future_pos = future_pos - plane_normal * dist;
                
                // Reflect velocity
                let v_dot_n = dot(velocity, plane_normal);
                if (v_dot_n < 0.0) {
                    let normal_vel = plane_normal * v_dot_n;
                    let tangent_vel = velocity - normal_vel;
                    
                    // Bounce (-0.2 restitution) and Friction (0.8)
                    velocity = tangent_vel * 0.8 - normal_vel * 0.2;
                }
            }
        } else if (col.shape_type == 1u) {
            // Sphere Collision
            let diff = future_pos - col.position.xyz;
            let dist_sq = dot(diff, diff);
            let r = col.radius;
            
            if (dist_sq < r * r && dist_sq > 0.0001) {
                let dist = sqrt(dist_sq);
                let normal = diff / dist;
                let penetration = r - dist;
                
                future_pos = future_pos + normal * penetration;
                
                let v_dot_n = dot(velocity, normal);
                if (v_dot_n < 0.0) {
                    let normal_vel = normal * v_dot_n;
                    let tangent_vel = velocity - normal_vel;
                    velocity = tangent_vel * 0.9 - normal_vel * 0.5;
                }
            }
        }
    }
    
    position = future_pos;
    
    nodes[idx].velocity_fixed = vec4<f32>(velocity.x, velocity.y, velocity.z, f32(is_fixed));
    nodes[idx].position_mass = vec4<f32>(position.x, position.y, position.z, mass);
}
