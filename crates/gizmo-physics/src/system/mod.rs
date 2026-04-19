pub mod types;
pub mod union_find;
pub mod broad_phase;
pub mod narrow_phase;
pub mod ccd;
pub mod solver;

pub use types::*;
pub use broad_phase::broad_phase;
pub use narrow_phase::{detect_collisions, detect_single_collision_pair, detect_pair};
pub use ccd::ccd_bisect;
pub use solver::{build_islands, solve_islands, solve_single_island, write_back};

use crate::components::{RigidBody, Transform, Velocity};
use crate::shape::Collider;
use crate::vehicle::VehicleController;
use gizmo_core::World;
use std::collections::HashMap;

pub fn physics_collision_system(world: &mut World, dt: f32) {
    let mut entities_to_wake: Vec<u32> = Vec::new();
    let mut collision_events: Vec<crate::CollisionEvent> = Vec::new();

    let parallel_physics = !world
        .get_resource::<crate::components::PhysicsConfig>()
        .map(|c| c.deterministic_simulation)
        .unwrap_or(false);

    {
        // Borrow scope — tüm ECS borrow'ları burada yaşar
        let mut transforms  = match world.borrow_mut::<Transform>()  { Some(t) => t, None => return };
        let mut velocities  = match world.borrow_mut::<Velocity>()   { Some(v) => v, None => return };
        let colliders       = match world.borrow::<Collider>()        { Some(c) => c, None => return };
        let rigidbodies     = match world.borrow::<RigidBody>()       { Some(r) => r, None => return };
        let vehicles        = world.borrow::<VehicleController>();
        let joint_world     = world.get_resource::<crate::constraints::JointWorld>();

        let vehicle_entities: std::collections::HashSet<u32> = match &vehicles {
            Some(v) => v.dense.iter().map(|e| e.entity).collect(),
            None    => std::collections::HashSet::new(),
        };
        let has_vehicles = vehicles.is_some();

        // 1. Broad-phase — olası çarpışma çiftleri
        let collision_pairs = broad_phase(&transforms, &colliders, &rigidbodies, &velocities, dt);
        let has_joints = joint_world.is_some() && !joint_world.as_ref().unwrap().joints.is_empty();
        if collision_pairs.is_empty() && !has_joints {
            return;
        }

        // 2. Narrow-phase — gerçek temas tespiti (isteğe bağlı Rayon)
        let detection_results = detect_collisions(
            &collision_pairs,
            &transforms,
            &colliders,
            &rigidbodies,
            &velocities,
            &vehicle_entities,
            has_vehicles,
            dt,
            parallel_physics,
        );

        // 3. Island generation — Union-Find ile gruplama
        let mut islands = build_islands(detection_results, &transforms, &velocities, &mut entities_to_wake, &rigidbodies, joint_world.as_deref());

        // 4. Çözücü — warm-start + SI + position projection (paralel island başına)
        let (solver_iters, frame_count) =
            if let Some(state) = world.get_resource_mut::<PhysicsSolverState>() {
                (state.solver_iterations, state.frame_counter)
            } else {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[Physics WARN] PhysicsSolverState bulunamadı. \
                     Warm-start devre dışı. world.insert_resource(PhysicsSolverState::new()) ekleyin."
                );
                (8, 0)
            };

        let contact_cache = if let Some(state) = world.get_resource_mut::<PhysicsSolverState>() {
            state.contact_cache.clone()
        } else {
            HashMap::new()
        };

        solve_islands(
            &mut islands,
            &contact_cache,
            solver_iters,
            frame_count,
            dt,
            parallel_physics,
        );

        // PhysicsConfig'den limitleri oku
        let (max_contacts_per_pair, event_throttle_frames) =
            world.get_resource::<crate::components::PhysicsConfig>()
                .map(|cfg| (cfg.max_contact_points_per_pair, cfg.collision_event_throttle_frames))
                .unwrap_or((4, 4));

        // 5. Write-back — ECS + cache + event
        if let Some(mut state) = world.get_resource_mut::<PhysicsSolverState>() {
            state.frame_counter += 1;
            write_back(
                islands,
                &mut transforms,
                &mut velocities,
                &vehicle_entities,
                &mut state,
                &mut collision_events,
                max_contacts_per_pair,
                event_throttle_frames,
            );
        } else {
            let mut dummy_state = PhysicsSolverState::new();
            write_back(
                islands,
                &mut transforms,
                &mut velocities,
                &vehicle_entities,
                &mut dummy_state,
                &mut collision_events,
                max_contacts_per_pair,
                event_throttle_frames,
            );
        }
    } // Borrow scope sonu

    // Event kuyruğuna yaz
    if !collision_events.is_empty() {
        let mut evs = world.get_resource_mut_or_default::<gizmo_core::event::Events<crate::CollisionEvent>>();
        for ev in collision_events {
            evs.push(ev);
        }
    }

    // Uyuyan nesneleri uyandır
    if !entities_to_wake.is_empty() {
        if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
            for e in entities_to_wake {
                if let Some(rb) = rbs.get_mut(e) {
                    rb.wake_up();
                }
            }
        }
    }
}
