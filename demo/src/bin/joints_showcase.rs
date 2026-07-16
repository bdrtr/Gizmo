//! # Eklem vitrini — motorun eklem (joint) sistemini CANLI gösterir (temiz sürüm)
//!
//! Soldan sağa dört istasyon, her biri statik çapa + dinamik gövde + tek eklem:
//!   1) **ROPE** (`Joint::rope`, x=-6) — 60° geri çekili top bir iple sarkaç gibi sallanır
//!      (ip ince görsel çubuktur, `update`'te pivot↔top yüzeyi arasına gerilir).
//!   2) **TORSİYONEL YAY** (`Joint::hinge` + torsional spring, x=-2) — paçavra Z ekseninde
//!      dinlenme açısına (0.9 rad) yaylanıp söner.
//!   3) **D6 SERVO KOL** (`Joint::d6` + açısal drive, x=2) — kol hedef açıya sürülür; hedef
//!      `update`'te salınır → robot kolu gibi süpürür.
//!   4) **SÜSPANSİYON** (`Joint::slider` + yay, x=6) — kutu dikey yay-kızakta zıplayıp söner.
//!
//! NEYİN motora, NEYİN demoya ait olduğu konusunda dürüst olalım:
//!   * **`Prefab` + `auto_box_collider`** — dört STATİK çapa direği tek blueprint'ten; kutu
//!     collider `Transform.scale`'den OTOMATİK türetilir (boyut bir kez). Direkler tekrar eden
//!     aynı kutu → Prefab'ın tam yeri.
//!   * **Dinamik eklem gövdeleri = doğrudan `spawn_bundle`** — top KÜRE, kol/paçavra/platform
//!     her biri kendine özgü (materyal/ölçek/başlangıç hızı farklı) TEK örnek; Prefab hız
//!     gömemez ve yalnız kutu verir → bunlar açık `Collider` ile spawn_bundle.
//!   * **Eklemler elle kurulan bir `PhysicsWorld` kaynağına** yığılır — demonun ASIL konusu bu.
//!     Fizik Transform'u sürer, render `ensure_global_transforms` ile otomatik senkron eder.
//!   * **Render = `default_render_pass` DOĞRUDAN** — `with_scene_render()` kısayolu SSR/SSGI/
//!     volumetric/TAA'yı kapatırdı; bu vitrin efektleri AÇIK ister.
//!
//! Geçici/uçan varlık, sahne-sıfırlama ve kullanıcı girdisi olmadığından `DespawnAfter` /
//! `despawn_all_with` / `is_key_just_*` idiomları burada geçerli değil (kasıtlı olarak yok).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::physics::joints::{D6Drive, D6Motion, Joint, JointData};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;

struct Showcase {
    rope_vis: u32,
    rope_ball: u32,
    rope_pivot: Vec3,
    servo_idx: usize,
    time: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> Showcase {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube = AssetManager::create_cube(&renderer.device);
    let sphere = AssetManager::create_sphere(&renderer.device, 0.4, 24, 24);

    // Statik çapa direği blueprint'i: collider her spawn'da Transform.scale'den türetilir.
    let post = Prefab::new(
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.65, 1.0), 0.5, 0.4),
    )
    .with_body(RigidBodyBundle::static_body())
    .auto_box_collider();
    let spawn_post = |world: &mut World, pos: Vec3, half: f32| -> u32 {
        post.spawn(world, Transform::new(pos).with_scale(Vec3::splat(half)))
            .id()
    };

    // Güneş + kamera + zemin
    world.spawn_bundle((
        Transform::new(Vec3::new(15.0, 40.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.9)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.2, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 3.0, 15.0)),
        Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.12, true),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -1.5, 0.0)).with_scale(Vec3::new(30.0, 0.5, 30.0)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.28, 0.30, 0.34, 1.0), 0.9, 0.05),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(30.0, 0.5, 30.0))),
    ));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    // ── 1) ROPE pendulum (x=-6) ────────────────────────────────────────────
    let rope_pivot = Vec3::new(-6.0, 5.0, 0.0);
    let rope_len = 2.6_f32;
    let post1 = spawn_post(world, rope_pivot, 0.12);
    // Top 60° geri çekili başlar → ip gerilip sallanır.
    let a = 60.0_f32.to_radians();
    let ball_pos = rope_pivot + rope_len * Vec3::new(-a.sin(), -a.cos(), 0.0);
    let rope_ball = world
        .spawn_bundle((
            Transform::new(ball_pos),
            sphere.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.85, 0.2, 0.2, 1.0), 0.4, 0.3),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(1.0).with_collider(Collider::sphere(0.4)),
        ))
        .id();
    phys.joints.push(Joint::rope(
        BodyHandle::from_id(post1),
        BodyHandle::from_id(rope_ball),
        Vec3::ZERO,
        Vec3::ZERO,
        rope_len,
    ));
    let rope_vis = world
        .spawn_bundle((
            Transform::new(rope_pivot),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.05, 0.05, 0.06, 1.0), 0.7, 0.2),
            MeshRenderer::new(),
        ))
        .id();

    // ── 2) TORSİYONEL YAY paçavra (x=-2) ───────────────────────────────────
    let pivot2 = Vec3::new(-2.0, 4.5, 0.0);
    let post2 = spawn_post(world, pivot2, 0.12);
    // Paçavra: pivotun altında; hinge Z; torsional spring rest 0.9 → o açıya yaylanır.
    let paddle = world
        .spawn_bundle((
            Transform::new(pivot2 - Vec3::new(0.0, 1.0, 0.0)).with_scale(Vec3::new(0.12, 1.0, 0.5)),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.7, 0.3, 1.0), 0.5, 0.4),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(1.0)
                .with_collider(Collider::box_collider(Vec3::new(0.12, 1.0, 0.5))),
        ))
        .id();
    let mut hinge = Joint::hinge(
        BodyHandle::from_id(post2),
        BodyHandle::from_id(paddle),
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::Z,
    );
    if let JointData::Hinge(ref mut d) = hinge.data {
        d.use_torsional_spring = true;
        d.torsional_stiffness = 60.0;
        d.torsional_damping = 6.0;
        d.rest_angle = 0.9;
    }
    phys.joints.push(hinge);

    // ── 3) D6 SERVO KOL (x=2) ──────────────────────────────────────────────
    let pivot3 = Vec3::new(2.0, 4.5, 0.0);
    let post3 = spawn_post(world, pivot3, 0.14);
    let arm = world
        .spawn_bundle((
            Transform::new(pivot3 - Vec3::new(0.0, 1.0, 0.0))
                .with_scale(Vec3::new(0.15, 1.0, 0.15)),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.95, 0.7, 0.1, 1.0), 0.5, 0.6),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(1.0)
                .with_collider(Collider::box_collider(Vec3::new(0.15, 1.0, 0.15))),
        ))
        .id();
    let mut d6 = Joint::d6(
        BodyHandle::from_id(post3),
        BodyHandle::from_id(arm),
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
    );
    if let JointData::D6(ref mut d) = d6.data {
        d.angular[2] = D6Motion::Free; // Z serbest
        d.angular_drives[2] = D6Drive {
            enabled: true,
            stiffness: 120.0,
            damping: 18.0,
            target_position: 0.0,
            target_velocity: 0.0,
            max_force: 2000.0,
        };
    }
    let servo_idx = phys.joints.len();
    phys.joints.push(d6);

    // ── 4) SÜSPANSİYON (x=6) ───────────────────────────────────────────────
    let pivot4 = Vec3::new(6.0, 5.0, 0.0);
    let post4 = spawn_post(world, pivot4, 0.12);
    let platform = world
        .spawn_bundle((
            Transform::new(pivot4 - Vec3::new(0.0, 1.5, 0.0)).with_scale(Vec3::new(0.6, 0.2, 0.6)),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.6, 0.4, 0.85, 1.0), 0.5, 0.5),
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(1.0)
                .with_collider(Collider::box_collider(Vec3::new(0.6, 0.2, 0.6)))
                .with_velocity(Vec3::new(0.0, -6.0, 0.0)), // aşağı it → zıplasın
        ))
        .id();
    let mut slider = Joint::slider(
        BodyHandle::from_id(post4),
        BodyHandle::from_id(platform),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::Y,
    );
    if let JointData::Slider(ref mut d) = slider.data {
        d.use_spring = true;
        d.spring_stiffness = 120.0;
        d.spring_damping = 6.0;
        d.spring_rest_position = -1.5; // yay dinlenme uzunluğu
    }
    phys.joints.push(slider);

    world.insert_resource(phys);

    Showcase {
        rope_vis,
        rope_ball,
        rope_pivot,
        servo_idx,
        time: 0.0,
    }
}

fn update(world: &mut World, state: &mut Showcase, dt: f32, _input: &Input) {
    state.time += dt;

    // D6 servo hedefini salındır → kol süpürür.
    let target = 1.1 * (state.time * 1.1).sin();
    if let Some(mut pw) = world.get_resource_mut::<PhysicsWorld>() {
        if let JointData::D6(ref mut d) = pw.joints[state.servo_idx].data {
            d.angular_drives[2].target_position = target;
        }
    }

    // İp görselini pivot↔top yüzeyi arasına ger.
    let ball = world
        .borrow::<Transform>()
        .get(state.rope_ball)
        .map(|t| t.position);
    if let Some(ball) = ball {
        let seg = state.rope_pivot - ball;
        let len = (seg.length() - 0.4).max(0.0);
        let dir = seg.normalize_or_zero();
        let surface = ball + dir * 0.4;
        let mut ts = world.borrow_mut::<Transform>();
        if let Some(mut tr) = ts.get_mut(state.rope_vis) {
            tr.position = surface + dir * (len * 0.5);
            tr.rotation = Quat::from_rotation_arc(Vec3::Y, dir);
            tr.scale = Vec3::new(0.03, len * 0.5, 0.03);
            tr.update_local_matrix();
        }
    }
}

fn render(
    world: &mut World,
    _s: &Showcase,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Showcase>::new(
        "Gizmo — Eklem Vitrini (rope / torsional / D6 servo / süspansiyon)",
        1280,
        720,
    )
    .add_plugin(PhysicsPlugin::new())
    .set_setup(setup)
    .set_update(update)
    .set_render(render)
    .run()
    .expect("uygulama çalıştırılamadı");
}
