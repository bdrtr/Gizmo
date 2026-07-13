// SceneUniforms from gizmo::common (camera_pos.w carries light_time). No shadow group;
// skeleton/instance from #{SKELETON_GROUP}/#{INSTANCE_GROUP} (3/4 native, 2/3 web).
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct SkeletonData {
    joints: array<mat4x4<f32>, 128>, // Maksimum 64 kemik destegi
};
@group(#{SKELETON_GROUP}) @binding(0)
var<uniform> skeleton: SkeletonData;

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
};

@group(#{INSTANCE_GROUP}) @binding(0)
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
    @location(1) normal: vec3<f32>,
    @location(2) tex_coords: vec2<f32>,
    @location(3) world_position: vec3<f32>,
    @location(4) inst_albedo: vec4<f32>,
};

// ── Gerstner dalga: tek dalganın pozisyon ötelemesi + analitik normal katkısı ──────
struct WaveContrib {
    disp: vec3<f32>,
    nrm: vec3<f32>,
};

fn gerstner_wave(
    dir: vec2<f32>,
    wavelength: f32,
    amplitude: f32,
    steepness: f32,
    speed: f32,
    p0: vec2<f32>,
    t: f32,
) -> WaveContrib {
    let d = normalize(dir);
    let w = 6.28318530718 / max(wavelength, 0.001); // 2π/L (dalga sayısı)
    let phi = speed * w;                             // faz hızı
    let theta = w * dot(d, p0) + phi * t;
    let c = cos(theta);
    let s = sin(theta);
    let wa = w * amplitude;
    var out: WaveContrib;
    // Yatay öteleme (steepness·A) sivri tepe/geniş vadi verir; dikey A·sin.
    out.disp = vec3<f32>(steepness * amplitude * d.x * c, amplitude * s, steepness * amplitude * d.y * c);
    // Analitik normal katkısı (GPU Gems Gerstner); taban (0,1,0) çağıranda eklenir.
    out.nrm = vec3<f32>(-d.x * wa * c, -steepness * wa * s, -d.y * wa * c);
    return out;
}

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

    // ── Gerstner Dalgaları (çoklu-dalga okyanus yüzeyi; sivri tepe + analitik normal) ──
    // ESKİDEN 3 üst-üste sinüs (yalnız dikey) + kaba normal vardı. Gerstner yatay öteleme de
    // yaparak gerçek okyanus tepe/vadi profili verir; normal analitik (ışık doğru kırılır).
    var pos = input.position;
    let time = scene.camera_pos.w;
    let p0 = pos.xz;

    var nrm = vec3<f32>(0.0, 1.0, 0.0);
    // dir, wavelength, amplitude, steepness, speed — büyükten küçüğe dalga hiyerarşisi.
    let w1 = gerstner_wave(vec2<f32>( 1.0,  0.3), 12.0, 0.40, 0.50, 1.2, p0, time);
    let w2 = gerstner_wave(vec2<f32>(-0.7,  1.0),  7.0, 0.22, 0.55, 1.6, p0, time);
    let w3 = gerstner_wave(vec2<f32>( 0.4, -0.9),  3.5, 0.11, 0.60, 2.1, p0, time);
    let w4 = gerstner_wave(vec2<f32>( 1.0, -0.2),  1.8, 0.05, 0.70, 2.8, p0, time);
    pos += w1.disp + w2.disp + w3.disp + w4.disp;
    nrm += w1.nrm + w2.nrm + w3.nrm + w4.nrm;
    out.normal = normalize(nrm);

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
    
    // Physically-inspired water lighting: Diffuse response to sun + ambient light + shiny specular hotspot
    let ambient = vec3<f32>(0.15, 0.2, 0.25);
    let diffuse = max(dot(N, L), 0.0) * sun_col;
    // Fresnel: sığ (grazing) açıda gökyüzü yansıması artar → gerçek su parlaklığı/opaklığı.
    let fresnel = pow(1.0 - max(dot(N, view_dir), 0.0), 5.0);
    let sky_reflect = vec3<f32>(0.45, 0.62, 0.85);
    let water_body = base_color * (ambient + diffuse * 0.6);
    let final_color = water_body + sky_reflect * fresnel * 0.5 + (vec3<f32>(0.7, 0.9, 1.0) * spec * sun_col * 2.0);
    // Grazing açıda su daha yansıtıcı/opak → kenar opaklığı fresnel ile biraz artar.
    let alpha = clamp(in.inst_albedo.a * tex_color.a * (0.7 + fresnel * 0.3), 0.0, 1.0);
    return vec4<f32>(final_color, alpha);
}
