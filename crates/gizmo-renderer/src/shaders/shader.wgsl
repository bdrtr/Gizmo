// SceneUniforms, LightData and the core BRDF (D_GGX/V_SmithJointGGX/F_Schlick/
// compute_direct_lighting) come from gizmo::common, composed by load_shader_composed
// (native) / load_shader_composed_web (web), and what a material is comes from gizmo::material,
// shared with the G-buffer — or, composed with `CUSTOM_SURFACE`, from a game's `material_surface`
// (`custom_material::SurfaceMaterial`), lit here as the engine's own would be. The forward shader diverges from the deferred
// path only in bind-group layout: WebGPU caps groups at 4, so the CSM shadow group is
// dropped on web. That divergence is expressed at the SOURCE via the shadow defs (`SUN_SHADOWS`,
// `LOCAL_LIGHT_MAPS`, with the group and binding each is read from) and `@group(#{SKELETON_GROUP})` / `@group(#{INSTANCE_GROUP})`
// (3/4 native, 2/3 web) — never by grepping naga's reformatted output.
// Three tiers on the forward-only path, where a phone pays for code whether or not a draw takes it.
// None of the defs: a plain opaque surface, which is most of a frame. `DETAILS` adds what a draw has to
// ask for: a back face lit as one, cut-outs, LOD crossfades, its own reflectance. `LOBES` adds anisotropy,
// clear coat and subsurface. The desktop path has them all, always. Measured on an Adreno 618 whose big
// draw is one floor, twenty readings each: 103 fps on the plain tier, 89 with the details compiled in and
// nothing taking them, 84 when the floor does (and about 97 for the plain tier in that sitting). See
// `SceneState::lit_pipeline`.
#ifdef DETAILS
#import gizmo::common::{dielectric_f0, instance_keeps}
#endif
#import gizmo::common::{SceneUniforms, compute_direct_lighting, scene_environment, height_fog}
// The local lights, in tiers on the forward-only path as the material is (`LightShape`): their loop only
// while a scene has one, a sized source's and a rectangle's lighting only while it has one of those. Not
// taken is not free there: on an Adreno 618 the rectangle's, tube's and sphere's branches cost a lit floor
// about 4.8 ms under one plain point light that took none of them (2026-11-13).
#ifdef LOCAL_LIGHTS
#import gizmo::common::{light_falloff, spot_cone, spot_cookie_uv, gizmo_cluster_index}
#endif
#ifdef SIZED_LIGHTS
#import gizmo::common::{compute_sphere_lighting, compute_tube_lighting}
#endif
#ifdef RECT_LIGHTS
#import gizmo::common::{compute_rect_lighting}
#endif
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
#import gizmo::common::{sun_shadow_coord, sun_shadow_lean, sun_shadow_tap, shadow_distance_fade}
#else
#import gizmo::common::{receiver_axes, sun_shadow_visibility, shadow_distance_fade}
#endif
#endif
#ifdef LOCAL_LIGHT_MAPS
#ifdef LOCAL_SHADOW_TAP
#import gizmo::common::{local_shadow_layer, local_shadow_tap}
#else
#import gizmo::common::{local_shadow_layer, local_shadow_visibility, light_source_radius}
#endif
#endif
#import gizmo::pbr_ext::{approximate_env_brdf}
#ifdef LOBES
#import gizmo::pbr_ext::{compute_direct_lighting_anisotropic, subsurface_ambient, anisotropic_reflection, coat_env_attenuation, coat_env_specular, layer_clear_coat, subsurface_light}
#endif
#import gizmo::surface::{Surface, tangent_frame}
#ifdef CUSTOM_SURFACE
#import gizmo::surface::{SurfaceInput}
#import gizmo::custom_surface::{material_surface}
#else
#import gizmo::material::{evaluate_surface, surface_uv}
#endif
#ifdef LOBES
#import gizmo::surface::{surface_lobes}
#endif
#ifdef PROBES
#import gizmo::reflection_probes::{probe_environment, environment_irradiance}
#endif
#ifdef PROBE_BACKGROUND
#import gizmo::reflection_probes::{probe_background}
#endif
#ifdef PROBE_BOXES
#import gizmo::reflection_probes::{probe_boxes}
#endif
#ifdef IRRADIANCE
#import gizmo::irradiance::{gi_sample}
#endif

// Inverse of a 3x3 matrix for correct normal transformation under non-uniform scale
fn inverse_transpose_3x3(m: mat3x3<f32>) -> mat3x3<f32> {
    let cross01 = cross(m[0], m[1]);
    let cross12 = cross(m[1], m[2]);
    let cross20 = cross(m[2], m[0]);
    let inv_det = 1.0 / dot(m[2], cross01);

    return mat3x3<f32>(
        cross12 * inv_det,
        cross20 * inv_det,
        cross01 * inv_det
    );
}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

// Clustered light lists (gizmo-renderer/src/clustered.rs). On the GLOBAL group because the web
// forward pipeline has no fifth bind group to spare — see `pipeline::layouts`.
@group(0) @binding(1)
var<storage, read> cluster_table: array<vec2<u32>>;
@group(0) @binding(2)
var<storage, read> cluster_light_indices: array<u32>;

#ifdef WIREFRAME
// This draw's `WireframeColor`; alpha 0 means it has none. Group 5 because the wireframe pipeline
// has no reflection probes. Native only: the web has no line mode, and no group to spare.
@group(5) @binding(0)
var<uniform> wire_color: vec4<f32>;
#endif

#ifdef TRANSMISSION
// The frame as it stood before any glass was drawn, with its mip chain (`transmission.rs`).
@group(0) @binding(3)
var t_behind: texture_2d<f32>;
#endif
// Linear, with linear mips: it reads the frame behind glass, and the environment cube.
@group(0) @binding(4)
var s_behind: sampler;
#ifdef ENV_CUBE
// The environment, mip by roughness (`env_cube`).
@group(0) @binding(5)
var t_environment: texture_cube<f32>;
#endif

// The sun's cascades: the shadow group's on the desktop, group 0's on the forward-only path (a
// browser's, a phone's), which has no shadow group and builds its lit pipelines with `SUN_SHADOWS` only
// while the scene has a sun that casts (`SceneState::lit_pipeline`).
#ifdef SUN_SHADOWS
@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_TEXTURE_BINDING})
var t_shadow: texture_depth_2d_array;

@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_SAMPLER_BINDING})
var s_shadow: sampler_comparison;
#endif

#ifdef LOCAL_LIGHT_MAPS
// The local lights' shadows (`gizmo_renderer::local_shadows`), a layer or six a light, and the spots'
// cookies (`gizmo_renderer::cookies`): the shadow group's on the desktop, group 0's on the forward-only
// path, which builds its lit pipelines with them only while a frame draws one
// (`SceneState::set_forward_local_light_maps`).
@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{LOCAL_SHADOW_TEXTURE_BINDING})
var t_local_shadow: texture_depth_2d_array;

@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{COOKIE_TEXTURE_BINDING})
var t_cookie: texture_2d_array<f32>;
@group(#{LOCAL_LIGHT_MAP_GROUP}) @binding(#{COOKIE_SAMPLER_BINDING})
var s_cookie: sampler;

#ifndef SUN_SHADOWS
// Their comparison sampler is the sun's, declared above only with the sun's shadow.
@group(#{SUN_SHADOW_GROUP}) @binding(#{SUN_SHADOW_SAMPLER_BINDING})
var s_shadow: sampler_comparison;
#endif
#endif

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>, // Maksimum 128 kemik destegi
};
@group(#{SKELETON_GROUP}) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
    // Mirrors `gpu_types::InstanceRaw`. Only `baked_lit.wgsl` reads these two, but every
    // shader indexing this buffer must declare them or its element stride is wrong.
    ambient: vec4<f32>,
    emissive: vec4<f32>,
    // `InstanceRaw::optics`: xyz = 1 − specular tint, w = 0xFFFFFF − a glass's volume (8 bits a channel).
    optics: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
var<storage, read> instances: array<InstanceData>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
    @location(6) tangent: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo: vec4<f32>,
    @location(5) inst_pbr: vec4<f32>,
#ifdef DETAILS
    // The cut-out threshold and a LOD crossfade, carried in the instance's `ambient.w` — the same
    // slot `baked_lit.wgsl` reads it from. See `instance_keeps`.
    @location(6) inst_cutoff: f32,
    // `Material::specular_tint`, from `1 − optics.xyz`.
    @location(13) inst_tint: vec3<f32>,
#endif
    // `Material::emissive` — added unlit, as the deferred path adds RT5.
    @location(7) inst_emissive: vec4<f32>,  // w = F0, < 0: receives no shadows
    @location(8) tangent: vec3<f32>,
    @location(9) bitangent: vec3<f32>,
#ifdef TRANSMISSION
    @location(10) inst_glass: vec2<f32>,  // Material::transmission, Material::thickness
    // `Material::volume_transmittance`, packed in `optics.w`: flat, because an interpolated float that
    // large would not hold its low bits.
    @location(12) @interpolate(flat) inst_volume: u32,
#endif
#ifdef LOBES
    @location(11) inst_coat_roughness: f32,  // Material::clear_coat_roughness
#endif
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
    // Where the sun's one cascade sees this point pushed off its front (`sun_shadow_coord`,
    // `sun_shadow_lean`). One location: the glass tier uses every other one there is.
    @location(14) sun_shadow: vec4<f32>,
#endif
#endif
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = input.color;
#ifdef CUSTOM_SURFACE
    // What group 1 means is the game's: see `gbuffer.wgsl`.
    out.tex_coords = input.tex_coords;
#else
    out.tex_coords = surface_uv(input.tex_coords);
#endif
    
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );

    // Skinning Matrix (Skeletal Animation)
    // Eger joint_weights'in tamami 0 ise iskelet yok demektir, kimligi (Identity) koru.
    var skin_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0)
    );
    
    if (input.joint_weights.x + input.joint_weights.y + input.joint_weights.z + input.joint_weights.w > 0.0) {
        skin_mat = 
            input.joint_weights.x * skeleton.joints[input.joint_indices.x] +
            input.joint_weights.y * skeleton.joints[input.joint_indices.y] +
            input.joint_weights.z * skeleton.joints[input.joint_indices.z] +
            input.joint_weights.w * skeleton.joints[input.joint_indices.w];
    }

    // Objenin Vertex'ini dünya evrenine taşı (Model space -> Skin space -> World space)
    let skinned_pos = skin_mat * vec4<f32>(input.position, 1.0);
    let world_pos = model * vec4<f32>(skinned_pos.xyz, 1.0);
    out.world_position = world_pos.xyz;
    
    // Obje veya animasyon döndürüldüğünde ışık da tepki versin (Non-uniform scale desteği ile).
    // Normaller skin uzayında da inverse-transpose ile taşınmalı — aksi halde non-uniform
    // bone scale/shear'da normal kayar. Rigid/uniform bone'da bu no-op'tur (fragment'ta
    // normalize edildiğinden ölçek zaten yok olur), yani yalnız bug-vakasını düzeltir.
    let skin_normal_mat = inverse_transpose_3x3(mat3x3<f32>(skin_mat[0].xyz, skin_mat[1].xyz, skin_mat[2].xyz));
    let skinned_normal = skin_normal_mat * input.normal;
    let normal_matrix = inverse_transpose_3x3(mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz));
    let world_normal = normal_matrix * skinned_normal;
    // A tangent is a direction on the surface, so it goes through the plain matrices, not the
    // inverse transpose.
    let skinned_tangent = skin_mat * vec4<f32>(input.tangent.xyz, 0.0);
    let frame = tangent_frame(world_normal, mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz) * skinned_tangent.xyz, input.tangent.w);
    out.normal = frame.N;
    out.tangent = frame.T;
    out.bitangent = frame.B;
    
    out.inst_albedo = inst.albedo_color;
    out.inst_pbr = inst.pbr;
#ifdef DETAILS
    out.inst_cutoff = inst.ambient.w;
    out.inst_tint = vec3<f32>(1.0) - inst.optics.xyz;
#endif
    out.inst_emissive = inst.emissive;
#ifdef TRANSMISSION
    out.inst_glass = inst.ambient.xy;
    out.inst_volume = 0xFFFFFFu - u32(inst.optics.w);
#endif
#ifdef LOBES
    out.inst_coat_roughness = inst.ambient.z;
#endif

#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
    out.sun_shadow = sun_shadow_coord(scene.light_view_proj[0], world_pos.xyz)
        + vec4<f32>(sun_shadow_lean(scene.light_view_proj[0], scene.cascade_params.y, scene.shadow_bias.y, frame.N), 0.0);
#endif
#endif

    // Kameraya yansıt
    out.clip_position = scene.view_proj * world_pos;
    
    return out;
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

// D_GGX / V_SmithJointGGX / F_Schlick / compute_direct_lighting are imported from
// gizmo::common (see the #import at the top). Only forward-specific code stays local.

// What lights and is reflected by a surface: `scene_environment` along `dir`. On the forward-only path
// it was drawn into a cube once, mip by roughness (`env_cube`), because working it out here at every
// fragment was most of a phone's frame.
#ifdef ENV_CUBE
fn get_procedural_environment(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    let lod = clamp(roughness, 0.0, 1.0) * f32(textureNumLevels(t_environment) - 1u);
    return textureSampleLevel(t_environment, s_behind, dir, lod).rgb;
}
#else
fn get_procedural_environment(dir: vec3<f32>, roughness: f32) -> vec3<f32> {
    return scene_environment(
        dir, roughness, scene.sky_zenith, scene.sky_horizon.rgb, scene.sky_ground.rgb, scene.environment_preset,
        scene.environment_preset_b, scene.environment_blend_t, scene.sun_direction,
    );
}
#endif

// What the material is at this fragment: the engine's own, or the game's `material_surface`.
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
    input.emissive = in.inst_emissive.xyz;
    input.front_facing = front;
    input.frag_coord = in.clip_position;
    input.time = scene.cascade_params.z;
    input.camera_position = scene.camera_pos.xyz;
    return material_surface(input);
#else
    return evaluate_surface(
        in.tex_coords, in.normal, in.tangent, in.bitangent, in.color, in.inst_albedo, in.inst_pbr, in.inst_emissive.xyz,
    );
#endif
}

#ifdef DETAILS
// One entry point per blend mode, because fog means something different under each. Fog stands
// between the camera and the surface, so what reaches the screen is less of the surface and, where
// the blend lets a colour through, some of the fog. An alpha or opaque draw fades to the fog's colour.
// An additive one only fades out: adding the fog's colour would light the fog up where it stands. A
// multiplied one fades to white, which multiplies by one. A premultiplied one does both by its alpha.
const FOG_MIX: u32 = 0u;
const FOG_ADDITIVE: u32 = 1u;
const FOG_MULTIPLY: u32 = 2u;
const FOG_PREMULTIPLIED: u32 = 3u;

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, FOG_MIX);
}

@fragment
fn fs_additive(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, FOG_ADDITIVE);
}

@fragment
fn fs_multiply(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, FOG_MULTIPLY);
}

@fragment
fn fs_premultiplied(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front, FOG_PREMULTIPLIED);
}

fn shade(in: VertexOutput, front: bool, fog_kind: u32) -> vec4<f32> {
#else
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return shade(in);
}

fn shade(in: VertexOutput) -> vec4<f32> {
#endif
#ifdef SUN_SHADOWS
#ifndef SUN_SHADOW_PER_VERTEX
    // How the surface runs across neighbouring pixels, for the soft shadows (`receiver_axes`) —
    // taken first, before any pixel of the quad discards.
    let pos_dx = dpdx(in.world_position);
    let pos_dy = dpdy(in.world_position);
#endif
#endif
#ifdef DETAILS
    let surface = fragment_surface(in, front);
#else
    // The plain tier draws front faces only. A game's surface is never built on it: only its own code
    // knows whether it cuts (`SurfaceMaterial`).
    let surface = fragment_surface(in, true);
#endif
#ifdef WIREFRAME
    // `WireframeConfig::color`: one flat colour for every line, readable on any material. Alpha 0
    // means none was asked for, and the lines keep the lit colour below.
    if (wire_color.a > 0.0) {
        return vec4<f32>(wire_color.rgb, 1.0);
    }
    if ((scene.wireframe_color >> 24u) != 0u) {
        return unpack4x8unorm(scene.wireframe_color);
    }
#endif
    
#ifdef DETAILS
    let final_alpha = surface.alpha;
    // **Two thresholds, and they are different questions.** The 0.01 is a floor: a fully
    // transparent texel has nothing to contribute and shading it is waste. The material's own
    // `alpha_cutoff` is a cut-out — foliage on a quad, a pierced fence — and it says where the
    // surface simply is not. The forward path read only the floor until 2026-08-22, so a cut-out
    // material shaded here faded instead of cutting, while the same material through the G-buffer
    // or baked-lit path cut correctly. Found while building `demo/src/bin/transparency_3d.rs`.
#ifndef TRANSMISSION
    if (final_alpha < 0.01) {
        discard;
    }
#endif
    if (!instance_keeps(in.inst_cutoff, final_alpha, in.clip_position.xy)) {
        discard;
    }
    if (surface.alpha_cutoff > 0.0 && final_alpha < surface.alpha_cutoff) {
        discard;
    }

    // A back face, which a double-sided or transparent draw shows, is lit as what it is: the normal
    // points at the viewer. The G-buffer does the same.
    let N = select(-surface.N, surface.N, front);
#else
    let N = surface.N;
#endif
    let base_color = surface.base_color;
    let metallic = surface.metallic;
#ifdef TRANSMISSION
    // Light that goes through does not also scatter: the diffuse gets what is left. A metal lets
    // nothing through.
    let glass = in.inst_glass.x * (1.0 - metallic);
    let albedo = surface.albedo * (1.0 - glass);
#else
    let albedo = surface.albedo;
#endif

    // Editor debug views, matching `deferred_lighting.wgsl`'s numbering exactly — the studio's
    // toolbar sets one uniform and must not mean two different things depending on which path
    // happens to run.
    //
    // These modes were reachable ONLY from the deferred pass, and the studio renders forward: the
    // toolbar wrote `shading_mode` every frame into a uniform this shader never read, so the
    // Normals and Albedo chips produced output byte-identical to Lit. Measured before this, at
    // 128x128: 0 of 65536 bytes differed for either.
    //
    // Placed here, before the lighting work, because that work is exactly what these views exist
    // to bypass. `shading_mode` is 0 for every non-studio caller, so the normal path pays one
    // comparison. A phone's plain surface does not even pay that: batching sends everything to the
    // `DETAILS` tier while a debug view is on.
#ifdef DETAILS
    if (scene.shading_mode == 1u) {
        return vec4<f32>(N * 0.5 + vec3<f32>(0.5), 1.0);
    }
    if (scene.shading_mode == 2u) {
        return vec4<f32>(base_color, 1.0);
    }
#endif

    // --- The sun's shadow: `sun_shadow_visibility` in `common.wgsl`, as the deferred pass takes it —
    // softened by the sun's size, faded out over the end of the covered range, pushed off the surface by
    // `ShadowSettings::normal_offset`. This path filtered 3×3 whatever the sun's size, ended its shadows
    // in a line at the covered range's edge and ignored the normal offset's multiplier, until
    // 2026-11-11. (Contact shadows stay the deferred pass's: they march the G-buffer, which a forward
    // draw has none of.)
    var shadow_visibility = 1.0;
#ifdef SUN_SHADOWS
#ifdef SUN_SHADOW_PER_VERTEX
    // The forward-only path's: one cascade, a coordinate from the vertices, one tap (`sun_shadow_tap`).
    let view_depth = dot(in.world_position - scene.camera_pos.xyz, scene.camera_forward.xyz);
    let shadow_fade = shadow_distance_fade(view_depth, scene.cascade_splits.w);
    if (scene.sun_direction.w > 0.5 && shadow_fade > 0.0 && in.inst_emissive.w >= 0.0) {
        var coord = in.sun_shadow;
#ifdef DETAILS
        // The back of a double-sided surface, which only this tier draws, is pushed off the other way:
        // the front's push there shadowed the back of a panel the sun shone straight on (84 against the
        // front's 194, 2026-11-12).
        if (!front) {
            let lean = sun_shadow_lean(scene.light_view_proj[0], scene.cascade_params.y, scene.shadow_bias.y, normalize(in.normal));
            coord -= vec4<f32>(2.0 * lean, 0.0);
        }
#endif
        shadow_visibility = mix(
            1.0,
            sun_shadow_tap(t_shadow, s_shadow, coord, N, normalize(-scene.sun_direction.xyz), scene.shadow_bias.x),
            shadow_fade,
        );
    }
#else
    // How the surface runs across neighbouring pixels — its geometry's plane, which the soft shadows
    // filter along (`receiver_axes`).
    let surface_axes = receiver_axes(pos_dx, pos_dy, N);
    let view_depth = dot(in.world_position - scene.camera_pos.xyz, scene.camera_forward.xyz);
    let shadow_fade = shadow_distance_fade(view_depth, scene.cascade_splits.w);
    // `inst_emissive.w < 0` is `MeshRenderer::receives_shadows == false`.
    if (scene.sun_direction.w > 0.5 && shadow_fade > 0.0 && in.inst_emissive.w >= 0.0) {
        let ci = select_cascade(view_depth);
        shadow_visibility = mix(
            1.0,
            sun_shadow_visibility(
                t_shadow, s_shadow, scene.light_view_proj[ci], i32(ci), scene.cascade_params.y, in.world_position, N,
                surface_axes, normalize(-scene.sun_direction.xyz), scene.shadow_bias.xy, scene.camera_forward.w,
            ),
            shadow_fade,
        );
    }
#endif
#endif

    // An unlit or sky flag only reaches this shader through the wireframe pipeline, which draws every
    // kind of surface with it. The plain tier's pipeline draws `MaterialType::Pbr` alone.
#ifdef DETAILS
    // Eger bu obje 'unlit' ise isiklari es gec
    if (in.inst_pbr.z > 1.5) {
        let view_dir = normalize(in.world_position - scene.camera_pos.xyz);
        let sky_y = view_dir.y;
        
        let sky_color = vec3<f32>(0.08, 0.28, 0.58);
        let horizon_color = vec3<f32>(0.65, 0.75, 0.85);
        let ground_color = vec3<f32>(0.15, 0.15, 0.18);

        var final_bg: vec3<f32>;
        if (sky_y > 0.0) {
            final_bg = mix(horizon_color, sky_color, sky_y);
        } else {
            final_bg = mix(horizon_color, ground_color, -sky_y);
        }
        return vec4<f32>(final_bg, 1.0);
    } else if (in.inst_pbr.z > 0.5) {
        return vec4<f32>(base_color, final_alpha);
    }
#endif

    let min_roughness = surface.roughness;
#ifdef LOBES
    // Anisotropy, clear coat and subsurface are this variant's alone: on a phone their branches cost
    // as much as if they were always taken (20 fps against 43 on an Adreno 618 with a cube and a
    // floor), so on the forward-only path only a material that uses them pays for them. See
    // `SceneState::lit_pipeline`.
    let lobes = surface_lobes(in.inst_pbr);
    let anisotropy = lobes.x;
    let clear_coat = lobes.y;
    let coat_roughness = max(in.inst_coat_roughness, 0.08);
    let subsurface = lobes.z;
    let T = normalize(in.tangent);
    let B = normalize(in.bitangent);
#endif
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
#ifdef DETAILS
    let f0 = mix(min(vec3<f32>(dielectric_f0(in.inst_emissive.w)) * in.inst_tint, vec3<f32>(1.0)), surface.albedo, metallic);
#else
    let f0 = mix(vec3<f32>(0.04), surface.albedo, metallic);
#endif
    let NdV = max(dot(N, view_dir), 0.001);
    
    // The deferred shader's environment terms, so a surface gets the same light on either path.
    let ambient_base = vec3<f32>(0.02, 0.02, 0.025);
    var irradiance = get_procedural_environment(N, 1.0);
#ifdef PROBES
    irradiance = environment_irradiance(N, irradiance);
#endif
    var ambient = albedo * (irradiance + ambient_base) * (1.0 - metallic);
#ifdef LOBES
    if (subsurface > 0.0) {
        ambient = subsurface_ambient(ambient, subsurface);
    }
#endif

    var R = reflect(-view_dir, N);
#ifdef LOBES
    if (anisotropy > 0.0) {
        R = anisotropic_reflection(R, N, T, anisotropy, min_roughness);
    }
#endif
    // With reflection probes in the scene the environment is theirs (native: group 5, the irradiance
    // volume's, decided per pixel by `probe_environment`; forward-only: group 0, decided when the
    // pipelines are built, so a background probe stands in for the sky without the sky being sampled).
#ifdef PROBE_BACKGROUND
    var env_color = probe_background(R, min_roughness);
#else
    var env_color = get_procedural_environment(normalize(mix(R, N, min_roughness)), min_roughness);
#endif
#ifdef PROBES
    env_color = probe_environment(R, in.world_position, min_roughness, env_color);
#endif
#ifdef PROBE_BOXES
    env_color = probe_boxes(R, in.world_position, min_roughness, env_color);
#endif
    let env_brdf = approximate_env_brdf(NdV, min_roughness);
    var specular_ibl = env_color * (f0 * env_brdf.x + env_brdf.y);

#ifdef LOBES
    if (clear_coat > 0.0) {
        let coat_atten_env = coat_env_attenuation(NdV, clear_coat);
        ambient *= coat_atten_env;
        specular_ibl *= coat_atten_env;
#ifdef PROBE_BACKGROUND
        var coat_env = probe_background(reflect(-view_dir, N), coat_roughness);
#else
        var coat_env = get_procedural_environment(normalize(mix(reflect(-view_dir, N), N, coat_roughness)), coat_roughness);
#endif
#ifdef PROBES
        coat_env = probe_environment(reflect(-view_dir, N), in.world_position, coat_roughness, coat_env);
#endif
#ifdef PROBE_BOXES
        coat_env = probe_boxes(reflect(-view_dir, N), in.world_position, coat_roughness, coat_env);
#endif
        specular_ibl += coat_env_specular(coat_env, NdV, coat_roughness) * clear_coat;
    }
#endif

    var total_lighting = vec3<f32>(0.0);

    if (scene.sun_direction.w > 0.5) { 
        let L = normalize(-scene.sun_direction.xyz);
        var sun_light = compute_direct_lighting(
            N, view_dir, L, albedo, min_roughness, metallic, f0,
            scene.sun_color.rgb, scene.sun_color.w, shadow_visibility
        );
#ifdef LOBES
        if (anisotropy > 0.0) {
            sun_light = compute_direct_lighting_anisotropic(
                N, view_dir, L, T, B, albedo, min_roughness, metallic, anisotropy, f0,
                scene.sun_color.rgb, scene.sun_color.w, shadow_visibility
            );
        }
        if (clear_coat > 0.0) {
            sun_light = layer_clear_coat(sun_light, N, view_dir, L, scene.sun_color.rgb, scene.sun_color.w, shadow_visibility, clear_coat, coat_roughness);
        }
        if (subsurface > 0.0) {
            sun_light += subsurface_light(N, view_dir, L, metallic, subsurface) * scene.sun_color.rgb * scene.sun_color.w * shadow_visibility;
        }
#endif
        total_lighting += sun_light;
    }

#ifdef LOCAL_LIGHTS
    // Clustered: only this fragment's cluster's lights. `i` is still the index into `scene.lights`
    // so the body is unchanged from when it looped over every light.
    let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, in.world_position);
    let cluster_range = cluster_table[cluster];
    for (var ci = 0u; ci < cluster_range.y; ci++) {
        let i = cluster_light_indices[cluster_range.x + ci];
        let light = scene.lights[i];
        let light_type = u32(light.params.y);
        let intensity = light.position.w;
        var L: vec3<f32>;
        var attenuation: f32 = 1.0;
        var window: f32 = 1.0;
        var cookie = vec3<f32>(1.0);
        if (light_type == 2u) {
            L = normalize(-light.direction.xyz);
            attenuation = 1.0;
        } else {
            let to_light = light.position.xyz - in.world_position;
            L = normalize(to_light);
            // The deferred pass's falloff, from the one function: see `light_falloff`.
            let falloff = light_falloff(length(to_light), light.color.a);
            attenuation = falloff.x;
            window = falloff.y;
            if (light_type == 1u) {
                attenuation *= spot_cone(light, -L);
#ifdef LOCAL_LIGHT_MAPS
                if (light.params.w > 0.0) {
                    cookie = textureSampleLevel(
                        t_cookie, s_cookie, spot_cookie_uv(light, in.world_position), i32(light.params.w) - 1, 0.0
                    ).rgb;
                }
#endif
            }
#ifdef LOCAL_LIGHT_MAPS
            // The light's shadow, as the deferred pass takes it: see `local_shadow_visibility` — or, on the
            // forward-only path, with one tap (`local_shadow_tap`).
            // `inst_emissive.w < 0` is `MeshRenderer::receives_shadows == false`.
            let shadow_layer = local_shadow_layer(scene.light_shadows[i / 4u][i % 4u], in.world_position - light.position.xyz);
            if (in.inst_emissive.w >= 0.0 && shadow_layer >= 0 && max(attenuation, window) > 0.0) {
#ifdef LOCAL_SHADOW_TAP
                let lit = local_shadow_tap(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[shadow_layer].view_proj,
                    scene.local_shadow_layers[shadow_layer].params, shadow_layer, in.world_position, N, to_light,
                    scene.shadow_bias.xy,
                );
#else
                let lit = local_shadow_visibility(
                    t_local_shadow, s_shadow, scene.local_shadow_layers[shadow_layer], shadow_layer,
                    in.world_position, N, surface_axes, to_light, scene.shadow_bias.xy, light_source_radius(light),
                );
#endif
                attenuation *= lit;
                window *= lit;
            }
#endif
        }
        let light_rgb = light.color.rgb * cookie;
        var light_color_contrib = vec3<f32>(0.0);
#ifdef RECT_LIGHTS
        if (light_type == 3u) {
            // A rectangle light (`compute_rect_lighting`; one-sided, so it gates itself).
            light_color_contrib = compute_rect_lighting(
                N, view_dir, light.position.xyz - in.world_position, light.direction, light.params.z,
                light.params.w, albedo, min_roughness, metallic, f0, light_rgb, intensity, window
            );
        } else
#endif
#ifdef LOBES
        if (anisotropy > 0.0) {
            light_color_contrib = compute_direct_lighting_anisotropic(
                N, view_dir, L, T, B, albedo, min_roughness, metallic, anisotropy, f0,
                light_rgb, intensity, attenuation
            );
        } else
#endif
#ifdef SIZED_LIGHTS
        if (light_type == 0u && light.params.w > 0.0) {
            // A tube light (`compute_tube_lighting`).
            light_color_contrib = compute_tube_lighting(
                N, view_dir, light.position.xyz - in.world_position, light.direction.xyz, light.params.w,
                light.params.z, albedo, min_roughness, metallic, f0, light_rgb, intensity, attenuation
            );
        } else if (light_type != 2u && light.params.z > 0.0) {
            // A sphere-sourced point or spot light (`compute_sphere_lighting`).
            light_color_contrib = compute_sphere_lighting(
                N, view_dir, light.position.xyz - in.world_position, light.params.z, albedo,
                min_roughness, metallic, f0, light_rgb, intensity, attenuation
            );
        } else
#endif
        {
            light_color_contrib = compute_direct_lighting(
                N, view_dir, L, albedo, min_roughness, metallic, f0,
                light_rgb, intensity, attenuation
            );
        }
#ifdef LOBES
        if (clear_coat > 0.0) {
            light_color_contrib = layer_clear_coat(light_color_contrib, N, view_dir, L, light_rgb, intensity, attenuation, clear_coat, coat_roughness);
        }
#endif
        total_lighting += light_color_contrib;
    }
#endif
    
    var indirect = vec3<f32>(0.0);
#ifdef IRRADIANCE
    // The baked probe grid, as the deferred shader adds it.
    indirect = gi_sample(in.world_position, N) * albedo;
#endif
    var final_color = ambient + indirect + total_lighting + specular_ibl + surface.emissive;

    // We render to an Rgba16Float HDR buffer. Tone mapping is handled in post_process.wgsl.
    
#ifdef TRANSMISSION
    let alpha = 1.0;
#else
#ifdef DETAILS
    let alpha = final_alpha;
#endif
#endif
    let fog = height_fog(in.world_position, scene.camera_pos.xyz, scene.fog, scene.fog_shape);
#ifdef DETAILS
    if (fog_kind == FOG_ADDITIVE) {
        final_color *= 1.0 - fog.a;
    } else if (fog_kind == FOG_MULTIPLY) {
        final_color = mix(final_color, vec3<f32>(1.0), fog.a);
    } else if (fog_kind == FOG_PREMULTIPLIED) {
        final_color = final_color * (1.0 - fog.a) + fog.rgb * fog.a * alpha;
    } else {
        final_color = mix(final_color, fog.rgb, fog.a);
    }
#else
    final_color = mix(final_color, fog.rgb, fog.a);
#endif

#ifdef TRANSMISSION
    // After the fog: the frame behind has its own up to that surface.
    final_color += transmitted_light(in.world_position, N, view_dir, NdV, surface.albedo, f0, surface.roughness, dielectric_f0(in.inst_emissive.w), in.inst_glass.y) * glass * volume_transmittance(in.inst_volume);
#endif

#ifdef DETAILS
    return vec4<f32>(final_color, alpha);
#else
    return vec4<f32>(final_color, 1.0);
#endif
}

#ifdef TRANSMISSION
// What comes through: the frame behind, where the bent ray leaves the surface, as blurred as the
// surface is rough, tinted by it, less what the surface reflected on the way in.
// How much of the light behind comes through the glass's thickness, a channel at a time
// (`Material::volume_transmittance`), from its 8 bits a channel.
fn volume_transmittance(packed: u32) -> vec3<f32> {
    return vec3<f32>(f32((packed >> 16u) & 255u), f32((packed >> 8u) & 255u), f32(packed & 255u)) / 255.0;
}

fn transmitted_light(
    world_position: vec3<f32>,
    N: vec3<f32>,
    view_dir: vec3<f32>,
    NdV: f32,
    tint: vec3<f32>,
    f0: vec3<f32>,
    roughness: f32,
    dielectric: f32,
    thickness: f32,
) -> vec3<f32> {
    // F0 back to the index of refraction it came from.
    let root = sqrt(min(dielectric, 0.99));
    let ior = (1.0 + root) / (1.0 - root);
    // `refract` gives zero for total internal reflection, which leaves the ray where it entered.
    let exit = world_position + refract(-view_dir, N, 1.0 / ior) * thickness;
    let clip = scene.view_proj * vec4<f32>(exit, 1.0);
    let ndc = clip.xy / max(clip.w, 0.0001);
    let uv = clamp(vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5), vec2<f32>(0.0), vec2<f32>(1.0));
    // `surface.roughness` is floored at 0.05 for the highlight's sake; a polished pane reads level 0.
    // The blur grows with roughness squared, as a share of the picture's height.
    let blur = max(roughness - 0.05, 0.0);
    let lod = log2(max(blur * blur * f32(textureDimensions(t_behind).y), 1.0));
    let behind = textureSampleLevel(t_behind, s_behind, uv, lod).rgb;
    let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - NdV, 5.0);
    return behind * tint * (vec3<f32>(1.0) - fresnel);
}
#endif
