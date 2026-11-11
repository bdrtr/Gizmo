// SSR apply pass
@group(0) @binding(0) var t_ssr:  texture_2d<f32>;
@group(0) @binding(1) var s_nearest:  sampler;
// How much of the G-buffer's surface the frame shows (`DeferredState::seen_view`): the reflection
// belongs to that surface, so it is not added over what was drawn forward in front of it.
@group(0) @binding(2) var t_seen: texture_2d<f32>;

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
    let ssr_color = textureSample(t_ssr, s_nearest, in.uv).rgb;
    let seen = textureLoad(t_seen, vec2<i32>(in.position.xy), 0).r;
    return vec4(ssr_color * seen, 1.0);
}
