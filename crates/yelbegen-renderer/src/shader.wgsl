struct EngineUniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_pos: vec4<f32>,
    light_color: vec4<f32>,
    albedo_color: vec4<f32>,
    roughness: f32,
    metallic: f32,
    unlit: f32,
    _padding: f32,
};

@group(0) @binding(0)
var<uniform> uniforms: EngineUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
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
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = input.color;
    out.tex_coords = input.tex_coords;
    
    // Objenin Vertex'ini dünya evrenine taşı
    let world_pos = uniforms.model * vec4<f32>(input.position, 1.0);
    out.world_position = world_pos.xyz;
    
    // Objeyi döndürdüğümüzde ışık da onunla dönebilsin diye normal'i de dünyaya göre döndür
    // (Skalalama çok deforme edici değilse düz matris çarpımı yeterlidir, aksi taktirde invert(transpose) gerekir)
    let world_normal = (uniforms.model * vec4<f32>(input.normal, 0.0)).xyz;
    out.normal = world_normal;
    
    // Kameraya yansıt
    out.clip_position = uniforms.view_proj * world_pos;
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let N = normalize(in.normal);
    
    // Temel Yüzey Rengi
    // (Henüz tam texture desteğimiz olmadığı için ve köşe pikselleri siyah olabildiği için geçici olarak sadece Albedo kullanıyoruz)
    let base_color = uniforms.albedo_color.rgb; // * tex_color.rgb kaldirildi
    let metallic = clamp(uniforms.metallic, 0.0, 1.0);

    // Eger bu obje 'unlit' (isik yemeyen gokyuzu vs.) ise isiklari es gec ve duz renk bas!
    if (uniforms.unlit > 0.5) {
        return vec4<f32>(base_color, uniforms.albedo_color.a * tex_color.a);
    }
    
    // Nokta Işık Vektörü (Bize gelen ışık)
    let L = normalize(uniforms.light_pos.xyz - in.world_position);
    
    // Yüzey normali ile açıya göre Diffuse
    let diff = max(dot(N, L), 0.1);
    
    // Roughness'tan (0.0 ile 1.0 arası) Shininess çıkarma (Düşük roughness = keskin parlama)
    let min_roughness = max(uniforms.roughness, 0.05);
    let shininess = 2.0 / (min_roughness * min_roughness) - 2.0;

    // Specular (Blinn-Phong)
    let view_dir = normalize(uniforms.camera_pos.xyz - in.world_position);
    let reflect_dir = reflect(-L, N);
    let spec = pow(max(dot(view_dir, reflect_dir), 0.0), shininess);
    
    // Mesafe Kaybı (Distance Attenuation)
    let distance = length(uniforms.light_pos.xyz - in.world_position);
    let attenuation = 1.0 / (1.0 + 0.09 * distance + 0.032 * (distance * distance));
    
    // Metal yüzeyler kendi renginde parlar (f0 tespiti) ve mat kısımları emer (diffuse azalır)
    let f0 = mix(vec3<f32>(0.04), base_color, metallic);
    
    // Ambient
    let ambient = base_color * 0.15; // Gölgeler kör zifiri karanlık olmasın diye 0.1'den 0.15'e çıkarıldı
    
    // Işık Şiddeti (Intensity), light_color.w üzerinden geliyor
    let intensity = uniforms.light_color.w;

    // Aydınlatma renklerini parçalama
    let diffuse_color = base_color * (1.0 - metallic) * diff * uniforms.light_color.rgb * attenuation * intensity;
    let specular_color = f0 * spec * (1.0 - min_roughness) * uniforms.light_color.rgb * attenuation * intensity;
    
    // Parçaları topla
    let final_color = in.color * (ambient + diffuse_color + specular_color);
    
    return vec4<f32>(final_color, uniforms.albedo_color.a * tex_color.a);
}
