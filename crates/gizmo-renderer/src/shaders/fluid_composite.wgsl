struct LightData {
    position: vec4<f32>,
    color: vec4<f32>,
    direction: vec4<f32>,
    params: vec4<f32>,
}

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    lights: array<LightData, 10>,
    light_view_proj: array<mat4x4<f32>, 4>,
    cascade_splits: vec4<f32>,
    camera_forward: vec4<f32>,
    cascade_params: vec4<f32>,
    num_lights: u32,
}
@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_depth: texture_2d<f32>;
@group(1) @binding(1) var s_depth: sampler;
@group(1) @binding(2) var t_thickness: texture_2d<f32>;
@group(1) @binding(3) var s_thickness: sampler;
@group(1) @binding(4) var t_opaque_bg: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);
    out.uv = vec2<f32>(x * 0.5, y * 0.5);
    out.clip_position = vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
    return out;
}

fn compute_normal(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let size = textureDimensions(t_depth);
    let texel = 1.0 / vec2<f32>(f32(size.x), f32(size.y));
    
    // Central difference for smoother normals
    let d_r = textureSampleLevel(t_depth, s_depth, uv + vec2<f32>(texel.x, 0.0), 0.0).x;
    let d_l = textureSampleLevel(t_depth, s_depth, uv - vec2<f32>(texel.x, 0.0), 0.0).x;
    let d_u = textureSampleLevel(t_depth, s_depth, uv + vec2<f32>(0.0, texel.y), 0.0).x;
    let d_d = textureSampleLevel(t_depth, s_depth, uv - vec2<f32>(0.0, texel.y), 0.0).x;
    
    let dx = (d_r - d_l) * 0.5;
    let dy = (d_u - d_d) * 0.5;
    
    // Normal strength. Reduced to prevent overly sharp edges on large tanks
    let normal_strength = 150.0;
    let N = normalize(vec3<f32>(-dx * normal_strength, dy * normal_strength, 1.0));
    return N;
}

// Procedural Noise Functions for Water Surface
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash12(i + vec2<f32>(0.0, 0.0)), hash12(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash12(i + vec2<f32>(0.0, 1.0)), hash12(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y
    );
}

fn fbm(p: vec2<f32>) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var shift = vec2<f32>(100.0);
    // Rotate to reduce axial bias
    let cos_r = 0.87758; // cos(0.5)
    let sin_r = 0.47942; // sin(0.5)
    let rot = mat2x2<f32>(cos_r, sin_r, -sin_r, cos_r);
    var p2 = p;
    for (var i = 0; i < 4; i = i + 1) {
        v += a * noise(p2);
        p2 = rot * p2 * 2.0 + shift;
        a *= 0.5;
    }
    return v;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let depth = textureSample(t_depth, s_depth, in.uv).x;
    
    if (depth >= 1.0) {
        discard;
    }
    
    let thickness = textureSample(t_thickness, s_thickness, in.uv).x;
    let N = compute_normal(in.uv, depth);
    
    // Light and View vectors
    let L = normalize(-scene.sun_direction.xyz);
    let V = vec3<f32>(0.0, 0.0, 1.0); // View vector in screen space approximation
    let H = normalize(L + V);
    
    // Beer-Lambert Volume Absorption (Thickness Tinting)
    // Deep water is dark blue, shallow is light cyan
    let deep_color = vec3<f32>(0.01, 0.1, 0.35);
    let shallow_color = vec3<f32>(0.2, 0.7, 0.85);
    let absorption_coeff = 2.5; 
    let transmittance = exp(-thickness * absorption_coeff);
    
    // The base color of the fluid
    let fluid_color = mix(deep_color, shallow_color, transmittance);
    
    // Refraction
    // Distort UV based on normal XY and thickness
    let refraction_strength = 0.05 * clamp(thickness, 0.0, 1.0);
    let distorted_uv = in.uv + N.xy * refraction_strength;
    let bg_color = textureSample(t_opaque_bg, s_thickness, distorted_uv).rgb;
    
    // Multiply background with our volume absorption color
    // Add some self-illumination (ambient boost) so the fluid is visible even against dark backgrounds
    let refracted_color = bg_color * fluid_color + fluid_color * 0.4;
    
    // Diffuse Lighting
    let diff = max(dot(N, L), 0.0);
    
    // Specular Highlight (Blinn-Phong) broken up by noise
    let time = scene.cascade_params.z;
    let wave_uv = in.uv * 15.0 + N.xy * 2.0 + vec2<f32>(time * 0.3, time * 0.2);
    let surface_noise = fbm(wave_uv);
    
    let shininess = 150.0;
    // Modulate normal with noise for sparkly water
    let N_turbulent = normalize(N + vec3<f32>(surface_noise - 0.5, surface_noise - 0.5, 0.0) * 0.3);
    let spec = pow(max(dot(N_turbulent, H), 0.0), shininess) * 4.0;
    let specular_color = scene.sun_color.rgb * spec;
    
    // Procedural Sky Reflection based on view normal
    let R = reflect(-V, N); 
    // R.y maps to up/down in screen space
    let reflection_factor = clamp(R.y * 0.5 + 0.5, 0.0, 1.0);
    let sky_color = mix(vec3<f32>(0.1, 0.3, 0.6), vec3<f32>(0.8, 0.9, 1.0), reflection_factor);
    
    // Schlick's Fresnel Approximation
    let f0 = 0.08; 
    let cos_theta = max(dot(N, V), 0.0);
    let fresnel = f0 + (1.0 - f0) * pow(1.0 - cos_theta, 5.0);
    
    // Procedural Foam
    // Foam forms where water is agitated (high noise) and edges (low thickness)
    let foam_base = smoothstep(0.5, 0.8, surface_noise);
    let edge_foam = smoothstep(0.8, 0.0, thickness); // more foam in thin areas
    let foam_mask = clamp(foam_base * edge_foam * 2.0, 0.0, 1.0);
    let foam_color = vec3<f32>(0.9, 0.95, 1.0);
    
    // Combine lighting components
    let ambient = 0.5;
    
    // Final Lit Color
    var lit_color = refracted_color + fluid_color * (diff * 0.4 + ambient);
    lit_color += specular_color;
    lit_color += sky_color * fresnel; // Beautiful sky reflection on edges
    lit_color = mix(lit_color, foam_color, foam_mask); // Foam on top
    
    // Alpha blending based on thickness and fresnel (edges are more opaque)
    let alpha = clamp(thickness * 10.0 + fresnel, 0.0, 1.0);
    
    return vec4<f32>(lit_color, alpha);
}
