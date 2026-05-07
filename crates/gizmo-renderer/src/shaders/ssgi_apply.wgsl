@group(0) @binding(0) var t_ssgi_blurred: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(frag_coord.x / f32(textureDimensions(t_ssgi_blurred).x), frag_coord.y / f32(textureDimensions(t_ssgi_blurred).y));
    let ssgi_color = textureSample(t_ssgi_blurred, s_linear, uv).rgb;
    
    // Additive blending will add this to the HDR texture
    return vec4(ssgi_color, 1.0);
}
