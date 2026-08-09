//! What one suspension raycast asks of the scene, and the two ways to answer it.
//!
//! A wheel is not a rigid body — it is a single ray cast along its strut axis, looking for the
//! nearest surface it may rest on. That is the whole of the vehicle model's contact with the
//! world, and it is expressed here as one trait method so the answer can come either from a
//! plain list of colliders ([`ColliderListQuery`], the pre-0.10 behaviour) or from the
//! broadphase-accelerated scene-query layer ([`PhysicsWorld::raycast_filtered`]).
//!
//! # Why this exists
//!
//! [`update_vehicle`](super::update_vehicle) used to take a `&[(BodyHandle, Transform,
//! Collider)]` and scan **all** of it, once per wheel, per step — after its caller had cloned
//! every collider in the world into that slice, also per step. On a city-sized scene that is
//! thousands of `Collider` clones and tens of thousands of AABB tests per step, spent to find
//! four ground contacts within a metre of the car.
//!
//! The engine now has a query layer that answers the same question through the BVH, with
//! multi-body exclusion and a layer mask ([`QueryFilter`]).
//! [`vehicle_controller_system`](crate::systems::vehicle_controller_system) routes through it.
//!
//! # The two implementations are not interchangeable
//!
//! They answer over **different sets of bodies**, which is a behaviour change and is spelled
//! out on [`vehicle_controller_system`](crate::systems::vehicle_controller_system). In short:
//! a [`PhysicsWorld`] contains exactly the bodies the rigid pipeline simulates, so a wheel
//! querying it rests on exactly what the chassis would collide with — no more (a `Collider`
//! with no `RigidBody` is not in there) and no less.

use gizmo_physics_core::components::PhysicsMaterial;
use gizmo_physics_core::raycast::{Ray, Raycast, RaycastHit};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::world::{PhysicsWorld, QueryFilter};

/// What a suspension ray found: the hit itself plus the surface's dynamic friction.
///
/// [`RaycastHit`] is tyre-agnostic and carries no material, but the grip model needs one, so
/// the friction the wheel will use is sampled at the same time as the hit rather than being
/// looked up again afterwards.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct WheelGroundHit {
    /// Nearest qualifying hit along the strut ray.
    pub hit: RaycastHit,
    /// `PhysicsMaterial::dynamic_friction` of the collider that was hit. Multiplied into the
    /// tyre's friction circle alongside the weather factor.
    pub dynamic_friction: f32,
}

impl WheelGroundHit {
    /// Builds a hit. Present because the struct is `#[non_exhaustive]` and the trait it is
    /// returned from is implementable outside this crate.
    pub fn new(hit: RaycastHit, dynamic_friction: f32) -> Self {
        Self {
            hit,
            dynamic_friction,
        }
    }
}

/// The scene, as far as a wheel is concerned: one downward ray, one nearest surface.
///
/// Implement this to drive [`update_vehicle_with_query`](super::update_vehicle_with_query)
/// against your own world representation — a heightfield, a navmesh, a fixed ground plane.
pub trait WheelGroundQuery {
    /// Nearest surface along `ray` within `max_distance`, or `None`.
    ///
    /// Contract every implementation must keep, because the suspension model depends on it:
    ///
    /// * `chassis` is **never** a hit. A wheel ray starts inside the car's own collider.
    /// * Trigger volumes are **never** a hit. Suspension is a movement query; resting a car on
    ///   a trigger is a bug.
    /// * The hit returned is the **nearest** one, and its `distance` is strictly less than
    ///   `max_distance`.
    fn cast_wheel_ray(
        &self,
        ray: &Ray,
        max_distance: f32,
        chassis: BodyHandle,
    ) -> Option<WheelGroundHit>;
}

/// Answers a wheel ray by scanning a list of colliders linearly — the pre-0.10 behaviour,
/// kept for [`update_vehicle`](super::update_vehicle) and for callers with no [`PhysicsWorld`].
///
/// Cost is `O(colliders)` per wheel per step, with an AABB pre-reject per candidate. Prefer
/// querying a [`PhysicsWorld`] when there is one; this exists so the ECS-free entry point and
/// the pre-first-step case keep working, not because it is a good idea at scale.
#[derive(Clone, Copy, Debug)]
pub struct ColliderListQuery<'a> {
    /// Every collider the wheels may rest on. Anything absent is invisible to them.
    pub colliders: &'a [(BodyHandle, Transform, Collider)],
}

impl<'a> ColliderListQuery<'a> {
    /// Wraps a collider list.
    pub fn new(colliders: &'a [(BodyHandle, Transform, Collider)]) -> Self {
        Self { colliders }
    }
}

impl WheelGroundQuery for ColliderListQuery<'_> {
    fn cast_wheel_ray(
        &self,
        ray: &Ray,
        max_distance: f32,
        chassis: BodyHandle,
    ) -> Option<WheelGroundHit> {
        let mut closest: Option<WheelGroundHit> = None;
        let mut closest_dist = max_distance;

        for (other_ent, other_trans, other_col) in self.colliders {
            if *other_ent == chassis || other_col.is_trigger {
                continue;
            }
            let aabb = other_col.compute_aabb(other_trans.position, other_trans.rotation);
            if Raycast::ray_aabb(ray, &aabb).is_none() {
                continue;
            }
            if let Some((dist, normal)) = Raycast::ray_shape(ray, &other_col.shape, other_trans) {
                if dist < closest_dist {
                    closest_dist = dist;
                    closest = Some(WheelGroundHit {
                        hit: RaycastHit {
                            entity: *other_ent,
                            point: ray.point_at(dist),
                            normal,
                            distance: dist,
                        },
                        dynamic_friction: other_col.material.dynamic_friction,
                    });
                }
            }
        }
        closest
    }
}

impl WheelGroundQuery for PhysicsWorld {
    /// One [`PhysicsWorld::raycast_filtered`] through the BVH, excluding the chassis.
    ///
    /// No layer mask (`QueryFilter::default()` is `u32::MAX`), so a wheel can rest on any
    /// layer — the same "everything solid" set the linear scan used. Triggers are excluded by
    /// the default filter.
    fn cast_wheel_ray(
        &self,
        ray: &Ray,
        max_distance: f32,
        chassis: BodyHandle,
    ) -> Option<WheelGroundHit> {
        let exclude = [chassis];
        let hit = self.raycast_filtered(ray, max_distance, QueryFilter::default().excluding(&exclude))?;
        // The hit came out of this world, so the index lookup cannot normally miss; the
        // fallback keeps a torn world from silently producing a zero-grip surface.
        let dynamic_friction = self
            .entity_index_map
            .get(&hit.entity.id())
            .map(|&i| self.colliders[i].material.dynamic_friction)
            .unwrap_or(PhysicsMaterial::ASPHALT.dynamic_friction);
        Some(WheelGroundHit {
            hit,
            dynamic_friction,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;
    use gizmo_physics_rigid::components::{RigidBody, Velocity};

    const CHASSIS: u32 = 1;
    const GROUND: u32 = 2;
    const TRIGGER: u32 = 3;
    const CEILING: u32 = 4;

    fn slab(hy: f32) -> Collider {
        Collider::box_collider(Vec3::new(50.0, hy, 50.0))
    }

    /// Chassis at the origin, ground slab with its top at `y = 0`, a trigger slab at
    /// `y = 0.5` (between the wheel and the ground) and a ceiling the wheel ray never
    /// reaches downward.
    fn scene() -> (PhysicsWorld, Vec<(BodyHandle, Transform, Collider)>) {
        let bodies: Vec<(BodyHandle, Transform, Collider)> = vec![
            (
                BodyHandle::from_id(CHASSIS),
                Transform::new(Vec3::new(0.0, 1.0, 0.0)),
                Collider::box_collider(Vec3::new(0.9, 0.3, 2.0)),
            ),
            (
                BodyHandle::from_id(GROUND),
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                slab(1.0).with_material(gizmo_physics_core::components::PhysicsMaterial::ICE),
            ),
            (
                BodyHandle::from_id(TRIGGER),
                Transform::new(Vec3::new(0.0, 0.5, 0.0)),
                slab(0.05).with_trigger(true),
            ),
            (
                BodyHandle::from_id(CEILING),
                Transform::new(Vec3::new(0.0, 20.0, 0.0)),
                slab(0.5),
            ),
        ];

        let mut pw = PhysicsWorld::new();
        for (h, t, c) in &bodies {
            pw.add_body(*h, RigidBody::new_static(), *t, Velocity::default(), c.clone());
        }
        (pw, bodies)
    }

    /// The filter contract the suspension model relies on: the chassis is invisible, triggers
    /// are invisible, and the nearest remaining surface wins.
    #[test]
    fn physics_world_query_excludes_the_chassis_and_triggers() {
        let (pw, _) = scene();
        // Straight down from inside the chassis, long enough to reach the ground.
        let ray = Ray::new(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Y);
        let hit = pw
            .cast_wheel_ray(&ray, 3.0, BodyHandle::from_id(CHASSIS))
            .expect("the ground is within reach");

        assert_eq!(
            hit.hit.entity.id(),
            GROUND,
            "the chassis (started inside) and the trigger slab at y=0.5 must both be skipped"
        );
        assert!((hit.hit.distance - 1.0).abs() < 1e-4, "distance {}", hit.hit.distance);
        assert_eq!(
            hit.dynamic_friction,
            gizmo_physics_core::components::PhysicsMaterial::ICE.dynamic_friction,
            "friction must come from the collider that was hit, not a default"
        );
    }

    /// A ray that reaches nothing inside `max_distance` reports nothing — the wheel's
    /// "airborne" case. `max_distance` is exclusive, matching the linear scan.
    #[test]
    fn physics_world_query_respects_max_distance() {
        let (pw, _) = scene();
        let ray = Ray::new(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Y);
        assert!(pw
            .cast_wheel_ray(&ray, 0.9, BodyHandle::from_id(CHASSIS))
            .is_none());
    }

    /// The two implementations must answer the same question the same way on a scene both can
    /// see, or switching `vehicle_controller_system` over would silently retune every car.
    #[test]
    fn broadphase_and_linear_scan_agree() {
        let (pw, bodies) = scene();
        let list = ColliderListQuery::new(&bodies);

        for (ox, oz) in [(0.0, 0.0), (0.8, 1.4), (-0.8, -1.4), (30.0, 30.0), (80.0, 0.0)] {
            let ray = Ray::new(Vec3::new(ox, 1.0, oz), Vec3::NEG_Y);
            let a = pw.cast_wheel_ray(&ray, 3.0, BodyHandle::from_id(CHASSIS));
            let b = list.cast_wheel_ray(&ray, 3.0, BodyHandle::from_id(CHASSIS));
            match (a, b) {
                (None, None) => {}
                (Some(a), Some(b)) => {
                    assert_eq!(a.hit.entity, b.hit.entity, "at ({ox}, {oz})");
                    assert_eq!(a.hit.distance, b.hit.distance, "at ({ox}, {oz})");
                    assert_eq!(a.hit.normal, b.hit.normal, "at ({ox}, {oz})");
                    assert_eq!(a.hit.point, b.hit.point, "at ({ox}, {oz})");
                    assert_eq!(a.dynamic_friction, b.dynamic_friction, "at ({ox}, {oz})");
                }
                (a, b) => panic!("disagreement at ({ox}, {oz}): {a:?} vs {b:?}"),
            }
        }
    }
}
