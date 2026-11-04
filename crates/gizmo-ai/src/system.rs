//! ECS systems that move [`NavAgent`] entities along paths found on a [`NavGrid`].
//!
//! Both systems are meant to run once per frame, [`ai_navmesh_rebuild_system`] before
//! [`ai_navigation_system`]. Neither takes `&mut World` — both mutate resources and components
//! through a shared `&World` reference.
//!
//! These systems write `Velocity::linear` directly instead of applying forces. While an agent
//! has a target the vertical component is forced to exactly zero at the end of every update;
//! while it has none, the whole velocity is damped toward zero.
//!
//! Navigation therefore never contributes vertical velocity of its own — but that is not the
//! same as the agent staying level. Anything else writing `Velocity::linear.y` between
//! navigation updates still moves it, gravity integration included, so a NavAgent on a
//! dynamic body does fall; it just re-zeroes each time this system runs instead of
//! accelerating. Kinematic agents are the case that stays level.

use crate::components::NavAgent;
use crate::pathfinding::NavGrid;
use crate::steering;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;
use gizmo_physics_rigid::components::Velocity;

/// Everything the AI subsystem does in one frame, in the one order that works.
///
/// **This is the entry point a host calls**, and it exists because there is more than one host:
/// the editor's update hook and `PlayLoop` (the ▶ button and every exported game) both need AI,
/// and a copy of "grid, then rebuild, then steer" in each of them is a contract with two
/// implementations — the shape that had already put the fight clock, the animation drivers and
/// GPU physics in only one of the two paths. The order is not free: navigation reads the grid
/// [`ai_navmesh_rebuild_system`] fills, so the reverse plans one frame of paths against a stale
/// one, and both read agents a behaviour tree may have retargeted.
///
/// Takes `&mut World` because [`ensure_nav_grid`] may have to *create* the grid resource, which
/// the two systems below — which only mutate what already exists — do not.
///
/// Cheap on a scene with no AI in it: three empty component borrows and a resource lookup.
pub fn ai_frame(world: &mut World, dt: f32) {
    crate::behavior_tree::behavior_tree_system(world, dt);
    ensure_nav_grid(world);
    ai_navmesh_rebuild_system(world, dt);
    ai_navigation_system(world, dt);
}

/// Gives the world a [`NavGrid`] if it has agents that need one and nothing supplied it.
///
/// **Nothing in the engine ever constructed a `NavGrid`.** Not the editor, not `PlayLoop`, not a
/// scene (the resource is not serialised), and not Lua — `ai.add_nav_agent` and `ai.set_target`
/// exist, so a script could ask for navigation, but the grid those two need had no way into a
/// world at all. [`ai_navigation_system`] returns on its first line without one, so every
/// `NavAgent` ever created stood still, in the editor and in exported games alike. This is the
/// missing constructor, and it is the same call the studio's *Rebuild NavMesh* button makes.
///
/// Only fires when at least one [`NavAgent`] exists: a scene with no agents pays a component
/// count per frame and nothing else, and — more to the point — a grid fitted before the level's
/// geometry is loaded would be fitted to nothing.
///
/// The bounds come from [`NavGrid::fitted_to`], i.e. from the static geometry, because a
/// hard-coded default would have to guess both size *and* position: the pre-`origin` grid always
/// sat in the positive quadrant, so a scene laid out around the world origin — every demo, and
/// the editor's default scene — would have had three quarters of itself out of bounds. A world
/// with no `PhysicsWorld` at all gets the fallback grid rather than nothing, so a test (or a game
/// that steers without simulating) still navigates.
///
/// Does nothing when a grid already exists, so a game that builds its own — coarser cells, tighter
/// bounds, hand-placed obstacles — keeps it.
pub fn ensure_nav_grid(world: &mut World) {
    if world.get_resource::<NavGrid>().is_some() {
        return;
    }
    if world.borrow::<NavAgent>().entities().next().is_none() {
        return;
    }

    let grid = match world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
        Some(physics) => NavGrid::fitted_to(NavGrid::DEFAULT_CELL_SIZE, &physics),
        None => NavGrid::centred_on(
            NavGrid::DEFAULT_CELL_SIZE,
            NavGrid::FALLBACK_CELLS,
            NavGrid::FALLBACK_CELLS,
            gizmo_math::Vec3::ZERO,
        ),
    };
    tracing::info!(
        width = grid.width,
        height = grid.height,
        cell_size = grid.cell_size,
        "[AI] NavGrid yoktu — ajanlar için oluşturuldu"
    );
    world.insert_resource(grid);
}

/// Refills the [`NavGrid`] obstacle set from the physics world's non-dynamic colliders, but
/// only when the grid's `needs_rebuild` flag is set.
///
/// A rebuild rasterises the world-space AABB of every collider whose body type is not
/// `Dynamic` into grid cells and **replaces** the obstacle set wholesale, so any cells added
/// by hand via `NavGrid::add_obstacle_world` are discarded. It then clears `needs_rebuild`, and
/// no code in this crate ever sets that flag back to `true` — after moving, spawning or
/// deleting static geometry the owner of the scene must raise it for navigation to notice.
///
/// Does nothing when either the `NavGrid` or the `PhysicsWorld` resource is absent — a missing
/// resource means navigation is simply not in use here and is not treated as an error.
///
/// `_dt` is ignored — a rebuild does not integrate anything. Run it before
/// [`ai_navigation_system`] in the frame, otherwise agents plan one frame's worth of paths
/// against the stale grid.
pub fn ai_navmesh_rebuild_system(world: &World, _dt: f32) {
    let grid_opt = world.get_resource_mut::<NavGrid>();
    let physics_opt = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>();

    if let (Some(mut grid), Some(physics_world)) = (grid_opt, physics_opt) {
        if grid.needs_rebuild {
            grid.update_from_physics_world(&physics_world);
            tracing::info!(
                obstacle_count = grid.obstacles.len(),
                "[AI] NavMesh PhysicsWorld üzerinden yeniden oluşturuldu"
            );
        }
    }
}

/// An agent's slot in the replan interval, `0.1..=1.0`: the golden-ratio sequence over its id,
/// so consecutive ids land far apart. Never 0, which would replan again on the next frame.
fn replan_phase(id: u32) -> f32 {
    let frac = (id as f32 * 0.618_034).fract();
    0.1 + 0.9 * frac
}

/// Advances every [`NavAgent`] one frame: replans its path when needed, steers it toward the
/// next waypoint and writes the result into `Velocity::linear`. `dt` is the frame time in
/// seconds.
///
/// Requires a [`NavGrid`] resource; without one the system returns immediately and leaves every
/// agent's velocity untouched. Agents missing a `Transform` or a `Velocity` are skipped.
///
/// Per agent, in order:
///
/// - **No target.** The linear velocity is damped by `1 - min(dt * 5, 1)` and the state becomes
///   `Idle`. The clamp means a frame of 0.2 s or longer zeroes the velocity outright rather than
///   reversing it.
/// - **Stuck detection.** While the agent stays within 0.05 m of the last position where it was
///   seen to move, `stuck_timer` accumulates `dt`; past 2 s the state becomes `Stuck` and the
///   path is cleared, which forces a replan on the same frame. The target is kept, so a stuck
///   agent keeps retrying rather than giving up.
/// - **Planar projection.** The target is flattened onto the agent's current Y before
///   pathfinding, so a target's height is ignored and agents only ever navigate in the XZ plane.
/// - **Replan** when the path is exhausted, `recalc.timer` has run out (it is reset to
///   `recalc.interval` seconds), the target has moved more than 2 m from the last planned one,
///   a wall moved, or nothing has been planned yet. After one of those *forced* replans the timer
///   is set to the agent's own slot of the interval (10–100 % of it, by id), so a crowd that was
///   spawned or ordered together plans together once and then spreads its replans out — it used to
///   replan on one frame every interval (1000 agents: 58 ms every 30th frame, 0.27 ms between).
///   The forced frame itself is still one spike: 56 ms for 1000 agents ordered at once. If `NavGrid::find_path` finds nothing — target inside an
///   obstacle, outside the grid, or unreachable — the path is cleared and the agent falls back to
///   steering straight at the target with no obstacle awareness at all, rather than stopping.
/// - **Steering.** [`crate::steering::arrive`] for the final waypoint (slowing radius
///   `2 * arrival_radius`), [`crate::steering::seek`] for the others, plus
///   [`crate::steering::separate`] against other agents. A waypoint counts as reached within
///   `arrival_radius`, or within `2.5 * arrival_radius` if the agent's speed has dropped below
///   0.2 m/s — the second test is what frees agents wedged against geometry.
/// - **Integration.** `velocity += steering * dt`, then the speed is clamped to `max_speed` and
///   the vertical component is forced to zero. On finishing the path the target is cleared and
///   the state becomes `Reached`.
///
/// Neighbours come from XZ buckets as wide as the largest [`NavAgent::separation_radius`], within
/// each agent's own radius, pushed with its [`NavAgent::separation_weight`] — linear in the agent
/// count where it used to scan every pair (1000 / 2000 agents: 13.1 / 53.2 ms a frame before,
/// 0.29 / 0.58 ms after, steering only). The neighbours are summed in agent order, so the result
/// is bit-identical to the scan. One agent with a huge radius widens every bucket and brings the
/// scan back.
///
/// # Aliasing
///
/// This takes `&World` but mutates `NavAgent` and `Velocity` through unchecked borrows, so the
/// compiler cannot catch a conflict. It must not run concurrently with any other system that
/// touches those two component types mutably.
#[tracing::instrument(skip_all, name = "ai_navigation_system")]
pub fn ai_navigation_system(world: &World, dt: f32) {
    let mut grid = match world.get_resource_mut::<NavGrid>() {
        Some(g) => g,
        None => {
            tracing::trace!("[AI] NavGrid resource yok, navigasyon atlanıyor");
            return;
        }
    };
    // A wall placed or removed since the last frame: every route may be wrong now.
    let grid_changed = grid.take_change();
    let grid = &*grid;

    let (agent_entities, cell): (Vec<u32>, f32) = {
        let agents = world.borrow::<NavAgent>();
        let widest = agents.iter().map(|(_, a)| a.separation_radius).fold(0.0f32, f32::max);
        (agents.entities().collect(), widest.max(0.01))
    };
    let agent_count = agent_entities.len();

    // Frame-başı toplu sayaçlar — iç döngüde per-iterasyon log yerine çıkışta AGGREGATE.
    let mut recalc_count = 0u32;
    let mut path_found = 0u32;
    let mut path_failed = 0u32;
    let mut stuck_count = 0u32;
    let mut reached_count = 0u32;

    // SAFETY: ai_navigation_system runs as a scheduled system; the scheduler guarantees no
    // other system mutably aliases NavAgent/Velocity while it runs. NavAgent and Velocity are
    // distinct component types, so their storages never overlap with each other either.
    let mut agents = unsafe { world.borrow_mut_unchecked::<NavAgent>() };
    let transforms = world.borrow::<Transform>();
    // SAFETY: same argument as `agents` above — Velocity is a distinct component type from
    // both NavAgent and Transform, so this view aliases neither.
    let mut velocities = unsafe { world.borrow_mut_unchecked::<Velocity>() };

    // Neighbours by bucket, not by scanning every agent: positions do not change during this
    // pass (it writes velocities only), so one snapshot serves every agent. Buckets are XZ cells
    // as wide as the largest separation radius, so an agent's neighbours lie in the cells its
    // own radius overlaps. Measured, 1000 / 2000 agents: 13.1 / 53.2 ms a frame with the scan.
    let positions: Vec<Option<Vec3>> = agent_entities.iter().map(|&id| transforms.get(id).map(|t| t.position)).collect();
    let key = |p: Vec3| ((p.x / cell).floor() as i32, (p.z / cell).floor() as i32);
    let mut buckets: std::collections::HashMap<(i32, i32), Vec<usize>> = std::collections::HashMap::new();
    for (k, p) in positions.iter().enumerate() {
        if let Some(p) = p {
            buckets.entry(key(*p)).or_default().push(k);
        }
    }
    let mut nearby: Vec<usize> = Vec::new();

    // Pass 1, in agent order: stuck detection and the replan decision — everything that changes
    // an agent's state before its route is known. What it decides is a list of route requests.
    let mut requests: Vec<(usize, Vec3, Vec3)> = Vec::new();
    for (index, &agent_id) in agent_entities.iter().enumerate() {
        let (Some(t), true) = (transforms.get(agent_id), velocities.get_mut(agent_id).is_some()) else {
            continue;
        };
        let Some(mut agent) = agents.get_mut(agent_id) else { continue };
        let Some(mut target_pos) = agent.target else { continue };

        // Stuck Algılama
        if let Some(last_pos) = agent.last_agent_pos {
            if (t.position - last_pos).length_squared() < 0.0025 {
                // 0.05^2
                agent.stuck_timer += dt;
                if agent.stuck_timer > 2.0 {
                    // Yalnız YENİ sıkışma geçişini say/logla (her frame tekrar etmesin).
                    if agent.state != crate::components::NavAgentState::Stuck {
                        stuck_count += 1;
                        tracing::trace!(
                            entity = agent_id,
                            stuck_timer = agent.stuck_timer,
                            "[AI] Ajan sıkıştı — yol temizleniyor, yeniden hesaplanacak"
                        );
                    }
                    agent.state = crate::components::NavAgentState::Stuck;
                    agent.clear_path();
                }
            } else {
                agent.stuck_timer = 0.0;
                agent.last_agent_pos = Some(t.position);
            }
        } else {
            agent.last_agent_pos = Some(t.position);
        }

        // Ajanın sadece XZ düzleminde yürümesi için (pathfinding'in çökmesini engeller)
        target_pos.y = t.position.y;

        agent.recalc.timer -= dt;
        // Forced: no route yet, the route is used up, a wall moved, or the target moved > 2 m.
        // A route that was searched for and does not exist is not "used up": retrying it every
        // frame is the same answer bought again — measured, 4000 agents on a cluttered grid put
        // ~950 searches into every frame that way. It waits for the schedule, a wall or a move.
        let target_moved = agent.recalc.last_target_pos.is_none_or(|last| (last - target_pos).length() > 2.0);
        let forced = (agent.is_done() && !agent.route_failed()) || grid_changed || target_moved;
        let scheduled = agent.recalc.timer <= 0.0;

        if forced || scheduled {
            agent.recalc.last_target_pos = Some(target_pos);
            // A scheduled replan keeps the agent's cadence. A forced one — which a whole crowd
            // gets on the same frame when it is spawned or ordered together — puts the next
            // scheduled one at the agent's own phase of the interval, so the crowd's replans
            // spread over it instead of all landing on one frame every `interval` (measured,
            // 1000 agents: 58 ms on every 30th frame against a 0.27 ms median).
            agent.recalc.timer = if forced {
                agent.recalc.interval * replan_phase(agent_id)
            } else {
                agent.recalc.interval
            };
            recalc_count += 1;
            requests.push((index, t.position, target_pos));
        }
    }

    // Pass 2: the routes, **in parallel**. Each search reads the grid and nothing else (its
    // scratch space is per thread), so the answers are what a sequential loop gives, in any
    // order. A crowd that gets a new target on one frame — spawned together, or an RTS "all of
    // you, there" — used to pay every search on one thread: measured, 1000 agents on a 100 × 100
    // grid, 44 ms that frame against a 2.9 ms median; 4000 agents, 168 ms.
    let routes: Vec<Option<Vec<Vec3>>> = {
        // A handful of searches is cheaper on this thread than spread over the pool.
        #[cfg(not(target_arch = "wasm32"))]
        if requests.len() >= 16 {
            use rayon::prelude::*;
            requests.par_iter().with_min_len(4).map(|&(_, from, to)| grid.find_smooth_path(from, to)).collect()
        } else {
            requests.iter().map(|&(_, from, to)| grid.find_smooth_path(from, to)).collect()
        }
        #[cfg(target_arch = "wasm32")]
        {
            requests.iter().map(|&(_, from, to)| grid.find_smooth_path(from, to)).collect()
        }
    };
    let mut planned: Vec<Option<Option<Vec<Vec3>>>> = vec![None; agent_entities.len()];
    for ((index, _, _), route) in requests.into_iter().zip(routes) {
        planned[index] = Some(route);
    }

    // Pass 3, in agent order: take the route, then steer.
    for (index, &e) in agent_entities.iter().enumerate() {
        let agent_id = e;

        let t = if let Some(t) = transforms.get(agent_id) {
            t
        } else {
            continue;
        };
        let mut v = if let Some(v) = velocities.get_mut(agent_id) {
            v
        } else {
            continue;
        };
        let mut agent = agents.get_mut(agent_id).unwrap();

        if agent.target.is_none() {
            // Yavaşla ve dur (Fiziksel sönümleme)
            v.linear *= 1.0 - (dt * 5.0).min(1.0);
            agent.state = crate::components::NavAgentState::Idle;
            continue;
        }

        let mut target_pos = agent.target.unwrap();
        target_pos.y = t.position.y;

        if let Some(route) = planned[index].take() {
            if let Some(new_path) = route {
                let path_len = new_path.len();
                agent.set_path(new_path);
                agent.set_route_failed(false);
                path_found += 1;
                tracing::trace!(
                    entity = agent_id,
                    path_len,
                    "[AI] Ajan yolu yeniden hesaplandı"
                );
            } else {
                // Yol bulunamadı: path temizlenir (sessiz yutmayı sayaca bağla — çıkışta warn!).
                agent.clear_path();
                agent.set_route_failed(true);
                path_failed += 1;
                tracing::trace!(
                    entity = agent_id,
                    "[AI] Ajan için yol bulunamadı (hedef ulaşılamaz/duvar içinde?)"
                );
            }
        }

        // Neighbours for separation: the buckets the agent's radius overlaps, visited in agent
        // order (sorted), so the forces sum in the same order a full scan sums them.
        let radius = agent.separation_radius;
        let mut neighbor_positions = Vec::new();
        if radius > 0.0 {
            let (lo, hi) = (key(t.position - Vec3::splat(radius)), key(t.position + Vec3::splat(radius)));
            nearby.clear();
            for x in lo.0..=hi.0 {
                for z in lo.1..=hi.1 {
                    if let Some(bucket) = buckets.get(&(x, z)) {
                        nearby.extend_from_slice(bucket);
                    }
                }
            }
            nearby.sort_unstable();
            for &k in &nearby {
                if k == index {
                    continue;
                }
                if let Some(p) = positions[k] {
                    if (p - t.position).length_squared() < radius * radius {
                        neighbor_positions.push(p);
                    }
                }
            }
        }

        let mut steering_force = Vec3::ZERO;

        // Eğer yolda node varsa o node'a doğru ilerle
        if !agent.is_done() {
            let mut next_node = agent.current_waypoint().copied().unwrap_or(t.position);
            next_node.y = t.position.y;

            let dist_to_node = (next_node - t.position).length();

            // Eğer node yarıçapına ulaştıysa VEYA çarpışıp hızı 0'a düştüyse bir sonrakine atla
            if dist_to_node < agent.arrival_radius
                || (v.linear.length() < 0.2 && dist_to_node < agent.arrival_radius * 2.5)
            {
                agent.advance(); // O(1) — remove(0) yerine indeks ilerlet
                if agent.is_done() {
                    // Yolu tamamladı
                    steering_force += steering::arrive(
                        t.position,
                        target_pos,
                        v.linear,
                        agent.max_speed,
                        agent.steering_force,
                        agent.arrival_radius * 2.0,
                    );

                    v.linear += steering_force * dt;
                    let speed = v.linear.length();
                    if speed > agent.max_speed {
                        v.linear = (v.linear / speed) * agent.max_speed;
                    }
                    v.linear.y = 0.0;

                    agent.target = None;
                    agent.state = crate::components::NavAgentState::Reached;
                    reached_count += 1;
                    tracing::trace!(entity = agent_id, "[AI] Ajan hedefe ulaştı");
                    continue;
                } else {
                    next_node = agent.current_waypoint().copied().unwrap_or(t.position);
                    next_node.y = t.position.y;
                }
            }

            // Eğer son node'a geldiysek Arrive, aksi halde Seek kullan
            let remaining_nodes = agent.path_len() - agent.path_index();
            steering_force += if remaining_nodes == 1 {
                steering::arrive(
                    t.position,
                    next_node,
                    v.linear,
                    agent.max_speed,
                    agent.steering_force,
                    agent.arrival_radius * 2.0,
                )
            } else {
                steering::seek(
                    t.position,
                    next_node,
                    v.linear,
                    agent.max_speed,
                    agent.steering_force,
                )
            };

            agent.state = crate::components::NavAgentState::Moving;
        } else {
            // No route: the target is in the agent's own cell (an empty path), or there is none to
            // it. Steering straight at it is all there is — and arriving has to count here too: an
            // agent sent to the cell it stands in used to stay `Moving` with its target set forever,
            // so a patrol whose first point was where the guard started never went on to the next.
            if (target_pos - t.position).length() < agent.arrival_radius {
                v.linear = Vec3::ZERO;
                agent.target = None;
                agent.state = crate::components::NavAgentState::Reached;
                reached_count += 1;
                continue;
            }
            // Yol yoksa, en azından hedef doğrultusunda doğrudan gitmeyi dene
            steering_force += steering::arrive(
                t.position,
                target_pos,
                v.linear,
                agent.max_speed,
                agent.steering_force,
                agent.arrival_radius * 2.0,
            );
            agent.state = crate::components::NavAgentState::Moving;
        }

        // Separation (Bireysel ayrılma kuvveti uygula, kütle çarpışmalarını yumuşatır)
        let separation = steering::separate(
            t.position,
            v.linear,
            &neighbor_positions,
            agent.separation_radius,
            agent.max_speed,
            agent.steering_force,
        );

        // Final kuvveti hıza ekle
        v.linear += (steering_force + separation * agent.separation_weight) * dt;

        // Kuvvet eklendikten sonra linear max_speed ile sınırlandırılır, böylece ajan sonsuza hızlanmaz
        let speed = v.linear.length();
        if speed > agent.max_speed {
            v.linear = (v.linear / speed) * agent.max_speed;
        }

        // Ajanın uzaya uçuşunu engellemek için Y eksenindeki suni birikimleri sıfırla
        v.linear.y = 0.0;
    }

    // Çıkışta AGGREGATE — yalnız bir navigasyon kararı olduysa logla (sessiz frame'ler gürültü yapmasın).
    if recalc_count > 0 || stuck_count > 0 || reached_count > 0 || path_failed > 0 {
        tracing::debug!(
            agent_count,
            recalc_count,
            path_found,
            path_failed,
            stuck_count,
            reached_count,
            "[AI] Navigasyon frame'i işlendi"
        );
    }

    // Yol hesaplama başarısızlıkları gerçek bir sorunu gizleyebilir (ulaşılamaz harita,
    // hatalı hedef, bozuk NavGrid) — çıkışta toplu warn! ile yüzeye çıkar.
    if path_failed > 0 {
        tracing::warn!(
            path_failed,
            recalc_count,
            agent_count,
            "[AI] Bazı ajanlar için yol hesaplanamadı bu frame'de"
        );
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use crate::components::NavAgentState;
    use gizmo_math::Vec3;
    use gizmo_physics_rigid::components::Velocity;

    /// A world with one agent standing at `pos`, wanting to be at `target`.
    fn world_with_agent(pos: Vec3, target: Option<Vec3>) -> (World, gizmo_core::Entity) {
        let mut world = World::new();
        let e = world.spawn();
        world.add_component(e, Transform::new(pos));
        world.add_component(e, Velocity::default());
        let mut agent = NavAgent::default();
        agent.target = target;
        world.add_component(e, agent);
        (world, e)
    }

    /// **The grid nothing ever built.** `ai_navigation_system` returns on its first line without
    /// a `NavGrid`, and no host, scene or script ever inserted one — so this is the assertion
    /// that navigation is reachable at all, not a detail of how the grid is sized.
    #[test]
    fn a_world_with_agents_and_no_grid_gets_one() {
        let (mut world, _) = world_with_agent(Vec3::ZERO, None);
        assert!(world.get_resource::<NavGrid>().is_none());

        ensure_nav_grid(&mut world);

        let grid = world.get_resource::<NavGrid>().expect("agents need a grid");
        assert!(
            grid.is_walkable(grid.world_to_grid(Vec3::ZERO)),
            "an agent standing at the world origin must be inside the grid it was given"
        );
    }

    /// Two things it must NOT do: build a grid for a world with no agents in it, and replace one
    /// a game built for itself (coarser cells, tighter bounds, hand-placed obstacles).
    #[test]
    fn an_existing_grid_is_kept_and_an_agentless_world_gets_none() {
        let mut empty = World::new();
        ensure_nav_grid(&mut empty);
        assert!(empty.get_resource::<NavGrid>().is_none());

        let (mut world, _) = world_with_agent(Vec3::ZERO, None);
        world.insert_resource(NavGrid::new(4.0, 7, 7));
        ensure_nav_grid(&mut world);
        let grid = world.get_resource::<NavGrid>().unwrap();
        assert_eq!((grid.cell_size, grid.width), (4.0, 7));
    }

    /// One call, three systems, in the order that works — and the agent moves toward its target
    /// without the caller having built anything.
    #[test]
    fn one_frame_of_ai_steers_an_agent_that_has_a_target() {
        let (mut world, e) = world_with_agent(Vec3::ZERO, Some(Vec3::new(10.0, 0.0, 0.0)));

        for _ in 0..3 {
            ai_frame(&mut world, 1.0 / 60.0);
        }

        let velocities = world.borrow::<Velocity>();
        let v = velocities.get(e.id()).unwrap().linear;
        assert!(v.x > 0.0, "the agent should be steered toward +X, got {v:?}");
        assert_eq!(v.y, 0.0, "navigation never adds vertical velocity");

        let agents = world.borrow::<NavAgent>();
        assert_eq!(agents.get(e.id()).unwrap().state, NavAgentState::Moving);
    }

    /// A wall dropped on an agent's route makes it replan at once — not when its timer next runs
    /// out — and the new route is longer, because it goes round.
    #[test]
    fn a_wall_on_the_route_makes_the_agent_replan_at_once() {
        use gizmo_core::query::Mut;
        let (mut world, e) = world_with_agent(Vec3::new(0.5, 0.0, 0.5), Some(Vec3::new(8.5, 0.0, 0.5)));
        world.insert_resource({
            let mut g = NavGrid::centred_on(1.0, 32, 32, Vec3::ZERO);
            g.needs_rebuild = false;
            g
        });
        let step = 1.0 / 240.0; // far inside the 0.5 s replan interval
        ai_navigation_system(&world, step);
        let straight = world.borrow::<NavAgent>().get(e.id()).unwrap().path_len();
        assert!(straight > 0, "no first path");

        // A wall across the route, thirteen cells tall: the grid steps diagonally, so going round
        // a short wall can cost no extra waypoints — this one costs about six.
        if let Some(mut grid) = world.get_resource_mut::<NavGrid>() {
            for z in -6..=6 {
                grid.add_obstacle_world(Vec3::new(4.5, 0.0, z as f32 + 0.5));
            }
        }
        ai_navigation_system(&world, step);
        let detour = world.borrow::<NavAgent>().get(e.id()).unwrap().path_len();
        assert!(detour > straight, "still on the old route: {straight} waypoints before, {detour} after");

        // And an unchanged grid does not replan every frame.
        if let Some(mut agents) = world.query_mut::<Mut<NavAgent>>() {
            for (_, mut a) in agents.iter_mut() {
                a.advance();
            }
        }
        let advanced = world.borrow::<NavAgent>().get(e.id()).unwrap().path_index();
        ai_navigation_system(&world, step);
        assert_eq!(world.borrow::<NavAgent>().get(e.id()).unwrap().path_index(), advanced, "replanned with nothing changed");
    }

    /// Separation is the agent's own: two agents walking side by side 1 m apart are pushed apart
    /// by the default 1.5 m radius, and keep their 1 m with a 0.8 m radius or a weight of 0 —
    /// what a formation spaced tighter than 1.5 m needs. A scene written before the fields
    /// existed loads with the defaults.
    #[test]
    fn separation_radius_and_weight_are_per_agent() {
        let gap_after = |agent: NavAgent| {
            let mut world = World::new();
            world.insert_resource({
                let mut g = NavGrid::centred_on(1.0, 80, 20, Vec3::ZERO);
                g.needs_rebuild = false;
                g
            });
            let mut ids = Vec::new();
            for z in [-0.5f32, 0.5] {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::new(0.0, 0.0, z)));
                world.add_component(e, Velocity::default());
                let mut a = agent.clone();
                a.target = Some(Vec3::new(30.0, 0.0, z));
                world.add_component(e, a);
                ids.push(e.id());
            }
            let dt = 1.0 / 60.0;
            for _ in 0..60 {
                ai_navigation_system(&world, dt);
                let moves: Vec<(u32, Vec3)> = {
                    let v = world.borrow::<Velocity>();
                    ids.iter().map(|&id| (id, v.get(id).unwrap().linear)).collect()
                };
                let mut t = world.borrow_mut::<Transform>();
                for (id, vel) in moves {
                    t.get_mut(id).unwrap().position += vel * dt;
                }
            }
            let t = world.borrow::<Transform>();
            assert!(t.get(ids[0]).unwrap().position.x > 1.0, "they walk");
            (t.get(ids[1]).unwrap().position.z - t.get(ids[0]).unwrap().position.z).abs()
        };
        let pushed = gap_after(NavAgent::default());
        let tight = gap_after(NavAgent::default().with_separation(0.8, 1.5));
        let off = gap_after(NavAgent::default().with_separation(1.5, 0.0));
        assert!(pushed > 1.1, "the default 1.5 m radius pushes 1 m neighbours apart: {pushed}");
        assert!((tight - 1.0).abs() < 1e-3, "a 0.8 m radius leaves 1 m alone: {tight}");
        assert!((off - 1.0).abs() < 1e-3, "weight 0 leaves them alone: {off}");

        let old: NavAgent = serde_json::from_str(r#"{"target":null,"max_speed":5.0,"steering_force":10.0,"arrival_radius":0.5}"#)
            .expect("a pre-separation scene row");
        assert_eq!((old.separation_radius, old.separation_weight), (1.5, 1.5));
    }

    /// The bucketed neighbour lookup gives what a scan of every agent gives, to the bit. A crowd
    /// of mixed radii (0.4–3 m) is stepped twice: once as it is, and once beside a far-off agent
    /// with a 1000 m radius, which widens the buckets until they hold the whole crowd — a full
    /// scan in agent order. The crowd's velocities must not differ in a single bit.
    #[test]
    fn bucketed_neighbours_match_a_full_scan_bit_for_bit() {
        use gizmo_math::Rng;
        let crowd = |far_radius: f32| -> Vec<Vec3> {
            let mut world = World::new();
            world.insert_resource({
                let mut g = NavGrid::centred_on(1.0, 2048, 64, Vec3::ZERO);
                g.needs_rebuild = false;
                g
            });
            let mut rng = Rng::new(7);
            let mut ids = Vec::new();
            for _ in 0..120 {
                let e = world.spawn();
                world.add_component(e, Transform::new(Vec3::new(rng.range(-6.0..6.0), 0.0, rng.range(-6.0..6.0))));
                world.add_component(e, Velocity { linear: Vec3::new(rng.range(-1.0..1.0), 0.0, rng.range(-1.0..1.0)), ..Default::default() });
                let mut a = NavAgent::default().with_separation(rng.range(0.4..3.0), 1.5);
                a.target = Some(Vec3::new(20.0, 0.0, rng.range(-5.0..5.0)));
                world.add_component(e, a);
                ids.push(e.id());
            }
            let far = world.spawn();
            world.add_component(far, Transform::new(Vec3::new(900.0, 0.0, 0.0)));
            world.add_component(far, Velocity::default());
            world.add_component(far, NavAgent::default().with_separation(far_radius, 1.5));
            ai_navigation_system(&world, 1.0 / 60.0);
            let v = world.borrow::<Velocity>();
            ids.iter().map(|&id| v.get(id).unwrap().linear).collect()
        };
        let bucketed = crowd(0.5);
        let scanned = crowd(1000.0);
        let differ = bucketed.iter().zip(&scanned).filter(|(a, b)| a.to_array().map(f32::to_bits) != b.to_array().map(f32::to_bits)).count();
        assert_eq!(differ, 0, "{differ} of {} velocities differ from the full scan", bucketed.len());
        assert!(bucketed.iter().any(|v| v.z.abs() > 0.5), "the crowd is pushed at all");
    }

    /// A crowd spawned together plans together once, and then spreads out: after the first
    /// frame, no frame replans more than a few agents more than its share of the interval —
    /// where all 300 used to replan on every 30th frame.
    #[test]
    fn a_crowd_spreads_its_replans_over_the_interval() {
        let mut world = World::new();
        world.insert_resource({
            let mut g = NavGrid::centred_on(1.0, 128, 128, Vec3::ZERO);
            g.needs_rebuild = false;
            g
        });
        let mut ids = Vec::new();
        for i in 0..300u32 {
            let e = world.spawn();
            world.add_component(e, Transform::new(Vec3::new((i % 20) as f32 * 1.2 - 30.0, 0.0, (i / 20) as f32 * 1.2 - 30.0)));
            world.add_component(e, Velocity::default());
            let mut a = NavAgent::default();
            a.target = Some(Vec3::new(50.0, 0.0, 50.0));
            world.add_component(e, a);
            ids.push(e.id());
        }
        let timers = |world: &World| -> Vec<f32> {
            let agents = world.borrow::<NavAgent>();
            ids.iter().map(|&id| agents.get(id).unwrap().recalc.timer).collect()
        };
        let dt = 1.0 / 60.0;
        ai_navigation_system(&world, dt);
        let mut worst = 0;
        for _ in 0..90 {
            let before = timers(&world);
            ai_navigation_system(&world, dt);
            // A replan reloads the timer, so it goes up; otherwise it only counts down.
            let replanned = timers(&world).iter().zip(&before).filter(|(after, b)| after > b).count();
            worst = worst.max(replanned);
        }
        // 300 agents over a 30-frame interval is 10 a frame; the golden-ratio slots are not
        // perfectly even, so allow some bunching — but nothing like 300.
        assert!(worst <= 30, "{worst} agents replanned on one frame");
    }

    /// An agent sent to the cell it stands in arrives — target cleared, `Reached` — instead of
    /// staying `Moving` with the target set forever (its route is empty, and only the route-
    /// following branch used to check for arrival). A patrol whose first point is where the guard
    /// starts depended on it.
    #[test]
    fn an_agent_sent_to_its_own_cell_arrives() {
        let (mut world, e) = world_with_agent(Vec3::new(3.5, 0.0, 3.5), Some(Vec3::new(3.6, 0.0, 3.4)));
        world.insert_resource({
            let mut g = NavGrid::centred_on(1.0, 16, 16, Vec3::ZERO);
            g.needs_rebuild = false;
            g
        });
        ai_navigation_system(&world, 1.0 / 60.0);
        let agents = world.borrow::<NavAgent>();
        let agent = agents.get(e.id()).unwrap();
        // `Reached` on the step it arrives; the next one, with no target, is `Idle`.
        assert_eq!((agent.target, agent.state), (None, NavAgentState::Reached));
    }

    /// An agent with nothing to do is damped, not steered — the same call, the other branch.
    #[test]
    fn an_agent_without_a_target_is_brought_to_a_stop() {
        let (mut world, e) = world_with_agent(Vec3::ZERO, None);
        {
            let mut velocities = world.borrow_mut::<Velocity>();
            velocities.get_mut(e.id()).unwrap().linear = Vec3::new(4.0, 0.0, 0.0);
        }

        for _ in 0..30 {
            ai_frame(&mut world, 1.0 / 60.0);
        }

        let velocities = world.borrow::<Velocity>();
        assert!(velocities.get(e.id()).unwrap().linear.length() < 0.5);
    }

    /// A target walled in on every side: the search fails, `route_failed` says so, and the next
    /// search waits for the schedule instead of running again on every frame (the timer counts
    /// down rather than being reloaded each frame — measured before: ~950 of 4000 agents
    /// re-searching every frame on a cluttered grid).
    #[test]
    fn an_unreachable_target_is_not_searched_for_every_frame() {
        let mut world = World::new();
        let mut grid = NavGrid::new(1.0, 20, 20);
        // A ring of walls around cell (15, 15).
        for (x, z) in [(14, 14), (15, 14), (16, 14), (14, 15), (16, 15), (14, 16), (15, 16), (16, 16)] {
            grid.add_obstacle_world(Vec3::new(x as f32 + 0.5, 0.0, z as f32 + 0.5));
        }
        grid.needs_rebuild = false;
        world.insert_resource(grid);
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(2.5, 0.0, 2.5)));
        world.add_component(e, Velocity::default());
        let mut agent = NavAgent::new(2.0, 8.0, 0.3);
        agent.set_target(Vec3::new(15.5, 0.0, 15.5));
        world.add_component(e, agent);
        let dt = 1.0 / 60.0;
        ai_navigation_system(&world, dt);
        assert!(world.borrow::<NavAgent>().get(e.id()).unwrap().route_failed());
        // Each search reloads the timer; between searches it only counts down. Half a second of
        // frames holds one scheduled search (interval 0.5 s) and maybe the phase-shortened first.
        let mut timers = Vec::new();
        for _ in 0..30 {
            ai_navigation_system(&world, dt);
            timers.push(world.borrow::<NavAgent>().get(e.id()).unwrap().recalc.timer);
        }
        let searches = timers.windows(2).filter(|w| w[1] >= w[0]).count();
        assert!(searches <= 2, "{searches} searches in 30 frames: timers {timers:?}");
        // A reachable target clears the flag.
        world.borrow_mut::<NavAgent>().get_mut(e.id()).unwrap().set_target(Vec3::new(8.5, 0.0, 2.5));
        ai_navigation_system(&world, dt);
        let a = world.borrow::<NavAgent>().get(e.id()).cloned().unwrap();
        assert!(!a.route_failed() && a.path_len() > 0);
    }

    /// The searches run in parallel and give every agent the route a search of its own gives,
    /// the same on every run.
    #[test]
    fn a_crowd_gets_the_routes_a_search_each_would_give() {
        let build = || {
            let mut world = World::new();
            let mut grid = NavGrid::new(1.0, 40, 40);
            for i in 0..40 {
                grid.add_obstacle_world(Vec3::new(20.5, 0.0, i as f32 + 0.5));
            }
            grid.remove_obstacle_world(Vec3::new(20.5, 0.0, 30.5));
            grid.needs_rebuild = false;
            world.insert_resource(grid);
            let mut ids = Vec::new();
            for i in 0..120u32 {
                let e = world.spawn();
                let start = Vec3::new((i % 15) as f32 + 2.5, 0.0, (i / 15) as f32 * 4.0 + 2.5);
                world.add_component(e, Transform::new(start));
                world.add_component(e, Velocity::default());
                let mut a = NavAgent::new(2.0, 8.0, 0.3);
                a.set_target(Vec3::new(35.5, 0.0, (i % 40) as f32 + 0.5));
                world.add_component(e, a);
                ids.push((e.id(), start, Vec3::new(35.5, 0.0, (i % 40) as f32 + 0.5)));
            }
            (world, ids)
        };
        let (world, ids) = build();
        ai_navigation_system(&world, 1.0 / 60.0);
        let grid = world.get_resource::<NavGrid>().unwrap();
        for &(id, start, target) in &ids {
            let alone = grid.find_smooth_path(start, target).map(|p| p.len()).unwrap_or(0);
            assert_eq!(world.borrow::<NavAgent>().get(id).unwrap().path_len(), alone, "agent {id}");
        }
        drop(grid);
        let (again, _) = build();
        ai_navigation_system(&again, 1.0 / 60.0);
        for &(id, ..) in &ids {
            assert_eq!(
                world.borrow::<Velocity>().get(id).unwrap().linear,
                again.borrow::<Velocity>().get(id).unwrap().linear,
                "agent {id} differs between two runs"
            );
        }
    }
}

