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

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>, // Maksimum 64 kemik destegi
};
@group(3) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
};

@group(4) @binding(0)
var<storage, read> instances: array<InstanceRaw>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
    @location(4) joint_indices: vec4<u32>,
    @location(5) joint_weights: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) inst_albedo: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );
    out.inst_albedo = inst.albedo_color;

    let world_pos_4 = model * vec4<f32>(input.position, 1.0);
    out.world_pos = world_pos_4.xyz;
    out.clip_position = scene.view_proj * world_pos_4;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coord = in.world_pos.xz;
    let grid_size = 2.0; // Izgara boşlukları daha geniş
    
    let grid_coord = coord / grid_size;
    let derivative = fwidth(grid_coord);
    let grid_abs = abs(fract(grid_coord - 0.5) - 0.5) / derivative;
    let line_val = min(grid_abs.x, grid_abs.y);
    
    let is_line = 1.0 - min(line_val, 1.0);
    
    let dist_x = abs(coord.x) / derivative.x;
    let dist_y = abs(coord.y) / derivative.y; // coord.y is actually world Z
    
    var color = vec3<f32>(0.25, 0.25, 0.25); // Parlaklığı biraz daha azalttık (çok daha mat)
    let axis_width = 1.5;
    
    // Axes: X (red), Z (green like Blender's Y)
    if dist_y < axis_width {
        color = vec3<f32>(0.7, 0.2, 0.25); // X Axis (Red)
    } else if dist_x < axis_width {
        color = vec3<f32>(0.35, 0.6, 0.2); // Z Axis (Green)
    }
    
    // Major lines every 10 units
    let major_grid_coord = coord / 10.0;
    let major_derivative = fwidth(major_grid_coord);
    let major_grid_abs = abs(fract(major_grid_coord - 0.5) - 0.5) / major_derivative;
    let major_line_val = min(major_grid_abs.x, major_grid_abs.y);
    let is_major_line = 1.0 - min(major_line_val, 1.0);
    
    if (dist_y >= axis_width && dist_x >= axis_width && is_major_line > 0.0) {
        color = vec3<f32>(0.3, 0.3, 0.3); // Ana çizgiler de biraz daha az parlak
    }

    // Yumuşak alfa, parlaklığı azaltmak için iyice kıstık
    let alpha_multiplier = max(is_line * 0.08, is_major_line * 0.25);

    // Fade out distance
    let dist_to_camera = distance(scene.camera_pos.xz, coord);
    let fade = 1.0 - clamp(dist_to_camera / 150.0, 0.0, 1.0);
    
    let final_alpha = alpha_multiplier * fade;
    if (final_alpha < 0.01) {
        discard;
    }
    
    return vec4<f32>(color, final_alpha * in.inst_albedo.a);
}

