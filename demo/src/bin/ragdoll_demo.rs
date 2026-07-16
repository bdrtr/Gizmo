//! Ragdoll showcase — humanoid iskeletler (RagdollBuilder::create_humanoid) yerden
//! yükseğe doğuyor, düşüyor ve YENİ CONE-TWIST + yumuşak (compliant) eklem limitleriyle
//! doğal biçimde savruluyor: uzuvlar kendi ekseninde serbest DÖNMÜYOR, hiperekstansiyona
//! GİRMİYOR, limitler yaylı hissettiriyor.
//!
//! **R** = yeniden başlat (kemikleri ilk pozlarına + başlangıç hızlarına sıfırlar).
//!
//! Her kemik fizik gövdesine (kapsül collider) render için bir kutu mesh takılır; fizik
//! Transform'u sürer, render `ensure_global_transforms` ile otomatik senkron eder.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

use gizmo::bundles::RigidBodyBundle;
use gizmo::physics::components::{Collider, RigidBody, Velocity};
use gizmo::physics::ragdoll::{spawn_ragdoll, RagdollBoneType, RagdollBuilder};
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, MeshRenderer};

/// Bir kemiğin yeniden-başlatma durumu: ilk poz + başlangıç hızı.
struct BoneReset {
    id: u32,
    transform: Transform,
    lin: Vec3,
    ang: Vec3,
}

struct RagdollDemo {
    resets: Vec<BoneReset>,
    prev_r: bool,
}

fn bone_color(bt: RagdollBoneType) -> Vec4 {
    use RagdollBoneType::*;
    match bt {
        Head => Vec4::new(0.95, 0.80, 0.65, 1.0),           // ten
        Pelvis | Torso => Vec4::new(0.20, 0.45, 0.85, 1.0), // gövde mavi
        _ => Vec4::new(0.85, 0.30, 0.25, 1.0),              // uzuvlar (kol/bacak) kırmızı
    }
}

fn setup(world: &mut World, renderer: &Renderer) -> RagdollDemo {
    let mut assets = AssetManager::new();
    let tex = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube = AssetManager::create_cube(&renderer.device);

    // Güneş
    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, 40.0, 15.0)).with_rotation(Quat::from_rotation_x(-0.95)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.2, LightRole::Sun),
    ));

    // Kamera
    let cam = Camera::new(FRAC_PI_3, 0.1, 500.0, -FRAC_PI_2, -0.25, true);
    world.spawn_bundle((Transform::new(Vec3::new(0.0, 4.5, 13.0)), cam));

    // Zemin (büyük statik kutu; üst yüzü y=0)
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.5, 0.0)).with_scale(Vec3::new(30.0, 0.5, 30.0)),
        cube.clone(),
        Material::new(tex.clone()).with_pbr(Vec4::new(0.28, 0.30, 0.34, 1.0), 0.9, 0.05),
        MeshRenderer::new(),
        RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::new(30.0, 0.5, 30.0))),
    ));

    // Yerçekimi dünyası (spawn_ragdoll eklemleri buraya iter)
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let mut resets: Vec<BoneReset> = Vec::new();

    // Birkaç ragdoll — farklı yükseklik/dönüşle düşsünler
    for k in 0..3usize {
        let root = Vec3::new(-4.0 + k as f32 * 4.0, 6.0 + k as f32 * 0.5, 0.0);
        let mut builder = RagdollBuilder::new(root);
        builder.create_humanoid();
        let defs = builder.build();
        let instance = spawn_ragdoll(world, defs.clone());

        let ang = Vec3::new(0.0, 0.0, (k as f32 - 1.0) * 1.6); // düşerken tumble
        let lin = Vec3::new((k as f32 - 1.0) * 0.6, 0.0, 0.0);

        for ((bt, entity), def) in instance.bones.iter().zip(defs.iter()) {
            world.add_component(*entity, cube.clone());
            world.add_component(
                *entity,
                Material::new(tex.clone()).with_pbr(bone_color(*bt), 0.55, 0.35),
            );
            world.add_component(*entity, MeshRenderer::new());

            // Kapsül (yarıçap r, Y boyunca uzunluk L) ≈ kutu (r, L/2+r, r) yarı-boyutları.
            let sx = def.radius.max(0.04);
            let sy = (def.length * 0.5 + def.radius).max(0.04);
            let sz = def.radius.max(0.04);

            let mut saved = Transform::new(Vec3::ZERO);
            {
                let mut ts = world.borrow_mut::<Transform>();
                if let Some(mut t) = ts.get_mut(entity.id()) {
                    t.scale = Vec3::new(sx, sy, sz);
                    t.update_local_matrix();
                    saved = *t;
                }
            }
            {
                let mut vs = world.borrow_mut::<Velocity>();
                if let Some(mut v) = vs.get_mut(entity.id()) {
                    *v = Velocity::new(lin).with_angular(ang);
                }
            }
            resets.push(BoneReset { id: entity.id(), transform: saved, lin, ang });
        }
    }

    RagdollDemo { resets, prev_r: false }
}

fn update(world: &mut World, state: &mut RagdollDemo, _dt: f32, input: &gizmo::core::input::Input) {
    let r = input.is_key_pressed(KeyCode::KeyR as u32);
    // Basış kenarı (tuşu basılı tutmak sürekli tetiklemesin).
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
                    *v = Velocity::new(b.lin).with_angular(b.ang);
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
    state.prev_r = r;
}

fn render(
    world: &mut World,
    _s: &RagdollDemo,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<RagdollDemo>::new("Gizmo — Ragdoll (R = yeniden başlat)", 1280, 720)
        .add_plugin(gizmo::plugins::PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
