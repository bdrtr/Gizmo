struct LightData {
    position: vec4<f32>,
    color: vec4<f32>,
};

struct SceneUniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    lights: array<LightData, 10>,
    light_view_proj: mat4x4<f32>,
    num_lights: u32,
};



@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(2) @binding(0)
var t_shadow: texture_depth_2d;

@group(2) @binding(1)
var s_shadow: sampler_comparison;

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
    @location(4) light_space_pos: vec4<f32>,
    @location(5) inst_albedo: vec4<f32>,
    @location(6) inst_pbr: vec4<f32>,
    @location(7) local_normal: vec3<f32>,
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

    // Skinning Matrix (Skeletal Animation)
    // Eger joint_weights'in tamami 0 ise iskelet yok demektir, kimligi (Identity) koru.
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

    // Objenin Vertex'ini dünya evrenine taşı (Model space -> Skin space -> World space)
    let skinned_pos = skin_mat * vec4<f32>(input.position, 1.0);
    let world_pos = model * vec4<f32>(skinned_pos.xyz, 1.0);
    out.world_position = world_pos.xyz;
    
    // Obje veya animasyon döndürüldüğünde ışık da tepki versin
    let skinned_normal = skin_mat * vec4<f32>(input.normal, 0.0);
    let world_normal = (model * vec4<f32>(skinned_normal.xyz, 0.0)).xyz;
    out.normal = world_normal;
    
    out.inst_albedo = inst.albedo_color;
    out.inst_pbr = inst.pbr;

    // Kameraya yansıt
    out.clip_position = scene.view_proj * world_pos;
    out.local_normal = input.normal;
    
    // Işık kamerasına yansıt (Gölge Haritası İçin)
    out.light_space_pos = scene.light_view_proj * world_pos;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let N = normalize(in.normal);
    
    // Temel Yüzey Rengi (Albedo Rengi * Texture Rengi)
    let base_color = in.inst_albedo.rgb * tex_color.rgb;
    let metallic = clamp(in.inst_pbr.y, 0.0, 1.0);

    // Eger bu obje 'unlit' (isik yemeyen gokyuzu vs.) ise isiklari es gec ve duz renk bas!
    if (in.inst_pbr.z > 1.5) {
        let view_dir = normalize(in.world_position - scene.camera_pos.xyz);
        let sky_y = view_dir.y;
        
        let sky_color = vec3<f32>(0.08, 0.28, 0.58); // Koyu Mavi
        let horizon_color = vec3<f32>(0.65, 0.75, 0.85); // Ufuk rengi (Puslu Acik Mavi)
        let ground_color = vec3<f32>(0.15, 0.15, 0.18); // Kara toprak

        var final_bg: vec3<f32>;
        if (sky_y > 0.0) {
            final_bg = mix(horizon_color, sky_color, sky_y);
        } else {
            final_bg = mix(horizon_color, ground_color, -sky_y);
        }
        return vec4<f32>(final_bg, 1.0);
    } else if (in.inst_pbr.z > 0.5) {
        return vec4<f32>(base_color, in.inst_albedo.a * tex_color.a);
    }
    
    let min_roughness = max(in.inst_pbr.x, 0.05);
    let shininess = 2.0 / (min_roughness * min_roughness) - 2.0;
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    let ambient = base_color * 0.4; // 0.15'ten 0.4'e çıkartarak gölgelerin kapkaranlık olmasını engelledik
    
    // --- Gölge Hesaplama (Shadow Mapping with PCF) ---
    var shadow_visibility = 1.0;
    
    // Homojen koordinatlara (NDCs) çevir [-1, 1]
    let light_ndc = in.light_space_pos.xyz / in.light_space_pos.w;
    
    // NDC -> Doku Koordinatları (Orijin sol üst)
    let shadow_uv = vec2<f32>(
        light_ndc.x * 0.5 + 0.5,
        (light_ndc.y * -0.5) + 0.5
    );
    
    // Eğer noktamız ışık kamerasının görüş alanı içerisindeyse
    if (shadow_uv.x >= 0.0 && shadow_uv.x <= 1.0 && shadow_uv.y >= 0.0 && shadow_uv.y <= 1.0 && light_ndc.z <= 1.0) {
        // Yüzey normaline ve ışık açısına bağlı adaptif Bias (Shadow Acne'yi düzeltir)
        let L_dir = normalize(-scene.sun_direction.xyz);
        let current_bias = max(0.001, 0.008 * (1.0 - max(dot(N, L_dir), 0.0)));
        
        var pcf_visibility = 0.0;
        let texel_size = 1.0 / 2048.0; // Texture ebadına göre 1 piksel boyutu
        for (var x = -1; x <= 1; x++) {
            for (var y = -1; y <= 1; y++) {
                let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
                pcf_visibility += textureSampleCompare(
                    t_shadow, s_shadow,
                    shadow_uv + offset,
                    light_ndc.z - current_bias
                );
            }
        }
        shadow_visibility = pcf_visibility / 9.0;
        
        // GÖLGE TESTİ: Gölge Acne'yi (siyahlıkları) kesin olarak teşhis etmek için geçici olarak 1.0 yapıyoruz!
        shadow_visibility = 1.0;
    }
    
    var total_diffuse = vec3<f32>(0.0);
    var total_specular = vec3<f32>(0.0);

    // --- 1. Directional Light (Güneş / Ana Işık) Hesaplaması ---
    if (scene.sun_direction.w > 0.5) { // Eğer güneş açıksa
        // Işık vektörü, güneş ışığına doğru bakan vektördür (yönün tersi)
        let L = normalize(-scene.sun_direction.xyz);
        let diff = max(dot(N, L), 0.0);
        
        var spec = 0.0;
        // Eğer yüzey ışığa dönük değilse, specular highlight oluşmamalı! (Işık sızmasını engeller)
        if (diff > 0.0) {
            let reflect_dir = reflect(-L, N);
            spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);
        }
        
        let intensity = scene.sun_color.w;
        let sun_color = scene.sun_color.rgb;

        // Gölge (shadow_visibility) sadece Güneş ışığını doğrudan maskeler
        total_diffuse += base_color * (1.0 - metallic) * diff * sun_color * intensity * shadow_visibility;
        total_specular += f0 * spec * (1.0 - min_roughness) * sun_color * intensity * shadow_visibility;
    }

    // --- 2. Point Lights Hesaplaması ---
    for (var i = 0u; i < scene.num_lights; i++) {
        let light = scene.lights[i];
        
        let L = normalize(light.position.xyz - in.world_position);
        let diff = max(dot(N, L), 0.0);
        
        var spec = 0.0;
        if (diff > 0.0) {
            let reflect_dir = reflect(-L, N);
            spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);
        }
        
        let distance = length(light.position.xyz - in.world_position);
        let attenuation = 1.0 / (1.0 + 0.09 * distance + 0.032 * (distance * distance));
        let intensity = light.position.w;

        // Noktasal ışıklar şu an için gölge üretmiyor
        total_diffuse += base_color * (1.0 - metallic) * diff * light.color.rgb * attenuation * intensity;
        total_specular += f0 * spec * (1.0 - min_roughness) * light.color.rgb * attenuation * intensity;
    }
    
    // Parçaları topla
    let final_color = in.color * (ambient + total_diffuse + total_specular);
    
    return vec4<f32>(final_color, in.inst_albedo.a * tex_color.a);
}
