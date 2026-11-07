#define_import_path gizmo::pbr_ext

// PBR extensions beyond the core Lambert + isotropic GGX in gizmo::common. The forward shader takes
// the environment BRDF from here too, and the rest is deferred-only:
// the Lazarov analytical split-sum environment BRDF, anisotropic GGX (tangent/bitangent
// stretched highlight), and a clear-coat lacquer lobe. Pure per the common.wgsl convention —
// no reference to a module-scope binding (`scene`, textures); everything arrives as params.
// The core D_GGX / V_SmithJointGGX / F_Schlick are shared from gizmo::common.
#import gizmo::common::{D_GGX, V_SmithJointGGX, F_Schlick}

// Analytical Environment BRDF (2D LUT approximation by Lazarov)
fn approximate_env_brdf(NdV: f32, roughness: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * NdV)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

fn D_GGX_anisotropic(ToH: f32, BoH: f32, NoH: f32, roughness_t: f32, roughness_b: f32) -> f32 {
    let at = roughness_t * roughness_t;
    let ab = roughness_b * roughness_b;
    let a2 = at * ab;
    // Correct Burley/Filament anisotropic GGX: the tangent/bitangent terms divide by
    // the per-axis alpha SQUARED (at = roughness_t^2 already), i.e. /(at*at), not /at.
    // The old /at under-divided them, flattening the highlight's anisotropic stretch.
    let denom = (ToH * ToH) / (at * at) + (BoH * BoH) / (ab * ab) + NoH * NoH;
    return 1.0 / (3.1415926535 * a2 * denom * denom);
}

fn V_SmithJointGGX_anisotropic(ToV: f32, BoV: f32, NoV: f32, ToL: f32, BoL: f32, NoL: f32, roughness_t: f32, roughness_b: f32) -> f32 {
    let at = roughness_t * roughness_t;
    let ab = roughness_b * roughness_b;
    let lambdaV = NoL * length(vec3<f32>(at * ToV, ab * BoV, NoV));
    let lambdaL = NoV * length(vec3<f32>(at * ToL, ab * BoL, NoL));
    return 0.5 / max(lambdaV + lambdaL, 0.0001);
}

fn compute_direct_lighting_anisotropic(
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    T: vec3<f32>,
    B: vec3<f32>,
    albedo: vec3<f32>,
    roughness: f32,
    metallic: f32,
    anisotropy: f32,
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

    // Clamp to the valid roughness range: roughness*(1+anisotropy) can exceed 1.0 for
    // a rough, strongly-anisotropic surface, pushing the GGX alpha out of [0,1].
    let roughness_t = clamp(roughness * (1.0 + anisotropy), 0.001, 1.0);
    let roughness_b = clamp(roughness * (1.0 - anisotropy), 0.001, 1.0);

    let ToH = dot(T, H);
    let BoH = dot(B, H);
    let ToV = dot(T, V);
    let BoV = dot(B, V);
    let ToL = dot(T, L);
    let BoL = dot(B, L);

    let D = D_GGX_anisotropic(ToH, BoH, NoH, roughness_t, roughness_b);
    let Vis = V_SmithJointGGX_anisotropic(ToV, BoV, NoV, ToL, BoL, NoL, roughness_t, roughness_b);
    let F = F_Schlick(VoH, f0);

    let kS = F;
    let kD = (vec3<f32>(1.0) - kS) * (1.0 - metallic);

    let diffuse = kD * albedo * NoL * 0.31830988618; // Lambert: albedo / PI (energy-consistent with the 1/PI in D_GGX)
    let specular = D * Vis * F * NoL;

    return (diffuse + specular) * light_color * intensity * atten;
}

fn compute_clear_coat(
    N: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
    light_color: vec3<f32>, intensity: f32, visibility: f32, roughness: f32
) -> vec3<f32> {
    let H = normalize(V + L);
    let NoH = max(dot(N, H), 0.0);
    let VoH = max(dot(V, H), 0.0);
    let NoL = max(dot(N, L), 0.0);
    let NoV = max(dot(N, V), 0.001);

    let D = D_GGX(NoH, roughness);
    let V_term = V_SmithJointGGX(NoV, NoL, roughness);
    let F = 0.04 + (1.0 - 0.04) * pow(1.0 - VoH, 5.0);

    return vec3<f32>(D * V_term * F) * light_color * intensity * visibility * NoL;
}

// Subsurface's lift of the ambient term.
fn subsurface_ambient(ambient: vec3<f32>, subsurface: f32) -> vec3<f32> {
    let sss_color = vec3<f32>(0.96, 0.28, 0.15);
    return ambient + ambient * subsurface * sss_color * 0.45;
}

// The mirror ray, bent along the tangent by anisotropy. A smooth surface bends it most.
fn anisotropic_reflection(R: vec3<f32>, N: vec3<f32>, T: vec3<f32>, anisotropy: f32, roughness: f32) -> vec3<f32> {
    let stretch = anisotropy * (1.0 - roughness);
    let direction = cross(cross(N, T), N);
    return normalize(mix(R, direction, stretch));
}

// How much of the environment gets through the clear coat to the layer under it.
fn coat_env_attenuation(NdV: f32, clear_coat: f32) -> f32 {
    let F_env = 0.04 + (1.0 - 0.04) * pow(1.0 - NdV, 5.0);
    return 1.0 - clear_coat * F_env;
}

// The clear coat's own reflection of `env`, seen from `NdV`, before weighting by the coat.
fn coat_env_specular(env: vec3<f32>, NdV: f32, roughness: f32) -> vec3<f32> {
    let lut = approximate_env_brdf(NdV, roughness);
    return env * (0.04 * lut.x + lut.y);
}

// One light's `lit` result with the clear coat laid over it.
fn layer_clear_coat(
    lit: vec3<f32>, N: vec3<f32>, V: vec3<f32>, L: vec3<f32>,
    light_color: vec3<f32>, intensity: f32, attenuation: f32, clear_coat: f32, coat_roughness: f32,
) -> vec3<f32> {
    let H = normalize(V + L);
    let VoH = max(dot(V, H), 0.0);
    let F_c = 0.04 + (1.0 - 0.04) * pow(1.0 - VoH, 5.0);
    let coat_atten = 1.0 - clear_coat * F_c;
    let coat_spec = compute_clear_coat(N, V, L, light_color, intensity, attenuation, coat_roughness);
    return lit * coat_atten + coat_spec * clear_coat;
}

// What scatters through the surface from a light, per unit of that light.
fn subsurface_light(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, metallic: f32, subsurface: f32) -> vec3<f32> {
    let sss_wrap = 0.35;
    let sss_ndl = max((dot(N, L) + sss_wrap) / (1.0 + sss_wrap), 0.0);
    let sss_power = 8.0;
    let sss_scale = 0.65;
    let sss_trans = pow(max(dot(-V, L), 0.0), sss_power) * sss_scale * (1.0 - metallic);
    let sss_color = vec3<f32>(0.96, 0.28, 0.15);
    return (sss_ndl * 0.12 + sss_trans) * subsurface * sss_color;
}
