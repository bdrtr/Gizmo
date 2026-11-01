#define_import_path gizmo::common

// Shared shader library, composed into other shaders by `load_shader_composed`
// (naga_oil `#import gizmo::common::{...}`). This is the SINGLE source of truth for the
// scene uniform layout and the core BRDF, replacing the hand-copied duplicates that used
// to live in 20+ shaders and silently drift apart. Only put things here that are pure
// (no reference to a module-scope binding like `scene`): structs, constants, and helper
// functions that take everything they need as parameters.

const PI: f32 = 3.1415926535;
const INV_PI: f32 = 0.31830988618; // 1 / PI — Lambert diffuse normalization

// One dynamic light. Encoding (matches gpu_types::LightData on the CPU):
//   position.xyz = world pos,        position.w = intensity
//   color.rgb    = colour,           color.a    = radius
//   direction.xyz= spot/dir axis,    direction.w= inner cutoff cos
//   params.x     = outer cutoff cos, params.y   = light type (0=point,1=spot,2=dir,3=rect)
//   (rect: direction = orientation quaternion, params.zw = half width, half height)
//   (spot: params.w = cookie layer + 1, 0 = no cookie)
//   params.z     = source radius (metres; 0 = a point — see compute_sphere_lighting)
//   params.w     = tube length along direction.xyz (point lights only; see compute_tube_lighting)
struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
};

// Global per-frame scene uniform. MUST stay byte-compatible with
// gpu_types::SceneUniforms on the CPU side.
struct SceneUniforms {
    view_proj:       mat4x4<f32>,
    camera_pos:      vec4<f32>,
    sun_direction:   vec4<f32>,   // xyz = sun dir, w = sun-present flag (1/0)
    sun_color:       vec4<f32>,   // rgb = colour, w = intensity
    lights:          array<LightData, 256>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,   // x=znear, y=1/shadowRes, z=time, w=point-shadow caster idx+1
    num_lights: u32,
    exposure: f32,
    _pre_align_pad: vec2<u32>,
    _align_pad: vec3<u32>,
    environment_blend_t: f32,
    environment_preset: u32,
    point_shadows_enabled: u32,
    // Named without a trailing digit: naga_oil reserves the `_<number>` suffix for naga's
    // WGSL writeback name-mangling and rejects composable-module identifiers that use it.
    // Only the byte layout must match gpu_types::SceneUniforms.environment_preset_2.
    environment_preset_b: u32,
    shading_mode: u32,
    // inverse(view_proj), computed once per frame on the CPU (gpu_types::SceneUniforms).
    // Fullscreen passes that unproject NDC→world read this instead of calling inverse_mat4
    // per fragment. Appended at the tail so all prior field offsets stay byte-compatible.
    inv_view_proj: mat4x4<f32>,
    // Clustered light culling (gizmo-renderer/src/clustered.rs). dims = (x, y, z, 0) tiles/slices;
    // depth = (z_scale, z_bias, 0, 0) for slice = floor(log(view_depth) * z_scale + z_bias).
    cluster_dims:  vec4<u32>,
    cluster_depth: vec4<f32>,
};

// ── Clustered light lookup ───────────────────────────────────────────────────
//
// The cluster a world position belongs to, by the SAME mapping `clustered.rs` assigns lights with:
// screen tile from NDC, depth slice from the log of the view depth. A shader that wants a cluster's
// light list also needs the two storage bindings (group 0, bindings 1 and 2) — they are declared by
// the shaders that light, not here, because a pipeline whose group 0 lacks them must still be able
// to compose this module.
fn gizmo_cluster_index(scene_: SceneUniforms, world_pos: vec3<f32>) -> u32 {
    let dims = scene_.cluster_dims;
    let clip = scene_.view_proj * vec4<f32>(world_pos, 1.0);
    // Behind the camera (w <= 0) has no cluster; clamp into the volume rather than wrap.
    let inv_w = select(1.0 / clip.w, 1.0, abs(clip.w) < 1e-9);
    let uv = clamp(clip.xy * inv_w * 0.5 + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(0.999999));
    let tile = vec2<u32>(uv * vec2<f32>(f32(dims.x), f32(dims.y)));

    let depth = max(dot(world_pos - scene_.camera_pos.xyz, scene_.camera_forward.xyz), 1e-6);
    let raw = log(depth) * scene_.cluster_depth.x + scene_.cluster_depth.y;
    let slice = min(u32(max(floor(raw), 0.0)), dims.z - 1u);

    return min(tile.x, dims.x - 1u)
         + min(tile.y, dims.y - 1u) * dims.x
         + slice * dims.x * dims.y;
}

// ── Core BRDF (Cook-Torrance / GGX) ──────────────────────────────────────────

fn D_GGX(NoH: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = NoH * NoH * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

fn V_SmithJointGGX(NoV: f32, NoL: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let lambdaV = NoL * sqrt(NoV * NoV * (1.0 - a2) + a2);
    let lambdaL = NoV * sqrt(NoL * NoL * (1.0 - a2) + a2);
    return 0.5 / max(lambdaV + lambdaL, 0.0001);
}

fn F_Schlick(VoH: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - VoH, 0.0, 1.0), 5.0);
}

// Direct PBR lighting for one light. Diffuse carries the Lambert 1/PI so it is
// energy-consistent with the 1/PI already inside D_GGX.
fn compute_direct_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let H = normalize(V + L);
    let NoL = max(dot(N, L), 0.0);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);

    if (NoL <= 0.0) {
        return vec3<f32>(0.0);
    }

    let D = D_GGX(NoH, roughness);
    let Vis = V_SmithJointGGX(NoV, NoL, roughness);
    let F = F_Schlick(VoH, f0);

    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - metallic);

    let diffuse = kD * albedo * NoL * INV_PI; // Lambert: albedo / PI
    let specular = D * Vis * F * NoL;

    return (diffuse + specular) * light_color * intensity * atten;
}

// A light with a SPHERICAL source of `source_radius` metres, centred `to_light` away. Diffuse
// uses the centre. Specular uses the representative point (Karis, "Real Shading in Unreal Engine
// 4", 2013): the point on the sphere nearest the reflected ray, so a glossy surface shows a highlight
// as big as the sphere's reflection rather than a pinpoint — with the distribution widened to match
// and renormalised by (α / α')², so a bigger source spreads the same energy rather than adding it.
fn compute_sphere_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    source_radius: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let dist = max(length(to_light), 1e-4);
    let L = to_light / dist;
    let NoL = max(dot(N, L), 0.0);
    let r = reflect(-V, N);
    let center_to_ray = dot(to_light, r) * r - to_light;
    let closest = to_light + center_to_ray * clamp(source_radius / max(length(center_to_ray), 1e-5), 0.0, 1.0);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (NoL <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }
    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_wide = clamp(alpha + source_radius / (2.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_wide, 1e-4)) * (alpha / max(alpha_wide, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let diffuse = kD * albedo * NoL * INV_PI;
    let specular = D * Vis * F * NoLs;
    return (diffuse + specular) * light_color * intensity * atten;
}

// A TUBE light: a segment `tube_length` metres long through `to_light` (the centre, relative to the
// surface) along `axis`, `source_radius` thick (Karis 2013, "tube lights"). Diffuse averages the
// cosine to the two ends. Specular takes the point on the segment nearest the reflected ray, then
// the sphere step around it, and renormalises the lobe once per widened dimension — (α/α'_line) for
// the length, (α/α'_sphere)² for the thickness — so a longer tube spreads its energy rather than
// adding it.
fn compute_tube_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    axis: vec3<f32>,
    tube_length: f32,
    source_radius: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    atten: f32
) -> vec3<f32> {
    let half_axis = axis * (tube_length * 0.5);
    let l0 = to_light - half_axis;
    let l1 = to_light + half_axis;
    let dist = max(length(to_light), 1e-4);
    let NoL = clamp(0.5 * (dot(N, l0) / max(length(l0), 1e-4) + dot(N, l1) / max(length(l1), 1e-4)), 0.0, 1.0);

    let r = reflect(-V, N);
    let ld = l1 - l0;
    let r_ld = dot(r, ld);
    let t = clamp((dot(r, l0) * r_ld - dot(l0, ld)) / max(dot(ld, ld) - r_ld * r_ld, 1e-6), 0.0, 1.0);
    let on_line = l0 + ld * t;
    let center_to_ray = dot(on_line, r) * r - on_line;
    let closest = on_line + center_to_ray * clamp(source_radius / max(length(center_to_ray), 1e-5), 0.0, 1.0);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (NoL <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }
    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_line = clamp(alpha + tube_length / (3.0 * dist), 0.0, 1.0);
    let alpha_sphere = clamp(alpha + source_radius / (2.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_line, 1e-4)) * (alpha / max(alpha_sphere, 1e-4)) * (alpha / max(alpha_sphere, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let diffuse = kD * albedo * NoL * INV_PI;
    let specular = D * Vis * F * NoLs;
    return (diffuse + specular) * light_color * intensity * atten;
}

// Where `world_pos` falls on a spot light's cookie (`gizmo_renderer::cookies`): the outer cone
// fills [0,1]², row 0 at the top. The cone's roll is not packed, so the image's up is world up as
// seen down the cone — world −Z for a cone pointing straight up or down.
fn spot_cookie_uv(light: LightData, world_pos: vec3<f32>) -> vec2<f32> {
    let d = normalize(light.direction.xyz);
    let up_ref = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0), abs(d.y) > 0.99);
    let right = normalize(cross(d, up_ref));
    let up = cross(right, d);
    let v = world_pos - light.position.xyz;
    let z = max(dot(v, d), 1e-4);
    let c = clamp(light.params.x, 1e-4, 0.9999);
    let tan_outer = sqrt(1.0 - c * c) / c;
    let p = vec2<f32>(dot(v, right), dot(v, up)) / (z * tan_outer);
    return vec2<f32>(0.5 + 0.5 * p.x, 0.5 - 0.5 * p.y);
}

// `v` turned by the unit quaternion `q` (xyzw).
fn rotate_by_quat(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// One edge's share of the vector form factor of a polygon: θ · n̂, with θ the angle between the two
// corner directions and n̂ the normal of the plane through them. `atan2` rather than `acos` keeps
// it stable for the near-parallel edges a distant light has; θ / sin θ → 1 as they close.
fn rect_edge(v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32> {
    let c = cross(v1, v2);
    let s = length(c);
    return c * (atan2(s, dot(v1, v2)) / max(s, 1e-7));
}

// A one-sided rectangular emitter (`RectLight`): centre `to_light` from the surface, oriented by the
// quaternion `orientation`, half extents `half_w` along its local X and `half_h` along its local Y,
// emitting along its local −Z. `window` is the light's range falloff alone — the inverse square is
// in the integral below, not in it.
//
// Diffuse is exact for a Lambertian receiver, before horizon clipping: the cosine-weighted solid
// angle of the rectangle is ½·Σ θ_e n̂_e · N over its edges (Lambert's polygon formula). `intensity`
// is the light's on-axis radiant intensity — the rectangle's radiance is intensity / area — so from
// far away, seen face on, it lights like a point light of the same intensity.
//
// Specular takes the representative point: where the reflected ray meets the light's plane,
// clamped into the rectangle, with the lobe widened across each extent and renormalised so the
// highlight spreads its energy instead of adding to it — the tube light's line step, twice.
fn compute_rect_lighting(
    N: vec3<f32>,
    V: vec3<f32>,
    to_light: vec3<f32>,
    orientation: vec4<f32>,
    half_w: f32,
    half_h: f32,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    f0: vec3<f32>,
    light_color: vec3<f32>,
    intensity: f32,
    window: f32
) -> vec3<f32> {
    let right = rotate_by_quat(orientation, vec3<f32>(1.0, 0.0, 0.0));
    let up = rotate_by_quat(orientation, vec3<f32>(0.0, 1.0, 0.0));
    let emit = -normalize(cross(right, up));
    let dist = max(length(to_light), 1e-4);
    // Behind the emitting face: nothing.
    let facing = dot(-to_light, emit) / dist;
    if (facing <= 0.0) {
        return vec3<f32>(0.0);
    }

    let a = right * half_w;
    let b = up * half_h;
    let p0 = normalize(to_light - a - b);
    let p1 = normalize(to_light + a - b);
    let p2 = normalize(to_light + a + b);
    let p3 = normalize(to_light - a + b);
    var form = 0.5 * (rect_edge(p0, p1) + rect_edge(p1, p2) + rect_edge(p2, p3) + rect_edge(p3, p0));
    // The winding's sign depends on which side the surface sees; the vector form factor points at
    // the light either way.
    if (dot(form, to_light) < 0.0) {
        form = -form;
    }
    let diffuse_integral = max(dot(form, N), 0.0);

    let r = reflect(-V, N);
    let denom = dot(r, emit);
    var on_plane: vec3<f32>;
    let t = dot(to_light, emit) / select(denom, -1e-6, abs(denom) < 1e-6);
    if (denom < -1e-6 && t > 0.0) {
        on_plane = r * t;
    } else {
        // The ray never meets the face: walk its in-plane direction far out and clamp back.
        on_plane = to_light + (r - emit * denom) * 1e4;
    }
    let local = on_plane - to_light;
    let closest = to_light + right * clamp(dot(local, right), -half_w, half_w)
                           + up * clamp(dot(local, up), -half_h, half_h);
    let Ls = normalize(closest);
    let NoLs = max(dot(N, Ls), 0.0);
    if (diffuse_integral <= 0.0 && NoLs <= 0.0) {
        return vec3<f32>(0.0);
    }

    let H = normalize(V + Ls);
    let NoV = max(dot(N, V), 0.001);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let alpha = roughness * roughness;
    let alpha_w = clamp(alpha + (2.0 * half_w) / (3.0 * dist), 0.0, 1.0);
    let alpha_h = clamp(alpha + (2.0 * half_h) / (3.0 * dist), 0.0, 1.0);
    let norm = (alpha / max(alpha_w, 1e-4)) * (alpha / max(alpha_h, 1e-4));

    let D = D_GGX(NoH, roughness) * norm;
    let Vis = V_SmithJointGGX(NoV, NoLs, roughness);
    let F = F_Schlick(VoH, f0);
    let kD = (vec3<f32>(1.0) - F) * (1.0 - metallic);
    let area = max(4.0 * half_w * half_h, 1e-6);
    let diffuse = kD * albedo * INV_PI * (diffuse_integral / area);
    let specular = D * Vis * F * NoLs * facing / (dist * dist + 0.01);
    return (diffuse + specular) * light_color * intensity * window;
}

// Full 4x4 inverse (used for NDC → world unprojection in fullscreen passes).
fn inverse_mat4(m: mat4x4<f32>) -> mat4x4<f32> {
    let n11 = m[0][0]; let n12 = m[1][0]; let n13 = m[2][0]; let n14 = m[3][0];
    let n21 = m[0][1]; let n22 = m[1][1]; let n23 = m[2][1]; let n24 = m[3][1];
    let n31 = m[0][2]; let n32 = m[1][2]; let n33 = m[2][2]; let n34 = m[3][2];
    let n41 = m[0][3]; let n42 = m[1][3]; let n43 = m[2][3]; let n44 = m[3][3];

    let t11 = n23 * n34 * n42 - n24 * n33 * n42 + n24 * n32 * n43 - n22 * n34 * n43 - n23 * n32 * n44 + n22 * n33 * n44;
    let t12 = n14 * n33 * n42 - n13 * n34 * n42 - n14 * n32 * n43 + n12 * n34 * n43 + n13 * n32 * n44 - n12 * n33 * n44;
    let t13 = n13 * n24 * n42 - n14 * n23 * n42 + n14 * n22 * n43 - n12 * n24 * n43 - n13 * n22 * n44 + n12 * n23 * n44;
    let t14 = n14 * n23 * n32 - n13 * n24 * n32 - n14 * n22 * n33 + n12 * n24 * n33 + n13 * n22 * n34 - n12 * n23 * n34;

    let det = n11 * t11 + n21 * t12 + n31 * t13 + n41 * t14;

    if (abs(det) < 1e-6) {
        return mat4x4<f32>(
            vec4<f32>(1.0, 0.0, 0.0, 0.0),
            vec4<f32>(0.0, 1.0, 0.0, 0.0),
            vec4<f32>(0.0, 0.0, 1.0, 0.0),
            vec4<f32>(0.0, 0.0, 0.0, 1.0)
        );
    }

    let idet = 1.0 / det;

    let t21 = n24 * n33 * n41 - n24 * n31 * n43 - n23 * n34 * n41 + n21 * n34 * n43 + n23 * n31 * n44 - n21 * n33 * n44;
    let t22 = n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n43 - n11 * n34 * n43 - n13 * n31 * n44 + n11 * n33 * n44;
    let t23 = n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n43 + n11 * n24 * n43 + n13 * n21 * n44 - n11 * n23 * n44;
    let t24 = n13 * n24 * n31 - n14 * n23 * n31 + n14 * n21 * n33 - n11 * n24 * n33 - n13 * n21 * n34 + n11 * n23 * n34;

    let t31 = n22 * n34 * n41 - n24 * n32 * n41 + n24 * n31 * n42 - n21 * n34 * n42 - n22 * n31 * n44 + n21 * n32 * n44;
    let t32 = n14 * n32 * n41 - n12 * n34 * n41 - n14 * n31 * n42 + n11 * n34 * n42 + n12 * n31 * n44 - n11 * n32 * n44;
    let t33 = n12 * n24 * n41 - n14 * n22 * n41 + n14 * n21 * n42 - n11 * n24 * n42 - n12 * n21 * n44 + n11 * n22 * n44;
    let t34 = n14 * n22 * n31 - n12 * n24 * n31 - n14 * n21 * n32 + n11 * n24 * n32 + n12 * n21 * n34 - n11 * n22 * n34;

    let t41 = n23 * n32 * n41 - n22 * n33 * n41 - n23 * n31 * n42 + n21 * n33 * n42 + n22 * n31 * n43 - n21 * n32 * n43;
    let t42 = n12 * n33 * n41 - n13 * n32 * n41 + n13 * n31 * n42 - n11 * n33 * n42 - n12 * n31 * n43 + n11 * n32 * n43;
    let t43 = n13 * n22 * n41 - n12 * n23 * n41 - n13 * n21 * n42 + n11 * n23 * n42 + n12 * n21 * n43 - n11 * n22 * n43;
    let t44 = n12 * n23 * n31 - n13 * n22 * n31 + n13 * n21 * n32 - n11 * n23 * n32 - n12 * n21 * n33 + n11 * n22 * n33;

    return mat4x4<f32>(
        vec4<f32>(t11 * idet, t21 * idet, t31 * idet, t41 * idet),
        vec4<f32>(t12 * idet, t22 * idet, t32 * idet, t42 * idet),
        vec4<f32>(t13 * idet, t23 * idet, t33 * idet, t43 * idet),
        vec4<f32>(t14 * idet, t24 * idet, t34 * idet, t44 * idet)
    );
}
