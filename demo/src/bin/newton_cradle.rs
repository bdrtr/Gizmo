//! # Newton Sarkacı — N elastik top kirişe **ip eklemiyle** asılı
//!
//! Gerçek fizik: her top X–Y düzleminde sallanır, çarpışmalar elastik
//! (restitution≈1, sürtünme≈0). Toplar dinlenerek başlar; en soldaki top
//! geri-çekilmiş doğar ve açılışta salınıma girer.
//!
//! Bu sürüm motorun modern olanaklarını kullanır; NEYİN motora NEYİN oyuna ait
//! olduğu konusunda dürüst olalım:
//!   * **Toplar = `spawn_bundle` + explicit `Collider::sphere`** — Prefab DEĞİL:
//!     Prefab yalnız kutu-collider verir ve her örneğin kendi başlangıç açısı/hızı
//!     olduğundan (Prefab bunları gömemez) doğrudan bundle ile spawn edilir.
//!   * **İp = `Joint::rope` (motorda birinci-sınıf)** — esnemez ama gevşeyebilir
//!     (dist ≤ L). Ankor A = kiriş pivotu, B = top merkezi. Elle konum kırpma HİLESİ yok.
//!   * **Görsel ip = fiziksiz ince çubuk** — her kare topun konumuna göre gerilir.
//!   * **Sürükleme = `Camera::screen_to_ray` + fizik `raycast`** — sol tıkla topu seç,
//!     KİNEMATİK yap (komşuları iter), bırakınca DİNAMİK'e dön ve servo hızını taşı.
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — `with_scene_render()` kısayolu
//!     SSR/SSGI/volumetric/TAA'yı kapatırdı; bu sahne yansımaları/keskinliği ister.
//!
//! Bu demoda geçici/uçan varlık (mermi/konfeti) ve sahne sıfırlama yok → dolayısıyla
//! `DespawnAfter`/`despawn_all_with` idiomları uygulanmaz.
//!
//! ## Kontroller
//!   * **Sol tık + sürükle** — bir topu yakala, ark üzerinde sürükle, bırak → salınır.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::core::input::mouse;
use gizmo::physics::components::{BodyType, CombineMode, PhysicsMaterial};
use gizmo::physics::joints::Joint;
use gizmo::physics::raycast::Ray;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;

// ------------------------------------------------------------------ ayarlar
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
    ropes: Vec<u32>, // her topa karşılık gelen görsel ip (fiziksiz ince çubuk)
    pivots: Vec<Vec3>,
    cam: Camera,
    cam_pos: Vec3,
    dragging: Option<usize>, // balls içindeki index
}

// --------------------------------------------------------------- setup
fn setup(world: &mut World, renderer: &Renderer) -> Cradle {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let sphere = AssetManager::create_sphere(&renderer.device, R, 32, 32);
    let cube = AssetManager::create_cube(&renderer.device);

    // Güneş
    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, 40.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0, LightRole::Sun),
    ));

    // Sabit ön-görünüm kamerası
    let cam = Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.05, true);
    let cam_pos = Vec3::new(0.0, PIVOT_Y - L + 1.0, 11.0);
    world.spawn_bundle((Transform::new(cam_pos), cam));

    let spacing = 2.0 * R + GAP;
    let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

    // Üst kiriş (statik): ipler buradaki sabit pivotlara bağlanır.
    let beam = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0)).with_scale(Vec3::new(
            N as f32 * spacing + 1.0,
            0.15,
            0.15,
        )),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.15, 0.15, 0.18, 1.0), 0.4, 0.5),
        MeshRenderer::new(),
        RigidBodyBundle::static_body(),
    ));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut balls = Vec::new();
    let mut ropes = Vec::new();
    let mut pivots = Vec::new();

    for i in 0..N {
        let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
        // Top 0 geri-çekilmiş doğar (pivottan L uzakta) → AÇILIŞTA salınır.
        let (center, rot) = if i == 0 {
            let a = 55.0_f32.to_radians();
            (
                pivot + L * Vec3::new(-a.sin(), -a.cos(), 0.0),
                Quat::from_rotation_z(-a),
            )
        } else {
            (pivot - Vec3::new(0.0, L, 0.0), Quat::IDENTITY)
        };
        let color = if i == 0 || i == N - 1 {
            Vec4::new(0.85, 0.15, 0.15, 1.0)
        } else {
            Vec4::new(0.82, 0.82, 0.86, 1.0)
        };

        // Küre gövde: collider'dan atalet otomatik türetilir, malzeme elastik.
        let ball = world.spawn_bundle((
            Transform::new(center).with_rotation(rot),
            sphere.clone(),
            Material::new(tex.clone()).with_pbr(color, 0.9, 0.2),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(MASS)
                .with_collider(Collider::sphere(R).with_material(elastic())),
        ));

        // İp eklemi: gevşekken serbest düşer, gerilince yakalar (dist ≤ L).
        phys.joints.push(Joint::rope(
            BodyHandle::from_id(beam.id()),
            BodyHandle::from_id(ball.id()),
            pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
            Vec3::ZERO,
            L,
        ));
        // Görsel ip: fiziksiz ince çubuk (pivot↔top), her kare `update`'te konumlanır.
        let rope = world.spawn_bundle((
            Transform::new(pivot),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.05, 0.05, 0.06, 1.0), 0.7, 0.2),
            MeshRenderer::new(),
        ));

        balls.push(ball.id());
        ropes.push(rope.id());
        pivots.push(pivot);
    }

    world.insert_resource(phys);
    world.insert_resource(assets);
    Cradle {
        balls,
        ropes,
        pivots,
        cam,
        cam_pos,
        dragging: None,
    }
}

// --------------------------------------------------------------- update
fn update(world: &mut World, state: &mut Cradle, _dt: f32, input: &Input) {
    // ── Fare ile sürükle-bırak ───────────────────────────────────────────────
    let viewport = world
        .get_resource::<WindowInfo>()
        .map(|w| (w.width, w.height))
        .unwrap_or((1280.0, 720.0));
    // Ekran pikselinden dünya ışını (unproject).
    let ray = state
        .cam
        .screen_to_ray(input.mouse_position(), viewport, state.cam_pos);
    // screen_to_ray SIMD `Ray` döner; fizik Ray/matematik Vec3 ister → dönüştür.
    let (ro, rd) = (Vec3::from(ray.origin), Vec3::from(ray.direction));
    let lmb = input.is_mouse_button_pressed(mouse::LEFT);

    // Yakalama: LMB basılıyken ışını topa raycast et (henüz sürüklenmiyorsa).
    if lmb && state.dragging.is_none() {
        let hit_id = world
            .get_resource::<PhysicsWorld>()
            .and_then(|p| p.raycast(&Ray::new(ro, rd), 100.0))
            .map(|h| h.entity.id());
        if let Some(idx) = hit_id.and_then(|id| state.balls.iter().position(|&b| b == id)) {
            state.dragging = Some(idx);
            set_body_type(world, state.balls[idx], BodyType::Kinematic);
        }
    }

    if lmb {
        // Sürükleme: ışını salınım düzlemi (z=0) ile kesiştir → fare noktası.
        if let Some(idx) = state.dragging {
            let pivot = state.pivots[idx];
            if rd.z.abs() > 1e-5 {
                let t = (pivot.z - ro.z) / rd.z;
                let p = ro + rd * t;
                // Top fareyi düzlemde TAKİP eder; ip boyunu (L) aşmasın diye mesafe ≤ L kırpılır.
                let from_pivot = p - pivot;
                let dist = from_pivot.length();
                let target = if dist > L {
                    pivot + from_pivot / dist * L
                } else {
                    p
                };

                let id = state.balls[idx];
                // Hedefe SABİT-KAZANÇLI hız servosu ile sür (dt'ye bölme YOK). Kinematik cisim
                // konumu hızından entegre eder → komşularla çarpışıp onları iter. Sabit ılımlı
                // kazanç dt tutarsızlığına bağışık ve pürüzsüz takip eder.
                let cur = world
                    .borrow::<Transform>()
                    .get(id)
                    .map(|t| t.position)
                    .unwrap_or(target);
                const DRAG_GAIN: f32 = 18.0;
                let vel = ((target - cur) * DRAG_GAIN).clamp_length_max(15.0);
                let mut vs = world.borrow_mut::<Velocity>();
                if let Some(mut v) = vs.get_mut(id) {
                    v.linear = vel;
                    v.angular = Vec3::ZERO;
                }
            }
        }
    } else if let Some(idx) = state.dragging.take() {
        // Bırakma: dinamiğe dön; servo hızı doğal fiske olarak kalır (patlama yok).
        let id = state.balls[idx];
        set_body_type(world, id, BodyType::Dynamic);
        let mut vs = world.borrow_mut::<Velocity>();
        if let Some(mut v) = vs.get_mut(id) {
            v.linear = v.linear.clamp_length_max(12.0);
        }
    }

    // ── Görsel ipleri toplara bağla ──────────────────────────────────────────
    // Fizik (schedule'da) bu kareden önce koştu → top konumları güncel. İpi pivot
    // ile topun pivota bakan yüzeyi arasına gerilmiş ince çubuk yap.
    let centers: Vec<Vec3> = {
        let ts = world.borrow::<Transform>();
        state
            .balls
            .iter()
            .map(|&b| ts.get(b).map(|t| t.position).unwrap_or(Vec3::ZERO))
            .collect()
    };
    let mut ts = world.borrow_mut::<Transform>();
    for (i, &rope) in state.ropes.iter().enumerate() {
        let pivot = state.pivots[i];
        let seg = pivot - centers[i]; // topun merkezinden pivota
        let len = (seg.length() - R).max(0.0); // yüzeyden pivota (topun içine girmesin)
        let dir = seg.normalize_or_zero();
        let surface = centers[i] + dir * R;
        if let Some(mut tr) = ts.get_mut(rope) {
            tr.position = surface + dir * (len * 0.5);
            tr.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
            tr.scale = Vec3::new(0.03, len * 0.5, 0.03);
            tr.update_local_matrix();
        }
    }
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

// --------------------------------------------------------------- render + main
// `default_render_pass` DOĞRUDAN: SSR/SSGI/volumetric/TAA'yı AÇIK tutar
// (`with_scene_render()` kısayolu bunları kapatırdı). gpu_physics motor-varsayılanı
// zaten None olduğundan render'da state-mutasyonu gerekmez.
fn render(
    world: &mut World,
    _s: &Cradle,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Cradle>::new("Gizmo — Newton Sarkacı", 1280, 720)
        .add_plugin(PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
