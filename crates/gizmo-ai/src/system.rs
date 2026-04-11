use crate::components::NavAgent;
use crate::pathfinding::{find_path, NavGrid};
use crate::steering;
use gizmo_core::World;
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

    let mut agents = match world.borrow_mut::<NavAgent>() {
        Some(a) => a,
        None => return,
    };

    let transforms = match world.borrow::<Transform>() {
        Some(t) => t,
        None => return,
    };

    let mut velocities = match world.borrow_mut::<Velocity>() {
        Some(v) => v,
        None => return,
    };

    let agent_entities: Vec<u32> = agents.dense.iter().map(|e| e.entity).collect();

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
            // Yavaşla ve dur
            v.linear = steering::arrive(
                t.position,
                t.position,
                v.linear,
                agent.max_speed,
                agent.max_force,
                1.0,
            ) * dt;
            continue;
        }

        let mut target_pos = agent.target.unwrap();
        // Ajanın sadece XZ düzleminde yürümesi için (pathfinding'in çökmesini engeller)
        target_pos.y = t.position.y;

        agent.path_recalc_timer -= dt;
        let mut needs_recalc =
            agent.current_path_index >= agent.path.len() || agent.path_recalc_timer <= 0.0;

        // Hedef çok yer değiştirdiyse yeniden hesapla
        if let Some(last_pos) = agent.last_target_pos {
            if (last_pos - target_pos).length() > 2.0 {
                needs_recalc = true;
            }
        } else {
            needs_recalc = true;
        }

        if needs_recalc {
            agent.last_target_pos = Some(target_pos);
            agent.path_recalc_timer = 1.0;

            if let Some(new_path) = find_path(&*grid, t.position, target_pos) {
                agent.path = new_path;
                agent.current_path_index = 0; // Yeni yol — baştan başla
            } else {
                agent.path.clear();
                agent.current_path_index = 0;
            }
        }

        // Eğer yolda node varsa o node'a doğru ilerle
        if agent.current_path_index < agent.path.len() {
            let mut next_node = agent.path[agent.current_path_index];
            next_node.y = t.position.y;

            let dist_to_node = (next_node - t.position).length();

            // Eğer node yarıçapına ulaştıysa VEYA çarpışıp hızı 0'a düştüyse bir sonrakine atla
            if dist_to_node < agent.reach_radius
                || (v.linear.length() < 0.2 && dist_to_node < agent.reach_radius * 2.5)
            {
                agent.current_path_index += 1; // O(1) — remove(0) yerine indeks ilerlet
                if agent.current_path_index >= agent.path.len() {
                    // Yolu tamamladı
                    v.linear += steering::arrive(
                        t.position,
                        target_pos,
                        v.linear,
                        agent.max_speed,
                        agent.max_force,
                        agent.reach_radius * 2.0,
                    ) * dt;
                    continue;
                } else {
                    next_node = agent.path[agent.current_path_index];
                    next_node.y = t.position.y;
                }
            }

            // Eğer son node'a geldiysek Arrive, aksi halde Seek kullan
            let remaining_nodes = agent.path.len() - agent.current_path_index;
            let steering_force = if remaining_nodes == 1 {
                steering::arrive(
                    t.position,
                    next_node,
                    v.linear,
                    agent.max_speed,
                    agent.max_force,
                    agent.reach_radius * 2.0,
                )
            } else {
                steering::seek(
                    t.position,
                    next_node,
                    v.linear,
                    agent.max_speed,
                    agent.max_force,
                )
            };

            v.linear += steering_force * dt;
        } else {
            // Yol yoksa, en azından hedef doğrultusunda doğrudan gitmeyi dene
            let steering_force = steering::arrive(
                t.position,
                target_pos,
                v.linear,
                agent.max_speed,
                agent.max_force,
                agent.reach_radius * 2.0,
            );
            v.linear += steering_force * dt;
        }

        // Zıplamayı/uçmayı engelle (Sadece X Z düzleminde yürüsün istiyorsak, duruma göre özelleştirilebilir)
        // v.linear.y = 0.0;
    }
}
