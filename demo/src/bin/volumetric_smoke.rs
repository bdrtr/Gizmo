//! Volumetrik duman demosu (T6 + CS2-conform) — GERÇEK katılımcı-ortam raymarch (billboard
//! DEĞİL). Motorun `SmokeVolume`'u 3B yoğunluk grid'ini advekte eder + ışın boyunca yürütür
//! (Beer-Lambert + güneş saçılımı + sahne-derinliği occlusion) ve HDR'ye kompozit eder.
//! CS2-tarzı: duman kaynaktan DIŞA doğru dolar (fill) ve merkezdeki direği DELMEYİP etrafını
//! sarar (obstacle field). Çalıştır: `cargo run -p demo --bin volumetric_smoke`
//!
//! İdiomatik kurulum (yikim seviyesi) — NEYİN motora, NEYİN demoya ait olduğu konusunda dürüst:
//!   * Sahne varlıkları tek `world.spawn_bundle((...))` ile kurulur (elle `spawn()` + tekrar
//!     tekrar `add_component` zinciri YOK). Yardımcı `placed()` her varlığa Transform +
//!     ilk-kare için ön-doldurulmuş GlobalTransform verir.
//!   * Fizik YOK — App yalnız `TransformPlugin` kaydeder. Bu yüzden rigid body / collider /
//!     `DespawnAfter` gibi ömür idiomları BU demoya UYGULANMAZ; nesneler statik görsel mesh'tir
//!     ve `update` boştur (girdi yok → `is_*_just_*` de yok).
//!   * Render GERÇEK özel iş yapar: `SmokeVolume` kurulumu + kullanılmayan ağır pass'lerin
//!     (gpu_fluid/SSR/SSGI/volumetric) kapatılması. Bu yüzden `default_render_pass` DOĞRUDAN
//!     çağrılır — `with_scene_render()` kısayoluna ÇEVRİLMEZ (o kısayol bu efekt kontrolünü
//!     elden alırdı).

use gizmo::prelude::*;
use gizmo::renderer::gpu_smoke::SmokeVolume;

struct S {
    cam_id: u32,
}

// Merkezdeki direğin dünya-uzayı AABB'si. Duman bu hacmi DELMEZ (obstacle field) —
// hem görünür mesh hem SmokeVolume::set_obstacle_boxes AYNI kutuyu kullanır ki duman
// tam görünen geometriye conform olsun.
const PILLAR_MIN: [f32; 3] = [0.45, 0.0, -0.25];
const PILLAR_MAX: [f32; 3] = [0.95, 3.2, 0.25];

/// Transform + ilk-kare için ön-doldurulmuş GlobalTransform. `TransformPlugin` yerel→global
/// matrisi her kare propagate eder ama ilk render'dan ÖNCE çalışmadığından global'i elle
/// veririz (yoksa ilk kare identity konumda çizilir). Nested bundle olarak spawn_bundle'a girer.
fn placed(t: Transform) -> (Transform, GlobalTransform) {
    (
        t,
        GlobalTransform {
            matrix: t.local_matrix,
        },
    )
}

fn main() {
    gizmo::app::setup_panic_hook();
    println!("Volumetrik duman (T6) — raymarch katılımcı ortam.");
    App::<S>::new("Gizmo — Volumetrik Duman (Raymarch)", 1600, 900)
        .add_plugin(TransformPlugin)
        .set_setup(setup)
        .set_update(|_w, _s, _dt, _i| {})
        .set_render(|world, state, encoder, view, renderer, _lt| {
            // İlk frame: volumetrik duman hacmini oluştur + ayarla.
            if renderer.smoke.is_none() {
                let mut sm = SmokeVolume::new(
                    &renderer.device,
                    &renderer.scene.global_bind_group_layout,
                    wgpu::TextureFormat::Rgba16Float,
                );
                // Sim kutusu + kaynak (grid-tabanlı advekte edilen yoğunluk). Kutu yüksekliği
                // dumanın gerçekten DOLDURDUĞU hacme göre (4m) — çok yüksek kutu (6m) dumanı
                // kadraj içinde minik bir dilim gibi gösteriyordu.
                sm.bounds_min = [-1.8, 0.02, -1.8];
                sm.bounds_max = [1.8, 4.0, 1.8];
                sm.source = [0.0, 0.8, 0.0]; // taban yakınında kaynak
                sm.source_radius = 0.6;
                sm.inject = 9.0; // güçlü enjeksiyon → görünür yoğunluk (headless profille ayarlı)
                sm.dissipation = 0.985; // frame başına yoğunluk çarpanı (dağılma)
                sm.buoyancy = 1.7; // yükselir + kutuyu doldurur (0.55 ince disk yapıyordu)
                sm.curl_strength = 2.0; // kıvrılma/türbülans
                sm.curl_scale = 0.7;
                sm.absorption = 2.8;
                sm.density_scale = 1.6;
                sm.steps = 64;
                sm.color = [0.95, 0.96, 1.0];
                sm.ambient = 0.4;
                // CS2-tarzı: kaynaktan DIŞA doğru dolar (fill), sınırlı yarıçapta durur.
                sm.fill_strength = 2.5;
                sm.fill_radius = 2.0;
                // ENGELE-CONFORM: merkezdeki direği DELMEZ, etrafını sarar. Obstacle AABB'si
                // sahnedeki direk mesh'iyle BİREBİR aynı (bkz. setup: PILLAR_MIN/MAX).
                sm.set_obstacle_boxes(&renderer.queue, &[(PILLAR_MIN, PILLAR_MAX)]);
                renderer.smoke = Some(sm);
            }
            // Stüdyo ortamı; bu showcase'in KULLANMADIĞI ağır pass'ler kapalı (SmokeVolume
            // kendi raymarch'ını yapar; SSR/SSGI/volumetric/fluid gereksiz maliyet olurdu).
            renderer.environment_preset = 1;
            renderer.environment_preset_2 = 1;
            renderer.gpu_fluid = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            let _ = state.cam_id;
            default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("çalıştırılamadı");
}

fn setup(world: &mut World, renderer: &Renderer) -> S {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    // Zemin
    world.spawn_bundle((
        placed(Transform::new(Vec3::ZERO)),
        AssetManager::create_plane(&renderer.device, 40.0),
        Material::new(white.clone()).with_pbr(Vec4::new(0.13, 0.13, 0.15, 1.0), 0.85, 0.0),
        MeshRenderer::new(),
    ));

    // Merkezdeki DİREK — duman bunu delmeyip etrafını saracak (engele-conform gösterimi).
    // Transform, PILLAR_MIN/MAX ile birebir: birim küp (-0.5..0.5) → merkez=(min+max)/2,
    // ölçek=(max-min).
    let pillar_center = Vec3::new(
        (PILLAR_MIN[0] + PILLAR_MAX[0]) * 0.5,
        (PILLAR_MIN[1] + PILLAR_MAX[1]) * 0.5,
        (PILLAR_MIN[2] + PILLAR_MAX[2]) * 0.5,
    );
    let pillar_scale = Vec3::new(
        PILLAR_MAX[0] - PILLAR_MIN[0],
        PILLAR_MAX[1] - PILLAR_MIN[1],
        PILLAR_MAX[2] - PILLAR_MIN[2],
    );
    world.spawn_bundle((
        placed(Transform::new(pillar_center).with_scale(pillar_scale)),
        AssetManager::create_cube(&renderer.device),
        Material::new(white.clone()).with_pbr(Vec4::new(0.5, 0.5, 0.55, 1.0), 0.5, 0.1),
        MeshRenderer::new(),
    ));

    // Işık
    world.spawn_bundle((
        placed(Transform::new(Vec3::new(0.0, 10.0, 0.0)).with_rotation(
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        )),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.2, LightRole::Sun),
    ));

    // Kamera — konumdan hedefe bakacak şekilde yaw/pitch türetilir.
    let cam_id = {
        let pos = Vec3::new(6.0, 3.0, 7.0);
        let target = Vec3::new(0.0, 2.2, 0.0);
        let dir = (target - pos).normalize();
        let yaw = dir.z.atan2(dir.x);
        let pitch = dir.y.clamp(-1.0, 1.0).asin();
        world
            .spawn_bundle((
                placed(Transform::new(pos)),
                Camera::new(1.0, 0.1, 500.0, yaw, pitch, true),
            ))
            .id()
    };

    S { cam_id }
}
