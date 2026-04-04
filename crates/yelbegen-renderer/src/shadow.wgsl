struct LightData {
    position: vec4<f32>,
    color: vec4<f32>,
};

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    lights: array<LightData, 10>,
    num_lights: u32,
    light_view_proj: mat4x4<f32>,
};

struct ObjectUniforms {
    model: mat4x4<f32>,
    albedo_color: vec4<f32>,
    roughness: f32,
    metallic: f32,
    unlit: f32,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var<uniform> object: ObjectUniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> @builtin(position) vec4<f32> {
    let world_pos = object.model * vec4<f32>(input.position, 1.0);
    return scene.light_view_proj * world_pos;
}
