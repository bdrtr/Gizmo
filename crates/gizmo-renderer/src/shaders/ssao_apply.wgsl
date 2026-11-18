// SSAO apply — outputs (ao, ao, ao, 1.0) for multiply-blend into the HDR target.
// Blend equation: hdr_final = hdr_existing * ao_factor
//
// The AO is at half resolution. Each full-resolution pixel takes it from the four texels around it,
// bilinearly — but only from those on its own surface (on its plane, to within a fraction of its
// camera distance, as in `ssao_blur.wgsl`): a plain bilinear fetch took a near object's occlusion
// onto the far pixels at its edge.

struct AoParams {
    strength: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var t_ao:  texture_2d<f32>;
@group(0) @binding(1) var s_ao:  sampler;
@group(0) @binding(2) var<uniform> params: AoParams;
// The G-buffer's full-resolution position (camera-relative; w = 0 where nothing was drawn).
@group(0) @binding(3) var t_position_rel_camera: texture_2d<f32>;
// Its normal, at the same pixels.
@group(0) @binding(4) var t_normal: texture_2d<f32>;

/// As in `ssao_blur.wgsl`.
const SAME_SURFACE: f32 = 0.05;
const NEARBY: f32 = 0.25;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var uv = array<vec2<f32>, 3>(vec2(0.0, 1.0), vec2(2.0, 1.0), vec2(0.0, -1.0));
    var out: VertexOutput;
    out.position = vec4(pos[vi], 0.0, 1.0);
    out.uv = uv[vi];
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let full_last = vec2<i32>(textureDimensions(t_position_rel_camera)) - vec2<i32>(1);
    let pixel = vec2<i32>(in.position.xy);
    let here = textureLoad(t_position_rel_camera, clamp(pixel, vec2<i32>(0), full_last), 0);
    // Nothing drawn here, or a baked-lit surface (`.w` above 150: `common.wgsl`'s `BAKED_SURFACE`,
    // written out because this shader is not composed): its baked light has its occlusion in it, and
    // darkening it again would draw the corners twice.
    if (here.w < 0.5 || here.w > 150.0) {
        return vec4(1.0);
    }
    let normal = normalize(textureLoad(t_normal, clamp(pixel, vec2<i32>(0), full_last), 0).xyz);
    let tolerance = length(here.xyz) * SAME_SURFACE;
    let reach = length(here.xyz) * NEARBY;

    // Texel `h` holds the AO computed at full-resolution pixel `2h`: this pixel's place among them.
    let at = in.position.xy * 0.5 - vec2(0.25);
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    let half_last = vec2<i32>(textureDimensions(t_ao)) - vec2<i32>(1);
    var sum = 0.0;
    var weight = 0.0;
    var nearest_ao = 1.0;
    var nearest_gap = 1e30;
    for (var j = 0; j < 4; j++) {
        let o = vec2<i32>(j & 1, j >> 1);
        let h = clamp(base + o, vec2<i32>(0), half_last);
        let p = textureLoad(t_position_rel_camera, clamp(h * 2, vec2<i32>(0), full_last), 0);
        let ao = textureLoad(t_ao, h, 0).r;
        let apart = length(p.xyz - here.xyz);
        let gap = select(1e30, apart, p.w >= 0.5);
        if (gap < nearest_gap) {
            nearest_gap = gap;
            nearest_ao = ao;
        }
        if (p.w >= 0.5 && abs(dot(p.xyz - here.xyz, normal)) <= tolerance && apart <= reach) {
            let w = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            sum += ao * w;
            weight += w;
        }
    }
    // None of the four on this surface (a sliver thinner than a texel): the closest in space.
    let ao_raw = select(nearest_ao, sum / weight, weight > 1e-4);
    let ao = mix(1.0, ao_raw, params.strength);
    return vec4(ao, ao, ao, 1.0);
}
