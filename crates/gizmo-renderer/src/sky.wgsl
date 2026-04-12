struct LightData {
    position:  vec4<f32>,  // xyz=pos, w=intensity
    color:     vec4<f32>,  // rgb=color, a=radius
    direction: vec4<f32>,  // xyz=dir (spot/directional), w=inner_cutoff_cos
    params:    vec4<f32>,  // x=outer_cutoff_cos, y=light_type (0=point,1=spot,2=dir)
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

// Shadow bg
@group(2) @binding(0) var t_shadow: texture_depth_2d_array;
@group(2) @binding(1) var s_shadow: sampler_comparison;

struct SkeletonData {
    joints: array<mat4x4<f32>, 64>,
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
    @location(0) world_pos: vec3<f32>,
    @location(1) inst_albedo: vec4<f32>, // Renk çarpanı
};

@vertex
fn vs_main(@builtin(instance_index) instance_idx: u32, input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    let inst = instances[instance_idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0,
        inst.model_matrix_1,
        inst.model_matrix_2,
        inst.model_matrix_3,
    );

    let world_pos = model * vec4<f32>(input.position, 1.0);
    
    // Skybox'ı kameranın etrafında sabit tutmak ve derinliğini en arkaya atmak için:
    // Fakat gizmo engine main loop'ta zaten kameraya takılır.
    out.clip_position = scene.view_proj * world_pos;
    // clip_position'da Z'yi .w yaparız böylece Z/W = 1 olur (En uzağa çizilir).
    out.clip_position.z = out.clip_position.w; 
    
    out.world_pos = world_pos.xyz;
    out.inst_albedo = inst.albedo_color;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 1. Ray Yönünü (Bakış Açısı) Hesapla
    let view_dir = normalize(in.world_pos - scene.camera_pos.xyz);
    
    // 2. Çok Şık Atmosferik Gradyan (Zenith ve Horizon)
    let zenith_color = vec3<f32>(0.08, 0.25, 0.6); // Koyu lacivert/mavi tepe noktası
    let horizon_color = vec3<f32>(0.5, 0.7, 0.9) * in.inst_albedo.rgb; // Daha açık ufuk rengi
    let ground_color = vec3<f32>(0.2, 0.2, 0.2); // Zemin rengi
    
    // Y eksenindeki durum (yukarı/aşağı)
    let y = view_dir.y; // -1 (Aşağı) ile 1 (Yukarı)
    
    var sky_color = vec3<f32>(0.0);
    
    if (y >= 0.0) {
        // Gökyüzü: Horizon'dan Zenith'e geçiş
        // pow kullanıp ufuk beyazlığını daraltıyoruz.
        let blend = pow(y, 0.5); 
        sky_color = mix(horizon_color, zenith_color, blend);
    } else {
        // Zemin: Horizon'dan Ground'a geçiş
        let blend = pow(max(-y, 0.0), 0.5);
        sky_color = mix(horizon_color, ground_color, blend);
    }
    
    // 3. Güneş Efekti (Sun Halo)
    // Güneş yönü pozitif ışık kaynağına doğru olan yöndür (Veya tersidir).
    let sun_dir = normalize(scene.sun_direction.xyz);
    let sun_dot = max(dot(view_dir, sun_dir), 0.0);
    
    // Büyük ve yumuşak bir ışık halesi (Rayleigh Scattering simülasyonu)
    let sun_halo = pow(sun_dot, 6.0) * 0.4;
    
    // Küçük ve sivri bir güneş diski (Mie Scattering)
    let sun_disk = pow(sun_dot, 500.0) * 2.5;
    
    let sun_glow_color = scene.sun_color.rgb * (sun_halo + sun_disk);
    
    sky_color += sun_glow_color;
    
    // Tonlama sonrası return (HDR Render pass zaten ACES yapıyor)
    return vec4<f32>(sky_color, 1.0);
}
