//! # Karınca — bir koloni, yuva ve yem
//!
//! Yukarıdan bakan bir toprak: ortada yuva, köşelerde yem yığınları, altı yüz karınca. Her
//! karınca bir yığına yürür, bir parça alır, yuvaya taşır, yeniden çıkar. Sol tıkla duvar koy ya
//! da kaldır — bütün koloni yolunu yeniden bulur; sağ tıkla yeni bir yem yığını bırak. Motorun
//! yirmi altıncı oyunu ve **kalabalık** yapay zekâsı olan ilki: her karınca bir `NavAgent`.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** | duvar koy / kaldır |
//! | **sağ tık** | yem yığını bırak |
//! | **R** | baştan |
//!
//! `karinca --test` pencere açmaz: koloni 40 saniye çalışır, 10. saniyede yuvayı ikiye bölen bir
//! duvar çekilir, 25.'de kaldırılır; taşınan yem ve en uzun yapay zekâ karesi yazılır
//! (`KARINCA_DEBUG=1` her beş saniyede koloninin durumunu da). Teslimat dalgalar halinde gelir:
//! bütün koloni aynı anda çıkar ve bir gidiş-dönüş ~20 saniyedir.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Bir kalabalığın yolları tek iş parçacığında aranıyordu.** Duvar konunca bütün koloni aynı
//!   karede yol istiyor; 1000 ajanda o kare 44 ms sürüyordu. **Düzeltildi:** aramalar paralel
//!   (1000 ajanda 8,4 ms).
//! - **Ulaşılamayan bir hedef her karede yeniden aranıyordu.** Duvarla kapatılmış bir yem
//!   yığınına giden karınca, yolu yok diye her karede yeniden yol arıyordu. **Düzeltildi:** başarısız
//!   arama zamanlamayı bekliyor; `NavAgent::route_failed()` karıncanın oraya gidemeyeceğini
//!   söylüyor — bu oyun onu görünce başka yığın seçiyor.

use gizmo::ai::{NavAgent, NavGrid};
use gizmo::core::input::Input;
use gizmo::physics::Velocity;
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, SimpleAppExt, SimpleSceneState};

const W: i32 = 48;
const ANTS: usize = 600;
const NEST: Vec3 = Vec3::new(24.0, 0.0, 24.0);
const PILES: [(f32, f32); 4] = [(5.5, 5.5), (42.5, 5.5), (5.5, 42.5), (42.5, 42.5)];
const PILE_FOOD: u32 = 400;

#[derive(Clone, Copy)]
struct Ant {
    carrying: bool,
    pile: usize,
}
gizmo::core::impl_component!(Ant);

#[derive(Clone, Copy)]
struct Wall;
gizmo::core::impl_component!(Wall);

struct Pile {
    at: Vec3,
    food: u32,
    marker: Option<Entity>,
}

struct Colony {
    piles: Vec<Pile>,
    delivered: u32,
    worst_ai_ms: f32,
    recent_ai_ms: f32,
    clock: f32,
}

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// Yemi olan, karıncanın en az uğradığı değil — basitçe: karıncanın sırasına göre dönen bir yığın.
fn choose_pile(colony: &Colony, start: usize) -> Option<usize> {
    let n = colony.piles.len();
    (0..n).map(|k| (start + k) % n).find(|&i| colony.piles[i].food > 0)
}

fn spawn_colony(world: &mut World, visual: bool) -> Colony {
    let mut grid = NavGrid::new(1.0, W, W);
    grid.needs_rebuild = false;
    world.insert_resource(grid);
    let colony = Colony {
        piles: PILES.iter().map(|&(x, z)| Pile { at: Vec3::new(x, 0.0, z), food: PILE_FOOD, marker: None }).collect(),
        delivered: 0,
        worst_ai_ms: 0.0,
        recent_ai_ms: 0.0,
        clock: 0.0,
    };
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    for i in 0..ANTS {
        let a = (rng(&mut s) % 6283) as f32 / 1000.0;
        let r = 1.5 + (rng(&mut s) % 1000) as f32 / 1000.0;
        let at = NEST + Vec3::new(a.cos() * r, 0.0, a.sin() * r);
        let pile = i % colony.piles.len();
        let mut agent = NavAgent::new(4.0, 20.0, 0.5).with_separation(0.35, 0.6);
        agent.set_target(colony.piles[pile].at);
        let e = world.spawn();
        world.add_component(e, Transform::new(at).with_scale(Vec3::splat(0.18)));
        world.add_component(e, Velocity::default());
        world.add_component(e, agent);
        world.add_component(e, Ant { carrying: false, pile });
        if visual {
            world.add_component(e, GlobalTransform::default());
            world.add_component(e, MeshSource("standard_cube".into()));
            world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#3B2A1A").to_vec4(), 0.8, 0.0));
            world.add_component(e, MeshRenderer::new());
        }
    }
    colony
}

/// Bir adım: yapay zekâ, hızın konuma işlenmesi, karıncaların işi. Yapay zekâ karesinin süresi ölçülür.
fn step(world: &mut World, colony: &mut Colony, dt: f32, visual: bool) {
    colony.clock += dt;
    let t0 = std::time::Instant::now();
    gizmo::ai::system::ai_frame(world, dt);
    let ms = t0.elapsed().as_secs_f32() * 1000.0;
    colony.worst_ai_ms = colony.worst_ai_ms.max(ms);
    colony.recent_ai_ms = colony.recent_ai_ms * 0.95 + ms * 0.05;
    let moves: Vec<(Entity, Vec3)> = world.query::<(Entity, &Velocity)>().unwrap().iter().map(|(entity, v)| (entity, v.linear)).collect();
    // Duvarın içine yürümesin: bir sonraki hücre kapalıysa durur, yol bulucu çevirir.
    let steps: Vec<(Entity, Vec3)> = {
        let grid = world.get_resource::<NavGrid>();
        let ts = world.borrow::<Transform>();
        moves
            .into_iter()
            .filter_map(|(id, v)| {
                let next = ts.get(id)?.position + Vec3::new(v.x, 0.0, v.z) * dt;
                grid.as_ref().is_none_or(|g| g.is_walkable(g.world_to_grid(next))).then_some((id, next))
            })
            .collect()
    };
    for (id, next) in steps {
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
            t.position = next;
        }
    }
    let ants: Vec<(Entity, Ant, Vec3, bool)> = {
        let ts = world.borrow::<Transform>();
        let ag = world.borrow::<NavAgent>();
        world.query::<(Entity, &Ant)>().unwrap().iter()
            .filter_map(|(entity, ant)| Some((entity, *ant, ts.get(entity)?.position, ag.get(entity).is_some_and(|n| n.route_failed()))))
            .collect()
    };
    for (id, mut ant, pos, failed) in ants {
        let flat = |p: Vec3| Vec2::new(p.x - pos.x, p.z - pos.z).length();
        let mut new_target = None;
        if ant.carrying {
            if flat(NEST) < 1.5 {
                ant.carrying = false;
                colony.delivered += 1;
                if let Some(p) = choose_pile(colony, ant.pile + 1) {
                    ant.pile = p;
                    new_target = Some(colony.piles[p].at);
                }
                if visual {
                    if let Some(mut m) = world.borrow_mut::<MaterialDesc>().get_mut(id) {
                        m.albedo = Color::srgb_hex("#3B2A1A").to_vec4();
                    }
                }
            }
        } else if colony.piles[ant.pile].food > 0 && flat(colony.piles[ant.pile].at) < 1.2 {
            colony.piles[ant.pile].food -= 1;
            ant.carrying = true;
            new_target = Some(NEST);
            if visual {
                if let Some(mut m) = world.borrow_mut::<MaterialDesc>().get_mut(id) {
                    m.albedo = Color::srgb_hex("#E6B422").to_vec4();
                }
            }
        } else if colony.piles[ant.pile].food == 0 || failed {
            // Yığın bitti ya da oraya yol yok: başka yığın.
            if let Some(p) = choose_pile(colony, ant.pile + 1) {
                if p != ant.pile {
                    ant.pile = p;
                    new_target = Some(colony.piles[p].at);
                }
            }
        }
        let wants = if ant.carrying { NEST } else { colony.piles[ant.pile].at };
        if let Some(mut a) = world.borrow_mut::<NavAgent>().get_mut(id) {
            if let Some(t) = new_target {
                a.set_target(t);
            } else if a.target.is_none() {
                // Hedefe vardı ama iş bitmedi (kalabalıkta yaklaştı, değmedi): tekrar.
                a.set_target(wants);
            }
        }
        if let Some(mut stored) = world.borrow_mut::<Ant>().get_mut(id) {
            *stored = ant;
        }
    }
}

fn set_wall(world: &mut World, cell: (i32, i32), on: bool, visual: bool) {
    let at = Vec3::new(cell.0 as f32 + 0.5, 0.0, cell.1 as f32 + 0.5);
    if let Some(mut g) = world.get_resource_mut::<NavGrid>() {
        if on {
            g.add_obstacle_world(at);
        } else {
            g.remove_obstacle_world(at);
        }
    }
    let existing: Option<Entity> = {
        let ts = world.borrow::<Transform>();
        world.query::<(Entity, &Wall)>().unwrap().iter().map(|(entity, _)| entity).find(|id| ts.get(*id).is_some_and(|t| (t.position.x - at.x).abs() < 0.1 && (t.position.z - at.z).abs() < 0.1))
    };
    match (on, existing) {
        (true, None) => {
            let e = world.spawn();
            world.add_component(e, Transform::new(at + Vec3::Y * 0.4).with_scale(Vec3::new(0.5, 0.4, 0.5)));
            world.add_component(e, Wall);
            if visual {
                world.add_component(e, GlobalTransform::default());
                world.add_component(e, MeshSource("standard_cube".into()));
                world.add_component(e, MaterialDesc::new().with_pbr(Color::srgb_hex("#7F8C8D").to_vec4(), 0.9, 0.0));
                world.add_component(e, MeshRenderer::new());
            }
        }
        (false, Some(id)) => {
            if let Some(e) = world.live(id) {
                world.despawn(e);
            }
        }
        _ => {}
    }
}

fn is_wall(world: &World, cell: (i32, i32)) -> bool {
    world.get_resource::<NavGrid>().is_some_and(|g| !g.is_walkable(g.world_to_grid(Vec3::new(cell.0 as f32 + 0.5, 0.0, cell.1 as f32 + 0.5))))
}

/// `--test`: 40 saniye, 10. saniyede yuvanın batısına bir duvar, 25.'de kaldırılır.
fn self_test() -> bool {
    let mut world = World::new();
    let mut colony = spawn_colony(&mut world, false);
    let dt = 1.0 / 60.0;
    let wall: Vec<(i32, i32)> = (8..40).filter(|&z| z != 24).map(|z| (16, z)).collect();
    let (mut at_wall, mut at_unwall) = (0, 0);
    for frame in 0..60 * 40 {
        if frame == 60 * 10 {
            at_wall = colony.delivered;
            for &c in &wall {
                set_wall(&mut world, c, true, false);
            }
        }
        if frame == 60 * 25 {
            at_unwall = colony.delivered;
            for &c in &wall {
                set_wall(&mut world, c, false, false);
            }
        }
        step(&mut world, &mut colony, dt, false);
        if std::env::var_os("KARINCA_DEBUG").is_some() && frame % 300 == 299 {
            let (failed, idle, carrying) = {
                let ag = world.borrow::<NavAgent>();
                let ants = world.borrow::<Ant>();
                (
                    ag.iter().filter(|a| a.route_failed()).count(),
                    ag.iter().filter(|a| a.target.is_none()).count(),
                    ants.iter().filter(|a| a.carrying).count(),
                )
            };
            let left: Vec<u32> = colony.piles.iter().map(|p| p.food).collect();
            println!("  {:4.0} s: taşınan {:4}, taşıyan {carrying:3}, yolsuz {failed:3}, hedefsiz {idle:3}, yığınlar {left:?}", colony.clock, colony.delivered);
        }
    }
    let positions_ok = world.borrow::<Transform>().iter().all(|t| t.position.is_finite());
    let ok = colony.delivered > 300 && colony.delivered > at_unwall && at_unwall > at_wall && positions_ok;
    println!(
        "karinca --test: {} karınca, 40 s'de {} yem taşındı (duvardan önce {at_wall}, duvarken {}, sonra {}); en uzun yapay zekâ karesi {:.1} ms → {}",
        ANTS,
        colony.delivered,
        at_unwall - at_wall,
        colony.delivered - at_unwall,
        colony.worst_ai_ms,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

struct Game {
    colony: Colony,
    hud: Entity,
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Karınca", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.1)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 1.8, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(24.0, 42.0, 50.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.05,
                primary: true,
                ..Default::default()
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(24.0, -0.05, 24.0)).with_scale(Vec3::new(24.0, 0.05, 24.0)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#8B6B47").to_vec4(), 0.95, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle((
                Transform::new(NEST).with_scale(Vec3::new(1.4, 0.3, 1.4)),
                GlobalTransform::default(),
                MeshSource("cylinder".into()),
                MaterialDesc::new().with_pbr(Color::srgb_hex("#5D4037").to_vec4(), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            let mut colony = spawn_colony(scene.world, true);
            for pile in &mut colony.piles {
                pile.marker = Some(spawn_pile_marker(scene.world, pile.at));
            }
            let hud = scene.world.spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),));
            scene.world.insert_resource(Game { colony, hud });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn spawn_pile_marker(world: &mut World, at: Vec3) -> Entity {
    world
        .spawn_bundle((
            Transform::new(at + Vec3::Y * 0.3).with_scale(Vec3::splat(0.8)),
            GlobalTransform::default(),
            MeshSource("sphere".into()),
            MaterialDesc::new().with_pbr(Color::srgb_hex("#7CB342").to_vec4(), 0.6, 0.0),
            MeshRenderer::new(),
        ))
        
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let cell = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y)
        .map(|p| (p.x.floor() as i32, p.z.floor() as i32))
        .filter(|&(x, z)| (0..W).contains(&x) && (0..W).contains(&z));
    if let Some(c) = cell {
        if input.is_mouse_button_just_pressed(0) {
            let on = !is_wall(world, c);
            set_wall(world, c, on, true);
        }
        if input.is_mouse_button_just_pressed(1) && !is_wall(world, c) {
            let at = Vec3::new(c.0 as f32 + 0.5, 0.0, c.1 as f32 + 0.5);
            let marker = spawn_pile_marker(world, at);
            game.colony.piles.push(Pile { at, food: 200, marker: Some(marker) });
        }
    }
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        let gone: Vec<Entity> = world.query::<(Entity, &Ant)>().unwrap().iter()
            .map(|(entity, _)| entity)
            .chain(world.query::<(Entity, &Wall)>().unwrap().iter().map(|(entity, _)| entity))
            .chain(game.colony.piles.iter().filter_map(|p| p.marker))
            .collect();
        for id in gone {
            if let Some(e) = world.live(id) {
                world.despawn(e);
            }
        }
        game.colony = spawn_colony(world, true);
        for pile in &mut game.colony.piles {
            pile.marker = Some(spawn_pile_marker(world, pile.at));
        }
    }
    step(world, &mut game.colony, dt.min(0.05), true);
    // Yığınlar yem azaldıkça küçülür.
    for pile in &game.colony.piles {
        if let Some(m) = pile.marker {
            if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(m) {
                t.scale = Vec3::splat(0.2 + 0.6 * (pile.food as f32 / PILE_FOOD as f32).min(1.0));
            }
        }
    }
    sync_transforms(world);
    let left: u32 = game.colony.piles.iter().map(|p| p.food).sum();
    let text = format!(
        "{ANTS} karınca   taşınan yem {}   kalan {left}\nyapay zekâ {:.1} ms (en uzun {:.1})\nsol tık: duvar   sağ tık: yem   R: baştan",
        game.colony.delivered, game.colony.recent_ai_ms, game.colony.worst_ai_ms
    );
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(game.hud) {
        if t.content != text {
            t.content = text;
        }
    }
    world.insert_resource(game);
}

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
