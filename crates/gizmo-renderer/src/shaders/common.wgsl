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
//   params.x     = outer cutoff cos, params.y   = light type (0=point,1=spot,2=dir)
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
    lights:          array<LightData, 10>,
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
};

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

    let t21 = n24 * n33 * n41 - n24 * n31 * n42 - n23 * n34 * n41 + n21 * n34 * n42 + n23 * n31 * n44 - n21 * n33 * n44;
    let t22 = n13 * n34 * n41 - n14 * n33 * n41 + n14 * n31 * n42 - n11 * n34 * n42 - n13 * n31 * n44 + n11 * n33 * n44;
    let t23 = n14 * n23 * n41 - n13 * n24 * n41 - n14 * n21 * n42 + n11 * n24 * n42 + n13 * n21 * n44 - n11 * n23 * n44;
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
