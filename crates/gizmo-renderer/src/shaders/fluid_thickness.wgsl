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
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    
    var quad_pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let offset = quad_pos[vertex_index];
    
    let particle = fluid_particles[instance_index];
    
    // Skip foam/spray particles
    if (particle.phase == 1u || particle.phase == 2u) {
        var skip_out: VertexOutput;
        skip_out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        skip_out.uv = vec2<f32>(0.0);
        return skip_out;
    }
    
    let world_pos = particle.position;
    let radius = 0.20;
    let to_camera = normalize(scene.camera_pos.xyz - world_pos);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(to_camera.y) > 0.999) {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(up, to_camera));
    let billboard_up = cross(to_camera, right);
    
    let vertex_world_pos = world_pos + (right * offset.x + billboard_up * offset.y) * radius;
    out.clip_position = scene.view_proj * vec4<f32>(vertex_world_pos, 1.0);
    out.uv = offset;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = length(in.uv);
    if (dist > 1.0) { discard; }
    
    // Soft Gaussian falloff
    let thickness = exp(-dist * dist * 4.0) * 0.05; 
    return vec4<f32>(thickness, 0.0, 0.0, thickness);
}
