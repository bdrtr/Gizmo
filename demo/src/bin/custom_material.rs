//! # Kendi materyalin
//!
//! Motorun sunmadığı bir gölgelendirme yazmak: kendi WGSL'i, kendi boru hattı, motorun geri kalanı
//! aynen yerinde.
//!
//! ## Neden ileri (forward), ve bu neden bir taviz değil
//!
//! Akla gelen ilk tasarım "özel gölgelendirici de G-tamponuna yazsın". Sığmıyor, ve sebebi bir
//! görüş değil bir **sayı**: dört G-tamponu hedefi `max_color_attachment_bytes_per_sample`
//! bütçesini paylaşıyor ve o bütçe **32** (WebGPU'nun garanti ettiği alt sınır). Şu an harcanan:
//!
//! | hedef | biçim | bayt |
//! |-------|-------|------|
//! | albedo + metaliklik | `Rgba8UnormSrgb` | 4 |
//! | normal + pürüz | `Rgba16Float` | 8 |
//! | konum + saçılım/anizotropi | `Rgba16Float` | 8 |
//! | teğet + el | `Rgba16Float` | 8 |
//! | **toplam** | | **28 / 32** |
//!
//! Geriye **4 bayt** kalıyor: tam olarak bir `Rgba8` hedefi, bir `Rgba16Float` değil. Yani özel bir
//! materyal kendi G-tamponu kanalını getiremiyor — ya paylaşılan bütçenin son kırıntısını tek bir
//! özellik için harcayacak, ya da ertelenmiş aydınlatmanın okuduğu bir şeyi kovacak.
//!
//! Bu yüzden özel materyal **kendini ileri ilan ediyor**: kendi boru hattı, kendi gölgelendiricisi,
//! ertelenmiş çözümlemeden sonra, sahnenin geri kalanının yazdığı derinlik tamponuyla. Gölge
//! atabiliyor — gölge geçişi yalnız derinlik yazıyor, bir parçanın nasıl gölgeleneceği umurunda
//! değil.
//!
//! ## Boş bağlama grubu yok — ve verinin gideceği yer grup 1
//!
//! | platform | grup düzeni | istenen sınır | boş |
//! |----------|-------------|---------------|-----|
//! | native | 0 sahne · 1 materyal · 2 gölge · 3 iskelet · 4 instance | 6 | 1 |
//! | web | 0 sahne · 1 materyal · 2 iskelet · 3 instance | 4 | **0** |
//!
//! Kendi grubunu getiren bir tasarım masaüstünde çalışıp tarayıcıda gölgelendirici derlemesini
//! kırardı — iki olası hatadan kötü olanı.
//!
//! Gerçek yer **grup 1**, ve dar değil: materyal bağlama grubu yedi girdi — taban renk, örnekleyici,
//! normal, metalik-pürüz, ışıyan, örtüşme, artı bir uniform tamponu — ve özel bir gölgelendirici
//! hepsine başka anlam yükleyebiliyor. Dört doku ve bir uniform, yeni grup yok, platform ayrımı yok.
//! [`AssetManager::material`] onları dolduruyor; bu demo `params` tamponunu kendi zamanı için
//! kullanıyor.
//!
//! ## Ölçülen: özel materyal kareye ulaşıyor
//!
//! Sahnede üç küre: solda motorun PBR'ı, ortada motorun `Unlit`'i, sağda **kayıtlı özel materyal** —
//! yükseklik bantlarına göre renk değiştiren, kenarında Fresnel parlaması olan bir "hologram".
//! Hiçbiri motorun sunduğu bir görünüm değil.
//!
//! Ölçüldü (2026-08-23, `GIZMO_CM_SLAB=<0|1|2>` ile her küre tek başına, kare 200):
//!
//! | küre | ortalama | parlaklık std | zemindeki gölge |
//! |------|----------|---------------|-----------------|
//! | PBR | 200,97 | 28,89 | 374 piksel |
//! | Unlit | 216,64 | 12,16 | 155 piksel |
//! | özel | 192,97 | **31,05** | **408 piksel** |
//!
//! Özel materyal PBR kadar gölge atıyor (408'e karşı 374), `Unlit`'in üç katı — çünkü
//! `CustomMaterial::casts_shadows` varsayılan olarak açık ve gölge geçişi onu gerçekten okuyor.
//! Bu ayrı bir düzeltme gerektirdi: özel materyal `unlit` olarak yönleniyor (ertelenmiş yoldan
//! çıkmasının yolu bu), ve gölge geçişi `unlit && !baked_lit` olan her şeyi atıyordu. "G-tamponunu
//! atlıyor" ile "katı değil" aynı şey değil, ve bayrak düzeltilmeden `casts_shadows: true` yazan
//! ama hiçbir şey yapmayan bir alan olurdu.
//!
//! ### Ölçülen 2: bantlar bir frekans
//!
//! Ortalama ve standart sapma üç küreyi zayıf ayırıyor; asıl fark **desende**. Kürenin dikey
//! parlaklık profili (satır ortalamaları, Hann penceresi) FFT'den geçirildiğinde:
//!
//! | küre | baskın çevrim | gücü | tayfın payı |
//! |------|---------------|------|-------------|
//! | PBR | 3 | 100,4 | %5,1 |
//! | Unlit | 18 | 7,1 | %1,7 |
//! | özel | **7** | **854,5** | **%13,3** |
//!
//! Unlit düz: baskın çevrim yüksek ve gücü yok, yani yalnız gürültü. PBR'ın 3. çevrimi küresel
//! gölgelemenin kendi eğimi. Özel materyalin 7. çevrimi **PBR'ın gücünün 8,5 katı** ve tayfın
//! sekizde biri — gölgelendiricinin `fract(world_pos.y * 4.0 - t)` bandı.
//!
//! ### Ölçülen 3: kayıtsız id hiçbir şey çizmiyor
//!
//! `GIZMO_CM_DANGLING=1` kayıtlı olmayan bir id atıyor. Ölçüldü: **hiçbir piksel çizilmiyor**.
//!
//! Bu bir tercih ve sebebi var: `routing.rs`'in kendi modül belgesi iki yeteneğin nasıl öldüğünü
//! anlatıyor — bir `_ => 0.0` kolu üzerinden sessizce ertelenmiş PBR'a düşerek. Yanlış gölgelenen
//! bir nesne yanlış bir cevap; kaybolan bir nesne bir soru. Motorun
//! `a_registered_material_draws_and_a_dangling_id_draws_nothing` testi ikisini de kilitliyor, ve
//! düşme kolu geri konduğunda **5770/16384 piksel** sızarak kırmızıya dönüyor.
//!
//! ## Kontroller
//!   * `GIZMO_CM_SLAB=<0|1|2>` — tek küreyi ortada yalnız bırak (ölçüm için)
//!   * `GIZMO_CM_DANGLING=1` — kayıtsız bir id ata: hiçbir şey çizilmemeli
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::MaterialType;
use gizmo::renderer::custom_material::{CustomMaterial, CustomPipelineOptions, MaterialId};
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Özel gölgelendirici.
///
/// Motorun kendi gölgelendiricileriyle aynı iki kolaylığı kullanıyor: `#import` ile `SceneUniforms`
/// (elle yazmak yerine), ve `#{INSTANCE_GROUP}` ile platform başına doğru grup indeksi.
const HOLOGRAM_WGSL: &str = r#"
#import gizmo::common::{SceneUniforms}

@group(0) @binding(0) var<uniform> scene: SceneUniforms;

@group(1) @binding(0) var t_base: texture_2d<f32>;
@group(1) @binding(1) var s_base: sampler;
// Grup 1'in yedinci girdisi: motorun materyal parametreleri. Bu demo yalnız ilk bileşenini
// kullanıyor ve ona "zaman" diyor — grup 1'in verilen anlamdan başka bir anlam taşıyabilmesi
// tam olarak bu materyalin var olma biçimi.
struct MatParams { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32> };
@group(1) @binding(6) var<uniform> params: MatParams;

struct InstanceRaw {
    model_matrix_0: vec4<f32>,
    model_matrix_1: vec4<f32>,
    model_matrix_2: vec4<f32>,
    model_matrix_3: vec4<f32>,
    albedo_color: vec4<f32>,
    pbr: vec4<f32>,
    ambient: vec4<f32>,
    emissive: vec4<f32>,
};
@group(#{INSTANCE_GROUP}) @binding(0) var<storage, read> instances: array<InstanceRaw>;

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
    @location(1) world_normal: vec3<f32>,
    @location(2) tint: vec4<f32>,
};

@vertex
fn vs_main(@builtin(instance_index) idx: u32, input: VertexInput) -> VertexOutput {
    let inst = instances[idx];
    let model = mat4x4<f32>(
        inst.model_matrix_0, inst.model_matrix_1,
        inst.model_matrix_2, inst.model_matrix_3,
    );
    let world = model * vec4<f32>(input.position, 1.0);

    var out: VertexOutput;
    out.world_pos = world.xyz;
    // Ölçek düzgün olduğu için 3x3 yeterli; ters-devrik gerekmiyor.
    out.world_normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    out.tint = inst.albedo_color;
    out.clip_position = scene.view_proj * world;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let t = params.a.x;

    // Yükseklik bantları: motorun hiçbir materyalinde olmayan bir şey.
    let band = fract(in.world_pos.y * 4.0 - t * 0.6);
    let stripe = smoothstep(0.42, 0.5, band) * (1.0 - smoothstep(0.5, 0.58, band));

    // Kenar parlaması: yüzey normali kameradan uzaklaştıkça artıyor.
    let view_dir = normalize(scene.camera_pos.xyz - in.world_pos);
    let fresnel = pow(1.0 - clamp(dot(in.world_normal, view_dir), 0.0, 1.0), 2.5);

    let base = textureSample(t_base, s_base, vec2<f32>(0.5, 0.5)).rgb * in.tint.rgb;
    let glow = base * (0.35 + stripe * 1.6 + fresnel * 2.2);
    return vec4<f32>(glow, 1.0);
}
"#;

/// Ölçüm defteri.
#[derive(Clone, Copy, Default)]
struct Report {
    frame: u32,
    registered: u32,
    /// Atanan id — kayıtsız kipte bunun karşılığı yok.
    id: u32,
}
gizmo::core::impl_component!(Report);

/// Küreye takılacak zaman tamponunu tutar.
struct Timed {
    buffer: wgpu::Buffer,
}
gizmo::core::impl_component!(Timed);

impl Clone for Timed {
    fn clone(&self) -> Self {
        // `Component` `Clone` istiyor ve bir GPU tamponu kopyalanamaz; kaynak olarak tek örnek
        // tutuluyor, yani bu kol hiç koşmuyor.
        unreachable!("Timed is a single resource and is never cloned")
    }
}

fn main() {
    let only: Option<usize> = std::env::var("GIZMO_CM_SLAB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok());
    let dangling = std::env::var("GIZMO_CM_DANGLING").is_ok();

    App::<SimpleSceneState>::new("Gizmo Engine - Custom Material", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let device = &scene.renderer.device;
            let queue = &scene.renderer.queue;

            // 1. Boru hattı. `from_wgsl` grup düzenini, köşe düzenini, hedef biçimini ve derinlik
            //    durumunu motorun sözleşmesinden alıyor — bir oyunun elle doğru kurması zor olan
            //    ve yanlış kurduğunda hiçbirini adlandırmayan kısım.
            let material = CustomMaterial::from_wgsl(
                device,
                &scene.renderer.scene,
                "hologram",
                HOLOGRAM_WGSL,
                CustomPipelineOptions::default(),
            );
            let id = scene.renderer.custom_materials.register(material);

            // 2. Materyal bağlama grubu: `params` bu demonun kendi tamponu.
            let time_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("hologram_params"),
                size: 64, // dört vec4
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let custom_bg = AssetManager::material()
                .params(&time_buf)
                .label("hologram")
                .build(
                    scene.asset_manager,
                    device,
                    queue,
                    &scene.renderer.scene.texture_bind_group_layout,
                );

            let white = scene.asset_manager.create_white_texture(
                device,
                queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let sphere = AssetManager::create_sphere(device, 1.0, 32, 32);

            // Kayıtsız id: kaydedilenden sonrası. Çizim yolu bunu hiçbir şeye düşürmemeli.
            let used_id = if dangling { MaterialId(id.0 + 99) } else { id };

            let spheres: [(f32, MaterialType, std::sync::Arc<wgpu::BindGroup>); 3] = [
                (-3.4, MaterialType::Pbr, white.clone()),
                (0.0, MaterialType::Unlit, white.clone()),
                (3.4, MaterialType::Custom(used_id), custom_bg),
            ];
            for (i, (x, kind, bg)) in spheres.into_iter().enumerate() {
                let x = match only {
                    Some(k) if k == i => 0.0,
                    Some(_) => continue,
                    None => x,
                };
                let mut mat = Material::new(bg).with_pbr(Vec4::new(0.35, 0.75, 0.95, 1.0), 0.4, 0.0);
                mat.material_type = kind;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.6, 0.0)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    mat,
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 26.0),
                Material::new(white).with_pbr(Vec4::new(0.10, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.8,
                ..Default::default()
            });

            scene.world.insert_resource(Timed { buffer: time_buf });
            scene.world.insert_resource(Report {
                frame: 0,
                registered: 1,
                id: used_id.0,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 1.6, 8.4), Vec3::new(0.0, 0.4, 0.0));
        })
        .set_render(|world, _state, encoder, view, renderer, _lt| {
            if let Some(mut r) = world.get_resource::<Report>().map(|r| *r) {
                r.frame += 1;
                if let Some(t) = world.get_resource::<Timed>() {
                    let secs = r.frame as f32 / 60.0;
                    let mut data = [0.0f32; 16];
                    data[0] = secs;
                    renderer
                        .queue
                        .write_buffer(&t.buffer, 0, bytemuck::cast_slice(&data));
                }
                world.insert_resource(r);
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |world, _state, ctx| {
            let r = world.get_resource::<Report>().map(|r| *r).unwrap_or_default();
            gizmo::egui::Area::new("cm".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(440.0);
                        ui.heading("Kendi materyalin");
                        ui.label(format!("kayıtlı materyal: {} · kare {}", r.registered, r.frame));
                        ui.monospace(format!("  MaterialType::Custom(MaterialId({}))", r.id));
                        if dangling {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "kayıtsız id — hiçbir şey çizilmemeli",
                            );
                        }
                        ui.separator();
                        ui.label("G-tamponu 28/32 bayt dolu: 4 bayt kaldı,");
                        ui.label("bir Rgba8 hedefi kadar, bir Rgba16F kadar değil.");
                        ui.label("-> özel materyal ileri (forward) çiziliyor.");
                        ui.separator();
                        ui.label("boş bağlama grubu yok (web'de 4/4 dolu).");
                        ui.label("verinin yeri grup 1: 4 doku + 1 uniform.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
