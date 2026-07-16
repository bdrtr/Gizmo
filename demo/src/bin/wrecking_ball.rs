//! # Yıkım Topu — rope eklemi + CCD + rijit istif (temiz sürüm)
//!
//! İp'e (`Joint::rope`) asılı AĞIR bir top (CCD açık) 70° geri çekili başlar; bırakılınca
//! salınıp bir kutu duvarını dağıtır. Bu tur eklem işini eğlenceli biçimde birleştirir:
//! rope eklemi + CCD (`with_ccd`) + rijit çarpışma + istif.
//!
//! Bu sürüm motorun yüksek-seviye olanaklarını kullanır; neyin motora neyin oyuna ait
//! olduğu konusunda dürüst olalım:
//!   * **`Prefab` + `auto_box_collider`** — KUTU DUVARI tek blueprint'ten; her tuğlanın box
//!     collider'ı spawn anında `Transform.scale`'den OTOMATİK türetilir (boyutu iki kez
//!     yazma yok). Renk per-örnek `with_pbr` ile. Top Prefab DEĞİL: küre collider'ı +
//!     kendine özel CCD/rope bağı olduğundan doğrudan `spawn_bundle`.
//!   * **`is_key_just_pressed`** — R kenar-tespiti motordan (elle `prev_r` bool takibi gitti).
//!   * **Yerinde reset (`despawn_all_with` DEĞİL)** — top + tuğlalar ilk transform/hız'a
//!     döndürülür ama SİLİNMEZ: rope eklemi topun entity-id'sine bağlı; despawn/respawn
//!     eklemi koparırdı. Bu yüzden bilinçli olarak gövdeleri yerinde sıfırlıyoruz (`resets`
//!     = ev pozları) — sahne-reset idiomunun eklem-güvenli varyantı.
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — motorun `with_scene_render()`
//!     tek-satır kısayolu VAR ama SSR/SSGI/volumetric/TAA'yı da kapatırdı; metalik topun
//!     yansımaları için bu efektleri AÇIK tutuyoruz. `gpu_physics` zaten motor-varsayılanı
//!     `None` (opt-in) olduğundan render'da state-mutasyonu GEREKMEZ.
//!
//! **R** = yeniden başlat (top + tüm kutular ilk pozlarına döner).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;

const BALL_R: f32 = 0.7;
const ROPE_LEN: f32 = 5.0;
const HALF: f32 = 0.25; // duvar tuğlası yarı-boyu

/// R'ye basınca gövdenin döndürüleceği "ev" pozu (id-anahtarlı yerinde reset).
struct BodyReset {
    id: u32,
    transform: Transform,
}

struct Wrecking {
    rope_vis: u32,
    ball: u32,
    pivot: Vec3,
    resets: Vec<BodyReset>,
}

fn setup(world: &mut World, renderer: &Renderer) -> Wrecking {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube = AssetManager::create_cube(&renderer.device);
    let sphere = AssetManager::create_sphere(&renderer.device, BALL_R, 28, 28);

    // Güneş + kamera + zemin
    world.spawn_bundle((
        Transform::new(Vec3::new(12.0, 40.0, 18.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.2, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(-1.0, 4.0, 13.0)),
        Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2 + 0.15, -0.18, true),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(30.0, 0.5, 30.0)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.28, 0.30, 0.34, 1.0), 0.9, 0.05),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(30.0, 0.5, 30.0))),
    ));

    // Yerçekimi + rope eklemini tutacak fizik dünyası (gövdeler ECS'ten senklenir, eklem burada).
    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut resets: Vec<BodyReset> = Vec::new();

    // Askı direği (görsel) + tepe kirişi — fiziksiz süs.
    let pivot = Vec3::new(0.0, 7.0, 0.0);
    let frame_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.35, 0.35, 0.4, 1.0), 0.5, 0.6);
    world.spawn_bundle((
        Transform::new(Vec3::new(-3.2, 3.5, 0.0)).with_scale(Vec3::new(0.12, 3.5, 0.12)),
        cube.clone(),
        frame_mat.clone(),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(-1.6, 7.0, 0.0)).with_scale(Vec3::new(1.7, 0.12, 0.12)),
        cube.clone(),
        frame_mat,
        MeshRenderer::new(),
    ));

    // Çapa (statik gövde, ipin bağlandığı görünmez nokta — Material/MeshRenderer yok).
    let anchor = world
        .spawn_bundle((
            Transform::new(pivot).with_scale(Vec3::splat(0.06)),
            cube.clone(),
            RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::splat(0.06))),
        ))
        .id();

    // AĞIR TOP — 70° geri çekili; CCD açık (hızlı+ağır, kutuları delmesin). Küre collider'ı +
    // kendine özel bağ → Prefab DEĞİL, doğrudan spawn_bundle.
    let a = 70.0_f32.to_radians();
    let ball_t = Transform::new(pivot + ROPE_LEN * Vec3::new(-a.sin(), -a.cos(), 0.0));
    let ball = world
        .spawn_bundle((
            ball_t,
            sphere.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.25, 0.26, 0.30, 1.0), 0.35, 0.9),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(20.0)
                .with_collider(Collider::sphere(BALL_R))
                .with_ccd(),
        ))
        .id();
    resets.push(BodyReset {
        id: ball,
        transform: ball_t,
    });
    phys.joints.push(Joint::rope(
        BodyHandle::from_id(anchor),
        BodyHandle::from_id(ball),
        Vec3::ZERO,
        Vec3::ZERO,
        ROPE_LEN,
    ));

    // İp görseli (her kare pivot↔top yüzeyine gerilir — update'te).
    let rope_vis = world
        .spawn_bundle((
            Transform::new(pivot),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.05, 0.05, 0.06, 1.0), 0.7, 0.2),
            MeshRenderer::new(),
        ))
        .id();

    // KUTU DUVARI (topun salınım düzleminde) — tek Prefab blueprint, collider Transform.scale'den.
    let brick = Prefab::new(cube.clone(), Material::new(tex.clone()))
        .with_body(RigidBodyBundle::dynamic(1.0))
        .auto_box_collider();
    let cols = [1.9_f32, 2.42, 2.94];
    let colors = [
        Vec4::new(0.85, 0.35, 0.25, 1.0),
        Vec4::new(0.9, 0.6, 0.2, 1.0),
        Vec4::new(0.8, 0.75, 0.3, 1.0),
    ];
    for (ci, &cx) in cols.iter().enumerate() {
        for row in 0..7 {
            let y = HALF + row as f32 * (2.0 * HALF);
            let t = Transform::new(Vec3::new(cx, y, 0.0)).with_scale(Vec3::splat(HALF));
            let color = colors[(ci + row) % colors.len()];
            let e = brick.clone().with_pbr(color, 0.6, 0.3).spawn(world, t);
            resets.push(BodyReset {
                id: e.id(),
                transform: t,
            });
        }
    }

    world.insert_resource(phys);
    Wrecking {
        rope_vis,
        ball,
        pivot,
        resets,
    }
}

fn update(world: &mut World, state: &mut Wrecking, _dt: f32, input: &Input) {
    // R = yeniden başlat — kenar-tespiti motorun is_key_just_pressed API'sinden (elle prev_r yok).
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        // Gövdeleri yerinde sıfırla: transform → ev pozu, hız → 0, uyuyanları uyandır.
        // Rope eklemi ball entity-id'sine bağlı olduğundan despawn/respawn YOK.
        {
            let mut ts = world.borrow_mut::<Transform>();
            for b in &state.resets {
                if let Some(mut t) = ts.get_mut(b.id) {
                    *t = b.transform;
                }
            }
        }
        {
            let mut vs = world.borrow_mut::<Velocity>();
            for b in &state.resets {
                if let Some(mut v) = vs.get_mut(b.id) {
                    *v = Velocity::default();
                }
            }
        }
        {
            let mut rbs = world.borrow_mut::<RigidBody>();
            for b in &state.resets {
                if let Some(mut rb) = rbs.get_mut(b.id) {
                    rb.wake_up();
                }
            }
        }
    }

    // İp görselini pivot↔top yüzeyi arasına ger (gerçek özel iş — her kare).
    let ball_pos = world
        .borrow::<Transform>()
        .get(state.ball)
        .map(|t| t.position);
    if let Some(bp) = ball_pos {
        let seg = state.pivot - bp;
        let len = (seg.length() - BALL_R).max(0.0);
        let dir = seg.normalize_or_zero();
        let surface = bp + dir * BALL_R;
        let mut ts = world.borrow_mut::<Transform>();
        if let Some(mut tr) = ts.get_mut(state.rope_vis) {
            tr.position = surface + dir * (len * 0.5);
            tr.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
            tr.scale = Vec3::new(0.035, len * 0.5, 0.035);
            tr.update_local_matrix();
        }
    }
}

fn render(
    world: &mut World,
    _s: &Wrecking,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Wrecking>::new("Gizmo — Yıkım Topu (R = yeniden başlat)", 1280, 720)
        .add_plugin(PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
