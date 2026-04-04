struct EngineUniforms {
    mvp: mat4x4<f32>,
    light_dir: vec4<f32>,
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
    @location(1) normal: vec3<f32>, // Interpolated for fragment shader
    @location(2) tex_coords: vec2<f32>,
};

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.color = model.color;
    // Normal vektörlerini (Yüzey Yönünü) Fragment shader'a yolluyoruz 
    out.normal = model.normal;
    out.tex_coords = model.tex_coords;
    
    out.clip_position = uniforms.mvp * vec4<f32>(model.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Dokuyu Koordinata göre Oku
    let tex_color = textureSample(t_diffuse, s_diffuse, in.tex_coords);

    // Normali sabitle (Bazen boyutlandırılırken yozlaşabilir)
    let n = normalize(in.normal);
    
    // Işığın geliş yönü (Directional Light / Güneş)
    let l = normalize(uniforms.light_dir.xyz);
    
    // Yüzey normali ile Işık vektörünün kesişim açısı. (Karanlık için min 0.1 Ambient koyduk)
    let diffuse_factor = max(dot(n, l), 0.15); 

    // Orijinal renk * Doku Rengi * Cisim Yüzeyine çarpan Işık 
    let final_color = in.color * tex_color.rgb * diffuse_factor;
    return vec4<f32>(final_color, tex_color.a);
}
