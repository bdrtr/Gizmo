@group(0) @binding(0) var t_ssgi_blurred: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;

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
    // Use the interpolated [0,1] UV (like ssr_apply / ssao_apply), NOT
    // frag_coord / textureDimensions(source): the apply pass renders at full HDR
    // resolution while t_ssgi_blurred is half-res, so dividing full-res frag_coord by
    // the half-res source size pushed UV up to ~2.0 and clamped everything outside the
    // top-left quadrant to the edge texel. The linear sampler upscales the half-res GI.
    let ssgi_color = textureSample(t_ssgi_blurred, s_linear, in.uv).rgb;

    // Additive blending will add this to the HDR texture
    return vec4(ssgi_color, 1.0);
}
