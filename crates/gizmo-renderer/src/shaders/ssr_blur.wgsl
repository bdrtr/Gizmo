// SSR glossy resolve: spreads each pixel's reflection over its surface's lobe.
//
// `ssr.wgsl` marches the mirror direction and leaves in alpha how wide, in full-resolution pixels, the
// surface's lobe spreads what it found (`blur_radius`: the roughness, and how far the ray went). This
// gathers over that width in two passes — along the axis the surface's normal takes on screen by the
// width, and across it by the width times N·V: a half vector tipped by δ turns the reflection by 2δ
// in the plane of incidence and by 2δ·N·V across it — so a glossy floor seen at a grazing angle draws
// a lamp as a long streak down the road and a thin one across it, as wet asphalt does, and a mirror
// (width under half a pixel) is left as it is. Only pixels of the same surface are gathered: on its plane, and
// facing its way.
//
// Half resolution, as the march is: the G-buffer is read at twice the coordinate.

#import gizmo::common::{SceneUniforms}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

// The march's result: rgb = what the pixel's reflection adds, a = the lobe's width in full-resolution
// pixels, 0 where the pixel reflects nothing.
@group(1) @binding(0) var t_src: texture_2d<f32>;
@group(1) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(1) @binding(2) var t_position_rel_camera: texture_2d<f32>;

// Samples each side of the pixel.
const TAPS: i32 = 7;
// The widest the gather goes, in full-resolution pixels.
const MAX_REACH_PX: f32 = 96.0;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_along(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    return resolve(frag_coord.xy, true);
}

@fragment
fn fs_across(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    return resolve(frag_coord.xy, false);
}

fn resolve(frag: vec2<f32>, along: bool) -> vec4<f32> {
    let dim = vec2<i32>(textureDimensions(t_src));
    let here = vec2<i32>(frag);
    let centre = textureLoad(t_src, here, 0);
    let width = centre.a;
    if (width < 0.5) {
        return centre;
    }
    let normal = normalize(textureLoad(t_normal_roughness, here * 2, 0).xyz);
    let position = textureLoad(t_position_rel_camera, here * 2, 0).xyz;
    let dist = length(position);
    let n_dot_v = max(dot(-position / max(dist, 1e-4), normal), 0.0);

    // The axis the normal takes on screen, in pixels; straight up the screen where it points at the
    // camera and has none.
    let world = position + scene.camera_pos.xyz;
    let a = scene.view_proj * vec4(world, 1.0);
    let b = scene.view_proj * vec4(world + normal * (0.01 * dist), 1.0);
    var axis = vec2<f32>(0.0, 1.0);
    if (a.w > 1e-4 && b.w > 1e-4) {
        let d = (b.xy / b.w - a.xy / a.w) * vec2<f32>(dim) * vec2<f32>(1.0, -1.0);
        if (dot(d, d) > 1e-8) {
            axis = normalize(d);
        }
    }
    var reach_px = min(width, MAX_REACH_PX);
    if (!along) {
        axis = vec2<f32>(-axis.y, axis.x);
        reach_px = reach_px * n_dot_v;
    }
    if (reach_px < 0.5) {
        return centre;
    }
    // The texture is half resolution.
    let stride = axis * (0.5 * reach_px / f32(TAPS));
    let tolerance = 0.02 * dist + 0.05;

    var sum = centre.rgb;
    var total = 1.0;
    for (var k = 1; k <= TAPS; k++) {
        let x = f32(k) / f32(TAPS);
        let w = exp(-3.0 * x * x);
        for (var side = -1; side <= 1; side += 2) {
            let at = clamp(vec2<i32>(round(frag + stride * f32(k * side))), vec2<i32>(0), dim - vec2<i32>(1));
            let found = textureLoad(t_src, at, 0);
            if (found.a <= 0.0) {
                continue;
            }
            let there = textureLoad(t_position_rel_camera, at * 2, 0);
            let facing = dot(normalize(textureLoad(t_normal_roughness, at * 2, 0).xyz), normal);
            if (there.w < 0.5 || abs(dot(there.xyz - position, normal)) > tolerance || facing < 0.9) {
                continue;
            }
            sum += found.rgb * w;
            total += w;
        }
    }
    return vec4(sum / total, width);
}
