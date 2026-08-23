//! # Ebeveyn/çocuk ağacı
//!
//! Ebeveyn/çocuk ağacının **yaşam döngüsü**: bağ kurmak, çocuk listesini gezmek, bir çocuğu yok
//! etmek, ve sonunda kökü yok edip bütün alt ağacın gitmesini beklemek. (Dönüşümün ebeveynden
//! çocuğa nasıl aktığı ayrı bir demoda: `parenting`.)
//!
//! ## Motorda ağaç iki sıradan bileşen, ve tutarlılığı elle korunuyor
//!
//! Gizmo'nun `hierarchy.rs`'i açık yazıyor: *"Hiyerarşi iki sıradan bileşendir ve tutarlılığını
//! [`HierarchyExt`] korur; bu bileşenleri doğrudan yazmak defter tutmayı atlar ve geri
//! göstermeyen çocuklara sahip bir ebeveyn bırakabilir."* İki bileşen [`Parent`] (tek `u32`) ve
//! [`Children`] (`Vec<u32>`).
//!
//! Buradan üç sonuç çıkıyor:
//!
//! **1. `with_children` yok.** Çocukları ebeveynin doğuşu sırasında bir kapanışta üretmenin yolu
//! yok; önce doğuruyor, sonra [`HierarchyExt::add_child`] ile bağlıyorsunuz.
//!
//! **2. `Children` varlık değil kimlik taşıyor.** `Vec<u32>`, yani **kuşak bilgisi yok**. Ölmüş
//! bir kimlik geri dönüştürülüp başka bir varlığa verilirse, listede kalmış eski kayıt yanlış
//! varlığı gösterir.
//!
//! **3. Ve asıl ölçüm: düz `despawn` ebeveynin listesini temizlemiyor.** Bir çocuğu düz
//! `despawn` ile yok edip ebeveynin listesinin kısalmasını beklerseniz kısalmıyor: bu bakım
//! [`HierarchyExt::remove_child`]'ın işi, ve düz `despawn` çağrısı ebeveynde **asılı bir kimlik**
//! bırakıyor.
//!
//! ## Ölçüldü (2026-08-23)
//!
//! | an     | yapılan                          | kökün `Children`'ı            |
//! |--------|----------------------------------|-------------------------------|
//! | 0,0 s  | iki çocuk bağlandı               | 2 kayıt, 2'si yaşıyor         |
//! | 2,0 s  | son çocuk düz `despawn` ile öldü | **hâlâ 2 kayıt, 1'i yaşıyor** |
//! | 3,0 s  | liste elle ayıklandı             | 1 kayıt, 1'i yaşıyor          |
//! | 4,0 s  | kök `despawn_recursive` ile öldü | 2 düğümden 0 kaldı            |
//!
//! İkinci satır demonun bütün gerekçesi: sahne bir buçuk saniye boyunca, artık var olmayan bir
//! kimliği gösteren bir ebeveynle çalıştı. Motor bunu çökmeden atlatıyor — ölü kimlik hiçbir
//! sorguda eşleşmiyor — ama liste yanlış, ve kimlik geri dönüştürülürse yanlışlığı görünür olur.
//!
//! **Üçüncü satır ikinci bir bulgu getirdi:** temizliğin doğru adresi olan
//! [`HierarchyExt::remove_child`] burada **çağrılamıyor**, çünkü imzası `Entity` istiyor ve ölü
//! bir kimlikten `Entity` üretilemez (`World::get_entity` `None` döner). Yani bir çocuk düz
//! `despawn` ile öldükten *sonra* motorun sunduğu bakım çağrısıyla defteri düzeltmenin yolu yok;
//! demo `Children`'ı doğrudan düzenlemek zorunda kalıyor — ki motorun kendi belgesi bunu
//! "defter tutmayı atlamak" diye adlandırıyor. Doğru sıra bu yüzden **önce `remove_child`, sonra
//! `despawn`**; ya da baştan [`HierarchyExt::despawn_recursive`].
//!
//! ## Kontroller
//!   * **R** — baştan kur · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::PI;

/// Ağaçtaki her küpün işareti.
#[derive(Clone, Copy)]
struct Node;
gizmo::core::impl_component!(Node);

/// Ağacın o anki durumu ve ölçüm defteri.
#[derive(Default, Clone)]
struct Tree {
    root: Option<u32>,
    /// Kökün `Children` listesindeki kayıt sayısı — HUD bunu okur.
    listed: usize,
    /// Bu kayıtların kaçı gerçekten yaşayan bir varlığı gösteriyor.
    alive: usize,
    elapsed: f32,
    /// Sırayla: çocuk düz despawn edildi / remove_child çağrıldı / kök yok edildi.
    stage: u8,
    log: Vec<String>,
    restart: bool,
}
gizmo::core::impl_component!(Tree);

/// Kökün rengi, ve iki çocuğun (mavi ile yeşil).
const ROOT_COLOR: Vec4 = Vec4::new(0.85, 0.80, 0.75, 1.0);
const CHILD_A: Vec4 = Vec4::new(0.25, 0.40, 0.95, 1.0);
const CHILD_B: Vec4 = Vec4::new(0.30, 0.85, 0.35, 1.0);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Hierarchy", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -4.5, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 20.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.11, 0.12, 0.14, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(Tree::default());
            let cube = AssetManager::create_cube(&scene.renderer.device);
            let template = Material::new(white).with_pbr(ROOT_COLOR, 0.5, 0.0);
            build_tree(scene.world, &cube, &template);

            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 11.0), Vec3::new(0.0, 0.0, 0.0));
        })
        .add_update_system(rotate.in_phase(Phase::Update).label("rotate"))
        .add_update_system(tick.in_phase(Phase::Update).after("rotate"))
        // Ağacı bozan/kuran her şey `&mut World` istiyor.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            advance_stages(world);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(tree) = world.get_resource::<Tree>().map(|t| t.clone()) else {
                return;
            };
            gizmo::egui::Area::new("hier".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(400.0);
                    ui.heading("Ağacın yaşam döngüsü");
                    ui.label(format!("geçen süre: {:.1} s", tree.elapsed));
                    ui.separator();
                    ui.label(format!("kökün Children listesi: {} kayıt", tree.listed));
                    ui.label(format!("bunların {} tanesi yaşayan varlık", tree.alive));
                    if tree.listed > tree.alive {
                        ui.label(format!(
                            ">> {} ASILI KİMLİK (düz despawn listeyi temizlemez)",
                            tree.listed - tree.alive
                        ));
                    }
                    ui.separator();
                    for line in &tree.log {
                        ui.label(line);
                    }
                    ui.separator();
                    ui.label("R — baştan kur");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kök bir yöne, çocuklar öbür yöne döner.
///
/// Çocuklar ebeveynin [`Children`] listesinden geziliyor — ama liste `u32` tuttuğu için önce
/// yaşayan varlığa çevrilmesi gerekiyor, ve **ölü bir kimlik sessizce atlanıyor**. Demonun
/// ikinci aşamasında listede tam olarak öyle bir kimlik var.
fn rotate(
    roots: Query<(&Children, With<Node>)>,
    mut transforms: Query<(Mut<Transform>, With<Node>)>,
    time: Res<Time>,
) {
    let dt = time.dt();

    // Önce hangi varlığın ne kadar döneceğini topla: iki sorgu aynı anda ödünç alınamaz.
    let mut root_ids = Vec::new();
    let mut child_ids = Vec::new();
    for (entity, (children, _)) in roots.iter() {
        root_ids.push(entity);
        child_ids.extend(children.0.iter().copied());
    }

    for (entity, (mut transform, _)) in transforms.iter_mut() {
        if root_ids.contains(&entity) {
            transform.rotate_z(-PI / 2.0 * dt);
        } else if child_ids.contains(&entity) {
            transform.rotate_z(PI * dt);
        }
    }
}

/// Süreyi biriktirir, defteri günceller, **R**'yi dinler.
fn tick(
    mut tree: ResMut<Tree>,
    roots: Query<(&Children, With<Node>)>,
    nodes: Query<&Node>,
    input: Res<Input>,
    time: Res<Time>,
) {
    tree.elapsed += time.dt();
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
        tree.restart = true;
    }

    let living: Vec<u32> = nodes.iter().map(|(entity, _)| entity).collect();
    let (mut listed, mut alive) = (0, 0);
    for (_entity, (children, _)) in roots.iter() {
        listed += children.0.len();
        alive += children.0.iter().filter(|id| living.contains(id)).count();
    }
    tree.listed = listed;
    tree.alive = alive;
}

/// Zamanı gelen aşamayı uygular. Hepsi `&mut World` isteyen işler.
fn advance_stages(world: &mut gizmo::core::World) {
    let Some((elapsed, stage, restart, root)) = world
        .get_resource::<Tree>()
        .map(|t| (t.elapsed, t.stage, t.restart, t.root))
    else {
        return;
    };

    if restart {
        rebuild(world);
        return;
    }
    let Some(root_id) = root else { return };
    let Some(root_entity) = world.get_entity(root_id) else {
        return;
    };

    // Kökün çocuk listesinin bir kopyası — listeyi tutan bileşeni ödünç alırken dünyayı
    // değiştiremeyiz.
    let children: Vec<u32> = world
        .query::<&Children>()
        .and_then(|q| {
            q.iter()
                .find(|(entity, _)| *entity == root_id)
                .map(|(_, c)| c.0.clone())
        })
        .unwrap_or_default();

    match stage {
        // 2 s: son çocuğu DÜZ despawn ile öldür — bakım çağrısına uğramadan.
        0 if elapsed >= 2.0 => {
            if let Some(last) = children.last().and_then(|id| world.get_entity(*id)) {
                world.despawn(last);
                note(
                    world,
                    format!("2,0 s — çocuk #{} düz despawn ile öldü", last.id()),
                );
            }
            set_stage(world, 1);
        }
        // 3 s: defteri elle temizle — motorun bu iş için verdiği çağrı.
        1 if elapsed >= 3.0 => {
            let dead: Vec<u32> = children
                .iter()
                .copied()
                .filter(|id| world.get_entity(*id).is_none())
                .collect();
            for id in &dead {
                // Ölü kimlik için `Entity` üretilemiyor; `remove_child` yaşayan bir tanesini
                // ister. Bu yüzden listeyi doğrudan düzeltmek gerekiyor — ve motorun belgesi
                // tam olarak bunun "defter tutmayı atlamak" olduğunu söylüyor.
                prune_child(world, root_id, *id);
            }
            note(
                world,
                format!("3,0 s — {} asılı kimlik listeden ayıklandı", dead.len()),
            );
            set_stage(world, 2);
        }
        // 4 s: kökü alt ağacıyla birlikte yok et.
        2 if elapsed >= 4.0 => {
            let before = world.query::<&Node>().map(|q| q.iter().count()).unwrap_or(0);
            world.despawn_recursive(root_entity);
            let after = world.query::<&Node>().map(|q| q.iter().count()).unwrap_or(0);
            note(
                world,
                format!("4,0 s — despawn_recursive: {before} düğümden {after} kaldı"),
            );
            set_stage(world, 3);
        }
        _ => {}
    }
}

/// Kökün `Children` listesinden ölü bir kimliği çıkarır.
fn prune_child(world: &mut gizmo::core::World, root_id: u32, dead_id: u32) {
    let Some(root) = world.get_entity(root_id) else {
        return;
    };
    let Some(ptr) = world.get_component_mut_ptr(root, std::any::TypeId::of::<Children>()) else {
        return;
    };
    // SAFETY: işaretçi `TypeId::of::<Children>()` ile anahtarlandı, yani yaşayan bir `Children`'ı
    // gösteriyor; `&mut World` üzerinden alındığı için tek canlı referans bu, ve blok içinde
    // hiçbir yapısal değişiklik olmuyor.
    let children = unsafe { &mut *(ptr as *mut Children) };
    children.0.retain(|id| *id != dead_id);
}

/// Deftere bir satır yazar.
fn note(world: &mut gizmo::core::World, line: String) {
    if let Some(mut tree) = world.get_resource_mut::<Tree>() {
        tree.log.push(line);
    }
}

/// Aşamayı ilerletir.
fn set_stage(world: &mut gizmo::core::World, stage: u8) {
    if let Some(mut tree) = world.get_resource_mut::<Tree>() {
        tree.stage = stage;
    }
}

/// Kök + iki çocuk.
fn build_tree(world: &mut gizmo::core::World, cube: &Mesh, template: &Material) {
    let spawn = |world: &mut gizmo::core::World, position: Vec3, scale: f32, albedo: Vec4| {
        let mut material = template.clone();
        material.albedo = albedo;
        world.spawn_bundle((
            Transform::new(position).with_scale(Vec3::splat(scale)),
            GlobalTransform::default(),
            cube.clone(),
            material,
            MeshRenderer::new(),
            Node,
        ))
    };

    let root = spawn(world, Vec3::ZERO, 1.0, ROOT_COLOR);
    // Doğuş sırasında bağlamanın yolu yok: doğur, sonra bağla.
    let child_a = spawn(world, Vec3::new(3.0, 0.0, 0.0), 0.75, CHILD_A);
    world.add_child(root, child_a);
    let child_b = spawn(world, Vec3::new(0.0, 3.0, 0.0), 0.75, CHILD_B);
    world.add_child(root, child_b);

    if let Some(mut tree) = world.get_resource_mut::<Tree>() {
        tree.root = Some(root.id());
        tree.elapsed = 0.0;
        tree.stage = 0;
        tree.log = vec!["0,0 s — kök + iki çocuk bağlandı".to_string()];
        tree.restart = false;
    }
}

/// **R**: kalan düğümleri silip ağacı yeniden kurar.
fn rebuild(world: &mut gizmo::core::World) {
    let doomed: Vec<u32> = world
        .query::<&Node>()
        .map(|q| q.iter().map(|(entity, _)| entity).collect())
        .unwrap_or_default();
    for id in doomed {
        if let Some(entity) = world.get_entity(id) {
            world.despawn(entity);
        }
    }

    let template = world
        .query::<(&Mesh, &Material)>()
        .and_then(|q| q.iter().next().map(|(_, (m, mat))| (m.clone(), mat.clone())));
    let Some((mesh, material)) = template else {
        return;
    };
    build_tree(world, &mesh, &material);
}
