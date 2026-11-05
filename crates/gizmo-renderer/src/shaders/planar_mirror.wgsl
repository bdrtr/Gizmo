// A surface showing what `PlanarReflection` captured from behind it: the reflection texture, looked
// up by where this fragment lands on screen, with a Fresnel rise towards grazing angles.
//
// The capture is the camera reflected in the plane, drawn with an ordinary rotation so nothing
// has to reverse its winding. Reflection would have turned the picture inside out, so a rotation's
// picture is the true one mirrored left to right: hence `1.0 - u`.
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_reflection: texture_2d<f32>;
@group(1) @binding(1)
var s_reflection: sampler;

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
    // Mirrors `gpu_types::InstanceRaw`. `ambient.x` carries the reflectance head-on.
    ambient: vec4<f32>,
    emissive: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
var<storage, read> instances: array<InstanceRaw>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(2) normal: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    // The clip position again, as a varying: from it the screen coordinate needs no viewport.
    @location(0) clip: vec4<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) base: vec3<f32>,
    @location(4) reflectance: f32,
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(inst.model_matrix_0, inst.model_matrix_1, inst.model_matrix_2, inst.model_matrix_3);
    let world = model * vec4<f32>(input.position, 1.0);
    var out: VertexOutput;
    out.position = scene.view_proj * world;
    out.clip = out.position;
    out.world_position = world.xyz;
    // Scale is removed by the normalize below; a mirror is flat, so shear does not arise.
    out.normal = (model * vec4<f32>(input.normal, 0.0)).xyz;
    out.base = inst.albedo_color.rgb;
    out.reflectance = inst.ambient.x;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let ndc = in.clip.xy / in.clip.w;
    let uv = vec2<f32>(1.0 - (ndc.x * 0.5 + 0.5), 0.5 - ndc.y * 0.5);
    let reflected = textureSample(t_reflection, s_reflection, uv).rgb;
    let to_eye = normalize(scene.camera_pos.xyz - in.world_position);
    let facing = clamp(abs(dot(normalize(in.normal), to_eye)), 0.0, 1.0);
    // Schlick: `reflectance` head-on, everything at a grazing angle.
    let f = in.reflectance + (1.0 - in.reflectance) * pow(1.0 - facing, 5.0);
    return vec4<f32>(mix(in.base, reflected, f), 1.0);
}
