// How much of each pixel's G-buffer surface the finished opaque frame shows (`DeferredState::seen_view`):
// 1 where the frame's depth is that surface, 0 where something drawn forward stands in front of it —
// an unlit or baked-lit surface, a game's own pipeline — which writes depth and no G-buffer.
//
// SSR and SSGI describe the G-buffer's surface and are added after the forward pass, so without this
// they were added over whatever stood in front of it: an opaque unlit panel took the reflection and
// the bounce light of the floor behind it.
#import gizmo::common::SceneUniforms

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_depth: texture_depth_2d;
@group(1) @binding(1) var t_position_rel_camera: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4<f32>(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(frag.xy);
    let surface = textureLoad(t_position_rel_camera, pixel, 0);
    if (surface.w < 0.5) {
        return vec4<f32>(0.0);
    }
    let d = textureLoad(t_depth, pixel, 0);
    let dims = vec2<f32>(textureDimensions(t_depth));
    let uv = (vec2<f32>(pixel) + 0.5) / dims;
    let h = scene.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let fwd = scene.camera_forward.xyz;
    let nearest = dot(h.xyz / h.w - scene.camera_pos.xyz, fwd);
    let gbuffer = dot(surface.xyz, fwd);
    // The G-buffer's position is f16, camera-relative: half a unit in the last place is 2^-11 of its
    // largest component. Four of those, and a millimetre: a forward surface nearer than that is lying
    // on this one, not in front of it.
    let p = abs(surface.xyz);
    let tolerance = max(max(p.x, p.y), p.z) * exp2(-9.0) + 1e-3;
    return vec4<f32>(select(0.0, 1.0, gbuffer - nearest <= tolerance));
}
