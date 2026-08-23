//! # Olayın hiyerarşide yukarı kabarması
//!
//! Bir olayın hiyerarşide **yukarı kabarması** (bubbling): yaprakta tetiklenen olay, ata zincirini
//! tırmanarak her katmandaki dinleyiciyi çalıştırır. Klasik anlatımı bir zırh/gövde/oyuncu
//! zinciridir — hasar zırha iniyor, oyuncuya kadar çıkıyor.
//!
//! ## Motorda kabarma **var** — ve üç sınırı var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | olayı kabarmaya açmak | [`EntityEvent::can_propagate`] -> `true` |
//! | varlığa bağlı dinleyici | `On<E>`, `world.observe(entity, ..)` |
//! | olayı gönderme | `trigger` |
//! | **yürüyüşün yolunu seçmek** | **yok** — yol her zaman `Parent` zinciri |
//! | **kabarmayı durdurmak** | **yok** — hiçbir dinleyici yürüyüşü kesemiyor |
//! | dinleyicinin bir şey döndürmesi | **yok** — `On<E>` değerle veriliyor, dönüş `()` |
//!
//! Üçüncüsü en can alıcı olanı, ve motorun kendi belgesi açıkça yazıyor: *"Listeners along the
//! chain all receive equal clones of the same event; **none of them can cancel the walk**."*
//!
//! Bir dinleyicinin olayı yakalayıp yukarı çıkmasını engellemesi motorda **yazılamıyor**;
//! yapılabilecek şey olayın *etkisini* bir bayrakla bastırmak, ama dinleyiciler yine de
//! çalışıyor.
//!
//! ## Ölçüldü
//!
//! Beş halkalık zincir, kök = 0, yaprak = 4. Olay **yaprakta** tetikleniyor.
//! Ölçüldü (2026-08-23, `GIZMO_PROP_SELFTEST=1`):
//!
//! | ne | uğradığı halkalar |
//! |----|-------------------|
//! | kabaran olay (`can_propagate` = `true`) | **`[4, 3, 2, 1, 0]`** — yapraktan köke, beşi de |
//! | kabarmayan olay | **`[4]`** — yalnız hedef |
//! | halka 2 yok edildikten sonra kabaran olay | **`[4, 3]`** — ölü ata yürüyüşü durduruyor |
//!
//! Ve durdurulamazlığın ölçüsü: halka **2**'deki dinleyici "burada dursun" diyebilseydi yürüyüş
//! orada biterdi. Bitmiyor — ondan sonra **2 halka daha** (1 ve 0) ziyaret ediliyor. Yürüyüşü
//! kesebilen bir çağrı olsaydı bu sayı 0 olurdu.
//!
//! Üçüncü satır da bir belge iddiasını doğruluyor: `Parent` nesilsiz ham bir id tutuyor, ve
//! `trigger` onu `World::entity` ile çözüyor — ölü bir id yürüyüşü **çökertmek yerine durduruyor**.
//!
//! ## Kontroller
//!   * **Boşluk** — yaprakta kabaran bir olay tetikle
//!   * **N** — kabarmayan bir olay tetikle (yalnız hedefe gider)
//!   * **X** — zincirin ortasındaki halkayı yok et (yürüyüş orada durmalı)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::entity::Entity;
use gizmo::core::input::Input;
use gizmo::core::observer::{EntityEvent, On};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::{Arc, Mutex};

/// Kabaran olay — kabarma [`EntityEvent::can_propagate`] ile açılıyor.
#[derive(Clone, Copy)]
struct Damage {
    target: Entity,
    amount: f32,
}

impl EntityEvent for Damage {
    fn target(&self) -> Entity {
        self.target
    }
    /// Kabarma buradan açılıyor. Kapatmanın **başka** yolu yok, ve açıldıktan sonra
    /// hiçbir dinleyici yürüyüşü kesemiyor.
    fn can_propagate(&self) -> bool {
        true
    }
}

/// Kabarmayan olay — karşılaştırma için.
#[derive(Clone, Copy)]
struct Poke {
    target: Entity,
}
impl EntityEvent for Poke {
    fn target(&self) -> Entity {
        self.target
    }
}

/// Zincirdeki bir halka, derinliğiyle.
#[derive(Clone, Copy)]
struct Link(usize);
gizmo::core::impl_component!(Link);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct PropReport {
    /// Zincir derinliği.
    depth: usize,
    /// Son kabaran olayın uğradığı halkalar (derinlik sırasıyla).
    visited: Vec<usize>,
    /// Son kabarmayan olayın uğradığı halkalar.
    visited_no_prop: Vec<usize>,
    /// "Durdurmak isteyen" dinleyicinin kaçıncı halkada olduğu.
    veto_at: usize,
    /// Veto denemesinden SONRA kaç halka daha ziyaret edildi — yürüyüş kesilebilseydi 0 olurdu.
    after_veto: usize,
    /// Zincir kesildikten sonraki ziyaret sayısı.
    visited_after_cut: Vec<usize>,
    cut_done: bool,
}
gizmo::core::impl_component!(PropReport);

/// Zincirin derinliği ve vetoyu deneyen halka.
const DEPTH: usize = 5;
const VETO_AT: usize = 2;

/// Dinleyicilerin dışarı yazdığı defter — `On<E>` dönüş kanalı taşımadığı için tek yol bu.
type Trail = Arc<Mutex<Vec<usize>>>;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Observer Propagation", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Kök -> ... -> yaprak zinciri. `Parent` zinciri yürüyüşün TEK yolu.
            let mut chain: Vec<Entity> = Vec::new();
            for depth in 0..DEPTH {
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new((depth as f32 - 2.0) * 1.7, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.75 - depth as f32 * 0.08)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.30 + depth as f32 * 0.14, 0.55, 0.85, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Link(depth),
                ));
                if let Some(parent) = chain.last().copied() {
                    scene.world.add_component(entity, gizmo::core::component::Parent(parent.id()));
                }
                chain.push(entity);
            }

            let trail: Trail = Arc::new(Mutex::new(Vec::new()));
            for (depth, entity) in chain.iter().enumerate() {
                let t = trail.clone();
                scene.world.observe::<Damage, _>(*entity, move |on: On<Damage>| {
                    t.lock().unwrap().push(depth);
                    // Yürüyüşü burada durduracak bir çağrı YOK: `On` değerle geliyor, dönüş
                    // `()`, ve `trigger` döngüsü dinleyicinin ne yaptığına bakmıyor.
                    let _ = on.event.amount;
                });
                let t = trail.clone();
                scene.world.observe::<Poke, _>(*entity, move |_on: On<Poke>| {
                    // Kabarmayan olay için ayrı bir iz — 1000 ekleyerek ayırt ediliyor.
                    t.lock().unwrap().push(1000 + depth);
                });
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(Chain {
                entities: chain,
                trail,
            });
            scene.world.insert_resource(PropReport {
                depth: DEPTH,
                veto_at: VETO_AT,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.5), Vec3::ZERO);
        })
        .set_update(|world, state, dt, input| {
            // Bkz. `demo::simple_scene_update` — `set_update` basit sahnenin kancasını eziyor.
            demo::simple_scene_update(world, state, dt, input);
            drive(world, input);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<PropReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("prop".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Olayın yukarı kabarması");
                        ui.label(format!("{} halkalık zincir · yaprak = {}", r.depth, r.depth - 1));
                        ui.separator();
                        ui.monospace(format!("kabaran olay  : {:?}", r.visited));
                        ui.monospace(format!("kabarmayan    : {:?}", r.visited_no_prop));
                        if r.cut_done {
                            ui.monospace(format!("zincir kesik  : {:?}", r.visited_after_cut));
                        }
                        ui.separator();
                        ui.label(format!("halka {} \"durdurmak istedi\"", r.veto_at));
                        if r.after_veto > 0 {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                format!("ama {} halka daha ziyaret edildi", r.after_veto),
                            );
                        }
                        ui.separator();
                        ui.label("yol her zaman Parent zinciri — seçilemiyor.");
                        ui.label("yürüyüşü kesecek bir çağrı YOK.");
                        ui.separator();
                        ui.label("boşluk — kabaran · N — kabarmayan · X — zinciri kes");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Zincir ve dinleyicilerin izi.
#[derive(Clone)]
struct Chain {
    entities: Vec<Entity>,
    trail: Trail,
}
gizmo::core::impl_component!(Chain);

/// Tuşları okur ve olayları tetikler.
fn drive(world: &mut World, input: &Input) {
    use gizmo::winit::keyboard::KeyCode;
    let selftest = std::env::var("GIZMO_PROP_SELFTEST").is_ok();
    let frame_bump = selftest;

    let Some(chain) = world.get_resource::<Chain>().map(|c| c.clone()) else {
        return;
    };
    let leaf = *chain.entities.last().expect("zincir boş olamaz");

    // Kabaran olay.
    if input.is_key_just_pressed(KeyCode::Space as u32) || (frame_bump && once(world, 0)) {
        chain.trail.lock().unwrap().clear();
        world.trigger(Damage {
            target: leaf,
            amount: 10.0,
        });
        let visited: Vec<usize> = chain.trail.lock().unwrap().clone();
        let after_veto = visited.iter().filter(|d| **d < VETO_AT).count();
        tint_visited(world, &visited);
        if let Some(mut r) = world.get_resource_mut::<PropReport>() {
            r.visited = visited.clone();
            r.after_veto = after_veto;
        }
        if selftest {
            gizmo::gizmo_log!(
                Info,
                "kabaran olay yaprakta tetiklendi · uğradığı halkalar {:?} · veto denemesinden sonra {} halka daha",
                visited,
                after_veto
            );
        }
    }

    // Kabarmayan olay.
    if input.is_key_just_pressed(KeyCode::KeyN as u32) || (frame_bump && once(world, 1)) {
        chain.trail.lock().unwrap().clear();
        world.trigger(Poke { target: leaf });
        let visited: Vec<usize> = chain
            .trail
            .lock()
            .unwrap()
            .iter()
            .map(|d| d.saturating_sub(1000))
            .collect();
        if let Some(mut r) = world.get_resource_mut::<PropReport>() {
            r.visited_no_prop = visited.clone();
        }
        if selftest {
            gizmo::gizmo_log!(Info, "kabarmayan olay · uğradığı halkalar {:?}", visited);
        }
    }

    // Zinciri kes: ortadaki halkayı yok et. Belge bunu söylüyor — ölü bir ata yürüyüşü durdurur.
    if input.is_key_just_pressed(KeyCode::KeyX as u32) || (frame_bump && once(world, 2)) {
        // Halkayı dizin sırasına göre değil, taşıdığı `Link` derinliğine göre bul — zincir
        // yeniden düzenlense de doğru halkayı keser.
        let middle_id = world.query::<&Link>().and_then(|q| {
            q.iter().find(|(_, link)| link.0 == DEPTH / 2).map(|(id, _)| id)
        });
        let Some(middle) = middle_id.and_then(|id| world.entity(id)) else {
            return;
        };
        world.despawn(middle);
        chain.trail.lock().unwrap().clear();
        world.trigger(Damage {
            target: leaf,
            amount: 10.0,
        });
        let visited: Vec<usize> = chain.trail.lock().unwrap().clone();
        if let Some(mut r) = world.get_resource_mut::<PropReport>() {
            r.visited_after_cut = visited.clone();
            r.cut_done = true;
        }
        if selftest {
            gizmo::gizmo_log!(
                Info,
                "zincir halka {}'de kesildi · kabaran olay artık {:?}",
                DEPTH / 2,
                visited
            );
        }
    }
}

/// Uğranan halkaları ekranda işaretler — `Link`'in derinliği burada iş görüyor.
fn tint_visited(world: &mut World, visited: &[usize]) {
    let mut targets: Vec<(u32, usize)> = Vec::new();
    if let Some(q) = world.query::<&Link>() {
        for (id, link) in q.iter() {
            targets.push((id, link.0));
        }
    }
    for (id, depth) in targets {
        let Some(entity) = world.entity(id) else { continue };
        let hit = visited.contains(&depth);
        let colour = if hit {
            Vec4::new(0.95, 0.75, 0.25, 1.0)
        } else {
            Vec4::new(0.30 + depth as f32 * 0.14, 0.55, 0.85, 1.0)
        };
        let existing = world
            .query::<&Material>()
            .and_then(|q| q.iter().find(|(e, _)| *e == id).map(|(_, m)| m.clone()));
        if let Some(material) = existing {
            world.add_component(entity, material.with_pbr(colour, 0.5, 0.0));
        }
    }
}

/// Kendi kendine sınamada her adımı bir kez tetiklemek için basit bir mandal.
fn once(world: &mut World, slot: usize) -> bool {
    let mut fired = world
        .get_resource::<Fired>()
        .map(|f| f.0)
        .unwrap_or([false; 3]);
    // Adımlar sırayla: her biri bir öncekinin karesinden sonra.
    let frame = world.tick;
    if fired[slot] || frame < (slot as u32 + 1) * 30 {
        return false;
    }
    fired[slot] = true;
    world.insert_resource(Fired(fired));
    true
}

/// Kendi kendine sınama mandalları.
#[derive(Clone, Copy, Default)]
struct Fired([bool; 3]);
gizmo::core::impl_component!(Fired);
