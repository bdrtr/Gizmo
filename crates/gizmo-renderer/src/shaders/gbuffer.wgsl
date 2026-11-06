// G-Buffer geometry pass.
// Writes opaque PBR surfaces to three MRTs; unlit/skybox objects are discarded here
// and drawn in a subsequent forward pass.

#import gizmo::surface::{evaluate_surface, surface_uv, tangent_frame, surface_lobes}

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
    lights:         array<vec4<f32>, 1024>, // MAX_LIGHTS(256) * LightData (4 vec4 each) — not used in G-pass
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
    // Mirrors `gpu_types::InstanceRaw`. `emissive` is read here (RT5); `ambient` only by
    // `baked_lit.wgsl`, but every shader indexing this buffer must declare both or its element
    // stride is wrong.
    ambient: vec4<f32>,
    emissive: vec4<f32>,
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
    @location(7) motion_now:     vec4<f32>,
    @location(8) motion_before:  vec4<f32>,
    // `Material::emissive`, per instance — what game code sets with `with_emissive`.
    @location(9) inst_emissive:  vec4<f32>,  // w = F0, < 0: receives no shadows
    @location(10) bitangent:     vec3<f32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;
@group(3) @binding(0) var<uniform> skeleton: SkeletonData;
@group(4) @binding(0) var<storage, read> instances: array<InstanceData>;

// Motion (`crate::motion`): this and last frame's UNJITTERED view-projections, and the model each
// instance was drawn with last frame, in instance order. The velocity target is the difference
// between the two projections of the same skinned point — object and camera motion together.
struct Motion {
    view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};
@group(5) @binding(0) var<uniform> motion: Motion;
@group(5) @binding(1) var<storage, read> previous_models: array<mat4x4<f32>>;

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color     = input.color;
    out.tex_coords = surface_uv(input.tex_coords);

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

    // Tangent bir yüzey YÖNÜdür → düz model 3x3 ile taşınır, inverse-transpose DEĞİL.
    // skin yarısı (skin_mat) zaten düz; model yarısı da düz olmalı.
    let skinned_tangent = skin_mat * vec4<f32>(input.tangent.xyz, 0.0);
    let model_mat3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    let frame = tangent_frame(normal_mat * skinned_normal, model_mat3 * skinned_tangent.xyz, input.tangent.w);
    out.normal = frame.N;
    out.world_tangent = vec4<f32>(frame.T, input.tangent.w);
    out.bitangent = frame.B;

    out.inst_albedo = inst.albedo_color;
    out.inst_pbr    = inst.pbr;
    out.inst_emissive = inst.emissive;
    out.clip_position = scene.view_proj * world_pos;
    out.motion_now    = motion.view_proj * world_pos;
    out.motion_before = motion.prev_view_proj * (previous_models[instance_idx] * vec4<f32>(skinned_pos.xyz, 1.0));
    return out;
}

// G-Buffer output — 36 bytes per sample in WebGPU's accounting (Rgba8UnormSrgb counts 8, not 4:
// the old note here added these up to 28), past the default 32, so the native device asks for more
// (`native_device_limits`):
//   RT0  albedo_metallic  Rgba8Unorm   — rgb=albedo,  a=metallic
//   RT1  normal_roughness Rgba16Float  — rgb=normal,  a=roughness
//   RT2  world_position   Rgba16Float  — rgb=pos,     a=written-flag + packed subsurface/anisotropy
//   RT3  world_tangent    Rgba16Float  — rgb=tangent, a=handedness + packed clear-coat
//   RT4  velocity         Rg16Float    — screen-UV motion since last frame (`crate::motion`)
//   RT5  emissive         Rgba16Float  — emitted light, added unlit after lighting (44 bytes in all)
struct GBufferOut {
    @location(0) albedo_metallic:  vec4<f32>,
    @location(1) normal_roughness: vec4<f32>,
    @location(2) world_position:   vec4<f32>,
    @location(3) world_tangent:    vec4<f32>,
    // RT4: screen-UV distance moved since last frame (x right, y down).
    @location(4) velocity:         vec4<f32>,
    // RT5: emitted light, linear HDR.
    @location(5) emissive:         vec4<f32>,
};

@fragment
fn fs_main(in: VertexOutput) -> GBufferOut {
    // Skip unlit / skybox objects — they are drawn in a forward pass
    if (in.inst_pbr.z > 0.5) { discard; }

    let surface = evaluate_surface(
        in.tex_coords, in.normal, in.world_tangent.xyz, in.bitangent, in.color, in.inst_albedo, in.inst_pbr, in.inst_emissive.xyz,
    );
    // glTF AlphaMode::Mask cutout: hard per-texel discard at alphaCutoff.
    // cutoff == 0 → no cutout (Opaque / Blend materials), so this is a no-op for them.
    if (surface.alpha_cutoff > 0.0 && surface.alpha < surface.alpha_cutoff) { discard; }

    let albedo = surface.albedo;
    let metallic = surface.metallic;
    let N = surface.N;
    let T = normalize(in.world_tangent.xyz);
    let roughness = surface.roughness;
    let emissive = surface.emissive;
    let lobes = surface_lobes(in.inst_pbr);
    let anisotropy_raw = lobes.x;
    let clear_coat_raw = lobes.y;
    let subsurface_raw = lobes.z;

    // `select`, not `sign`: `sign(0.0)` is 0, which would zero the clear-coat magnitude as well as
    // the handedness for any mesh whose tangent w is 0. The decoder already resolves 0 to +1
    // (`tangent_sample.w >= 0.0`); this makes the encoder agree, instead of leaving the two ends
    // with different ideas about the same edge case.
    let handedness = select(-1.0, 1.0, in.world_tangent.w >= 0.0);
    let packed_tangent_w = handedness * (0.01 + 0.99 * clear_coat_raw);

    var out: GBufferOut;
    out.albedo_metallic  = vec4<f32>(albedo, metallic);
    out.normal_roughness = vec4<f32>(N, roughness);
    // **Stored relative to the camera, not in absolute world coordinates.** The four G-buffer
    // targets share a 32-byte-per-sample budget (4 + 8 + 8 + 8 = 28), so this one cannot be
    // Rgba32Float — but the budget is about the *bytes*, not about what goes in them. Absolute
    // coordinates in f16 quantise to 6 cm at 100 m from the origin, 50 cm at 1 km and a full
    // metre at 2 km, against a nearest-cascade shadow texel of 4.3 mm; a city-sized level was
    // therefore sampling its shadows from a position rounded to the nearest half-metre. Storing
    // the offset from the camera keeps the same 8 bytes and the values stay view-scale, where f16
    // is good to centimetres anywhere in the world. Every reader adds `camera_pos` back.
    // **`floor` before packing, or the two values are not separable.** `.w` carries subsurface in
    // its integer part and anisotropy in its fraction, and the decoder recovers them with
    // `floor(w)/100` and `frac(w)`. That only works if `100·subsurface` is already an integer.
    // It usually is not, and then subsurface's own fraction lands in anisotropy's slot: measured,
    // subsurface 0.234 with anisotropy **0** decoded as **0.82**, and with anisotropy **1** as
    // **0**, i.e. exactly inverted. Rounding the integer part here costs the 1 % of subsurface
    // resolution the `/100` decode was always going to quantise away anyway, and makes the
    // fraction mean only what it is supposed to.
    let packed_ss_aniso = (0.5 + 0.49 * anisotropy_raw) + floor(100.0 * subsurface_raw);
    out.world_position   = vec4<f32>(in.world_position - scene.camera_pos.xyz, packed_ss_aniso);
    out.world_tangent    = vec4<f32>(T, packed_tangent_w);
    let now_ndc    = in.motion_now.xy / in.motion_now.w;
    let before_ndc = in.motion_before.xy / in.motion_before.w;
    out.velocity   = vec4<f32>((now_ndc - before_ndc) * vec2<f32>(0.5, -0.5), 0.0, 0.0);
    // .w: `Material::reflectance`, negative when `MeshRenderer::receives_shadows == false`. The
    // lighting pass reads both; see `dielectric_f0`.
    out.emissive   = vec4<f32>(max(emissive, vec3<f32>(0.0)), in.inst_emissive.w);
    return out;
}
