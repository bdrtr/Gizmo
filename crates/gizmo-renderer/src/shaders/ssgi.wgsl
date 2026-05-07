// SSGI (Screen Space Global Illumination) Shader

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

// Pseudo-random number generator
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

// Generate cosine-weighted hemisphere sample
fn get_sample_dir(normal: vec3<f32>, seed1: f32, seed2: f32) -> vec3<f32> {
    let theta = acos(sqrt(1.0 - seed1));
    let phi = 2.0 * 3.14159265 * seed2;

    let x = sin(theta) * cos(phi);
    let y = sin(theta) * sin(phi);
    let z = cos(theta);

    let up = select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(normal.z) < 0.999);
    let tangent = normalize(cross(up, normal));
    let bitangent = cross(normal, tangent);

    return tangent * x + bitangent * y + normal * z;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let iuv = vec2<i32>(i32(frag_coord.x) * 2, i32(frag_coord.y) * 2);
    let tex_dim = vec2<f32>(textureDimensions(t_hdr));

    let normal_roughness = textureLoad(t_normal_roughness, iuv, 0);
    let pos_sample = textureLoad(t_world_position, iuv, 0);

    // Skip sky/unwritten pixels
    if (pos_sample.w < 0.5) {
        return vec4(0.0);
    }

    let world_pos = pos_sample.xyz;
    let normal = normalize(normal_roughness.xyz);
    let view_dir = normalize(world_pos - scene.camera_pos.xyz);
    
    var indirect_light = vec3<f32>(0.0);
    
    // Ray marching params
    let max_steps = 8;
    let step_size = 0.5;
    let ray_count = 1; // Num rays per pixel
    
    for (var r = 0; r < ray_count; r++) {
        // Generate random seeds per ray
        let s1 = hash(frag_coord.xy + vec2<f32>(f32(r) * 13.0, f32(r) * 31.0));
        let s2 = hash(frag_coord.xy + vec2<f32>(f32(r) * 27.0, f32(r) * 19.0));
        
        let ray_dir = get_sample_dir(normal, s1, s2);
        var current_pos = world_pos + ray_dir * 0.2; // Offset
        var hit_color = vec3<f32>(0.0);

        for (var i = 0; i < max_steps; i++) {
            current_pos += ray_dir * step_size;
            
            let clip_pos = scene.view_proj * vec4(current_pos, 1.0);
            let ndc = clip_pos.xyz / clip_pos.w;
            
            if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) {
                break; // Out of screen
            }
            
            let screen_uv = vec2(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
            let sample_iuv = vec2<i32>(i32(screen_uv.x * tex_dim.x), i32(screen_uv.y * tex_dim.y));
            
            let scene_pos = textureLoad(t_world_position, sample_iuv, 0);
            
            // Depth check
            if (scene_pos.w > 0.5) {
                let scene_z = length(scene_pos.xyz - scene.camera_pos.xyz);
                let current_z = length(current_pos - scene.camera_pos.xyz);
                let depth_diff = current_z - scene_z;
                
                if (depth_diff > 0.0 && depth_diff < 1.0) {
                    let hit_normal = normalize(textureLoad(t_normal_roughness, sample_iuv, 0).xyz);
                    
                    // Don't bounce light from backsides
                    let n_dot_l = max(dot(hit_normal, -ray_dir), 0.0);
                    
                    if (n_dot_l > 0.0) {
                        let sample_color = textureLoad(t_hdr, sample_iuv, 0).rgb;
                        // Edge fade
                        let edge_fade = smoothstep(0.0, 0.1, screen_uv.x) * smoothstep(1.0, 0.9, screen_uv.x) *
                                        smoothstep(0.0, 0.1, screen_uv.y) * smoothstep(1.0, 0.9, screen_uv.y);
                        hit_color = sample_color * n_dot_l * edge_fade;
                    }
                    break;
                }
            }
        }
        
        indirect_light += hit_color;
    }

    indirect_light /= f32(ray_count);
    
    // Boost SSGI brightness a little to make it visible
    return vec4(indirect_light * 0.5, 1.0);
}
