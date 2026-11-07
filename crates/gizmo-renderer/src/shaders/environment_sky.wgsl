// An `EnvironmentMap` behind the scene: a fullscreen triangle at the far plane, so it lands only where
// nothing was drawn.
#import gizmo::common::{SceneUniforms}
#ifndef ENV_CUBE
#import gizmo::reflection_probes::{environment_background}
#endif

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

#ifdef ENV_CUBE
// The forward-only path's environment cube (`env_cube`), which holds the map where the desktop's probe
// array would, its intensity already in it.
@group(0) @binding(4) var s_behind: sampler;
@group(0) @binding(5) var t_environment: texture_cube<f32>;

fn environment_background(direction: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(t_environment, s_behind, direction, 0.0).rgb;
}
#endif

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    let ndc = uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    var out: VertexOut;
    out.position = vec4<f32>(ndc, 1.0, 1.0);
    out.ndc = ndc;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let far = scene.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    return vec4<f32>(environment_background(far.xyz / far.w - scene.camera_pos.xyz), 1.0);
}
