//! # Olay zinciri
//!
//! Sistemler arası iletişim: bir sistem olay **yazıyor**, başkaları **okuyor**, ve kimse
//! kimseyi doğrudan çağırmıyor. Örnek, bir oyunda tanıdık bir zinciri kuruyor — zaman aşımıyla
//! hasar üret → zırhı düş → kalan hasarı cana uygula → ses ve parçacık tetikle — ve her adımı
//! ayrı bir sisteme veriyor.
//!
//! ## Motorun olay modeli ÇİFT TAMPONLU, ve bu zincirin ritmini değiştiriyor
//!
//! Yazılan bir olayın **aynı karede** okunabildiği bir modelde zinciri kurmak sistemleri sıraya
//! dizmek demektir. Gizmo'nun [`Events<T>`]'i başka çalışır: yazılan olay o karede **görünmez**,
//! kare sonunda `update()` tamponları döndürür ve olay **bir sonraki** karede okunur.
//!
//! Sonucu bu demoda ölçüldü: dört aşamalı zincir tek karede değil, **kare kare** akıyor —
//! HUD'dan alınan bir tur şöyle:
//!
//! ```text
//! 1. hasar üretildi (DealDamage):        kare 207
//! 2. zırh düşüldü (DamageAfterArmor):    kare 208
//! 3. cana uygulandı (DamageReceived):    kare 209
//! 4. ses + parçacık:                     kare 210
//! ```
//!
//! Her sıçrama tam bir kare; zincirin boyu 3.
//! Bu bir kusur değil, başka bir denge: her okuyucu aynı yığını görür (okumak tüketmez) ve
//! sistem sırası olayların görünürlüğünü etkilemez — yani zincir kurmak için sistemleri sıraya
//! dizmek gerekmez, ama zincirin gecikmesi vardır.
//!
//! Uçuştaki bir olayı aynı sistemde hem okuyup hem değiştirmek motorda **yok**:
//! `EventReader`/`EventWriter` ayrıdır. Bu yüzden "zırhı doğrudan olayın içinden düş" adımı
//! burada "azaltılmış hasarı yeni bir olay olarak yaz" biçiminde yazıldı — aynı sonuç, motorun
//! kendi imkânıyla.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera (sahne tek bir can çubuğu)

use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::query::{Mut, Query, With};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bir şeyin hasar vermeye çalıştığını duyuran olay.
#[derive(Debug, Clone, Copy)]
struct DealDamage {
    amount: i32,
}

/// Zırhtan geçtikten sonra kalan hasar.
#[derive(Debug, Clone, Copy)]
struct DamageAfterArmor {
    amount: i32,
}

/// Zırhın hasarı tamamen yuttuğu an.
#[derive(Debug, Clone, Copy)]
struct ArmorBlockedDamage;

/// Canın gerçekten azaldığı an — sesi ve parçacığı bu tetikler.
#[derive(Debug, Clone, Copy)]
struct DamageReceived;

/// Hasar sayacı ve zincirin izlendiği defter.
#[derive(Clone, Copy)]
struct Combat {
    /// Bir sonraki vuruşa kalan süre.
    timer: f32,
    /// Canın kalanı (0..=MAX_HEALTH).
    health: i32,
    /// Zırhın her vuruşta yuttuğu miktar.
    armor: i32,
    /// Zincirin dört aşamasının en son tetiklendiği kare.
    stage_frame: [u64; 4],
    /// Ses ve parçacık kaç kez tetiklendi (iki ayrı okuyucu, aynı olay).
    sounds: u32,
    particles: u32,
}

impl Default for Combat {
    fn default() -> Self {
        Self {
            timer: TICK_SECONDS,
            health: MAX_HEALTH,
            armor: 3,
            stage_frame: [0; 4],
            sounds: 0,
            particles: 0,
        }
    }
}

gizmo::core::impl_component!(Combat);

/// Can çubuğunun işareti — ölçeği canı gösterir.
#[derive(Clone, Copy)]
struct HealthBar;
gizmo::core::impl_component!(HealthBar);

/// Vuruş sayacının periyodu.
const TICK_SECONDS: f32 = 1.0;
/// Başlangıç canı.
const MAX_HEALTH: i32 = 100;
/// Her vuruşun ham hasarı.
const HIT: i32 = 10;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Events", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            // Can çubuğu: X'te ölçeklenen bir küp.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.6, 0.0)).with_scale(Vec3::new(2.0, 0.25, 0.25)),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.85, 0.2, 0.2, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                HealthBar,
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 8.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(Combat::default());
            scene.spawn_camera(state, Vec3::new(0.0, 1.6, 4.0), Vec3::new(0.0, 0.6, 0.0));
        })
        // Olay tipleri kayıt ediliyor: her biri bir kaynak + kare sonu döndürme demek.
        .add_event::<DealDamage>()
        .add_event::<DamageAfterArmor>()
        .add_event::<ArmorBlockedDamage>()
        .add_event::<DamageReceived>()
        // Zincirin dört aşaması. SIRA ÖNEMLİ DEĞİL: olaylar zaten bir kare gecikmeli okunuyor,
        // yani hangi sistem önce koşarsa koşsun sonuç aynı.
        .add_update_system(deal_damage_over_time.in_phase(Phase::Update))
        .add_update_system(apply_armor_to_damage.in_phase(Phase::Update))
        .add_update_system(apply_damage_to_health.in_phase(Phase::Update))
        .add_update_system(play_damage_received_sound.in_phase(Phase::Update))
        .add_update_system(play_damage_received_particle_effect.in_phase(Phase::Update))
        .add_update_system(update_health_bar.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(combat) = world.get_resource::<Combat>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("events".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Olay zinciri");
                    ui.label(format!("can: {} / {MAX_HEALTH}", combat.health));
                    ui.label(format!("zırh: her vuruşta {}", combat.armor));
                    ui.separator();
                    ui.label("aşamaların en son tetiklendiği kare:");
                    for (i, name) in [
                        "1. hasar üretildi (DealDamage)",
                        "2. zırh düşüldü (DamageAfterArmor)",
                        "3. cana uygulandı (DamageReceived)",
                        "4. ses + parçacık",
                    ]
                    .iter()
                    .enumerate()
                    {
                        ui.label(format!("  {name}: {}", combat.stage_frame[i]));
                    }
                    let span = combat.stage_frame[3].saturating_sub(combat.stage_frame[0]);
                    ui.label(format!("zincirin kare cinsinden boyu: {span}"));
                    ui.separator();
                    ui.label(format!(
                        "aynı olayı iki okuyucu gördü: {} ses · {} parçacık",
                        combat.sounds, combat.particles
                    ));
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// 1. aşama: sayaç dolunca `DealDamage` yaz.
fn deal_damage_over_time(
    mut combat: ResMut<Combat>,
    mut writer: EventWriter<DealDamage>,
    time: Res<Time>,
) {
    combat.timer -= time.dt();
    if combat.timer > 0.0 {
        return;
    }
    combat.timer += TICK_SECONDS;
    combat.stage_frame[0] = time.frame();
    writer.send(DealDamage { amount: HIT });
}

/// 2. aşama: zırhı düş, kalanı yeni bir olay olarak yaz.
///
/// Uçuştaki olayı yerinde DEĞİŞTİRMEK yok; motorda okuma ve yazma ayrı olduğu için azaltılmış
/// hasar ayrı bir tip olarak yazılıyor — ve bu, zinciri okurken daha açık: her aşamanın kendi
/// olayı var.
fn apply_armor_to_damage(
    mut combat: ResMut<Combat>,
    reader: EventReader<DealDamage>,
    mut after_armor: EventWriter<DamageAfterArmor>,
    mut blocked: EventWriter<ArmorBlockedDamage>,
    time: Res<Time>,
) {
    for damage in reader.iter() {
        combat.stage_frame[1] = time.frame();
        let remaining = damage.amount - combat.armor;
        if remaining <= 0 {
            blocked.send(ArmorBlockedDamage);
        } else {
            after_armor.send(DamageAfterArmor { amount: remaining });
        }
    }
}

/// 3. aşama: kalan hasarı cana uygula ve `DamageReceived` duyur.
fn apply_damage_to_health(
    mut combat: ResMut<Combat>,
    reader: EventReader<DamageAfterArmor>,
    mut received: EventWriter<DamageReceived>,
    time: Res<Time>,
) {
    for damage in reader.iter() {
        combat.stage_frame[2] = time.frame();
        combat.health = (combat.health - damage.amount).max(0);
        if combat.health == 0 {
            combat.health = MAX_HEALTH; // döngü sürsün
        }
        received.send(DamageReceived);
    }
}

/// 4a. aşama: sesi çal. **Aynı olayı** 4b de okuyor — okumak tüketmiyor.
fn play_damage_received_sound(
    mut combat: ResMut<Combat>,
    reader: EventReader<DamageReceived>,
    time: Res<Time>,
) {
    for _ in reader.iter() {
        combat.stage_frame[3] = time.frame();
        combat.sounds += 1;
        gizmo::gizmo_log!(Info, "ses çalınıyor");
    }
}

/// 4b. aşama: parçacığı at.
fn play_damage_received_particle_effect(
    mut combat: ResMut<Combat>,
    reader: EventReader<DamageReceived>,
) {
    for _ in reader.iter() {
        combat.particles += 1;
        gizmo::gizmo_log!(Info, "parçacık atılıyor");
    }
}

/// Can çubuğunun boyunu cana göre ayarlar — zincirin görünür ucu.
fn update_health_bar(
    mut bars: Query<(Mut<Transform>, With<HealthBar>)>,
    combat: Res<Combat>,
) {
    let ratio = combat.health as f32 / MAX_HEALTH as f32;
    for (_entity, (mut transform, _)) in bars.iter_mut() {
        transform.scale.x = 2.0 * ratio.max(0.02);
    }
}
