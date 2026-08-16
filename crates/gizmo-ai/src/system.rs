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
///   or nothing has been planned yet. If `NavGrid::find_path` finds nothing — target inside an
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
/// Neighbour lookup is a brute-force O(N²) scan over all agents with a hard-coded 10 m radius,
/// separation radius 1.5 m and separation weight 1.5; none of these are configurable per agent,
/// and the cost grows quadratically with agent count.
///
/// # Aliasing
///
/// This takes `&World` but mutates `NavAgent` and `Velocity` through unchecked borrows, so the
/// compiler cannot catch a conflict. It must not run concurrently with any other system that
/// touches those two component types mutably.
#[tracing::instrument(skip_all, name = "ai_navigation_system")]
pub fn ai_navigation_system(world: &World, dt: f32) {
    let grid = match world.get_resource::<NavGrid>() {
        Some(g) => g,
        None => {
            tracing::trace!("[AI] NavGrid resource yok, navigasyon atlanıyor");
            return;
        }
    };

    let agent_entities: Vec<u32> = world.borrow::<NavAgent>().entities().collect();
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

    // Iterasyon
    for &e in &agent_entities {
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

        let mut target_pos = agent.target.unwrap();
        // Ajanın sadece XZ düzleminde yürümesi için (pathfinding'in çökmesini engeller)
        target_pos.y = t.position.y;

        agent.recalc.timer -= dt;
        let mut needs_recalc = agent.is_done() || agent.recalc.timer <= 0.0;

        // Hedef çok yer değiştirdiyse yeniden hesapla
        if let Some(last_pos) = agent.recalc.last_target_pos {
            if (last_pos - target_pos).length() > 2.0 {
                needs_recalc = true;
            }
        } else {
            needs_recalc = true;
        }

        if needs_recalc {
            agent.recalc.last_target_pos = Some(target_pos);
            agent.recalc.timer = agent.recalc.interval;
            recalc_count += 1;

            if let Some(new_path) = grid.find_path(t.position, target_pos) {
                let path_len = new_path.len();
                agent.set_path(new_path);
                path_found += 1;
                tracing::trace!(
                    entity = agent_id,
                    path_len,
                    "[AI] Ajan yolu yeniden hesaplandı"
                );
            } else {
                // Yol bulunamadı: path temizlenir (sessiz yutmayı sayaca bağla — çıkışta warn!).
                agent.clear_path();
                path_failed += 1;
                tracing::trace!(
                    entity = agent_id,
                    "[AI] Ajan için yol bulunamadı (hedef ulaşılamaz/duvar içinde?)"
                );
            }
        }

        // Komşuları bul (Separation / Avoidance için)
        // Optimizasyon: O(N^2) yerine basit dist check. Agent sayısı düşükse sorun olmaz.
        let mut neighbor_positions = Vec::new();
        for &other_e in &agent_entities {
            if other_e != agent_id {
                if let Some(ot) = transforms.get(other_e) {
                    if (ot.position - t.position).length_squared() < 100.0 {
                        neighbor_positions.push(ot.position);
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
            1.5,
            agent.max_speed,
            agent.steering_force,
        );

        // Final kuvveti hıza ekle
        v.linear += (steering_force + separation * 1.5) * dt;

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
