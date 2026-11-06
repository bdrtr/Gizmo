#define_import_path gizmo::surface

// What a material is at one fragment: its seven-entry bind group (group 1) and the instance's values,
// turned into the numbers lighting needs. The G-buffer and the forward shader both come here, so a
// map or a material knob cannot work on one path and not on the other. `shader.wgsl` read only the
// base colour for as long as both existed, which is how normal maps never reached a browser.

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

struct Surface {
    // Instance colour × texture: what an unlit or debug view shows.
    base_color: vec3<f32>,
    // `base_color` × vertex colour × AO: what light lands on.
    albedo: vec3<f32>,
    alpha: f32,
    // glTF's Mask threshold from the material's params; 0 = no cut-out.
    alpha_cutoff: f32,
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
    roughness: f32,
    metallic: f32,
    // Added unlit, in linear HDR.
    emissive: vec3<f32>,
    anisotropy: f32,
    clear_coat: f32,
    subsurface: f32,
};

// KHR_texture_transform: scale, then rotate, then translate the UV
// (matrix = translation * rotation * scale, per the extension's reference).
fn apply_uv_transform(uv: vec2<f32>, offset: vec2<f32>, rot: f32, scale: vec2<f32>) -> vec2<f32> {
    let s = uv * scale;
    let c = cos(rot);
    let sn = sin(rot);
    let rotated = vec2<f32>(c * s.x - sn * s.y, sn * s.x + c * s.y);
    return rotated + offset;
}

fn evaluate_surface(
    tex_coords: vec2<f32>,
    normal: vec3<f32>,
    world_tangent: vec4<f32>,
    vertex_color: vec3<f32>,
    inst_albedo: vec4<f32>,
    inst_pbr: vec4<f32>,
    inst_emissive: vec3<f32>,
) -> Surface {
    // KHR_texture_transform: transform the incoming UV once, reuse for every map.
    let uv = apply_uv_transform(
        tex_coords,
        material.occlusion_uv_rot_offset.zw,
        material.occlusion_uv_rot_offset.y,
        material.uv_scale.xy,
    );
    let tex_color = textureSample(t_diffuse, s_diffuse, uv);

    // ── Geometric tangent basis (TBN) ─────────────────────────────────────
    var raw_normal = normal;
    if (length(raw_normal) < 0.001) { raw_normal = vec3<f32>(0.0, 1.0, 0.0); }
    let geo_N = normalize(raw_normal);

    var raw_tangent = world_tangent.xyz;
    // Fall back to a synthesised tangent when the supplied one is missing OR nearly
    // PARALLEL to the normal. Gram-Schmidt of parallel vectors is normalize(0) = NaN/garbage,
    // which produced a per-pixel-varying TBN → normal-map streaks ONLY on the ±X cube faces,
    // whose default vertex tangent [1,0,0] coincides with the face normal. (Front/back/top/
    // bottom faces have a non-parallel tangent, so they were always clean.)
    let tan_len = length(raw_tangent);
    if (tan_len < 0.001 || abs(dot(raw_tangent / max(tan_len, 1e-8), geo_N)) > 0.999) {
        if (abs(geo_N.x) > 0.9) {
            raw_tangent = cross(vec3<f32>(0.0, 1.0, 0.0), geo_N);
        } else {
            raw_tangent = cross(vec3<f32>(1.0, 0.0, 0.0), geo_N);
        }
    }
    // Gram-Schmidt re-orthogonalise the tangent against the (interpolated) normal.
    let T = normalize(raw_tangent - geo_N * dot(geo_N, raw_tangent));
    let B = cross(geo_N, T) * sign(world_tangent.w);

    // ── Normal map (tangent-space → world) ────────────────────────────────
    // Flat-default map (0.5,0.5,1.0) → (0,0,1) → geometric normal unchanged.
    let normal_scale = material.emissive_and_normal_scale.w;
    var ts_normal = textureSample(t_normal, s_diffuse, uv).xyz * 2.0 - 1.0;
    ts_normal = vec3<f32>(ts_normal.xy * normal_scale, max(ts_normal.z, 1e-4));
    ts_normal = normalize(ts_normal);

    // ── Metallic-roughness map (glTF: factor × texture) ───────────────────
    // White-default map → sampled g=b=1 → scalar factors preserved.
    let mr = textureSample(t_mr, s_diffuse, uv);

    // ── Ambient occlusion (glTF: 1 + strength·(ao-1)) ─────────────────────
    // White-default map (r=1) → ao=1 → no darkening. The G-buffer has no channel of its own for AO,
    // so it goes into the albedo, here for both paths.
    let ao_strength = material.occlusion_uv_rot_offset.x;
    let ao = 1.0 + ao_strength * (textureSample(t_ao, s_diffuse, uv).r - 1.0);

    // Anisotropy, clear coat and subsurface. Two digits per field — see `pack_pbr_params`. Three
    // needed nine digits, which an f32 cannot hold as an exact integer, and the low field carried into
    // its neighbour above 2^24.
    let packed = inst_pbr.w - floor(inst_pbr.w / 10000.0) * 10000.0;

    var surface: Surface;
    surface.base_color = inst_albedo.rgb * tex_color.rgb;
    // Vertex colour, taken at face value — `baked_lit.wgsl`'s rule and its reason: a source with no
    // colours is already opaque white (`gpu_types::Vertex::default`, and import normalises), so
    // rewriting a near-black colour to white only turns a surface painted black white.
    surface.albedo = surface.base_color * vertex_color * ao;
    surface.alpha = inst_albedo.a * tex_color.a;
    surface.alpha_cutoff = material.uv_scale.z;
    surface.N = normalize(T * ts_normal.x + B * ts_normal.y + geo_N * ts_normal.z);
    surface.T = T;
    surface.B = B;
    surface.roughness = clamp(inst_pbr.x * mr.g, 0.05, 1.0);
    surface.metallic = clamp(inst_pbr.y * mr.b, 0.0, 1.0);
    // The material's own (`Material::emissive`, per instance) plus glTF's emissiveFactor × its texture
    // (white-default map × zero factor → nothing).
    surface.emissive = inst_emissive
        + material.emissive_and_normal_scale.xyz * textureSample(t_emissive, s_diffuse, uv).rgb;
    surface.subsurface = floor(inst_pbr.w / 10000.0) / 100.0;
    surface.clear_coat = floor(packed / 100.0) / 100.0;
    surface.anisotropy = (packed - floor(packed / 100.0) * 100.0) / 100.0;
    return surface;
}
