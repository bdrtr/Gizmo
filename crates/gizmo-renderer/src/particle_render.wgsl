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

struct ParticleInstance {
    @location(0) pos_life: vec4<f32>, // xyz: pos, w: life
    @location(1) vel_maxlife: vec4<f32>, // xyz: vel, w: max_life
    @location(2) color: vec4<f32>,       
    @location(3) sizes: vec4<f32>, // x: start_size, y: end_size
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(4) quad_pos: vec2<f32>,
    instance: ParticleInstance
) -> VertexOutput {
    var out: VertexOutput;

    // Eğer particle ölmüşse veya henüz doğmamışsa, görünmez yap (ekran dışına at)
    if (instance.pos_life.w >= instance.vel_maxlife.w || instance.vel_maxlife.w == 0.0) {
        out.clip_position = vec4<f32>(-999.0, -999.0, -999.0, 1.0);
        out.color = vec4<f32>(0.0);
        return out;
    }

    let progress = clamp(instance.pos_life.w / instance.vel_maxlife.w, 0.0, 1.0);
    // Boyut interpolasyonu
    let current_size = mix(instance.sizes.x, instance.sizes.y, progress);
    
    // Billboard efekti: quad'in kameraya bakmasi icin camera pos ve particle pos ile matris oluştur
    let to_camera = normalize(scene.camera_pos.xyz - instance.pos_life.xyz);
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), to_camera));
    let up = cross(to_camera, right);
    
    // View tabanli quad kosesi hesapla:
    let world_pos = instance.pos_life.xyz 
                    + right * quad_pos.x * current_size
                    + up * quad_pos.y * current_size;
                    
    out.clip_position = scene.view_proj * vec4<f32>(world_pos, 1.0);
    
    // Fade out as it dies
    var alpha = instance.color.a;
    if (progress > 0.8) {
        alpha *= (1.0 - progress) / 0.2;
    }
    
    out.color = vec4<f32>(instance.color.rgb, alpha);
    
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
