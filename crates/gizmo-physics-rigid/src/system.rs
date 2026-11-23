use gizmo_physics_core::{BodyHandle, Collider, Transform};
use crate::components::{RigidBody, Velocity};use crate::world::PhysicsWorld;
use gizmo_core::entity::Entity;
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
pub fn apply_impulse(world: &mut World, id: Entity, impulse: gizmo_math::Vec3, point: gizmo_math::Vec3) -> bool {
    let mut q = world.query_mut::<(&mut RigidBody, &Transform, &mut Velocity)>();
    let Some((mut rb, transform, mut vel)) = q.get_mut(id) else { return false };
    if !rb.is_dynamic() {
        return false;
    }
    rb.wake_up();
    crate::integrator::Integrator::apply_impulse_at_point(&rb, transform, &mut vel, impulse, point);
    true
}

/// Puts the step's `value` in an ECS component, stamping it changed only when it differs.
///
/// The value is stored either way, bit for bit: `==` on floats calls `-0.0` and `0.0` equal, and
/// skipping the store on equality would leave the ECS holding a zero of the other sign than the
/// step produced — which the next step reads back. Every bit the world held before this was
/// marked conditionally is still the bit it holds (`headless_stress_test`'s hash is unchanged).
fn write_back<T: PartialEq>(slot: &mut gizmo_core::query::Mut<'_, T>, value: T) {
    if **slot != value {
        **slot = value;
    } else {
        *slot.bypass_change_detection() = value;
    }
}

/// The collider a body is simulated with: its own, with the entity's `PhysicsMaterial` over its
/// material, and — when it has children with colliders — their shapes too, as a compound.
#[allow(clippy::too_many_arguments)]
fn body_collider(
    world: &World,
    id: Entity,
    col: &Collider,
    transform: &Transform,
    material: Option<&gizmo_physics_core::components::PhysicsMaterial>,
    children: Option<&gizmo_core::component::Children>,
    children_query: &gizmo_core::query::Query<'_, &gizmo_core::component::Children>,
    trans_query: &gizmo_core::query::Query<'_, &Transform>,
    col_query: &gizmo_core::query::Query<'_, &Collider>,
) -> Collider {
    // **A body with no children is its own collider**, and costs one clone: the walk below
    // allocated a shape list, a boxed copy of the shape, a visited set and a stack to arrive at
    // exactly `col.clone()`.
    if children.is_none_or(|c| c.0.is_empty()) {
        let mut own = col.clone();
        own.material = col.material_with(material);
        return own;
    }
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
    //
    // **A child's `Transform` is local to its parent**, as everywhere else in the engine — the
    // render draws a child at `parent · local` (`TransformPropagateSystem`), and the editor writes
    // children that way. This walk read it as a world pose and subtracted the body's position from
    // it: a floor parented one metre under a body at (10, 3, 0) was simulated ten metres off, and
    // as a compound body moved its children's shapes slid the other way. Each child is now placed
    // by its parent's pose relative to the body, composed with its own: the position through the
    // parent's rotation and scale, as the render does — and its shape scaled by the scales above
    // it, along its own axes, as the render scales its mesh. Its own scale is its mesh's, as a root
    // body's is, and stays out. Until 2026-11-23 no scale reached a shape: a child collider written
    // like its mesh, in the child's frame, came out the mesh's size divided by the scale above it,
    // so `misket` had to write its tray walls' colliders in metres while their meshes were in tray
    // units.
    let mut visited = std::collections::HashSet::new();
    visited.insert(id);
    let mut stack = vec![(id, Relative { position: gizmo_math::Vec3::ZERO, rotation: gizmo_math::Quat::IDENTITY, scale: transform.scale })];
    while let Some((curr_id, curr)) = stack.pop() {
        if let Some(children) = children_query.get(curr_id) {
            for &child in &children.0 {
                if !world.is_alive(child) || !visited.insert(child) {
                    continue;
                }
                let local = trans_query.get(child);
                let rel = curr.then(&local.copied().unwrap_or_default());
                // A trigger under the body is not part of its solid shape: it is a trigger of its own,
                // carried with the body (`compound_parts`, `gather_static_colliders`). Merged in, it
                // took the body's flags — a trigger zone in a moving tray was a solid block, and
                // nothing ever entered it.
                if let (Some(local), Some(child_col)) = (local, col_query.get(child).filter(|c| !c.is_trigger)) {
                    let local_t = gizmo_physics_core::Transform::new(rel.position).with_rotation(rel.rotation);
                    let above = gizmo_physics_core::components::collider::scale_along(curr.scale, local.rotation);
                    compound_shapes.push((local_t, Box::new(child_col.shape.scaled(above))));
                }
                stack.push((child, rel));
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

    final_collider.material = col.material_with(material);
    final_collider
}

/// A descendant's pose in its body's frame, for the compound walk in [`body_collider`]: where it
/// is and how it is turned relative to the body, and the scale its own children's offsets go
/// through.
#[derive(Clone, Copy)]
struct Relative {
    position: gizmo_math::Vec3,
    rotation: gizmo_math::Quat,
    scale: gizmo_math::Vec3,
}

impl Relative {
    /// The pose of a child at `local` under this one.
    fn then(&self, local: &Transform) -> Relative {
        Relative {
            position: self.position + self.rotation * (self.scale * local.position),
            rotation: self.rotation * local.rotation,
            scale: self.scale * local.scale,
        }
    }
}

/// What the ECS bridge knows about what it brought over last time, so that the next step reads
/// only what changed since (see `physics_step_system`). Kept in the `PhysicsWorld`, which is the
/// resource that lives as long as the bodies it describes.
#[derive(Debug, Default)]
pub(crate) struct EcsSync {
    /// The ECS world the rest describes (`World::id`), or `None` when nothing is vouched for:
    /// never synced, or a step that did not write back.
    world: Option<u64>,
    /// The tick read up to: `World::tick − 1` at the last step, so what is written later in that
    /// same tick is news next time (the bridge's own write-back is then seen once more, and
    /// costs a comparison).
    tick: u32,
    /// `PhysicsWorld::body_revision` as the last step left it.
    revision: u64,
    /// This step's number, from 1; `present` holds it for the ids in the body set this step.
    epoch: u32,
    /// Per ECS id: the last step (`epoch`) the id was in the body set — what tells the ids that
    /// came in (or back: a pooled entity reactivated writes nothing) from those that stayed, and
    /// the ones that left.
    present: Vec<u32>,
    /// Per ECS id: `WRITTEN` / `SYNCED` / `MOVED` for this step, and what the id had at its last
    /// sync that no tick reports the loss of (`MATERIAL`, `CHILDREN`).
    flags: Vec<u8>,
}

impl EcsSync {
    const WRITTEN: u8 = 1;
    const SYNCED: u8 = 2;
    const MATERIAL: u8 = 4;
    const CHILDREN: u8 = 8;
    const MOVED: u8 = 16;
    const KEPT: u8 = Self::MATERIAL | Self::CHILDREN;

    /// Whether the record describes `world` as the physics world now holds it: the same ECS world,
    /// a tick that has not gone back, and no body set or row changed outside the bridge.
    fn vouches_for(&self, world: &World, revision: u64) -> bool {
        self.world == Some(world.id()) && self.revision == revision && world.tick > self.tick
    }

    /// Opens a step; `full` forgets what was known, so every body is read.
    fn begin(&mut self, full: bool) {
        self.epoch = self.epoch.wrapping_add(1);
        if full || self.epoch == 1 || self.epoch == 0 {
            self.present.iter_mut().for_each(|p| *p = 0);
            self.flags.iter_mut().for_each(|f| *f = 0);
            self.epoch = self.epoch.max(1);
        } else {
            self.flags.iter_mut().for_each(|f| *f &= Self::KEPT);
        }
        if full {
            self.world = None;
        }
    }

    /// The tick a `query_since` reads from.
    fn since(&self) -> u32 {
        self.tick
    }

    fn slot(&mut self, id: u32) -> usize {
        let i = id as usize;
        if i >= self.flags.len() {
            self.flags.resize(i + 1, 0);
            self.present.resize(i + 1, 0);
        }
        i
    }

    fn mark_written(&mut self, id: u32) {
        let i = self.slot(id);
        self.flags[i] |= Self::WRITTEN;
    }

    /// Enters `id` in this step's body set and says whether it must be brought over: when the
    /// record does not vouch for it (`full` was asked, it is new to the set, it was written, it
    /// lost a material or its children) or `always`. A body with children always is: what moves
    /// its children's colliders is written on them, not on it.
    fn needs(&mut self, id: u32, material: bool, children: Option<&gizmo_core::component::Children>, always: bool) -> bool {
        let i = self.slot(id);
        let kept = if material { Self::MATERIAL } else { 0 }
            | if children.is_some_and(|c| !c.0.is_empty()) { Self::CHILDREN } else { 0 };
        let stayed = self.present[i] == self.epoch.wrapping_sub(1) && self.epoch > 1;
        self.present[i] = self.epoch;
        let flags = self.flags[i];
        let needed = always
            || !stayed
            || flags & Self::WRITTEN != 0
            || flags & Self::KEPT != kept
            || kept & Self::CHILDREN != 0;
        self.flags[i] = (flags & !Self::KEPT) | kept | if needed { Self::SYNCED } else { 0 };
        needed
    }

    /// Whether `id` is in this step's body set.
    fn present(&self, id: u32) -> bool {
        self.present.get(id as usize) == Some(&self.epoch)
    }

    /// Marks `id` as moved by the step while it stayed asleep (`PhysicsWorld::moved_asleep`).
    fn mark_moved(&mut self, id: u32) {
        let i = self.slot(id);
        self.flags[i] |= Self::MOVED;
    }

    fn moved(&self, id: u32) -> bool {
        self.flags.get(id as usize).is_some_and(|f| f & Self::MOVED != 0)
    }

    /// Whether `id` was brought over this step.
    fn synced(&self, id: u32) -> bool {
        self.flags.get(id as usize).is_some_and(|f| f & Self::SYNCED != 0)
    }

    /// Closes a step that wrote the ECS back.
    fn finish(&mut self, world: &World, revision: u64) {
        self.world = Some(world.id());
        self.tick = world.tick.wrapping_sub(1);
        self.revision = revision;
    }

    /// A step that did not write back: nothing is vouched for.
    fn invalidate(&mut self) {
        self.world = None;
    }
}

/// What leaving a body out of the sync relies on, checked wherever tests run (debug builds call
/// it): the physics world already holds exactly what the sync would have put there — its rows,
/// its collider, its generation, and a broadphase box the refresh would not have changed.
fn assert_clean(physics_world: &PhysicsWorld, id: Entity, rb: &RigidBody, transform: &Transform, vel: &Velocity, collider: &Collider) {
    let Some(&i) = physics_world.entity_index_map.get(&id.id()) else {
        panic!("body {id:?} left out of the sync is not in the physics world");
    };
    assert!(
        physics_world.rigid_bodies[i] == *rb && physics_world.transforms[i] == *transform,
        "body {id:?} left out of the sync differs from the physics world's copy"
    );
    let sleeping_dynamic = rb.is_sleeping && rb.is_dynamic();
    assert!(
        sleeping_dynamic || physics_world.velocities[i] == *vel,
        "body {id:?} left out of the sync has another velocity in the physics world"
    );
    assert!(physics_world.colliders[i] == *collider, "body {id:?} left out of the sync has another collider in the physics world");
    assert_eq!(physics_world.ecs_generations.get(&id.id()), Some(&id.generation()), "body {id:?}: another generation");
    assert!(
        physics_world.spatial_hash.holds(BodyHandle::from_id(id.id()), &crate::world::synced_bounds(rb, transform, vel, collider)),
        "body {id:?} left out of the sync is not inside its broadphase box"
    );
}

/// Exclusive system that updates the entire physics simulation.
/// It reads all rigid and soft bodies from the ECS, steps the physics world,
/// and writes the transformed positions and velocities back to the ECS.
#[tracing::instrument(skip_all, name = "physics_step_system")]
pub fn physics_step_system(world: &World, dt: f32) {
    // 1. Acquire PhysicsWorld Resource
    let mut physics_world = match world.try_get_resource_mut::<PhysicsWorld>() {
        Ok(res) => res,
        Err(e) => {
            // No physics world is physics not in use here, which is no failure in a world with no
            // `RigidBody` — a puzzle on a grid, a menu. Bodies that cannot be simulated are one: they
            // stand still, and that is the warning, and only that. A game without physics used to
            // log it on every fixed step (`yilan --test`: 1 690 times).
            if world.query::<&RigidBody>().iter().next().is_some() {
                tracing::warn!(
                    error = ?e,
                    "[Physics] PhysicsWorld resource unavailable — the RigidBodies in this world are not simulated"
                );
            } else {
                tracing::trace!("[Physics] no PhysicsWorld and no RigidBody — nothing to simulate");
            }
            return;
        }
    };
    // Record profiler scope (if FrameProfiler resource is available). After the world is found: a
    // scope opened before an early return here was never closed.
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.begin_scope("physics_total");
    }

    note_bodies_without_collider(world, &mut physics_world);

    // 2. What to bring over.
    //
    // **Only what changed since the last step is read.** Every body used to be gathered, its
    // collider cloned (twice), its rows copied into the physics world and its broadphase box
    // recomputed and looked up, every step — for a static city of 14 600 bodies that was most of
    // a 12 ms step that simulated nothing. Now a body is brought over when something may have
    // moved it on the ECS side: a write to its `RigidBody`, `Transform`, `Velocity`, `Collider`,
    // `PhysicsMaterial` or `Children` since the last step (`World::query_since`), a material or
    // children taken off (which no tick reports), its arrival in the body set — and always when
    // it is awake or kinematic, has children (whose own moves no tick on it reports) or a force
    // component (pushed every step). What is left is a static body or a sleeping one nobody
    // touched, which the physics world already holds exactly. Everything is read again when the
    // record cannot vouch for that: another ECS world, a tick that went backwards, or a body set
    // changed outside this bridge (`PhysicsWorld::body_revision`). See `EcsSync`.
    let mut sync = std::mem::take(&mut physics_world.ecs_sync);
    let full = !sync.vouches_for(world, physics_world.body_revision);
    sync.begin(full);
    if !full {
        mark_written_since(world, &mut sync);
    }

    // 3. Bring the bodies over that need it (`gather_bodies`), the still ones that have no
    // `Velocity` (`gather_still_bodies`), and the colliders with no body at all, which are static
    // geometry (`gather_static_colliders`).
    let mut brought = BroughtOver::default();
    gather_bodies(world, &physics_world, &mut sync, dt, &mut brought);
    gather_still_bodies(world, &physics_world, &mut sync, &mut brought);
    gather_static_colliders(world, &physics_world, &mut sync, &mut brought);

    // 4. Step Simulation
    // An id whose entity was despawned and handed to a new one since the last step is a new
    // body: drop the old row (and its joints) so the sync adds it fresh. Measured before this: a
    // body spawned 15 m away on a dead body's id was pulled 5 m by the dead body's rope in a
    // second (`tests/joint_recycled_id.rs`).
    for (handle, ..) in &brought.bodies {
        if let Some(entity) = world.entity(handle.id()) {
            physics_world.note_ecs_generation(handle.id(), entity.generation());
        }
    }
    physics_world.update_bodies(brought.bodies.iter());
    // A body that left the set — despawned, lost its collider, pooled — leaves the world.
    physics_world.retain_bodies(|id| sync.present(id));

    // A step that rewrites rows itself (a rewind) leaves bodies changed that this bridge did not
    // bring over: they are all written back.
    let revision_before_step = physics_world.body_revision;
    // A numerical error (NaN/Inf/Overflow) in physics may originate from
    // user-controlled body state; panicking would crash the whole engine.
    // Instead, log and skip this frame gracefully (signature unchanged).
    if let Err(e) = physics_world.step(dt) {
        tracing::error!(error = ?e, "Physics step failed (NaN/Inf/Overflow), skipping frame");
        // Make sure the profiler scope opened above is closed before returning.
        // The ECS was not written back: read everything next step.
        sync.invalidate();
        physics_world.ecs_sync = sync;
        drop(physics_world); // release PhysicsWorld lock
        if let Ok(mut profiler) =
            world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>()
        {
            profiler.end_scope("physics_total");
        }
        return;
    }

    forget_owners_in_triggers(&mut physics_world, &brought.trigger_owners);
    let write_all = physics_world.body_revision != revision_before_step;

    read_back(&mut physics_world, &mut brought);
    write_back_to_ecs(world, &physics_world, &mut sync, brought, write_all);
    sync.finish(world, physics_world.body_revision);
    physics_world.ecs_sync = sync;

    dispatch_events(world, &physics_world);

    if physics_world.step_once {
        physics_world.step_once = false;
    }

    // Close profiler scope
    drop(physics_world); // PhysicsWorld lock'unu bırak
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.end_scope("physics_total");
    }
}

/// The bodies a step brings over from the ECS, in the order the physics world is handed them.
#[derive(Default)]
struct BroughtOver {
    /// Each body's handle, rows and collider.
    bodies: Vec<(BodyHandle, RigidBody, Transform, Velocity, Collider)>,
    /// The entity of each entry above, for the write-back: a `BodyHandle` is the physics side's raw id.
    entities: Vec<Entity>,
    /// Kinematic bodies driven to a pose: their index in `bodies`, and the pose to put them on after
    /// the step.
    targets: Vec<(usize, crate::components::KinematicTarget)>,
    /// Each trigger carried by a body, and that body: the trigger's own body is not something that
    /// enters it (`forget_owners_in_triggers`).
    trigger_owners: Vec<(u32, u32)>,
}

/// Counts the `RigidBody`s without a `Collider`, which are not simulated, and warns when the count
/// grows.
fn note_bodies_without_collider(world: &World, physics_world: &mut PhysicsWorld) {
    // A body is synced only with a Collider: a `RigidBody` without one is not simulated at all —
    // no gravity, no velocity integration — and stands still without a word. A guard driven by
    // its `NavAgent`'s velocity did exactly that (`gizli`). Say so once, when the count grows.
    let orphans = world
        .query_filtered::<&RigidBody, gizmo_core::query::Without<Collider>>()
        .iter()
        .count();
    if orphans > physics_world.bodies_without_collider {
        tracing::warn!(
            orphans,
            "[Physics] {orphans} RigidBody entit{} without a Collider — not simulated, so they will not move; \
             add a Collider (one on a layer with mask 0 if it should touch nothing)",
            if orphans == 1 { "y" } else { "ies" }
        );
    }
    physics_world.bodies_without_collider = orphans;
}

/// Marks every body the ECS wrote since the last step (`World::query_since`).
fn mark_written_since(world: &World, sync: &mut EcsSync) {
    use gizmo_core::query::{Changed, Or};
    let written = world.query_since::<
        Entity,
        Or<
            Changed<RigidBody>,
            Or<
                Changed<Transform>,
                Or<
                    Changed<Velocity>,
                    Or<
                        Changed<Collider>,
                        Or<Changed<gizmo_physics_core::components::PhysicsMaterial>, Changed<gizmo_core::component::Children>>,
                    >,
                >,
            >,
        >,
    >(sync.since());
    for e in written.iter() {
        sync.mark_written(e.id());
    }
}

/// Step 3: the bodies with a `Velocity` that need bringing over (`EcsSync::needs`), with their
/// compound collider, their inertia when it was never set, this step's forces, and a kinematic
/// body's velocity toward its target or its pose.
fn gather_bodies(world: &World, physics_world: &PhysicsWorld, sync: &mut EcsSync, dt: f32, brought: &mut BroughtOver) {
    let children_query = world.query::<&gizmo_core::component::Children>();
    let trans_query = world.query::<&Transform>();
    let col_query = world.query::<&Collider>();
    let target_query = world.query::<&crate::components::KinematicTarget>();
    let query = world.query_filtered::<(
        Entity,
        &RigidBody,
        &Transform,
        &Velocity,
        &Collider,
        Option<&gizmo_physics_core::components::PhysicsMaterial>,
        Option<&gizmo_core::component::Children>,
        // Forces that push on every step, not once a frame — see `ExternalForce`.
        Option<&crate::components::ExternalForce>,
        // And the engine's own, from the body's surroundings — see `EnvironmentForce`.
        Option<&crate::components::EnvironmentForce>,
    ), (gizmo_core::query::Without<gizmo_core::pool::Pooled>, gizmo_core::query::Without<gizmo_core::component::IsDeleted>)>();
    for (id, rb, transform, vel, col, material, children, external, environment) in query.iter() {
        let always = !(rb.is_static() || (rb.is_dynamic() && rb.is_sleeping)) || external.is_some() || environment.is_some();
        if !sync.needs(id.id(), material.is_some(), children, always) {
            if cfg!(debug_assertions) {
                let collider = body_collider(world, id, col, transform, material, children, &children_query, &trans_query, &col_query);
                assert_clean(physics_world, id, rb, transform, vel, &collider);
            }
            continue;
        }
        let final_collider = body_collider(world, id, col, transform, material, children, &children_query, &trans_query, &col_query);
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
        if let Some(push) = external {
            if rb.is_dynamic() {
                rb.force_accumulator += push.force;
                rb.torque_accumulator += push.torque;
            }
        }
        if let Some(push) = environment {
            if rb.is_dynamic() {
                rb.force_accumulator += push.force;
                rb.torque_accumulator += push.torque;
            }
        }
        // A kinematic body with a target moves by whatever velocity reaches it this step.
        let mut vel = *vel;
        let mut start = *transform;
        if let Some(target) = rb
            .is_kinematic()
            .then(|| target_query.get(id).copied())
            .flatten()
        {
            let com = transform.rotation * rb.center_of_mass;
            let (linear, angular) = target.velocity_from(transform.position, transform.rotation, com, dt);
            vel.linear = linear;
            vel.angular = angular;
            brought.targets.push((brought.bodies.len(), target));
        } else if rb.is_kinematic() {
            // **A kinematic body moved outside the step** — its `Transform` written by an
            // animation, a script, the game — is carried there from where the last step left
            // it, as if its new pose were a `KinematicTarget`: so it has the velocity of that
            // move, and what stands on it goes with it (`platform_carry` reads it). It used to
            // jump there with no velocity at all, and a platform keyed in the timeline slid out
            // from under the player. Once it has arrived and is not moved again, it stops.
            let moved = physics_world
                .last_pose(id.id(), id.generation())
                .filter(|last| last.position != transform.position || last.rotation != transform.rotation);
            if let Some(last) = moved {
                let target = crate::components::KinematicTarget::new(transform.position, transform.rotation);
                start.position = last.position;
                start.rotation = last.rotation;
                let com = start.rotation * rb.center_of_mass;
                let (linear, angular) = target.velocity_from(start.position, start.rotation, com, dt);
                vel.linear = linear;
                vel.angular = angular;
                rb.pose_driven = true;
                brought.targets.push((brought.bodies.len(), target));
            } else if rb.pose_driven {
                vel.linear = gizmo_math::Vec3::ZERO;
                vel.angular = gizmo_math::Vec3::ZERO;
                rb.pose_driven = false;
            }
        }
        // Bridge: ECS entity id -> opaque physics BodyHandle (generation is
        // unused by physics; bodies are keyed by id only).
        brought.bodies.push((BodyHandle::from_id(id.id()), rb, start, vel, final_collider));
        brought.entities.push(id);
    }
}

/// 3b. Bodies without a `Velocity`.
///
/// The query above needs one, and a body without it used to be left out of the simulation
/// **silently**: a static floor written as `RigidBody::new_static()` + a collider — no
/// velocity, because a floor does not move — was not there, and everything above it fell
/// through (`lobut`, 2026-11-02, measured: ten pins and a ball in free fall from frame one).
/// A static body's velocity is zero by definition, so it joins this step with one. A body
/// that moves needs a velocity that persists between steps, which only the component can
/// hold, so it is given one through the command queue and simulated from the next frame on.
/// Scenes whose bodies all carry a `Velocity` — every one written before this — take neither
/// branch, so their bodies, their order and their hashes are unchanged.
fn gather_still_bodies(world: &World, physics_world: &PhysicsWorld, sync: &mut EcsSync, brought: &mut BroughtOver) {
    let children_query = world.query::<&gizmo_core::component::Children>();
    let trans_query = world.query::<&Transform>();
    let col_query = world.query::<&Collider>();
    let query = world.query_filtered::<(
        Entity,
        &RigidBody,
        &Transform,
        Option<&Collider>,
        Option<&gizmo_physics_core::components::PhysicsMaterial>,
        Option<&gizmo_core::component::Children>,
    ), (gizmo_core::query::Without<Velocity>, gizmo_core::query::Without<gizmo_core::pool::Pooled>, gizmo_core::query::Without<gizmo_core::component::IsDeleted>)>();
    for (id, rb, transform, col, material, children) in query.iter() {
        if rb.is_static() {
            if let Some(col) = col {
                if !sync.needs(id.id(), material.is_some(), children, false) {
                    if cfg!(debug_assertions) {
                        let collider = body_collider(world, id, col, transform, material, children, &children_query, &trans_query, &col_query);
                        assert_clean(physics_world, id, rb, transform, &Velocity::default(), &collider);
                    }
                    continue;
                }
                let final_collider = body_collider(world, id, col, transform, material, children, &children_query, &trans_query, &col_query);
                brought.bodies.push((BodyHandle::from_id(id.id()), *rb, *transform, Velocity::default(), final_collider));
                brought.entities.push(id);
            }
        } else if let Ok(queue) = world.try_get_resource::<gizmo_core::commands::CommandQueue>() {
            tracing::debug!(entity = %id, "[Physics] a moving RigidBody has no Velocity — adding one; it is simulated from the next frame");
            queue.push(move |w: &mut World| {
                if w.is_alive(id) && w.query::<&Velocity>().get(id).is_none() {
                    w.add_component(id, Velocity::default());
                }
            });
        } else {
            tracing::warn!(entity = %id, "[Physics] a moving RigidBody has no Velocity and there is no CommandQueue to add one — it is not simulated");
        }
    }
}

/// 3c. Colliders without a `RigidBody`: static geometry.
///
/// A collider on its own is a wall, a floor, a level's mesh — static, as in every engine with this
/// split (Unity's static collider, Rapier's collider with no parent body). The editor's ➕ Collider
/// adds just one, the collider overlay drew them as static, and the character controller has always
/// stood on them, reading them from the ECS. This world left them out: a ball dropped on a floor
/// that was only a collider fell through it while a character walked on it, and a car's suspension
/// rays, which go through this world's broadphase, found it on their first step and never again.
///
/// Two kinds of bare collider are not static geometry and stay out: one under a body is a part of
/// that body's compound (`body_collider`), and one with a `Velocity` is moved by something else —
/// a character controller. A collider under a parent that is no body (a group in the editor) is
/// placed at its world pose, since its `Transform` is local; it is brought over every step, as a
/// body with children is, because what moves it is written on its ancestors.
///
/// Appended after every body, so a scene with no bare collider steps exactly as before.
fn gather_static_colliders(world: &World, physics_world: &PhysicsWorld, sync: &mut EcsSync, brought: &mut BroughtOver) {
    use gizmo_core::query::Without;
    let parents = world.query::<&gizmo_core::component::Parent>();
    let bodies = world.query::<&RigidBody>();
    let trans_query = world.query::<&Transform>();
    let parts = compound_parts(world);
    let query = world.query_filtered::<(
        Entity,
        &Transform,
        &Collider,
        Option<&gizmo_physics_core::components::PhysicsMaterial>,
        Option<&gizmo_core::component::Parent>,
    ), (Without<RigidBody>, Without<Velocity>, Without<gizmo_core::pool::Pooled>, Without<gizmo_core::component::IsDeleted>)>();
    for (id, transform, col, material, parent) in query.iter() {
        if parts.solid.contains(&id.id()) {
            continue;
        }
        let carried = parts.triggers.get(&id.id()).copied();
        // The pose, and the scale of what is above the collider, which its shape takes.
        let (pose, above) = if let Some((owner, rel, above)) = carried {
            // Where the body holds it, as the body stood at the start of this step.
            let Some(body) = trans_query.get(owner) else { continue };
            brought.trigger_owners.push((id.id(), owner.id()));
            let pose = Transform::new(body.position + body.rotation * rel.position)
                .with_rotation(body.rotation * rel.rotation)
                .with_scale(transform.scale);
            (pose, above)
        } else {
            match parent {
                None => (*transform, gizmo_math::Vec3::ONE),
                Some(_) => match world_pose_outside_bodies(id, transform, &parents, &bodies, &trans_query) {
                    Some(found) => found,
                    None => continue,
                },
            }
        };
        let rb = RigidBody::new_static();
        let mut collider = col.clone();
        collider.shape = col.shape.scaled(above);
        collider.material = col.material_with(material);
        if !sync.needs(id.id(), material.is_some(), None, parent.is_some() || carried.is_some()) {
            if cfg!(debug_assertions) {
                assert_clean(physics_world, id, &rb, &pose, &Velocity::default(), &collider);
            }
            continue;
        }
        brought.bodies.push((BodyHandle::from_id(id.id()), rb, pose, Velocity::default(), collider));
        brought.entities.push(id);
    }
}

/// Drops the step's trigger events between a trigger a body carries and that body. A zone in a tray
/// overlaps the tray, and "the tray entered its own zone" on every step is no event anyone asked
/// for — the trigger is the body's, as a child collider of a Unity rigidbody is.
fn forget_owners_in_triggers(physics_world: &mut PhysicsWorld, owners: &[(u32, u32)]) {
    if owners.is_empty() {
        return;
    }
    physics_world.trigger_events.retain(|e| {
        let (a, b) = (e.trigger_entity.id(), e.other_entity.id());
        !owners.iter().any(|&(trigger, owner)| (a == trigger && b == owner) || (b == trigger && a == owner))
    });
}

/// What lies under the bodies' `Children`, at any depth: the parts `body_collider` builds its
/// compound from, and the triggers it leaves out. Read from the `Children` side because that is
/// the side the compound walk follows — a child listed there without a `Parent` of its own is still
/// a part.
#[derive(Default)]
struct BodyParts {
    /// The ids that are part of a body's compound.
    solid: std::collections::HashSet<u32>,
    /// Each trigger under a body: the body, the trigger's pose in its frame — composed as the
    /// compound composes its parts — and the scale of what is above it, along its own axes, which
    /// its shape takes as a solid part's does.
    triggers: std::collections::HashMap<u32, (Entity, Relative, gizmo_math::Vec3)>,
}

fn compound_parts(world: &World) -> BodyParts {
    let children_query = world.query::<&gizmo_core::component::Children>();
    let trans_query = world.query::<&Transform>();
    let col_query = world.query::<&Collider>();
    let mut parts = BodyParts::default();
    let roots = world.query_filtered::<(Entity, &gizmo_core::component::Children, &Transform), gizmo_core::query::With<RigidBody>>();
    for (root, children, root_t) in roots.iter() {
        let top = Relative { position: gizmo_math::Vec3::ZERO, rotation: gizmo_math::Quat::IDENTITY, scale: root_t.scale };
        let mut stack: Vec<(Entity, Relative)> = children.0.iter().map(|&c| (c, top)).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some((e, above)) = stack.pop() {
            if e == root || !seen.insert(e.id()) {
                continue;
            }
            let local = trans_query.get(e).copied().unwrap_or_default();
            let rel = above.then(&local);
            if col_query.get(e).is_some_and(|c| c.is_trigger) {
                let scale = gizmo_physics_core::components::collider::scale_along(above.scale, local.rotation);
                parts.triggers.insert(e.id(), (root, rel, scale));
            } else {
                parts.solid.insert(e.id());
            }
            if let Some(grand) = children_query.get(e) {
                stack.extend(grand.0.iter().map(|&c| (c, rel)));
            }
        }
    }
    parts
}

/// The world pose of `id`, at `local` under its `Parent` chain, and the scale of that chain along
/// `id`'s own axes (what its shape takes) — or `None` when an ancestor is a `RigidBody`, whose
/// compound the collider is part of. The ancestors' rotations and scales carry the offsets, as the
/// render's propagation does; the chain is followed 32 links at most, so a `Parent` loop ends.
fn world_pose_outside_bodies(
    id: Entity,
    local: &Transform,
    parents: &gizmo_core::query::Query<'_, &gizmo_core::component::Parent>,
    bodies: &gizmo_core::query::Query<'_, &RigidBody>,
    trans_query: &gizmo_core::query::Query<'_, &Transform>,
) -> Option<(Transform, gizmo_math::Vec3)> {
    let mut chain = Vec::new();
    let mut at = id;
    while let Some(parent) = parents.get(at) {
        let up = parent.0;
        if up == id || chain.len() >= 32 {
            break;
        }
        if bodies.get(up).is_some() {
            return None;
        }
        chain.push(trans_query.get(up).copied().unwrap_or_default());
        at = up;
    }
    let mut pose = Relative { position: gizmo_math::Vec3::ZERO, rotation: gizmo_math::Quat::IDENTITY, scale: gizmo_math::Vec3::ONE };
    for ancestor in chain.iter().rev() {
        pose = pose.then(ancestor);
    }
    let above = gizmo_physics_core::components::collider::scale_along(pose.scale, local.rotation);
    let pose = pose.then(local);
    Some((Transform::new(pose.position).with_rotation(pose.rotation).with_scale(local.scale), above))
}

/// The stepped rows of the bodies brought over, for the write-back; a driven body lands on its
/// target exactly.
fn read_back(physics_world: &mut PhysicsWorld, brought: &mut BroughtOver) {
    // Sync back to the bodies brought over, for the write-back below.
    for (handle, rb, trans, vel, _) in &mut brought.bodies {
        if let Some(&i) = physics_world.entity_index_map.get(&handle.id()) {
            *rb = physics_world.rigid_bodies[i];
            *trans = physics_world.transforms[i];
            *vel = physics_world.velocities[i];
        }
    }
    // Driven bodies land on their targets exactly: the integrator's average of two velocities
    // would otherwise leave them a little off, and a little off every step is drift.
    // On both sides: the world's own copy too, or the next step would read the integrator's
    // near-miss as the body having been moved again.
    for (idx, target) in std::mem::take(&mut brought.targets) {
        let (handle, _, trans, _, _) = &mut brought.bodies[idx];
        trans.position = target.position;
        trans.rotation = target.rotation;
        if let Some(&row) = physics_world.entity_index_map.get(&handle.id()) {
            physics_world.transforms[row].position = target.position;
            physics_world.transforms[row].rotation = target.rotation;
        }
    }
}

/// Step 5: the step's results into the ECS — the bodies brought over, then those the step changed
/// without their having been (woken, moved asleep, or every body after a rewind). Each component is
/// marked changed only where the step changed it.
fn write_back_to_ecs(world: &World, physics_world: &PhysicsWorld, sync: &mut EcsSync, brought: BroughtOver, write_all: bool) {
    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    let mut query = unsafe {
        world.query_filtered_unchecked::<(&mut RigidBody, &mut Transform, &mut Velocity), gizmo_core::query::Without<gizmo_core::pool::Pooled>>()
    };
    for ((_, rb, transform, vel, _collider), entity) in brought.bodies.into_iter().zip(brought.entities) {
        // Marked changed only where the step changed something. Every body was written
        // back through `DerefMut` every step, a resting or static one included, which
        // stamped each `Transform`, `Velocity` and `RigidBody` as changed every frame: a
        // `Changed<Transform>` filter on a static city of 14 600 cells matched all of
        // them, always, and a pass that reads only what changed could skip nothing a
        // physics body touched — the render world's extractor above all, which copies a
        // `Transform` when it is `Changed` and so cloned every body's, every frame.
        if let Some((mut ecs_rb, mut ecs_trans, mut ecs_vel)) = query.get_mut(entity) {
            write_back(&mut ecs_rb, rb);
            write_back(&mut ecs_trans, transform);
            write_back(&mut ecs_vel, vel);
        }
    }
    // And the bodies the step changed without their having been brought over: a sleeping
    // body woken by a contact, one moved while it stayed asleep (`moved_asleep`), or all of
    // them after a rewind. A static body, and one asleep before and after and not moved, is
    // what the step leaves alone.
    for &id in &physics_world.moved_asleep {
        sync.mark_moved(id);
    }
    for i in 0..physics_world.entities.len() {
        let id = physics_world.entities[i].id();
        let rb = physics_world.rigid_bodies[i];
        let untouched = !write_all && !sync.moved(id) && (sync.synced(id) || rb.is_static() || rb.is_sleeping);
        if untouched && (sync.synced(id) || !cfg!(debug_assertions)) {
            continue;
        }
        let Some(entity) = world.entity(id) else { continue };
        let Some((mut ecs_rb, mut ecs_trans, mut ecs_vel)) = query.get_mut(entity) else { continue };
        if untouched {
            // Debug builds only: what leaving it out relies on.
            assert!(
                *ecs_rb == rb && *ecs_trans == physics_world.transforms[i] && *ecs_vel == physics_world.velocities[i],
                "body {entity:?} was neither brought over nor written back, and the step changed it"
            );
            continue;
        }
        write_back(&mut ecs_rb, rb);
        write_back(&mut ecs_trans, physics_world.transforms[i]);
        write_back(&mut ecs_vel, physics_world.velocities[i]);
    }
}

/// The trigger and collision events of every physics step since its reader last took them.
///
/// The physics world keeps only the last step's events (`PhysicsWorld::trigger_events`), and a
/// frame takes as many fixed steps as its time buys — none in a short frame, two or more in a long
/// one. A reader that looks once a frame at the world's list therefore missed every step but the
/// last, and saw the last again on a frame with no step: a Lua script counted a ball entering a
/// zone 0 times at 30 fps, once at 60, twice at 120 and four times at 240. With this resource in the
/// world, each step appends its events here, and the reader takes them (`std::mem::take`) — each
/// event exactly once, whatever the frame rate.
///
/// Inserted by the host that has such a reader (`gizmo::systems::PlayLoop`, for the scripts), and
/// emptied by it: nothing else takes from it, so a log nobody reads grows.
#[derive(Debug, Default)]
pub struct PhysicsEventLog {
    /// Every trigger event since the last take, in step order.
    pub triggers: Vec<gizmo_physics_core::TriggerEvent>,
    /// Every collision event since the last take, in step order.
    pub collisions: Vec<gizmo_physics_core::CollisionEvent>,
}

/// Step 7: the step's trigger and collision events, into the ECS's event queues and the event log.
fn dispatch_events(world: &World, physics_world: &PhysicsWorld) {
    if let Ok(mut log) = world.try_get_resource_mut::<PhysicsEventLog>() {
        log.triggers.extend(physics_world.trigger_events.iter().cloned());
        log.collisions.extend(physics_world.collision_events.iter().cloned());
    }
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
}

/// System that processes collision events and breaks objects that exceed their threshold.
///
/// [`fracture_step`] without its answer; the breaks still reach `Events<BreakEvent>` when the
/// world has that queue.
pub fn physics_fracture_system(world: &World, dt: f32) {
    let _ = fracture_step(world, dt);
}

/// One fracture step: ages the debris, breaks every [`Breakable`](crate::components::Breakable)
/// the step's collisions broke, and returns what broke, in the order it broke — each break once,
/// also sent into `Events<BreakEvent>` when the world has that queue.
///
/// The return value is for the caller that answers a break itself (`gizmo` plays its
/// `break_sound`): a broken body is despawned only when the commands are applied, so looking for
/// latched `is_broken` bodies would see one break again on a second step of the same frame.
pub fn fracture_step(world: &World, dt: f32) -> Vec<crate::components::BreakEvent> {
    use crate::components::Breakable;
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let physics_world = match world.try_get_resource::<PhysicsWorld>() {
        Ok(res) => res,
        // No physics world ⇒ nothing to fracture. Optional-resource absence, not a bug → trace.
        Err(_) => {
            tracing::trace!("[Physics] fracture system: no PhysicsWorld resource, skipping");
            return Vec::new();
        }
    };

    let mut commands = match Commands::fetch_stateless(world, dt) {
        Ok(c) => c,
        // Commands are the ECS deferred-op channel; without them every fracture spawn is
        // silently dropped — a hidden failure worth surfacing (debug: infra-level).
        Err(e) => {
            tracing::debug!(error = ?e, "[Physics] fracture system: Commands unavailable, fractures dropped this frame");
            return Vec::new();
        }
    };

    age_debris(world, &mut commands, dt);

    let mut shattered = std::collections::HashSet::new();
    let mut breaks = Vec::new();

    // `Velocity` is optional: a breakable standing as bare collider (static geometry — a pane, a
    // wall) has none, and requiring it made such a body immune to every hit. Its pieces start at
    // rest.
    // SAFETY: scheduled system; scheduler guarantees disjoint mutable access.
    let query_opt = unsafe {
        world.query_filtered_unchecked::<(&mut Breakable, &Transform, &Collider, Option<&Velocity>, Option<&RigidBody>), gizmo_core::query::Without<gizmo_core::pool::Pooled>>()
    };
    let mut query = query_opt;

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
            if let Some((entity_a, (mut breakable, transform, collider, vel, body))) =
                resolved_a.and_then(|e| query.get_mut(e).map(|q| (e, q)))
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        // Latch only if it really broke — see `shatter_entity`.
                        let pieces = shatter_entity(
                            &mut commands,
                            entity_a,
                            &breakable,
                            transform,
                            collider,
                            vel.copied().unwrap_or_default(),
                            body,
                            -impact_normal,
                        );
                        if let Some(pieces) = pieces {
                            breakable.is_broken = true;
                            shattered.insert(event.entity_a.id());
                            release_parts(world, &mut commands, entity_a, transform, vel.copied().unwrap_or_default(), body, breakable.debris_lifetime);
                            breaks.push(report_break(world, entity_a, &breakable, transform.position, impact_point, pieces));
                        }
                    }
                }
            }
        }

        // Check Entity B
        if !shattered.contains(&event.entity_b.id()) {
            // Resolved rather than fabricated — see the note on entity A above.
            let resolved_b = world.entity(event.entity_b.id());
            if let Some((entity_b, (mut breakable, transform, collider, vel, body))) =
                resolved_b.and_then(|e| query.get_mut(e).map(|q| (e, q)))
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        let pieces = shatter_entity(
                            &mut commands,
                            entity_b,
                            &breakable,
                            transform,
                            collider,
                            vel.copied().unwrap_or_default(),
                            body,
                            impact_normal,
                        );
                        if let Some(pieces) = pieces {
                            breakable.is_broken = true;
                            shattered.insert(event.entity_b.id());
                            release_parts(world, &mut commands, entity_b, transform, vel.copied().unwrap_or_default(), body, breakable.debris_lifetime);
                            breaks.push(report_break(world, entity_b, &breakable, transform.position, impact_point, pieces));
                        }
                    }
                }
            }
        }
    }
    drop(query);
    breaks
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
    vel: Velocity,
    body: Option<&RigidBody>,
    impact_direction: gizmo_math::Vec3,
) -> Option<u32> {
    use crate::fracture::voronoi_shatter;

    let (center, extents) = shatter_bounds(collider)?;

    // Despawn the original entity
    commands.entity(entity).despawn();

    // Generate chunks. The seed is derived from the entity id rather than being a constant, so
    // two objects breaking under identical conditions produce different debris while a given
    // object still breaks the same way on every replay — see `shatter_seed`.
    let chunks = voronoi_shatter(extents, breakable.max_pieces, shatter_seed(entity.id()));
    let pieces = chunks.len() as u32;

    for chunk in chunks {
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
        let mut new_vel = vel;
        let outward = chunk.center_of_mass.normalize_or_zero();
        new_vel.linear += outward * 2.0 + impact_direction * 5.0; // Explosion effect

        // The piece is a body of the cell's own shape, the hull the facade draws it with. It used
        // to be a sphere of radius `(0.1 · volume)^(1/3)` — 42 % of the cell's volume — which the
        // drawn cell sank into, and which, having no rolling friction, rolled on forever: measured,
        // a broken crate's ten pieces were still rolling at 1.7 m/s after eight seconds and had
        // spread 14 m, where hulls come to rest within four seconds and 3.3 m.
        let local = center_of_mass_frame(&chunk);
        let chunk_collider = Collider::convex_hull(&local).with_material(collider.material);
        let mut rb = RigidBody::new(chunk.volume * collider.material.density, true);
        rb.update_inertia_from_collider(&chunk_collider);
        keep_locks(&mut rb, body);
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

    Some(pieces)
}

/// A piece of a broken body keeps the body's axis locks: what broke in a plane (`RigidBody::planar_xy`,
/// every side-on game) breaks in it. The pieces were spawned free, and a 2-D game's debris and
/// parts flew out of its plane — measured in `inis`, a lander's legs 1.7 m off it.
fn keep_locks(piece: &mut RigidBody, broken: Option<&RigidBody>) {
    let Some(b) = broken else { return };
    piece.lock_translation_x = b.lock_translation_x;
    piece.lock_translation_y = b.lock_translation_y;
    piece.lock_translation_z = b.lock_translation_z;
    piece.lock_rotation_x = b.lock_rotation_x;
    piece.lock_rotation_y = b.lock_rotation_y;
    piece.lock_rotation_z = b.lock_rotation_z;
}

/// What a shattered body held besides its own shape, let go of: each part of its compound — a
/// descendant with a solid collider and no body of its own — becomes a body of its own where it
/// was, moving as that point of the body was (`v + ω × r`), its shape at the scale it had there,
/// and ages as debris (`lifetime`); its own children stay with it. A descendant with a body of its
/// own lives on. Triggers and visuals under the broken body go with it.
///
/// Until 2026-11-23 the break despawned the body alone, and despawning a parent unparents its
/// children: a lander's leg, parented one metre down and to the side, became static geometry at
/// that LOCAL position read as a world one — beside the world's origin, half in the ground — and a
/// mesh under the body was left drawn where the body had stood.
fn release_parts(
    world: &World,
    commands: &mut gizmo_core::commands::Commands,
    body: Entity,
    transform: &Transform,
    vel: Velocity,
    broken: Option<&RigidBody>,
    lifetime: f32,
) {
    use gizmo_core::component::{Children, Parent};
    let children_q = world.query::<&Children>();
    let trans_q = world.query::<&Transform>();
    let col_q = world.query::<&Collider>();
    let bodies = world.query::<&RigidBody>();
    let mut visited = std::collections::HashSet::new();
    visited.insert(body);
    let mut stack = vec![(body, Relative { position: gizmo_math::Vec3::ZERO, rotation: gizmo_math::Quat::IDENTITY, scale: transform.scale })];
    while let Some((at, rel)) = stack.pop() {
        let Some(children) = children_q.get(at) else { continue };
        for &child in &children.0 {
            if !world.is_alive(child) || !visited.insert(child) || bodies.get(child).is_some() {
                continue;
            }
            let local = trans_q.get(child).copied().unwrap_or_default();
            let here = rel.then(&local);
            let Some(col) = col_q.get(child).filter(|c| !c.is_trigger) else {
                commands.entity(child).despawn();
                stack.push((child, here));
                continue;
            };
            let above = gizmo_physics_core::components::collider::scale_along(rel.scale, local.rotation);
            let mut part = col.clone();
            part.shape = col.shape.scaled(above);
            let position = transform.position + transform.rotation * here.position;
            let pose = Transform::new(position).with_rotation(transform.rotation * here.rotation).with_scale(here.scale);
            let moving = Velocity { linear: vel.linear + vel.angular.cross(position - transform.position), ..vel };
            let mut rb = RigidBody::new((part.volume() * part.material.density).max(1e-3), true);
            rb.update_inertia_from_collider(&part);
            keep_locks(&mut rb, broken);
            commands
                .entity(child)
                .remove::<Parent>()
                .insert(pose)
                .insert(part)
                .insert(rb)
                .insert(moving)
                .insert(crate::components::Debris {
                    source: body.id(),
                    vertices: Vec::new(),
                    normals: Vec::new(),
                    indices: Vec::new(),
                    age: 0.0,
                    lifetime,
                });
        }
    }
}

/// The [`BreakEvent`](crate::components::BreakEvent) for a body that has just shattered, also
/// sent into the world's `Events<BreakEvent>` when it has that queue.
fn report_break(
    world: &World,
    body: Entity,
    breakable: &crate::components::Breakable,
    position: gizmo_math::Vec3,
    impact_point: gizmo_math::Vec3,
    pieces: u32,
) -> crate::components::BreakEvent {
    let event = crate::components::BreakEvent { body, position, impact_point, pieces, sound: breakable.break_sound.clone() };
    if let Ok(mut events) = world.try_get_resource_mut::<gizmo_core::event::Events<crate::components::BreakEvent>>() {
        events.send(event.clone());
    }
    event
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
    let mut pieces = unsafe { world.query_unchecked::<(Entity, &mut crate::components::Debris)>() };
    for (id, mut piece) in pieces.iter_mut() {
        piece.age += dt;
        if piece.lifetime > 0.0 && piece.age >= piece.lifetime {
            commands.entity(id).despawn();
        }
    }
}

/// System that checks for Explosion components and applies outward forces
/// to all rigid bodies and soft body nodes within the radius.
///
/// [`explosion_step`] without its answer; the breaks still reach `Events<BreakEvent>` when the
/// world has that queue.
pub fn physics_explosion_system(world: &World, dt: f32) {
    let _ = explosion_step(world, dt);
}

/// One explosion pass: every active [`Explosion`](crate::components::Explosion) pushes the bodies
/// in its `force_radius`, damages the breakables in its `damage_radius`, and is despawned. Returns
/// what broke, as [`fracture_step`] does.
pub fn explosion_step(world: &World, dt: f32) -> Vec<crate::components::BreakEvent> {
    use crate::components::{Explosion, ExplosionFalloff};
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let mut commands = match Commands::fetch_stateless(world, dt) {
        Ok(c) => c,
        // Without Commands the explosion despawns/impulses can't be issued — surface it.
        Err(e) => {
            tracing::debug!(error = ?e, "[Physics] explosion system: Commands unavailable, skipping");
            return Vec::new();
        }
    };

    let explosion_query_opt = world.query_filtered::<(Entity, &Explosion, &Transform), gizmo_core::query::Without<gizmo_core::pool::Pooled>>();
    let mut active_explosions = Vec::new();

    {
        let exp_query = &explosion_query_opt;
        for (exp_entity, explosion, transform) in exp_query.iter() {
            if explosion.is_active {
                // Resolved, not fabricated: this handle is what despawns the explosion at the
                // end of the pass. A generation-0 handle to a recycled slot fails
                // `World::despawn`'s liveness check SILENTLY, leaving `is_active` set — so the
                // blast re-detonates on every subsequent frame, against `Explosion`'s own
                // documented contract that it applies exactly once and then despawns.
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
        return Vec::new(); // Nothing to explode
    }

    let mut shattered = std::collections::HashSet::new();
    let mut breaks = Vec::new();

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
        world.query_filtered_unchecked::<(Entity, &mut crate::components::Breakable, &Transform, &Collider, Option<&Velocity>, Option<&RigidBody>), gizmo_core::query::Without<gizmo_core::pool::Pooled>>()
    };
    {
        let breakable_query = &mut breakable_query_opt;
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (entity, mut breakable, transform, collider, vel, body) in breakable_query.iter_mut() {
                if breakable.is_broken || shattered.contains(&entity) {
                    continue;
                }

                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();

                if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let intensity =
                        calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                    let impulse_mag = explosion.force * intensity;
                    // The damage reaches as far as `damage_radius`, on its own falloff — until
                    // 2026-11-22 a field nothing read, the damage riding on the push's radius.
                    let hurt = calculate_intensity(dist, explosion.damage_radius, explosion.falloff);

                    if hurt > 0.0 && impulse_mag > breakable.threshold {
                        breakable.current_health -= explosion.damage * hurt;
                        if breakable.current_health <= 0.0 {
                            let dir = diff / dist;
                            let mut exp_vel = vel.copied().unwrap_or_default();
                            exp_vel.linear += dir * impulse_mag * 0.1; // Estimate mass
                            // Resolved, not fabricated. A generation-0 handle to a recycled
                            // slot makes `shatter_entity`'s despawn a silent no-op while the
                            // debris still spawns and `is_broken` latches below — leaving the
                            // original in the scene beside its own debris and, since every
                            // damage path is gated on `!is_broken` and nothing clears it,
                            // permanently undamageable.
                            let pieces = shatter_entity(&mut commands, entity, &breakable, transform, collider, exp_vel, body, dir);
                            if let Some(pieces) = pieces {
                                breakable.is_broken = true;
                                shattered.insert(entity);
                                release_parts(world, &mut commands, entity, transform, exp_vel, body, breakable.debris_lifetime);
                                breaks.push(report_break(world, entity, &breakable, transform.position, *exp_pos, pieces));
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
        world.query_filtered_unchecked::<(Entity, &mut RigidBody, &Transform, &mut Velocity), gizmo_core::query::Without<gizmo_core::pool::Pooled>>()
    };
    {
        let rb_query = &mut rb_query_opt;
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (entity, mut rb, transform, mut vel) in rb_query.iter_mut() {
                if !rb.is_dynamic() || shattered.contains(&entity) {
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
    breaks
}



#[cfg(test)]
mod apply_impulse_tests {
    use super::*;
    use gizmo_math::Vec3;

    fn body(world: &mut World, rb: RigidBody) -> Entity {
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::ZERO));
        world.add_component(e, rb);
        world.add_component(e, Velocity::default());
        world.add_component(e, Collider::sphere(0.5));
        e
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
        let v = *world.query::<&Velocity>().get(ball).unwrap();
        assert!((v.linear - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5, "{v:?}");
        assert!(v.angular.length() < 1e-6, "through the centre must not spin it");
        assert!(!world.query::<&RigidBody>().get(ball).unwrap().is_sleeping, "still asleep");

        assert!(apply_impulse(&mut world, ball, Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 0.5, 0.0)));
        assert!(world.query::<&Velocity>().get(ball).unwrap().angular.length() > 0.1, "off-centre must spin it");

        let wall = body(&mut world, RigidBody::new_static());
        assert!(!apply_impulse(&mut world, wall, Vec3::X, Vec3::ZERO));
        assert_eq!(world.query::<&Velocity>().get(wall).unwrap().linear, Vec3::ZERO);
        assert!(!apply_impulse(&mut world, Entity::new(9999, 0), Vec3::X, Vec3::ZERO));
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
        let y = |e: gizmo_core::entity::Entity| world.query::<&Transform>().get(e).unwrap().position.y;
        assert_eq!(y(bare), 5.0, "not simulated");
        assert!(y(full) < 4.0, "the same body with a collider falls");
    }
}
