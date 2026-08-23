//! # Bevy'nin `3d/color_grading` örneğinin Gizmo Engine portu
//!
//! Kareye çekilen son rötuş: köşe karartma, renk sapması, film grenli. Bevy'nin `ColorGrading`'i
//! gölge/orta ton/vurgu bölgelerini ayrı ayrı ele alıyor; Gizmo'nunki farklı bir kümeye bakıyor.
//!
//! ## İki farklı liste
//!
//! | Bevy'nin `ColorGrading`'i | Gizmo'nun [`PostProcess`]'i |
//! |---------------------------|------------------------------|
//! | `exposure` (bölge bölge) | **kamerada** — `Camera::exposure`, tek sayı |
//! | `gamma`, `pre_saturation`, `post_saturation` (üç bölge için ayrı ayrı) | **yok** |
//! | `hue`, `contrast`, `lift`, `gain` | **yok** |
//! | `balance` (renk sıcaklığı/tonu) | **yok** |
//! | — | `vignette` (köşe karartma) |
//! | — | `chromatic_aberration` (kanal başına radyal kayma) |
//! | — | `film_grain` |
//! | — | `bloom_intensity`, `bloom_threshold`, üç DOF sayısı |
//!
//! Yani ikisi aynı işi yapmıyor: Bevy'ninki bir **renk derecelendirme** aracı (bölge bölge ton
//! eğrisi), Gizmo'nunki bir **efekt paketi**. Renk derecelendirmenin kendisi — gamma, doygunluk,
//! kontrast, lift/gain, renk dengesi — motorda hiç yok.
//!
//! ## Ölçülen bir asimetri: `vignette`'e yalnız bileşenden ulaşılıyor
//!
//! Post-işlem ayarları iki kaynaktan geliyor (bkz. `bevy_depth_of_field`): kameranın taşıdığı
//! [`PostProcess`] bileşeni, yoksa `Renderer`'ın alanları. Ama iki liste **aynı değil**:
//!
//!   * `Renderer`'da `chromatic_aberration` ve `film_grain_intensity` var, ikisi de okunuyor.
//!   * `Renderer`'da **`vignette` alanı hiç yok**. Yedek dalda o alan yazılmıyor, yani
//!     `PostProcessUniforms::default()`'un 0,25'i kalıyor.
//!
//! Sonuç: bileşen takmayan bir oyun köşe karartmasını **ne açabiliyor ne kapatabiliyor** — sabit
//! 0,25'e mahkûm. Demo ikisini yan yana ölçüyor.
//!
//! Ölçüldü (2026-08-23, kare 240, düz gri bir duvar, bloom kapalı):
//!
//! | bakış | köşe/merkez | gren (std) | kenarda \|R−B\| tepesi |
//! |-------|-------------|------------|----------------------|
//! | bileşen takılı, `vignette = 0` | **1,344** | 0,43 | 10 |
//! | `vignette = 0,9` | **0,635** | 0,84 | 10 |
//! | `chromatic_aberration = 0,006` | 1,344 | 0,43 | **40** (720 piksel eşiği aşıyor) |
//! | `film_grain = 0,12` | 1,343 | **3,22** | 10 |
//! | **bileşen YOK** (yedek yol) | **1,206** | 0,43 | 10 |
//!
//! Üç efekt de çalışıyor: köşe karartma oranı 1,344'ten 0,635'e (%53 karartma), gren sapması
//! 0,43'ten 3,22'ye (7,5 kat), renk sapması kenarda `|R−B|`'yi 10'dan 40'a çıkarıyor ve nötr
//! karede eşiği aşan hiç piksel yokken burada 720 tane var.
//!
//! **Ve son satır asimetriyi kanıtlıyor.** Bileşen çıkarıldığında köşe/merkez oranı 1,206 —
//! yani "vignette = 0"ın 1,344'ü ile "vignette = 0,9"un 0,635'inin **arasında**. Bu, kimsenin
//! istemediği `PostProcessUniforms::default()`'un 0,25'i. Bileşen takmayan bir oyun köşe
//! karartmasını ne açabiliyor ne kapatabiliyor; `Renderer`'da o alan yok.
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
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Color Grading", 1280, 720)
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
                        ui.label("Bevy'nin renk derecelendirmesi (gamma, doygunluk, kontrast,");
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
