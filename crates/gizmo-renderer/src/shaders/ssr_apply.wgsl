// SSR apply pass
@group(0) @binding(0) var t_ssr:  texture_2d<f32>;
@group(0) @binding(1) var s_nearest:  sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    let ssr_color = textureLoad(t_ssr, iuv, 0).rgb;
    return vec4(ssr_color, 1.0);
}
