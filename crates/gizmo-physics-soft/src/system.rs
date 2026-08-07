use crate::coupling::{RigidImpulse, RigidReaction};
use crate::{cloth::Cloth, rope::Rope, SoftBodyMesh};
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};

/// Soft-body integratörleri büyük `dt`'de (kare sıçraması/hitch) patlar (FEM
/// kararlılık sınırı + XPBD aşırı-tahmin). Adımı bu üst sınıra kırp.
const MAX_SOFT_DT: f32 = 1.0 / 30.0;

/// Steps every [`SoftBodyMesh`] in the world by `dt` seconds under `gravity`,
/// resolving collisions against all rigid colliders present in the world.
///
/// The timestep is clamped to a stable upper bound to keep the FEM integrator
/// stable across frame hitches.
///
/// # Two-way coupling
///
/// With the `rigid-coupling` feature (on by default) the exchange runs **both ways**: the
/// mass properties and velocities of the rigid bodies behind the colliders are read first
/// (`collect_rigid_reactions`), so each node solves a genuine two-body impulse instead of
/// bouncing off an assumed-immovable wall, and the equal-and-opposite half is then spent on
/// the bodies (`apply_reaction_impulses`). Static and kinematic bodies are never pushed; a
/// sleeping dynamic body that *is* pushed is woken, or the integrator would skip it and
/// swallow the impulse.
///
/// The impulse lands in `Velocity` and is integrated by the **next** `physics_step_system`,
/// because the facade runs the rigid step before this one. That one-frame latency is the same
/// one every other velocity write between steps has.
///
/// Without the feature the driver is byte-for-byte the old one-way version: the reaction
/// table is empty, every collider is immovable and the impulses are discarded.
#[tracing::instrument(skip_all, name = "soft_body_step_system")]
pub fn soft_body_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    // 1. Collect all rigid colliders for collision resolution
    let rigid_colliders = collect_rigid_colliders(world);

    // 2. …and what the bodies behind them are made of, so the response can be two-sided.
    #[cfg(feature = "rigid-coupling")]
    let reactions = collect_rigid_reactions(world);
    #[cfg(not(feature = "rigid-coupling"))]
    let reactions: Vec<(BodyHandle, RigidReaction)> = Vec::new();

    // 3. Query and step all SoftBodyMesh components
    let mut impulses: Vec<RigidImpulse> = Vec::new();
    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<SoftBodyMesh>>() }
    {
        for (_, mut soft_body) in q.iter_mut() {
            soft_body.step_coupled(dt, gravity, &rigid_colliders, &reactions, &mut impulses);
        }
    }

    // 4. Pay the rigid bodies back. This is the half that did not exist: the response
    //    computed an impulse and threw it away, so a soft body could not shove a light crate.
    #[cfg(feature = "rigid-coupling")]
    apply_reaction_impulses(world, &impulses);
    #[cfg(not(feature = "rigid-coupling"))]
    debug_assert!(
        impulses.is_empty(),
        "an empty reaction table can only produce immovable contacts"
    );
}

/// Every entity carrying both a `Transform` and a `Collider`, as the soft solvers want them:
/// the same list `soft_body_step_system` and `cloth_step_system` both sweep against.
///
/// Panik koruması: entity sorgu ile `get_entity` arasında despawn edilmiş olabilir
/// (yarış/tutarsızlık) → unwrap yerine bu collider sessizce atlanır.
fn collect_rigid_colliders(world: &World) -> Vec<(BodyHandle, Transform, Collider)> {
    let mut rigid_colliders = Vec::new();
    if let Some(q) = world.query::<(&Transform, &Collider)>() {
        for (e, (trans, col)) in q.iter() {
            if let Some(entity) = world.get_entity(e) {
                // Bridge: ECS entity -> opaque physics BodyHandle (id only).
                rigid_colliders.push((BodyHandle::from_id(entity.id()), *trans, col.clone()));
            }
        }
    }
    rigid_colliders
}

/// Reads the mass properties and current velocity of every rigid body that a soft-body node
/// could actually push, keyed by the same [`BodyHandle`] [`collect_rigid_colliders`] uses.
///
/// **Static bodies are deliberately omitted.** A missing entry resolves to
/// [`RigidReaction::IMMOVABLE`], which is both the physically right answer for them and the
/// value that makes the node response take its bit-for-bit legacy branch — so level geometry
/// behaves exactly as it did before coupling existed, with no float drift.
///
/// Kinematic bodies *are* included, with zero inverse mass but their real velocity: they
/// cannot be pushed, but a moving platform now kicks a node it sweeps into, where before
/// every collider was assumed to be at rest.
#[cfg(feature = "rigid-coupling")]
pub fn collect_rigid_reactions(world: &World) -> Vec<(BodyHandle, RigidReaction)> {
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    let mut out = Vec::new();
    if let Some(q) = world.query::<(&RigidBody, &Velocity, &Transform)>() {
        for (e, (rb, vel, trans)) in q.iter() {
            if rb.is_static() {
                continue;
            }
            let Some(entity) = world.get_entity(e) else {
                continue;
            };
            out.push((
                BodyHandle::from_id(entity.id()),
                RigidReaction::new(
                    rb.inv_mass(),
                    rb.inv_world_inertia_tensor(trans.rotation),
                    // The lever arms are measured about the CENTRE OF MASS, not the transform
                    // origin; the two agree only while the body's COM offset is zero.
                    trans.position + trans.rotation * rb.center_of_mass,
                    vel.linear,
                    vel.angular,
                ),
            ));
        }
    }
    out
}

/// Spends the reactions the soft bodies owe: `v += inv_mass · linear`,
/// `ω += I⁻¹ · angular`.
///
/// Two guards, both load-bearing:
///
/// * **Non-dynamic bodies are skipped.** `inv_mass()` and `inv_world_inertia_tensor()` are
///   already zero for them so the arithmetic would be a no-op, but skipping outright keeps a
///   static or kinematic body from being woken by a push that cannot move it. (The solver
///   also withholds the impulse at the source — see [`RigidReaction::can_react`] — so this is
///   the second of two locks on "static/kinematic bodies are never pushed".)
///
/// * **A body that is pushed is WOKEN.** A sleeping body is skipped by both integration
///   stages and its island is not even solved, so the velocity would sit in the component
///   unspent and move nothing — the exact bug `physics_explosion_system` was fixed for. The
///   wake is scoped the same way it is there: only bodies the impulse actually MOVES
///   (non-zero Δv or Δω), never a body that merely appeared in the list.
#[cfg(feature = "rigid-coupling")]
pub fn apply_reaction_impulses(world: &World, impulses: &[RigidImpulse]) {
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    if impulses.is_empty() {
        return;
    }

    // SAFETY: scheduled system; the scheduler guarantees disjoint mutable access, and the
    // collider/reaction queries above have already been dropped.
    let mut q = unsafe {
        world.query_unchecked::<(
            gizmo_core::query::Mut<RigidBody>,
            &Transform,
            gizmo_core::query::Mut<Velocity>,
        )>()
    };
    if let Some(q) = &mut q {
        for (id, (mut rb, trans, mut vel)) in q.iter_mut() {
            if !rb.is_dynamic() {
                continue;
            }
            let handle = BodyHandle::from_id(id);
            let Some(impulse) = impulses.iter().find(|i| i.body == handle) else {
                continue;
            };

            let delta_v = impulse.linear * rb.inv_mass();
            let delta_w = rb.inv_world_inertia_tensor(trans.rotation) * impulse.angular;
            if delta_v == Vec3::ZERO && delta_w == Vec3::ZERO {
                continue;
            }

            vel.linear += delta_v;
            vel.angular += delta_w;
            rb.wake_up();
        }
    }
}

/// Steps every [`Cloth`] in the world by `dt` seconds under `gravity` using a fixed
/// number of XPBD sub-steps. The timestep is clamped to a stable upper bound.
///
/// Cloth is still **one-way**: it collides against the rigid colliders but pushes nothing
/// back. Only [`SoftBodyMesh`] has the two-body response so far.
#[tracing::instrument(skip_all, name = "cloth_step_system")]
pub fn cloth_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    // Determine a fixed number of XPBD substeps for stability
    let sub_steps = 10;

    // Collect rigid colliders so the cloth drapes over them (not just the floor).
    let rigid_colliders = collect_rigid_colliders(world);

    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<Cloth>>() } {
        for (_, mut cloth) in q.iter_mut() {
            cloth.step(dt, gravity, sub_steps, &rigid_colliders);
        }
    }
}

/// Steps every [`Rope`] in the world by `dt` seconds under `gravity`.
/// The timestep is clamped to a stable upper bound.
#[tracing::instrument(skip_all, name = "rope_step_system")]
pub fn rope_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<Rope>>() } {
        for (_, mut rope) in q.iter_mut() {
            rope.step(dt, gravity);
        }
    }
}
