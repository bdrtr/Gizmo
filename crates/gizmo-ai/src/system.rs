//! ECS systems that move [`NavAgent`] entities along paths found on a [`NavGrid`].
//!
//! Both systems are meant to run once per frame, [`ai_nav_grid_rebuild_system`] before
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

use gizmo_core::Entity;
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
/// [`ai_nav_grid_rebuild_system`] fills, so the reverse plans one frame of paths against a stale
/// one, and both read agents a behaviour tree may have retargeted.
///
/// Takes `&mut World` because [`ensure_nav_grid`] may have to *create* the grid resource, which
/// the two systems below — which only mutate what already exists — do not.
///
/// Cheap on a scene with no AI in it: three empty component borrows and a resource lookup.
pub fn ai_frame(world: &mut World, dt: f32) {
    crate::behavior_tree::behavior_tree_system(world, dt);
    ensure_nav_grid(world);
    ai_nav_grid_rebuild_system(world, dt);
    ai_navigation_system(world, dt);
}

/// Gives the world a [`NavGrid`] if it has agents that need one and nothing supplied it.
///
/// **Nothing in the engine ever constructed a `NavGrid`.** Not the editor, not `PlayLoop`, not a
/// scene (the resource is not serialised), and not Lua — `ai.add_nav_agent` and `ai.set_target`
/// exist, so a script could ask for navigation, but the grid those two need had no way into a
/// world at all. [`ai_navigation_system`] returns on its first line without one, so every
/// `NavAgent` ever created stood still, in the editor and in exported games alike. This is the
/// missing constructor, and it is the same call the studio's navigation-grid rebuild button makes.
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
    if world.query::<&NavAgent>().is_empty() {
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
pub fn ai_nav_grid_rebuild_system(world: &World, _dt: f32) {
    let grid_opt = world.get_resource_mut::<NavGrid>();
    let physics_opt = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>();

    if let (Some(mut grid), Some(physics_world)) = (grid_opt, physics_opt) {
        if grid.needs_rebuild {
            grid.update_from_physics_world(&physics_world);
            tracing::info!(
                obstacle_count = grid.obstacles.len(),
                "[AI] NavGrid PhysicsWorld üzerinden yeniden oluşturuldu"
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

/// How far avoidance looks first around an agent of body `radius` for its nearest neighbours, metres:
/// six of its widths, between half a metre and two — where, in a crowd, its ten nearest are.
fn near_search(radius: f32) -> f32 {
    (6.0 * radius).clamp(0.5, 2.0)
}
/// How many of the nearest neighbours avoidance weighs (RVO2's `maxNeighbors`).
const MAX_AVOIDED: usize = 10;

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

    // Buckets as wide as the widest separation, and no wider than the first avoidance search: a
    // bucket the size of a whole reach puts a crowd in nine of them.
    let (agent_entities, cell): (Vec<Entity>, f32) = {
        let agents = world.query::<(Entity, &NavAgent)>();
        let widest = agents.iter().map(|(_, a)| a.separation_radius.max(reach(a).min(near_search(a.radius)))).fold(0.0f32, f32::max);
        (agents.iter().map(|(entity, _)| entity).collect(), widest.max(0.01))
    };
    let agent_count = agent_entities.len();

    // Frame-başı toplu sayaçlar — iç döngüde per-iterasyon log yerine çıkışta AGGREGATE.
    let mut tally = NavTally::default();

    // SAFETY: ai_navigation_system runs as a scheduled system; the scheduler guarantees no
    // other system mutably aliases NavAgent/Velocity while it runs. NavAgent and Velocity are
    // distinct component types, so their storages never overlap with each other either.
    let mut agents = unsafe { world.query_unchecked::<&mut NavAgent>() };
    let transforms = world.query::<&Transform>();
    // SAFETY: same argument as `agents` above — Velocity is a distinct component type from
    // both NavAgent and Transform, so this view aliases neither.
    let mut velocities = unsafe { world.query_unchecked::<&mut Velocity>() };

    // Neighbours by bucket, not by scanning every agent: positions do not change during this
    // pass (it writes velocities only), so one snapshot serves every agent. Buckets are XZ cells
    // as wide as the largest separation radius, so an agent's neighbours lie in the cells its
    // own radius overlaps. Measured, 1000 / 2000 agents: 13.1 / 53.2 ms a frame with the scan.
    let positions: Vec<Option<Vec3>> = agent_entities.iter().map(|&id| transforms.get(id).map(|t| t.position)).collect();
    // What avoidance reads of the others, as the frame began: how they move, how wide they are,
    // whether they will step aside (they avoid, and have somewhere to go).
    let others: Vec<(Vec3, f32, bool)> = agent_entities
        .iter()
        .map(|&id| {
            let velocity = velocities.get_mut(id).map_or(Vec3::ZERO, |v| v.linear);
            let agent = agents.get_mut(id);
            let (radius, steps_aside) = agent.map_or((0.0, false), |a| (a.radius, a.avoidance && a.target.is_some()));
            (velocity, radius, steps_aside)
        })
        .collect();
    let crowd = Crowd::new(positions, others, cell);
    let mut nearby: Vec<usize> = Vec::new();

    // Pass 1, in agent order: stuck detection and the replan decision — everything that changes
    // an agent's state before its route is known. What it decides is a list of route requests.
    let mut requests: Vec<(usize, Vec3, Vec3)> = Vec::new();
    for (index, &agent_id) in agent_entities.iter().enumerate() {
        let (Some(t), true) = (transforms.get(agent_id), velocities.get_mut(agent_id).is_some()) else {
            continue;
        };
        let Some(mut agent) = agents.get_mut(agent_id) else { continue };
        let Some(target_pos) = agent.target else { continue };
        if let Some(to) = decide_replan(&mut agent, agent_id, t, target_pos, dt, grid_changed, &mut tally) {
            requests.push((index, t.position, to));
        }
    }

    // Pass 2: the routes, **in parallel**. Each search reads the grid and nothing else (its
    // scratch space is per thread), so the answers are what a sequential loop gives, in any
    // order. A crowd that gets a new target on one frame — spawned together, or an RTS "all of
    // you, there" — used to pay every search on one thread: measured, 1000 agents on a 100 × 100
    // grid, 44 ms that frame against a 2.9 ms median; 4000 agents, 168 ms.
    let routes = find_routes(grid, &requests);
    let mut planned: Vec<Option<Option<Vec<Vec3>>>> = vec![None; agent_entities.len()];
    for ((index, _, _), route) in requests.into_iter().zip(routes) {
        planned[index] = Some(route);
    }

    // Pass 3, in agent order: take the route, then steer.
    for (index, &agent_id) in agent_entities.iter().enumerate() {
        let Some(t) = transforms.get(agent_id) else { continue };
        let Some(mut v) = velocities.get_mut(agent_id) else { continue };
        let mut agent = agents.get_mut(agent_id).unwrap();
        steer(&mut agent, &mut v, agent_id, t, planned[index].take(), &crowd, index, &mut nearby, dt, &mut tally);
    }

    let NavTally { recalc: recalc_count, path_found, path_failed, newly_failed, stuck: stuck_count, reached: reached_count } = tally;
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
    // hatalı hedef, bozuk NavGrid) — çıkışta toplu warn! ile yüzeye çıkar. Only the NEW ones: an
    // agent searching again, on its schedule, for a target it already could not reach was warned
    // about on every retry — `karinca --test` logged this 900 times for one ant. The retries are in
    // the debug summary above.
    if newly_failed > 0 {
        tracing::warn!(
            newly_failed,
            path_failed,
            recalc_count,
            agent_count,
            "[AI] Bazı ajanlar için yol hesaplanamadı bu frame'de"
        );
    }
}

/// An agent's reach: how far it looks for neighbours, to separate from or to avoid (what it could
/// cover in its avoidance horizon, and two bodies' width).
fn reach(a: &NavAgent) -> f32 {
    let avoid = if a.avoidance { a.max_speed * a.time_horizon + 2.0 * a.radius } else { 0.0 };
    a.separation_radius.max(avoid)
}

/// One frame's navigation tallies, logged once at the end rather than per agent.
#[derive(Default)]
struct NavTally {
    recalc: u32,
    path_found: u32,
    path_failed: u32,
    /// Failures that are not an agent retrying a target it already could not reach.
    newly_failed: u32,
    stuck: u32,
    reached: u32,
}

/// The crowd as the frame began, for separation and avoidance: where each agent is, what avoidance
/// reads of it, and which XZ bucket it is in.
struct Crowd {
    positions: Vec<Option<Vec3>>,
    /// How each agent moves, how wide it is, whether it will step aside (it avoids, and has
    /// somewhere to go).
    others: Vec<(Vec3, f32, bool)>,
    buckets: std::collections::HashMap<(i32, i32), Vec<usize>>,
    cell: f32,
}

impl Crowd {
    /// The crowd, bucketed in XZ cells `cell` wide.
    fn new(positions: Vec<Option<Vec3>>, others: Vec<(Vec3, f32, bool)>, cell: f32) -> Self {
        let mut crowd = Crowd { positions, others, buckets: std::collections::HashMap::new(), cell };
        for (k, p) in crowd.positions.iter().enumerate() {
            if let Some(p) = p {
                crowd.buckets.entry(crowd.key(*p)).or_default().push(k);
            }
        }
        crowd
    }

    fn key(&self, p: Vec3) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.z / self.cell).floor() as i32)
    }

    /// Agent `index`'s neighbours: the ones within its separation radius, and for avoidance the
    /// ones within its reach with their squared distances.
    fn neighbours(
        &self,
        index: usize,
        t: &Transform,
        agent: &NavAgent,
        nearby: &mut Vec<usize>,
    ) -> (Vec<Vec3>, Vec<(f32, crate::avoidance::Neighbour)>) {
        // Neighbours, for separation and for avoidance: the buckets a radius around the agent
        // overlaps, visited in agent order (sorted), so the forces sum in the same order a full scan
        // sums them. Avoidance wants the ten nearest within the agent's reach; in a crowd they are a
        // few of its own widths away, so the search starts there (`near_search`) and widens to the
        // whole reach only when it has not found ten — measured, 600 ants searching their whole 4 m
        // reach every frame: 12 ms.
        let radius = agent.separation_radius;
        let look = reach(agent);
        let mut neighbor_positions = Vec::new();
        let mut avoid_neighbours: Vec<(f32, crate::avoidance::Neighbour)> = Vec::new();
        let first = if agent.avoidance { look.min(near_search(agent.radius)).max(radius) } else { radius };
        for (pass, within) in [first, look].into_iter().enumerate() {
            if within <= 0.0 || (pass == 1 && (!agent.avoidance || within <= first || avoid_neighbours.len() >= MAX_AVOIDED)) {
                continue;
            }
            neighbor_positions.clear();
            avoid_neighbours.clear();
            let (lo, hi) = (self.key(t.position - Vec3::splat(within)), self.key(t.position + Vec3::splat(within)));
            nearby.clear();
            for x in lo.0..=hi.0 {
                for z in lo.1..=hi.1 {
                    if let Some(bucket) = self.buckets.get(&(x, z)) {
                        nearby.extend_from_slice(bucket);
                    }
                }
            }
            nearby.sort_unstable();
            for &k in nearby.iter() {
                if k == index {
                    continue;
                }
                if let Some(p) = self.positions[k] {
                    let dist_sq = (p - t.position).length_squared();
                    if dist_sq < radius * radius {
                        neighbor_positions.push(p);
                    }
                    if agent.avoidance && dist_sq < within * within {
                        let (velocity, r, steps_aside) = self.others[k];
                        avoid_neighbours.push((
                            dist_sq,
                            crate::avoidance::Neighbour {
                                position: gizmo_math::Vec2::new(p.x, p.z),
                                velocity: gizmo_math::Vec2::new(velocity.x, velocity.z),
                                radius: r,
                                reciprocal: steps_aside,
                            },
                        ));
                    }
                }
            }
        }
        (neighbor_positions, avoid_neighbours)
    }
}

/// Pass 1 for one agent with a target: stuck detection, and whether its route is planned again
/// this frame — `Some(target)`, flattened onto the agent's height, when it is.
#[allow(clippy::too_many_arguments)]
fn decide_replan(
    agent: &mut NavAgent,
    agent_id: Entity,
    t: &Transform,
    mut target_pos: Vec3,
    dt: f32,
    grid_changed: bool,
    tally: &mut NavTally,
) -> Option<Vec3> {
    // Stuck Algılama
    if let Some(last_pos) = agent.last_agent_pos {
        if (t.position - last_pos).length_squared() < 0.0025 {
            // 0.05^2
            agent.stuck_timer += dt;
            if agent.stuck_timer > 2.0 {
                // Yalnız YENİ sıkışma geçişini say/logla (her frame tekrar etmesin).
                if agent.state != crate::components::NavAgentState::Stuck {
                    tally.stuck += 1;
                    tracing::trace!(
                        entity = %agent_id,
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
            agent.recalc.interval * replan_phase(agent_id.id())
        } else {
            agent.recalc.interval
        };
        tally.recalc += 1;
        return Some(target_pos);
    }
    None
}

/// Pass 2: the routes the replans asked for, in their order.
fn find_routes(grid: &NavGrid, requests: &[(usize, Vec3, Vec3)]) -> Vec<Option<Vec<Vec3>>> {
    {
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
    }
}

/// The route pass 2 found for this agent, if it asked for one: taken, or the failure recorded.
fn take_route(agent: &mut NavAgent, agent_id: Entity, route: Option<Option<Vec<Vec3>>>, tally: &mut NavTally) {
    if let Some(route) = route {
        if let Some(new_path) = route {
            let path_len = new_path.len();
            agent.set_path(new_path);
            agent.set_route_failed(false);
            tally.path_found += 1;
            tracing::trace!(
                entity = %agent_id,
                path_len,
                "[AI] Ajan yolu yeniden hesaplandı"
            );
        } else {
            // Yol bulunamadı: path temizlenir (sessiz yutmayı sayaca bağla — çıkışta warn!).
            agent.clear_path();
            let repeat = agent.set_route_failed(true);
            tally.path_failed += 1;
            if !repeat {
                tally.newly_failed += 1;
            }
            tracing::trace!(
                entity = %agent_id,
                "[AI] Ajan için yol bulunamadı (hedef ulaşılamaz/duvar içinde?)"
            );
        }
    }
}

/// Pass 3 for one agent: take its route, then steer — the route's force, separation, the speed
/// limit, avoidance — or slow to a stop with no target.
#[allow(clippy::too_many_arguments)]
fn steer(
    agent: &mut NavAgent,
    v: &mut Velocity,
    agent_id: Entity,
    t: &Transform,
    route: Option<Option<Vec<Vec3>>>,
    crowd: &Crowd,
    index: usize,
    nearby: &mut Vec<usize>,
    dt: f32,
    tally: &mut NavTally,
) {
    if agent.target.is_none() {
        // Yavaşla ve dur (Fiziksel sönümleme)
        v.linear *= 1.0 - (dt * 5.0).min(1.0);
        agent.state = crate::components::NavAgentState::Idle;
        return;
    }

    let mut target_pos = agent.target.unwrap();
    target_pos.y = t.position.y;

    take_route(agent, agent_id, route, tally);

    // Neighbours, for separation and for avoidance: the buckets a radius around the agent
    // overlaps, visited in agent order (sorted), so the forces sum in the same order a full scan
    // sums them. Avoidance wants the ten nearest within the agent's reach; in a crowd they are a
    // few of its own widths away, so the search starts there (`near_search`) and widens to the
    // whole reach only when it has not found ten — measured, 600 ants searching their whole 4 m
    // reach every frame: 12 ms.
    let (neighbor_positions, avoid_neighbours) = crowd.neighbours(index, t, agent, nearby);

    let Some(steering_force) = path_force(agent, v, agent_id, t, target_pos, dt, tally) else {
        return;
    };

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

    apply_avoidance(agent, v, t, crowd.others[index].0, avoid_neighbours, dt);

    // Ajanın uzaya uçuşunu engellemek için Y eksenindeki suni birikimleri sıfırla
    v.linear.y = 0.0;
}

/// The force along the agent's route — seek, or arrive on its last waypoint — or straight at the
/// target with no route. `None` when the agent arrived this frame (its velocity then already set).
fn path_force(
    agent: &mut NavAgent,
    v: &mut Velocity,
    agent_id: Entity,
    t: &Transform,
    target_pos: Vec3,
    dt: f32,
    tally: &mut NavTally,
) -> Option<Vec3> {
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
                tally.reached += 1;
                tracing::trace!(entity = %agent_id, "[AI] Ajan hedefe ulaştı");
                return None;
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
            tally.reached += 1;
            return None;
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
    Some(steering_force)
}

/// Avoidance: the velocity nearest the agent's that runs into none of its nearest neighbours
/// within the horizon. `own` is how it moved as the frame began.
fn apply_avoidance(
    agent: &NavAgent,
    v: &mut Velocity,
    t: &Transform,
    own: Vec3,
    mut avoid_neighbours: Vec<(f32, crate::avoidance::Neighbour)>,
    dt: f32,
) {
    // Avoidance: the velocity nearest this one that runs into no neighbour within the horizon —
    // the ten nearest, as RVO2 takes them.
    if agent.avoidance && !avoid_neighbours.is_empty() {
        if avoid_neighbours.len() > MAX_AVOIDED {
            avoid_neighbours.select_nth_unstable_by(MAX_AVOIDED - 1, |a, b| a.0.total_cmp(&b.0));
            avoid_neighbours.truncate(MAX_AVOIDED);
        }
        // Nearest first, and so the same order whatever the buckets gave.
        avoid_neighbours.sort_by(|a, b| a.0.total_cmp(&b.0));
        let neighbours: Vec<crate::avoidance::Neighbour> = avoid_neighbours.iter().map(|(_, n)| *n).collect();
        let chosen = crate::avoidance::avoid(
            gizmo_math::Vec2::new(t.position.x, t.position.z),
            gizmo_math::Vec2::new(own.x, own.z),
            agent.radius,
            gizmo_math::Vec2::new(v.linear.x, v.linear.z),
            agent.max_speed,
            &neighbours,
            agent.time_horizon,
            dt,
        );
        v.linear.x = chosen.x;
        v.linear.z = chosen.y;
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

        let velocities = world.query::<&Velocity>();
        let v = velocities.get(e).unwrap().linear;
        assert!(v.x > 0.0, "the agent should be steered toward +X, got {v:?}");
        assert_eq!(v.y, 0.0, "navigation never adds vertical velocity");

        let agents = world.query::<&NavAgent>();
        assert_eq!(agents.get(e).unwrap().state, NavAgentState::Moving);
    }

    /// A wall dropped on an agent's route makes it replan at once — not when its timer next runs
    /// out — and the new route is longer, because it goes round.
    #[test]
    fn a_wall_on_the_route_makes_the_agent_replan_at_once() {
        
        let (mut world, e) = world_with_agent(Vec3::new(0.5, 0.0, 0.5), Some(Vec3::new(8.5, 0.0, 0.5)));
        world.insert_resource({
            let mut g = NavGrid::centred_on(1.0, 32, 32, Vec3::ZERO);
            g.needs_rebuild = false;
            g
        });
        let step = 1.0 / 240.0; // far inside the 0.5 s replan interval
        ai_navigation_system(&world, step);
        let straight = world.query::<&NavAgent>().get(e).unwrap().path_len();
        assert!(straight > 0, "no first path");

        // A wall across the route, thirteen cells tall: the grid steps diagonally, so going round
        // a short wall can cost no extra waypoints — this one costs about six.
        if let Some(mut grid) = world.get_resource_mut::<NavGrid>() {
            for z in -6..=6 {
                grid.add_obstacle_world(Vec3::new(4.5, 0.0, z as f32 + 0.5));
            }
        }
        ai_navigation_system(&world, step);
        let detour = world.query::<&NavAgent>().get(e).unwrap().path_len();
        assert!(detour > straight, "still on the old route: {straight} waypoints before, {detour} after");

        // And an unchanged grid does not replan every frame.
        {
            let mut agents = world.query_mut::<&mut NavAgent>();
            for mut a in agents.iter_mut() {
                a.advance();
            }
        }
        let advanced = world.query::<&NavAgent>().get(e).unwrap().path_index();
        ai_navigation_system(&world, step);
        assert_eq!(world.query::<&NavAgent>().get(e).unwrap().path_index(), advanced, "replanned with nothing changed");
    }

    /// **Two agents walking at each other step aside and pass.** Head-on along a corridor-free grid,
    /// each to the other's start: with avoidance (the default) they never come closer than their two
    /// bodies' width and both arrive; with it off they walk into each other (separation alone pushes
    /// only once they are close), coming far nearer.
    #[test]
    fn two_agents_head_on_step_aside_and_pass() {
        let closest = |agent: NavAgent| {
            let mut world = World::new();
            world.insert_resource({
                let mut g = NavGrid::centred_on(1.0, 40, 20, Vec3::ZERO);
                g.needs_rebuild = false;
                g
            });
            let ends = [(-8.0f32, 8.0f32), (8.0, -8.0)];
            let ids: Vec<Entity> = ends
                .iter()
                .map(|&(from, to)| {
                    let e = world.spawn();
                    world.add_component(e, Transform::new(Vec3::new(from, 0.0, 0.0)));
                    world.add_component(e, Velocity::default());
                    let mut a = agent.clone();
                    a.max_speed = 2.0;
                    a.target = Some(Vec3::new(to, 0.0, 0.0));
                    world.add_component(e, a);
                    e
                })
                .collect();
            let dt = 1.0 / 60.0;
            let mut nearest = f32::INFINITY;
            for _ in 0..(14.0 / dt) as usize {
                ai_navigation_system(&world, dt);
                let moves: Vec<(Entity, Vec3)> = {
                    let v = world.query::<&Velocity>();
                    ids.iter().map(|&id| (id, v.get(id).unwrap().linear)).collect()
                };
                let mut t = world.query_mut::<&mut Transform>();
                for (id, vel) in moves {
                    t.get_mut(id).unwrap().position += vel * dt;
                }
                drop(t);
                let t = world.query::<&Transform>();
                nearest = nearest.min((t.get(ids[0]).unwrap().position - t.get(ids[1]).unwrap().position).length());
            }
            let arrived = ids.iter().all(|&id| world.query::<&NavAgent>().get(id).is_some_and(|a| a.target.is_none()));
            (nearest, arrived)
        };
        let (avoiding, arrived) = closest(NavAgent::default());
        assert!(avoiding >= 0.8 - 0.05, "they came {avoiding} m apart, under their bodies' 0.8 m");
        assert!(arrived, "both arrived");
        let (blind, _) = closest(NavAgent::default().without_avoidance());
        assert!(blind < avoiding - 0.2, "without avoidance they walk into each other: {blind} m, against {avoiding} m");
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
                ids.push(e);
            }
            let dt = 1.0 / 60.0;
            for _ in 0..60 {
                ai_navigation_system(&world, dt);
                let moves: Vec<(Entity, Vec3)> = {
                    let v = world.query::<&Velocity>();
                    ids.iter().map(|&id| (id, v.get(id).unwrap().linear)).collect()
                };
                let mut t = world.query_mut::<&mut Transform>();
                for (id, vel) in moves {
                    t.get_mut(id).unwrap().position += vel * dt;
                }
            }
            let t = world.query::<&Transform>();
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
                ids.push(e);
            }
            let far = world.spawn();
            world.add_component(far, Transform::new(Vec3::new(900.0, 0.0, 0.0)));
            world.add_component(far, Velocity::default());
            world.add_component(far, NavAgent::default().with_separation(far_radius, 1.5));
            ai_navigation_system(&world, 1.0 / 60.0);
            let v = world.query::<&Velocity>();
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
            ids.push(e);
        }
        let timers = |world: &World| -> Vec<f32> {
            let agents = world.query::<&NavAgent>();
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
        let agents = world.query::<&NavAgent>();
        let agent = agents.get(e).unwrap();
        // `Reached` on the step it arrives; the next one, with no target, is `Idle`.
        assert_eq!((agent.target, agent.state), (None, NavAgentState::Reached));
    }

    /// An agent with nothing to do is damped, not steered — the same call, the other branch.
    #[test]
    fn an_agent_without_a_target_is_brought_to_a_stop() {
        let (mut world, e) = world_with_agent(Vec3::ZERO, None);
        {
            let mut velocities = world.query_mut::<&mut Velocity>();
            velocities.get_mut(e).unwrap().linear = Vec3::new(4.0, 0.0, 0.0);
        }

        for _ in 0..30 {
            ai_frame(&mut world, 1.0 / 60.0);
        }

        let velocities = world.query::<&Velocity>();
        assert!(velocities.get(e).unwrap().linear.length() < 0.5);
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
        assert!(world.query::<&NavAgent>().get(e).unwrap().route_failed());
        // Each search reloads the timer; between searches it only counts down. Half a second of
        // frames holds one scheduled search (interval 0.5 s) and maybe the phase-shortened first.
        let mut timers = Vec::new();
        for _ in 0..30 {
            ai_navigation_system(&world, dt);
            timers.push(world.query::<&NavAgent>().get(e).unwrap().recalc.timer);
        }
        let searches = timers.windows(2).filter(|w| w[1] >= w[0]).count();
        assert!(searches <= 2, "{searches} searches in 30 frames: timers {timers:?}");
        // A reachable target clears the flag.
        world.query_mut::<&mut NavAgent>().get_mut(e).unwrap().set_target(Vec3::new(8.5, 0.0, 2.5));
        ai_navigation_system(&world, dt);
        let a = world.query::<&NavAgent>().get(e).cloned().unwrap();
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
                ids.push((e, start, Vec3::new(35.5, 0.0, (i % 40) as f32 + 0.5)));
            }
            (world, ids)
        };
        let (world, ids) = build();
        ai_navigation_system(&world, 1.0 / 60.0);
        let grid = world.get_resource::<NavGrid>().unwrap();
        for &(id, start, target) in &ids {
            let alone = grid.find_smooth_path(start, target).map(|p| p.len()).unwrap_or(0);
            assert_eq!(world.query::<&NavAgent>().get(id).unwrap().path_len(), alone, "agent {id}");
        }
        drop(grid);
        let (again, _) = build();
        ai_navigation_system(&again, 1.0 / 60.0);
        for &(id, ..) in &ids {
            assert_eq!(
                world.query::<&Velocity>().get(id).unwrap().linear,
                again.query::<&Velocity>().get(id).unwrap().linear,
                "agent {id} differs between two runs"
            );
        }
    }
}
