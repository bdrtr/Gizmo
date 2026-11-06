use gizmo_physics_core::{BodyHandle, Collider, Transform};
use crate::components::{RigidBody, Velocity};use crate::world::PhysicsWorld;
use gizmo_core::entity::Entity;
use gizmo_core::query::Mut;
use gizmo_core::world::World;

/// Gives the body of entity `id` an instantaneous `impulse` (N·s, world space) at world `point`
/// — linear and angular, about its centre of mass — and wakes it. Returns whether the entity has
/// a body it could push: `false` for a missing entity, one without `RigidBody`/`Transform`/
/// `Velocity`, or a static or kinematic one, which an impulse does not move.
///
/// The ECS form of [`PhysicsWorld::apply_impulse`](crate::world::PhysicsWorld::apply_impulse),
/// for the thing a game does on a hit: `id` is what a raycast's
/// [`RaycastHit::entity`](gizmo_physics_core::raycast::RaycastHit) names, and the next physics
/// step carries the velocity it writes. Before this a game borrowed the three components itself,
/// woke the body by hand and reached into `Integrator` — which the facade does not even export
/// (`poligon`, 2026-11-02).
pub fn apply_impulse(world: &mut World, id: u32, impulse: gizmo_math::Vec3, point: gizmo_math::Vec3) -> bool {
    let Some(mut q) = world.query_mut::<(Mut<RigidBody>, &Transform, Mut<Velocity>)>() else { return false };
    let Some((mut rb, transform, mut vel)) = q.get_mut(id) else { return false };
    if !rb.is_dynamic() {
        return false;
    }
    rb.wake_up();
    crate::integrator::Integrator::apply_impulse_at_point(&rb, transform, &mut vel, impulse, point);
    true
}

/// Exclusive system that updates the entire physics simulation.
/// It reads all rigid and soft bodies from the ECS, steps the physics world,
/// and writes the transformed positions and velocities back to the ECS.
#[tracing::instrument(skip_all, name = "physics_step_system")]
pub fn physics_step_system(world: &World, dt: f32) {
    // Record profiler scope (if FrameProfiler resource is available)
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.begin_scope("physics_total");
    }

    // 1. Acquire PhysicsWorld Resource
    let mut physics_world = match world.try_get_resource_mut::<PhysicsWorld>() {
        Ok(res) => res,
        Err(e) => {
            // Physics cannot run without its world resource. Recoverable (skip the frame),
            // but a real per-frame functional failure → warn, not info.
            tracing::warn!(
                error = ?e,
                "[Physics] PhysicsWorld resource unavailable — skipping physics this frame"
            );
            return;
        }
    };

    // A body is synced only with a Collider: a `RigidBody` without one is not simulated at all —
    // no gravity, no velocity integration — and stands still without a word. A guard driven by
    // its `NavAgent`'s velocity did exactly that (`gizli`). Say so once, when the count grows.
    let orphans = world
        .query::<(&RigidBody, gizmo_core::query::Without<Collider>)>()
        .map_or(0, |q| q.iter().count());
    if orphans > physics_world.bodies_without_collider {
        tracing::warn!(
            orphans,
            "[Physics] {orphans} RigidBody entit{} without a Collider — not simulated, so they will not move; \
             add a Collider (one on a layer with mask 0 if it should touch nothing)",
            if orphans == 1 { "y" } else { "ies" }
        );
    }
    physics_world.bodies_without_collider = orphans;

    // 2. Gather Compound Shapes (Read Locks Only)
    let mut compound_shapes_map = std::collections::HashMap::new();
    {
        if let Some(query) = world.query::<(
            &Collider,
            &Transform,
            &RigidBody,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
            gizmo_core::query::Without<gizmo_core::component::IsDeleted>,
        )>() {
            let mut children_query = world.query::<&gizmo_core::component::Children>();
            let trans_query = world.query::<&Transform>();
            let col_query = world.query::<&Collider>();
            // An entity's `PhysicsMaterial` component overrides its collider's own material —
            // see `Collider::material_with`, which `gizmo-physics-dynamics` applies too.
            let material_query = world.query::<&gizmo_physics_core::components::PhysicsMaterial>();

            for (id, (col, transform, _rb, _, _)) in query.iter() {
                let mut compound_shapes = Vec::new();
                compound_shapes.push((
                    gizmo_physics_core::Transform::default(),
                    Box::new(col.shape.clone()),
                ));

                // Check children recursively.
                //
                // `visited` is the cycle guard, and this walk is inside the per-frame step: a
                // `Children` loop — which `add_component` can write directly and scene loading
                // writes verbatim out of a file — meant this step never returned, on the FIRST
                // tick after such a scene loaded. Unguarded until 2026-08-30.
                //
                // `stack` is not what grew: this walk pops before it pushes, so a simple cycle
                // holds one id in it forever. `compound_shapes` is the one that climbs, because
                // the loop body appends a sub-shape per child it visits — so the body being
                // assembled swells for as long as the step spins.
                //
                // The same `visited.insert` also keeps a DIAMOND from adding the same child
                // collider to the compound body twice, which is the quieter half: it terminates
                // on its own and just builds a body nobody authored.
                let mut visited = std::collections::HashSet::new();
                visited.insert(id);
                let mut stack = vec![id];
                while let Some(curr_id) = stack.pop() {
                    if let Some(children_query_ref) = &mut children_query {
                        if let Some(children) = children_query_ref.get(curr_id) {
                            for &child in &children.0 {
                                let child_id = child.id();
                                if !world.is_alive(child) || !visited.insert(child_id) {
                                    continue;
                                }
                                stack.push(child_id);
                                if let (Some(tq), Some(cq)) = (&trans_query, &col_query) {
                                    if let (Some(child_trans), Some(child_col)) = (tq.get(child_id), cq.get(child_id)) {
                                        let inv_rot = transform.rotation.inverse();
                                        let local_pos =
                                            inv_rot.mul_vec3(child_trans.position - transform.position);
                                        let local_rot = inv_rot * child_trans.rotation;

                                        let local_t = gizmo_physics_core::Transform::new(local_pos)
                                            .with_rotation(local_rot);
                                        compound_shapes
                                            .push((local_t, Box::new(child_col.shape.clone())));
                                    }
                                }
                            }
                        }
                    }
                }

                // One collider for this rigid body, rebuilt every frame from the entity's own
                // shape plus its children's — and it carries the AUTHORED collider forward, not
                // just its geometry.
                //
                // **This is the third field to go missing here, and the first two are why it is
                // written this way.** The line used to be `Collider::from_shape(shape)`, and
                // `from_shape` fills everything but the shape from `Default`. So a custom
                // `material` was silently discarded on every ECS entity — an elastic ball
                // (restitution 1) behaved as the default 0.3 — and that was fixed by appending
                // `.with_material(col.material)`, one field at a time. The two fields nobody
                // appended were `is_trigger` and `collision_layer`:
                //
                // - a collider with the inspector's "Trigger (Tetikleyici)" box ticked was rebuilt
                //   as a solid one, every frame. The badge under the checkbox promises "no
                //   physical response, only enter/exit events"; the player walked into the door
                //   sensor instead of through it, and because `pipeline.rs` decides between a
                //   contact manifold and a `TriggerEvent` on this same flag, **no ECS body could
                //   ever emit a trigger event at all** — which also left Lua's `physics.triggers`
                //   list structurally empty rather than merely wrong.
                // - `Collider::with_layer` was equally inert from the ECS: layer filtering is
                //   opt-in, and opting in did nothing.
                //
                // Cloning the authored collider and replacing only the shape ends the class:
                // a field added tomorrow travels without anyone remembering this line.
                let mut final_collider = if compound_shapes.is_empty() {
                    Collider::default() // Should technically not be simulated
                } else if compound_shapes.len() == 1 {
                    // Single collider, avoid nesting in Compound
                    let (_t, s) = compound_shapes.remove(0);
                    let mut rebuilt = col.clone();
                    rebuilt.shape = *s;
                    rebuilt
                } else {
                    let mut rebuilt = col.clone();
                    rebuilt.shape = gizmo_physics_core::ColliderShape::Compound(compound_shapes);
                    rebuilt
                };

                final_collider.material =
                    col.material_with(material_query.as_ref().and_then(|q| q.get(id)));
                compound_shapes_map.insert(id, final_collider);
            }
        }
    }

    // 3. Query Rigid Bodies (Write Locks)
    let mut rigid_bodies = Vec::new();
    // Kinematic bodies driven to a pose: their index in `rigid_bodies`, and the pose to put them
    // on after the step.
    let mut targets: Vec<(usize, crate::components::KinematicTarget)> = Vec::new();
    let target_query = world.query::<&crate::components::KinematicTarget>();
    // Forces that push on every step, not once a frame — see `ExternalForce`.
    let force_query = world.query::<&crate::components::ExternalForce>();
    // SAFETY: physics_step_system runs as a scheduled system; the scheduler guarantees
    // no other system mutably aliases these components while it runs (see `query_unchecked`).
    if let Some(mut query) = unsafe {
        world.query_unchecked::<(
            Mut<RigidBody>,
            Mut<Transform>,
            Mut<Velocity>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
            gizmo_core::query::Without<gizmo_core::component::IsDeleted>,
        )>()
    } {
        for (id, (rb, transform, vel, _, _)) in query.iter_mut() {
            if let Some(final_collider) = compound_shapes_map.remove(&id) {
                // A moving body whose inertia was never set gets its collider's.
                //
                // `RigidBody::new` cannot know the shape, so it stores a 1×1×1-box placeholder,
                // and the rule was "call `update_inertia_from_collider` yourself" — which 28 of
                // the 36 `RigidBody::new` calls in `demo/` did not. Measured in `lobut`: a 20 cm
                // ball, I = 0.4·m·r² = 0.096, spun as if I were ~1 — ten times too hard to turn,
                // so it never stopped sliding and lost 80 % of its speed to friction in 1.5 s
                // instead of settling into a roll at 5/7 of it. The derived value is written back
                // with the body, so this runs once; an inertia set by hand is anything but the
                // placeholder and is left alone.
                let mut rb = *rb;
                if !rb.is_static() && rb.local_inertia == RigidBody::UNSET_INERTIA {
                    rb.update_inertia_from_collider(&final_collider);
                }
                if let Some(push) = force_query.as_ref().and_then(|q| q.get(id)) {
                    if rb.is_dynamic() {
                        rb.force_accumulator += push.force;
                        rb.torque_accumulator += push.torque;
                    }
                }
                // A kinematic body with a target moves by whatever velocity reaches it this step.
                let mut vel = *vel;
                if let Some(target) = rb
                    .is_kinematic()
                    .then(|| target_query.as_ref().and_then(|q| q.get(id).copied()))
                    .flatten()
                {
                    let com = transform.rotation * rb.center_of_mass;
                    let (linear, angular) = target.velocity_from(transform.position, transform.rotation, com, dt);
                    vel.linear = linear;
                    vel.angular = angular;
                    targets.push((rigid_bodies.len(), target));
                }
                // Bridge: ECS entity id -> opaque physics BodyHandle (generation is
                // unused by physics; bodies are keyed by id only).
                rigid_bodies.push((BodyHandle::from_id(id), rb, *transform, vel, final_collider));
            }
        }
    } else {
        // The write query failed to build — no bodies get stepped this frame. Recoverable
        // but a genuine failure (physics silently freezes), so warn rather than info.
        tracing::warn!(
            "[Physics] could not mutably borrow RigidBody/Transform/Velocity — no bodies stepped this frame"
        );
    }

    // 3b. Bodies without a `Velocity`.
    //
    // The query above needs one, and a body without it used to be left out of the simulation
    // **silently**: a static floor written as `RigidBody::new_static()` + a collider — no
    // velocity, because a floor does not move — was not there, and everything above it fell
    // through (`lobut`, 2026-11-02, measured: ten pins and a ball in free fall from frame one).
    // A static body's velocity is zero by definition, so it joins this step with one. A body
    // that moves needs a velocity that persists between steps, which only the component can
    // hold, so it is given one through the command queue and simulated from the next frame on.
    // Scenes whose bodies all carry a `Velocity` — every one written before this — take neither
    // branch, so their bodies, their order and their hashes are unchanged.
    if let Some(query) = world.query::<(
        &RigidBody,
        &Transform,
        gizmo_core::query::Without<Velocity>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        gizmo_core::query::Without<gizmo_core::component::IsDeleted>,
    )>() {
        for (id, (rb, transform, _, _, _)) in query.iter() {
            if rb.is_static() {
                if let Some(final_collider) = compound_shapes_map.remove(&id) {
                    rigid_bodies.push((BodyHandle::from_id(id), *rb, *transform, Velocity::default(), final_collider));
                }
            } else if let Ok(queue) = world.try_get_resource::<gizmo_core::commands::CommandQueue>() {
                tracing::debug!(entity = id, "[Physics] a moving RigidBody has no Velocity — adding one; it is simulated from the next frame");
                queue.push(move |w: &mut World| {
                    if w.borrow::<Velocity>().get(id).is_none() {
                        if let Some(e) = w.get_entity(id) {
                            w.add_component(e, Velocity::default());
                        }
                    }
                });
            } else {
                tracing::warn!(entity = id, "[Physics] a moving RigidBody has no Velocity and there is no CommandQueue to add one — it is not simulated");
            }
        }
    }

    // 4. Step Simulation
    // An id whose entity was despawned and handed to a new one since the last step is a new
    // body: drop the old row (and its joints) so the sync adds it fresh. Measured before this: a
    // body spawned 15 m away on a dead body's id was pulled 5 m by the dead body's rope in a
    // second (`tests/joint_recycled_id.rs`).
    for (handle, ..) in &rigid_bodies {
        if let Some(entity) = world.get_entity(handle.id()) {
            physics_world.note_ecs_generation(handle.id(), entity.generation());
        }
    }
    physics_world.sync_bodies(rigid_bodies.iter());

    // A numerical error (NaN/Inf/Overflow) in physics may originate from
    // user-controlled body state; panicking would crash the whole engine.
    // Instead, log and skip this frame gracefully (signature unchanged).
    if let Err(e) = physics_world.step(dt) {
        tracing::error!(error = ?e, "Physics step failed (NaN/Inf/Overflow), skipping frame");
        // Make sure the profiler scope opened above is closed before returning.
        drop(physics_world); // release PhysicsWorld lock
        if let Ok(mut profiler) =
            world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>()
        {
            profiler.end_scope("physics_total");
        }
        return;
    }

    // Sync back to rigid_bodies so vehicles/ECS writeback works.
    //
    // Through an index built once, not a linear `find` per body. The scan was O(N²) — at a few
    // thousand bodies that is millions of comparisons every frame, spent entirely on the bridge
    // rather than on simulating anything. Entity ids are unique, so the map finds exactly what
    // the scan's first match found.
    let ecs_index: rustc_hash::FxHashMap<u32, usize> = rigid_bodies
        .iter()
        .enumerate()
        .map(|(idx, (handle, ..))| (handle.id(), idx))
        .collect();
    for i in 0..physics_world.entities.len() {
        if let Some(&idx) = ecs_index.get(&physics_world.entities[i].id()) {
            let (_, rb, trans, vel, _) = &mut rigid_bodies[idx];
            *rb = physics_world.rigid_bodies[i];
            *trans = physics_world.transforms[i];
            *vel = physics_world.velocities[i];
        }
    }
    // Driven bodies land on their targets exactly: the integrator's average of two velocities
    // would otherwise leave them a little off, and a little off every step is drift.
    for (idx, target) in targets {
        let (_, _, trans, _, _) = &mut rigid_bodies[idx];
        trans.position = target.position;
        trans.rotation = target.rotation;
    }

    // 5. Write back to ECS (Rigid Bodies)
    if !rigid_bodies.is_empty() {
        // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
        if let Some(mut query) = unsafe {
            world.query_unchecked::<(
                Mut<RigidBody>,
                Mut<Transform>,
                Mut<Velocity>,
                gizmo_core::query::Without<gizmo_core::pool::Pooled>,
            )>()
        } {
            for (entity, rb, transform, vel, _collider) in rigid_bodies {
                if let Some((mut ecs_rb, mut ecs_trans, mut ecs_vel, _)) = query.get_mut(entity.id()) {
                    *ecs_rb = rb;
                    *ecs_trans = transform;
                    *ecs_vel = vel;
                }
            }
        }
    }

    

    // 7. Dispatch Events
    if let Ok(mut trigger_queue) =
        world.try_get_resource_mut::<gizmo_core::event::Events<gizmo_physics_core::TriggerEvent>>()
    {
        for event in &physics_world.trigger_events {
            trigger_queue.send(event.clone());
        }
    }

    if let Ok(mut collision_queue) =
        world.try_get_resource_mut::<gizmo_core::event::Events<gizmo_physics_core::CollisionEvent>>()
    {
        for event in &physics_world.collision_events {
            collision_queue.send(event.clone());
        }
    }

    if physics_world.step_once {
        physics_world.step_once = false;
    }

    // Close profiler scope
    drop(physics_world); // PhysicsWorld lock'unu bırak
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.end_scope("physics_total");
    }
}

/// System that processes collision events and breaks objects that exceed their threshold.
pub fn physics_fracture_system(world: &World, dt: f32) {
    use crate::components::Breakable;
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let physics_world = match world.try_get_resource::<PhysicsWorld>() {
        Ok(res) => res,
        // No physics world ⇒ nothing to fracture. Optional-resource absence, not a bug → trace.
        Err(_) => {
            tracing::trace!("[Physics] fracture system: no PhysicsWorld resource, skipping");
            return;
        }
    };

    let mut commands = match Commands::fetch_stateless(world, dt) {
        Ok(c) => c,
        // Commands are the ECS deferred-op channel; without them every fracture spawn is
        // silently dropped — a hidden failure worth surfacing (debug: infra-level).
        Err(e) => {
            tracing::debug!(error = ?e, "[Physics] fracture system: Commands unavailable, fractures dropped this frame");
            return;
        }
    };

    age_debris(world, &mut commands, dt);

    let mut shattered = std::collections::HashSet::new();

    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    let query_opt = unsafe {
        world.query_unchecked::<(
            gizmo_core::query::Mut<Breakable>,
            &Transform,
            &Collider,
            &Velocity,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>()
    };
    let mut query = match query_opt {
        Some(q) => q,
        None => {
            tracing::trace!("[Physics] fracture system: breakable query unavailable, skipping");
            return;
        }
    };

    for event in &physics_world.collision_events {
        let mut max_impulse = 0.0;
        let mut impact_normal = gizmo_math::Vec3::ZERO;
        let mut impact_point = gizmo_math::Vec3::ZERO;

        for contact in &event.contact_points {
            if contact.normal_impulse > max_impulse {
                max_impulse = contact.normal_impulse;
                impact_normal = contact.normal;
                impact_point = contact.point;
            }
        }

        // Fallback: estimate impact from relative velocity when solver impulse is unavailable
        if max_impulse <= 0.0 && !event.contact_points.is_empty() {
            // Look up velocities of both entities to estimate impact force
            let vel_a = physics_world
                .entity_index_map
                .get(&event.entity_a.id())
                .map(|&idx| physics_world.velocities[idx].linear)
                .unwrap_or(gizmo_math::Vec3::ZERO);
            let vel_b = physics_world
                .entity_index_map
                .get(&event.entity_b.id())
                .map(|&idx| physics_world.velocities[idx].linear)
                .unwrap_or(gizmo_math::Vec3::ZERO);
            let mass_a = physics_world
                .entity_index_map
                .get(&event.entity_a.id())
                .map(|&idx| physics_world.rigid_bodies[idx].mass)
                .unwrap_or(1.0);
            let mass_b = physics_world
                .entity_index_map
                .get(&event.entity_b.id())
                .map(|&idx| physics_world.rigid_bodies[idx].mass)
                .unwrap_or(1.0);

            let rel_speed = (vel_b - vel_a).length();
            let reduced_mass = if mass_a > 0.0 && mass_b > 0.0 {
                (mass_a * mass_b) / (mass_a + mass_b)
            } else {
                mass_a.max(mass_b)
            };
            max_impulse = rel_speed * reduced_mass;
            if let Some(contact) = event.contact_points.first() {
                impact_normal = contact.normal;
                impact_point = contact.point;
            }
        }

        if max_impulse <= 0.0 {
            continue;
        }

        // Check Entity A
        if !shattered.contains(&event.entity_a.id()) {
            // `BodyHandle` is a bare id, so the ECS handle must be RESOLVED, not fabricated.
            // `World::entity` returns the id's CURRENT generation and `None` for an id that is
            // dead — which is exactly the check the comment below claims. Building
            // `Entity::new(id, 0)` here instead (until 2026-08-25) made that claim false for
            // every recycled slot: `get_mut_entity` rejected the stale handle, so a breakable
            // whose id had been reused once was silently immune to contact damage, and the
            // shatter below would have despawned through the same stale handle.
            //
            // Generation kontrolü, çarpışma olayı üretildikten sonra yeniden kullanılmış bir
            // slota yanlış yazmayı engeller.
            let resolved_a = world.entity(event.entity_a.id());
            if let Some((entity_a, (mut breakable, transform, collider, vel, _))) =
                resolved_a.and_then(|e| query.get_mut_entity(e).map(|q| (e, q)))
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        // Latch only if it really broke — see `shatter_entity`.
                        let broke = shatter_entity(
                            &mut commands,
                            entity_a,
                            &breakable,
                            transform,
                            collider,
                            vel,
                            -impact_normal,
                            impact_point,
                        );
                        if broke {
                            breakable.is_broken = true;
                            shattered.insert(event.entity_a.id());
                        }
                    }
                }
            }
        }

        // Check Entity B
        if !shattered.contains(&event.entity_b.id()) {
            // Resolved rather than fabricated — see the note on entity A above.
            let resolved_b = world.entity(event.entity_b.id());
            if let Some((entity_b, (mut breakable, transform, collider, vel, _))) =
                resolved_b.and_then(|e| query.get_mut_entity(e).map(|q| (e, q)))
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        let broke = shatter_entity(
                            &mut commands,
                            entity_b,
                            &breakable,
                            transform,
                            collider,
                            vel,
                            impact_normal,
                            impact_point,
                        );
                        if broke {
                            breakable.is_broken = true;
                            shattered.insert(event.entity_b.id());
                        }
                    }
                }
            }
        }
    }
    drop(query);
}

/// The local-space box the Voronoi cells are cut out of: `(center, half_extents)`.
///
/// `None` means the shape has no finite solid to shatter, and the caller must leave the body
/// alone entirely — see [`shatter_entity`] for why that is not the same as "do nothing here".
///
/// Every bounded shape is shattered through its **local bounding box**, which is coarser than
/// its real silhouette: a sphere breaks like the cube around it. That matches what the debris
/// already is — [`shatter_entity`] approximates each Voronoi cell with a sphere of matching
/// volume regardless of the cell's actual geometry — so the bound is not the weak link.
///
/// `Box` keeps its own arm rather than going through the AABB. Numerically the two agree
/// (an identity rotation folds min/max over ±components and gives back the half-extents
/// exactly), but this is the only pre-existing shatter path and it must stay bit-identical,
/// which is easier to state than to re-derive.
fn shatter_bounds(collider: &Collider) -> Option<(gizmo_math::Vec3, gizmo_math::Vec3)> {
    use gizmo_physics_core::ColliderShape;

    match &collider.shape {
        ColliderShape::Box(b) => Some((gizmo_math::Vec3::ZERO, b.half_extents)),

        // A `Plane` is a half-space: `compute_aabb` hands back a ±10 km cube for it, so going
        // through the generic arm would "shatter" the floor into multi-kilometre boulders.
        // A `TriMesh` is the concave, static variant — no inertia tensor, and the convex debris
        // this path spawns cannot represent it.
        ColliderShape::Plane(_) | ColliderShape::TriMesh(_) => None,

        // Sphere / Capsule / ConvexHull / Compound. The AABB is taken at the origin with no
        // rotation, i.e. in the collider's own frame; the caller re-applies the body transform.
        _ => {
            let aabb = collider.compute_aabb(gizmo_math::Vec3::ZERO, gizmo_math::Quat::IDENTITY);
            // An empty hull or compound reports the inverted `(+inf, -inf)` box (documented on
            // `compute_aabb`), which would reach `voronoi_shatter` as a NaN extent.
            if aabb.is_empty() {
                return None;
            }
            // `Aabb` carries `Vec3A`; the rest of this path is plain `Vec3`.
            let half_extents = ((aabb.max - aabb.min) * 0.5).into();
            Some((((aabb.min + aabb.max) * 0.5).into(), half_extents))
        }
    }
}

/// Domain separator for [`shatter_seed`], so this path's RNG stream cannot line up with any
/// other stream in the crate that happens to be keyed off the same entity id. The bytes spell
/// `SHATTER\0`; the value carries no meaning beyond being fixed and non-zero.
const SHATTER_SEED_DOMAIN: u64 = 0x5348_4154_5445_5200;

/// The Voronoi seed a shattering entity gets: a pure function of its **ECS id**.
///
/// # Why the id, and only the id
///
/// This used to be a literal `42`, so every object in the game broke into the exact same
/// debris pattern — a quality bug, not a determinism one. The seed has to distinguish
/// *entities* from each other while staying reproducible for a given entity, and the id is
/// the only thing in scope that does both.
///
/// A frame/tick counter was the obvious second ingredient and is deliberately **not** used.
/// There is no rollback-safe one to use:
///
/// * [`PhysicsWorld`] has no step counter at all — its
///   [`WorldSnapshot`](crate::world::WorldSnapshot) restores transforms, velocities, bodies,
///   the contact cache, the sub-step `accumulator`, zones, joints and weather, and nothing
///   that counts;
/// * `gizmo_net`'s ECS rollback snapshot restores only per-entity position/rotation/velocity/
///   sleep, so anything else a re-simulated frame reads is whatever the *original* run left
///   behind, not what that frame saw the first time;
/// * `gizmo_core::time::Time::frame_count` is neither restored by either of those nor tied to
///   the fixed step — it counts render frames fed from the wall clock, so the same sim tick
///   lands on a different count at a different frame rate.
///
/// Mixing any of those in would make the debris of a rolled-back-and-resimulated break differ
/// from the debris of the original break: a silent replay desync, which is worse than the
/// uniform pattern being fixed here. And nothing is lost by leaving them out, because a
/// breakable can only shatter **once** —
/// [`Breakable::is_broken`](crate::components::Breakable::is_broken) latches on the first
/// successful shatter and nothing in the engine ever clears it — so the seed never has to tell
/// two *occasions* apart, only two *entities*.
///
/// The entity **generation** is left out for the same reason: it is allocator state, not
/// simulation state, and no snapshot restores it. (It is not even in scope — two of the three
/// call sites hand this function an `Entity::new(id, 0)` built from a collision event.) The
/// price is that a breakable spawned into a recycled id slot repeats the debris of whatever
/// occupied that slot before it, which is a repeat nobody can observe side by side.
///
/// # Why it is mixed rather than passed straight through
///
/// Today's `StdRng` is ChaCha12 and `SeedableRng::seed_from_u64` already expands the `u64`
/// through PCG32 before seeding it, so consecutive ids would in fact produce independent
/// streams as-is. That is a property of the generator `rand` currently ships, though, and
/// `rand` documents `StdRng` as replaceable at will; `voronoi_shatter`'s `seed` is public API
/// besides. A SplitMix64 finalizer costs three multiplies and makes `id -> seed` avalanche on
/// its own, so the debris variety does not quietly depend on which PRNG is underneath.
fn shatter_seed(entity_id: u32) -> u64 {
    // SplitMix64's finalizer (the `fmix64`-style avalanche), applied to the domain-shifted id.
    let mut z = (entity_id as u64).wrapping_add(SHATTER_SEED_DOMAIN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Despawns `entity` and spawns its debris, returning whether it actually shattered.
///
/// **The return value is load-bearing.** All three call sites used to latch
/// [`Breakable::is_broken`](crate::components::Breakable::is_broken) *before* calling this, and
/// nothing in the engine ever clears that flag again — so a bail-out here left the body in the
/// scene at zero health, undamageable (every damage path is gated on `!is_broken`) and
/// unbreakable. Not "unsupported shape does nothing": unsupported shape *permanently disabled the
/// entity*. Callers must therefore latch only on `true`, which is why they now latch afterwards.
fn shatter_entity(
    commands: &mut gizmo_core::commands::Commands,
    entity: Entity,
    breakable: &crate::components::Breakable,
    transform: &Transform,
    collider: &Collider,
    vel: &Velocity,
    impact_direction: gizmo_math::Vec3,
    _impact_point: gizmo_math::Vec3,
) -> bool {
    use crate::fracture::voronoi_shatter;

    let Some((center, extents)) = shatter_bounds(collider) else {
        return false;
    };

    // Despawn the original entity
    commands.entity(entity).despawn();

    // Generate chunks. The seed is derived from the entity id rather than being a constant, so
    // two objects breaking under identical conditions produce different debris while a given
    // object still breaks the same way on every replay — see `shatter_seed`.
    let chunks = voronoi_shatter(extents, breakable.max_pieces, shatter_seed(entity.id()));

    for chunk in chunks {
        // Create new convex hull colliders or approximated boxes for the chunks.
        // For simplicity, we approximate each chunk with a small sphere or box based on its volume.
        // A full implementation would use ConvexHull shapes.
        let radius = (chunk.volume * 0.1).powf(1.0 / 3.0).max(0.1);

        // Offset chunk center by parent's transform. `center` is where the shatter box sits in
        // the collider's own frame — zero for every shape whose geometry is built about the
        // origin, non-zero only for an off-centre hull or compound.
        let world_offset = transform.rotation * (center + chunk.center_of_mass);
        let mut new_transform = *transform;
        new_transform.position += world_offset;
        // The piece's geometry is in metres; a scale copied from the parent (which games use
        // to size its mesh) would size it a second time when it is drawn.
        new_transform.scale = gizmo_math::Vec3::ONE;

        // Give chunks a slight explosive velocity outwards from the center of mass
        let mut new_vel = *vel;
        let outward = chunk.center_of_mass.normalize_or_zero();
        new_vel.linear += outward * 2.0 + impact_direction * 5.0; // Explosion effect

        let chunk_collider = Collider::sphere(radius).with_material(collider.material);
        let mut rb = RigidBody::new(chunk.volume * collider.material.density, true);
        rb.update_inertia_from_collider(&chunk_collider);

        let local = center_of_mass_frame(&chunk);
        commands
            .spawn()
            .insert(rb)
            .insert(chunk_collider)
            .insert(new_transform)
            .insert(new_vel)
            .insert(crate::components::Debris {
                source: entity.id(),
                vertices: local,
                normals: chunk.normals,
                indices: chunk.indices,
                age: 0.0,
                lifetime: breakable.debris_lifetime,
            });
    }

    true
}

/// A chunk's vertices moved into its own frame: about its centre of mass, where its body sits.
fn center_of_mass_frame(chunk: &crate::fracture::ProceduralChunk) -> Vec<gizmo_math::Vec3> {
    chunk.vertices.iter().map(|v| *v - chunk.center_of_mass).collect()
}

/// Ages every [`Debris`](crate::components::Debris) piece by `dt` and despawns the ones past
/// their lifetime.
fn age_debris(world: &World, commands: &mut gizmo_core::commands::Commands, dt: f32) {
    // SAFETY: called from `physics_fracture_system` before its breakable query is opened, and
    // `Debris` is in no other query of that system.
    let Some(mut pieces) = (unsafe { world.query_unchecked::<gizmo_core::query::Mut<crate::components::Debris>>() }) else {
        return;
    };
    for (id, mut piece) in pieces.iter_mut() {
        piece.age += dt;
        if piece.lifetime > 0.0 && piece.age >= piece.lifetime {
            if let Some(e) = world.entity(id) {
                commands.entity(e).despawn();
            }
        }
    }
}

/// System that checks for Explosion components and applies outward forces
/// to all rigid bodies and soft body nodes within the radius.
pub fn physics_explosion_system(world: &World, dt: f32) {
    use crate::components::{Explosion, ExplosionFalloff};
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let mut commands = match Commands::fetch_stateless(world, dt) {
        Ok(c) => c,
        // Without Commands the explosion despawns/impulses can't be issued — surface it.
        Err(e) => {
            tracing::debug!(error = ?e, "[Physics] explosion system: Commands unavailable, skipping");
            return;
        }
    };

    let explosion_query_opt = world.query::<(
        &Explosion,
        &Transform,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>();
    let mut active_explosions = Vec::new();

    if let Some(exp_query) = &explosion_query_opt {
        for (ent_id, (explosion, transform, _)) in exp_query.iter() {
            if explosion.is_active {
                // Resolved, not fabricated: this handle is what despawns the explosion at the
                // end of the pass. A generation-0 handle to a recycled slot fails
                // `World::despawn`'s liveness check SILENTLY, leaving `is_active` set — so the
                // blast re-detonates on every subsequent frame, against `Explosion`'s own
                // documented contract that it applies exactly once and then despawns.
                let exp_entity = match world.entity(ent_id) {
                    Some(e) => e,
                    None => continue,
                };
                // Apply offset to transform position
                active_explosions.push((
                    exp_entity,
                    *explosion,
                    transform.position + explosion.offset,
                ));
            }
        }
    }

    if active_explosions.is_empty() {
        return; // Nothing to explode
    }

    let mut shattered = std::collections::HashSet::new();

    // Helper closure to calculate falloff intensity
    let calculate_intensity = |dist: f32, radius: f32, falloff: ExplosionFalloff| -> f32 {
        if dist >= radius {
            return 0.0;
        }
        match falloff {
            ExplosionFalloff::None => 1.0,
            ExplosionFalloff::Linear => 1.0 - (dist / radius),
            ExplosionFalloff::Quadratic => {
                let ratio = 1.0 - (dist / radius);
                ratio * ratio
            }
        }
    };

    // Check for Breakables that should shatter
    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    let mut breakable_query_opt = unsafe {
        world.query_unchecked::<(
            gizmo_core::query::Mut<crate::components::Breakable>,
            &Transform,
            &Collider,
            &Velocity,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>()
    };
    if let Some(breakable_query) = &mut breakable_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (mut breakable, transform, collider, vel, _)) in breakable_query.iter_mut() {
                if breakable.is_broken || shattered.contains(&id) {
                    continue;
                }

                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();

                if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let intensity =
                        calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                    let impulse_mag = explosion.force * intensity;

                    if impulse_mag > breakable.threshold {
                        breakable.current_health -= explosion.damage * intensity;
                        if breakable.current_health <= 0.0 {
                            let dir = diff / dist;
                            let mut exp_vel = *vel;
                            exp_vel.linear += dir * impulse_mag * 0.1; // Estimate mass
                            // Resolved, not fabricated. A generation-0 handle to a recycled
                            // slot makes `shatter_entity`'s despawn a silent no-op while the
                            // debris still spawns and `is_broken` latches below — leaving the
                            // original in the scene beside its own debris and, since every
                            // damage path is gated on `!is_broken` and nothing clears it,
                            // permanently undamageable.
                            let entity = match world.entity(id) {
                                Some(e) => e,
                                None => continue,
                            };
                            let broke = shatter_entity(
                                &mut commands,
                                entity,
                                &breakable,
                                transform,
                                collider,
                                &exp_vel,
                                dir,
                                transform.position,
                            );
                            if broke {
                                breakable.is_broken = true;
                                shattered.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    // Apply to Rigid Bodies
    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    let mut rb_query_opt = unsafe {
        world.query_unchecked::<(
            Mut<RigidBody>,
            &Transform,
            Mut<Velocity>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>()
    };
    if let Some(rb_query) = &mut rb_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (mut rb, transform, mut vel, _)) in rb_query.iter_mut() {
                if !rb.is_dynamic() || shattered.contains(&id) {
                    continue;
                }

                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();

                // The radius test gates the WAKE below as well as the impulse — it must stay
                // ahead of it, since waking a body the blast never reached would be its own bug.
                if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let dir = diff / dist;

                    let intensity =
                        calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                    let impulse_mag = explosion.force * intensity;

                    // Apply instantaneous velocity change
                    let delta_v = dir * impulse_mag * rb.inv_mass();
                    vel.linear += delta_v;

                    // …and WAKE, or that write is swallowed whole. A sleeping body is skipped
                    // by both integration stages (`Integrator::integrate_velocities` and
                    // `integrate_positions` return early on `is_sleeping`, and `pipeline.rs`
                    // skips it again before calling either), and an island whose members are
                    // all asleep is not even solved (`island_active`, pipeline.rs) — so the
                    // velocity would sit in the component, unspent, and move nothing. The
                    // visible symptom was a settled stack beside a blast that simply did not
                    // react: it looked like the explosion had not happened. This is the same
                    // contract the world's own impulse helpers keep — see
                    // `world/query.rs::apply_impulse`, which takes `&mut RigidBody` for
                    // exactly this reason.
                    //
                    // Scoped deliberately to bodies the blast actually MOVES. Out-of-range
                    // bodies never reach here (radius test above), and a zero `delta_v` — a
                    // dynamic body with `mass == 0`, or one sitting where the falloff has
                    // decayed to nothing — is not moved, so waking it would only spend
                    // simulation on a body the blast did not touch. Sleeping neighbours the
                    // blast did not reach are woken afterwards by the contact path instead,
                    // island by island, once a woken body starts moving (`pipeline.rs`,
                    // `island_has_mover` → `wake_updates`).
                    if delta_v != gizmo_math::Vec3::ZERO {
                        rb.wake_up();
                    }
                }
            }
        }
    }

    // Despawn the explosions so they don't trigger again
    // Note: If game logic needs to read explosion damage, it must run BEFORE the physics_explosion_system in the schedule!
    for (exp_entity, _, _) in active_explosions {
        commands.entity(exp_entity).despawn();
    }
}



#[cfg(test)]
mod apply_impulse_tests {
    use super::*;
    use gizmo_math::Vec3;

    fn body(world: &mut World, rb: RigidBody) -> u32 {
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, rb);
        world.add_component(e, Velocity::default());
        world.add_component(e, Collider::sphere(0.5));
        e.id()
    }

    /// Through the centre: pure linear, `impulse / mass`. Off the centre: it spins too. A
    /// sleeping body is woken. A static body and a missing entity are refused.
    #[test]
    fn an_impulse_moves_a_dynamic_body_and_nothing_else() {
        let mut world = World::new();
        let mut rb = RigidBody::new(2.0, true);
        rb.update_inertia_from_collider(&Collider::sphere(0.5));
        rb.is_sleeping = true;
        let ball = body(&mut world, rb);
        assert!(apply_impulse(&mut world, ball, Vec3::new(4.0, 0.0, 0.0), Vec3::ZERO));
        let v = *world.borrow::<Velocity>().get(ball).unwrap();
        assert!((v.linear - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5, "{v:?}");
        assert!(v.angular.length() < 1e-6, "through the centre must not spin it");
        assert!(!world.borrow::<RigidBody>().get(ball).unwrap().is_sleeping, "still asleep");

        assert!(apply_impulse(&mut world, ball, Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 0.5, 0.0)));
        assert!(world.borrow::<Velocity>().get(ball).unwrap().angular.length() > 0.1, "off-centre must spin it");

        let wall = body(&mut world, RigidBody::new_static());
        assert!(!apply_impulse(&mut world, wall, Vec3::X, Vec3::ZERO));
        assert_eq!(world.borrow::<Velocity>().get(wall).unwrap().linear, Vec3::ZERO);
        assert!(!apply_impulse(&mut world, 9999, Vec3::X, Vec3::ZERO));
    }
}

#[cfg(test)]
mod collider_less_body_tests {
    use super::*;
    use gizmo_core::world::World;
    use gizmo_math::Vec3;

    /// A `RigidBody` without a `Collider` is not simulated — it does not fall — and the sync
    /// counts it so it can warn (once, when the count grows) instead of leaving it standing still
    /// without a word. With a collider the same body falls.
    #[test]
    fn a_body_without_a_collider_is_counted_and_does_not_move() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
        let bare = world.spawn();
        world.add_component(bare, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
        world.add_component(bare, RigidBody::new(1.0, true));
        world.add_component(bare, Velocity::default());
        let full = world.spawn();
        world.add_component(full, Transform::new(Vec3::new(3.0, 5.0, 0.0)));
        world.add_component(full, RigidBody::new(1.0, true));
        world.add_component(full, Velocity::default());
        world.add_component(full, Collider::sphere(0.5));
        for _ in 0..30 {
            physics_step_system(&world, 1.0 / 60.0);
        }
        assert_eq!(world.get_resource::<PhysicsWorld>().unwrap().bodies_without_collider, 1);
        let y = |e: gizmo_core::entity::Entity| world.borrow::<Transform>().get(e.id()).unwrap().position.y;
        assert_eq!(y(bare), 5.0, "not simulated");
        assert!(y(full) < 4.0, "the same body with a collider falls");
    }
}
