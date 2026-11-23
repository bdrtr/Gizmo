// soft_body.wgsl — the elastic forces of tetrahedral FEM soft bodies, one invocation per element.
//
// The element pass of `SoftBodyMesh::step_coupled` (soft_body.rs), with the same expressions:
// the deformation gradient F = Ds · Dm⁻¹, the Neo-Hookean first Piola–Kirchhoff stress
// P = μ·(F − F⁻ᵀ) + λ·ln(J)·F⁻ᵀ, and the four nodal forces −P · Dm⁻ᵀ · V₀. Each element carries
// its own body's μ and λ. Nothing else happens here: `GpuCompute::step_soft_bodies` reads the
// forces back and the CPU's own integrator (`SoftBodyMesh::advance_nodes`) sums, integrates,
// damps and collides them, so a GPU step and a CPU step differ only by this pass's rounding.

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
    // Its body's Lamé parameters, in pascals.
    mu: f32,
    lambda: f32,
    pad3: f32,
    // The tail is three SCALARS on purpose. A `vec3<f32>` here aligns to 16, so after
    // `rest_volume` ends at 68 the shader would place it at 80 and round the struct to 96 bytes
    // against the 80 the CPU uploads — every element past index 0 read from the wrong offset,
    // silently. `tests/wgsl_layout.rs` checks the layout with naga.
}

struct Parameters {
    num_elements: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

// Node positions in world space; `w` is unused.
@group(0) @binding(0) var<storage, read> positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> elements: array<Tetrahedron>;
@group(0) @binding(2) var<uniform> params: Parameters;
// Four per element: the force on i0, i1, i2 and i3, in newtons, with `w` = 1 when the element
// pushes and 0 when it is crushed past `J_EPS` and pushes on nothing.
@group(0) @binding(3) var<storage, read_write> forces: array<vec4<f32>>;

// The CPU's `soft_body::J_EPS`: the smallest det(F) at which an element still pushes.
const J_EPS: f32 = 1e-4;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let e = global_id.x;
    if (e >= params.num_elements) {
        return;
    }
    let elem = elements[e];
    let out = e * 4u;
    let p0 = positions[elem.i0].xyz;
    let ds0 = positions[elem.i1].xyz - p0;
    let ds1 = positions[elem.i2].xyz - p0;
    let ds2 = positions[elem.i3].xyz - p0;

    // F = Ds · Dm⁻¹, column by column; Dm⁻¹ is stored by columns.
    let inv0 = elem.inv_rest_matrix_col0;
    let inv1 = elem.inv_rest_matrix_col1;
    let inv2 = elem.inv_rest_matrix_col2;
    let f0 = ds0 * inv0.x + ds1 * inv0.y + ds2 * inv0.z;
    let f1 = ds0 * inv1.x + ds1 * inv1.y + ds2 * inv1.z;
    let f2 = ds0 * inv2.x + ds1 * inv2.y + ds2 * inv2.z;

    // J = det(F), and F⁻ᵀ from F's cofactors: its columns are f1×f2, f2×f0, f0×f1 over J.
    let cof0 = cross(f1, f2);
    let cof1 = cross(f2, f0);
    let cof2 = cross(f0, f1);
    let j = dot(f0, cof0);
    if (j <= J_EPS) {
        forces[out] = vec4<f32>(0.0);
        forces[out + 1u] = vec4<f32>(0.0);
        forces[out + 2u] = vec4<f32>(0.0);
        forces[out + 3u] = vec4<f32>(0.0);
        return;
    }
    let fit0 = cof0 / j;
    let fit1 = cof1 / j;
    let fit2 = cof2 / j;
    let volume_term = elem.lambda * log(j);
    let p0c = elem.mu * (f0 - fit0) + volume_term * fit0;
    let p1c = elem.mu * (f1 - fit1) + volume_term * fit1;
    let p2c = elem.mu * (f2 - fit2) + volume_term * fit2;

    // The forces on nodes 1..3 are the columns of −P · Dm⁻ᵀ · V₀; node 0 takes what balances them.
    let n1 = -(p0c * inv0.x + p1c * inv1.x + p2c * inv2.x) * elem.rest_volume;
    let n2 = -(p0c * inv0.y + p1c * inv1.y + p2c * inv2.y) * elem.rest_volume;
    let n3 = -(p0c * inv0.z + p1c * inv1.z + p2c * inv2.z) * elem.rest_volume;
    forces[out] = vec4<f32>(-(n1 + n2 + n3), 1.0);
    forces[out + 1u] = vec4<f32>(n1, 1.0);
    forces[out + 2u] = vec4<f32>(n2, 1.0);
    forces[out + 3u] = vec4<f32>(n3, 1.0);
}
