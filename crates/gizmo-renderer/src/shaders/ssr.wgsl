// SSR Raymarching Shader
// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms, dielectric_f0, scene_environment, is_baked_surface}
#import gizmo::pbr_ext::{approximate_env_brdf}
#import gizmo::reflection_probes::{probe_environment}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_hdr: texture_2d<f32>;
@group(1) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(1) @binding(2) var t_position_rel_camera: texture_2d<f32>;
@group(1) @binding(3) var s_nearest: sampler;

// The eight shaping numbers: `SsrParams` on the Rust side.
struct SsrParams {
    roughness_cutoff: f32,
    fade_start: f32,
    fade_end: f32,
    max_distance: f32,
    max_steps: f32,
    thickness: f32,
    start_offset: f32,
    edge_fade: f32,
    // TAA's frame counter, 0 without TAA (`SsrState::upload`): which frame's noise the steps start on.
    frame: u32,
}
@group(1) @binding(4) var<uniform> params: SsrParams;
// RT5: its alpha holds `Material::reflectance` (see `dielectric_f0`).
@group(1) @binding(5) var t_emissive: texture_2d<f32>;
// RT0: albedo, and metallic in its alpha. A metal's reflection is coloured by its albedo.
@group(1) @binding(6) var t_albedo_metallic: texture_2d<f32>;
// RT4: the specular tint's hue in `zw`, as `deferred_lighting.wgsl` reads it.
@group(1) @binding(7) var t_velocity_gbuf: texture_2d<f32>;

// The environment the lighting pass reflected, which this pass takes back wherever it finds what is
// really there. The same expression as `deferred_lighting.wgsl`'s.
fn reflected_environment(R: vec3<f32>, N: vec3<f32>, world_pos: vec3<f32>, roughness: f32) -> vec3<f32> {
    let bent = normalize(mix(R, N, roughness));
    let sky = scene_environment(
        bent, roughness, scene.sky_zenith, scene.sky_horizon.rgb, scene.sky_ground.rgb, scene.environment_preset,
        scene.environment_preset_b, scene.environment_blend_t, scene.sun_direction,
    );
    return probe_environment(R, world_pos, roughness, sky);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var pos = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x) * 2, i32(frag_coord.y) * 2);
    let tex_dim = vec2<f32>(textureDimensions(t_hdr));

    // The roughness's sign marks a lightmapped surface (`gbuffer.wgsl`); SSR wants the roughness.
    var normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    normal_roughness.w = abs(normal_roughness.w);
    let pos_sample = textureLoad(t_position_rel_camera, iuv, 0);

    // Skip unwritten pixels or rough surfaces
    if (pos_sample.w < 0.5 || normal_roughness.w > params.roughness_cutoff) {
        return vec4(0.0);
    }

    // G-buffer position is camera-relative (see gbuffer.wgsl); put it back in world space.
    let world_pos = pos_sample.xyz + scene.camera_pos.xyz;
    let normal = normalize(normal_roughness.xyz);
    let view_dir = normalize(world_pos - scene.camera_pos.xyz);
    
    // Reflect vector
    let R = normalize(reflect(view_dir, normal));

    // How much of what it reflects this surface shows: the lighting pass's own weight, so a metal
    // shows nearly all of it in its own colour.
    let cos_theta = max(dot(-view_dir, normal), 0.001);
    let albedo_metallic = textureLoad(t_albedo_metallic, iuv, 0);
    let hue_rb = vec2<f32>(1.0) - textureLoad(t_velocity_gbuf, iuv, 0).zw;
    let tint = vec3<f32>(hue_rb.x, max((1.0 - 0.2126 * hue_rb.x - 0.0722 * hue_rb.y) / 0.7152, 0.0), hue_rb.y);
    let f0 = mix(min(vec3<f32>(dielectric_f0(textureLoad(t_emissive, iuv, 0).w)) * tint, vec3<f32>(1.0)), albedo_metallic.rgb, albedo_metallic.a);
    let brdf = approximate_env_brdf(cos_theta, normal_roughness.w);
    let weight = f0 * brdf.x + brdf.y;
    let fade_roughness = 1.0 - smoothstep(params.fade_start, params.fade_end, normal_roughness.w);

    // A baked-lit surface (`BAKED_SURFACE`) was given no environment by the lighting pass — its
    // baked light is all it has — so this pass gives it all of its reflection: the environment
    // where the ray finds nothing on screen, what it finds where it does. A lit surface already has
    // the environment, and gets only the difference.
    let environment = reflected_environment(R, normal, world_pos, normal_roughness.w);
    let baked = is_baked_surface(pos_sample.w);
    let unfound = select(vec3<f32>(0.0), environment * weight * fade_roughness, baked);

    // The march is in screen space: the ray, cut at `max_distance` and in front of the near plane,
    // is projected and walked across the screen in equal steps until it leaves it. A march in equal
    // world steps reached `step × steps` metres and no further, which on a street is the next car:
    // what a wet road shows — the lights, the signs, the buildings down the street — is a hundred
    // metres off and more, and a few pixels away on screen.
    //
    // The ray is the mirror direction. A glossy surface spreads what it reflects over its lobe, and
    // `ssr_blur.wgsl` spreads it so, by the radius this pass leaves in alpha (`blur_radius`): a
    // direction drawn from the lobe each frame did the same with noise only TAA could take out, and
    // a game may run without TAA.
    let ray = R;
    let origin = world_pos + ray * params.start_offset;
    let clip_origin = scene.view_proj * vec4(origin, 1.0);
    let clip_dir = scene.view_proj * vec4(ray, 0.0);
    // `w` is the view depth: kept past the near plane, so a ray towards the camera ends before it.
    let near = max(scene.cascade_params.x, 1e-3);
    var reach = params.max_distance;
    if (clip_dir.w < 0.0) {
        reach = min(reach, (clip_origin.w - near) / -clip_dir.w);
    }
    // The lobe's width, for `ssr_blur.wgsl`: pixels per metre at a metre's depth, vertically.
    let focal_px = 0.5 * tex_dim.y * length(vec3<f32>(scene.view_proj[0][1], scene.view_proj[1][1], scene.view_proj[2][1]));
    let view_distance = length(pos_sample.xyz);
    if (clip_origin.w <= near || reach <= 0.0) {
        return vec4(unfound, blur_radius(normal_roughness.w, 0.0, view_distance, focal_px));
    }
    let clip_end = clip_origin + clip_dir * reach;
    let uv_origin = ndc_to_uv(clip_origin.xy / clip_origin.w);
    let uv_end = ndc_to_uv(clip_end.xy / clip_end.w);
    // How much of the way it stays on screen, so that no step is spent off it.
    let along = uv_end - uv_origin;
    var on_screen = 1.0;
    for (var axis = 0; axis < 2; axis++) {
        let d = along[axis];
        let start = uv_origin[axis];
        if (d > 1e-6) {
            on_screen = min(on_screen, (1.0 - start) / d);
        } else if (d < -1e-6) {
            on_screen = min(on_screen, -start / d);
        }
    }
    on_screen = max(on_screen, 0.0);
    let span_px = length(along * on_screen * tex_dim);
    let steps = clamp(i32(ceil(span_px)), 1, max(i32(params.max_steps), 1));
    let stride = on_screen / f32(steps);
    // Perspective-correct: `1/w` and `t/w` are linear across the screen, `t` (metres along R) is not.
    let inv_w0 = 1.0 / clip_origin.w;
    let inv_w1 = 1.0 / clip_end.w;
    // Where in its first step each pixel starts (interleaved gradient noise, moved each frame), so
    // that the steps' banding becomes grain TAA averages away.
    let noise_at = frag_coord.xy + 5.588238 * f32(params.frame % 64u);
    let jitter = fract(52.9829189 * fract(dot(noise_at, vec2<f32>(0.06711056, 0.00583715))));

    var previous_f = 0.0;
    var previous_depth = length(origin - scene.camera_pos.xyz);
    for (var i = 0; i < steps; i++) {
        let f = stride * (f32(i) + jitter);
        let ray_depth = march_depth(f, inv_w0, inv_w1, reach, origin, ray);
        let scene_rel = textureLoad(t_position_rel_camera, uv_to_texel(mix(uv_origin, uv_end, f), tex_dim), 0);
        // `>= 0.5`, not `> 0.5`: a written pixel of an ordinary material carries exactly 0.5 (see
        // `gbuffer.wgsl`), and the strict form once made every hit a miss. An unwritten one is sky,
        // which the ray passes over.
        if (scene_rel.w >= 0.5) {
            let scene_depth = length(scene_rel.xyz);
            // A hit is the ray passing the surface's depth during this step: behind it now, and
            // in front of its back (`thickness`) at the step's near end. A long step on a far
            // stretch of road is many metres, which a fixed thickness alone would step over.
            if (ray_depth >= scene_depth && min(previous_depth, ray_depth) <= scene_depth + params.thickness) {
                // Where in the step it crossed.
                var lo = previous_f;
                var hi = f;
                for (var k = 0; k < 5; k++) {
                    let mid = 0.5 * (lo + hi);
                    let mid_rel = textureLoad(t_position_rel_camera, uv_to_texel(mix(uv_origin, uv_end, mid), tex_dim), 0);
                    if (mid_rel.w >= 0.5 && march_depth(mid, inv_w0, inv_w1, reach, origin, ray) >= length(mid_rel.xyz)) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                let screen_uv = mix(uv_origin, uv_end, hi);
                let hit_color = textureLoad(t_hdr, uv_to_texel(screen_uv, tex_dim), 0).rgb;

                let ef = params.edge_fade;
                let edge_fade = smoothstep(0.0, ef, screen_uv.x) * smoothstep(1.0, 1.0 - ef, screen_uv.x) *
                                smoothstep(0.0, ef, screen_uv.y) * smoothstep(1.0, 1.0 - ef, screen_uv.y);

                // What it found replaces the environment, so this can be negative, and the additive
                // pass that applies it takes light away: a dark thing reflected in a metal floor is
                // dark. Added on top, a metal would have shown both pictures at once.
                let travelled = params.start_offset + march_t(hi, inv_w0, inv_w1, reach);
                return vec4(
                    (hit_color - environment) * weight * fade_roughness * edge_fade + unfound,
                    blur_radius(normal_roughness.w, travelled, view_distance, focal_px),
                );
            }
        }
        previous_f = f;
        previous_depth = ray_depth;
    }

    // Nothing found: the lobe is as wide as the ray went before it left the screen.
    let travelled = params.start_offset + march_t(on_screen, inv_w0, inv_w1, reach);
    return vec4(unfound, blur_radius(normal_roughness.w, travelled, view_distance, focal_px));
}

// How wide, in full-resolution pixels, a surface of this roughness spreads what it reflects from
// `travelled` metres away: the lobe's width there (α = r², GGX's, as an angle), seen from the camera at
// the distance of the reflected image — the surface's, and the ray's beyond it. At least a hair over
// zero, which tells `ssr_blur.wgsl` the pixel reflects.
fn blur_radius(roughness: f32, travelled: f32, view_distance: f32, focal_px: f32) -> f32 {
    let alpha = roughness * roughness;
    return max(alpha * travelled / max(view_distance + travelled, 1e-3) * focal_px, 1e-3);
}

fn ndc_to_uv(ndc: vec2<f32>) -> vec2<f32> {
    return vec2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

fn uv_to_texel(uv: vec2<f32>, dim: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(uv * dim), vec2<i32>(0), vec2<i32>(dim) - vec2<i32>(1));
}

// How far along the ray, in metres, the march's point a fraction `f` of the way across the screen is.
fn march_t(f: f32, inv_w0: f32, inv_w1: f32, reach: f32) -> f32 {
    return f * reach * inv_w1 / mix(inv_w0, inv_w1, f);
}

// That point's distance from the camera.
fn march_depth(f: f32, inv_w0: f32, inv_w1: f32, reach: f32, origin: vec3<f32>, R: vec3<f32>) -> f32 {
    return length(origin + R * march_t(f, inv_w0, inv_w1, reach) - scene.camera_pos.xyz);
}
