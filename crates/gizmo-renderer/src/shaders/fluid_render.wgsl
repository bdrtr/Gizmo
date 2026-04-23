struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
}
@group(0) @binding(0) var<uniform> scene: SceneUniforms;

struct FluidParticle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    pressure: f32,
    force: vec3<f32>,
    next_index: i32,
}
@group(1) @binding(1) var<storage, read> fluid_particles: array<FluidParticle>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) light_dir: vec3<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    let particle = fluid_particles[instance_index];
    let world_position = particle.position + model.position * 0.35; // Gerçek fizik boyutuna uygun görsel
    out.clip_position = scene.view_proj * vec4<f32>(world_position, 1.0);
    
    // Advanced Water/Foam logic
    let speed = length(particle.velocity);
    let deep_color = vec3<f32>(0.0, 0.15, 0.5); // Deep ocean blue
    let mid_color = vec3<f32>(0.0, 0.6, 0.9);   // Turquoise
    let foam_color = vec3<f32>(0.9, 0.95, 1.0); // Foam/Splash white

    // Mix based on speed
    var final_base_color = deep_color;
    if (speed < 20.0) {
        let t = speed / 20.0;
        final_base_color = mix(deep_color, mid_color, t);
    } else {
        let t = clamp((speed - 20.0) / 30.0, 0.0, 1.0);
        final_base_color = mix(mid_color, foam_color, t);
    }
    
    // High Density also generates foam
    let density_ratio = particle.density / 1000.0;
    if (density_ratio > 1.2) {
        let foam_t = clamp((density_ratio - 1.2) * 2.0, 0.0, 1.0);
        final_base_color = mix(final_base_color, foam_color, foam_t);
    }

    out.color = final_base_color;
    out.normal = model.normal;
    out.light_dir = normalize(scene.sun_direction.xyz);
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let diff = max(dot(normalize(in.normal), in.light_dir), 0.0);
    let ambient = 0.4;
    let final_color = in.color * (diff * 0.6 + ambient);
    return vec4<f32>(final_color, 1.0);
}
