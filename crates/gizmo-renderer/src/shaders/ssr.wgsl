// SSR Raymarching Shader
// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms}

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

    // Fresnel effect
    let cos_theta = max(dot(-view_dir, normal), 0.0);
    let fresnel = 0.04 + (1.0 - 0.04) * pow(1.0 - cos_theta, 5.0);
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
                
                let reflection_intensity = fresnel * fade_roughness * edge_fade;
                return vec4(hit_color * reflection_intensity, 1.0);
            }
        }
    }

    return vec4(0.0);
}
