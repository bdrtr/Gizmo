// SSR Raymarching Shader

struct SceneUniforms {
    view_proj:       mat4x4<f32>,
    camera_pos:      vec4<f32>,
    sun_direction:   vec4<f32>,
    sun_color:       vec4<f32>,
    lights:          array<vec4<f32>, 40>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,
    num_lights:      u32,
    _pad:            vec3<u32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_hdr: texture_2d<f32>;
@group(1) @binding(1) var t_normal_roughness: texture_2d<f32>;
@group(1) @binding(2) var t_world_position: texture_2d<f32>;
@group(1) @binding(3) var s_nearest: sampler;

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
    let pos_sample = textureLoad(t_world_position, iuv, 0);

    // Skip unwritten pixels or rough surfaces
    if (pos_sample.w < 0.5 || normal_roughness.w > 0.5) {
        return vec4(0.0);
    }

    let world_pos = pos_sample.xyz;
    let normal = normalize(normal_roughness.xyz);
    let view_dir = normalize(world_pos - scene.camera_pos.xyz);
    
    // Reflect vector
    let R = normalize(reflect(view_dir, normal));

    // Fresnel effect
    let cos_theta = max(dot(-view_dir, normal), 0.0);
    let fresnel = 0.04 + (1.0 - 0.04) * pow(1.0 - cos_theta, 5.0);
    let fade_roughness = 1.0 - smoothstep(0.1, 0.5, normal_roughness.w);

    // Ray marching params
    let step_size = 1.0;
    let max_steps = 20;
    var current_pos = world_pos + R * 0.1; // offset slightly
    
    for (var i = 0; i < max_steps; i++) {
        current_pos += R * step_size;
        
        let clip_pos = scene.view_proj * vec4(current_pos, 1.0);
        let ndc = clip_pos.xyz / clip_pos.w;
        
        if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
            break; // Out of screen
        }
        
        let screen_uv = vec2(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
        let sample_iuv = vec2<i32>(i32(screen_uv.x * tex_dim.x), i32(screen_uv.y * tex_dim.y));
        
        let scene_pos = textureLoad(t_world_position, sample_iuv, 0);
        
        // Depth test check
        if (scene_pos.w > 0.5) {
            let depth_diff = length(current_pos - scene.camera_pos.xyz) - length(scene_pos.xyz - scene.camera_pos.xyz);
            
            // Hit condition
            if (depth_diff > 0.0 && depth_diff < 1.0) {
                let hit_color = textureLoad(t_hdr, sample_iuv, 0).rgb;
                
                // Edge fade
                let edge_fade = smoothstep(0.0, 0.1, screen_uv.x) * smoothstep(1.0, 0.9, screen_uv.x) *
                                smoothstep(0.0, 0.1, screen_uv.y) * smoothstep(1.0, 0.9, screen_uv.y);
                
                let reflection_intensity = fresnel * fade_roughness * edge_fade;
                return vec4(hit_color * reflection_intensity, 1.0);
            }
        }
    }

    return vec4(0.0);
}
