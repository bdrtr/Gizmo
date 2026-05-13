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

fn get_grid_line(coord: vec2<f32>, scale: f32) -> f32 {
    let grid_coord = coord / scale;
    let derivative = max(fwidth(grid_coord), vec2<f32>(0.00001, 0.00001));
    let grid_abs = abs(fract(grid_coord - 0.5) - 0.5) / derivative;
    let line_val = min(grid_abs.x, grid_abs.y);
    return 1.0 - min(line_val, 1.0);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let coord = in.world_pos.xz;
    let dist_3d = distance(scene.camera_pos.xyz, in.world_pos);
    
    // 3 Kademeli Dinamik Izgara (LOD) - UE5 Stili
    let line_2m = get_grid_line(coord, 2.0);
    let line_10m = get_grid_line(coord, 10.0);
    let line_100m = get_grid_line(coord, 100.0);
    
    // Uzaklığa göre yumuşakça kaybolma (Fade)
    let fade_2m = 1.0 - smoothstep(20.0, 60.0, dist_3d);
    let fade_10m = 1.0 - smoothstep(100.0, 300.0, dist_3d);
    let fade_100m = 1.0 - smoothstep(1000.0, 4000.0, dist_3d);
    
    var alpha = 0.0;
    alpha = max(alpha, line_2m * 0.08 * fade_2m);
    alpha = max(alpha, line_10m * 0.20 * fade_10m);
    alpha = max(alpha, line_100m * 0.35 * fade_100m);
    
    var color = vec3<f32>(0.25, 0.25, 0.25);
    if (line_100m > 0.0 && fade_100m > 0.0) {
        color = vec3<f32>(0.35, 0.35, 0.35); // 100m çizgileri biraz daha belirgin
    } else if (line_10m > 0.0 && fade_10m > 0.0) {
        color = vec3<f32>(0.3, 0.3, 0.3); // 10m çizgileri
    }

    // Eksenleri Çiz (X=Kırmızı, Z=Yeşil)
    let deriv = max(fwidth(coord), vec2<f32>(0.00001, 0.00001));
    let dist_x = abs(coord.x) / deriv.x;
    let dist_y = abs(coord.y) / deriv.y; // coord.y aslında world_Z
    let axis_width = 1.5;
    
    if dist_y < axis_width {
        color = vec3<f32>(0.7, 0.2, 0.25); // X Eksen
        alpha = max(alpha, 0.4 * fade_100m);
    } else if dist_x < axis_width {
        color = vec3<f32>(0.35, 0.6, 0.2); // Z Eksen
        alpha = max(alpha, 0.4 * fade_100m);
    }

    if alpha < 0.01 {
        discard;
    }
    
    return vec4<f32>(color, alpha * in.inst_albedo.a);
}
