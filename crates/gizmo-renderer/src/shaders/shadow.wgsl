// Depth into the shadow maps: the sun's cascades and a point light's six faces.
//
// Plain, it is a vertex stage alone: what a surface is does not move where it is. With `CUT_OUT` it
// has a fragment stage too, which discards where the surface is not — a cut-out's holes, a LOD
// crossfade's dither, and, composed with `CUSTOM_SURFACE`, wherever a game's surface says it is cut.
// Without it a cut-out cast its whole quad (2026-11-11).
//
// Every shadow pipeline has the same groups, so a pass binds the same ones whichever it draws with:
// 0 this pass's light, 1 the material (read only with `CUT_OUT`), 2 the skeleton, 3 the instances.

#ifdef CUT_OUT
#import gizmo::common::{instance_keeps}
#import gizmo::surface::{Surface, tangent_frame}
#ifdef CUSTOM_SURFACE
#import gizmo::surface::{SurfaceInput}
#import gizmo::custom_surface::{material_surface}
#else
#import gizmo::material::{evaluate_surface, surface_uv}
#endif
#endif

// Mirrors `gpu_types::ShadowVsUniform`.
struct ShadowVsUniform {
    light_view_proj: mat4x4<f32>,
    // The frame's camera, for a surface whose cut depends on where it is seen from.
    camera_position: vec4<f32>,
    // x = seconds since the renderer started: the clock `SceneUniforms` carries.
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> shadow_vs: ShadowVsUniform;

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>,
};
@group(2) @binding(0) var<uniform> skeleton: SkeletonData;

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
    ambient: vec4<f32>,
    emissive: vec4<f32>,
    optics: vec4<f32>,
};
@group(3) @binding(0) var<storage, read> instances: array<InstanceData>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
#ifdef CUT_OUT
    @location(6) tangent: vec4<f32>,
#endif
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
#ifdef CUT_OUT
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo: vec4<f32>,
    @location(5) inst_pbr: vec4<f32>,
    @location(6) tangent: vec3<f32>,
    @location(7) bitangent: vec3<f32>,
    @location(8) inst_emissive: vec3<f32>,
    // The instance's cut-out threshold and LOD crossfade, `instance_keeps`.
    @location(9) inst_keep: f32,
#endif
};

fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let cross01 = cross(m[0], m[1]);
    let cross12 = cross(m[1], m[2]);
    let cross20 = cross(m[2], m[0]);
    let inv_det = 1.0 / dot(m[2], cross01);
    return mat3x3<f32>(cross12 * inv_det, cross20 * inv_det, cross01 * inv_det);
}

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(inst.model_matrix_0, inst.model_matrix_1, inst.model_matrix_2, inst.model_matrix_3);

    var skin_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    );
    if (input.joint_weights.x + input.joint_weights.y + input.joint_weights.z + input.joint_weights.w > 0.0) {
        skin_mat =
            input.joint_weights.x * skeleton.joints[input.joint_indices.x] +
            input.joint_weights.y * skeleton.joints[input.joint_indices.y] +
            input.joint_weights.z * skeleton.joints[input.joint_indices.z] +
            input.joint_weights.w * skeleton.joints[input.joint_indices.w];
    }
    let skinned_pos = skin_mat * vec4<f32>(input.position, 1.0);
    let world_pos = model * vec4<f32>(skinned_pos.xyz, 1.0);

    var out: VertexOutput;
    out.clip_position = shadow_vs.light_view_proj * world_pos;
#ifdef CUT_OUT
    out.color = input.color;
#ifdef CUSTOM_SURFACE
    out.tex_coords = input.tex_coords;
#else
    out.tex_coords = surface_uv(input.tex_coords);
#endif
    out.world_position = world_pos.xyz;
    let skin_normal_mat = inverse_transpose_3x3(mat3x3<f32>(skin_mat[0].xyz, skin_mat[1].xyz, skin_mat[2].xyz));
    let normal_mat = inverse_transpose_3x3(mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz));
    let skinned_tangent = skin_mat * vec4<f32>(input.tangent.xyz, 0.0);
    let model_mat3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    let frame = tangent_frame(normal_mat * (skin_normal_mat * input.normal), model_mat3 * skinned_tangent.xyz, input.tangent.w);
    out.normal = frame.N;
    out.tangent = frame.T;
    out.bitangent = frame.B;
    out.inst_albedo = inst.albedo_color;
    out.inst_pbr = inst.pbr;
    out.inst_emissive = inst.emissive.xyz;
    out.inst_keep = inst.ambient.w;
#endif
    return out;
}

#ifdef CUT_OUT
// What the material is at this fragment — as `gbuffer.wgsl` and `shader.wgsl` ask it, so a surface
// is cut in its shadow exactly where it is cut in the picture.
fn fragment_surface(in: VertexOutput, front: bool) -> Surface {
#ifdef CUSTOM_SURFACE
    var input: SurfaceInput;
    input.world_position = in.world_position;
    input.uv = in.tex_coords;
    input.vertex_color = in.color;
    input.N = in.normal;
    input.T = in.tangent;
    input.B = in.bitangent;
    input.albedo = in.inst_albedo;
    input.roughness = in.inst_pbr.x;
    input.metallic = in.inst_pbr.y;
    input.emissive = in.inst_emissive;
    input.front_facing = front;
    input.frag_coord = in.clip_position;
    input.time = shadow_vs.params.x;
    input.camera_position = shadow_vs.camera_position.xyz;
    return material_surface(input);
#else
    return evaluate_surface(
        in.tex_coords, in.normal, in.tangent, in.bitangent, in.color, in.inst_albedo, in.inst_pbr, in.inst_emissive,
    );
#endif
}

@fragment
fn fs_cut_out(in: VertexOutput, @builtin(front_facing) front: bool) {
    let surface = fragment_surface(in, front);
    if (surface.alpha_cutoff > 0.0 && surface.alpha < surface.alpha_cutoff) { discard; }
    if (!instance_keeps(in.inst_keep, surface.alpha, in.clip_position.xy)) { discard; }
}
#endif
