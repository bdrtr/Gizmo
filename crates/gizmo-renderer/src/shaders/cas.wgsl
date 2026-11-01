// CAS — contrast-adaptive sharpening (after AMD FidelityFX CAS, non-scaling form).
//
// The last pass of the chain: it gives back the detail TAA's history blend and FXAA's edge blur
// take away. Each pixel is pushed away from its four neighbours by a weight that ADAPTS to local
// contrast: from the 3×3 neighbourhood's soft minimum and maximum (cross + box, so 0..2) it takes
// how far the pixel can move before clipping, `min(mn, 2 − mx) / mx`. A flat area has no
// neighbour difference to amplify and is left exactly as it is; an edge already near black or
// white gets a small weight, so sharpening does not ring there.
//
// Taps are sampled one texel apart around the pixel's own uv, so on a full-screen pass they land
// exactly on texel centres (the bilinear sampler returns the texel itself), and drawn into a
// split-screen rectangle they still read the right neighbourhood of the full-size input.

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

struct CasParams {
    // 0 = the mildest sharpening CAS does, 1 = the strongest.
    sharpness: f32,
    _p0: f32,
    _p1: f32,
    _p2: f32,
};

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> params: CasParams;

fn tap(uv: vec2<f32>, offset: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(input_tex, input_sampler, uv + offset, 0.0).rgb;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(input_tex));
    let uv = in.uv;
    // a b c
    // d e f
    // g h i
    let a = tap(uv, vec2<f32>(-t.x, -t.y));
    let b = tap(uv, vec2<f32>(0.0, -t.y));
    let c = tap(uv, vec2<f32>(t.x, -t.y));
    let d = tap(uv, vec2<f32>(-t.x, 0.0));
    let centre = textureSampleLevel(input_tex, input_sampler, uv, 0.0);
    let e = centre.rgb;
    let f = tap(uv, vec2<f32>(t.x, 0.0));
    let g = tap(uv, vec2<f32>(-t.x, t.y));
    let h = tap(uv, vec2<f32>(0.0, t.y));
    let i = tap(uv, vec2<f32>(t.x, t.y));

    // Soft min and max: the cross plus the whole box, so each runs 0..2.
    let mn_cross = min(min(min(d, e), min(f, b)), h);
    let mn = mn_cross + min(mn_cross, min(min(a, c), min(g, i)));
    let mx_cross = max(max(max(d, e), max(f, b)), h);
    let mx = mx_cross + max(mx_cross, max(max(a, c), max(g, i)));

    // How far this pixel may move before clipping, relative to the local maximum.
    let amp = sqrt(clamp(min(mn, vec3<f32>(2.0) - mx) / max(mx, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0)));
    // A negative lobe: −1/8 at the mildest, −1/5 at the strongest.
    let peak = -1.0 / mix(8.0, 5.0, clamp(params.sharpness, 0.0, 1.0));
    let w = amp * peak;
    let sharpened = (b * w + d * w + f * w + h * w + e) / (vec3<f32>(1.0) + 4.0 * w);
    return vec4<f32>(clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0)), centre.a);
}
