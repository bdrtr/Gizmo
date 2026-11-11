// G-Buffer geometry pass.
// Writes opaque lit surfaces to the G-buffer's targets; unlit/skybox objects are discarded here
// and drawn in a subsequent forward pass.
//
// What the surface is comes from the engine's material (`gizmo::material`), or, composed with
// `CUSTOM_SURFACE`, from a game's `material_surface` (`custom_material::SurfaceMaterial`): the same
// shader around it, so a game's surface is written to the G-buffer exactly as the engine's own is.

#import gizmo::common::{instance_keeps, dielectric_f0}
#import gizmo::surface::{Surface, tangent_frame, surface_lobes}
#ifdef CUSTOM_SURFACE
#import gizmo::surface::{SurfaceInput}
#import gizmo::custom_surface::{material_surface}
#else
#import gizmo::material::{evaluate_surface, surface_uv}
#endif

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
    // `InstanceRaw::optics`: xyz = 1 − specular tint, w = 0xFFFFFF − a glass's volume (8 bits a channel).
    optics: vec4<f32>,
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
    // `Material::clear_coat_roughness`.
    @location(11) inst_coat_roughness: f32,
    // The instance's cut-out and LOD crossfade, `instance_keeps`.
    @location(12) inst_keep: f32,
    // `Material::specular_tint`, from `1 − optics.xyz`.
    @location(13) inst_tint: vec3<f32>,
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
#ifdef CUSTOM_SURFACE
    // What group 1 means is the game's, so the material's UV transform is not applied here: a
    // surface that wants it calls `gizmo::material::standard_surface`, which does.
    out.tex_coords = input.tex_coords;
#else
    out.tex_coords = surface_uv(input.tex_coords);
#endif

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
    out.inst_coat_roughness = inst.ambient.z;
    out.inst_keep = inst.ambient.w;
    out.inst_tint = vec3<f32>(1.0) - inst.optics.xyz;
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
//   RT4  velocity         Rgba16Float  — xy = screen-UV motion since last frame (`crate::motion`),
//                                        zw = the specular tint's hue, `1 −` red and blue over luminance
//   RT5  emissive         Rgba16Float  — emitted light, added unlit after lighting; a = F0 × the tint's
//                                        luminance, negative for no shadows (48 bytes in all)
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

// What the material is at this fragment: the engine's own, or the game's `material_surface`.
fn fragment_surface(in: VertexOutput, front: bool) -> Surface {
#ifdef CUSTOM_SURFACE
    var input: SurfaceInput;
    input.world_position = in.world_position;
    input.uv = in.tex_coords;
    input.vertex_color = in.color;
    input.N = in.normal;
    input.T = in.world_tangent.xyz;
    input.B = in.bitangent;
    input.albedo = in.inst_albedo;
    input.roughness = in.inst_pbr.x;
    input.metallic = in.inst_pbr.y;
    input.emissive = in.inst_emissive.xyz;
    input.front_facing = front;
    input.frag_coord = in.clip_position;
    input.time = scene.cascade_params.z;
    input.camera_position = scene.camera_pos.xyz;
    return material_surface(input);
#else
    return evaluate_surface(
        in.tex_coords, in.normal, in.world_tangent.xyz, in.bitangent, in.color, in.inst_albedo, in.inst_pbr, in.inst_emissive.xyz,
    );
#endif
}

// The depth prepass of a draw that discards: a cut-out, or a LOD crossfade. The plain prepass has no
// fragment stage, which is right for everything else and wrong here: it writes depth over the holes,
// and what stands behind them then fails the depth test in the G-buffer pass. A game's surface always
// takes this one, since only its own code knows whether it cuts.
@fragment
fn fs_prepass(in: VertexOutput, @builtin(front_facing) front: bool) {
    let surface = fragment_surface(in, front);
    if (surface.alpha_cutoff > 0.0 && surface.alpha < surface.alpha_cutoff) { discard; }
    if (!instance_keeps(in.inst_keep, surface.alpha, in.clip_position.xy)) { discard; }
}

// How a surface drawn forward moved — unlit, baked-lit — which writes depth and no G-buffer: RT4's `xy`
// alone (its pipeline writes nothing else, so the tint's hue in `zw` stays the surface's behind it,
// which no forward surface reads), where the forward draw left the frame's depth. TAA reprojects by
// it and motion blur smears along it; without it both read the motion of what stands behind.
@fragment
fn fs_motion(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let surface = fragment_surface(in, front);
    if (surface.alpha_cutoff > 0.0 && surface.alpha < surface.alpha_cutoff) { discard; }
    if (!instance_keeps(in.inst_keep, surface.alpha, in.clip_position.xy)) { discard; }
    let now_ndc    = in.motion_now.xy / in.motion_now.w;
    let before_ndc = in.motion_before.xy / in.motion_before.w;
    return vec4<f32>((now_ndc - before_ndc) * vec2<f32>(0.5, -0.5), 0.0, 0.0);
}

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> GBufferOut {
    // Skip unlit / skybox objects — they are drawn in a forward pass
    if (in.inst_pbr.z > 0.5) { discard; }

    let surface = fragment_surface(in, front);
    // glTF AlphaMode::Mask cutout: hard per-texel discard at alphaCutoff.
    // cutoff == 0 → no cutout (Opaque / Blend materials), so this is a no-op for them.
    if (surface.alpha_cutoff > 0.0 && surface.alpha < surface.alpha_cutoff) { discard; }
    if (!instance_keeps(in.inst_keep, surface.alpha, in.clip_position.xy)) { discard; }

    let albedo = surface.albedo;
    let metallic = surface.metallic;
    // A back face is lit as what it is, with the normal towards the viewer. It carried the front's
    // until 2026-11-07, so the inside of a double-sided cube was lit from the outside.
    let N = select(-surface.N, surface.N, front);
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
    // Two numbers and a sign in one half float, as an integer it holds exactly (up to 2048): 32 steps
    // of coat, 32 of its roughness from the hard lacquer up, and 1 so that nothing is still signed.
    let coat_steps = round(clear_coat_raw * 31.0);
    let coat_rough_steps = round(clamp((in.inst_coat_roughness - 0.08) / 0.92, 0.0, 1.0) * 31.0);
    let packed_tangent_w = handedness * (1.0 + coat_steps * 32.0 + coat_rough_steps);

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
    // `Material::specular_tint` has three numbers and the G-buffer two free channels: its luminance goes
    // into the F0 below and its hue into RT4's `zw`, red and blue over the luminance, as `1 −` that so
    // that a target nothing wrote reads white. The lighting pass puts green back from the luminance.
    let tint = max(in.inst_tint, vec3<f32>(0.0));
    let tint_luminance = max(dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-4);
    let hue = tint / tint_luminance;
    out.velocity   = vec4<f32>((now_ndc - before_ndc) * vec2<f32>(0.5, -0.5), 1.0 - hue.r, 1.0 - hue.b);
    // .w: the F0 (`Material::reflectance`) times the tint's luminance, negative when
    // `MeshRenderer::receives_shadows == false`. The lighting pass reads both; see `dielectric_f0`.
    let f0 = dielectric_f0(in.inst_emissive.w) * tint_luminance;
    out.emissive   = vec4<f32>(max(emissive, vec3<f32>(0.0)), select(f0, -f0, in.inst_emissive.w < 0.0));
    return out;
}
