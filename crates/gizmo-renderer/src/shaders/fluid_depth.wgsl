struct LightData {
    position: vec4<f32>,
    color: vec4<f32>,
    direction: vec4<f32>,
    params: vec4<f32>, // x: range, y: intensity, z: type, w: pad
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
    @location(0) view_pos: vec3<f32>,
    @location(1) sphere_center_view: vec3<f32>,
    @location(2) radius: f32,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    var out: VertexOutput;
    
    // Quad vertices (Triangle strip)
    var quad_pos = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0)
    );
    let offset = quad_pos[vertex_index];
    
    let particle = fluid_particles[instance_index];
    
    // Skip foam/spray particles — they are rendered in a separate pass
    if (particle.phase == 1u || particle.phase == 2u) {
        var skip_out: VertexOutput;
        skip_out.clip_position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        skip_out.view_pos = vec3<f32>(0.0);
        skip_out.sphere_center_view = vec3<f32>(0.0);
        skip_out.radius = 0.0;
        return skip_out;
    }
    
    let radius = 0.20; // Increased from 0.08 so spheres overlap and merge smoothly into a continuous surface
    
    let world_pos = particle.position;
    // Wait, Gizmo-engine view matrix isn't directly exposed in SceneUniforms, it's combined into view_proj.
    // Let's assume view matrix can be derived, or we just do billboard in world space.
    // Actually, screen-aligned billboard:
    let to_camera = normalize(scene.camera_pos.xyz - world_pos);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (abs(to_camera.y) > 0.999) {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(up, to_camera));
    let billboard_up = cross(to_camera, right);
    
    let vertex_world_pos = world_pos + (right * offset.x + billboard_up * offset.y) * radius;
    out.clip_position = scene.view_proj * vec4<f32>(vertex_world_pos, 1.0);
    
    out.view_pos = vertex_world_pos;
    out.sphere_center_view = world_pos;
    out.radius = radius;
    
    return out;
}

struct DepthOutput {
    @builtin(frag_depth) depth: f32,
    @location(0) color: vec4<f32>,
}

@fragment
fn fs_main(in: VertexOutput) -> DepthOutput {
    let ray_origin = scene.camera_pos.xyz;
    let ray_dir = normalize(in.view_pos - ray_origin);
    
    let oc = ray_origin - in.sphere_center_view;
    let b = dot(oc, ray_dir);
    let c = dot(oc, oc) - in.radius * in.radius;
    let h = b * b - c;
    
    if (h < 0.0) {
        discard;
    }
    
    let t0 = -b - sqrt(h);
    let t1 = -b + sqrt(h);
    let t = select(t1, t0, t0 > 0.0);
    if (t < 0.0) { discard; }
    let hit_pos = ray_origin + ray_dir * t;
    let clip_pos = scene.view_proj * vec4<f32>(hit_pos, 1.0);
    let depth_val = clamp(clip_pos.z / clip_pos.w, 0.0, 1.0);
    
    // Accurate analytical world-space normal for the sphere
    let normal = normalize(hit_pos - in.sphere_center_view);
    
    var out: DepthOutput;
    out.depth = depth_val;
    out.color = vec4<f32>(normal, depth_val);
    return out;
}
