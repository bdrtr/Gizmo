//! Newton's cradle — N elastik top bir sıra halinde kirişe **hinge** ile asılı.
//!
//! Gerçek fizik: her top X–Y düzleminde sallanır (hinge Z ekseni), çarpışmalar
//! elastik (restitution≈1, sürtünme≈0). Toplar dinlenerek başlar.
//!
//! **FARE İLE SÜRÜKLE-BIRAK:** sol tık ile bir topu yakala, arkın üzerinde
//! sürükle, bırak → salınır (bir topu geri çekip bırakmak gibi). Ekran→dünya
//! ışını için motora yeni eklenen `Camera::screen_to_ray` kullanılır (Bevy'nin
//! `viewport_to_world`'ü); ışın fizik `raycast`'iyle topu seçer, sürüklenen top
//! KİNEMATİK yapılır (fizik onu itmez ama komşuları o iter), bırakınca DİNAMİK'e
//! döner ve son hareketin hızını alır.
//!
//! İdiomatik ECS: `world.spawn_bundle((..tuple..))`. API eksikleri dosya sonunda.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::bundles::RigidBodyBundle;
use gizmo::core::input::mouse;
use gizmo::core::window::WindowInfo;
use gizmo::physics::components::{
    BodyType, Collider, CombineMode, PhysicsMaterial, RigidBody, Velocity,
};
use gizmo::physics::joints::Joint;
use gizmo::physics::raycast::Ray;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, MeshRenderer};

const N: usize = 5; // top sayısı
const R: f32 = 0.5; // top yarıçapı
const L: f32 = 4.0; // ip uzunluğu (pivot → top merkezi)
const PIVOT_Y: f32 = 6.0; // asma yüksekliği
const MASS: f32 = 1.0;
const GAP: f32 = 0.01; // toplar dinlenirken sadece değsin

/// Elastik (mükemmel yansıyan, sürtünmesiz) çarpışma malzemesi.
fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

/// Statik ön-görünüm kamerası + sürükleme durumu.
struct Cradle {
    balls: Vec<u32>,
    pivots: Vec<Vec3>,
    cam: Camera,
    cam_pos: Vec3,
    dragging: Option<usize>, // balls içindeki index
    target: Vec3,            // son ark hedefi (bırakma hızı için)
    target_prev: Vec3,
}

fn setup(world: &mut World, renderer: &Renderer) -> Cradle {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let sphere = AssetManager::create_sphere(&renderer.device, R, 32, 32);
    let cube = AssetManager::create_cube(&renderer.device);

    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, 40.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0, LightRole::Sun),
    ));

    let cam = Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.05, true);
    let cam_pos = Vec3::new(0.0, PIVOT_Y - L + 1.0, 11.0);
    world.spawn_bundle((Transform::new(cam_pos), cam));

    let spacing = 2.0 * R + GAP;
    let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

    // (Eski API-EKSİK #6 DÜZELTİLDİ: render artık GlobalTransform'u otomatik
    // backfill+sync ediyor → sadece Transform+Mesh+Material ile spawn yeter.)
    let beam = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0))
            .with_scale(Vec3::new(N as f32 * spacing + 1.0, 0.15, 0.15)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.15, 0.15, 0.18, 1.0), 0.4, 0.5),
        MeshRenderer::new(),
        RigidBodyBundle::static_body(),
    ));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut balls = Vec::new();
    let mut pivots = Vec::new();

    for i in 0..N {
        let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
        let center = pivot - Vec3::new(0.0, L, 0.0); // hepsi düz aşağı (dinlenme)
        let color = if i == 0 || i == N - 1 {
            Vec4::new(0.85, 0.15, 0.15, 1.0)
        } else {
            Vec4::new(0.82, 0.82, 0.86, 1.0)
        };

        // (Eski API-EKSİK #1 DÜZELTİLDİ: RigidBodyBundle artık collider'dan ataleti
        // otomatik türetiyor → elle calculate_sphere_inertia gerekmiyor.)
        let ball = world.spawn_bundle((
            Transform::new(center),
            sphere.clone(),
            Material::new(tex.clone()).with_pbr(color, 0.9, 0.2),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(MASS)
                .with_collider(Collider::sphere(R).with_material(elastic())),
        ));

        // API-EKSİK #2: joint'ler ECS bileşeni değil; PhysicsWorld.joints'e elle push.
        phys.joints.push(Joint::hinge(
            BodyHandle::from_id(beam.id()),
            BodyHandle::from_id(ball.id()),
            pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
            Vec3::new(0.0, L, 0.0),
            Vec3::Z,
        ));
        balls.push(ball.id());
        pivots.push(pivot);
    }

    world.insert_resource(phys);
    world.insert_resource(assets);
    Cradle { balls, pivots, cam, cam_pos, dragging: None, target: Vec3::ZERO, target_prev: Vec3::ZERO }
}

fn update(world: &mut World, state: &mut Cradle, dt: f32, input: &gizmo::core::input::Input) {
    // ── Fare ile sürükle-bırak ───────────────────────────────────────────────
    let viewport = world
        .get_resource::<WindowInfo>()
        .map(|w| (w.width, w.height))
        .unwrap_or((1280.0, 720.0));
    // Motora yeni eklenen unproject: ekran pikselinden dünya ışını.
    let ray = state.cam.screen_to_ray(input.mouse_position(), viewport, state.cam_pos);
    // gizmo_math::Ray SIMD Vec3A tutar; fizik Ray/matematik Vec3 ister → dönüştür.
    let (ro, rd) = (Vec3::from(ray.origin), Vec3::from(ray.direction));
    let lmb = input.is_mouse_button_pressed(mouse::LEFT);

    // Yakalama (basış anı): ışını topa raycast et.
    if lmb && state.dragging.is_none() {
        let hit_id = world
            .get_resource::<PhysicsWorld>()
            .and_then(|p| p.raycast(&Ray::new(ro, rd), 100.0))
            .map(|h| h.entity.id());
        if let Some(idx) = hit_id.and_then(|id| state.balls.iter().position(|&b| b == id)) {
            state.dragging = Some(idx);
            state.target = world.borrow::<Transform>().get(state.balls[idx]).map(|t| t.position).unwrap_or(Vec3::ZERO);
            state.target_prev = state.target;
            set_body_type(world, state.balls[idx], BodyType::Kinematic);
        }
    }

    if lmb {
        // Sürükleme: ışını salınım düzlemi (z=0) ile kesiştir, arka (yarıçap L) kilitle.
        if let Some(idx) = state.dragging {
            let pivot = state.pivots[idx];
            if rd.z.abs() > 1e-5 {
                let t = (pivot.z - ro.z) / rd.z;
                let p = ro + rd * t;
                let dir = (p - pivot).normalize_or_zero();
                let dir = if dir.length() > 0.5 { dir } else { Vec3::NEG_Y };
                let target = pivot + dir * L;
                let rot = Quat::from_rotation_arc(Vec3::Y, (pivot - target).normalize_or_zero());
                state.target_prev = state.target;
                state.target = target;

                let id = state.balls[idx];
                {
                    let mut ts = world.borrow_mut::<Transform>();
                    if let Some(mut tr) = ts.get_mut(id) {
                        tr.position = target;
                        tr.rotation = rot;
                        tr.update_local_matrix();
                    }
                }
                let mut vs = world.borrow_mut::<Velocity>();
                if let Some(mut v) = vs.get_mut(id) {
                    v.linear = Vec3::ZERO;
                    v.angular = Vec3::ZERO;
                }
            }
        }
    } else if let Some(idx) = state.dragging.take() {
        // Bırakma: dinamiğe dön, son sürükleme hareketinin hızını ver (fiske).
        let id = state.balls[idx];
        set_body_type(world, id, BodyType::Dynamic);
        let flick = ((state.target - state.target_prev) / dt.max(1e-4)).clamp_length_max(12.0);
        let mut vs = world.borrow_mut::<Velocity>();
        if let Some(mut v) = vs.get_mut(id) {
            v.linear = flick;
        }
    }

    // (Eski API-EKSİK #3 DÜZELTİLDİ: fizik artık PhysicsPlugin ile sabit-timestep
    //  schedule'da OTOMATİK adımlanıyor — burada elle adım YOK. Transform→Global
    //  senkronu da render'da otomatik — #6 düzeltildi.)
}

fn set_body_type(world: &mut World, id: u32, bt: BodyType) {
    let mut rbs = world.borrow_mut::<RigidBody>();
    if let Some(mut rb) = rbs.get_mut(id) {
        rb.body_type = bt;
        if bt == BodyType::Dynamic {
            rb.wake_up();
        }
    }
}

fn render(
    world: &mut World,
    _s: &Cradle,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Cradle>::new("Gizmo — Newton Sarkacı", 1280, 720)
        // Fizik artık sabit-timestep'te OTOMATİK adımlanıyor (elle adım yok).
        .add_plugin(gizmo::plugins::PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}

// ============================================================================
//  API EKSİKLERİ (yazarken karşılaştıklarım — düzeltilebilir):
//
//  #1  RigidBodyBundle collider'dan atalet türetmiyor → `calculate_sphere_inertia`
//      elle. Öneri: bundle apply/ilk adımda collider'dan otomatik türet.
//
//  #2  Joint'ler ECS bileşeni değil → `PhysicsWorld.joints`'e `BodyHandle::from_id`
//      ile elle push; entity despawn'da dangling. Öneri: `Joint` bileşen + toplama sistemi.
//
//  #3  Fizik zamanlanmış sistem değil → `cpu_physics_step_system` her update'te elle;
//      `PhysicsPlugin` adımı kaydetmiyor. Sabit-adım accumulator'ı da motor sağlamıyor.
//
//  #4  İki paralel asset sistemi → `MeshBundle` `Handle<Mesh>` ister ama `AssetManager`
//      doğrudan `Mesh` döner; uyumsuz.
//
//  #5  Restitution yalnız Collider malzemesinde; `RigidBody.restitution` okunmuyor.
//      AYRICA çözücü restitution'ı yeterince uygulamıyor (elastik çarpışma zayıf,
//      efektif ~0.1) — CCD/stack için bilinçli ödünler; ayrı bir çözücü işi.
//
//  #6  Mesh render için GlobalTransform ŞART + custom App'te propagate ELLE. Mesh
//      sorgusu `(&Mesh, &GlobalTransform, &Material)` (render/mod.rs) — GlobalTransform
//      yoksa nesne HİÇ çizilmez (kamera/ışıkta Transform fallback var, mesh'te yok);
//      fizik yalnız Transform yazdığından `TransformSync`+`Propagate`'i her update'te
//      elle koşmak gerekiyor. Öneri: mesh sorgusuna Transform fallback / otomatik backfill.
//
//  ARTI (bu turda EKLENDİ): `Camera::screen_to_ray(screen, viewport, world_pos)` —
//  ekran→dünya ışını (Bevy `viewport_to_world` karşılığı); picking/drag için gerekliydi,
//  eksikti. `gizmo_math::Ray::from_ndc` zaten vardı; üstüne ince kamera-wrapper + test.
// ============================================================================
