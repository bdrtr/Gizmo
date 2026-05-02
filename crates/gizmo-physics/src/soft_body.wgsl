// soft_body.wgsl

struct SoftBodyNode {
    position: vec3<f32>,
    mass: f32,
    velocity: vec3<f32>,
    is_fixed: u32,
}

struct Tetrahedron {
    i0: u32,
    i1: u32,
    i2: u32,
    i3: u32,
    inv_rest_matrix_col0: vec3<f32>,
    pad0: f32,
    inv_rest_matrix_col1: vec3<f32>,
    pad1: f32,
    inv_rest_matrix_col2: vec3<f32>,
    pad2: f32,
    rest_volume: f32,
    pad3: vec3<f32>,
}

struct Parameters {
    dt: f32,
    mu: f32,
    lambda: f32,
    damping: f32,
    gravity: vec3<f32>,
    num_elements: u32,
}

@group(0) @binding(0) var<storage, read_write> nodes: array<SoftBodyNode>;
@group(0) @binding(1) var<storage, read> elements: array<Tetrahedron>;
@group(0) @binding(2) var<uniform> params: Parameters;
@group(0) @binding(3) var<storage, read_write> forces_x: array<atomic<i32>>;
@group(0) @binding(4) var<storage, read_write> forces_y: array<atomic<i32>>;
@group(0) @binding(5) var<storage, read_write> forces_z: array<atomic<i32>>;

const FIXED_POINT_MULTIPLIER: f32 = 100000.0;

fn atomic_add_force(index: u32, force: vec3<f32>) {
    let fx = i32(force.x * FIXED_POINT_MULTIPLIER);
    let fy = i32(force.y * FIXED_POINT_MULTIPLIER);
    let fz = i32(force.z * FIXED_POINT_MULTIPLIER);
    
    atomicAdd(&forces_x[index], fx);
    atomicAdd(&forces_y[index], fy);
    atomicAdd(&forces_z[index], fz);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let elem_idx = global_id.x;
    if (elem_idx >= params.num_elements) {
        return;
    }

    let elem = elements[elem_idx];
    
    let n0 = nodes[elem.i0];
    let n1 = nodes[elem.i1];
    let n2 = nodes[elem.i2];
    let n3 = nodes[elem.i3];

    // Deformed shape matrix (Ds)
    let ds_col0 = n1.position - n0.position;
    let ds_col1 = n2.position - n0.position;
    let ds_col2 = n3.position - n0.position;
    
    // Matrix multiplication Ds * Dm^-1
    // Dm^-1 is stored column-wise in elem
    let inv0 = elem.inv_rest_matrix_col0;
    let inv1 = elem.inv_rest_matrix_col1;
    let inv2 = elem.inv_rest_matrix_col2;
    
    let f_col0 = ds_col0 * inv0.x + ds_col1 * inv0.y + ds_col2 * inv0.z;
    let f_col1 = ds_col0 * inv1.x + ds_col1 * inv1.y + ds_col2 * inv1.z;
    let f_col2 = ds_col0 * inv2.x + ds_col1 * inv2.y + ds_col2 * inv2.z;

    // Cauchy-Green deformation tensor (C = F^T * F)
    let c00 = dot(f_col0, f_col0);
    let c11 = dot(f_col1, f_col1);
    let c22 = dot(f_col2, f_col2);
    let ic = c00 + c11 + c22; // Trace(C)
    
    // Determinant of F (J)
    let j = f_col0.x * (f_col1.y * f_col2.z - f_col2.y * f_col1.z)
          - f_col1.x * (f_col0.y * f_col2.z - f_col2.y * f_col0.z)
          + f_col2.x * (f_col0.y * f_col1.z - f_col1.y * f_col0.z);
          
    if (j < 0.05) {
        return;
    }
    
    // Neo-Hookean Energy derivative (Piola-Kirchhoff stress P)
    // P = mu * (F - F^-T) + lambda * log(J) * F^-T
    
    // Need cofactor matrix of F for F^-T
    let cof0 = vec3<f32>(
        f_col1.y * f_col2.z - f_col2.y * f_col1.z,
        f_col2.x * f_col1.z - f_col1.x * f_col2.z,
        f_col1.x * f_col2.y - f_col2.x * f_col1.y
    );
    let cof1 = vec3<f32>(
        f_col2.y * f_col0.z - f_col0.y * f_col2.z,
        f_col0.x * f_col2.z - f_col2.x * f_col0.z,
        f_col2.x * f_col0.y - f_col0.x * f_col2.y
    );
    let cof2 = vec3<f32>(
        f_col0.y * f_col1.z - f_col1.y * f_col0.z,
        f_col1.x * f_col0.z - f_col0.x * f_col1.z,
        f_col0.x * f_col1.y - f_col1.x * f_col0.y
    );
    
    let f_inv_t_col0 = cof0 / j;
    let f_inv_t_col1 = cof1 / j;
    let f_inv_t_col2 = cof2 / j;
    
    let log_j = log(j);
    let mu = params.mu;
    let lambda = params.lambda;
    
    let p_col0 = mu * (f_col0 - f_inv_t_col0) + lambda * log_j * f_inv_t_col0;
    let p_col1 = mu * (f_col1 - f_inv_t_col1) + lambda * log_j * f_inv_t_col1;
    let p_col2 = mu * (f_col2 - f_inv_t_col2) + lambda * log_j * f_inv_t_col2;
    
    // H = -P * Dm^-T * rest_volume
    // This part involves transpose of inv_rest_matrix, let's keep it simple:
    // f1, f2, f3 are forces on nodes 1, 2, 3. f0 = -(f1+f2+f3).
    let h_col0 = (p_col0 * inv0.x + p_col1 * inv1.x + p_col2 * inv2.x) * (-elem.rest_volume);
    let h_col1 = (p_col0 * inv0.y + p_col1 * inv1.y + p_col2 * inv2.y) * (-elem.rest_volume);
    let h_col2 = (p_col0 * inv0.z + p_col1 * inv1.z + p_col2 * inv2.z) * (-elem.rest_volume);
    
    let f1 = h_col0;
    let f2 = h_col1;
    let f3 = h_col2;
    let f0 = -(f1 + f2 + f3);
    
    // Accumulate forces using atomic additions
    atomic_add_force(elem.i0, f0);
    atomic_add_force(elem.i1, f1);
    atomic_add_force(elem.i2, f2);
    atomic_add_force(elem.i3, f3);
}

// Another compute shader pass would run over nodes to integrate positions using the accumulated forces!
