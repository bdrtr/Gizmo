// SSAO apply — outputs (ao, ao, ao, 1.0) for multiply-blend into the HDR target.
// Blend equation: hdr_final = hdr_existing * ao_factor

struct AoParams {
    strength: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var t_ao:  texture_2d<f32>;
@group(0) @binding(1) var s_ao:  sampler;
@group(0) @binding(2) var<uniform> params: AoParams;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    let ao_raw = textureLoad(t_ao, iuv, 0).r;
    // strength=0 → no darkening; strength=1 → full AO
    let ao = mix(1.0, ao_raw, params.strength);
    return vec4(ao, ao, ao, 1.0);
}
