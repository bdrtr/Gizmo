use gizmo_physics_core::{BodyHandle, Collider, Transform};
use crate::components::{RigidBody, Velocity};use crate::world::PhysicsWorld;
use gizmo_core::entity::Entity;
use gizmo_core::query::Mut;
use gizmo_core::world::World;

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

            for (id, (col, transform, _rb, _, _)) in query.iter() {
                let mut compound_shapes = Vec::new();
                compound_shapes.push((
                    gizmo_physics_core::Transform::default(),
                    Box::new(col.shape.clone()),
                ));

                // Check children recursively
                let mut stack = vec![id];
                while let Some(curr_id) = stack.pop() {
                    if let Some(children_query_ref) = &mut children_query {
                        if let Some(children) = children_query_ref.get(curr_id) {
                            for &child_id in &children.0 {
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
                let final_collider = if compound_shapes.is_empty() {
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

                compound_shapes_map.insert(id, final_collider);
            }
        }
    }

    // 3. Query Rigid Bodies (Write Locks)
    let mut rigid_bodies = Vec::new();
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
                // Bridge: ECS entity id -> opaque physics BodyHandle (generation is
                // unused by physics; bodies are keyed by id only).
                rigid_bodies.push((BodyHandle::from_id(id), *rb, *transform, *vel, final_collider));
            }
        }
    } else {
        // The write query failed to build — no bodies get stepped this frame. Recoverable
        // but a genuine failure (physics silently freezes), so warn rather than info.
        tracing::warn!(
            "[Physics] could not mutably borrow RigidBody/Transform/Velocity — no bodies stepped this frame"
        );
    }

    // 4. Step Simulation
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

        // Give chunks a slight explosive velocity outwards from the center of mass
        let mut new_vel = *vel;
        let outward = chunk.center_of_mass.normalize_or_zero();
        new_vel.linear += outward * 2.0 + impact_direction * 5.0; // Explosion effect

        let chunk_collider = Collider::sphere(radius).with_material(collider.material);
        let mut rb = RigidBody::new(chunk.volume * collider.material.density, true);
        rb.update_inertia_from_collider(&chunk_collider);

        commands
            .spawn()
            .insert(rb)
            .insert(chunk_collider)
            .insert(new_transform)
            .insert(new_vel);
    }

    true
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


