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

@group(3) @binding(0)
var t_shadow: texture_depth_2d;

@group(3) @binding(1)
var s_shadow: sampler_comparison;

@group(1) @binding(0)
var<uniform> object: ObjectUniforms;

@group(2) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(2) @binding(1)
var s_diffuse: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tex_coords: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) light_space_pos: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    
    // Objenin Vertex'ini dünya evrenine taşı
    let world_pos = object.model * vec4<f32>(input.position, 1.0);
    out.world_position = world_pos.xyz;
    
    // Objeyi döndürdüğümüzde ışık da onunla dönebilsin diye normal'i de dünyaya göre döndür
    // (Skalalama çok deforme edici değilse düz matris çarpımı yeterlidir, aksi taktirde invert(transpose) gerekir)
    let world_normal = (object.model * vec4<f32>(input.normal, 0.0)).xyz;
    out.normal = world_normal;
    
    // Kameraya yansıt
    out.clip_position = scene.view_proj * world_pos;
    
    // Işık kamerasına yansıt (Gölge Haritası İçin)
    out.light_space_pos = scene.light_view_proj * world_pos;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let N = normalize(in.normal);
    
    // Temel Yüzey Rengi (Albedo Rengi * Texture Rengi)
    let base_color = object.albedo_color.rgb * tex_color.rgb;
    let metallic = clamp(object.metallic, 0.0, 1.0);

    // Eger bu obje 'unlit' (isik yemeyen gokyuzu vs.) ise isiklari es gec ve duz renk bas!
    if (object.unlit > 1.5) {
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
    } else if (object.unlit > 0.5) {
        return vec4<f32>(base_color, object.albedo_color.a * tex_color.a);
    }
    
    let min_roughness = max(object.roughness, 0.05);
    let shininess = 2.0 / (min_roughness * min_roughness) - 2.0;
    let view_dir = normalize(scene.camera_pos.xyz - in.world_position);
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    let ambient = base_color * 0.15;
    
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
        let bias = 0.005; // Yüzey kusurlarını önlemek için bias
        
        var pcf_visibility = 0.0;
        let texel_size = 1.0 / 2048.0; // Texture ebadına göre 1 piksel boyutu
        for (var x = -1; x <= 1; x++) {
            for (var y = -1; y <= 1; y++) {
                let offset = vec2<f32>(f32(x), f32(y)) * texel_size;
                pcf_visibility += textureSampleCompare(
                    t_shadow, s_shadow,
                    shadow_uv + offset,
                    light_ndc.z - bias
                );
            }
        }
        shadow_visibility = pcf_visibility / 9.0;
    }
    
    var total_diffuse = vec3<f32>(0.0);
    var total_specular = vec3<f32>(0.0);

    for (var i = 0u; i < scene.num_lights; i++) {
        let light = scene.lights[i];
        
        let L = normalize(light.position.xyz - in.world_position);
        let diff = max(dot(N, L), 0.1);
        
        let reflect_dir = reflect(-L, N);
        let spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);
        
        let distance = length(light.position.xyz - in.world_position);
        let attenuation = 1.0 / (1.0 + 0.09 * distance + 0.032 * (distance * distance));
        let intensity = light.position.w;

        // Gölgeyi sadece 1. ışığa (Ana Işık) uygula
        var current_shadow_factor = 1.0;
        if (i == 0u) {
            current_shadow_factor = shadow_visibility;
        }

        total_diffuse += base_color * (1.0 - metallic) * diff * light.color.rgb * attenuation * intensity * current_shadow_factor;
        total_specular += f0 * spec * (1.0 - min_roughness) * light.color.rgb * attenuation * intensity * current_shadow_factor;
    }
    
    // Parçaları topla
    let final_color = in.color * (ambient + total_diffuse + total_specular);
    
    return vec4<f32>(final_color, object.albedo_color.a * tex_color.a);
}
