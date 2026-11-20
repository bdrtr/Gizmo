// SceneUniforms shared from gizmo::common (composed by load_shader_composed).
#import gizmo::common::{SceneUniforms, LightData, height_fog, light_falloff, rotate_by_quat, spot_cone, spot_cookie_uv, gizmo_cluster_index, scene_environment}

@group(0) @binding(0)
var<uniform> scene: SceneUniforms;

// Group 1: the scene's depth (soft particles, read raw with `textureLoad`), and what a lit particle is
// lit by at its centre: the view's clustered lights and the spots' cookies (`particle_light`).
@group(1) @binding(0)
var scene_depth: texture_depth_2d;
@group(1) @binding(1) var<storage, read> cluster_table: array<vec2<u32>>;
@group(1) @binding(2) var<storage, read> cluster_light_indices: array<u32>;
@group(1) @binding(3) var t_cookie: texture_2d_array<f32>;
@group(1) @binding(4) var s_cookie: sampler;

// Group 2: the flipbook atlas (smoke sprites). cfg.x = frames per edge, cfg.y = flipbook on, cfg.z = lit,
// cfg.w = how much a lit particle throws its local lights forward (`GpuParticleSystem::scatter_anisotropy`).
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
    // A lit particle's light at its centre: the sky's and its local lights' (`particle_light`). The sun is
    // added per pixel, through the puff's spherical normal.
    @location(6) light: vec3<f32>,
};

const PI: f32 = 3.14159265;

// Henyey–Greenstein, normalised so that light scattered evenly is 1.
fn scatter_phase(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4), 1.5);
}

// What light a lit particle at `p` takes, seen from along `to_camera`: the sky's, and each of its
// cluster's local lights' — through a spot's cone and cookie, on a rectangle's lit side, thrown forward
// by the phase — over π, as a matt surface takes it. Once a particle, in the vertex stage: a puff of
// spray or smoke is soft, and a big one drawn over itself many times would pay for every pixel.
fn particle_light(p: vec3<f32>, to_camera: vec3<f32>) -> vec3<f32> {
    let sky = scene_environment(
        vec3<f32>(0.0, 1.0, 0.0), 1.0, scene.sky_zenith, scene.sky_horizon.rgb, scene.sky_ground.rgb,
        scene.environment_preset, scene.environment_preset_b, scene.environment_blend_t, scene.sun_direction,
    );
    let cluster = gizmo_cluster_index(scene.view_proj, scene.camera_pos.xyz, scene.camera_forward.xyz, scene.cluster_dims, scene.cluster_depth, p);
    let range = cluster_table[cluster];
    var total = vec3<f32>(0.0);
    for (var ci = 0u; ci < range.y; ci++) {
        let light = scene.lights[cluster_light_indices[range.x + ci]];
        let kind = u32(light.params.y);
        // A directional light is the sun's, added per pixel.
        if (kind == 2u) {
            continue;
        }
        let from_light = p - light.position.xyz;
        if (kind == 3u && dot(from_light, rotate_by_quat(light.direction, vec3<f32>(0.0, 0.0, -1.0))) <= 0.0) {
            continue;
        }
        let dist = length(from_light);
        var reach = light_falloff(dist, light.color.a).x;
        let travel = from_light / max(dist, 1e-6);
        var colour = light.color.rgb;
        if (kind == 1u) {
            reach *= spot_cone(light, travel);
            if (reach > 0.0 && light.params.w > 0.0) {
                colour *= textureSampleLevel(t_cookie, s_cookie, spot_cookie_uv(light, p), i32(light.params.w) - 1, 0.0).rgb;
            }
        }
        if (reach > 0.0) {
            total += colour * light.position.w * reach * scatter_phase(flipbook_cfg.w, dot(travel, to_camera));
        }
    }
    return sky + total / PI;
}

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
        out.light = vec3<f32>(0.0);
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
    out.light = vec3<f32>(0.0);
    if (flipbook_cfg.z > 0.5) {
        out.light = particle_light(instance.pos_life.xyz, to_camera);
    }

    return out;
}

// Soft-particle fade mesafesi (dünya birimi): parçacık, arkasındaki yüzeye bu kadar
// yaklaşınca yavaşça kaybolur.
const SOFT_FADE: f32 = 0.6;

// The particle's texel: the flipbook's frame for its age, or the soft round, as rgb and alpha — before
// its own tint and fade (`in.color`), lighting and the scene's depth soften it.
fn particle_texel(in: VertexOutput) -> vec4<f32> {
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
    return vec4<f32>(base_rgb, base_alpha);
}

// The particle's depth where it shows at all, for the volumetric march's split
// (`VolumetricState::veil_depth_view`): the air behind it is added ahead of it and shows through it as
// much as it lets through. A texel nearly clear of it is left out, so the air behind is not split at a
// corner of a quad that shows nothing.
@fragment
fn fs_veil(in: VertexOutput) {
    if (in.color.a * particle_texel(in).a < 0.05) {
        discard;
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = particle_texel(in);
    var base_rgb = texel.rgb;
    let base_alpha = texel.a;

    // Lit: the sky's and the local lights' light at the particle's centre (`particle_light`), and the
    // sun's through the billboard taken as a sphere — a spherical normal, half-Lambert — so a puff is
    // bright on its sunlit side and shaded on the other. Over π, as every matt surface's. Off, the
    // particle gives off its own light (sparks, fire).
    if (flipbook_cfg.z > 0.5) {
        let r2 = clamp(dot(in.uv, in.uv), 0.0, 1.0);
        let zc = sqrt(1.0 - r2);
        let to_cam = normalize(scene.camera_pos.xyz - in.world_pos);
        let n = normalize(in.bb_right * in.uv.x + in.bb_up * in.uv.y + to_cam * zc);
        var lightv = in.light;
        if (scene.sun_direction.w > 0.5) {
            let ndl = dot(n, normalize(scene.sun_direction.xyz)) * 0.5 + 0.5; // half-lambert
            lightv += scene.sun_color.rgb * scene.sun_color.w * ndl / PI;
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

    let fog = height_fog(in.world_pos, scene.camera_pos.xyz, scene.fog, scene.fog_shape);
    return vec4<f32>(mix(in.color.rgb * base_rgb, fog.rgb, fog.a), in.color.a * base_alpha * soft_depth);
}
