//! # Kendi materyalin
//!
//! Motorun sunmadığı bir gölgelendirme yazmak: kendi WGSL'i, kendi boru hattı, motorun geri kalanı
//! aynen yerinde.
//!
//! ## İki tür: motorun aydınlattığı bir yüzey, ve oyunun kendi boru hattı
//!
//! **Özel yüzey** (`SurfaceMaterial`, `MaterialType::CustomSurface`): oyun bir parçada yüzeyin *ne
//! olduğunu* yazıyor — rengi, normali, pürüzü, metalikliği, ışıması — bir WGSL fonksiyonunda
//! (`material_surface`), gerisini motor yapıyor. Fonksiyon motorun kendi G-tamponu
//! gölgelendiricisine ve ileri aydınlatma gölgelendiricisine, motorun materyalinin yerine
//! birleştiriliyor: masaüstünde G-tamponuna `Pbr` gibi yazılıyor ve ertelenmiş aydınlatmayı, SSAO'yu,
//! SSR'ı, SSGI'yı, çıkartmaları alıyor; tarayıcıda, telefonda, editörde ve saydam yüzeyde aynı
//! fonksiyonu ileri gölgelendirici aydınlatıyor. Bu demoda sağdaki küre: soğumuş kabuğun
//! çatlaklarından sızan lav.
//!
//! Bu belge 2026-08-23'te bunun olamayacağını yazıyordu: G-tamponu bütçesi (o zaman 28/32 bayt)
//! özel bir materyalin *kendi* kanalını getirmesine yer bırakmıyordu. Doğruydu ve sorun değildi:
//! bir yüzey kendi kanalını değil, her aydınlatılan çizimin doldurduğu kanalları dolduruyor.
//!
//! **Özel boru hattı** (`CustomMaterial`, `MaterialType::Custom`): gölgelendirmenin hepsi oyunun —
//! motor yalnız grupları bağlıyor ve ileri çiziyor, ertelenmiş çözümlemeden sonra, sahnenin geri
//! kalanının yazdığı derinlik tamponuyla. Aydınlatılan bir yüzey olmayan şey için: bu demodaki
//! hologram. Gölge atabiliyor — gölge geçişi yalnız derinlik yazıyor, bir parçanın nasıl
//! gölgeleneceği umurunda değil.
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
//! Sahnede dört küre: solda motorun PBR'ı, sonra motorun `Unlit`'i, sonra **kayıtlı özel boru hattı**
//! — yükseklik bantlarına göre renk değiştiren, kenarında Fresnel parlaması olan bir "hologram" — ve
//! en sağda **özel yüzey**, lav. Ölçümler aşağıda ilk üçün; özel yüzeyin ölçümü motorun testinde
//! (`a_custom_surface_is_drawn_as_the_engines_material_is`): motorun materyalini döndüren bir yüzey
//! `Pbr`'ın karesini bayt bayt çiziyor, opak ve saydam; kırmızıya boyayan bir yüzey G-tamponuna tam
//! o kırmızıyı yazıyor (243, 63, 63).
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
//!   * `GIZMO_CM_SLAB=<0|1|2|3>` — tek küreyi ortada yalnız bırak (ölçüm için)
//!   * `GIZMO_CM_DANGLING=1` — kayıtsız bir id ata: hiçbir şey çizilmemeli
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::renderer::components::MaterialType;
use gizmo::renderer::custom_material::{CustomMaterial, CustomPipelineOptions, MaterialId, SurfaceId};
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
    // `InstanceRaw::optics`: xyz = 1 − specular tint, w = 0xFFFFFF − a glass's volume (8 bits a channel).
    optics: vec4<f32>,
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

/// Özel yüzey: soğumuş lav kabuğu ve çatlaklarından sızan ışık.
///
/// Yalnız yüzeyin ne olduğu burada — renk, pürüz, ışıma. Işığı, gölgesi, SSAO'su ve yansıması
/// motorun, G-tamponu yoluyla, `Pbr`'ınkiler gibi. `standard_surface` motorun materyali; bu fonksiyon
/// ondan başlayıp çoğunu değiştiriyor. Grup 0'ı bildirmesi gerekmiyor: zaman ve kamera
/// `SurfaceInput`'ta geliyor.
const LAVA_WGSL: &str = r#"
#import gizmo::surface::{Surface, SurfaceInput}
#import gizmo::material::{standard_surface}

fn material_surface(input: SurfaceInput) -> Surface {
    var surface = standard_surface(input);
    let p = input.world_position * 2.6;
    let veins = sin(p.x * 1.7 + sin(p.y * 2.3)) * sin(p.z * 1.9 + sin(p.x * 1.1))
        * sin(p.y * 2.9 + input.time * 0.4);
    let crack = 1.0 - smoothstep(0.0, 0.12, abs(veins));
    surface.albedo = mix(vec3<f32>(0.08, 0.07, 0.065), vec3<f32>(0.02, 0.012, 0.01), crack);
    surface.roughness = mix(0.85, 0.35, crack);
    surface.metallic = 0.0;
    surface.emissive = vec3<f32>(4.0, 1.1, 0.15) * crack * (0.75 + 0.25 * sin(input.time * 2.0));
    return surface;
}
"#;

/// Ölçüm defteri.
#[derive(Clone, Copy, Default)]
struct Report {
    frame: u32,
    registered: u32,
    /// Atanan id — kayıtsız kipte bunun karşılığı yok.
    id: u32,
    /// Atanan yüzey id'si, aynı kuralla.
    surface: u32,
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
            // 0. Yüzey: motorun gölgelendiricilerine birleştirilip derleniyor. WGSL'de bir hata
            //    `SurfaceError` olarak dönüyor, gölgelendirici derleyicisinin iletisiyle.
            let surface = scene
                .renderer
                .add_surface_material("lava", LAVA_WGSL)
                .unwrap_or_else(|e| panic!("{e}"));

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
            let used_surface = if dangling { SurfaceId(surface.0 + 99) } else { surface };

            let spheres: [(f32, MaterialType, std::sync::Arc<wgpu::BindGroup>); 4] = [
                (-5.1, MaterialType::Pbr, white.clone()),
                (-1.7, MaterialType::Unlit, white.clone()),
                (1.7, MaterialType::Custom(used_id), custom_bg),
                (5.1, MaterialType::CustomSurface(used_surface), white.clone()),
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
                registered: 2,
                id: used_id.0,
                surface: used_surface.0,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 1.9, 11.0), Vec3::new(0.0, 0.4, 0.0));
        })
        // Kare sayacı oyun mantığı: update'te, basit sahnenin kancasının ardından.
        .add_update_hook(|world, _state, _dt, _input| {
            if let Some(mut r) = world.get_resource_mut::<Report>() {
                r.frame += 1;
            }
        })
        // Zamanı `Timed` tamponuna yüklemek renderer'ın işi: her senkron noktasında, kare
        // hazırlanmadan önce, sayaç oyunun kaynağından okunarak.
        .set_render_stage(RenderWorld::new().with_settings(|game, renderer| {
            let (Some(r), Some(t)) = (game.get_resource::<Report>(), game.get_resource::<Timed>()) else {
                return;
            };
            let secs = r.frame as f32 / 60.0;
            let mut data = [0.0f32; 16];
            data[0] = secs;
            renderer
                .queue
                .write_buffer(&t.buffer, 0, bytemuck::cast_slice(&data));
        }))
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
                        ui.monospace(format!("  MaterialType::CustomSurface(SurfaceId({}))", r.surface));
                        if dangling {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "kayıtsız id — hiçbir şey çizilmemeli",
                            );
                        }
                        ui.separator();
                        ui.label("hologram: oyunun kendi boru hattı, ileri çiziliyor.");
                        ui.label("lav: oyunun yüzeyi, motorun G-tamponuna yazılıyor —");
                        ui.label("ışığı, gölgesi, SSAO'su ve yansıması motorun.");
                        ui.separator();
                        ui.label("boş bağlama grubu yok (web'de 4/4 dolu).");
                        ui.label("verinin yeri grup 1: 4 doku + 1 uniform.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
