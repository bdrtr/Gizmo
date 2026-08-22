//! # Bevy'nin `3d/spotlight` örneğinin Gizmo Engine portu
//!
//! On altı spot ışığı, dört-dörtlük bir ızgarada, kırk mavi küpün üstünde salınıyor. Her ışığın
//! koni açısı da zamanla açılıp kapanıyor — yani hem `Transform` (koninin nereye baktığı) hem de
//! [`SpotLight`]'ın kendi alanları (`inner_angle`/`outer_angle`) her karede değişiyor.
//!
//! Aydınlatma dersinin ikinci yarısı bu: [`bevy_lighting`] ışık **türlerini** yan yana koyuyordu,
//! bu demo tek bir türü sayıca ve hareketle zorluyor. On altı gölge düşüren koni, motorun
//! kümelenmiş ışıklandırmasının (clustered lighting) gerçek bir yükü.
//!
//! ## Bevy'den ayrılan üç yer, üçü de bilerek
//!
//!   * **Küplerin dağılımı** Bevy'de `ChaCha8Rng` ile tohumlanıyor. Buraya dört satırlık bir
//!     xorshift kondu — deponun `particles.rs`'inde aynı gerekçeyle yapılanın aynısı: dağılımın
//!     belirleyici olması yeter, kriptografik olması gerekmez, ve demo bir bağımlılık daha
//!     getirmez.
//!   * **Sahneyi gezdiren tuşlar** Bevy'de WASD. Burada **ok tuşları**: basit-sahne kamerası
//!     zaten WASD'ı kullanıyor, ikisini üst üste bindirmek hem kamerayı hem zemini oynatırdı.
//!     Ok tuşları da motorun ortak hareket karışımından (`move_axis_with`) geçiyor, elle
//!     okunmuyor — deponun kuralı bu.
//!   * **Işık şiddeti** Bevy'de lümen (40 000); Gizmo'nun `intensity`'si düz bir çarpan, o yüzden
//!     görsel olarak eşlendi.
//!
//! Işıkların ucundaki küçük parlayan küreler Bevy'deki gibi ışığın **çocuğu**: konumları
//! hiyerarşiden geliyor, ayrıca hizalanmıyor.
//!
//! ## Kontroller
//!   * **Ok tuşları** — zemini ve küpleri gezdir · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::{Input, MoveKeys};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res};
use gizmo::core::HierarchyExt;
use gizmo::prelude::*;
use gizmo::renderer::components::SpotLight;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

/// Ok tuşlarıyla gezdirilen zemin ve küplerin işareti (Bevy'nin `Movable`'ı).
#[derive(Clone, Copy)]
struct Movable;
gizmo::core::impl_component!(Movable);

/// `srgb_u8(124, 144, 255)` doğrusalda — küplerin mavisi.
const CUBE_BLUE: Vec4 = Vec4::new(0.191_202, 0.278_894, 1.0, 1.0);
/// Küp sayısı (Bevy'de 40).
const CUBE_COUNT: usize = 40;
/// Sahnenin gezdirilme hızı.
const MOVE_SPEED: f32 = 4.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Spotlight", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Zemin — 100×100, beyaz. Işık konileri en iyi böyle okunuyor.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 100.0),
                Material::new(white.clone()).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
                Movable,
            ));

            // Kırk küp, belirleyici bir dağılımla.
            let cube = AssetManager::create_cube(&scene.renderer.device);
            let mut rng = Xorshift::new(0x5EED_1234);
            for _ in 0..CUBE_COUNT {
                let x = rng.range(-5.0, 5.0);
                let y = rng.range(0.0, 3.0);
                let z = rng.range(-5.0, 5.0);
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, y, z)).with_scale(Vec3::splat(0.25)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(CUBE_BLUE, 0.5, 0.0),
                    MeshRenderer::new(),
                    Movable,
                ));
            }

            // 4×4 spot ızgarası, her biri iki parlayan işaretçiyle.
            let bulb = AssetManager::create_sphere(&scene.renderer.device, 0.05, 12, 18);
            let nose = AssetManager::create_sphere(&scene.renderer.device, 0.1, 12, 18);
            for gx in 0..4 {
                for gz in 0..4 {
                    let x = gx as f32 - 2.0;
                    let z = gz as f32 - 2.0;
                    let light = scene.world.spawn_bundle(SpotLightBundle {
                        position: Vec3::new(1.0 + x, 2.0, z),
                        // Yerel −Z aşağı baksın: koni zemine düşer.
                        rotation: Quat::from_rotation_x(-FRAC_PI_2),
                        color: Vec3::new(1.0, 1.0, 1.0),
                        intensity: 2.5,
                        radius: 5.0,
                        inner_angle: FRAC_PI_4 * 0.85,
                        outer_angle: FRAC_PI_4,
                    });

                    // Ampul: kırmızı ve parlayan.
                    let marker = scene.world.spawn_bundle((
                        Transform::new(Vec3::ZERO),
                        GlobalTransform::default(),
                        bulb.clone(),
                        Material::new(white.clone())
                            .with_pbr(Vec4::new(1.0, 0.0, 0.0, 1.0), 0.5, 0.0)
                            .with_emissive(Vec3::new(1.0, 0.0, 0.0)),
                        MeshRenderer::new(),
                    ));
                    scene.world.add_child(light, marker);

                    // Burun: koninin baktığı yönü gösteren, daha koyu ikinci küre.
                    let direction = scene.world.spawn_bundle((
                        Transform::new(Vec3::new(0.0, 0.0, -0.1)),
                        GlobalTransform::default(),
                        nose.clone(),
                        Material::new(white.clone())
                            .with_pbr(Vec4::new(0.369, 0.0, 0.0, 1.0), 0.5, 0.0)
                            .with_emissive(Vec3::new(0.369, 0.0, 0.0)),
                        MeshRenderer::new(),
                    ));
                    scene.world.add_child(light, direction);
                }
            }

            scene.spawn_camera(state, Vec3::new(-4.0, 5.0, 10.0), Vec3::ZERO);
        })
        .add_update_system(light_sway.in_phase(Phase::Update))
        .add_update_system(movement.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bevy'nin `light_sway`'i: koniyi salla, açısını da aç-kapa.
///
/// İki ayrı şeye dokunuyor — `Transform.rotation` koninin nereye baktığını, `SpotLight`'ın
/// açıları ise ne kadar geniş olduğunu belirliyor. Çocuk işaretçiler dönüşü hiyerarşiden
/// aldıkları için ayrıca döndürülmüyor.
fn light_sway(mut lights: Query<(Mut<Transform>, Mut<SpotLight>)>, time: Res<Time>) {
    let t = time.elapsed() as f32;
    let rotation = Quat::from_euler(
        EulerRot::XYZ,
        -FRAC_PI_2 + (t * 0.67 * 3.0).sin() * 0.5,
        (t * 3.0).sin() * 0.5,
        0.0,
    );
    let angle = ((t * 1.2).sin() + 1.0) * (FRAC_PI_4 - 0.1);

    for (_entity, (mut transform, mut light)) in lights.iter_mut() {
        transform.rotation = rotation;
        light.outer_angle = angle;
        light.inner_angle = angle * 0.8;
    }
}

/// Ok tuşlarıyla zemini ve küpleri gezdirir — ışıklar yerinde kalır, koniler sahnede gezinir.
fn movement(
    mut movables: Query<(Mut<Transform>, With<Movable>)>,
    input: Res<Input>,
    time: Res<Time>,
) {
    let (x, z) = input.move_axis_with(MoveKeys::ARROWS);
    if x == 0.0 && z == 0.0 {
        return;
    }
    let step = Vec3::new(x, 0.0, -z) * MOVE_SPEED * time.dt();
    for (_entity, (mut transform, _)) in movables.iter_mut() {
        transform.position += step;
    }
}

/// Dört satırlık xorshift — dağılımın belirleyici olması yeter.
///
/// Bevy örneği `ChaCha8Rng`'i "testin belirleyici olması için" tohumluyor; aynı gerekçe burada
/// bir bağımlılık gerektirmiyor. Deponun `render/particles.rs`'i de aynı yolu seçmişti.
struct Xorshift(u32);

impl Xorshift {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    /// `[low, high)` aralığında bir sayı.
    fn range(&mut self, low: f32, high: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        low + (self.0 as f32 / u32::MAX as f32) * (high - low)
    }
}
