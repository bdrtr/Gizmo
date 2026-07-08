// G-Buffer geometry pass.
// Writes opaque PBR surfaces to three MRTs; unlit/skybox objects are discarded here
// and drawn in a subsequent forward pass.

fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let cross01 = cross(m[0], m[1]);
    let cross12 = cross(m[1], m[2]);
    let cross20 = cross(m[2], m[0]);
    let inv_det = 1.0 / dot(m[2], cross01);
    return mat3x3<f32>(cross12 * inv_det, cross20 * inv_det, cross01 * inv_det);
}

struct SceneUniforms {
    view_proj:      mat4x4<f32>,
    camera_pos:     vec4<f32>,
    sun_direction:  vec4<f32>,
    sun_color:      vec4<f32>,
    lights:         array<vec4<f32>, 40>, // 10 * LightData (4 vec4 each) — not used in G-pass
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,
    num_lights: u32,
    _pad: vec3<u32>,
};

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>,
};

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color:   vec4<f32>,
    pbr:            vec4<f32>,  // x=roughness, y=metallic, z=unlit_flag
};

struct VertexInput {
    @location(0) position:      vec3<f32>,
    @location(1) color:         vec3<f32>,
    @location(2) normal:        vec3<f32>,
    @location(3) tex_coords:    vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
    @location(6) tangent:       vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color:         vec3<f32>,
    @location(1) normal:        vec3<f32>,
    @location(2) tex_coords:    vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo:   vec4<f32>,
    @location(5) inst_pbr:      vec4<f32>,
    @location(6) world_tangent:  vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
@group(3) @binding(0) var<uniform> skeleton: SkeletonData;
@group(4) @binding(0) var<storage, read> instances: array<InstanceData>;

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color     = input.color;
    out.tex_coords = input.tex_coords;

    let inst  = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0, inst.model_matrix_1,
        inst.model_matrix_2, inst.model_matrix_3,
    );

    var skin_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0), vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0),
    );
    if (input.joint_weights.x + input.joint_weights.y + input.joint_weights.z + input.joint_weights.w > 0.0) {
        skin_mat =
            input.joint_weights.x * skeleton.joints[input.joint_indices.x] +
            input.joint_weights.y * skeleton.joints[input.joint_indices.y] +
            input.joint_weights.z * skeleton.joints[input.joint_indices.z] +
            input.joint_weights.w * skeleton.joints[input.joint_indices.w];
    }

    let skinned_pos  = skin_mat * vec4<f32>(input.position, 1.0);
    let world_pos    = model    * vec4<f32>(skinned_pos.xyz, 1.0);
    out.world_position = world_pos.xyz;

    // Normal skin uzayında inverse-transpose ile taşınır (non-uniform bone scale/shear
    // doğru; rigid/uniform'da no-op çünkü fragment'ta normalize edilir). Tangent ise
    // bir yön olarak doğrudan matrisle taşınır (inverse-transpose DEĞİL).
    let skin_normal_mat = inverse_transpose_3x3(mat3x3<f32>(skin_mat[0].xyz, skin_mat[1].xyz, skin_mat[2].xyz));
    let skinned_normal = skin_normal_mat * input.normal;
    let normal_mat     = inverse_transpose_3x3(mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz));
    out.normal = normal_mat * skinned_normal;

    let skinned_tangent = skin_mat * vec4<f32>(input.tangent.xyz, 0.0);
    out.world_tangent = vec4<f32>(normal_mat * skinned_tangent.xyz, input.tangent.w);

    out.inst_albedo = inst.albedo_color;
    out.inst_pbr    = inst.pbr;
    out.clip_position = scene.view_proj * world_pos;
    return out;
}

// G-Buffer output (4 MRTs fit the 32 bytes/sample color-attachment budget: 4+8+8+8=28):
//   RT0  albedo_metallic  Rgba8Unorm   — rgb=albedo,  a=metallic
//   RT1  normal_roughness Rgba16Float  — rgb=normal,  a=roughness
//   RT2  world_position   Rgba16Float  — rgb=pos,     a=written-flag + packed subsurface/anisotropy
//   RT3  world_tangent    Rgba16Float  — rgb=tangent, a=handedness + packed clear-coat
struct GBufferOut {
    @location(0) albedo_metallic:  vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position:   vec4<f32>,
    @location(3) world_tangent:    vec4<f32>,
};

@fragment
fn fs_main(in: VertexOutput) -> GBufferOut {
    // Skip unlit / skybox objects — they are drawn in a forward pass
    if (in.inst_pbr.z > 0.5) { discard; }

    let tex_color   = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let final_alpha = in.inst_albedo.a * tex_color.a;
    // if (final_alpha < 0.5) { discard; }

    var raw_normal = in.normal;
    if (length(raw_normal) < 0.001) { raw_normal = vec3<f32>(0.0, 1.0, 0.0); }
    let N = normalize(raw_normal);

    var raw_tangent = in.world_tangent.xyz;
    if (length(raw_tangent) < 0.001) {
        if (abs(N.x) > 0.9) {
            raw_tangent = cross(vec3<f32>(0.0, 1.0, 0.0), N);
        } else {
            raw_tangent = cross(vec3<f32>(1.0, 0.0, 0.0), N);
        }
    }
    let T = normalize(raw_tangent);

    let albedo   = in.inst_albedo.rgb * tex_color.rgb;
    let metallic = clamp(in.inst_pbr.y, 0.0, 1.0);
    let roughness = clamp(in.inst_pbr.x, 0.05, 1.0);

    // Unpack anisotropy, clear_coat, and subsurface from in.inst_pbr.w (packed_params)
    let subsurface_raw = floor(in.inst_pbr.w / 1000000.0) / 100.0;
    let rem_packed = in.inst_pbr.w - floor(in.inst_pbr.w / 1000000.0) * 1000000.0;
    let clear_coat_raw = floor(rem_packed / 1000.0) / 1000.0;
    let anisotropy_raw = (rem_packed - floor(rem_packed / 1000.0) * 1000.0) / 1000.0;

    let packed_tangent_w = sign(in.world_tangent.w) * (0.01 + 0.99 * clear_coat_raw);

    var out: GBufferOut;
    out.albedo_metallic  = vec4<f32>(albedo, metallic);
    out.normal_roughness = vec4<f32>(N, roughness);
    out.world_position   = vec4<f32>(in.world_position, (0.5 + 0.49 * anisotropy_raw) + 100.0 * subsurface_raw);
    out.world_tangent    = vec4<f32>(T, packed_tangent_w);
    return out;
}
