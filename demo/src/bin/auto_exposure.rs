//! # Pozlama ve göz uyumu
//!
//! Karanlık bir koridordan güneşe çıkınca gözün yavaşça kısılması. Bir çizim motorunda bu,
//! karenin parlaklığını ölçüp pozlamayı ona göre kaydırmak: ölç → yumuşat → uygula.
//!
//! ## Motorda halkanın yarısı var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | pozlamayı **uygulamak** | var — [`Camera::exposure`], doğrusal bir çarpan |
//! | kare parlaklığını **ölçmek** | **yok** — histogram yok, indirgeme geçişi yok |
//! | uyum hızı / yumuşatma sabiti | **yok** |
//! | en düşük/en yüksek EV, hedef parlaklık | **yok** |
//! | ölçüm maskesi (merkez ağırlıklı, spot) | **yok** |
//! | fiziksel kamera (diyafram/enstantane/ISO) | **yok** — `exposure` ham bir çarpan, EV değil |
//!
//! Yani **aktüatör var, sensör yok**. Bir oyun her karede `Camera::exposure` yazabiliyor; ne
//! yazacağını söyleyecek bir ölçüm yok.
//!
//! Ve bu bir "henüz yazılmadı" değil, boru hattında bir üretici yokluğu: ağaçtaki dokuz compute
//! shader'ının hiçbiri indirgeme ya da histogram değil, ve pozlama tek bir yerde, ACES eğrisinin
//! hemen öncesinde, doğrusal olarak uygulanıyor (`post_process.wgsl`).
//!
//! ### İki ölü kardeş alan
//!
//! `exposure` adında üç alan var ve ikisi ölü: `SceneUniforms::exposure` (yalnız yerleşim
//! kararlılığı için duruyor, shader'ın kendisi öyle diyor) ve `Renderer::exposure` (hiçbir şey
//! okumuyor — `docs/CAPABILITY_GAPS.md` §G bir demonun kaydırağının bu ölü alana bağlanmış
//! olduğunu kaydediyor). Canlı olan `Camera::exposure`.
//!
//! ## Ölçüldü — A: parlaklık savruluyor, hiçbir şey geri çekmiyor
//!
//! İki istasyon, pozlama ikisinde de 1,0. Ölçüm sol yarının HUD altı, ortalama parlaklık
//! (2026-08-23, 948×1028):
//!
//! | istasyon | kare 120 | kare 700 | 580 karelik sürüklenme |
//! |----------|----------|----------|------------------------|
//! | parlak | 148,974 | 149,102 | **+0,128** |
//! | karanlık | 56,384 | 56,400 | **+0,015** |
//!
//! İki istasyon arasında **2,64 kat** fark var, ve on saniyeye yakın bir sürede hiçbiri
//! kımıldamıyor: sürüklenme yakalama gürültüsünün mertebesinde. Göz uyumu olan bir boru hattında
//! bu iki sayı bir iki saniye içinde birbirine yaklaşırdı. Halka **açık**.
//!
//! ## Ölçüldü — B: aktüatörün yetkisi var
//!
//! Aynı karanlık istasyon, yalnız `Camera::exposure` değişiyor:
//!
//! | pozlama | ortalama parlaklık | 0,5'e oran |
//! |---------|--------------------|------------|
//! | 0,5 | 31,847 | 1,000 |
//! | 1,0 | 56,384 | 1,771 |
//! | 2,0 | 90,912 | 2,855 |
//! | 4,0 | 130,401 | **4,095** |
//!
//! Yani sekiz kat pozlama, ekranda 4,1 kat parlaklık — ve bu, iki istasyon arasındaki 2,64 katı
//! **fazlasıyla** kapatmaya yeter. Karanlık istasyonu parlak istasyonun seviyesine çıkarmak için
//! gereken pozlama 5 civarında, yani menzilin içinde.
//!
//! Demek ki eksik olan yetki değil, **bilgi**: bir oyun pozlamayı istediği yere koyabiliyor, ama
//! nereye koyacağını söyleyecek bir ölçüm yok.
//!
//! Merdivenin doğrusal olmaması da bilgi: 0,5 → 1,0 katsayısı 1,77, 1,0 → 2,0'de 1,61, 2,0 → 4,0'da
//! 1,43'e düşüyor. Bu, ACES eğrisinin yukarıda yatmasıdır — pozlama eğrinin **öncesinde**,
//! doğrusal olarak uygulanıyor, ve eğri artışı yutuyor.
//!
//! ## Halkayı oyun tarafında kapatmanın bedeli
//!
//! Sensörü elle yazmak mümkün ama pahalı: ağaçtaki tek geri okuma yolu
//! `gizmo_renderer::capture::texture_to_png`, ve kendi belgesi onu "bir tanılama" diye anıyor —
//! kopyalama komutunu kendi gönderiyor, okuma dönene kadar **bloke oluyor**, ve üstüne bir de
//! PNG kodluyor. Kare başına çalıştırılacak bir şey değil.
//!
//! ## Kontroller
//!   * `GIZMO_AE_POS=parlak|karanlik` — kamerayı iki istasyondan birine koy
//!   * `GIZMO_AE_EXPOSURE=<sayı>` — elle pozlama (varsayılan 1,0)
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İki istasyon: aydınlık dış alan ve karanlık iç mekân.
const BRIGHT_POS: Vec3 = Vec3::new(0.0, 2.0, 14.0);
const DARK_POS: Vec3 = Vec3::new(0.0, 2.0, -13.0);

#[derive(Clone, Copy)]
struct Ae {
    exposure: f32,
    dark: bool,
    frame: u32,
}
gizmo::core::impl_component!(Ae);

fn config() -> (f32, bool) {
    let e = std::env::var("GIZMO_AE_EXPOSURE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0f32)
        .clamp(0.05, 16.0);
    (e, matches!(std::env::var("GIZMO_AE_POS").as_deref(), Ok("karanlik")))
}

fn main() {
    let (exposure, dark) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Auto Exposure", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Aydınlık taraf: açık zemin, güneş altında.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 10.0)).with_scale(Vec3::new(20.0, 0.4, 20.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.85, 0.83, 0.78, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            // Karanlık taraf: koyu zemin ve üstünü kapatan bir tavan.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, -10.0)).with_scale(Vec3::new(20.0, 0.4, 20.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.10, 0.10, 0.12, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 6.0, -10.0)).with_scale(Vec3::new(20.0, 0.4, 22.0)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone()).with_pbr(Vec4::new(0.08, 0.08, 0.09, 1.0), 0.95, 0.0),
                MeshRenderer::new(),
            ));

            // İki tarafta da aynı nesneler — karşılaştırılacak şey aydınlatma, geometri değil.
            for side in [10.0f32, -10.0] {
                for i in 0..5 {
                    scene.world.spawn_bundle((
                        Transform::new(Vec3::new((i as f32 - 2.0) * 2.2, 1.1, side))
                            .with_scale(Vec3::splat(0.8)),
                        GlobalTransform::default(),
                        cube.clone(),
                        Material::new(white.clone()).with_pbr(
                            Vec4::new(0.70, 0.45, 0.30, 1.0),
                            0.5,
                            0.0,
                        ),
                        MeshRenderer::new(),
                    ));
                }
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.2) * Quat::from_rotation_x(-0.9),
                intensity: 3.4,
                ..Default::default()
            });
            let _ = white;

            let pos = if dark { DARK_POS } else { BRIGHT_POS };
            let look = if dark {
                Vec3::new(0.0, 1.0, -10.0)
            } else {
                Vec3::new(0.0, 1.0, 10.0)
            };
            scene.spawn_camera(state, pos, look);
            scene.world.insert_resource(Ae {
                exposure,
                dark,
                frame: 0,
            });
            gizmo::gizmo_log!(
                Info,
                "istasyon: {} · elle pozlama: {}",
                if dark { "karanlık" } else { "parlak" },
                exposure
            );
        })
        // Pozlama her karede yazılıyor: aktüatörün canlı olduğunu göstermenin yolu bu.
        .add_update_system(apply_exposure.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(a) = world.get_resource::<Ae>().map(|a| *a) else {
                return;
            };
            gizmo::egui::Area::new("ae".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(410.0);
                        ui.heading("Pozlama");
                        ui.label(format!(
                            "istasyon: {} · pozlama {:.2} · kare {}",
                            if a.dark { "karanlık" } else { "parlak" },
                            a.exposure,
                            a.frame
                        ));
                        ui.separator();
                        ui.label("Camera::exposure yazılabiliyor — aktüatör CANLI.");
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "ama kare parlaklığını ölçen hiçbir şey yok",
                        );
                        ui.label("histogram yok · indirgeme geçişi yok · uyum yok");
                        ui.separator();
                        ui.label("SceneUniforms::exposure ve Renderer::exposure ÖLÜ;");
                        ui.label("canlı olan Camera::exposure.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Pozlamayı kameraya yazar. Motorda bunu **otomatik** yapan bir şey yok; değeri veren biziz.
fn apply_exposure(mut cameras: Query<Mut<Camera>>, mut ae: ResMut<Ae>) {
    ae.frame += 1;
    let e = ae.exposure;
    for (_entity, mut camera) in cameras.iter_mut() {
        camera.exposure = e;
    }
}
