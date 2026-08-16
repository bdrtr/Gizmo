// Forward Decal Shader — the editor's copy of `decal.wgsl`.
//
// A decal is a projector: it paints whatever surface is already there, so it needs that
// surface's world position. The deferred decal reads it from the G-buffer, which is why a decal
// placed in the editor was invisible until the game ran — the editor's pipeline is forward and
// has no G-buffer at all. This one reconstructs the same position from the DEPTH buffer, which
// the forward pass does write, and blends into the lit HDR image instead of into the albedo
// target.
//
// Everything else is deliberately identical to the deferred version: the same unit-cube volume,
// the same box test, the same XZ projection and the same circular fade — a decal that looks one
// way in the editor and another in the game is worse than one that is missing.
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_depth: texture_depth_2d;

struct DecalUniforms {
    inv_model: mat4x4<f32>,
    model: mat4x4<f32>,
    albedo_color: vec4<f32>,
}
@group(2) @binding(0) var t_albedo: texture_2d<f32>;
@group(2) @binding(1) var s_albedo: sampler;

@group(3) @binding(0) var<uniform> decal: DecalUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.position = scene.view_proj * decal.model * vec4<f32>(pos, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(in.position.xy);
    let depth = textureLoad(t_depth, coord, 0);

    // Nothing was drawn here — sky. A projector with no surface to project onto paints nothing.
    if (depth >= 1.0) {
        discard;
    }

    // Reconstruct the world position of whatever the depth buffer says is here.
    let dims = vec2<f32>(textureDimensions(t_depth));
    let uv = in.position.xy / dims;
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let h = scene.inv_view_proj * vec4<f32>(ndc, 1.0);
    let world_pos = h.xyz / h.w;

    // `inv_model` is the CPU-folded `model⁻¹ · T(camera)`, built for the deferred path where the
    // G-buffer stores positions RELATIVE to the camera. Subtracting the camera here feeds it
    // exactly what it expects, so both paths share one uniform buffer and cannot disagree about
    // where the decal is.
    let local_pos = decal.inv_model * vec4<f32>(world_pos - scene.camera_pos.xyz, 1.0);

    // Unit cube bounds [-0.5, 0.5]: outside the projector volume, paint nothing.
    if (abs(local_pos.x) > 0.5 || abs(local_pos.y) > 0.5 || abs(local_pos.z) > 0.5) {
        discard;
    }

    // Project the UV down the local Y axis onto the XZ plane.
    let decal_uv = vec2<f32>(local_pos.x + 0.5, local_pos.z + 0.5);
    var color = textureSample(t_albedo, s_albedo, decal_uv) * decal.albedo_color;

    // Circular fade, so it reads as a splatter rather than a box.
    let dist = distance(decal_uv, vec2<f32>(0.5, 0.5));
    color.a *= 1.0 - smoothstep(0.3, 0.5, dist);

    return color;
}
