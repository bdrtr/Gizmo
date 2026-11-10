//! Duman demosu — motorun GPU parçacık sistemi + YENİ AAA katmanları:
//!   • T1 Soft Particles: parçacıklar zemine/kutulara SERT girmez, yumuşakça kaybolur
//!     (FS sahne derinliğini örnekler).
//!   • T2 Flipbook/SubUV: her parçacık PROSEDÜREL üretilmiş animasyonlu bir duman sprite'ı
//!     oynatır (`set_procedural_smoke_flipbook`) — düz diskten çok daha gerçekçi.
//! Yükselen bir duman sütunu; kamera yandan bakar. Çalıştır: `cargo run -p demo --bin smoke_demo`
//!
//! İdiom notları (davranış AYNI, yalnız kod motorun modern yüzeyine oturtuldu):
//!   * Her varlık tek `world.spawn_bundle((...))` ile kurulur — `spawn()` + elle N×
//!     `add_component` YOK. Sahnede fizik yok (yalnız `TransformPlugin`), dolayısıyla
//!     `RigidBodyBundle`/collider/Prefab, ömür komponentleri (`DespawnAfter/BelowY`) ve
//!     `despawn_all_with` UYGULANMAZ: kutular dekoratif statik mesh'ler, duman ise ECS
//!     varlığı değil GPU parçacığı (kendi `max_life`'ı var). Girdi de yalnız zamanı
//!     ilerletir → kenar-tespiti (`is_*_just_*`) gerekmez.
//!   * Elle `GlobalTransform` eklenmez: karenin hazırlığı (`prepare_frame`) her renderlanabilir
//!     mesh'e eksik `GlobalTransform`'u geri-doldurur ve local matrisleri senkronlar; kamera da
//!     `GlobalTransform` yoksa `Transform` konumuna düşer.
//!   * Render `RenderWorld` (with_scene_render'a çevrilmez, o TAA'yı da kapatıyor): flipbook
//!     kurulumu ve `gpu_fluid/ssr/ssgi/volumetric = None` kurulumda bir kez; her kare parçacık
//!     püskürtme `with_settings`'te, senkron noktasında — kaynağın saati (`SmokeClock`) oyunun
//!     kaynağı, update ilerletiyor.

use gizmo::prelude::*;
use gizmo::renderer::gpu_particles::GpuParticle;
use std::f32::consts::{FRAC_PI_4, TAU};

/// Kaynağın saati: update ilerletiyor, duman püskürtmesi senkron noktasında okuyor.
struct SmokeClock(f32);

const SOURCE: Vec3 = Vec3::new(0.0, 0.2, 0.0);

fn main() {
    gizmo::app::setup_panic_hook();
    println!("Duman demosu — soft particles + flipbook (prosedürel animasyonlu duman).");
    App::<()>::new("Gizmo — Duman (Soft + Flipbook)", 1600, 900)
        .add_plugin(TransformPlugin)
        .set_setup(setup)
        .set_update(|world, _state, dt, _input| {
            if let Some(mut clock) = world.get_resource_mut::<SmokeClock>() {
                clock.0 += dt;
            }
        })
        // Her kare kaynaktan duman püskürt — renderer'ın işi, oyunun saatiyle.
        .set_render_stage(RenderWorld::new().with_settings(|game, renderer| {
            let t = game.get_resource::<SmokeClock>().map_or(0.0, |clock| clock.0);
            if let Some(p) = renderer.gpu_particles.as_ref() {
                p.spawn_particles(&renderer.queue, &emit_smoke(t));
            }
        }))
        .run()
        .expect("çalıştırılamadı");
}

/// Kaynaktan yükselen, büyüyen duman parçacıkları (deterministik dağılım; her frame bir demet).
fn emit_smoke(t: f32) -> Vec<GpuParticle> {
    let n = 10;
    let mut v = Vec::with_capacity(n);
    for i in 0..n {
        let a = (i as f32 / n as f32) * TAU + t * 2.3;
        let spread = 0.35;
        let vy = 1.4 + (i % 3) as f32 * 0.25;
        v.push(GpuParticle {
            position: [
                SOURCE.x + a.cos() * 0.12,
                SOURCE.y,
                SOURCE.z + a.sin() * 0.12,
            ],
            life: 0.0,
            velocity: [a.cos() * spread, vy, a.sin() * spread],
            max_life: 3.2,
            color: [0.9, 0.92, 1.0, 0.55],
            size_start: 0.35,
            size_end: 1.7, // ömür boyunca büyür (duman yayılır)
            _padding: [0.0; 2],
        });
    }
    v
}

fn setup(world: &mut World, renderer: &mut Renderer) {
    // Prosedürel duman flipbook'u + duman fiziği (buoyancy/drag), bir kez.
    if let Some(p) = renderer.gpu_particles.as_mut() {
        p.set_procedural_smoke_flipbook(&renderer.device, &renderer.queue);
        p.gravity = -1.2; // negatif → yükselir (buoyancy)
        p.drag = 0.35;
        p.curl_strength = 2.2; // T3: diverjanssız curl-noise → gerçekçi kıvrılma
        p.lit = true; // T4: güneşe göre ışıklandır (aydınlık/gölge yüz)
    }
    // Stüdyo ortamı; kullanılmayan ağır pass'ler kapalı.
    renderer.environment_preset = 1;
    renderer.environment_preset_2 = 1;
    renderer.gpu_fluid = None;
    renderer.ssr = None;
    renderer.ssgi = None;
    renderer.volumetric = None;

    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Zemin
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        AssetManager::create_plane(&renderer.device, 40.0),
        Material::new(white.clone()).with_pbr(Vec4::new(0.14, 0.14, 0.16, 1.0), 0.85, 0.0),
        MeshRenderer::new(),
    ));

    // Birkaç kutu — soft particles'ın yumuşak kaybolmasını gösterir (duman kutulara girer).
    let cube = AssetManager::create_cube(&renderer.device);
    for (i, &x) in [-1.8f32, 1.8].iter().enumerate() {
        let col = if i == 0 {
            Vec4::new(0.6, 0.2, 0.2, 1.0)
        } else {
            Vec4::new(0.2, 0.3, 0.6, 1.0)
        };
        world.spawn_bundle((
            Transform::new(Vec3::new(x, 0.5, 0.0)).with_scale(Vec3::new(0.8, 1.0, 0.8)),
            cube.clone(),
            Material::new(white.clone()).with_pbr(col, 0.4, 0.2),
            MeshRenderer::new(),
        ));
    }

    // Işık
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 10.0, 0.0))
            .with_rotation(Quat::from_rotation_x(-FRAC_PI_4) * Quat::from_rotation_y(FRAC_PI_4)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0, LightRole::Sun),
    ));

    // Yan kamera — hedefe bakan yaw/pitch'ten kurulur.
    let pos = Vec3::new(5.0, 2.5, 6.5);
    let target = Vec3::new(0.0, 1.6, 0.0);
    let dir = (target - pos).normalize();
    let yaw = dir.z.atan2(dir.x);
    let pitch = dir.y.clamp(-1.0, 1.0).asin();
    world.spawn_bundle((
        Transform::new(pos),
        Camera::new(1.0, 0.1, 500.0, yaw, pitch, true),
    ));

    world.insert_resource(SmokeClock(0.0));
}
