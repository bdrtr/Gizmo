//! Yıkım topu — ip'e (Joint::rope) asılı AĞIR bir top (CCD açık) geri çekili başlar,
//! bırakılınca salınıp bir kutu duvarını dağıtır. Bu turdaki eklem işini eğlenceli
//! biçimde birleştirir: rope eklemi + CCD (with_ccd) + rijit çarpışma + istif.
//!
//! **R** = yeniden başlat (top + tüm kutular ilk pozlarına döner).

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::bundles::RigidBodyBundle;
use gizmo::physics::components::Collider;
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, MeshRenderer};

struct BodyReset {
    id: u32,
    transform: Transform,
}

struct Wrecking {
    rope_vis: u32,
    ball: u32,
    pivot: Vec3,
    resets: Vec<BodyReset>,
    prev_r: bool,
}

const BALL_R: f32 = 0.7;
const ROPE_LEN: f32 = 5.0;

fn setup(world: &mut World, renderer: &Renderer) -> Wrecking {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube = AssetManager::create_cube(&renderer.device);
    let sphere = AssetManager::create_sphere(&renderer.device, BALL_R, 28, 28);

    // Işık + kamera + zemin
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
        RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::new(30.0, 0.5, 30.0))),
    ));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut resets: Vec<BodyReset> = Vec::new();

    // Askı direği (görsel) + tepe kirişi
    let pivot = Vec3::new(0.0, 7.0, 0.0);
    world.spawn_bundle((
        Transform::new(Vec3::new(-3.2, 3.5, 0.0)).with_scale(Vec3::new(0.12, 3.5, 0.12)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.35, 0.35, 0.4, 1.0), 0.5, 0.6),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::new(-1.6, 7.0, 0.0)).with_scale(Vec3::new(1.7, 0.12, 0.12)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.35, 0.35, 0.4, 1.0), 0.5, 0.6),
        MeshRenderer::new(),
    ));

    // Çapa (statik gövde, ipin bağlandığı nokta)
    let anchor = spawn_static_anchor(world, &cube, pivot);

    // AĞIR TOP — 70° geri çekili; CCD açık (hızlı+ağır, kutuları delmesin).
    let a = 70.0_f32.to_radians();
    let ball_pos = pivot + ROPE_LEN * Vec3::new(-a.sin(), -a.cos(), 0.0);
    let ball_t = Transform::new(ball_pos);
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
    resets.push(BodyReset { id: ball, transform: ball_t });
    phys.joints.push(Joint::rope(
        BodyHandle::from_id(anchor),
        BodyHandle::from_id(ball),
        Vec3::ZERO,
        Vec3::ZERO,
        ROPE_LEN,
    ));

    // İp görseli
    let rope_vis = world
        .spawn_bundle((
            Transform::new(pivot),
            cube.clone(),
            Material::new(tex.clone()).with_pbr(Vec4::new(0.05, 0.05, 0.06, 1.0), 0.7, 0.2),
            MeshRenderer::new(),
        ))
        .id();

    // KUTU DUVARI (topun salınım düzleminde, sağda)
    let half = 0.25_f32;
    let cols = [1.9_f32, 2.42, 2.94];
    let colors = [
        Vec4::new(0.85, 0.35, 0.25, 1.0),
        Vec4::new(0.9, 0.6, 0.2, 1.0),
        Vec4::new(0.8, 0.75, 0.3, 1.0),
    ];
    for (ci, &cx) in cols.iter().enumerate() {
        for row in 0..7 {
            let y = half + row as f32 * (2.0 * half);
            let t = Transform::new(Vec3::new(cx, y, 0.0)).with_scale(Vec3::splat(half));
            let color = colors[(ci + row) % colors.len()];
            let b = world
                .spawn_bundle((
                    t,
                    cube.clone(),
                    Material::new(tex.clone()).with_pbr(color, 0.6, 0.3),
                    MeshRenderer::new(),
                    RigidBodyBundle::dynamic(1.0).with_collider(Collider::box_collider(Vec3::splat(half))),
                ))
                .id();
            resets.push(BodyReset { id: b, transform: t });
        }
    }

    world.insert_resource(phys);
    Wrecking { rope_vis, ball, pivot, resets, prev_r: false }
}

fn spawn_static_anchor(world: &mut World, cube: &gizmo::renderer::components::Mesh, pos: Vec3) -> u32 {
    world
        .spawn_bundle((
            Transform::new(pos).with_scale(Vec3::splat(0.06)),
            cube.clone(),
            RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::splat(0.06))),
        ))
        .id()
}

fn update(world: &mut World, state: &mut Wrecking, _dt: f32, input: &gizmo::core::input::Input) {
    // R = yeniden başlat (kenar-algılamalı)
    let r = input.is_key_pressed(KeyCode::KeyR as u32);
    if r && !state.prev_r {
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
            let mut rbs = world.borrow_mut::<gizmo::physics::components::RigidBody>();
            for b in &state.resets {
                if let Some(mut rb) = rbs.get_mut(b.id) {
                    rb.wake_up();
                }
            }
        }
    }
    state.prev_r = r;

    // İp görselini pivot↔top yüzeyi arasına ger.
    let ball_pos = world.borrow::<Transform>().get(state.ball).map(|t| t.position);
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
    renderer.gpu_physics = None;
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Wrecking>::new("Gizmo — Yıkım Topu (R = yeniden başlat)", 1280, 720)
        .add_plugin(gizmo::plugins::PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
