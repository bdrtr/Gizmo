struct LightData {
    position: vec4<f32>,
    color: vec4<f32>,
};

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>, // w = light_time (Zaman Degiskeni)
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    lights: array<LightData, 10>,
    light_view_proj: mat4x4<f32>,
    num_lights: u32,
};

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct SkeletonData {
    joints: array<mat4x4<f32>, 64>, // Maksimum 64 kemik destegi
};
@group(3) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceData {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
};

@group(4) @binding(0)
var<storage, read> instances: array<InstanceData>;

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
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo: vec4<f32>,
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

    // Kendi pos'umuzu degistirelim: (Sine Wave Vertex Displacement)
    var pos = input.position;
    let time = scene.camera_pos.w;
    
    // Basit bir dalgalanma efekti (time ve x, z koordinatlarina bagli)
    let wave_x = sin(pos.x * 2.0 + time * 3.0) * 0.1;
    let wave_z = cos(pos.z * 2.0 + time * 3.5) * 0.1;
    let wave_y = sin(pos.x * 1.5 + pos.z * 1.5 + time * 2.0) * 0.15;
    
    pos.y += wave_x + wave_z + wave_y;

    // Normali de asagi yukari saptiralim ki isik kiriliyo gibi gozuksun
    out.normal = normalize(input.normal + vec3<f32>(-wave_x * 2.0, 0.0, -wave_z * 2.0));

    var skin_mat = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 1.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0)
    );
    
    if (input.joint_weights.x + input.joint_weights.y + input.joint_weights.z + input.joint_weights.w > 0.0) {
        skin_mat = 
            input.joint_weights.x * skeleton.joints[input.joint_indices.x] +
            input.joint_weights.y * skeleton.joints[input.joint_indices.y] +
            input.joint_weights.z * skeleton.joints[input.joint_indices.z] +
            input.joint_weights.w * skeleton.joints[input.joint_indices.w];
    }

    let skinned_pos = skin_mat * vec4<f32>(pos, 1.0);
    let world_pos = model * vec4<f32>(skinned_pos.xyz, 1.0);
    let world_normal = (model * vec4<f32>(out.normal, 0.0)).xyz;
    
    out.world_position = world_pos.xyz;
    out.normal = world_normal;
    out.inst_albedo = inst.albedo_color;
    
    out.clip_position = scene.view_proj * world_pos;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let base_color = in.inst_albedo.rgb * tex_color.rgb; // Maviimsi
    
    let N = normalize(in.normal);
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
    
    // Su isik yansimasi (Specular - Shininess)
    let L = normalize(-scene.sun_direction.xyz);
    let reflect_dir = reflect(-L, N);
    let spec = pow(max(dot(view_dir, reflect_dir), 0.0), 32.0); // Suyun parlamasi
    
    let sun_col = scene.sun_color.xyz * scene.sun_color.w;
    
    // Hafifce isigin altina gecir (Ambient)
    let final_color = in.color * base_color + (vec3<f32>(0.2, 0.5, 0.8) * spec * sun_col * 2.0);
    return vec4<f32>(final_color, in.inst_albedo.a * tex_color.a * 0.85); // Azcik saydam
}
