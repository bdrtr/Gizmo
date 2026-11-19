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
// Half resolution, as the march is, and reading only what is at that resolution: the march's result
// and its geometry (`ssr_prepare.wgsl`), not the full-resolution G-buffer, which every tap read twice
// until 2026-11-19.

#import gizmo::common::{SceneUniforms, decode_octahedral}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

// The march's result: rgb = what the pixel's reflection adds, a = the lobe's width in full-resolution
// pixels, 0 where the pixel reflects nothing.
@group(1) @binding(0) var t_src: texture_2d<f32>;
// The geometry at this resolution (`ssr_prepare.wgsl`): the distance from the camera in `x`, 0 for
// nothing; the normal folded into `yz` (`encode_octahedral`).
@group(1) @binding(1) var t_geometry: texture_2d<f32>;

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

// Where a pixel's surface is, relative to the camera: `distance` along the ray through the
// full-resolution texel the march took for it, at twice its coordinate.
fn surface_at(pixel: vec2<i32>, distance: f32, full_dim: vec2<f32>) -> vec3<f32> {
    let uv = (vec2<f32>(pixel * 2) + 0.5) / full_dim;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let through = scene.inv_view_proj * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(through.xyz / through.w - scene.camera_pos.xyz) * distance;
}

fn resolve(frag: vec2<f32>, along: bool) -> vec4<f32> {
    let dim = vec2<i32>(textureDimensions(t_src));
    let here = vec2<i32>(frag);
    let centre = textureLoad(t_src, here, 0);
    let width = centre.a;
    if (width < 0.5) {
        return centre;
    }
    let geometry = textureLoad(t_geometry, here, 0);
    let dist = geometry.x;
    if (dist <= 0.0) {
        return centre;
    }
    let full_dim = vec2<f32>(textureDimensions(t_geometry)) * 2.0;
    let normal = decode_octahedral(geometry.yz);
    let position = surface_at(here, dist, full_dim);
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
            let there = textureLoad(t_geometry, at, 0);
            if (there.x <= 0.0 || dot(decode_octahedral(there.yz), normal) < 0.9) {
                continue;
            }
            if (abs(dot(surface_at(at, there.x, full_dim) - position, normal)) > tolerance) {
                continue;
            }
            sum += found.rgb * w;
            total += w;
        }
    }
    return vec4(sum / total, width);
}
