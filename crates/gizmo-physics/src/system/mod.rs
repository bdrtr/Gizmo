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

pub fn physics_collision_system(world: &mut World, dt: f32) {
    // Warm-start cache'in her karede kaybolmaması (dummy_state sıfırlaması) için state'i garantiye al.
    {
        let _ = world.get_resource_mut_or_default::<PhysicsSolverState>();
    }

    let mut entities_to_wake: Vec<u32> = Vec::new();
    let mut collision_events: Vec<crate::CollisionEvent> = Vec::new();

    let (parallel_physics, max_contacts_per_pair, event_throttle_frames, ccd_velocity_threshold, solver_iterations) = {
        if let Some(cfg) = world.get_resource::<crate::components::PhysicsConfig>() {
            (!cfg.deterministic_simulation, cfg.max_contact_points_per_pair, cfg.collision_event_throttle_frames, cfg.ccd_velocity_threshold, cfg.solver_iterations)
        } else {
            (false, 4, 4, 0.1, 8)
        }
    };

    'physics: {
        // Borrow scope — tüm ECS borrow'ları burada yaşar
        let colliders = world.borrow::<Collider>();
        let rigidbodies = world.borrow::<RigidBody>();
        let joint_world = world.get_resource::<crate::constraints::JointWorld>();

        let vehicle_entities: std::collections::HashSet<u32> = {
            world.borrow::<VehicleController>().entities().collect()
        };

        // 1. Broad-phase, 2. Narrow-phase & 3. Island generation — SHARED BORROW
        let (_detection_results, islands) = {
            let transforms = world.borrow::<Transform>();
            let velocities = world.borrow::<Velocity>();

            let pairs = broad_phase(&transforms, &colliders, &rigidbodies, &velocities, dt, parallel_physics);
            
            let has_joints = joint_world.as_ref().map_or(false, |jw| !jw.joints.is_empty());
            if pairs.is_empty() && !has_joints {
                break 'physics;
            }

            let results = detect_collisions(
                &pairs,
                &transforms,
                &colliders,
                &rigidbodies,
                &velocities,
                dt,
                parallel_physics,
                ccd_velocity_threshold,
            );

            let islands = build_islands(results.clone(), &transforms, &velocities, &mut entities_to_wake, &rigidbodies, joint_world.as_deref());
            (results, islands)
        };

        // 4. Solver — MUTABLE BORROW
        let mut transforms = world.borrow_mut::<Transform>();
        let mut velocities = world.borrow_mut::<Velocity>();

        let mut islands = islands; // Move islands to outer scope

        // 4. Çözücü — warm-start + SI + position projection (paralel island başına)
        let solver_state = world.get_resource_mut::<PhysicsSolverState>();
        if let Some(mut state) = solver_state {
            solve_islands(
                &mut islands,
                &state.contact_cache,
                solver_iterations,
                state.frame_counter,
                dt,
                parallel_physics,
            );

            // 5. Write-back — ECS + cache + event
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
            #[cfg(debug_assertions)]
            eprintln!(
                "[Physics WARN] PhysicsSolverState bulunamadı. \
                 Warm-start devre dışı. world.insert_resource(PhysicsSolverState::new()) ekleyin."
            );
            
            let mut dummy_state = PhysicsSolverState::new();
            solve_islands(
                &mut islands,
                &dummy_state.contact_cache,
                solver_iterations, 
                0, // default frame_counter
                dt,
                parallel_physics,
            );
            
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
    // (ECS borrow çakışmalarını / aliasing'i önlemek için borrow scope dışında saklanır)
    if !collision_events.is_empty() {
        let mut evs = world.get_resource_mut_or_default::<gizmo_core::event::Events<crate::CollisionEvent>>();
        for ev in collision_events {
            evs.push(ev);
        }
    }

    // Uyuyan nesneleri uyandır
    // (RigidBody mut borrow çakışmasını önlemek için borrow scope dışında uygulanır)
    if !entities_to_wake.is_empty() {
        let mut rbs = world.borrow_mut::<RigidBody>();
        for e in entities_to_wake {
            if let Some(rb) = rbs.get_mut(e) {
                rb.wake_up();
            }
        }
    }
}
