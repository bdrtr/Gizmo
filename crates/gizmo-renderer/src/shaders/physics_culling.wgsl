struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
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
    _pad_scene: vec3<u32>,
}

struct SimParams {
    dt: f32,
    _pad1: vec3<f32>,
    gravity: vec3<f32>,
    damping: f32,
    num_boxes: u32,
    num_colliders: u32,
    _pad2: vec2<u32>,
}

struct GpuBox {
    pos_mass: vec4<f32>,
    vel_state: vec4<f32>,
    rotation: vec4<f32>,
    ang_sleep: vec4<f32>,
    color: vec4<f32>,
    extents_pad: vec4<f32>,
}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var<uniform> params: SimParams;
@group(1) @binding(1) var<storage, read> boxes: array<GpuBox>;
@group(1) @binding(2) var<storage, read_write> culled_boxes: array<GpuBox>;
@group(1) @binding(3) var<storage, read_write> indirect: array<atomic<u32>>;

fn rotate_vector_by_quat(v: vec3<f32>, q: vec4<f32>) -> vec3<f32> {
    let u = q.xyz;
    let s = q.w;
    return 2.0 * dot(u, v) * u + (s * s - dot(u, u)) * v + 2.0 * s * cross(u, v);
}

@compute @workgroup_size(256)
fn cull_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let idx = global_id.x;
    if (idx >= params.num_boxes) { return; }

    let box = boxes[idx];
    
    // Extents and Position
    let extents = box.extents_pad.xyz;
    let pos = box.pos_mass.xyz;
    let rot = box.rotation;
    
    // OBB corners
    var corners = array<vec3<f32>, 8>(
        vec3<f32>(-extents.x, -extents.y, -extents.z),
        vec3<f32>( extents.x, -extents.y, -extents.z),
        vec3<f32>(-extents.x,  extents.y, -extents.z),
        vec3<f32>( extents.x,  extents.y, -extents.z),
        vec3<f32>(-extents.x, -extents.y,  extents.z),
        vec3<f32>( extents.x, -extents.y,  extents.z),
        vec3<f32>(-extents.x,  extents.y,  extents.z),
        vec3<f32>( extents.x,  extents.y,  extents.z)
    );
    
    var in_frustum = true;
    
    // Check if ALL corners are outside one of the 6 frustum planes
    var out_left = true;
    var out_right = true;
    var out_bottom = true;
    var out_top = true;
    var out_near = true;
    var out_far = true;
    
    for (var i = 0; i < 8; i++) {
        let rotated_corner = rotate_vector_by_quat(corners[i], rot);
        let world_pos = rotated_corner + pos;
        let clip_pos = scene.view_proj * vec4<f32>(world_pos, 1.0);
        
        let w = clip_pos.w; // or abs(clip_pos.w) ? Actually depending on near plane w might be complex, but usually w > 0.
        // If w < 0, it means it's behind the camera. WGPU uses 0 to 1 for Z.
        // It's safer to just let triangle clipping handle near plane, but checking is good.
        
        // Is it inside?
        if (clip_pos.x >= -w) { out_left = false; }
        if (clip_pos.x <= w) { out_right = false; }
        if (clip_pos.y >= -w) { out_bottom = false; }
        if (clip_pos.y <= w) { out_top = false; }
        if (clip_pos.z >= 0.0) { out_near = false; }
        if (clip_pos.z <= w) { out_far = false; }
    }
    
    if (out_left || out_right || out_bottom || out_top || out_near || out_far) {
        in_frustum = false;
    }
    
    if (in_frustum) {
        // indirect buffer holds: [vertex_count, instance_count, first_index, base_vertex, first_instance]
        // index 1 is instance_count
        let instance_idx = atomicAdd(&indirect[1], 1u);
        culled_boxes[instance_idx] = box;
    }
}
