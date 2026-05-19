// Point Light Shadow Pass (writes to a cubemap face)
struct Uniforms {
    view_proj: mat4x4<f32>,
    light_pos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: Uniforms;

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color:   vec4<f32>,
    pbr:            vec4<f32>,
};

@group(1) @binding(0) var<storage, read> instances: array<InstanceData>;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> @builtin(position) vec4<f32> {
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0, inst.model_matrix_1,
        inst.model_matrix_2, inst.model_matrix_3,
    );
    let world_pos = model * vec4<f32>(input.position, 1.0);
    return ubo.view_proj * world_pos;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) {
    // Depth is automatically written to the depth attachment.
}
