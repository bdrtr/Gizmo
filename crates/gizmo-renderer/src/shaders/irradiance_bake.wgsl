// Baking an irradiance volume: each probe's six captured faces, box-filtered down to 32 texels a side,
// then projected into the probe's spherical harmonics as irradiance over π (see `gi.rs`).
//
// The faces follow the cube's addressing, as `reflection_probe.wgsl`'s do: each is rendered by a
// right-handed camera looking down the face's axis (`reflection::face_basis`), whose picture is the
// face mirrored left-right, so `fs_downsample` reads the frame at 1 − u and `face_dir` is that file's.

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

const FACE: u32 = 32u;
const TAPS: u32 = 8u;

@group(0) @binding(0) var t_frame: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;

// The frame averaged over this texel's share of it: 8×8 bilinear taps, each of 2×2 pixels. A texel of
// a 32² face covers about 40×22 pixels of a 1280×720 frame, which one tap would alias — a small bright
// window caught or missed by where the tap fell.
@fragment
fn fs_downsample(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = floor(in.pos.xy);
    var sum = vec3<f32>(0.0);
    for (var j = 0u; j < TAPS; j = j + 1u) {
        for (var i = 0u; i < TAPS; i = i + 1u) {
            let uv = (texel + (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(TAPS)) / f32(FACE);
            sum += textureSampleLevel(t_frame, s_linear, vec2<f32>(1.0 - uv.x, uv.y), 0.0).rgb;
        }
    }
    return vec4<f32>(sum / f32(TAPS * TAPS), 1.0);
}

fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let sc = uv.x * 2.0 - 1.0;
    let tc = uv.y * 2.0 - 1.0;
    switch face {
        case 0u: { return normalize(vec3<f32>(1.0, -tc, -sc)); }
        case 1u: { return normalize(vec3<f32>(-1.0, -tc, sc)); }
        case 2u: { return normalize(vec3<f32>(sc, 1.0, tc)); }
        case 3u: { return normalize(vec3<f32>(sc, -1.0, -tc)); }
        case 4u: { return normalize(vec3<f32>(sc, -tc, 1.0)); }
        default: { return normalize(vec3<f32>(-sc, -tc, -1.0)); }
    }
}

// The projection's own bindings, numbered past the downsample's so no two variables share one.
@group(0) @binding(2) var t_faces: texture_2d_array<f32>;
// Every probe's 28 floats, laid out as `SHCoeffs::to_gpu_data`.
@group(0) @binding(3) var<storage, read_write> probes: array<f32>;
// x = which probe, yzw = the volume's ambient, a radiance.
@group(0) @binding(4) var<uniform> job: vec4<f32>;

const THREADS: u32 = 64u;
var<workgroup> partial: array<array<vec3<f32>, 9>, 64>;

// One probe: its radiance over the sphere, each texel weighted by the solid angle it covers on the
// cube, projected onto the nine basis functions — the order and constants of `SHCoeffs` — then turned
// into irradiance over π by the clamped cosine's bands (`gi::COSINE_BANDS`: 1, 2/3, 1/4).
@compute @workgroup_size(64)
fn cs_project(@builtin(local_invocation_index) t: u32) {
    var c: array<vec3<f32>, 9>;
    for (var i = t; i < 6u * FACE * FACE; i = i + THREADS) {
        let face = i / (FACE * FACE);
        let k = i % (FACE * FACE);
        let xy = vec2<u32>(k % FACE, k / FACE);
        let uv = (vec2<f32>(xy) + 0.5) / f32(FACE);
        let st = uv * 2.0 - 1.0;
        let d = face_dir(face, uv);
        // A texel's solid angle on the unit cube's face: (2/n)² / (1 + s² + t²)^{3/2}.
        let w = 4.0 / f32(FACE * FACE) / pow(1.0 + dot(st, st), 1.5);
        let radiance = textureLoad(t_faces, xy, face, 0).rgb * w;
        c[0] += radiance * 0.282095;
        c[1] += radiance * (0.488603 * d.y);
        c[2] += radiance * (0.488603 * d.z);
        c[3] += radiance * (0.488603 * d.x);
        c[4] += radiance * (1.092548 * d.x * d.y);
        c[5] += radiance * (1.092548 * d.y * d.z);
        c[6] += radiance * (0.315392 * (3.0 * d.z * d.z - 1.0));
        c[7] += radiance * (1.092548 * d.x * d.z);
        c[8] += radiance * (0.546274 * (d.x * d.x - d.y * d.y));
    }
    partial[t] = c;
    workgroupBarrier();
    if (t != 0u) {
        return;
    }
    var sum: array<vec3<f32>, 9>;
    for (var j = 0u; j < THREADS; j = j + 1u) {
        for (var k = 0u; k < 9u; k = k + 1u) {
            sum[k] += partial[j][k];
        }
    }
    // The ambient is a uniform radiance: its projection is √(4π) of it, and band 0's factor is 1.
    sum[0] += job.yzw * 3.5449077;
    let base = u32(job.x) * 28u;
    for (var k = 0u; k < 9u; k = k + 1u) {
        var band = 1.0;
        if (k >= 4u) {
            band = 0.25;
        } else if (k >= 1u) {
            band = 2.0 / 3.0;
        }
        probes[base + k * 3u] = sum[k].x * band;
        probes[base + k * 3u + 1u] = sum[k].y * band;
        probes[base + k * 3u + 2u] = sum[k].z * band;
    }
    probes[base + 27u] = 0.0;
}
