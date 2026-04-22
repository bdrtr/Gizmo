// Uses the same `SceneUniforms` / global buffer layout as the main scene (binding 0 only).

struct LightData {
    position:  vec4<f32>,
    color:     vec4<f32>,
    direction: vec4<f32>,
    params:    vec4<f32>,
};

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
};

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
}

struct GpuBox {
    @location(6) pos_mass: vec4<f32>,
    @location(7) vel_state: vec4<f32>,
    @location(8) rotation: vec4<f32>,
    @location(9) ang_sleep: vec4<f32>,
    @location(10) color: vec4<f32>,
    @location(11) extents_pad: vec4<f32>,
}

fn rotate_vector_by_quat(v: vec3<f32>, q: vec4<f32>) -> vec3<f32> {
    let u = q.xyz;
    let s = q.w;
    return 2.0 * dot(u, v) * u
         + (s * s - dot(u, u)) * v
         + 2.0 * s * cross(u, v);
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
    instance: GpuBox,
) -> VertexOutput {
    var out: VertexOutput;
    
    let half_extents = instance.extents_pad.xyz;
    let scaled_pos = model.position * half_extents;
    let rotated_pos = rotate_vector_by_quat(scaled_pos, instance.rotation);
    let world_pos = rotated_pos + instance.pos_mass.xyz;
    out.world_position = world_pos;
    out.clip_position = scene.view_proj * vec4<f32>(world_pos, 1.0);
    
    let state = bitcast<u32>(instance.vel_state.w);
    
    if (state == 1u) {
        // Sleep state: Darken the color
        out.color = vec4<f32>(instance.color.xyz * 0.2, 1.0);
    } else {
        out.color = instance.color;
    }
    
    out.normal = rotate_vector_by_quat(model.normal, instance.rotation);
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var N = in.normal;
    if (length(N) < 0.001) { N = vec3<f32>(0.0, 1.0, 0.0); }
    N = normalize(N);
    
    let base_color = in.color.rgb;
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
    
    let ambient = base_color * 0.3;
    var diffuse = vec3<f32>(0.0);
    var specular = vec3<f32>(0.0);
    
    if (scene.sun_direction.w > 0.5) {
        let L = normalize(-scene.sun_direction.xyz);
        let diff = max(dot(N, L), 0.0);
        
        let reflect_dir = reflect(-L, N);
        let spec = pow(max(dot(view_dir, reflect_dir), 0.0), 32.0);
        
        diffuse = base_color * diff * scene.sun_color.rgb;
        specular = vec3<f32>(0.3) * spec * scene.sun_color.rgb;
    }
    
    let final_color = ambient + diffuse + specular;
    return vec4<f32>(final_color, in.color.a);
}
