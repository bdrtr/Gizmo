// SSR Raymarching Shader
// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms, dielectric_f0, sky_gradient, preset_environment}
#import gizmo::pbr_ext::{approximate_env_brdf}
#import gizmo::reflection_probes::{probe_environment}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_hdr: texture_2d<f32>;
@group(1) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(1) @binding(2) var t_position_rel_camera: texture_2d<f32>;
@group(1) @binding(3) var s_nearest: sampler;

// The eight numbers that used to be literals in this file. See `SsrParams` on the Rust side; the
// defaults there are exactly the values that stood here, so nothing changed by being moved.
struct SsrParams {
    roughness_cutoff: f32,
    fade_start: f32,
    fade_end: f32,
    step_size: f32,
    max_steps: f32,
    thickness: f32,
    start_offset: f32,
    edge_fade: f32,
}
@group(1) @binding(4) var<uniform> params: SsrParams;
// RT5: its alpha holds `Material::reflectance` (see `dielectric_f0`).
@group(1) @binding(5) var t_emissive: texture_2d<f32>;
// RT0: albedo, and metallic in its alpha. A metal's reflection is coloured by its albedo.
@group(1) @binding(6) var t_albedo_metallic: texture_2d<f32>;

// The environment the lighting pass reflected, which this pass takes back wherever it finds what is
// really there. The same expression as `deferred_lighting.wgsl`'s.
fn reflected_environment(R: vec3<f32>, N: vec3<f32>, world_pos: vec3<f32>, roughness: f32) -> vec3<f32> {
    let bent = normalize(mix(R, N, roughness));
    var sky = vec3<f32>(0.0);
    if (scene.sky_zenith.a >= 0.0) {
        sky = sky_gradient(bent, scene.sky_zenith.rgb, scene.sky_horizon.rgb, scene.sky_ground.rgb, roughness) * scene.sky_zenith.a;
    } else {
        sky = preset_environment(bent, roughness, scene.environment_preset, scene.environment_preset_b, scene.environment_blend_t, normalize(-scene.sun_direction.xyz));
    }
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

    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
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
    let f0 = mix(vec3<f32>(dielectric_f0(textureLoad(t_emissive, iuv, 0).w)), albedo_metallic.rgb, albedo_metallic.a);
    let brdf = approximate_env_brdf(cos_theta, normal_roughness.w);
    let weight = f0 * brdf.x + brdf.y;
    let fade_roughness = 1.0 - smoothstep(params.fade_start, params.fade_end, normal_roughness.w);

    // Ray marching params
    let step_size = params.step_size;
    let max_steps = i32(max(params.max_steps, 1.0));
    var current_pos = world_pos + R * params.start_offset;
    
    for (var i = 0; i < max_steps; i++) {
        current_pos += R * step_size;
        
        let clip_pos = scene.view_proj * vec4(current_pos, 1.0);
        // Guard the perspective divide: once the ray marches behind the camera clip_pos.w
        // goes negative, the divide flips signs and a point behind the viewer can fold back
        // into [-1,1] NDC and register as a false reflection hit.
        if (clip_pos.w <= 0.0) {
            break;
        }
        let ndc = clip_pos.xyz / clip_pos.w;

        if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
            break; // Out of screen
        }
        
        let screen_uv = vec2(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
        let sample_iuv = vec2<i32>(i32(screen_uv.x * tex_dim.x), i32(screen_uv.y * tex_dim.y));
        
        let scene_pos = textureLoad(t_position_rel_camera, sample_iuv, 0);
        
        // Depth test check. `>= 0.5`, NOT `> 0.5`: gbuffer.wgsl writes the written-flag as
        // `(0.5 + 0.49·anisotropy) + floor(100·subsurface)`, so a written pixel of an ordinary
        // material (anisotropy 0, subsurface 0) carries EXACTLY 0.5 — and 0.5 is exactly
        // representable in this Rgba16Float target, so the strict form was false for every
        // such pixel. The march then ran its full 20 steps without ever registering a hit and
        // returned black for the whole frame: measured, SSR moved 0 of 65536 bytes on a mirror
        // floor whether the pass ran or not. The entry gate above and every other reader of
        // this flag (ssao, ssgi's own gate, deferred_lighting, volumetric, taa, ssgi_temporal)
        // already treat 0.5 as written.
        if (scene_pos.w >= 0.5) {
            // `scene_pos` is already camera-relative; `current_pos` is in world space.
            let depth_diff = length(current_pos - scene.camera_pos.xyz) - length(scene_pos.xyz);
            
            // Hit condition
            if (depth_diff > 0.0 && depth_diff < params.thickness) {
                let hit_color = textureLoad(t_hdr, sample_iuv, 0).rgb;
                
                // Edge fade
                let ef = params.edge_fade;
                let edge_fade = smoothstep(0.0, ef, screen_uv.x) * smoothstep(1.0, 1.0 - ef, screen_uv.x) *
                                smoothstep(0.0, ef, screen_uv.y) * smoothstep(1.0, 1.0 - ef, screen_uv.y);
                
                // The lighting pass has already added the environment here. What it found replaces that,
                // so this can be negative, and the additive pass that applies it takes light away: a
                // dark thing reflected in a metal floor is dark. Added on top, a metal would have shown
                // both pictures at once, which is why this was held down at 4 % for everything.
                let environment = reflected_environment(R, normal, world_pos, normal_roughness.w);
                return vec4((hit_color - environment) * weight * fade_roughness * edge_fade, 1.0);
            }
        }
    }

    return vec4(0.0);
}
