@group(0) @binding(0) var t_ssgi: texture_2d<f32>;
@group(0) @binding(1) var s_linear: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(frag_coord.x / f32(textureDimensions(t_ssgi).x), frag_coord.y / f32(textureDimensions(t_ssgi).y));
    
    var color = vec3<f32>(0.0);
    var count = 0.0;
    
    // Simple 5x5 Box Blur
    let tex_size = vec2<f32>(textureDimensions(t_ssgi));
    let texel_size = 1.0 / tex_size;

    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
            color += textureSample(t_ssgi, s_linear, uv + offset).rgb;
            count += 1.0;
        }
    }

    return vec4(color / count, 1.0);
}
