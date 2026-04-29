// SSAO blur — 5×5 box blur to reduce hemisphere-sampling noise.

@group(0) @binding(0) var t_ao: texture_2d<f32>;
@group(0) @binding(1) var s_ao: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    var sum = 0.0;
    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            sum += textureLoad(t_ao, iuv + vec2(x, y), 0).r;
        }
    }
    let ao = sum / 25.0;
    return vec4(ao, ao, ao, 1.0);
}
