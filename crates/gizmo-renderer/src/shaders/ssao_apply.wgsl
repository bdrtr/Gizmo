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
    let ao_raw = textureSample(t_ao, s_ao, in.uv).r;
    let ao = mix(1.0, ao_raw, params.strength);
    return vec4(ao, ao, ao, 1.0);
}
