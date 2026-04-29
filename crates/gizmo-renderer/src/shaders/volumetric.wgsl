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
    let iuv = vec2<i32>(i32(in.position.x) * 2, i32(in.position.y) * 2);
    let pos_sample = textureLoad(t_world_position, iuv, 0);
    
    let cam_pos = scene.camera_pos.xyz;
    var target_pos = pos_sample.xyz;
    
    // If the pixel is skybox (w == 0.0), target_pos is 0.0. 
    // We must reconstruct ray direction to far plane.
    var ray_dir = vec3<f32>(0.0);
    var max_dist = 0.0;
    
    if (pos_sample.w < 0.5) {
        return vec4(0.0);
    } else {
        let diff = target_pos - cam_pos;
        max_dist = min(length(diff), 100.0); // max 100 units for volumetrics
        ray_dir = normalize(diff);
    }
    
    if (max_dist < 1.0) {
        return vec4(0.0);
    }
    
    let steps = 12.0; // Optimized from 16 to 12
    let step_size = max_dist / steps;
    
    var current_pos = cam_pos;
    
    // Jitter starting position to trade banding for noise (TAA will smooth it out)
    let jitter = rand(in.position.xy) * step_size;
    current_pos += ray_dir * jitter;
    
    var total_scatter = vec3<f32>(0.0);
    
    // Precalculate light properties
    let L = normalize(-scene.sun_direction.xyz);
    let cos_theta = dot(ray_dir, L);
    let phase = phase_function(0.6, cos_theta); 
    // Normalize scattering based on step count and reduce intensity drastically
    let light_intensity = scene.sun_color.rgb * scene.sun_color.w * 0.002;
    
    for (var i = 0.0; i < steps; i += 1.0) {
        current_pos += ray_dir * step_size;
        
        let view_depth = dot(current_pos - cam_pos, scene.camera_forward.xyz);
        let ci = select_cascade(view_depth);
        
        let light_clip = scene.light_view_proj[ci] * vec4<f32>(current_pos, 1.0);
        let light_ndc = light_clip.xyz / light_clip.w;
        let shadow_uv = vec2<f32>(light_ndc.x * 0.5 + 0.5, light_ndc.y * -0.5 + 0.5);
        
        if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
            let bias = 0.001;
            // 1 tap shadow sample is enough for volumetrics
            let shadow_val = textureSampleCompare(t_shadow, s_shadow, shadow_uv, ci, light_ndc.z - bias);
            
            if (shadow_val > 0.0) {
                total_scatter += light_intensity * phase * shadow_val * step_size;
            }
        }
    }
    
    return vec4<f32>(total_scatter, 1.0);
}
