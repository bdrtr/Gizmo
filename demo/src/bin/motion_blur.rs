//! # Hareket bulanıklığı
//!
//! Hızlı hareket eden bir şeyin karede iz bırakması. Gerçek bir kamerada bu, deklanşörün açık
//! kaldığı süre boyunca ışığın birikmesi; bir çizim motorunda ise ya hız tamponundan ya da
//! önceki karenin dönüşümünden türetilir.
//!
//! ## Motorda yok — ve iki yarısı farklı derinlikte yok
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | deklanşör süresi / şiddet / örnek sayısı ayarı | **yok** |
//! | hız (velocity) tamponu | **yok** — G-tamponunun dört hedefi var, hiçbiri hız değil |
//! | nesne başına önceki dönüşüm | **yok** — `InstanceRaw` yalnız `model` taşıyor |
//! | bulanıklık geçişi | **yok** — `PostProcess`'in sekiz alanı var, hiçbiri hareket değil |
//!
//! `PostProcess`'in alanları şunlar: `bloom_intensity`, `bloom_threshold`, `vignette`,
//! `chromatic_aberration`, `dof_focus_dist`, `dof_focus_range`, `dof_blur_size`, `film_grain`.
//!
//! ## Ama kamera yarısının malzemesi zaten GPU'da
//!
//! Kamera hareketinden gelen bulanıklık için gereken iki girdi hazır: TAA geçen karenin
//! **titremesiz** görüntü-izdüşüm matrisini tutuyor (`Taa::prev_vp`, `TaaParams.prev_view_proj`)
//! ve G-tamponu `world_position` yazıyor. Bu ikisi, bir pikselin geçen kare nerede olduğunu
//! hesaplamaya yeter — `taa.wgsl` bu aritmetiği bugün zaten yapıyor.
//!
//! **Nesne** hareketinden gelen bulanıklık ise veri katmanında kapalı: hiçbir yerde önceki
//! `Transform` tutulmuyor, ve `InstanceRaw`'a ikinci bir matris eklemek iki çizim yolunun da
//! aynı biçimde doldurması gereken bir alan demek.
//!
//! ## Ölçüldü — kamera tarafı kesin
//!
//! Üç hız, **aynı açıda** yakalanıyor: kamera 1200 karede 1, 4 ya da 16 tur atıyor, ve açı
//! kareden tam hesaplandığı (biriktirilmediği) için yakalama karesinde üçü de tam sıfırda.
//! Aradaki tek fark hız. Ölçüldü (2026-08-23, 948×1028, kare 1200, Sobel kenar enerjisi,
//! sol yarı):
//!
//! | kamera hızı | kenar enerjisi | tabana oran |
//! |-------------|----------------|-------------|
//! | 1 tur/1200 kare | 9,977 | 1,0000 |
//! | 4 tur/1200 kare | 9,977 | **1,0000** |
//! | 16 tur/1200 kare | 9,977 | **1,0000** |
//!
//! Ve sayılar yuvarlanmadan da aynı: iki kare arasındaki fark kutusu **yalnız HUD metni**
//! (`(534, 42)–(753, 54)`), yani sahnenin çizimi **bit bit özdeş**. On altı kat hız, aynı piksel.
//!
//! Deklanşörü olan bir çizim yolunda kenar enerjisi hızla düşerdi. Burada kare, süresi sıfır
//! olan bir örnek.
//!
//! ### Kontrol
//!
//! `GIZMO_BLUR_FREEZE=1` ile hiç döndürmeden aynı iki koşu: HUD altında **sıfır** farklı piksel,
//! maks 0. Yani sahne koşular arasında deterministik, ve yukarıdaki "aynı" gerçekten aynı.
//!
//! ## Nesne tarafı: veri katmanından kesin, pikselden değil
//!
//! Nesne dönüşü için aynı deneyi kuramadım, ve sebebini saklamıyorum. Yakalama karesini 1196'dan
//! 1202'ye taradığımda fark V çiziyor — en düşük 1199'da (2 417 farklı örnek, maks 118) — ama
//! **hiçbir karede sıfıra inmiyor**, oysa sistemimin yazdığı açı 1200'de iki hızda da tam sıfır.
//! Yani çizilen duruş, o karede yazdığım duruşa denk gelmiyor; arada bir kayma var. TAA'yı
//! kapatmak (`GIZMO_BLUR_NO_TAA=1`) değiştirmedi, ve dondurma kontrolü sahnenin deterministik
//! olduğunu gösteriyor. Kaymanın kaynağını kovalamadım — **açık soru**.
//!
//! Ama nesne bulanıklığının yokluğu zaten pikselle kanıtlanmayı gerektirmiyor, çünkü veri
//! katmanında kapalı:
//!
//!   * `InstanceRaw` yalnız `model` taşıyor — nesne başına **önceki** matris yok;
//!   * ağaçta hiçbir yerde önceki `Transform` tutulmuyor;
//!   * G-tamponunun dört hedefi var (`albedo_metallic`, `normal_roughness`, `world_position`,
//!     `world_tangent`) ve hiçbiri hız değil.
//!
//! Bu üçü olmadan nesne hızı **hesaplanamaz**, yani bulanıklık üretilemez.
//!
//! ## Kontroller
//!   * `GIZMO_BLUR_RATE=1|4|16` — kamera bir turu kaç karede tamamlasın (1200 / 300 / 75)
//!   * `GIZMO_BLUR_SPIN=1` — kamerayı durdur, nesneyi döndür (nesne bulanıklığı tarafı)
//!   * `GIZMO_BLUR_NO_TAA=1` — TAA'yı kapat, ölçümü ondan ayır
//!   * **Sağ-tık + fare / WASDQE** — kamera (ölçüm için dokunmayın)

use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::TAU;

/// Ölçümün yakalandığı kare. Üç hızın üçü de bu karede **tam tur sayısını** tamamlıyor, yani
/// kamera aynı açıda oluyor. Fark ne varsa bulanıklıktandır.
const CAPTURE_FRAME: u32 = 1200;

/// Dönen nesneyi işaretler.
#[derive(Clone, Copy)]
struct Spinner;
gizmo::core::impl_component!(Spinner);

/// Ölçüm durumu.
#[derive(Clone, Copy)]
struct Motion {
    /// Kaç tur/1200 kare.
    rate: u32,
    /// Nesne dönüyor, kamera duruyor.
    spin_object: bool,
    angle: f32,
    frame: u32,
}
gizmo::core::impl_component!(Motion);

const ORBIT_RADIUS: f32 = 9.0;

fn config() -> (u32, bool) {
    let rate = std::env::var("GIZMO_BLUR_RATE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1u32)
        .clamp(1, 64);
    (rate, std::env::var("GIZMO_BLUR_SPIN").is_ok())
}

fn main() {
    let (rate, spin_object) = config();

    App::<SimpleSceneState>::new("Gizmo Engine - Motion Blur", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Kenarı bol, yüksek kontrastlı geometri: bulanıklığın ölçüldüğü şey kenar enerjisi,
            // ve ölçülecek kenar olmazsa fark da olmaz.
            let cube = AssetManager::create_cube(device);
            for i in 0..24 {
                let a = i as f32 / 24.0 * TAU;
                let r = 3.4 + (i % 3) as f32 * 1.1;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(a.cos() * r, ((i % 5) as f32 - 2.0) * 0.9, a.sin() * r))
                        .with_scale(Vec3::splat(0.5))
                        .with_rotation(Quat::from_rotation_y(a) * Quat::from_rotation_x(0.4)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        if i % 2 == 0 {
                            Vec4::new(0.95, 0.95, 0.95, 1.0)
                        } else {
                            Vec4::new(0.06, 0.06, 0.07, 1.0)
                        },
                        0.55,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
                if spin_object {
                    scene.world.add_component(entity, Spinner);
                }
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -3.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white).with_pbr(Vec4::new(0.35, 0.36, 0.40, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.7),
                intensity: 2.8,
                ..Default::default()
            });

            scene.world.insert_resource(Motion {
                rate,
                spin_object,
                angle: 0.0,
                frame: 0,
            });
            gizmo::gizmo_log!(
                Info,
                "hız {} tur/1200 kare · kare başına {:.5} rad · nesne dönüyor: {}",
                rate,
                TAU * rate as f32 / CAPTURE_FRAME as f32,
                spin_object
            );
            scene.spawn_camera(state, Vec3::new(0.0, 1.2, ORBIT_RADIUS), Vec3::ZERO);
        })
        .add_update_system(drive.in_phase(Phase::PreUpdate))
        .set_render(|world, _state, encoder, view, renderer, _lt| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            // TAA, boru hattında harekete tepki veren TEK bileşen — ve tepkisi geçmişi
            // reddetmek, yani hızlı hareketi bulanıklaştırmak değil **keskinleştirmek**.
            // Ölçümü ondan ayırabilmek için kapatılabiliyor.
            if std::env::var("GIZMO_BLUR_NO_TAA").is_ok() {
                renderer.taa = None;
            }
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |world, _state, ctx| {
            let Some(m) = world.get_resource::<Motion>().map(|m| *m) else {
                return;
            };
            gizmo::egui::Area::new("mb".into())
                .anchor(gizmo::egui::Align2::RIGHT_TOP, [-12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(400.0);
                        ui.heading("Hareket bulanıklığı");
                        ui.label(format!(
                            "{} tur/1200 kare · kare {} · açı {:.3} rad",
                            m.rate,
                            m.frame,
                            m.angle % TAU
                        ));
                        ui.label(if m.spin_object {
                            "nesne dönüyor, kamera duruyor"
                        } else {
                            "kamera dönüyor, nesne duruyor"
                        });
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 160, 80),
                            "motorda hareket bulanıklığı YOK",
                        );
                        ui.label("hız tamponu yok · önceki dönüşüm yok · geçiş yok");
                        ui.separator();
                        ui.label("ama kamera yarısının girdisi hazır:");
                        ui.label("TAA'nın prev_view_proj'u + G-tamponun world_position'ı");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kamerayı (ya da nesneleri) **sabit** açı adımıyla döndürür.
///
/// `dt` ile değil: kare süresine bağlı bir dönüş, iki koşuyu aynı karede farklı açıya koyar ve
/// karşılaştırmayı yok eder.
fn drive(
    mut cameras: Query<(Mut<Transform>, With<Camera>)>,
    mut spinners: Query<(Mut<Transform>, With<Spinner>)>,
    mut motion: ResMut<Motion>,
    time: gizmo::core::system::Res<Time>,
) {
    motion.frame += 1;
    // Açı BİRİKTİRİLMİYOR, kareden tam hesaplanıyor.
    //
    // İlk kurulumda `angle += step` vardı ve yakalama karesinde üç hızın kalıntısı farklıydı
    // (0,000047 · 0,000189 · 0,000755 rad). O kadarcık açı bile nesneleri piksel altı kaydırıp
    // bütün kareyi değiştiriyordu — yani ölçülen fark bulanıklık değil, benim sürücümün
    // kayan nokta birikimiydi. `period` 1200'ü tam böldüğü için bu biçimde açı, yakalama
    // karesinde üç hızda da **tam sıfır**.
    let period = (CAPTURE_FRAME / motion.rate).max(1);
    // `GIZMO_BLUR_FREEZE=1`: hiç döndürme. Sahnenin iki koşuda aynı kareyi verip vermediğini
    // sınayan kontrol — vermiyorsa ölçülen fark hareketten değil, belirsizlikten gelir.
    let a = if std::env::var("GIZMO_BLUR_FREEZE").is_ok() {
        0.0
    } else {
        TAU * (motion.frame % period) as f32 / period as f32
    };
    motion.angle = a;

    if motion.spin_object {
        // Nesne dönüyor, kamera sabit — kamera yeniden izdüşümünün hiçbir katkısı olamayacağı
        // durum. Nesne bulanıklığı ancak nesne başına önceki dönüşümle üretilebilirdi.
        // YALNIZ Y ekseni, ve yalnız `a` ile. İlk kurulumda `from_rotation_x(a * 0.6)` de
        // vardı: `a` tam tur katıyken `a * 0.6` değil, yani iki koşu yakalama karesinde aynı
        // duruşta olmuyordu ve fark bulanıklık değil **poz** farkıydı.
        for (_e, (mut t, _)) in spinners.iter_mut() {
            t.rotation = Quat::from_rotation_y(a);
        }
    } else {
        for (_e, (mut t, _)) in cameras.iter_mut() {
            t.position = Vec3::new(a.cos() * ORBIT_RADIUS, 1.2, a.sin() * ORBIT_RADIUS);
        }
    }

    if (CAPTURE_FRAME - 2..=CAPTURE_FRAME + 2).contains(&motion.frame) {
        gizmo::gizmo_log!(
            Info,
            "kendi kare {} · motor kare {} · açı {:.6} rad · period {}",
            motion.frame,
            time.frame(),
            a,
            period
        );
    }
}
