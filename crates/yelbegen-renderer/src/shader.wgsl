struct EngineUniforms {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_pos: vec4<f32>,
    light_color: vec4<f32>,
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
    
    // Nokta Işık Vektörü (Bize gelen ışık)
    let L = normalize(uniforms.light_pos.xyz - in.world_position);
    
    // Diffuse (Yüzeye çarpan genel aydınlık) - Minimum 0.1 Ambient koyuyoruz karanlık olmasın diye
    let diff = max(dot(N, L), 0.1);
    
    // Specular (Parlaklık - Blinn-Phong/Phong)
    let view_dir = normalize(uniforms.camera_pos.xyz - in.world_position);
    let reflect_dir = reflect(-L, N);
    let spec = pow(max(dot(view_dir, reflect_dir), 0.0), 32.0) * 0.5; // 32 parlaklık çapı, 0.5 şiddet
    
    // Mesafe Kaybı (Distance Attenuation)
    let distance = length(uniforms.light_pos.xyz - in.world_position);
    let attenuation = 1.0 / (1.0 + 0.09 * distance + 0.032 * (distance * distance));
    
    // Parçaları topla
    let ambient = vec3<f32>(0.1) * tex_color.rgb; // Baz karanlık renk
    let diffuse_color = diff * uniforms.light_color.rgb * tex_color.rgb * attenuation;
    let specular_color = spec * uniforms.light_color.rgb * attenuation;
    
    let final_color = in.color * (ambient + diffuse_color + specular_color);
    
    return vec4<f32>(final_color, tex_color.a);
}
