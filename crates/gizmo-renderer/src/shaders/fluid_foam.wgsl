// ═══════════════════════════════════════════════════════════════════════
//  Foam / Spray / Droplet Render Shader
//  Renders classified non-liquid particles as individual billboards
//  Phase 1 = Spray (small, bright, velocity-stretched)
//  Phase 2 = Foam  (medium, white, soft circles on surface)
// ═══════════════════════════════════════════════════════════════════════

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
}
@group(0) @binding(0) var<uniform> scene: SceneUniforms;

struct FluidParticle {
    position: vec3<f32>,
    density: f32,
    velocity: vec3<f32>,
    lambda: f32,
    predicted_position: vec3<f32>,
    phase: u32,
    vorticity: vec3<f32>,
    _pad_vort: f32,
}
@group(1) @binding(1) var<storage, read> fluid_particles: array<FluidParticle>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    
    let particle = fluid_particles[instance_index];
    
    // Only render spray (1) and foam (2) — skip liquid and uninitialized
    if (particle.phase != 1u && particle.phase != 2u) {
        out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0); // Behind far plane
        out.uv = vec2<f32>(0.0);
        out.color = vec4<f32>(0.0);
        return out;
    }
    
    var quad_pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let offset = quad_pos[vertex_index];
    
    let world_pos = particle.position;
    let speed = length(particle.velocity);
    
    // Size and color based on particle type
    var radius: f32;
    var particle_color: vec4<f32>;
    
    if (particle.phase == 1u) {
        // ── SPRAY ── Small bright droplets
        radius = 0.025 + speed * 0.003; // Faster = slightly larger
        let intensity = clamp(speed / 8.0, 0.4, 1.0);
        particle_color = vec4<f32>(0.85, 0.92, 1.0, intensity * 0.7);
    } else {
        // ── FOAM ── Larger soft white patches
        radius = 0.04;
        particle_color = vec4<f32>(0.92, 0.96, 1.0, 0.5);
    }
    
    // Billboard facing camera
    let to_camera = normalize(scene.camera_pos.xyz - world_pos);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(to_camera.y) > 0.999) {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(up, to_camera));
    let billboard_up = cross(to_camera, right);
    
    // Velocity-stretch for spray (elongated along motion direction)
    var stretch_offset = offset;
    if (particle.phase == 1u && speed > 2.0) {
        let vel_dir = normalize(particle.velocity);
        let vel_screen_right = dot(vel_dir, right);
        let vel_screen_up = dot(vel_dir, billboard_up);
        let stretch = min(speed * 0.15, 1.5);
        stretch_offset.x += vel_screen_right * stretch * sign(offset.x);
        stretch_offset.y += vel_screen_up * stretch * sign(offset.y);
    }
    
    let vertex_world_pos = world_pos + (right * stretch_offset.x + billboard_up * stretch_offset.y) * radius;
    out.clip_position = scene.view_proj * vec4<f32>(vertex_world_pos, 1.0);
    out.uv = offset;
    out.color = particle_color;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(in.uv);
    if (dist > 1.0) { discard; }
    
    // Soft Gaussian falloff
    let falloff = exp(-dist * dist * 3.0);
    let alpha = in.color.a * falloff;
    
    // Premultiplied alpha output for additive blending
    return vec4<f32>(in.color.rgb * alpha, alpha);
}
