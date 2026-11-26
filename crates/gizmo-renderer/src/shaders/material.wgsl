#define_import_path gizmo::material

// The engine's own material: its bind group (group 1), as `AssetManager::material` fills
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
    // xy = UV scale; z = alpha cutoff (glTF Mask; 0 = no cutout); w = lightmap strength (0 = none).
    uv_scale: vec4<f32>,
};

@group(1) @binding(0) var t_diffuse:  texture_2d<f32>;
@group(1) @binding(1) var s_diffuse:  sampler;
@group(1) @binding(2) var t_normal:   texture_2d<f32>;
// r=occlusion, g=roughness, b=metallic: glTF's ORM. The group has no occlusion texture of its own —
// WebGPU's 16 textures a stage had one left on the forward-only path — and a material's occlusion map
// is drawn into this one's red (`material_pack.rs`).
@group(1) @binding(3) var t_mr:       texture_2d<f32>;
@group(1) @binding(4) var t_emissive: texture_2d<f32>;
// The surface's own textures, one per layer (`MaterialMaps::surface`): what a custom surface or a
// shader graph samples beyond the engine's maps. Colours, read from sRGB as a base colour is.
@group(1) @binding(5) var t_surface:  texture_2d_array<f32>;
@group(1) @binding(6) var<uniform> material: MaterialParams;
@group(1) @binding(7) var t_lightmap: texture_2d<f32>; // baked light, linear, at the second UV set
// The texture pool's array (`texture_pool`): a pooled material's base colour is one of its layers.
@group(1) @binding(8) var t_pool: texture_2d_array<f32>;

// The base colour at `uv`: the material's own texture, or, for `layer` above zero, layer `layer - 1`
// of its texture pool's array (`InstanceRaw::texture`). Both are sampled and one chosen: a branch on
// a per-instance value would put the sample in non-uniform control flow, where implicit derivatives
// are not allowed, and the one not wanted is a 1×1 placeholder — the material's own slot holds one
// in a pooled material, the pool's in any other.
fn base_colour(uv: vec2<f32>, layer: u32) -> vec4<f32> {
    let own = textureSample(t_diffuse, s_diffuse, uv);
    let pooled = textureSample(t_pool, s_diffuse, uv, i32(max(layer, 1u) - 1u));
    return select(own, pooled, layer > 0u);
}

// The material's baked light where its lightmap puts `lightmap_uv` (the vertex's second UV set,
// untransformed): rgb the light, times the material's strength, and `w` 1 when the material has a
// lightmap at all. With one, that light stands in for the ambient and the irradiance volume — both
// paths do the same with it (`gbuffer.wgsl` and `deferred_lighting.wgsl`, `shader.wgsl`).
fn baked_light(lightmap_uv: vec2<f32>) -> vec4<f32> {
    let strength = material.uv_scale.w;
    if (strength <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(textureSample(t_lightmap, s_diffuse, lightmap_uv).rgb * strength, 1.0);
}

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
    // `InstanceRaw::texture.x`: where the base colour is (`base_colour`).
    base_layer: u32,
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
    vertex_color: vec3<f32>,
    inst_albedo: vec4<f32>,
    inst_pbr: vec4<f32>,
    inst_emissive: vec3<f32>,
) -> Surface {
    let tex_color = base_colour(uv, base_layer);

    // ── Normal map (tangent-space → world) ────────────────────────────────
    // Flat-default map (0.5,0.5,1.0) → (0,0,1) → geometric normal unchanged.
    let normal_scale = material.emissive_and_normal_scale.w;
    var ts_normal = textureSample(t_normal, s_diffuse, uv).xyz * 2.0 - 1.0;
    ts_normal = vec3<f32>(ts_normal.xy * normal_scale, max(ts_normal.z, 1e-4));
    let world_normal = T * ts_normal.x + B * ts_normal.y + N * ts_normal.z;

    // ── Metallic-roughness map (glTF: factor × texture) ───────────────────
    // White-default map → sampled g=b=1 → scalar factors preserved.
    let mr = textureSample(t_mr, s_diffuse, uv);

    // ── Ambient occlusion (glTF: 1 + strength·(ao-1)), the ORM map's red ──
    // White-default map (r=1) → ao=1 → no darkening. The G-buffer has no channel of its own for AO,
    // so it goes into the albedo, here for both paths.
    let ao_strength = material.occlusion_uv_rot_offset.x;
    let ao = 1.0 + ao_strength * (mr.r - 1.0);

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
        input.base_layer,
        input.N,
        input.T,
        input.B,
        input.vertex_color,
        input.albedo,
        vec4<f32>(input.roughness, input.metallic, 0.0, 0.0),
        input.emissive,
    );
}
