//! # Kareye son rötuş
//!
//! Kareye çekilen son rötuş: köşe karartma, renk sapması, film grenli.
//!
//! ## Envanter
//!
//! | yetenek | Gizmo'nun [`PostProcess`]'i |
//! |---------------------------|------------------------------|
//! | pozlama, bölge bölge | **kamerada** — `Camera::exposure`, tek sayı |
//! | gamma, ön ve son doygunluk (üç bölge için ayrı ayrı) | **yok** |
//! | renk tonu, kontrast, lift, gain | **yok** |
//! | renk dengesi (sıcaklık/ton) | **yok** |
//! | köşe karartma | `vignette` |
//! | kanal başına radyal kayma | `chromatic_aberration` |
//! | film greni | `film_grain` |
//! | hale ve alan derinliği | `bloom_intensity`, `bloom_threshold`, üç DOF sayısı |
//!
//! Yani motorunki bir **renk derecelendirme** aracı (bölge bölge ton eğrisi) değil, bir **efekt
//! paketi**. Renk derecelendirmenin kendisi — gamma, doygunluk, kontrast, lift/gain, renk
//! dengesi — motorda hiç yok.
//!
//! ## Ölçülen bir asimetri — ve kapatılışı
//!
//! Post-işlem ayarları iki kaynaktan geliyor (bkz. `depth_of_field`): kameranın taşıdığı
//! [`PostProcess`] bileşeni, yoksa `Renderer`'ın alanları. Bu demo yazıldığında iki liste **aynı
//! değildi**: `Renderer`'da `vignette` alanı **hiç yoktu**, yedek dal onu yazmıyordu, ve
//! `PostProcessUniforms::default()`'un 0,25'i sızıyordu. Yani bileşen takmayan bir oyun köşe
//! karartmasını ne açabiliyor ne kapatabiliyordu.
//!
//! **2026-08-23'te kapatıldı:** `Renderer::vignette_intensity` eklendi ve yedek dal onu okuyor.
//! Varsayılan yine 0,25 — yani hiçbir piksel değişmedi, yalnız değer ulaşılabilir oldu.
//!
//! ## Ölçüldü (2026-08-23, kare 240, düz duvar, bloom kapalı, hepsi TEK partide)
//!
//! | durum | köşe/merkez | gren (std) | kenarda \|R−B\| tepesi | eşiği aşan piksel |
//! |-------|-------------|------------|----------------------|-------------------|
//! | bileşen: `vignette = 0` | **1,095** | 0,40 | 10 | 0 |
//! | bileşen: `vignette = 0,9` | **0,608** | — | 10 | 0 |
//! | bileşen: `chromatic_aberration = 0,006` | 1,095 | 0,40 | **34** | **1343** |
//! | bileşen: `film_grain = 0,12` | 1,094 | **3,23** | 10 | 0 |
//! | bileşensiz: `renderer = 0,0` | **1,095** | — | 10 | 0 |
//! | bileşensiz: `renderer = 0,25` | 1,001 | — | 10 | 0 |
//! | bileşensiz: `renderer = 0,9` | **0,608** | — | 10 | 0 |
//!
//! Üç efekt de çalışıyor: karartma 1,095'ten 0,608'e, gren sapması 0,40'tan 3,23'e (8 kat), renk
//! sapması kenarda `|R−B|`'yi 10'dan 34'e çıkarıyor ve nötr karede eşiği aşan hiç piksel yokken
//! burada 1343 tane var.
//!
//! **Ve simetri artık tam:** bileşenle `vignette = 0` ile bileşensiz `renderer = 0,0` **aynı**
//! sayıyı veriyor (1,095), `0,9` de öyle (0,608). İki yol aynı ayarda aynı kareyi üretiyor.
//!
//! ### Ölçmenin kuralı, üçüncü kez
//!
//! İlk tablo iki ayrı partiden derlenmişti ve "bileşensiz 1,206" ile "renderer=0,25 → 1,001"
//! çelişiyordu. Sebep motor değildi: **pencere boyutu koşular arasında değişmişti** (1028'e karşı
//! 492 piksel), yani sabit piksel kutuları sahnenin başka yerine düşüyordu. Yukarıdaki tablonun
//! tamamı tek boyutta, ve ölçüm betiği boyutları önce denetliyor.
//!
//! ## Kontroller
//!   * **1** nötr · **2** köşe karartma · **3** renk sapması · **4** gren · **5** hepsi
//!   * **0** — bileşeni çıkar (yedek yola düş)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::renderer::components::PostProcess;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi rötuşun açık olduğu.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// Bileşen takılı ama her şey nötr.
    Neutral,
    Vignette,
    Chromatic,
    Grain,
    All,
    /// Bileşen yok — `Renderer` yedek yolu.
    NoComponent,
}
gizmo::core::impl_component!(Look);

impl Look {
    /// Başsız koşuda tuşa basacak kimse yok.
    fn from_env() -> Self {
        match std::env::var("GIZMO_GRADE").as_deref() {
            Ok("vignette") => Self::Vignette,
            Ok("chromatic") => Self::Chromatic,
            Ok("grain") => Self::Grain,
            Ok("all") => Self::All,
            Ok("none") => Self::NoComponent,
            _ => Self::Neutral,
        }
    }

    /// Bu bakışın istediği ayarlar.
    fn settings(self) -> PostProcess {
        let mut p = PostProcess::new();
        // Bloom bilerek kapalı: ölçülen şey rötuş, rötuş + hale değil.
        p.bloom_intensity = 0.0;
        match self {
            Self::Vignette => p.vignette = 0.9,
            Self::Chromatic => p.chromatic_aberration = 0.006,
            Self::Grain => p.film_grain = 0.12,
            Self::All => {
                p.vignette = 0.9;
                p.chromatic_aberration = 0.006;
                p.film_grain = 0.12;
            }
            _ => p.vignette = 0.0,
        }
        p
    }
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Color Grading", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Düz, aydınlık bir duvar: köşe karartması ve gren ancak düz bir alanda ölçülür.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -6.0)).with_scale(Vec3::splat(30.0)),
                GlobalTransform::default(),
                AssetManager::create_sprite_quad(device, 1.0, 1.0),
                Material::new(white.clone()).with_unlit(Vec4::new(0.55, 0.55, 0.58, 1.0)),
                MeshRenderer::new(),
            ));
            // Renk sapması kenarlarda okunur: duvarın üstünde sert kenarlı çubuklar.
            let cube = AssetManager::create_cube(device);
            for i in -3..=3 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(i as f32 * 1.6, 0.0, -4.0))
                        .with_scale(Vec3::new(0.12, 2.2, 0.12)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_unlit(Vec4::new(0.05, 0.05, 0.06, 1.0)),
                    MeshRenderer::new(),
                ));
            }

            scene.world.insert_resource(Look::from_env());
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, -6.0));
        })
        .add_update_system(grade_input.in_phase(Phase::Update))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // Yedek yolun karşılaştırılabilir olması için: bileşen yokken bloom da kapalı olsun.
            renderer.bloom_intensity = 0.0;
            // Bileşensiz yolda köşe karartması artık ULAŞILABİLİR (2026-08-23'te eklendi).
            // `GIZMO_VIGNETTE` verilmezse motorun kendi varsayılanı (0,25) korunuyor.
            if let Ok(v) = std::env::var("GIZMO_VIGNETTE") {
                if let Ok(v) = v.parse::<f32>() {
                    renderer.vignette_intensity = v;
                }
            }

            apply_look(world);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let look = world.get_resource::<Look>().map(|l| *l).unwrap_or(Look::Neutral);
            gizmo::egui::Area::new("grade".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(460.0);
                        ui.heading("Son rötuş");
                        ui.label(match look {
                            Look::Neutral => "bileşen takılı, her şey nötr",
                            Look::Vignette => "köşe karartma 0,90",
                            Look::Chromatic => "renk sapması 0,006",
                            Look::Grain => "film greni 0,12",
                            Look::All => "üçü birden",
                            Look::NoComponent => "BİLEŞEN YOK — Renderer yedek yolu",
                        });
                        ui.separator();
                        ui.label("Renderer'da vignette ALANI YOK: bileşen takmayan bir oyun");
                        ui.label("köşe karartmasını ne açabilir ne kapatabilir (sabit 0,25).");
                        ui.separator();
                        ui.label("renk derecelendirmesi (gamma, doygunluk, kontrast,");
                        ui.label("lift/gain, renk dengesi) motorda hiç yok.");
                        ui.separator();
                        ui.label("1 nötr · 2 köşe · 3 sapma · 4 gren · 5 hepsi · 0 bileşensiz");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Yalnız tuşları okur; bileşeni takıp çıkarmak yapısal bir değişiklik, o `set_render`'de.
fn grade_input(mut look: ResMut<Look>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    for (key, value) in [
        (KeyCode::Digit1, Look::Neutral),
        (KeyCode::Digit2, Look::Vignette),
        (KeyCode::Digit3, Look::Chromatic),
        (KeyCode::Digit4, Look::Grain),
        (KeyCode::Digit5, Look::All),
        (KeyCode::Digit0, Look::NoComponent),
    ] {
        if input.is_key_just_pressed(key as u32) {
            *look = value;
        }
    }
}

/// Kameranın [`PostProcess`] bileşenini seçilen bakışa göre takar ya da çıkarır.
///
/// Bileşenin **varlığı** anlamlı: yoksa motor `Renderer`'ın alanlarına düşüyor, ve o yolun listesi
/// bileşeninkiyle aynı değil — `vignette` orada hiç yok.
fn apply_look(world: &mut gizmo::core::World) {
    let Some(look) = world.get_resource::<Look>().map(|l| *l) else {
        return;
    };
    let camera = world
        .query::<(&Camera, &Transform)>()
        .and_then(|q| {
            q.iter()
                .find(|(_, (c, _))| c.primary)
                .map(|(entity, _)| entity)
        })
        .and_then(|id| world.get_entity(id));
    let Some(camera) = camera else { return };

    if look == Look::NoComponent {
        world.remove_component::<PostProcess>(camera);
    } else {
        world.add_component(camera, look.settings());
    }
}
