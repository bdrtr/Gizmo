use crate::components::NavAgent;
use crate::pathfinding::NavGrid;
use crate::steering;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{Transform, Velocity};

/// AI Navigasyon Sistemi — Per-frame ajan güncelleme döngüsü.
///
/// **Borrow Güvenliği Notu:**
/// Bu fonksiyon eşzamanlı 4 RefCell borrow tutar:
/// - `NavGrid` (Resource, immutable)
/// - `NavAgent` (Component, **mutable**)
/// - `Transform` (Component, immutable)
/// - `Velocity` (Component, **mutable**)
///
/// Bu güvenlidir çünkü her biri **farklı TypeId** ile ayrı `RefCell`'de saklanır.
/// Ancak bu fonksiyon çalışırken başka bir sistem aynı anda `NavAgent` veya
/// `Velocity` için `borrow_mut` yaparsa `try_borrow_mut` başarısız olur.
/// Bu nedenle AI sistemi fizik adım döngüsü **içinde** çağrılmalıdır (main.rs),
/// dışardan paralel çağrılmamalıdır.
pub fn ai_navigation_system(world: &World, dt: f32) {
    let grid = match world.get_resource::<NavGrid>() {
        Some(g) => g,
        None => return,
    };

    let mut agents = world.borrow_mut::<NavAgent>();
    let transforms = world.borrow::<Transform>();
    let mut velocities = world.borrow_mut::<Velocity>();

    let mut agent_entities: Vec<u32> = Vec::with_capacity(agents.len());
    for (id, _) in agents.iter() {
        agent_entities.push(id);
    }

    // Iterasyon
    for &e in &agent_entities {
        let agent_id = e;

        let t = if let Some(t) = transforms.get(agent_id) {
            t
        } else {
            continue;
        };
        let v = if let Some(v) = velocities.get_mut(agent_id) {
            v
        } else {
            continue;
        };
        let agent = agents.get_mut(agent_id).unwrap();

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

            if let Some(new_path) = grid.find_path(t.position, target_pos) {
                agent.set_path(new_path);
            } else {
                agent.clear_path();
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
}
