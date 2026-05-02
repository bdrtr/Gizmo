struct GlobalUniforms {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient_color: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    time: f32,
    _pad: vec3<f32>,
}

@group(0) @binding(0) var<uniform> global: GlobalUniforms;

struct SoftBodyNodeRender {
    position_mass: vec4<f32>,
    velocity_fixed: vec4<f32>,
    forces: vec4<i32>,
}

@group(1) @binding(0) var<storage, read> nodes: array<SoftBodyNodeRender>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) v_idx: u32
) -> VertexOutput {
    var out: VertexOutput;
    
    let node = nodes[v_idx];
    let pos = node.position_mass.xyz;
    
    out.world_pos = pos;
    out.clip_position = global.view_proj * vec4<f32>(pos, 1.0);
    
    let speed = length(node.velocity_fixed.xyz);
    let base_color = vec3<f32>(0.2, 0.6, 1.0); // Blueish
    let strain_color = vec3<f32>(1.0, 0.2, 0.2); // Redish
    
    out.color = mix(base_color, strain_color, clamp(speed * 0.1, 0.0, 1.0));
    out.normal = vec3<f32>(0.0, 1.0, 0.0); // Dummy, computed in FS
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Flat shading normal computation
    let dx = dpdx(in.world_pos);
    let dy = dpdy(in.world_pos);
    let n = normalize(cross(dx, dy));
    
    let light_dir = normalize(vec3<f32>(0.5, 1.0, 0.3));
    let diffuse = max(dot(n, light_dir), 0.1);
    
    let final_color = in.color * diffuse;
    return vec4<f32>(final_color, 1.0);
}
