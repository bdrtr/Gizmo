//! Which collider shapes shatter, and what happens to the ones that cannot.
//!
//! The bug these lock down: `shatter_entity` bailed out on every non-box collider, but all
//! three call sites had ALREADY written `Breakable::is_broken = true` before calling it. So a
//! sphere, capsule or hull breakable that ran out of health entered a state the engine has no
//! way out of — no debris, no despawn, and, since every damage path is gated on `!is_broken`
//! and nothing ever clears that flag, no further damage either. It sat in the scene at zero
//! health, permanently inert. "Unsupported shape" silently meant "destroyed entity".
//!
//! Both halves of the fix are asserted here, because either one alone is still wrong:
//!
//! * bounded shapes (sphere/capsule/hull/compound) now really shatter, through the collider's
//!   local bounding box;
//! * `Plane` and `TriMesh` still do not — and are therefore left DAMAGEABLE rather than latched,
//!   which is the half that would be easy to lose by just widening the shape match.
//!
//! Everything runs through `physics_explosion_system`, which is the shatter call site reachable
//! without a stepped contact: one pass, no integration, so nothing but the system under test
//! touches the components. The two collision call sites share `shatter_entity` and its return
//! value with it.
//!
//! The last three tests cover a second, later fix to the same function: the Voronoi seed was a
//! literal `42`, so every object in the game broke into the *same* debris pattern. It is now
//! derived from the entity's ECS id, which has to buy variety **without** costing
//! reproducibility — a seed taken from a wall clock, a process-global counter or anything else
//! rollback cannot restore would trade one for the other, silently. Both halves are asserted.

use gizmo_core::entity::Entity;
use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{Breakable, Explosion, ExplosionFalloff, RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_explosion_system;

/// A blast at the origin that reaches 10 m and carries far more damage than the breakables
/// below have health, so "did it break?" never turns on a threshold arithmetic detail.
fn spawn_blast(world: &mut World) {
    let e = world.spawn();
    world.add_component(
        e,
        Explosion {
            force_radius: 10.0,
            force: 500.0,
            damage: 1000.0,
            damage_radius: 10.0,
            falloff: ExplosionFalloff::Linear,
            offset: Vec3::ZERO,
            is_active: true,
        },
    );
    world.add_component(e, Transform::new(Vec3::ZERO));
}

/// 100 health, and a threshold low enough that the blast above clears it at these distances.
fn breakable() -> Breakable {
    Breakable {
        max_pieces: 6,
        threshold: 1.0,
        current_health: 100.0,
        max_health: 100.0,
        ..Default::default()
    }
}

/// The breakable carries no `RigidBody`, so every `RigidBody` in the world afterwards is debris.
fn spawn_breakable(world: &mut World, collider: Collider, pos: Vec3) -> Entity {
    let e = world.spawn();
    world.add_component(e, breakable());
    world.add_component(e, collider);
    world.add_component(e, Transform::new(pos));
    world.add_component(e, Velocity::default());
    e
}

/// One pass of the explosion system, with the deferred spawns/despawns flushed.
fn blast_once(world: &mut World) {
    spawn_blast(world);
    physics_explosion_system(world, 1.0 / 60.0);
    world.apply_commands();
}

/// Positions of every debris body, i.e. of every `RigidBody` in the world.
fn debris_positions(world: &World) -> Vec<Vec3> {
    let mut out = Vec::new();
    if let Some(q) = world.query::<(&RigidBody, &Transform)>() {
        for (_id, (_rb, t)) in q.iter() {
            out.push(t.position);
        }
    }
    out
}

fn health_of(world: &World, e: Entity) -> f32 {
    world
        .borrow::<Breakable>()
        .get(e.id())
        .expect("breakable still present")
        .current_health
}

fn is_broken(world: &World, e: Entity) -> bool {
    world
        .borrow::<Breakable>()
        .get(e.id())
        .expect("breakable still present")
        .is_broken
}

/// A sphere breakable must actually break: original gone, debris in its place.
///
/// This is the assertion that fails before the fix — pre-fix the sphere is still alive, holds
/// `is_broken == true`, and no `RigidBody` was ever spawned.
#[test]
fn a_sphere_breakable_shatters_instead_of_going_inert() {
    let mut world = World::new();
    let e = spawn_breakable(&mut world, Collider::sphere(0.5), Vec3::new(2.0, 0.0, 0.0));

    blast_once(&mut world);

    assert!(
        !world.is_alive(e),
        "a sphere breakable at zero health must be despawned, not left in the scene"
    );
    let debris = debris_positions(&world);
    assert!(
        !debris.is_empty(),
        "shattering must spawn debris; got none, which is the pre-fix silent bail-out"
    );
}

/// The same for a capsule and for a convex hull — the other two bounded shapes a game is
/// likely to make breakable. Not a copy for its own sake: the capsule's bound comes from
/// `half_height + radius` along Y and the hull's from its vertex cloud, so they exercise
/// different arms of the bounds derivation.
#[test]
fn capsules_and_hulls_shatter_too() {
    for (name, collider) in [
        ("capsule", Collider::capsule(0.3, 0.8)),
        (
            "hull",
            Collider::convex_hull(&[
                Vec3::new(-0.4, -0.4, -0.4),
                Vec3::new(0.4, -0.4, -0.4),
                Vec3::new(0.0, 0.5, -0.4),
                Vec3::new(0.0, 0.0, 0.5),
            ]),
        ),
    ] {
        let mut world = World::new();
        let e = spawn_breakable(&mut world, collider, Vec3::new(2.0, 0.0, 0.0));

        blast_once(&mut world);

        assert!(!world.is_alive(e), "{name} breakable must be despawned");
        assert!(
            !debris_positions(&world).is_empty(),
            "{name} breakable must spawn debris"
        );
    }
}

/// The debris of an OFF-CENTRE hull lands on the hull, not on the body's origin.
///
/// The Voronoi cells are cut from a box about the origin, so their centres are relative to the
/// bounding box's own centre; that centre has to be added back. A hull sitting entirely in
/// +X between 4 and 5 m makes the omission unmissable — without it every piece would appear
/// around x = 0, inside a body that is nowhere near there.
#[test]
fn debris_follows_an_off_centre_hull() {
    let mut world = World::new();
    let hull = Collider::convex_hull(&[
        Vec3::new(4.0, -0.5, -0.5),
        Vec3::new(5.0, -0.5, -0.5),
        Vec3::new(5.0, 0.5, -0.5),
        Vec3::new(4.0, 0.5, 0.5),
        Vec3::new(5.0, -0.5, 0.5),
        Vec3::new(4.5, 0.5, 0.5),
    ]);
    // The body is displaced along Z only — the blast skips a body exactly at its own centre —
    // so every debris x below is the hull's own offset, with no body translation along that
    // axis to explain it away.
    let e = spawn_breakable(&mut world, hull, Vec3::new(0.0, 0.0, 3.0));

    blast_once(&mut world);
    assert!(!world.is_alive(e));

    let debris = debris_positions(&world);
    assert!(!debris.is_empty(), "off-centre hull must spawn debris");
    for p in &debris {
        assert!(
            (3.9..=5.1).contains(&p.x),
            "debris at {p:?} is not on the hull (x ∈ [4, 5]) — the bounding box centre was \
             dropped, so the pieces were placed around the body origin instead"
        );
    }
}

/// A `Plane` cannot shatter — and must therefore stay UNLATCHED and damageable.
///
/// Two blasts, and the second one has to bite. Pre-fix the first blast latched `is_broken`,
/// which is what made the body permanently immune: the `!breakable.is_broken` gate at the top
/// of every damage path meant health never moved again. The plane's own AABB is a ±10 km cube,
/// so this also pins that the generic bounding-box arm does not swallow it and shatter the
/// floor into kilometre-wide boulders.
#[test]
fn an_unshatterable_plane_stays_damageable() {
    let mut world = World::new();
    let e = spawn_breakable(
        &mut world,
        Collider::plane(Vec3::Y, 0.0),
        Vec3::new(2.0, 0.0, 0.0),
    );

    blast_once(&mut world);

    assert!(world.is_alive(e), "a plane must not be despawned");
    assert!(
        debris_positions(&world).is_empty(),
        "a plane must not spawn debris"
    );
    assert!(
        !is_broken(&world, e),
        "`is_broken` must NOT latch on a shape that cannot shatter — nothing ever clears it, \
         so latching here disables the entity forever"
    );

    let after_first = health_of(&world, e);
    assert!(
        after_first < 100.0,
        "the blast should still have dealt damage (health {after_first})"
    );

    blast_once(&mut world);
    let after_second = health_of(&world, e);
    assert!(
        after_second < after_first,
        "a second blast must still damage it: health went {after_first} -> {after_second}, \
         i.e. the body is inert — this is exactly the pre-fix failure, one shape wider"
    );
}

/// Same contract for the static concave shape.
#[test]
fn an_unshatterable_trimesh_stays_damageable() {
    let mut world = World::new();
    let mesh = Collider::trimesh(
        vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(0.0, 0.0, 1.0),
        ],
        vec![0, 1, 2],
    );
    let e = spawn_breakable(&mut world, mesh, Vec3::new(2.0, 0.0, 0.0));

    blast_once(&mut world);

    assert!(world.is_alive(e), "a trimesh must not be despawned");
    assert!(
        debris_positions(&world).is_empty(),
        "a trimesh must not spawn debris"
    );
    assert!(!is_broken(&world, e), "`is_broken` must not latch");

    let after_first = health_of(&world, e);
    blast_once(&mut world);
    assert!(
        health_of(&world, e) < after_first,
        "a trimesh breakable must keep taking damage"
    );
}

/// Where the breakables below sit. Off the blast's own centre, which it skips.
const BODY_POS: Vec3 = Vec3::new(2.0, 0.0, 0.0);

/// Blast a lone 1 m cube breakable in a fresh world and report `(its entity id, its debris)`.
///
/// `filler` empty entities are spawned first, which is the whole point of the parameter: it
/// moves the breakable onto a different ECS id while leaving every other input — collider,
/// transform, health, blast — byte-for-byte the same. An entity with no components matches no
/// query, so the fillers change nothing else. The returned id is checked by the callers, so a
/// test can never pass by accident because two runs happened to share an id.
///
/// Debris is sorted lexicographically rather than by x alone: the comparison must not depend on
/// the order the ECS happens to iterate spawned chunks in.
fn shatter_lone_cube(filler: usize) -> (u32, Vec<[f32; 3]>) {
    let mut world = World::new();
    for _ in 0..filler {
        world.spawn();
    }
    let e = spawn_breakable(&mut world, Collider::box_collider(Vec3::splat(0.5)), BODY_POS);

    blast_once(&mut world);
    assert!(!world.is_alive(e), "the cube must have shattered");

    let mut debris: Vec<[f32; 3]> = debris_positions(&world)
        .iter()
        .map(|p| [p.x, p.y, p.z])
        .collect();
    assert!(!debris.is_empty(), "the cube must have spawned debris");
    debris.sort_by(|a, b| {
        a[0].total_cmp(&b[0])
            .then(a[1].total_cmp(&b[1]))
            .then(a[2].total_cmp(&b[2]))
    });
    (e.id(), debris)
}

/// The quality half: two objects breaking under identical conditions must NOT break alike.
///
/// This is the assertion that fails before the seed fix. With the old literal `42` the two
/// worlds below are handed the same seed, so `voronoi_shatter` lays down the same cell centres
/// and the two debris fields come out bit-for-bit equal — every crate in the game shattering
/// into the same six pieces in the same six places. The only difference between the two runs
/// here is the breakable's entity id, so a difference in the output can have come from nothing
/// else.
#[test]
fn two_different_entities_shatter_into_different_debris() {
    let (id_a, first) = shatter_lone_cube(0);
    let (id_b, second) = shatter_lone_cube(7);

    assert_ne!(
        id_a, id_b,
        "the two runs must really be different entities, or this test proves nothing"
    );
    assert_eq!(
        first.len(),
        second.len(),
        "same recipe, so the piece COUNT should not move — only the arrangement"
    );
    assert_ne!(
        first, second,
        "entities {id_a} and {id_b} broke into identical debris: the shatter seed is not \
         entity-dependent, so every object in the scene comes apart the same way"
    );
}

/// The reproducibility half: one entity, one scene, same debris every single time.
///
/// A different entity is shattered *in between* the two identical runs on purpose. That is what
/// separates an id-derived seed from the tempting alternatives: a process-global counter, a
/// wall-clock value or an allocation address would all survive a single run and only diverge
/// once something else has happened in the process, which is exactly what the middle call
/// simulates — and is how such a seed would fail in the field, as a replay that desyncs only
/// when the recorded session broke something earlier.
///
/// Unlike its sibling above this one also passed before the fix (a constant seed is trivially
/// reproducible); it is here to stop the fix for that one from being paid for with this.
#[test]
fn the_same_entity_shatters_identically_on_every_run() {
    let (id_first, first) = shatter_lone_cube(3);
    let (id_other, _) = shatter_lone_cube(0);
    let (id_again, again) = shatter_lone_cube(3);

    assert_eq!(id_first, id_again, "the two runs must share an entity id");
    assert_ne!(
        id_other, id_first,
        "the interleaved break must be a different entity"
    );
    assert_eq!(
        first, again,
        "entity {id_first} broke differently the second time round — the shatter seed depends \
         on something outside simulation state, so replay and rollback will desync"
    );
}

/// The box path's exact debris, pinned.
///
/// The numbers are a bit-equality pin, not a description: they are what the box shatter
/// produces and any change to the fracture path, the seed derivation or the spawn arithmetic
/// moves them. They were **re-blessed** when the seed stopped being a literal `42` and became
/// `shatter_seed(entity.id())` — that change was expected to move them and did; the values
/// below are entity id 0's pattern. Nothing here is timing- or order-dependent: the seed is a
/// pure function of the id, and this world always gives the breakable id 0.
#[test]
fn the_box_path_is_bit_identical() {
    let mut world = World::new();
    let e = spawn_breakable(
        &mut world,
        Collider::box_collider(Vec3::splat(0.5)),
        Vec3::new(2.0, 0.0, 0.0),
    );
    assert_eq!(e.id(), 0, "the pinned values below are entity id 0's pattern");

    blast_once(&mut world);
    assert!(!world.is_alive(e));

    let mut debris = debris_positions(&world);
    debris.sort_by(|a, b| a.x.total_cmp(&b.x));
    let expected: [[f32; 3]; 6] = [
        [1.6888595, -0.18551427, -0.23534574],
        [1.9529462, 0.285228, 0.09344177],
        [1.9958057, -0.15311843, 0.25925562],
        [2.0924218, -0.15759228, -0.17816569],
        [2.3184972, -0.37037852, 0.2993501],
        [2.3633454, -0.09555735, -0.2860109],
    ];
    assert_eq!(debris.len(), expected.len(), "debris count changed");
    for (got, want) in debris.iter().zip(expected.iter()) {
        assert_eq!(
            [got.x, got.y, got.z],
            *want,
            "the box shatter is not bit-identical any more"
        );
    }
}

// ── Recycled id slots ────────────────────────────────────────────────────────────────────
//
// Query iteration yields a bare `u32` — no generation — and this system rebuilt handles from it
// as `Entity::new(id, 0)`. That is the right handle only while the id is on its FIRST life.
// `World::despawn` bumps the generation and `Entities::reserve_entity` drains the free list
// first, so the very next spawn after any despawn lands on a recycled slot at generation 1 —
// ordinary, not exotic, in any game that pools or destroys anything.
//
// A fabricated generation-0 handle then fails `World::despawn`'s `is_alive` check and the
// despawn is skipped ENTIRELY AND SILENTLY. Everything else in the frame still happens, which is
// what makes the result worse than a no-op.
//
// `World::entity` is the documented way to turn a raw id into a live handle, and its own rustdoc
// warns against fabricating one; the same class was already fixed in gizmo-scene twice.

/// A breakable in a recycled id slot must shatter, not survive alongside its own debris.
///
/// Pre-fix this produced the strictly worst outcome available: the despawn was skipped, the
/// debris spawned anyway, and `is_broken` latched on the original — and since every damage path
/// is gated on `!is_broken` and nothing ever clears it, the survivor is *permanently
/// undamageable*. One blast leaves a duplicate that can never be removed by gameplay.
#[test]
fn a_recycled_id_breakable_shatters_instead_of_outliving_its_own_debris() {
    let mut world = World::new();

    // Burn one id so the next spawn is handed the same id at a bumped generation.
    let throwaway = world.spawn();
    world.despawn(throwaway);

    let e = spawn_breakable(&mut world, Collider::sphere(0.5), Vec3::new(2.0, 0.0, 0.0));
    assert_eq!(e.id(), throwaway.id(), "sanity: the allocator recycled the id");
    assert_ne!(
        e.generation(),
        0,
        "sanity: a recycled handle must carry a bumped generation, or this test proves nothing"
    );

    blast_once(&mut world);

    assert!(
        !world.is_alive(e),
        "a breakable in a recycled id slot was left in the scene — its despawn went to a \
         fabricated generation-0 handle and was silently skipped"
    );
    assert!(
        !debris_positions(&world).is_empty(),
        "the debris must still spawn; the bug is the original surviving it, not the debris"
    );
    assert!(
        world.borrow::<Breakable>().get(e.id()).is_none(),
        "no Breakable may remain on the recycled id — a survivor here is latched is_broken and \
         can never be damaged again"
    );
}

/// An explosion entity in a recycled id slot must fire once and then be gone.
///
/// The same defect on the other end of the same function: the explosion is collected as
/// `Entity::new(ent_id, 0)` and despawned through that handle at the end of the pass. On a
/// recycled slot the despawn is skipped, `is_active` stays true, and the blast re-detonates on
/// EVERY SUBSEQUENT FRAME — re-impulsing bodies and re-damaging breakables that survived the
/// first pass. `Explosion`'s own documentation promises it "applies an active explosion exactly
/// once and then despawns the entity".
#[test]
fn a_recycled_id_explosion_fires_once_and_then_despawns() {
    let mut world = World::new();

    let throwaway = world.spawn();
    world.despawn(throwaway);

    // The blast lands on the recycled id.
    let blast = world.spawn();
    assert_eq!(blast.id(), throwaway.id(), "sanity: the allocator recycled the id");
    assert_ne!(blast.generation(), 0, "sanity: bumped generation");
    world.add_component(
        blast,
        Explosion {
            force_radius: 10.0,
            force: 500.0,
            damage: 1000.0,
            damage_radius: 10.0,
            falloff: ExplosionFalloff::Linear,
            offset: Vec3::ZERO,
            is_active: true,
        },
    );
    world.add_component(blast, Transform::new(Vec3::ZERO));

    physics_explosion_system(&world, 1.0 / 60.0);
    world.apply_commands();

    assert!(
        !world.is_alive(blast),
        "an explosion in a recycled id slot stayed alive, so it re-detonates every frame"
    );
    assert!(
        world.query::<&Explosion>().map(|q| q.iter().count()).unwrap_or(0) == 0,
        "no Explosion component may survive the pass that consumed it"
    );
}

/// The third symptom, and the quietest: on the COLLISION path a breakable in a recycled id slot
/// took no contact damage at all.
///
/// Here the fabricated handle failed `get_mut_entity`'s own generation check rather than
/// `despawn`'s, so the entity was skipped before any damage was applied. That direction fails
/// *safe* — nothing is corrupted — which is exactly why it could sit unnoticed: the object simply
/// never breaks, and an invulnerable crate reads as a tuning problem rather than a bug. The
/// comment at that call site claimed the generation check was protecting against writing into a
/// reused slot; with the generation pinned to 0 it was rejecting every reused slot instead.
///
/// Driven through `physics_fracture_system` with a hand-built `CollisionEvent`, because the
/// impulse is read straight off `ContactPoint::normal_impulse` — no stepped contact, and no body
/// fixture, is needed to reach the branch.
#[test]
fn a_recycled_id_breakable_still_takes_contact_damage() {
    use gizmo_physics_core::{BodyHandle, CollisionEvent, CollisionEventType, ContactPoint, ContactPoints};
    use gizmo_physics_rigid::system::physics_fracture_system;
    use gizmo_physics_rigid::PhysicsWorld;

    let mut world = World::new();

    let throwaway = world.spawn();
    world.despawn(throwaway);
    let e = spawn_breakable(&mut world, Collider::sphere(0.5), Vec3::ZERO);
    assert_eq!(e.id(), throwaway.id(), "sanity: the allocator recycled the id");
    assert_ne!(e.generation(), 0, "sanity: bumped generation");

    // A SECOND breakable, also on a recycled slot, so the event pair can be filled both ways.
    // `entity_a` and `entity_b` are handled by two copy-pasted blocks in the system, and a
    // half-applied fix there is exactly the failure a single-slot test would wave through.
    let throwaway2 = world.spawn();
    world.despawn(throwaway2);
    let f = spawn_breakable(&mut world, Collider::sphere(0.5), Vec3::new(5.0, 0.0, 0.0));
    assert_ne!(f.generation(), 0, "sanity: the second slot is recycled too");

    let hard_hit = || {
        let mut points = ContactPoints::new();
        points.push(ContactPoint {
            // Far above the breakable's threshold of 1.0 and its 100 health, so the branch is
            // not reached or missed on an arithmetic detail.
            normal_impulse: 500.0,
            normal: Vec3::Y,
            ..Default::default()
        });
        points
    };

    let mut pw = PhysicsWorld::default();
    // `e` in the entity_a slot…
    pw.collision_events.push(CollisionEvent {
        entity_a: BodyHandle::from_id(e.id()),
        entity_b: BodyHandle::from_id(9999),
        event_type: CollisionEventType::Started,
        contact_points: hard_hit(),
    });
    // …and `f` in the entity_b slot.
    pw.collision_events.push(CollisionEvent {
        entity_a: BodyHandle::from_id(9999),
        entity_b: BodyHandle::from_id(f.id()),
        event_type: CollisionEventType::Started,
        contact_points: hard_hit(),
    });
    world.insert_resource(pw);

    physics_fracture_system(&world, 1.0 / 60.0);
    world.apply_commands();

    assert!(
        !world.is_alive(e),
        "entity_a slot: a breakable in a recycled id slot took no contact damage — the \
         fabricated generation-0 handle was rejected by `get_mut_entity`, so it was skipped"
    );
    assert!(
        !world.is_alive(f),
        "entity_b slot: same defect in the copy-pasted twin of the block above"
    );
    assert!(
        !debris_positions(&world).is_empty(),
        "they must actually shatter, not merely be reached"
    );
}
