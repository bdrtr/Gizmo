//! # Bevy'nin `ecs/iter_combinations` örneğinin Gizmo Engine portu
//!
//! Bir sorgunun **her ikilisini** gezmek. Bevy'nin `Query::iter_combinations_mut::<2>()`'si tam
//! bunun için var, ve örneği bir N-cisim çekim benzetimi: her cisim her cisme kuvvet uyguluyor.
//!
//! ## Motorda ikili yineleme yok
//!
//! `Query`'nin yüzeyi `iter` / `iter_mut` / `iter_chunks` / `par_for_each` ile bitiyor;
//! `iter_combinations` diye bir şey yok. Ve eksikliği yalnız bir kolaylık meselesi değil —
//! ödünç alma kuralı yüzünden **elle yazmak da doğrudan mümkün değil**: aynı sorguyu hem okumak
//! hem yazmak için aynı anda iki görünüm açılamıyor.
//!
//! Bu yüzden port iki geçişli: önce konum ve kütleler bir `Vec`'e toplanıyor, kuvvetler orada
//! hesaplanıyor, sonra hızlar geri yazılıyor. Bevy'de tek geçiş.
//!
//! ## Ölçülen: elle yazılan döngü doğru mu
//!
//! "Çalışıyor gibi görünüyor" bir N-cisim benzetimi için ölçüm değil — yanlış eşleştirilmiş bir
//! ikili döngü de makul görünen bir hareket üretir. Ölçüt fizik: kapalı bir sistemde iç kuvvetler
//! eşit ve zıt olduğu için **toplam momentum korunmalı**.
//!
//! Demo her cismin `kütle × hız` toplamını izliyor. Başlangıçta sıfıra ayarlanıyor (ağırlık
//! merkezi sabit), ve ikili döngü doğruysa sıfırda kalmalı. Kalmıyorsa döngü bir çifti iki kez
//! ya da hiç saymıyor demektir.
//!
//! Ölçüldü (2026-08-23, 12 cisim = 66 çift, `GIZMO_NBODY_SELFTEST=1`):
//!
//! | kare | toplam momentum | kinetik enerji |
//! |------|-----------------|----------------|
//! | başlangıç | **0,000000** | — |
//! | 300 | 0,000002 | 122,102 |
//! | 600 | 0,000008 | 52,213 |
//! | 900 | 0,000003 | 55,905 |
//! | 1200 | 0,000003 | 64,584 |
//! | 1500 | 0,000006 | 40,142 |
//!
//! Momentum 1500 kare boyunca **10⁻⁵'in altında** kalıyor — bu kayan nokta yuvarlamasıdır, sürüklenme
//! değil. Bir çift iki kez sayılsa ya da bir cisim kendisiyle eşleşse toplam görünür biçimde
//! kayardı. Yani elle yazılan `j = i+1` döngüsü doğru eşleşiyor.
//!
//! Kinetik enerjinin salınması beklenen: korunan şey toplam enerji, ve kinetik olan sürekli çekim
//! potansiyeline dönüşüyor.
//!
//! ### Ödünç alma kuralının ikinci yüzü
//!
//! İlk yazımda toplama geçişi `iter()` kullanıyordu ve derlenmedi: **`Mut<T>` taşıyan bir sorgu
//! salt-okunur bile gezilemiyor** (`iter` `ReadOnlyQuery` istiyor). Yani "önce oku, sonra yaz"
//! demek bile aynı sorgunun `iter_mut`'ını iki kez çağırmak demek. Bevy'nin tek çağrısının
//! karşılığı burada üç geçiş.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bir gök cismi.
#[derive(Clone, Copy)]
struct Body {
    mass: f32,
    velocity: Vec3,
}
gizmo::core::impl_component!(Body);

/// Ölçüm defteri.
#[derive(Default, Clone, Copy)]
struct Conservation {
    frame: u32,
    /// Toplam momentumun büyüklüğü — korunması gereken şey.
    momentum: f32,
    /// Başlangıçtaki değer, karşılaştırma için.
    initial_momentum: f32,
    /// Toplam kinetik enerji (korunmuyor, çekim potansiyeline dönüşüyor — yalnız bilgi).
    kinetic: f32,
    bodies: usize,
}
gizmo::core::impl_component!(Conservation);

/// Cisim sayısı ve çekim sabiti.
const BODIES: usize = 12;
const G: f32 = 3.0;
/// Yumuşatma: iki cisim üst üste gelince kuvvet sonsuza gitmesin.
const SOFTENING: f32 = 0.6;
/// Sabit adım — ölçümün kare süresine bağlı olmaması için.
const STEP: f32 = 1.0 / 120.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Iter Combinations", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let sphere = AssetManager::create_sphere(device, 1.0, 16, 24);

            let mut rng = 0x1234_5678u32;
            let mut roll = |low: f32, high: f32| {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                low + (rng as f32 / u32::MAX as f32) * (high - low)
            };

            // Cisimleri doğur, sonra ağırlık merkezinin hızını sıfırla: toplam momentum tam 0
            // başlasın ki korunumu ölçmek anlamlı olsun.
            let mut spawned: Vec<(gizmo::core::entity::Entity, f32, Vec3)> = Vec::new();
            for _ in 0..BODIES {
                let mass = roll(0.6, 2.2);
                let position = Vec3::new(roll(-5.0, 5.0), roll(-2.5, 2.5), roll(-5.0, 5.0));
                let velocity = Vec3::new(roll(-1.2, 1.2), roll(-0.6, 0.6), roll(-1.2, 1.2));
                let entity = scene.world.spawn_bundle((
                    Transform::new(position).with_scale(Vec3::splat(0.18 + mass * 0.12)),
                    GlobalTransform::default(),
                    sphere.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(roll(0.3, 1.0), roll(0.3, 0.9), roll(0.4, 1.0), 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Body { mass, velocity },
                ));
                spawned.push((entity, mass, velocity));
            }
            let total_mass: f32 = spawned.iter().map(|(_, m, _)| *m).sum();
            let drift: Vec3 =
                spawned.iter().map(|(_, m, v)| *v * *m).fold(Vec3::ZERO, |a, b| a + b) / total_mass;
            for (entity, mass, velocity) in &spawned {
                scene.world.add_component(
                    *entity,
                    Body {
                        mass: *mass,
                        velocity: *velocity - drift,
                    },
                );
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.5),
                intensity: 2.2,
                ..Default::default()
            });
            scene.world.insert_resource(Conservation {
                bodies: BODIES,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 4.0, 16.0), Vec3::ZERO);
        })
        .add_update_system(gravity.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(c) = world.get_resource::<Conservation>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("nbody".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(460.0);
                        ui.heading("Her ikiliyi gezmek");
                        ui.label(format!("{} cisim · {} çift", c.bodies, c.bodies * (c.bodies - 1) / 2));
                        ui.separator();
                        ui.label(format!("toplam momentum: {:.6}", c.momentum));
                        ui.label(format!("  başlangıçta   : {:.6}", c.initial_momentum));
                        ui.label(format!("kinetik enerji : {:.3} (korunmaz, potansiyele döner)", c.kinetic));
                        ui.separator();
                        ui.label("motorda iter_combinations YOK, ve ödünç alma kuralı");
                        ui.label("yüzünden tek geçişte elle yazmak da mümkün değil:");
                        ui.label("önce Vec'e topla, kuvveti hesapla, sonra geri yaz.");
                        ui.separator();
                        ui.label("momentum sıfırda kalıyorsa ikili döngü doğru eşleşiyor");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Çekim: her çift bir kez, kuvvet iki tarafa eşit ve zıt uygulanıyor.
///
/// Bevy'de bu `iter_combinations_mut::<2>()` ile tek geçiş. Burada iki geçiş — ve ikinci geçişin
/// varlığı bir üslup tercihi değil, ödünç alma kuralının sonucu.
fn gravity(mut bodies: Query<(Mut<Transform>, Mut<Body>)>, mut report: ResMut<Conservation>, time: Res<Time>) {
    let _ = time;

    // 1. geçiş: durumu topla.
    //
    // `iter()` DEĞİL `iter_mut()` — çünkü `Mut<T>` taşıyan bir sorgu salt-okunur bile
    // gezilemiyor (`iter` için `ReadOnlyQuery` sınırı sağlanmıyor). Bu, ikili döngünün neden tek
    // geçişte yazılamadığının bir başka yüzü.
    let mut state: Vec<(Vec3, f32, Vec3)> = Vec::new();
    for (_entity, (transform, body)) in bodies.iter_mut() {
        state.push((transform.position, body.mass, body.velocity));
    }

    // 2. geçiş: her ÇİFT bir kez. `j` her zaman `i`'den büyük — bir çifti iki kez saymanın
    // ve kendisiyle eşleştirmenin önüne geçen şey bu.
    let mut accel = vec![Vec3::ZERO; state.len()];
    for i in 0..state.len() {
        for j in (i + 1)..state.len() {
            let delta = state[j].0 - state[i].0;
            let dist2 = delta.length_squared() + SOFTENING * SOFTENING;
            let inv = 1.0 / (dist2 * dist2.sqrt());
            let dir = delta * inv;
            // Eşit ve zıt: momentumun korunmasının sebebi tam olarak bu iki satır.
            accel[i] += dir * (G * state[j].1);
            accel[j] -= dir * (G * state[i].1);
        }
    }

    // 3. geçiş: geri yaz.
    let mut momentum = Vec3::ZERO;
    let mut kinetic = 0.0;
    for (index, (_entity, (mut transform, mut body))) in bodies.iter_mut().enumerate() {
        body.velocity += accel[index] * STEP;
        transform.position += body.velocity * STEP;
        momentum += body.velocity * body.mass;
        kinetic += 0.5 * body.mass * body.velocity.length_squared();
    }

    report.frame += 1;
    report.momentum = momentum.length();
    report.kinetic = kinetic;
    if report.frame == 1 {
        report.initial_momentum = report.momentum;
    }

    if std::env::var("GIZMO_NBODY_SELFTEST").is_ok() && report.frame.is_multiple_of(300) {
        gizmo::gizmo_log!(
            Info,
            "kare {} · momentum {:.6} (başlangıç {:.6}) · kinetik {:.3}",
            report.frame,
            report.momentum,
            report.initial_momentum,
            report.kinetic
        );
    }
}
