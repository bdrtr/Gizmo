// SSR geometry, at the reflections' own resolution: for each half-resolution pixel, the distance from
// the camera of the surface `ssr.wgsl` reflects from there (the full-resolution texel at twice its
// coordinate, as the march takes it) and its normal, folded into two channels (`encode_octahedral`).
// Zero distance where the G-buffer holds nothing.
//
// The march reads the distance at every step, and the glossy resolve (`ssr_blur.wgsl`) the distance
// and the normal at every tap: from here, eight bytes a texel and neighbours in the same cache line,
// rather than the full-resolution G-buffer's position and normal, sixteen bytes scattered over four
// times the area. Measured in `nfsu2-gizmo`'s wet city on a Radeon 780M (2026-11-19): the march and
// the resolve, a reflection's two largest costs, each a third of it.

#import gizmo::common::{encode_octahedral}

@group(0) @binding(0) var t_normal_roughness: texture_2d<f32>;
@group(0) @binding(1) var t_position_rel_camera: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let full = vec2<i32>(frag_coord.xy) * 2;
    let position = textureLoad(t_position_rel_camera, full, 0);
    // `>= 0.5`: a written pixel of an ordinary material carries exactly 0.5 (see `gbuffer.wgsl`).
    if (position.w < 0.5) {
        return vec4<f32>(0.0);
    }
    let normal = normalize(textureLoad(t_normal_roughness, full, 0).xyz);
    return vec4<f32>(length(position.xyz), encode_octahedral(normal), 0.0);
}
