// SSAO blur — a 5×5 filter over the half-resolution AO that keeps to one surface.
//
// The AO sample kernel is noisy by construction, and this is what removes the noise. A plain box
// filter did, and it also averaged across silhouettes: a crease's darkness on a near object spread
// onto whatever lay behind its edge on screen — a wall metres away darkened in a halo around it
// (`ssao_does_not_bleed_across_a_silhouette`). A neighbour counts only if it lies on this pixel's
// plane, to within a fraction of the camera distance: the same surface, or one meeting it. The
// plane and not the camera distance, because along a surface seen edge-on the camera distance
// changes fast from one pixel to the next and every neighbour looked like another surface — the
// noise stayed in. And near it as well as on its plane: a plane runs on past the surface, and the
// side of a pole is in line with the wall three metres behind it.

@group(0) @binding(0) var t_ao: texture_2d<f32>;
@group(0) @binding(1) var s_ao: sampler;
// The G-buffer's full-resolution position (camera-relative; w = 0 where nothing was drawn). The AO
// texel at `h` was computed for the full-resolution pixel `2h`.
@group(0) @binding(2) var t_position_rel_camera: texture_2d<f32>;
// Its normal, at the same pixels.
@group(0) @binding(3) var t_normal: texture_2d<f32>;

/// How far off this pixel's plane, as a fraction of its camera distance, a neighbour may lie and
/// still be on its surface.
const SAME_SURFACE: f32 = 0.05;
/// And how far from this pixel's point, as the same fraction.
const NEARBY: f32 = 0.25;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

/// The camera-relative position at AO texel `h`; `w` is 0 where nothing was drawn.
fn position_at(h: vec2<i32>) -> vec4<f32> {
    let full_last = vec2<i32>(textureDimensions(t_position_rel_camera)) - vec2<i32>(1);
    return textureLoad(t_position_rel_camera, clamp(h * 2, vec2<i32>(0), full_last), 0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x), i32(frag_coord.y));
    let centre = position_at(iuv);
    if (centre.w < 0.5) {
        return vec4(1.0);
    }
    let full_last = vec2<i32>(textureDimensions(t_normal)) - vec2<i32>(1);
    let normal = normalize(textureLoad(t_normal, clamp(iuv * 2, vec2<i32>(0), full_last), 0).xyz);
    let tolerance = length(centre.xyz) * SAME_SURFACE;
    let reach = length(centre.xyz) * NEARBY;
    // A load past the edge reads zero, and zero is full occlusion: unclamped, every frame had a dark
    // band along its top and left, 15 levels deep.
    let last = vec2<i32>(textureDimensions(t_ao)) - vec2<i32>(1);
    var sum = 0.0;
    var weight = 0.0;
    for (var x = -2; x <= 2; x++) {
        for (var y = -2; y <= 2; y++) {
            let h = clamp(iuv + vec2(x, y), vec2<i32>(0), last);
            let p = position_at(h);
            if (p.w < 0.5 || abs(dot(p.xyz - centre.xyz, normal)) > tolerance || length(p.xyz - centre.xyz) > reach) {
                continue;
            }
            sum += textureLoad(t_ao, h, 0).r;
            weight += 1.0;
        }
    }
    // The centre always counts, so `weight` is at least 1.
    let ao = sum / weight;
    return vec4(ao, ao, ao, 1.0);
}
