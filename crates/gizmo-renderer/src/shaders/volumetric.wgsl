// Volumetric Lighting Shader (God Rays)
// SceneUniforms and LightData come from gizmo::common (composed in by load_shader_composed).
// Only volumetric-specific helpers stay local. The NDC→world unprojection reads the
// CPU-computed scene.inv_view_proj instead of inverting view_proj per fragment.
#import gizmo::common::{SceneUniforms, LightData}

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
    // Unproject the pixel through the inverse view-projection instead of assuming a
    // fixed 1280x720 / 45°-FOV camera. The old hardcoded aspect + fov_factor produced
    // the wrong world ray on any other resolution or FOV, throwing the sky god-rays off
    // in the wrong direction — exactly where they are most visible.
    let ndc = vec2<f32>(in.screen_uv.x * 2.0 - 1.0, 1.0 - in.screen_uv.y * 2.0);
    let inv_vp = scene.inv_view_proj;
    let near_h = inv_vp * vec4<f32>(ndc, 0.0, 1.0);
    let near_world = near_h.xyz / near_h.w;
    let ray_dir_reconstructed = normalize(near_world - cam_pos);

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
