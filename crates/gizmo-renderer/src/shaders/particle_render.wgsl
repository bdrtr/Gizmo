// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

// Group 1: sahne derinlik dokusu (soft particles). textureLoad ile ham derinlik okunur.
@group(1) @binding(0)
var scene_depth: texture_depth_2d;

// Group 2: flipbook/SubUV atlas (duman sprite'ları). cfg.x=kenar-kare sayısı, cfg.y=açık(1/0).
@group(2) @binding(0) var flipbook_tex: texture_2d<f32>;
@group(2) @binding(1) var flipbook_samp: sampler;
@group(2) @binding(2) var<uniform> flipbook_cfg: vec4<f32>;

struct ParticleInstance {
    @location(0) pos_life: vec4<f32>, // xyz: pos, w: life
    @location(1) vel_maxlife: vec4<f32>, // xyz: vel, w: max_life
    @location(2) color: vec4<f32>,       
    @location(3) sizes: vec4<f32>, // x: start_size, y: end_size
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>, // [-1,1]² — yumuşak yuvarlak falloff için
    @location(2) world_pos: vec3<f32>, // soft particles: fragment dünya konumu
    @location(3) progress: f32, // flipbook: ömür ilerlemesi (kare seçimi)
    @location(4) bb_right: vec3<f32>, // billboard sağ ekseni (ışıklandırma: küresel normal)
    @location(5) bb_up: vec3<f32>, // billboard yukarı ekseni
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
        out.uv = vec2<f32>(0.0);
        out.world_pos = vec3<f32>(0.0);
        out.progress = 0.0;
        out.bb_right = vec3<f32>(1.0, 0.0, 0.0);
        out.bb_up = vec3<f32>(0.0, 1.0, 0.0);
        return out;
    }

    let progress = clamp(instance.pos_life.w / instance.vel_maxlife.w, 0.0, 1.0);
    // Boyut interpolasyonu
    let current_size = mix(instance.sizes.x, instance.sizes.y, progress);

    // Kameraya bakan billboard; UZUN ekseni HIZ yönüne hizala → duman ŞERİDİ görünümü.
    let to_camera = normalize(scene.camera_pos.xyz - instance.pos_life.xyz);
    let vel = instance.vel_maxlife.xyz;
    let speed = length(vel);
    // Hızın billboard düzlemine izdüşümü (kameraya paralel bileşeni çıkar).
    var along = vel - to_camera * dot(vel, to_camera);
    var right: vec3<f32>;
    if (length(along) > 1e-3) {
        right = normalize(along);
    } else {
        right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), to_camera));
    }
    let up = normalize(cross(to_camera, right));

    // Hızlandıkça akış boyunca uza (şerit). Enine eksen sabit kalır → ince uzun duman.
    let stretch = clamp(1.0 + speed * 0.04, 1.0, 3.5);
    let world_pos = instance.pos_life.xyz
                    + right * quad_pos.x * current_size * stretch
                    + up * quad_pos.y * current_size;

    out.clip_position = scene.view_proj * vec4<f32>(world_pos, 1.0);

    // Ömür başında fade-in, sonunda fade-out (yumuşak doğuş/ölüm).
    var alpha = instance.color.a;
    if (progress > 0.8) {
        alpha *= (1.0 - progress) / 0.2;
    }
    if (progress < 0.1) {
        alpha *= progress / 0.1;
    }

    out.color = vec4<f32>(instance.color.rgb, alpha);
    out.uv = quad_pos * 2.0;
    out.world_pos = world_pos;
    out.progress = progress;
    out.bb_right = right;
    out.bb_up = up;

    return out;
}

// Soft-particle fade mesafesi (dünya birimi): parçacık, arkasındaki yüzeye bu kadar
// yaklaşınca yavaşça kaybolur.
const SOFT_FADE: f32 = 0.6;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var base_rgb: vec3<f32> = vec3<f32>(1.0);
    var base_alpha: f32;
    if (flipbook_cfg.y > 0.5) {
        // FLIPBOOK/SubUV: ömür ilerlemesinden kare seç, ardışık iki kareyi harmanla → pürüzsüz
        // animasyonlu duman sprite'ı (prosedürel diskten çok daha gerçekçi, baked-sim tarzı).
        let tiles = flipbook_cfg.x;
        let frame_count = tiles * tiles;
        let fpos = clamp(in.progress, 0.0, 0.999) * frame_count;
        let f0 = floor(fpos);
        let f1 = min(f0 + 1.0, frame_count - 1.0);
        let frac = fpos - f0;
        let local = in.uv * 0.5 + 0.5; // [-1,1] -> [0,1]
        let inv = 1.0 / tiles;
        let uv0 = (vec2<f32>(f0 % tiles, floor(f0 * inv)) + local) * inv;
        let uv1 = (vec2<f32>(f1 % tiles, floor(f1 * inv)) + local) * inv;
        let s0 = textureSampleLevel(flipbook_tex, flipbook_samp, uv0, 0.0);
        let s1 = textureSampleLevel(flipbook_tex, flipbook_samp, uv1, 0.0);
        let s = mix(s0, s1, frac);
        base_rgb = s.rgb;
        base_alpha = s.a;
    } else {
        // Prosedürel yumuşak yuvarlak (kıvılcım/toz — flipbook kapalıyken eski davranış).
        base_alpha = smoothstep(1.0, 0.0, length(in.uv));
    }

    // T4 LIT: billboard'u KÜRE gibi ele al → küresel normal + half-lambert + ambient → duman
    // güneşe göre aydınlanır (bir yanı parlak, diğeri gölgeli). Emissive için kapalı.
    if (flipbook_cfg.z > 0.5) {
        let r2 = clamp(dot(in.uv, in.uv), 0.0, 1.0);
        let zc = sqrt(1.0 - r2);
        let to_cam = normalize(scene.camera_pos.xyz - in.world_pos);
        let n = normalize(in.bb_right * in.uv.x + in.bb_up * in.uv.y + to_cam * zc);
        var lightv = vec3<f32>(0.4); // ambient
        if (scene.sun_direction.w > 0.5) {
            let ndl = dot(n, normalize(scene.sun_direction.xyz)) * 0.5 + 0.5; // half-lambert
            lightv += scene.sun_color.rgb * (scene.sun_color.w * 0.55) * ndl;
        }
        base_rgb *= lightv;
    }

    // SOFT PARTICLES: sahne derinliğini oku → dünya konumunu geri-projekte et → parçacık ile
    // arkasındaki yüzey arası mesafeye göre alpha'yı yumuşat (sert geometri kesişimi yok).
    let dims = vec2<f32>(textureDimensions(scene_depth));
    let scene_z = textureLoad(scene_depth, vec2<i32>(in.clip_position.xy), 0);
    let uv = in.clip_position.xy / dims;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let sworld_h = scene.inv_view_proj * vec4<f32>(ndc, scene_z, 1.0);
    let scene_world = sworld_h.xyz / sworld_h.w;
    let d_scene = length(scene_world - scene.camera_pos.xyz);
    let d_frag = length(in.world_pos - scene.camera_pos.xyz);
    let soft_depth = smoothstep(0.0, SOFT_FADE, d_scene - d_frag);

    return vec4<f32>(in.color.rgb * base_rgb, in.color.a * base_alpha * soft_depth);
}
