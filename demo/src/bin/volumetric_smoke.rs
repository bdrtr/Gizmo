//! Volumetrik duman demosu (T6) — GERÇEK katılımcı-ortam raymarch (billboard DEĞİL).
//! Motorun yeni `SmokeVolume`'u sınırlı bir kutu içinde animasyonlu 3B fBm yoğunluğunu ışın
//! boyunca yürütür (Beer-Lambert + güneş saçılımı + sahne-derinliği occlusion) ve HDR'ye
//! kompozit eder. Çalıştır: `cargo run -p demo --bin volumetric_smoke`

use gizmo::app::App;
use gizmo::core::world::World;
use gizmo::math::{Quat, Vec3, Vec4};
use gizmo::physics::components::GlobalTransform;
use gizmo::physics::Transform;
use gizmo::plugins::TransformPlugin;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, DirectionalLight, LightRole, Material, MeshRenderer};
use gizmo::renderer::gpu_smoke::SmokeVolume;

struct S {
    cam_id: u32,
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
                    gizmo::wgpu::TextureFormat::Rgba16Float,
                );
                // Sim kutusu + kaynak (grid-tabanlı advekte edilen yoğunluk).
                sm.bounds_min = [-1.8, 0.02, -1.8];
                sm.bounds_max = [1.8, 6.0, 1.8];
                sm.source = [0.0, 0.5, 0.0]; // taban yakınında kaynak
                sm.source_radius = 0.7;
                sm.inject = 5.0; // enjeksiyon hızı
                sm.dissipation = 0.99; // frame başına yoğunluk çarpanı (dağılma) — yüksek→plume yükselir
                sm.buoyancy = 1.3; // yukarı sürükleme
                sm.curl_strength = 1.8; // kıvrılma
                sm.curl_scale = 0.7;
                sm.absorption = 2.6;
                sm.density_scale = 1.5;
                sm.steps = 64;
                sm.color = [0.95, 0.96, 1.0];
                sm.ambient = 0.4;
                renderer.smoke = Some(sm);
            }
            renderer.environment_preset = 1;
            renderer.environment_preset_2 = 1;
            renderer.gpu_fluid = None;
            renderer.gpu_physics = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            let _ = state.cam_id;
            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("çalıştırılamadı");
}

fn setup(world: &mut World, renderer: &gizmo::renderer::Renderer) -> S {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    // Zemin
    {
        let mesh = AssetManager::create_plane(&renderer.device, 40.0);
        let e = world.spawn();
        let t = Transform::new(Vec3::ZERO);
        world.add_component(e, t);
        world.add_component(e, GlobalTransform { matrix: t.local_matrix });
        world.add_component(e, mesh);
        world.add_component(
            e,
            Material::new(white.clone()).with_pbr(Vec4::new(0.13, 0.13, 0.15, 1.0), 0.85, 0.0),
        );
        world.add_component(e, MeshRenderer::new());
    }
    // Occlusion için bir kutu (duman arkasına girsin/önünde kalsın gösterimi)
    {
        let mesh = AssetManager::create_cube(&renderer.device);
        let e = world.spawn();
        let mut t = Transform::new(Vec3::new(1.4, 0.7, 0.8));
        t.scale = Vec3::new(1.0, 1.4, 1.0);
        t.update_local_matrix();
        world.add_component(e, t);
        world.add_component(e, GlobalTransform { matrix: t.local_matrix });
        world.add_component(e, mesh);
        world.add_component(
            e,
            Material::new(white.clone()).with_pbr(Vec4::new(0.55, 0.2, 0.2, 1.0), 0.4, 0.1),
        );
        world.add_component(e, MeshRenderer::new());
    }
    // Işık
    {
        let e = world.spawn();
        let t = Transform::new(Vec3::new(0.0, 10.0, 0.0)).with_rotation(
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_4)
                * Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        );
        world.add_component(e, t);
        world.add_component(e, GlobalTransform { matrix: t.local_matrix });
        world.add_component(
            e,
            DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.2, LightRole::Sun),
        );
    }
    // Kamera
    let cam_id = {
        let e = world.spawn();
        let pos = Vec3::new(6.0, 3.0, 7.0);
        let target = Vec3::new(0.0, 2.2, 0.0);
        let dir = (target - pos).normalize();
        let yaw = dir.z.atan2(dir.x);
        let pitch = dir.y.clamp(-1.0, 1.0).asin();
        let t = Transform::new(pos);
        world.add_component(e, t);
        world.add_component(e, GlobalTransform { matrix: t.local_matrix });
        world.add_component(e, Camera::new(1.0, 0.1, 500.0, yaw, pitch, true));
        e.id()
    };
    S { cam_id }
}
