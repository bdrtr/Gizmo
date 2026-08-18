//! ECS system wrappers that drive the (manually-written, audited) gameplay
//! controllers in this crate from the schedule.
//!
//! All three use the exclusive-barrier signature `fn(&World, dt)` — the same
//! shape as [`gizmo_physics_rigid::physics_step_system`] — so they can be added
//! straight into a [`gizmo_core::system::Schedule`] (see
//! `gizmo_app::gameplay`). A `fn(&World, f32)` reports itself as an *exclusive*
//! system, so the scheduler runs it alone; that is what makes the
//! `query_unchecked` mutable borrows below sound.
//!
//! # Ordering / determinism
//! * [`vehicle_controller_system`] applies suspension + tire forces to the
//!   chassis `Velocity`, so it must run **before** the rigid physics step
//!   integrates that velocity. Register it in [`gizmo_core::system::Phase::Physics`]
//!   with `.before("physics_step_system")`.
//! * [`character_controller_system`] performs its own kinematic
//!   move/step/slide and writes `Transform`/`Velocity` directly; it does not
//!   depend on the rigid solver.
//! * [`fighter_frame_system`] touches no physics state at all — it counts frames on
//!   `FighterController` — so it has no ordering edge against the rigid step. It does need to
//!   run in the **fixed** schedule, once per step, because frames are what it counts.
//!
//! All three are **no-ops** on worlds that contain no vehicle/character/fighter
//! entities (the component-tuple query matches nothing), so registering them
//! does not perturb a plain rigid-body scene (e.g. the determinism oracle).
//!
//! # How each one sees the scene
//! * [`vehicle_controller_system`] casts its suspension rays through the
//!   [`PhysicsWorld`](gizmo_physics_rigid::world::PhysicsWorld) broadphase, one
//!   `raycast_filtered` per wheel. It clones nothing. **This changed what a wheel can rest
//!   on** — see that function's docs.
//! * [`character_controller_system`] still snapshots every collider in the world into an owned
//!   `Vec` each step (`gather_colliders`) and scans it. Same defect, different fix (a capsule
//!   sweep, not a ray); tracked as a follow-up.

use gizmo_core::component::IsDeleted;
use gizmo_core::query::{Mut, Without};
use gizmo_core::world::World;
use gizmo_physics_core::components::{CharacterController, FighterController};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};

use crate::character::update_character;
use crate::vehicle::{
    update_vehicle_with_query, weather_grip_factor, ColliderListQuery, VehicleController,
    WheelGroundQuery,
};

/// Snapshot every live `(Transform, Collider)` into an owned buffer so the
/// controllers can raycast against the scene while we later hold *mutable*
/// borrows on the moving entities. `Transform` is `Copy` and `Collider` is
/// cheap to clone (shapes are `Arc`-backed), matching `physics_step_system`.
///
/// The read-only query is fully drained into the returned `Vec` and dropped
/// here, so it never overlaps the mutable query opened by the callers.
///
/// # Cost, and who still pays it
///
/// This is `O(colliders)` clones **per step**, and each caller that scans the result pays
/// `O(colliders)` again per ray. [`vehicle_controller_system`] no longer calls it except on
/// the one step before the physics world has been populated — it queries the broadphase
/// instead. [`character_controller_system`] still calls it every step: the KCC is the twin of
/// the same defect and is deliberately left for a follow-up, because it needs a *capsule
/// sweep* rather than a ray and because it is routinely run against `Collider`-only entities
/// that never enter a `PhysicsWorld` (see this module's tests).
fn gather_colliders(world: &World) -> Vec<(BodyHandle, Transform, Collider)> {
    let mut colliders = Vec::new();
    if let Some(query) = world.query::<(&Transform, &Collider, Without<IsDeleted>)>() {
        for (id, (transform, collider, _)) in query.iter() {
            colliders.push((BodyHandle::from_id(id), *transform, collider.clone()));
        }
    }
    colliders
}

/// Drives [`VehicleController`] (Pacejka tire model + suspension + gearbox) for
/// every vehicle entity each fixed step.
///
/// Query: `Mut<VehicleController>` + `Mut<RigidBody>` + `&Transform` +
/// `Mut<Velocity>`. The controller reads steering/throttle/brake inputs off the
/// `VehicleController` component and writes the resulting forces into `Velocity`
/// (which the rigid physics step then integrates), so this must run *before*
/// `physics_step_system`.
///
/// # Where the wheels look (changed in 0.10 — read this)
///
/// The suspension rays go through the **broadphase**, as one
/// [`PhysicsWorld::raycast_filtered`](gizmo_physics_rigid::world::PhysicsWorld::raycast_filtered)
/// per wheel, excluding the chassis. They used to go through an owned snapshot of *every*
/// `(Transform, Collider)` entity in the ECS — cloned in full, then scanned linearly once per
/// wheel, every step. That snapshot is gone from this path.
///
/// The set of surfaces a wheel can rest on therefore changed:
///
/// * **A wheel now rests only on bodies the rigid pipeline simulates** — entities carrying
///   `RigidBody` + `Transform` + `Velocity` + `Collider` and neither `Pooled` nor `IsDeleted`,
///   because that is exactly what `physics_step_system` syncs into the [`PhysicsWorld`]. A
///   bare `Collider` + `Transform` entity (no `RigidBody`) is no longer drivable. It was
///   before — while being intangible to the chassis, which collided with nothing there. Give
///   level geometry a `RigidBody::new_static()` and a `Velocity`, as every vehicle demo in
///   this repo already does, or call [`update_vehicle`](crate::vehicle::update_vehicle)
///   directly with your own list.
/// * **A body becomes visible to the wheels one step after it is spawned**, when the next
///   `physics_step_system` syncs it. Before the world's very first step the `PhysicsWorld`
///   holds no bodies at all, and this system falls back to the old ECS scan for that step, so
///   "spawn a car and read `is_grounded` before stepping" still works.
/// * **…and one step behind on geometry moved from OUTSIDE the solver.** This system runs
///   *before* `physics_step_system`, which is the call that copies ECS `Transform`s into the
///   [`PhysicsWorld`], so the poses the rays see are the ones the previous step left behind.
///   A body the simulation itself moves is unaffected (the step wrote both), but a teleport, a
///   scripted/kinematic mover or an editor drag applied to a `Transform` this frame reaches the
///   wheels next frame. The old ECS scan read those writes immediately. At 120 Hz a platform
///   moving 10 m/s is 8 cm of lag; if that matters, step physics before the controller or drive
///   the mover through the solver.
/// * **A body must be in the ECS to stay in the world.** `physics_step_system` calls
///   `sync_bodies` with the ECS `RigidBody`+`Transform`+`Velocity`+`Collider` set and *deletes*
///   every body absent from it — so a surface pushed straight into the world with
///   `PhysicsWorld::add_body`, without the matching components, is evicted on the first step
///   and is then invisible to the wheels as well as to the solver.
/// * **A wheel sees the collider the *solver* sees**, which is the entity's own shape merged
///   with its children into a `Compound`, carrying the entity's `PhysicsMaterial` — and,
///   today, `is_trigger == false` regardless of what the ECS `Collider` says, because
///   `physics_step_system` drops that flag when it rebuilds the collider. So an ECS trigger
///   volume that also has a `RigidBody` is now drivable. That is a defect in the ECS→solver
///   bridge, not in the filter used here (which does exclude triggers); the wheels simply
///   agree with the solver, for which that volume is already solid.
/// * **Unchanged:** the chassis never hits itself, there is no layer mask (every layer is
///   fair game), dynamic bodies are still drivable, the nearest hit wins, and the surface's
///   `dynamic_friction` still comes from the collider that was hit.
///
/// [`PhysicsWorld`]: gizmo_physics_rigid::world::PhysicsWorld
#[tracing::instrument(skip_all, name = "vehicle_controller_system")]
pub fn vehicle_controller_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    // Hava durumu grip çarpanı ve tekerlek raycast'lerinin ikisi de PhysicsWorld'den gelir;
    // resource guard tek seferde alınır ve aşağıdaki unsafe *bileşen* query'siyle çakışmaz
    // (kaynak deposu ayrı — `character_controller_system` de aynısını yapar).
    let phys = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>();

    let weather = match phys.as_ref() {
        Some(w) => w.weather,
        None => {
            tracing::trace!(
                "[Vehicle] no PhysicsWorld resource — defaulting weather to Sunny (no grip penalty)"
            );
            gizmo_physics_rigid::world::Weather::default()
        }
    };

    // Broadphase yolu asıl yol. Geri düşüş YALNIZ fizik dünyası henüz hiç adım atmamışken
    // (kaynak yok ya da içi boş) devreye girer: o tek kare için eski ECS taraması yapılır,
    // yoksa "aracı spawn et, hemen is_grounded oku" akışı sessizce bozulurdu.
    let broadphase_ready = phys.as_ref().is_some_and(|pw| !pw.entities.is_empty());
    let fallback = if broadphase_ready {
        None
    } else {
        Some(gather_colliders(world))
    };
    let fallback_query = fallback.as_deref().map(ColliderListQuery::new);

    let ground: &dyn WheelGroundQuery = match (fallback_query.as_ref(), phys.as_ref()) {
        (Some(list), _) => list,
        (None, Some(pw)) => &**pw,
        // `fallback` is `Some` whenever `phys` is `None`, so this arm is unreachable; it is
        // written out rather than `unreachable!()` so a future edit degrades to a no-op.
        (None, None) => return,
    };

    // SAFETY: a `fn(&World, f32)` system reports `is_exclusive`, so the scheduler
    // runs it alone — no other query mutably aliases these components while this
    // one is live. The read-only `gather_colliders` query above (fallback path only)
    // was already dropped, so there is no overlapping `&`/`&mut` on `Transform` either.
    let query = unsafe {
        world.query_unchecked::<(
            Mut<VehicleController>,
            Mut<RigidBody>,
            &Transform,
            Mut<Velocity>,
            Without<IsDeleted>,
        )>()
    };
    if let Some(mut query) = query {
        let mut vehicle_count = 0usize;
        for (id, (mut vehicle, mut rb, transform, mut vel, _)) in query.iter_mut() {
            // Aquaplaning hıza bağlı → her araç kendi hızıyla değerlendirilir.
            let wg = weather_grip_factor(weather, vel.linear.length());
            update_vehicle_with_query(
                BodyHandle::from_id(id),
                &mut vehicle,
                &mut rb,
                transform,
                &mut vel,
                ground,
                wg,
                dt,
            );
            vehicle_count += 1;
        }
        tracing::trace!(
            vehicle_count,
            broadphase = broadphase_ready,
            cloned_colliders = fallback.as_ref().map_or(0, Vec::len),
            "[Vehicle] controller system tick"
        );
    }
}

/// Drives the kinematic character controller ([`CharacterController`] +
/// [`update_character`]) for every character entity each fixed step.
///
/// Query: `Mut<CharacterController>` + `Mut<Transform>` + `Mut<Velocity>` +
/// `&Collider`. The KCC does its own gravity / ground-snap / step / slide
/// integration and writes `Transform`/`Velocity` directly, so KCC entities must
/// **not** also carry a dynamic `RigidBody` (that would double-integrate
/// gravity via the rigid step).
#[tracing::instrument(skip_all, name = "character_controller_system")]
pub fn character_controller_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    let all_colliders = gather_colliders(world);

    // Yüzme modu için sahnenin PhysicsWorld'ünden fluid-zone batık sorgusu (yoksa hiç su → kara).
    // Copy olmayan bir referans; aşağıdaki unsafe component query'siyle çakışmaz (resource ayrı
    // storage). Her karakter kendi konumuyla `water_at` sorgulanır.
    let phys = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>();

    // SAFETY: see `vehicle_controller_system` — exclusive barrier system, and the
    // read-only gather query is dropped before this mutable query is opened, so
    // the `Mut<Transform>` here never aliases the `&Transform` used above.
    let query = unsafe {
        world.query_unchecked::<(
            Mut<CharacterController>,
            Mut<Transform>,
            Mut<Velocity>,
            &Collider,
            Without<IsDeleted>,
        )>()
    };
    if let Some(mut query) = query {
        let mut char_count = 0usize;
        for (id, (mut kcc, mut transform, mut vel, collider, _)) in query.iter_mut() {
            let water_surface_y = phys
                .as_ref()
                .and_then(|pw| pw.water_at(transform.position).map(|s| s.surface_y));
            update_character(
                BodyHandle::from_id(id),
                &mut kcc,
                &mut transform,
                &mut vel,
                collider,
                &all_colliders,
                water_surface_y,
                dt,
            );
            char_count += 1;
        }
        tracing::trace!(
            char_count,
            collider_count = all_colliders.len(),
            "[KCC] controller system tick"
        );
    }
}

/// Advances every [`FighterController`] in the world by one fixed frame:
/// [`FighterController::tick`] on each.
///
/// **This system is the fight subsystem's clock, and before it nothing was.** The component is
/// pure data whose every duration is a frame count, and its own documentation said "the game (or
/// a script) must tick them once per fixed frame" — but no game, script or system in the engine
/// did. The measured consequence: `fighter.apply_hitstop(id, 6)` from Lua froze that fighter for
/// the rest of the process rather than for six frames, `fighter.set_move` started a move that
/// never reached its active window, and `is_in_active_window` therefore never once answered
/// `true`. The subsystem had a full authoring surface — the studio adds the component, the
/// inspector edits it, the scene format serialises it, the fight HUD reads it, three Lua calls
/// write it — over a state machine with no clock.
///
/// **Fixed schedule, not update.** The counters are frames, so one call must mean one fixed
/// step; called once per rendered frame instead, a move's timing would follow the frame rate.
/// `gizmo_app::gameplay::register_gameplay_physics_systems` places it in
/// [`Phase::Physics`](gizmo_core::system::Phase::Physics), and `gizmo::systems::PlayLoop` calls
/// it once per step of its own accumulator.
///
/// **What it deliberately does not do**: it does not feed `input_buffer` (what to record is the
/// game's set of action names — see [`FighterInputBuffer::update`](gizmo_core::input::FighterInputBuffer::update)),
/// it does not start moves from input (which move a button means is the game's or a script's),
/// and it does not drive `Hitbox::active` from the active window. Each of those is a policy the
/// engine would be guessing at; the clock is not.
///
/// `dt` is used only as the pause signal (`dt <= 0.0` returns without ticking), matching
/// [`oxygen_system`](crate::oxygen::oxygen_system): a paused game must not spend frames.
#[tracing::instrument(skip_all, name = "fighter_frame_system")]
pub fn fighter_frame_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    // SAFETY: exclusive `fn(&World, f32)` system — the scheduler runs it alone — and
    // `FighterController` is the only component type borrowed here, so nothing aliases.
    let query = unsafe { world.query_unchecked::<(Mut<FighterController>, Without<IsDeleted>)>() };
    if let Some(mut query) = query {
        let mut fighter_count = 0usize;
        for (_id, (mut fighter, _)) in query.iter_mut() {
            fighter.tick();
            fighter_count += 1;
        }
        if fighter_count > 0 {
            tracing::trace!(fighter_count, "[Fight] frame clock tick");
        }
    }
}

/// Resolves fighting-game hits: drives each fighter's hitboxes from its active window, tests them
/// against everyone else's hurtboxes, and reports every connection as a
/// [`HitEvent`](gizmo_physics_core::components::HitEvent).
///
/// **The engine reports; the game decides what a hit costs.** Nothing here subtracts health,
/// applies hitstun or ends a round — those are the rules of a particular fighting game, and they
/// belong to the game reading these events. What is not game-specific, and what was missing, is
/// the middle: turning "this move is in its active frames" into "these two volumes overlap, once".
///
/// Events go to `gizmo_core::event::Events<HitEvent>` if that resource exists; with no queue in
/// the world the system still drives `Hitbox::active` (which is what the debug gizmo draws) and
/// still records the hit, so a scene without an event queue does not silently re-hit forever.
///
/// # What it decides, and how
///
/// - **Which hitbox is live.** A hitbox owned by a fighter — on the fighter entity or anywhere
///   under it — is active exactly while that fighter is inside its move's active window *and* the
///   box's [`move_name`](gizmo_physics_core::components::Hitbox::move_name) matches (or is
///   `None`, meaning every move). A hitbox with no fighter above it is left alone: a trap or a
///   projectile drives its own flag.
/// - **Who owns what.** Ownership is the nearest `FighterController` up the `Parent` chain, so a
///   fist entity parented to a fighter attacks *for* that fighter and a torso hurtbox on a limb
///   defends *for* it. A hurtbox under the attacker itself is skipped — nobody punches themselves.
/// - **Once per move.** The attacker records each victim in `already_hit`, which
///   [`FighterController::start_move`](gizmo_physics_core::components::fighter::FighterController::start_move)
///   and the end of a move clear. Without it a three-frame active window would report the same hit
///   three times, one per frame, since the boxes go on overlapping.
/// - **Where the boxes are.** Poses are composed from the local `Transform` chain up to the root,
///   so this needs no transform-propagation pass to have run. Scale is ignored — see
///   [`hit_volumes_overlap`](gizmo_physics_core::components::hit_volumes_overlap), which the
///   engine's own debug gizmo agrees with.
///
/// Fixed schedule, after `fighter_frame_system`: the window it reads has to be this frame's.
#[tracing::instrument(skip_all, name = "hit_detection_system")]
pub fn hit_detection_system(world: &World, dt: f32) {
    use gizmo_core::component::Parent;
    use gizmo_physics_core::components::{hit_volumes_overlap, HitEvent, Hitbox, Hurtbox};
    use std::collections::HashMap;

    if dt <= 0.0 {
        return;
    }

    /// What a fighter's move is worth on the frame it connects.
    struct Attack {
        move_name: String,
        damage: f32,
        hitstun: u32,
        hitstop: u32,
    }

    // ── 1. Every fighter, and which of them are swinging this frame ──────────────────────────
    let mut is_fighter: HashMap<u32, ()> = HashMap::new();
    let mut attacks: HashMap<u32, Attack> = HashMap::new();
    let mut already_hit: HashMap<u32, Vec<u32>> = HashMap::new();
    {
        let controllers = world.borrow::<FighterController>();
        for (id, fighter) in controllers.iter() {
            is_fighter.insert(id, ());
            already_hit.insert(id, fighter.already_hit.clone());
            if let Some(active) = &fighter.active_move {
                if fighter.is_in_active_window() {
                    attacks.insert(
                        id,
                        Attack {
                            move_name: active.name.clone(),
                            damage: active.frame_data.damage,
                            hitstun: active.frame_data.hitstun,
                            hitstop: active.frame_data.hitstop,
                        },
                    );
                }
            }
        }
    }
    if is_fighter.is_empty() {
        return;
    }

    let transforms = world.borrow::<Transform>();
    let parents = world.borrow::<Parent>();

    // Composed local→world pose, walking the parent chain. Depth-guarded: a `Children`/`Parent`
    // cycle is refused elsewhere, but a system that walks links must not be the one that hangs.
    let world_pose = |start: u32| -> Option<(gizmo_math::Vec3, gizmo_math::Quat)> {
        let mut chain = Vec::new();
        let mut cur = start;
        for _ in 0..32 {
            chain.push(cur);
            match parents.get(cur) {
                Some(p) => cur = p.0,
                None => break,
            }
        }
        let mut pos = gizmo_math::Vec3::ZERO;
        let mut rot = gizmo_math::Quat::IDENTITY;
        for &e in chain.iter().rev() {
            let t = transforms.get(e)?;
            pos += rot.mul_vec3(t.position);
            rot *= t.rotation;
        }
        Some((pos, rot))
    };

    // The nearest fighter at or above `start`, if any.
    let owning_fighter = |start: u32| -> Option<u32> {
        let mut cur = start;
        for _ in 0..32 {
            if is_fighter.contains_key(&cur) {
                return Some(cur);
            }
            cur = parents.get(cur)?.0;
        }
        None
    };

    // ── 2. Drive `active`, and collect the boxes that are live ───────────────────────────────
    struct LiveBox {
        entity: u32,
        owner: u32,
        hitbox: Hitbox,
        pos: gizmo_math::Vec3,
        rot: gizmo_math::Quat,
    }
    let mut live: Vec<LiveBox> = Vec::new();
    {
        // SAFETY: exclusive `fn(&World, f32)` system — the scheduler runs it alone — and `Hitbox`
        // is a different component type from the `Transform`/`Parent` queries borrowed above.
        let mut hitboxes = unsafe { world.borrow_mut_unchecked::<Hitbox>() };
        for (entity, mut hitbox) in hitboxes.iter_mut() {
            let Some(owner) = owning_fighter(entity) else {
                continue; // not a fighter's box — its `active` is somebody else's business
            };
            let swinging = attacks.get(&owner).is_some_and(|attack| {
                hitbox
                    .move_name
                    .as_ref()
                    .is_none_or(|name| *name == attack.move_name)
            });
            hitbox.active = swinging;
            if swinging {
                if let Some((pos, rot)) = world_pose(entity) {
                    live.push(LiveBox {
                        entity,
                        owner,
                        hitbox: hitbox.clone(),
                        pos,
                        rot,
                    });
                }
            }
        }
    }
    if live.is_empty() {
        return;
    }

    // ── 3. Test them against every hurtbox that is not the attacker's own ────────────────────
    let mut hits: Vec<HitEvent> = Vec::new();
    {
        let hurtboxes = world.borrow::<Hurtbox>();
        for (victim_entity, hurtbox) in hurtboxes.iter() {
            let victim_owner = owning_fighter(victim_entity);
            let Some((u_pos, u_rot)) = world_pose(victim_entity) else {
                continue;
            };
            for attacker_box in &live {
                if victim_owner == Some(attacker_box.owner) || victim_entity == attacker_box.owner {
                    continue; // your own hurtboxes are not targets
                }
                let victim = victim_owner.unwrap_or(victim_entity);
                if already_hit
                    .get(&attacker_box.owner)
                    .is_some_and(|hit| hit.contains(&victim))
                {
                    continue; // this move already connected with them
                }
                if !hit_volumes_overlap(
                    &attacker_box.hitbox,
                    attacker_box.pos,
                    attacker_box.rot,
                    hurtbox,
                    u_pos,
                    u_rot,
                ) {
                    continue;
                }

                let attack = &attacks[&attacker_box.owner];
                already_hit
                    .entry(attacker_box.owner)
                    .or_default()
                    .push(victim);
                hits.push(HitEvent {
                    attacker: attacker_box.owner,
                    attacker_hitbox: attacker_box.entity,
                    victim,
                    victim_hurtbox: victim_entity,
                    damage: attack.damage * hurtbox.damage_multiplier,
                    hitstun: attack.hitstun,
                    hitstop: attack.hitstop,
                    move_name: attack.move_name.clone(),
                });
            }
        }
    }
    if hits.is_empty() {
        return;
    }

    // ── 4. Write the record back, then report ────────────────────────────────────────────────
    drop(transforms);
    drop(parents);
    {
        // SAFETY: as above — the `Transform`/`Parent` queries are dropped, and `FighterController`
        // is not borrowed anywhere else in this exclusive system.
        let mut controllers = unsafe { world.borrow_mut_unchecked::<FighterController>() };
        for hit in &hits {
            if let Some(mut fighter) = controllers.get_mut(hit.attacker) {
                if !fighter.already_hit.contains(&hit.victim) {
                    fighter.already_hit.push(hit.victim);
                }
            }
        }
    }

    let hit_count = hits.len();
    if let Ok(mut queue) = world.try_get_resource_mut::<gizmo_core::event::Events<HitEvent>>() {
        for hit in hits {
            queue.send(hit);
        }
    }
    tracing::debug!(hit_count, "[Fight] hits resolved");
}

#[cfg(test)]
mod tests {
    use super::{character_controller_system, gather_colliders, vehicle_controller_system};
    use crate::vehicle::{
        update_vehicle, update_vehicle_with_query, Axle, VehicleController, Wheel,
    };
    use gizmo_core::entity::Entity;
    use gizmo_core::system::{Phase, Schedule, SystemConfig};
    use gizmo_core::world::World;
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::CharacterController;
    use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_rigid::physics_step_system;
    use gizmo_physics_rigid::world::PhysicsWorld;

    fn box_collider(hx: f32, hy: f32, hz: f32) -> Collider {
        Collider::from_shape(ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(hx, hy, hz),
        }))
    }

    /// A large static floor whose top surface sits at y = 0.
    fn spawn_ground(world: &mut World) -> Entity {
        let e = world.spawn();
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, box_collider(100.0, 1.0, 100.0));
        world.add_component(e, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
        world.add_component(e, Velocity::default());
        e
    }

    /// A four-wheeled rear-wheel-drive vehicle sized so its wheels rest on a
    /// floor whose top is at y = 0.
    fn make_vehicle() -> VehicleController {
        let mut vc = VehicleController::new();
        let corners = [
            (-0.8, -1.4, Axle::Front, true),
            (0.8, -1.4, Axle::Front, false),
            (-0.8, 1.4, Axle::Rear, true),
            (0.8, 1.4, Axle::Rear, false),
        ];
        for (x, z, axle, is_left) in corners {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.0, z),
                axle_type: axle,
                is_left,
                radius: 0.35,
                suspension_rest_length: 0.4,
                suspension_max_travel: 0.3,
                ..Default::default()
            });
        }
        vc
    }

    fn spawn_vehicle(world: &mut World, throttle: f32) -> Entity {
        let mut rb = RigidBody::new(900.0, true);
        let collider = box_collider(0.9, 0.3, 2.0);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();

        let mut vc = make_vehicle();
        vc.throttle_input = throttle;

        let e = world.spawn();
        world.add_component(e, rb);
        world.add_component(e, collider);
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
        world.add_component(e, Velocity::default());
        world.add_component(e, vc);
        e
    }

    /// Forward is `-Z` (see `update_vehicle`): applying throttle over several
    /// fixed steps must move the chassis in `-Z`. Drives the system function
    /// directly (interleaved with the rigid step that integrates the forces).
    #[test]
    fn vehicle_drives_forward_when_called_directly() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);

        let start_z = world
            .query::<&Transform>()
            .unwrap()
            .get(car.id())
            .unwrap()
            .position
            .z;

        let dt = 1.0 / 120.0;
        for _ in 0..300 {
            vehicle_controller_system(&world, dt);
            physics_step_system(&world, dt);
        }

        let end = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        assert!(end.position.is_finite(), "vehicle position went non-finite: {:?}", end.position);
        assert!(
            start_z - end.position.z > 0.2,
            "vehicle should drive forward (-Z): start_z {start_z} -> end_z {} (Δ {})",
            end.position.z,
            start_z - end.position.z
        );
    }

    /// Same scenario, but the systems are wired into a `Schedule` (vehicle
    /// controller `.before("physics_step_system")` in `Phase::Physics`).
    #[test]
    fn vehicle_drives_forward_via_schedule() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);

        let start_z = world
            .query::<&Transform>()
            .unwrap()
            .get(car.id())
            .unwrap()
            .position
            .z;

        let mut schedule = Schedule::new();
        schedule.add_di_system(
            SystemConfig::new(Box::new(vehicle_controller_system))
                .in_phase(Phase::Physics)
                .label("vehicle_controller_system")
                .before("physics_step_system"),
        );
        schedule.add_di_system(
            SystemConfig::new(Box::new(physics_step_system))
                .in_phase(Phase::Physics)
                .label("physics_step_system"),
        );

        let dt = 1.0 / 120.0;
        for _ in 0..300 {
            schedule.run(&mut world, dt);
        }

        let end = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        assert!(end.position.is_finite());
        assert!(
            start_z - end.position.z > 0.2,
            "vehicle (schedule) should drive forward (-Z): Δz {}",
            start_z - end.position.z
        );
    }

    /// A KCC with an `+X` target velocity walks forward on flat ground. The KCC
    /// does its own integration, so no `RigidBody` / physics step is involved.
    #[test]
    fn character_walks_forward_on_flat_ground() {
        let mut world = World::new();
        spawn_ground(&mut world);

        let kcc = CharacterController {
            target_velocity: Vec3::new(2.0, 0.0, 0.0),
            is_grounded: true,
            ..Default::default()
        };
        let e = world.spawn();
        world.add_component(e, kcc);
        world.add_component(e, Collider::capsule(0.3, 0.6));
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.9, 0.0)));
        world.add_component(e, Velocity::default());

        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            character_controller_system(&world, dt);
        }

        let pos = world.query::<&Transform>().unwrap().get(e.id()).unwrap().position;
        assert!(pos.is_finite(), "character position went non-finite: {pos:?}");
        assert!(pos.x > 0.5, "character should have walked forward in +X, got x = {}", pos.x);
    }

    /// A KCC walking into a low ledge (< step_height) climbs onto it: its `y`
    /// rises. Exercises the step/slide path through the ECS system wrapper.
    #[test]
    fn character_steps_up_a_low_ledge() {
        let mut world = World::new();
        // Flat ground (top at y = 0).
        let g = world.spawn();
        world.add_component(g, box_collider(50.0, 0.5, 50.0));
        world.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
        // A 0.15-high step whose vertical face is at x = 1.0.
        let s = world.spawn();
        world.add_component(s, box_collider(25.0, 0.075, 50.0));
        world.add_component(s, Transform::new(Vec3::new(26.0, 0.075, 0.0)));

        // Thin character so the low sweep ray can hit the short step.
        let kcc = CharacterController {
            target_velocity: Vec3::new(2.0, 0.0, 0.0),
            is_grounded: true,
            step_height: 0.3,
            ..Default::default()
        };
        let e = world.spawn();
        world.add_component(e, kcc);
        world.add_component(e, box_collider(0.1, 0.9, 0.1));
        world.add_component(e, Transform::new(Vec3::new(0.88, 0.9, 0.0)));
        world.add_component(e, Velocity::default());

        let dt = 0.0125;
        let start_y = 0.9;
        for _ in 0..40 {
            character_controller_system(&world, dt);
        }

        let pos = world.query::<&Transform>().unwrap().get(e.id()).unwrap().position;
        assert!(pos.is_finite());
        assert!(
            pos.y > start_y + 0.05,
            "character should have stepped up onto the ledge, y {start_y} -> {}",
            pos.y
        );
        assert!(pos.x > 1.0, "character should have advanced past the step face, x = {}", pos.x);
    }

    /// Determinism guard: on a scene with no vehicle/character components, both
    /// gameplay systems are strict no-ops, so a plain rigid-body simulation
    /// evolves bit-identically whether or not they run each step (the
    /// determinism oracle scene relies on this).
    #[test]
    fn gameplay_systems_are_noop_without_components() {
        fn run(with_gameplay: bool) -> Vec3 {
            let mut world = World::new();
            world.insert_resource(PhysicsWorld::new());
            // static floor
            let g = world.spawn();
            world.add_component(g, RigidBody::new_static());
            world.add_component(g, box_collider(50.0, 0.5, 50.0));
            world.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
            world.add_component(g, Velocity::default());
            // a plain falling dynamic box
            let mut rb = RigidBody::new(1.0, true);
            let c = box_collider(0.5, 0.5, 0.5);
            rb.update_inertia_from_collider(&c);
            rb.wake_up();
            let b = world.spawn();
            world.add_component(b, rb);
            world.add_component(b, c);
            world.add_component(b, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
            world.add_component(b, Velocity::default());

            let dt = 1.0 / 120.0;
            for _ in 0..120 {
                if with_gameplay {
                    vehicle_controller_system(&world, dt);
                    character_controller_system(&world, dt);
                }
                physics_step_system(&world, dt);
            }
            world.query::<&Transform>().unwrap().get(b.id()).unwrap().position
        }

        let baseline = run(false);
        let with_systems = run(true);
        assert_eq!(
            baseline, with_systems,
            "gameplay systems must not perturb a plain rigid-body scene"
        );
    }

    /// Track C wiring: vehicle_controller_system reads PhysicsWorld.weather and applies it to
    /// grip. Snow (weather_grip 0.3) must cover markedly less ground than sunny, which exercises
    /// the read-weather → wg → grip chain end to end (every earlier test was Sunny only).
    #[test]
    fn snow_weather_reduces_travel_vs_sunny() {
        use gizmo_physics_rigid::world::Weather;
        fn run(weather: Weather) -> f32 {
            let mut world = World::new();
            let mut pw = PhysicsWorld::new();
            pw.weather = weather;
            world.insert_resource(pw);
            spawn_ground(&mut world);
            let car = spawn_vehicle(&mut world, 1.0);
            let start_z = world.query::<&Transform>().unwrap().get(car.id()).unwrap().position.z;
            let dt = 1.0 / 120.0;
            for _ in 0..300 {
                vehicle_controller_system(&world, dt);
                physics_step_system(&world, dt);
            }
            let end_z = world.query::<&Transform>().unwrap().get(car.id()).unwrap().position.z;
            start_z - end_z // ileri (-Z) kat edilen mesafe
        }
        let sunny = run(Weather::Sunny);
        let snow = run(Weather::Snow);
        assert!(sunny > 0.2, "sunny'de araç ilerlemeli, Δ {sunny}");
        assert!(
            snow < sunny * 0.75,
            "kar hava PhysicsWorld'den okunup grip'i düşürmeli: sunny Δ{sunny:.2} vs snow Δ{snow:.2}"
        );
    }

    /// Track C wiring: with NO PhysicsWorld resource, weather is unwrap_or_default() = Sunny →
    /// no panic and the vehicle drives normally (the fallback path).
    #[test]
    fn vehicle_controller_system_ok_without_physics_world_resource() {
        let mut world = World::new();
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);
        let dt = 1.0 / 120.0;
        for _ in 0..30 {
            vehicle_controller_system(&world, dt); // yalnız kontrolcü; kuvvetleri Velocity'ye yazar
        }
        let v = world.query::<&Velocity>().unwrap().get(car.id()).unwrap().linear;
        assert!(v.is_finite(), "PhysicsWorld'süz kontrolcü sonlu kalmalı, bulundu {v:?}");
    }

    // ================================================================================
    // Broadphase-backed suspension rays (0.10). See `vehicle_controller_system`'s docs
    // for the behaviour contract these pin.
    // ================================================================================

    /// Per-wheel `is_grounded`, read out of the ECS.
    fn grounded_flags(world: &World, car: Entity) -> Vec<bool> {
        let q = world.query::<&VehicleController>().unwrap();
        q.get(car.id())
            .unwrap()
            .wheels
            .iter()
            .map(|w| w.is_grounded)
            .collect()
    }

    /// A `Transform` + `Collider` entity with **no** `RigidBody`: present in the ECS, never
    /// synced into the `PhysicsWorld`, and therefore intangible to the solver.
    fn spawn_ecs_only_ground(world: &mut World) -> Entity {
        let e = world.spawn();
        world.add_component(e, box_collider(100.0, 1.0, 100.0));
        world.add_component(e, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
        e
    }

    /// **The regression test for this change.** The wheel rays go through the
    /// `PhysicsWorld` broadphase, so they see the bodies the solver simulates — and *only*
    /// those. A `Collider`-only floor is not one of them.
    ///
    /// Red before the fix: the old system cloned every `(Transform, Collider)` entity in the
    /// ECS into a slice and scanned it, so all four wheels parked happily on a floor the
    /// chassis would have fallen straight through.
    #[test]
    fn wheels_ignore_ecs_only_colliders_the_solver_never_sees() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ecs_only_ground(&mut world);
        let car = spawn_vehicle(&mut world, 0.0);

        let dt = 1.0 / 120.0;
        // One step so the physics world is populated (and so the pre-first-step fallback is
        // not what answers). Only the chassis is a rigid body, so only it gets synced.
        physics_step_system(&world, dt);
        assert_eq!(
            world.get_resource::<PhysicsWorld>().unwrap().entities.len(),
            1,
            "only the chassis has a RigidBody, so only it reaches the PhysicsWorld"
        );

        vehicle_controller_system(&world, dt);

        assert_eq!(
            grounded_flags(&world, car),
            vec![false; 4],
            "a Collider-only floor is invisible to the solver, so it must be invisible to \
             the wheels too — see vehicle_controller_system's docs"
        );
    }

    /// The control for the test above: the identical scene with a `RigidBody` on the floor
    /// grounds all four wheels. Without this, "no wheel is grounded" could just mean the ray
    /// never reached that far.
    #[test]
    fn wheels_rest_on_rigid_body_ground_through_the_broadphase() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world); // same geometry, plus RigidBody::new_static() + Velocity
        let car = spawn_vehicle(&mut world, 0.0);

        let dt = 1.0 / 120.0;
        physics_step_system(&world, dt);
        assert_eq!(
            world.get_resource::<PhysicsWorld>().unwrap().entities.len(),
            2,
            "floor + chassis"
        );

        vehicle_controller_system(&world, dt);

        assert_eq!(grounded_flags(&world, car), vec![true; 4]);
    }

    /// Before the world's first `physics_step_system` the `PhysicsWorld` holds no bodies at
    /// all, so the system falls back to the ECS scan for that one step. "Spawn a car, read
    /// `is_grounded`, then start stepping" therefore behaves exactly as it did.
    #[test]
    fn first_step_falls_back_to_the_ecs_scan() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 0.0);

        assert!(
            world.get_resource::<PhysicsWorld>().unwrap().entities.is_empty(),
            "nothing has been stepped yet"
        );
        vehicle_controller_system(&world, 1.0 / 120.0);

        assert_eq!(
            grounded_flags(&world, car),
            vec![true; 4],
            "the pre-first-step fallback must still find the ground"
        );
    }

    /// Equivalence: on a scene both paths can see in full — every collider belongs to a rigid
    /// body — the broadphase query and the old linear scan produce **bit-identical** vehicle
    /// state and forces. This is what makes the switch safe for every existing scene.
    #[test]
    fn broadphase_and_legacy_scan_produce_identical_vehicle_state() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        // A kerb to one side and a distant block, so the ray has more than one candidate and
        // the broadphase has something to reject.
        for (x, z, hy) in [(1.6f32, 0.0f32, 0.06f32), (40.0, 40.0, 2.0)] {
            let e = world.spawn();
            world.add_component(e, RigidBody::new_static());
            world.add_component(e, box_collider(0.4, hy, 4.0));
            world.add_component(e, Transform::new(Vec3::new(x, hy, z)));
            world.add_component(e, Velocity::default());
        }
        let car = spawn_vehicle(&mut world, 1.0);

        let dt = 1.0 / 120.0;
        for _ in 0..20 {
            vehicle_controller_system(&world, dt);
            physics_step_system(&world, dt);
        }

        // Freeze the inputs both paths will be handed.
        let transform = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        let rb0 = *world.query::<&RigidBody>().unwrap().get(car.id()).unwrap();
        let vel0 = *world.query::<&Velocity>().unwrap().get(car.id()).unwrap();
        let handle = gizmo_physics_core::BodyHandle::from_id(car.id());

        let colliders = gather_colliders(&world);
        let phys = world.get_resource::<PhysicsWorld>().unwrap();
        assert_eq!(
            colliders.len(),
            phys.entities.len(),
            "this scene must be fully visible to both paths for the comparison to mean anything"
        );

        let mut legacy = (make_vehicle(), rb0, vel0);
        let mut queried = (make_vehicle(), rb0, vel0);
        for v in [&mut legacy.0, &mut queried.0] {
            v.throttle_input = 0.7;
            v.steering_input = 0.35;
        }

        update_vehicle(
            handle, &mut legacy.0, &mut legacy.1, &transform, &mut legacy.2, &colliders, 1.0, dt,
        );
        update_vehicle_with_query(
            handle, &mut queried.0, &mut queried.1, &transform, &mut queried.2, &*phys, 1.0, dt,
        );

        for (i, (a, b)) in legacy.0.wheels.iter().zip(queried.0.wheels.iter()).enumerate() {
            assert_eq!(a.is_grounded, b.is_grounded, "wheel {i} is_grounded");
            assert_eq!(a.suspension_length, b.suspension_length, "wheel {i} suspension_length");
            assert_eq!(a.suspension_force, b.suspension_force, "wheel {i} suspension_force");
            assert_eq!(a.surface_friction, b.surface_friction, "wheel {i} surface_friction");
            assert_eq!(a.angular_velocity, b.angular_velocity, "wheel {i} angular_velocity");
            let (ha, hb) = (a.ground_hit.as_ref(), b.ground_hit.as_ref());
            assert_eq!(ha.map(|h| h.entity), hb.map(|h| h.entity), "wheel {i} hit entity");
            assert_eq!(ha.map(|h| h.distance), hb.map(|h| h.distance), "wheel {i} hit distance");
            assert_eq!(ha.map(|h| h.normal), hb.map(|h| h.normal), "wheel {i} hit normal");
        }
        assert_eq!(legacy.2.linear, queried.2.linear, "resulting linear velocity");
        assert_eq!(legacy.2.angular, queried.2.angular, "resulting angular velocity");
        assert!(
            legacy.0.wheels.iter().any(|w| w.is_grounded),
            "the comparison is vacuous if no wheel touched anything"
        );
    }

    /// The measurement behind the change. `#[ignore]`d — it is a cost report, not a gate.
    ///
    /// ```text
    /// cargo test -p gizmo-physics-dynamics --release wheel_query_cost -- --ignored --nocapture
    /// ```
    ///
    /// Reports, for a scene of a few thousand static bodies:
    ///   * `Collider` clones per step — the `gather_colliders` snapshot the old path rebuilt
    ///     every step, and which the new path does not build at all;
    ///   * colliders *visited* per step — 4 wheels × every body, versus the broadphase
    ///     candidate set the BVH hands back for the same four rays;
    ///   * wall time for `vehicle_controller_system` over N steps, both ways.
    ///
    /// Measured 2026-08-09, `--release`, 4 098 colliders, one vehicle:
    ///
    /// ```text
    /// Collider clones per step   : linear scan    4098   broadphase       0
    /// colliders visited per step : linear scan   16392   broadphase       8  (4 rays)
    /// vehicle_controller_system  : linear scan   0.434 ms/step   broadphase 0.002 ms/step  (212x)
    /// ```
    #[test]
    #[ignore = "measurement, not a gate — run with --ignored --nocapture"]
    fn wheel_query_cost_scan_vs_broadphase() {
        use std::time::Instant;

        const BODIES: usize = 4_000;
        const STEPS: usize = 100;

        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        // A grid of static blocks well away from the car, standing in for city geometry.
        let side = (BODIES as f32).sqrt().ceil() as i32;
        for i in 0..side {
            for j in 0..side {
                let e = world.spawn();
                world.add_component(e, RigidBody::new_static());
                world.add_component(e, box_collider(0.4, 0.4, 0.4));
                world.add_component(
                    e,
                    Transform::new(Vec3::new(20.0 + i as f32 * 3.0, 0.4, 20.0 + j as f32 * 3.0)),
                );
                world.add_component(e, Velocity::default());
            }
        }
        let car = spawn_vehicle(&mut world, 1.0);

        let dt = 1.0 / 120.0;
        physics_step_system(&world, dt); // populate the PhysicsWorld

        let n_colliders = gather_colliders(&world).len();

        // --- counts -----------------------------------------------------------------
        // Reconstruct the four suspension rays and ask the broadphase what it would return.
        let (candidates, ray_count) = {
            let transform = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
            let q = world.query::<&VehicleController>().unwrap();
            let vc = q.get(car.id()).unwrap();
            let phys = world.get_resource::<PhysicsWorld>().unwrap();
            let mut total = 0usize;
            for w in &vc.wheels {
                let attach = transform.position + transform.rotation.mul_vec3(w.attachment_local_pos);
                let dir = transform.rotation.mul_vec3(w.direction_local).normalize();
                let start = attach - dir * 0.5;
                let max = w.suspension_rest_length + w.radius + w.suspension_max_travel + 0.5;
                total += phys.spatial_hash.query_ray(start, dir, max).len();
            }
            (total, vc.wheels.len())
        };

        // --- timing -----------------------------------------------------------------
        let t0 = Instant::now();
        for _ in 0..STEPS {
            vehicle_controller_system(&world, dt);
        }
        let broadphase_time = t0.elapsed();

        // The old path, reproduced exactly: rebuild the owned snapshot every step, then scan
        // it once per wheel.
        let transform = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        let rb0 = *world.query::<&RigidBody>().unwrap().get(car.id()).unwrap();
        let vel0 = *world.query::<&Velocity>().unwrap().get(car.id()).unwrap();
        let handle = gizmo_physics_core::BodyHandle::from_id(car.id());
        let t0 = Instant::now();
        for _ in 0..STEPS {
            let colliders = gather_colliders(&world);
            let (mut vc, mut rb, mut vel) = (make_vehicle(), rb0, vel0);
            vc.throttle_input = 1.0;
            update_vehicle(handle, &mut vc, &mut rb, &transform, &mut vel, &colliders, 1.0, dt);
        }
        let scan_time = t0.elapsed();

        println!("\n--- wheel suspension query, {n_colliders} colliders, {STEPS} steps ---");
        println!(
            "Collider clones per step   : linear scan {n_colliders:>7}   broadphase       0"
        );
        println!(
            "colliders visited per step : linear scan {:>7}   broadphase {candidates:>7}  ({ray_count} rays)",
            n_colliders * ray_count
        );
        println!(
            "vehicle_controller_system  : linear scan {:>7.3} ms/step   broadphase {:>7.3} ms/step   speedup {:.1}x",
            scan_time.as_secs_f64() * 1e3 / STEPS as f64,
            broadphase_time.as_secs_f64() * 1e3 / STEPS as f64,
            scan_time.as_secs_f64() / broadphase_time.as_secs_f64().max(f64::EPSILON),
        );

        assert!(
            candidates * 50 < n_colliders * ray_count,
            "the broadphase should visit orders of magnitude fewer colliders: \
             {candidates} vs {}",
            n_colliders * ray_count
        );
    }

    // ── Hit detection ───────────────────────────────────────────────────────────────────────

    use super::{fighter_frame_system, hit_detection_system};
    use gizmo_core::component::Parent;
    use gizmo_core::event::Events;
    use gizmo_physics_core::components::fighter::{CombatMove, FighterController, FrameData};
    use gizmo_physics_core::components::{HitEvent, Hitbox, Hurtbox};

    const DT: f32 = 1.0 / 60.0;

    /// A 2/2/1 move: five frames long, hitting on the third and fourth.
    fn jab() -> CombatMove {
        let mut fd = FrameData::default();
        fd.startup = 2;
        fd.active = 2;
        fd.recovery = 1;
        fd.damage = 8.0;
        fd.hitstun = 20;
        fd.hitstop = 5;
        let mut m = CombatMove::default();
        m.name = "Jab".to_string();
        m.frame_data = fd;
        m
    }

    /// Two fighters a metre apart, the attacker's fist parented to it and reaching the defender's
    /// torso. Returns `(world, attacker, fist, defender)`.
    fn versus() -> (World, u32, u32, u32) {
        let mut world = World::new();
        world.insert_resource(Events::<HitEvent>::new());

        let attacker = world.spawn();
        world.add_component(attacker, Transform::new(Vec3::ZERO));
        world.add_component(attacker, FighterController::new(1));

        // The fist: a child, so the pose has to be composed through the parent link.
        let fist = world.spawn();
        world.add_component(fist, Transform::new(Vec3::new(0.0, 0.0, -0.8)));
        world.add_component(fist, Parent(attacker.id()));
        world.add_component(fist, Hitbox::new(Vec3::splat(0.2), 99.0));

        let defender = world.spawn();
        world.add_component(defender, Transform::new(Vec3::new(0.0, 0.0, -1.0)));
        world.add_component(defender, FighterController::new(2));
        world.add_component(defender, Hurtbox::new(Vec3::new(0.3, 0.5, 0.3)));

        (world, attacker.id(), fist.id(), defender.id())
    }

    /// Runs one fixed frame of the fight: clock, then hit detection, then the event swap an app
    /// does at the end of a frame. Returns what was reported this frame.
    fn fight_frame(world: &mut World) -> Vec<HitEvent> {
        fighter_frame_system(world, DT);
        hit_detection_system(world, DT);
        let mut queue = world
            .get_resource_mut::<Events<HitEvent>>()
            .expect("the queue is a resource");
        queue.update();
        queue.iter().cloned().collect()
    }

    fn start(world: &mut World, fighter: u32, m: CombatMove) {
        let mut controllers = world.borrow_mut::<FighterController>();
        controllers
            .get_mut(fighter)
            .expect("fighter")
            .start_move(m);
    }

    /// **A move connects once, on the frames it is meant to.**
    ///
    /// The whole chain in one test: the clock advances the move, the active window drives the
    /// fist's `Hitbox::active`, the boxes overlap, and one `HitEvent` comes out — one, not one per
    /// active frame, and not on the startup or recovery frames.
    #[test]
    fn a_move_reports_one_hit_on_its_active_frames() {
        let (mut world, attacker, _fist, _defender) = versus();
        start(&mut world, attacker, jab());

        let mut reported: Vec<(u32, usize)> = Vec::new();
        for frame in 1..=6u32 {
            let hits = fight_frame(&mut world);
            if !hits.is_empty() {
                reported.push((frame, hits.len()));
            }
        }

        assert_eq!(
            reported,
            vec![(2, 1)],
            "a 2/2/1 jab must connect exactly once, on the first frame of its active window — \
             two entries here is a hit reported once per active frame"
        );

        let hit = {
            // Re-run to inspect the payload: same setup, stopped on the frame that connects.
            let (mut world, attacker, fist, defender) = versus();
            start(&mut world, attacker, jab());
            fight_frame(&mut world);
            let hits = fight_frame(&mut world);
            assert_eq!(hits.len(), 1);
            let hit = hits[0].clone();
            assert_eq!((hit.attacker, hit.attacker_hitbox), (attacker, fist));
            assert_eq!((hit.victim, hit.victim_hurtbox), (defender, defender));
            hit
        };
        assert_eq!(hit.move_name, "Jab");
        assert_eq!(
            hit.damage, 8.0,
            "the MOVE's damage scaled by the region's multiplier — not the hitbox's own 99, which \
             is for a hit volume with no fighter behind it"
        );
        assert_eq!((hit.hitstun, hit.hitstop), (20, 5));
    }

    /// A weak point multiplies the move's damage; the engine reports the product and applies
    /// nothing.
    #[test]
    fn the_hurtbox_multiplier_scales_the_reported_damage() {
        let (mut world, attacker, _fist, defender) = versus();
        {
            let mut hurtboxes = world.borrow_mut::<Hurtbox>();
            hurtboxes.get_mut(defender).expect("hurtbox").damage_multiplier = 1.5;
        }
        start(&mut world, attacker, jab());

        fight_frame(&mut world);
        let hits = fight_frame(&mut world);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].damage, 12.0, "8 damage on a 1.5× weak point");

        // And the health is untouched: reporting is not applying.
        let health = world
            .borrow::<FighterController>()
            .get(defender)
            .expect("defender")
            .health;
        assert_eq!(health, 100.0, "the engine reports hits, the game spends them");
    }

    /// **A hitbox tagged with a move's name goes live only for that move**, which is what keeps a
    /// fighter's jab box out of its kick. An untagged box belongs to every move.
    #[test]
    fn a_tagged_hitbox_only_goes_live_for_its_own_move() {
        let (mut world, attacker, fist, _defender) = versus();
        {
            let mut hitboxes = world.borrow_mut::<Hitbox>();
            hitboxes.get_mut(fist).expect("fist").move_name = Some("Roundhouse".to_string());
        }
        start(&mut world, attacker, jab());

        let mut hits = 0;
        for _ in 0..6 {
            hits += fight_frame(&mut world).len();
        }
        assert_eq!(hits, 0, "the jab must not swing the kick's hitbox");

        // The same box, with the move it belongs to, does connect.
        let mut roundhouse = jab();
        roundhouse.name = "Roundhouse".to_string();
        start(&mut world, attacker, roundhouse);
        let mut hits = 0;
        for _ in 0..6 {
            hits += fight_frame(&mut world).len();
        }
        assert_eq!(hits, 1, "its own move swings it");
    }

    /// Nobody punches themselves: a hurtbox under the attacker is not a target.
    #[test]
    fn a_fighter_cannot_hit_its_own_hurtbox() {
        let (mut world, attacker, _fist, defender) = versus();
        // Give the attacker a hurtbox of its own, right where its fist is.
        {
            let e = world.get_entity(attacker).expect("attacker");
            world.add_component(e, Hurtbox::new(Vec3::splat(0.6)));
        }
        // And move the defender out of reach so only the self-hit could report.
        {
            let mut transforms = world.borrow_mut::<Transform>();
            transforms.get_mut(defender).expect("defender").position = Vec3::new(0.0, 0.0, -50.0);
        }
        start(&mut world, attacker, jab());

        let mut hits = 0;
        for _ in 0..6 {
            hits += fight_frame(&mut world).len();
        }
        assert_eq!(hits, 0, "the attacker's own hurtbox is not a target");
    }

    /// A second move connects again — the once-per-move record is cleared when the move ends, not
    /// kept for the life of the fighter.
    #[test]
    fn the_next_move_can_hit_the_same_victim_again() {
        let (mut world, attacker, _fist, _defender) = versus();

        let mut total = 0;
        for _ in 0..3 {
            start(&mut world, attacker, jab());
            for _ in 0..6 {
                total += fight_frame(&mut world).len();
            }
        }
        assert_eq!(total, 3, "three jabs, three hits");
    }
}
