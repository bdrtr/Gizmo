// Volumetric Lighting Shader (God Rays)

struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
};

struct SceneUniforms {
    view_proj:       mat4x4<f32>,
    camera_pos:      vec4<f32>,
    sun_direction:   vec4<f32>,
    sun_color:       vec4<f32>,
    lights:          array<LightData, 10>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits:  vec4<f32>,
    camera_forward:  vec4<f32>,
    cascade_params:  vec4<f32>,
    num_lights: u32,
    _pad: vec3<u32>,
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_shadow: texture_depth_2d_array;
@group(1) @binding(1) var s_shadow: sampler_comparison;

@group(2) @binding(0) var t_world_position: texture_2d<f32>;
@group(2) @binding(1) var s_linear: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) screen_uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 3.0, -1.0),
        vec2<f32>(-1.0,  3.0),
    );
    var uv = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(2.0, 1.0),
        vec2<f32>(0.0, -1.0),
    );
    var out: VertexOutput;
    out.position = vec4<f32>(pos[vi], 0.0, 1.0);
    out.screen_uv = uv[vi];
    return out;
}

fn select_cascade(view_depth: f32) -> u32 {
    if (view_depth < scene.cascade_splits.x) { return 0u; }
    if (view_depth < scene.cascade_splits.y) { return 1u; }
    if (view_depth < scene.cascade_splits.z) { return 2u; }
    return 3u;
}

// Pseudo-random number generator
fn rand(co: vec2<f32>) -> f32 {
    return fract(sin(dot(co.xy ,vec2(12.9898,78.233))) * 43758.5453);
}

// Henyey-Greenstein phase function for forward scattering
fn phase_function(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    let num = 1.0 - g2;
    let denom = pow(1.0 + g2 - 2.0 * g * cos_theta, 1.5);
    return (1.0 / (4.0 * 3.14159)) * (num / max(denom, 0.001));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = textureDimensions(t_world_position);
    let iuv = vec2<i32>(in.screen_uv * vec2<f32>(tex_size));
    let pos_sample = textureLoad(t_world_position, iuv, 0);
    let cam_pos = scene.camera_pos.xyz;
    
    // ── Ray Direction Reconstruction ──
    let aspect = 1280.0 / 720.0; 
    let fov_factor = 0.414; // tan(pi/8) approx
    let ndc = vec2<f32>(in.screen_uv.x * 2.0 - 1.0, (1.0 - in.screen_uv.y) * 2.0 - 1.0);
    
    var cam_right = normalize(cross(scene.camera_forward.xyz, vec3<f32>(0.0, 1.0, 0.0)));
    if (length(cam_right) < 0.001) {
        cam_right = normalize(cross(scene.camera_forward.xyz, vec3<f32>(0.0, 0.0, 1.0)));
    }
    let cam_up = cross(cam_right, scene.camera_forward.xyz);
    
    let ray_dir_reconstructed = normalize(
        scene.camera_forward.xyz +
        ndc.x * fov_factor * aspect * cam_right +
        ndc.y * fov_factor * cam_up
    );

    var target_pos = pos_sample.xyz;
    var ray_dir = ray_dir_reconstructed;
    var max_dist = 100.0; // Default far clip for skybox God Rays
    
    if (pos_sample.w >= 0.5) {
        let diff = target_pos - cam_pos;
        max_dist = min(length(diff), 100.0);
        ray_dir = normalize(diff);
    }
    
    if (max_dist < 0.1) {
        return vec4(0.0);
    }
    
    let steps = 16.0; 
    let step_size = max_dist / steps;
    
    var current_pos = cam_pos;
    
    // Jitter starting position to trade banding for noise (smooths out with TAA/bloom)
    let jitter = rand(in.position.xy) * step_size;
    current_pos += ray_dir * jitter;
    
    var total_scatter = vec3<f32>(0.0);
    
    // Sun Directional Light Precalculations
    let L_sun = normalize(-scene.sun_direction.xyz);
    let cos_theta = dot(ray_dir, L_sun);
    let sun_phase = phase_function(0.55, cos_theta); 
    let sun_intensity = scene.sun_color.rgb * scene.sun_color.w * 0.0015;
    
    for (var i = 0.0; i < steps; i += 1.0) {
        current_pos += ray_dir * step_size;
        
        // ── 1. Sun God Rays (Directional Cascade Shadows) ──
        if (scene.sun_direction.w > 0.5) {
            let view_depth = dot(current_pos - cam_pos, scene.camera_forward.xyz);
            let ci = select_cascade(view_depth);
            
            let light_clip = scene.light_view_proj[ci] * vec4<f32>(current_pos, 1.0);
            let light_ndc = light_clip.xyz / light_clip.w;
            let shadow_uv = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);
            
            if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
                let bias = 0.0015;
                let shadow_val = textureSampleCompare(t_shadow, s_shadow, shadow_uv, ci, light_ndc.z - bias);
                
                if (shadow_val > 0.0) {
                    total_scatter += sun_intensity * sun_phase * shadow_val * step_size;
                }
            }
        }
        
        // ── 2. Local Point & Spot Light Bulb Glows ──
        for (var j = 0u; j < scene.num_lights; j += 1u) {
            let light = scene.lights[j];
            let light_pos = light.position.xyz;
            let intensity = light.position.w;
            let color = light.color.rgb;
            let radius = light.color.a;
            
            let to_light = light_pos - current_pos;
            let dist = length(to_light);
            
            if (dist < radius) {
                let atten = clamp(1.0 - (dist / radius), 0.0, 1.0);
                // Dynamic volumetric light scattering factor
                let scatter = color * intensity * 0.0008 * atten * atten;
                total_scatter += scatter * step_size;
            }
        }
    }
    
    return vec4<f32>(total_scatter, 1.0);
}
