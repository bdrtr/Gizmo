#define_import_path gizmo::material

// The engine's own material: its seven-entry bind group (group 1), as `AssetManager::material` fills
// it, and the instance's values, turned into a `gizmo::surface::Surface`. The G-buffer and the
// forward shader both come here, so a map or a material knob cannot work on one path and not on the
// other. `shader.wgsl` read only the base colour for as long as both existed, which is how normal
// maps never reached a browser. A game's own surface can start from it with `standard_surface`.

#import gizmo::surface::{Surface, SurfaceInput}

// Per-material scalar params (group 1, binding 6). Mirrors `gpu_types::MaterialParams`.
struct MaterialParams {
    // xyz = emissive factor (linear), w = normal-map scale.
    emissive_and_normal_scale: vec4<f32>,
    // x = occlusion (AO) strength; y = UV rotation (radians); zw = UV offset.
    occlusion_uv_rot_offset: vec4<f32>,
    // xy = UV scale; z = alpha cutoff (glTF Mask; 0 = no cutout); w reserved.
    uv_scale: vec4<f32>,
};

@group(1) @binding(0) var t_diffuse:  texture_2d<f32>;
@group(1) @binding(1) var s_diffuse:  sampler;
@group(1) @binding(2) var t_normal:   texture_2d<f32>;
@group(1) @binding(3) var t_mr:       texture_2d<f32>;  // g=roughness, b=metallic
@group(1) @binding(4) var t_emissive: texture_2d<f32>;
@group(1) @binding(5) var t_ao:       texture_2d<f32>;  // r=occlusion
@group(1) @binding(6) var<uniform> material: MaterialParams;

// KHR_texture_transform: scale, then rotate, then translate the UV (matrix = translation * rotation
// * scale, per the extension's reference). For the VERTEX stage: it is linear, so it interpolates
// exactly, and a sine and a cosine per fragment took a phone from 69 fps to 44 together with the
// tangent basis below.
fn surface_uv(uv: vec2<f32>) -> vec2<f32> {
    let offset = material.occlusion_uv_rot_offset.zw;
    let rot = material.occlusion_uv_rot_offset.y;
    let s = uv * material.uv_scale.xy;
    let c = cos(rot);
    let sn = sin(rot);
    let rotated = vec2<f32>(c * s.x - sn * s.y, sn * s.x + c * s.y);
    return rotated + offset;
}

// The material at a fragment. `uv` is `surface_uv`'s and `N`, `T`, `B` are `tangent_frame`'s,
// interpolated.
fn evaluate_surface(
    uv: vec2<f32>,
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
    vertex_color: vec3<f32>,
    inst_albedo: vec4<f32>,
    inst_pbr: vec4<f32>,
    inst_emissive: vec3<f32>,
) -> Surface {
    let tex_color = textureSample(t_diffuse, s_diffuse, uv);

    // ── Normal map (tangent-space → world) ────────────────────────────────
    // Flat-default map (0.5,0.5,1.0) → (0,0,1) → geometric normal unchanged.
    let normal_scale = material.emissive_and_normal_scale.w;
    var ts_normal = textureSample(t_normal, s_diffuse, uv).xyz * 2.0 - 1.0;
    ts_normal = vec3<f32>(ts_normal.xy * normal_scale, max(ts_normal.z, 1e-4));
    let world_normal = T * ts_normal.x + B * ts_normal.y + N * ts_normal.z;

    // ── Metallic-roughness map (glTF: factor × texture) ───────────────────
    // White-default map → sampled g=b=1 → scalar factors preserved.
    let mr = textureSample(t_mr, s_diffuse, uv);

    // ── Ambient occlusion (glTF: 1 + strength·(ao-1)) ─────────────────────
    // White-default map (r=1) → ao=1 → no darkening. The G-buffer has no channel of its own for AO,
    // so it goes into the albedo, here for both paths.
    let ao_strength = material.occlusion_uv_rot_offset.x;
    let ao = 1.0 + ao_strength * (textureSample(t_ao, s_diffuse, uv).r - 1.0);

    var surface: Surface;
    surface.base_color = inst_albedo.rgb * tex_color.rgb;
    // Vertex colour, taken at face value — `baked_lit.wgsl`'s rule and its reason: a source with no
    // colours is already opaque white (`gpu_types::Vertex::default`, and import normalises), so
    // rewriting a near-black colour to white only turns a surface painted black white.
    surface.albedo = surface.base_color * vertex_color * ao;
    surface.alpha = inst_albedo.a * tex_color.a;
    surface.alpha_cutoff = material.uv_scale.z;
    // Not `normalize`: a degenerate sum comes out dark, not NaN.
    surface.N = world_normal / max(length(world_normal), 1e-6);
    surface.roughness = clamp(inst_pbr.x * mr.g, 0.05, 1.0);
    surface.metallic = clamp(inst_pbr.y * mr.b, 0.0, 1.0);
    // The material's own (`Material::emissive`, per instance) plus glTF's emissiveFactor × its texture
    // (white-default map × zero factor → nothing).
    surface.emissive = inst_emissive
        + material.emissive_and_normal_scale.xyz * textureSample(t_emissive, s_diffuse, uv).rgb;
    return surface;
}

// The engine's material at what a game's `material_surface` is handed: what `MaterialType::Pbr`
// would have drawn there, for a surface that changes some of it. The UV transform is worked out here,
// per fragment, since the vertex stage of a custom surface cannot know what group 1 means to it.
fn standard_surface(input: SurfaceInput) -> Surface {
    return evaluate_surface(
        surface_uv(input.uv),
        input.N,
        input.T,
        input.B,
        input.vertex_color,
        input.albedo,
        vec4<f32>(input.roughness, input.metallic, 0.0, 0.0),
        input.emissive,
    );
}
