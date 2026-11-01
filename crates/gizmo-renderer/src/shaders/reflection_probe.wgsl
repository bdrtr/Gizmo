// Reflection probe: copying a captured face into the cube, and prefiltering its mips.
//
// The cube follows WebGPU's addressing (the D3D/Vulkan one): for face f and texel uv (origin
// top-left), with sc = 2u − 1 and tc = 2v − 1, the direction is
//   +X (1, −tc, −sc)   −X (−1, −tc, sc)   +Y (sc, 1, tc)
//   −Y (sc, −1, −tc)   +Z (sc, −tc, 1)    −Z (−sc, −tc, −1)
// Each face is rendered by an ordinary right-handed camera looking down the face's axis, whose
// picture is the same view mirrored left-right (the cube convention is left-handed) — so `fs_copy`
// samples the frame at 1 − u. `face_dir` below is the formula above, and the prefilter and the
// lighting shader both read the cube through the hardware's own addressing, so all three agree.

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

// x = face (0..5), y = roughness of the mip being written, z = sample count, w = source size.
@group(0) @binding(0) var<uniform> job: vec4<f32>;
@group(0) @binding(1) var t_frame: texture_2d<f32>;
@group(0) @binding(2) var t_cube: texture_cube<f32>;
@group(0) @binding(3) var s_linear: sampler;

@fragment
fn fs_copy(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(t_frame, s_linear, vec2<f32>(1.0 - in.uv.x, in.uv.y), 0.0).rgb, 1.0);
}

fn face_dir(face: i32, uv: vec2<f32>) -> vec3<f32> {
    let sc = uv.x * 2.0 - 1.0;
    let tc = uv.y * 2.0 - 1.0;
    switch face {
        case 0: { return normalize(vec3<f32>(1.0, -tc, -sc)); }
        case 1: { return normalize(vec3<f32>(-1.0, -tc, sc)); }
        case 2: { return normalize(vec3<f32>(sc, 1.0, tc)); }
        case 3: { return normalize(vec3<f32>(sc, -1.0, -tc)); }
        case 4: { return normalize(vec3<f32>(sc, -tc, 1.0)); }
        default: { return normalize(vec3<f32>(-sc, -tc, -1.0)); }
    }
}

fn radical_inverse(bits_in: u32) -> f32 {
    var bits = bits_in;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xAAAAAAAAu) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xCCCCCCCCu) >> 2u);
    bits = ((bits & 0x0F0F0F0Fu) << 4u) | ((bits & 0xF0F0F0F0u) >> 4u);
    bits = ((bits & 0x00FF00FFu) << 8u) | ((bits & 0xFF00FF00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}

// GGX prefilter (split-sum, N = V = R): the radiance a surface of `job.y` roughness reflects along
// each direction, importance-sampled around it. Samples read a mip chosen from their pdf ("filtered
// importance sampling"), so a rough mip averages a footprint rather than aliasing 64 texels — the
// source view holds every mip below the one being written.
@fragment
fn fs_prefilter(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = face_dir(i32(job.x), in.uv);
    let roughness = max(job.y, 0.02);
    let a = roughness * roughness;
    let count = u32(job.z);
    // Tangent frame around n.
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(n.y) > 0.999) { up = vec3<f32>(1.0, 0.0, 0.0); }
    let tx = normalize(cross(up, n));
    let ty = cross(n, tx);
    let texel_solid_angle = 4.0 * 3.14159265 / (6.0 * job.w * job.w);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let xi = vec2<f32>(f32(i) / f32(count), radical_inverse(i));
        let phi = 2.0 * 3.14159265 * xi.x;
        let cos_t = sqrt((1.0 - xi.y) / (1.0 + (a * a - 1.0) * xi.y));
        let sin_t = sqrt(1.0 - cos_t * cos_t);
        let h = normalize(tx * (cos(phi) * sin_t) + ty * (sin(phi) * sin_t) + n * cos_t);
        let l = normalize(2.0 * dot(n, h) * h - n);
        let ndl = dot(n, l);
        if (ndl > 0.0) {
            let ndh = max(dot(n, h), 0.0);
            let d_denom = ndh * ndh * (a * a - 1.0) + 1.0;
            let d = a * a / (3.14159265 * d_denom * d_denom);
            let pdf = d / 4.0 + 1e-4; // N = V: pdf(l) = D·(n·h) / (4·(v·h)) = D / 4
            let sample_solid_angle = 1.0 / (f32(count) * pdf);
            let lod = max(0.5 * log2(sample_solid_angle / texel_solid_angle) + 1.0, 0.0);
            sum += textureSampleLevel(t_cube, s_linear, l, lod).rgb * ndl;
            weight += ndl;
        }
    }
    return vec4<f32>(sum / max(weight, 1e-4), 1.0);
}
