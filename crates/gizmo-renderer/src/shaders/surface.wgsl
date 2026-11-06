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
    roughness: f32,
    metallic: f32,
    // Added unlit, in linear HDR.
    emissive: vec3<f32>,
};

struct TangentFrame {
    N: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
};

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

// The tangent basis at a vertex, in world space. For the VERTEX stage, for the same reason, and the
// fragment stage interpolates it without renormalising: only the normal it finally builds has to be
// unit length, and its branches and three square roots per fragment are what this saves.
fn tangent_frame(world_normal: vec3<f32>, world_tangent: vec3<f32>, handedness: f32) -> TangentFrame {
    var raw_normal = world_normal;
    if (length(raw_normal) < 0.001) { raw_normal = vec3<f32>(0.0, 1.0, 0.0); }
    let N = normalize(raw_normal);

    var raw_tangent = world_tangent;
    // Fall back to a synthesised tangent when the supplied one is missing OR nearly
    // PARALLEL to the normal. Gram-Schmidt of parallel vectors is normalize(0) = NaN/garbage,
    // which produced a per-pixel-varying TBN → normal-map streaks ONLY on the ±X cube faces,
    // whose default vertex tangent [1,0,0] coincides with the face normal. (Front/back/top/
    // bottom faces have a non-parallel tangent, so they were always clean.)
    let tan_len = length(raw_tangent);
    if (tan_len < 0.001 || abs(dot(raw_tangent / max(tan_len, 1e-8), N)) > 0.999) {
        if (abs(N.x) > 0.9) {
            raw_tangent = cross(vec3<f32>(0.0, 1.0, 0.0), N);
        } else {
            raw_tangent = cross(vec3<f32>(1.0, 0.0, 0.0), N);
        }
    }
    // Gram-Schmidt re-orthogonalise the tangent against the normal.
    let T = normalize(raw_tangent - N * dot(N, raw_tangent));
    var frame: TangentFrame;
    frame.N = N;
    frame.T = T;
    // `select`, not `sign`: `sign(0.0)` is 0, which would zero the bitangent of a mesh whose tangent w is 0.
    frame.B = cross(N, T) * select(-1.0, 1.0, handedness >= 0.0);
    return frame;
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

// Anisotropy, clear coat and subsurface, in that order, from the instance's packed slot. Two digits
// per field — see `pack_pbr_params`. Three needed nine digits, which an f32 cannot hold as an exact
// integer, and the low field carried into its neighbour above 2^24.
fn surface_lobes(inst_pbr: vec4<f32>) -> vec3<f32> {
    let packed = inst_pbr.w - floor(inst_pbr.w / 10000.0) * 10000.0;
    return vec3<f32>(
        (packed - floor(packed / 100.0) * 100.0) / 100.0,
        floor(packed / 100.0) / 100.0,
        floor(inst_pbr.w / 10000.0) / 100.0,
    );
}
