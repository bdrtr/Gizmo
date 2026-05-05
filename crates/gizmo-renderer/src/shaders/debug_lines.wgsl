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
    _align_pad: vec3<u32>,
    _pad_scene: vec3<u32>,
    _end_pad: u32,
}

@group(0) @binding(0)
var<uniform> global: SceneUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = global.view_proj * vec4<f32>(model.position, 1.0);
    out.color = model.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
