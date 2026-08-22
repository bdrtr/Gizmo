//! # Bevy'nin `picking/mesh_picking` örneğinin Gizmo Engine portu
//!
//! Fareyle sahnedeki nesneleri seçmek. Bir sıra şekil duruyor; imleç birinin üstüne gelince o
//! şekil aydınlanıyor, çarpma noktası ve yüzey normali çiziliyor, sol tık rengini kalıcı olarak
//! değiştiriyor.
//!
//! ## Motorda seçim, bir "picking" alt sistemi değil — üç parçanın toplamı
//!
//! Bevy'nin `MeshPickingPlugin`'i var: takarsınız, `PointerInteraction` olaylarını dinlersiniz.
//! Gizmo'da böyle bir katman yok, ama üçü de hazır olan parçalar var ve birleştirmesi altı satır:
//!
//!   1. [`Camera::screen_to_ray`] — piksel koordinatını dünya ışınına çevirir (NDC dönüşümü ve
//!      tekil matris korumaları motorun içinde).
//!   2. [`PhysicsWorld::raycast`] — ışını sahnenin **çarpışma** gövdelerine karşı atar ve en
//!      yakın isabeti verir (varlık, mesafe, nokta, normal).
//!   3. [`Gizmos`] — isabeti çizer.
//!
//! **Ve bunun bir bedeli var, port onu saklamıyor:** ışın *mesh*'e değil **collider**'a çarpar.
//! Yani seçilebilir olması istenen her nesnenin bir çarpışma şekli olmalı, ve o şekil mesh'in
//! yaklaşığıdır — burada küre/kutu/kapsül/silindir/koni birebir eşleşiyor, ama karmaşık bir
//! mesh için `Collider::trimesh` ya da `convex_hull` gerekir. Bevy'nin varsayılan arka ucu
//! üçgenlere karşı ışın atar; buradaki karşılığı fizik dünyasıdır.
//!
//! Nesneler **statik gövde**: seçilebilirlik çarpışma şeklinden geliyor, düşmelerine gerek yok.
//!
//! Klavyesiz/faresiz doğrulama için `GIZMO_PICK_SELFTEST=1`: ışın ekranın ortasından atılır.
//!
//! ## Kontroller
//!   * **Fare** — üstüne gel (aydınlanır) · **Sol tık** — rengini değiştir
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Seçilebilir şekillerin işareti ve o anki renk sırası.
#[derive(Clone, Copy)]
struct Shape {
    palette: usize,
}
gizmo::core::impl_component!(Shape);

/// İmlecin o anki isabeti — HUD ve boyama bunu okur.
#[derive(Default, Clone, Copy)]
struct Hover {
    entity: Option<u32>,
    distance: f32,
    point: Vec3,
    normal: Vec3,
    clicks: u32,
}
gizmo::core::impl_component!(Hover);

/// Şekillerin gezindiği renkler.
const PALETTE: [Vec4; 4] = [
    Vec4::new(0.30, 0.31, 0.36, 1.0),
    Vec4::new(0.90, 0.35, 0.25, 1.0),
    Vec4::new(0.25, 0.70, 0.40, 1.0),
    Vec4::new(0.30, 0.45, 0.95, 1.0),
];
/// İmlecin altındaki şeklin ek parlaklığı.
const HOVER_BOOST: Vec4 = Vec4::new(0.35, 0.35, 0.35, 0.0);
/// Işının menzili.
const PICK_RANGE: f32 = 100.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Mesh Picking", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Beş şekil, beşinin de collider'ı mesh'iyle birebir. Tek sayı bilerek: ortadaki
            // şekil ekranın merkezine denk gelsin ki `GIZMO_PICK_SELFTEST` bir şeye çarpsın.
            let shapes: [(Mesh, Collider); 5] = [
                (AssetManager::create_cube(device), Collider::box_collider(Vec3::splat(0.5))),
                (AssetManager::create_sphere(device, 0.5, 16, 24), Collider::sphere(0.5)),
                (
                    AssetManager::create_capsule(device, 0.35, 0.5, 8, 16),
                    Collider::capsule(0.35, 0.25),
                ),
                (
                    AssetManager::create_cylinder(device, 0.4, 1.0, 24),
                    Collider::cylinder(0.4, 0.5),
                ),
                (
                    AssetManager::create_cone(device, 0.5, 1.0, 24),
                    Collider::cone(0.5, 0.5),
                ),
            ];

            for (i, (mesh, collider)) in shapes.into_iter().enumerate() {
                let x = (i as f32 - 2.0) * 1.6;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    mesh,
                    Material::new(white.clone()).with_pbr(PALETTE[0], 0.5, 0.0),
                    MeshRenderer::new(),
                    Shape { palette: 0 },
                ));
                // Statik gövde: seçilebilirlik çarpışma şeklinden gelir, düşmesi gerekmez.
                scene.world.add_bundle(
                    entity,
                    RigidBodyBundle::static_body().with_collider(collider),
                );
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 14.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.7),
                intensity: 1.4,
                ..Default::default()
            });

            scene.world.insert_resource(Hover::default());
            scene.world.insert_resource(Gizmos::default());
            scene.spawn_camera(state, Vec3::new(0.0, 3.0, 7.0), Vec3::new(0.0, 0.5, 0.0));
        })
        .add_update_system(pick_under_cursor.in_phase(Phase::Update))
        .add_update_system(paint_hover.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(hover) = world.get_resource::<Hover>().map(|h| *h) else {
                return;
            };
            gizmo::egui::Area::new("picking".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Fareyle seçim");
                    match hover.entity {
                        Some(entity) => {
                            ui.label(format!("isabet: varlık #{entity}"));
                            ui.label(format!("mesafe: {:.2} m", hover.distance));
                            ui.label(format!(
                                "nokta: ({:.2}, {:.2}, {:.2})",
                                hover.point.x, hover.point.y, hover.point.z
                            ));
                            ui.label(format!(
                                "normal: ({:.2}, {:.2}, {:.2})",
                                hover.normal.x, hover.normal.y, hover.normal.z
                            ));
                        }
                        None => {
                            ui.label("isabet yok — imleç boşlukta");
                        }
                    }
                    ui.separator();
                    ui.label(format!("tıklama: {}", hover.clicks));
                    ui.label("ışın mesh'e değil COLLIDER'a çarpar");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// İmlecin altındaki şekli bulur, çizer, ve tıklanınca rengini değiştirir.
fn pick_under_cursor(
    mut shapes: Query<(Mut<Shape>, Mut<Material>)>,
    cameras: Query<(&Camera, &Transform)>,
    physics: Res<PhysicsWorld>,
    window: Res<WindowInfo>,
    input: Res<Input>,
    mut hover: ResMut<Hover>,
    mut gizmos: ResMut<Gizmos>,
) {
    gizmos.clear();
    hover.entity = None;

    // 1) Kamera: sahnenin baktığı kamerayı bul.
    let mut camera_data = None;
    for (_entity, (camera, transform)) in cameras.iter() {
        if camera.primary {
            camera_data = Some((*camera, transform.position));
            break;
        }
    }
    let Some((camera, camera_pos)) = camera_data else {
        return;
    };

    // 2) İmleç → ışın. Ekran dışındaysa iş yok.
    //
    // `GIZMO_PICK_SELFTEST=1` imleci ekranın ortasına sabitler: başsız bir kare yakalamasında
    // fare yoktur, ve seçimin çalıştığını başka türlü göstermenin yolu da yoktur.
    let (mx, my) = if std::env::var("GIZMO_PICK_SELFTEST").is_ok() {
        (window.width * 0.5, window.height * 0.5)
    } else {
        input.mouse_position()
    };
    if mx < 0.0 || my < 0.0 || mx > window.width || my > window.height {
        return;
    }
    let ray = camera.screen_to_ray((mx, my), (window.width, window.height), camera_pos);

    // 3) Işını çarpışma dünyasına at.
    //
    // **İki ayrı `Ray` tipi var ve burada karşılaşıyorlar:** kamera `gizmo_math::Ray` üretiyor
    // (alanları `Vec3A`, yani SIMD hizalı), fizik dünyası ise `gizmo_physics_core::Ray` bekliyor
    // (alanları `Vec3`). İkisi de aynı iki şeyi taşıyor; dönüşüm tek satır ama derleyiciye kadar
    // görünmüyor — motorda kapatılmaya değer bir pürüz olarak not.
    let ray = gizmo::physics::Ray::new(Vec3::from(ray.origin), Vec3::from(ray.direction));
    let Some(hit) = physics.raycast(&ray, PICK_RANGE) else {
        return;
    };

    hover.entity = Some(hit.entity.id());
    hover.distance = hit.distance;
    hover.point = hit.point;
    hover.normal = hit.normal;

    // Çarpma noktası: küçük bir artı + normal boyunca bir çizgi.
    let color = [1.0, 0.85, 0.2, 1.0];
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.draw_line(hit.point - axis * 0.12, hit.point + axis * 0.12, color);
    }
    gizmos.draw_line(hit.point, hit.point + hit.normal * 0.6, [0.2, 0.9, 1.0, 1.0]);

    // Sol tık: sıradaki renge geç.
    if input.is_mouse_button_just_pressed(0) {
        hover.clicks += 1;
        for (entity, (mut shape, mut material)) in shapes.iter_mut() {
            if entity == hit.entity.id() {
                shape.palette = (shape.palette + 1) % PALETTE.len();
                material.albedo = PALETTE[shape.palette];
            }
        }
    }
}

/// İmlecin altındaki şekli aydınlatır, diğerlerini kendi rengine bırakır.
fn paint_hover(mut shapes: Query<(Mut<Shape>, Mut<Material>)>, hover: Res<Hover>) {
    for (entity, (shape, mut material)) in shapes.iter_mut() {
        let base = PALETTE[shape.palette];
        material.albedo = if hover.entity == Some(entity) {
            base + HOVER_BOOST
        } else {
            base
        };
    }
}
