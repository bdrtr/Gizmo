#define_import_path gizmo::irradiance

// Irradiance volumes, sampled on the GPU.
//
// A direct port of `gi::ProbeGrid::sample` + `gi::SHCoeffs::evaluate`: the same trilinear blend
// over the same eight corners, the same basis constants, the same clamp to non-negative. That is
// deliberate and load-bearing — `irradiance_volumes` measures the CPU path's answers, and this has
// to reproduce them or the two paths are two features.

// No padding fields, because there is nowhere to put one: naga_oil rejects identifiers that would
// need substitution under naga's writeback rules, and both `_pad0` and `pad0` are such — the
// composer says "Composable module identifiers must not require substitution".
//
// So the fourth component of each `vec4` carries a real value instead of a pad. Three vec4s, 48
// bytes, and the alignment falls out rather than being arranged.
struct GridMeta {
    // xyz = grid minimum corner, w = probe count (as a float; the count fits exactly well past
    // any grid that would fit in memory).
    grid_min_count: vec4<f32>,
    // xyz = cell size, w unused.
    cell_size: vec4<f32>,
    // xyz = resolution, w unused.
    resolution: vec4<u32>,
};

// 28 floats per probe: 27 coefficients (L0, 3×L1, 5×L2, three channels each) + one pad. Flat
// rather than a struct of vec3s because WGSL would pad every vec3 to 16 bytes and the Rust side
// packs them at 12.
@group(3) @binding(0) var<storage, read> gi_probes: array<f32>;
@group(3) @binding(1) var<uniform> gi_meta: GridMeta;

fn gi_l0(i: u32) -> vec3<f32> {
    let b = i * 28u;
    return vec3<f32>(gi_probes[b], gi_probes[b + 1u], gi_probes[b + 2u]);
}
fn gi_l1(i: u32, k: u32) -> vec3<f32> {
    let b = i * 28u + 3u + k * 3u;
    return vec3<f32>(gi_probes[b], gi_probes[b + 1u], gi_probes[b + 2u]);
}
fn gi_l2(i: u32, k: u32) -> vec3<f32> {
    let b = i * 28u + 12u + k * 3u;
    return vec3<f32>(gi_probes[b], gi_probes[b + 1u], gi_probes[b + 2u]);
}

// One probe's nine coefficient vectors, evaluated in `direction`.
fn gi_evaluate(i: u32, direction: vec3<f32>) -> vec3<f32> {
    let d = normalize(direction);

    let y0 = 0.282095;
    let y1_neg1 = 0.488603 * d.y;
    let y1_0 = 0.488603 * d.z;
    let y1_pos1 = 0.488603 * d.x;
    let y2_neg2 = 1.092548 * d.x * d.y;
    let y2_neg1 = 1.092548 * d.y * d.z;
    let y2_0 = 0.315392 * (3.0 * d.z * d.z - 1.0);
    let y2_pos1 = 1.092548 * d.x * d.z;
    let y2_pos2 = 0.546274 * (d.x * d.x - d.y * d.y);

    var r = gi_l0(i) * y0;
    r += gi_l1(i, 0u) * y1_neg1;
    r += gi_l1(i, 1u) * y1_0;
    r += gi_l1(i, 2u) * y1_pos1;
    r += gi_l2(i, 0u) * y2_neg2;
    r += gi_l2(i, 1u) * y2_neg1;
    r += gi_l2(i, 2u) * y2_0;
    r += gi_l2(i, 3u) * y2_pos1;
    r += gi_l2(i, 4u) * y2_pos2;

    // No negative light — the same clamp the CPU path ends with.
    return max(r, vec3<f32>(0.0));
}

fn gi_index(x: u32, y: u32, z: u32) -> u32 {
    let i = z * gi_meta.resolution.y * gi_meta.resolution.x + y * gi_meta.resolution.x + x;
    return min(i, u32(gi_meta.grid_min_count.w) - 1u);
}

// Trilinear blend of the eight surrounding probes, evaluated in `normal`.
//
// Evaluating each corner and then blending the *results* is not what the CPU path does — it blends
// the coefficients and evaluates once. For a linear operator the two agree, and SH evaluation is
// linear in the coefficients, so this is the same number with less arithmetic in the shader.
fn gi_sample(world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    if (gi_meta.grid_min_count.w < 0.5) {
        return vec3<f32>(0.0);
    }

    let local = world_pos - gi_meta.grid_min_count.xyz;
    let f = max(local / gi_meta.cell_size.xyz - vec3<f32>(0.5), vec3<f32>(0.0));

    // `resolution - 2` is the last cell's low corner. `max(res, 2) - 2` rather than a saturating
    // subtract: a one-probe axis would wrap to a huge u32 and index out of the buffer.
    let hi = max(gi_meta.resolution.xyz, vec3<u32>(2u)) - vec3<u32>(2u);
    let i0 = min(vec3<u32>(u32(f.x), u32(f.y), u32(f.z)), hi);
    let t = clamp(f - vec3<f32>(f32(i0.x), f32(i0.y), f32(i0.z)), vec3<f32>(0.0), vec3<f32>(1.0));

    let c000 = gi_evaluate(gi_index(i0.x, i0.y, i0.z), normal);
    let c100 = gi_evaluate(gi_index(i0.x + 1u, i0.y, i0.z), normal);
    let c010 = gi_evaluate(gi_index(i0.x, i0.y + 1u, i0.z), normal);
    let c110 = gi_evaluate(gi_index(i0.x + 1u, i0.y + 1u, i0.z), normal);
    let c001 = gi_evaluate(gi_index(i0.x, i0.y, i0.z + 1u), normal);
    let c101 = gi_evaluate(gi_index(i0.x + 1u, i0.y, i0.z + 1u), normal);
    let c011 = gi_evaluate(gi_index(i0.x, i0.y + 1u, i0.z + 1u), normal);
    let c111 = gi_evaluate(gi_index(i0.x + 1u, i0.y + 1u, i0.z + 1u), normal);

    let c00 = mix(c000, c100, t.x);
    let c01 = mix(c001, c101, t.x);
    let c10 = mix(c010, c110, t.x);
    let c11 = mix(c011, c111, t.x);
    let c0 = mix(c00, c10, t.y);
    let c1 = mix(c01, c11, t.y);
    return mix(c0, c1, t.z);
}
