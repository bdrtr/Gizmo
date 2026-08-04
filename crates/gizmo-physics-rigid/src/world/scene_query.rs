//! Scene queries: filtered raycasts, shape overlap, shape casting and point queries.
//!
//! Everything a gameplay programmer does *after* the simulation step goes through here —
//! character movement, melee hitboxes, area-of-effect, "can I place this building", line of
//! sight, projectiles with volume, mouse picking.
//!
//! Until 0.10 the entire public query surface was three unfilterable raycasts
//! ([`PhysicsWorld::raycast`], [`raycast_excluding`](PhysicsWorld::raycast_excluding),
//! [`raycast_all`](PhysicsWorld::raycast_all)). A ray has no volume, so a character
//! controller could not sweep its own capsule; and with no layer mask every gameplay ray hit
//! trigger volumes, invisible blockers, debris and the caster's own body. The engine's own
//! character controller and vehicle code are the evidence: both bypass the broadphase
//! entirely and hand-roll an `O(N)` scan over every collider in the world, every frame,
//! because there was no query to call.
//!
//! The pieces were all present — [`DynamicAabbTree::query_aabb`], [`NarrowPhase::test_collision`],
//! [`CollisionLayer`] and a fully implemented but unwired
//! [`Gjk::conservative_advancement`]. This module wires them together.
//!
//! # Filtering
//!
//! Every query takes a [`QueryFilter`]. The default hits solid colliders on every layer and
//! skips triggers, which is what a movement or line-of-sight query wants; opt back in with
//! [`QueryFilter::include_triggers`].

use super::PhysicsWorld;
use gizmo_math::{Aabb, Quat, Vec3};
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::raycast::{Ray, Raycast, RaycastHit};
use gizmo_physics_core::{BodyHandle, ColliderShape, NarrowPhase};

/// Distance tolerance for the sweep's touching pose, in world units.
const SWEEP_EPS: f32 = 1e-4;
/// Upper bound on march samples, so a long sweep against a tiny target stays bounded.
const MAX_SWEEP_SAMPLES: u32 = 512;
/// Bisection refinements after the march brackets the contact.
const SWEEP_BISECTIONS: u32 = 24;

/// Which colliders a scene query is allowed to see.
///
/// `Copy`, so it can be built once and reused across a frame's queries. The default hits
/// everything solid and nothing that is a trigger:
///
/// ```
/// # use gizmo_physics_rigid::world::QueryFilter;
/// let f = QueryFilter::default();
/// assert_eq!(f.mask, u32::MAX);
/// assert!(!f.include_triggers);
/// assert!(f.exclude.is_empty());
/// ```
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct QueryFilter<'a> {
    /// Bitmask of collider layers to consider. A collider is visible when
    /// `mask & (1 << collider.collision_layer.layer) != 0`.
    ///
    /// Note this is the *query's* mask against the collider's layer — it is deliberately
    /// one-directional, unlike collision filtering, which additionally requires the other
    /// body to accept you back. A query is an observer, not a participant.
    pub mask: u32,
    /// Bodies to skip entirely. Usually the caster: a character sweeping its own capsule, or
    /// a vehicle raycasting a wheel past its own chassis.
    pub exclude: &'a [BodyHandle],
    /// Whether trigger volumes count as hits. Off by default — a movement query that stops
    /// on a trigger is a bug, while an "what am I standing in" query wants exactly them.
    pub include_triggers: bool,
}

impl Default for QueryFilter<'_> {
    fn default() -> Self {
        Self {
            mask: u32::MAX,
            exclude: &[],
            include_triggers: false,
        }
    }
}

impl<'a> QueryFilter<'a> {
    /// Hit only colliders whose layer is set in `mask`.
    pub fn with_mask(mut self, mask: u32) -> Self {
        self.mask = mask;
        self
    }

    /// Hit only colliders on a single layer.
    pub fn on_layer(mut self, layer: u32) -> Self {
        self.mask = 1u32 << (layer & 31);
        self
    }

    /// Skip these bodies.
    pub fn excluding(mut self, bodies: &'a [BodyHandle]) -> Self {
        self.exclude = bodies;
        self
    }

    /// Include trigger volumes in the results.
    pub fn with_triggers(mut self) -> Self {
        self.include_triggers = true;
        self
    }
}

/// Where a swept shape first touched something.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ShapeCastHit {
    /// The body that was hit.
    pub entity: BodyHandle,
    /// Distance travelled along the cast direction before contact, in world units.
    /// Zero means the shape was already touching at its start pose.
    pub distance: f32,
    /// Surface normal at the contact, pointing back toward the caster.
    pub normal: Vec3,
}

impl PhysicsWorld {
    /// Does this collider index pass the filter?
    fn passes_filter(&self, index: usize, entity: BodyHandle, filter: &QueryFilter<'_>) -> bool {
        if filter.exclude.contains(&entity) {
            return false;
        }
        let collider = &self.colliders[index];
        if collider.is_trigger && !filter.include_triggers {
            return false;
        }
        let layer_bit = 1u32 << (collider.collision_layer.layer & 31);
        filter.mask & layer_bit != 0
    }

    /// Resolve a broadphase handle to its dense index, if it is still live.
    fn index_of(&self, entity: BodyHandle) -> Option<usize> {
        self.entity_index_map.get(&entity.id()).copied()
    }

    /// Cast a ray and return the closest hit that passes `filter`.
    ///
    /// This is [`raycast`](Self::raycast) with layers, trigger control and multi-body
    /// exclusion. Post-filtering the unfiltered version cannot substitute for it: `raycast`
    /// returns only the *closest* hit, so discarding it afterwards drops the query entirely
    /// rather than revealing what was behind — which is exactly the bug
    /// [`raycast_excluding`](Self::raycast_excluding) was added to work around for one body.
    pub fn raycast_filtered(
        &self,
        ray: &Ray,
        max_distance: f32,
        filter: QueryFilter<'_>,
    ) -> Option<RaycastHit> {
        let mut closest: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        for (entity, _) in self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance)
        {
            let Some(i) = self.index_of(entity) else {
                continue;
            };
            if !self.passes_filter(i, entity, &filter) {
                continue;
            }
            if let Some((distance, normal)) =
                Raycast::ray_shape(ray, &self.colliders[i].shape, &self.transforms[i])
            {
                if distance < closest_distance {
                    closest_distance = distance;
                    closest = Some(RaycastHit {
                        entity,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }
        closest
    }

    /// Every body whose collider overlaps `shape` placed at `pos`/`rot`.
    ///
    /// Broadphase-accelerated: the shape's world AABB narrows the candidate set through the
    /// BVH, then each candidate gets an exact [`NarrowPhase::test_collision`].
    ///
    /// This is the query behind spawn-placement checks ("does this fit?"), area-of-effect,
    /// melee hitboxes and trigger interrogation.
    pub fn overlap_shape(
        &self,
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        filter: QueryFilter<'_>,
    ) -> Vec<BodyHandle> {
        let probe = gizmo_physics_core::Collider::from_shape(shape.clone());
        let aabb = probe.compute_aabb(pos, rot);

        let mut hits = Vec::new();
        for entity in self.spatial_hash.query_aabb(aabb) {
            let Some(i) = self.index_of(entity) else {
                continue;
            };
            if !self.passes_filter(i, entity, &filter) {
                continue;
            }
            let t = &self.transforms[i];
            if NarrowPhase::test_collision(
                shape,
                pos,
                rot,
                &self.colliders[i].shape,
                t.position,
                t.rotation,
            )
            .is_some()
            {
                hits.push(entity);
            }
        }
        hits
    }

    /// Every body whose collider contains `point`.
    ///
    /// Implemented as an overlap against a point-sized sphere, so it inherits the
    /// narrowphase's tolerances rather than being an exact containment predicate — a point
    /// exactly on a surface may or may not be reported. Callers wanting "what am I inside"
    /// should generally prefer that behaviour to a knife-edge test.
    pub fn point_query(&self, point: Vec3, filter: QueryFilter<'_>) -> Vec<BodyHandle> {
        let probe = ColliderShape::Sphere(gizmo_physics_core::SphereShape { radius: 1e-4 });
        self.overlap_shape(&probe, point, Quat::IDENTITY, filter)
    }

    /// Sweep `shape` from `pos` along `dir` and return the first thing it hits.
    ///
    /// This is the query a character controller needs and could not previously express: a
    /// ray has no volume, so sweeping a capsule meant either three centre-line rays (what
    /// the built-in controller does, which walks through anything not intersecting the
    /// centre line) or a hand-rolled scan over every collider.
    ///
    /// `dir` is normalised internally; a zero-length direction or a non-positive
    /// `max_distance` yields `None`. A shape already overlapping something at its start pose
    /// reports that body at distance `0.0`, so a caller can detect "started inside" rather
    /// than tunnelling out of it.
    pub fn cast_shape(
        &self,
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        dir: Vec3,
        max_distance: f32,
        filter: QueryFilter<'_>,
    ) -> Option<ShapeCastHit> {
        if max_distance <= 0.0 || dir.length_squared() < 1e-12 {
            return None;
        }
        let dir = dir.normalize();

        // Already-overlapping bodies are reported explicitly at distance 0 rather than being
        // swept past, so a caller can detect "spawned inside geometry" instead of tunnelling
        // out of it.
        if let Some(&entity) = self.overlap_shape(shape, pos, rot, filter).first() {
            return Some(ShapeCastHit {
                entity,
                distance: 0.0,
                normal: -dir,
            });
        }

        // Broadphase over the swept volume: the start AABB merged with the end AABB.
        let probe = gizmo_physics_core::Collider::from_shape(shape.clone());
        let start_aabb = probe.compute_aabb(pos, rot);
        let end_aabb = probe.compute_aabb(pos + dir * max_distance, rot);
        let swept: Aabb = start_aabb.merge(end_aabb);

        let mut best: Option<ShapeCastHit> = None;
        for entity in self.spatial_hash.query_aabb(swept) {
            let Some(i) = self.index_of(entity) else {
                continue;
            };
            if !self.passes_filter(i, entity, &filter) {
                continue;
            }
            let t = &self.transforms[i];
            if let Some(distance) = self.sweep_against(
                shape,
                pos,
                rot,
                dir,
                max_distance,
                &self.colliders[i].shape,
                t.position,
                t.rotation,
            ) {
                if best.is_none_or(|b| distance < b.distance) {
                    // Normal from the touching pose: ask the narrowphase at the contact.
                    let touch = pos + dir * (distance + SWEEP_EPS);
                    let normal = NarrowPhase::test_collision(
                        shape,
                        touch,
                        rot,
                        &self.colliders[i].shape,
                        t.position,
                        t.rotation,
                    )
                    .map(|c| -c.normal)
                    .unwrap_or(-dir);
                    best = Some(ShapeCastHit {
                        entity,
                        distance,
                        normal,
                    });
                }
            }
        }
        best
    }

    /// Earliest distance along `dir` at which `shape` first touches one target, or `None`.
    ///
    /// Deliberately a march-and-bisect over [`NarrowPhase::test_collision`] rather than
    /// [`Gjk::conservative_advancement`], which is present in the crate and looks like the
    /// obvious tool. Conservative advancement is only correct for a head-on approach here:
    /// with any lateral offset its step `dist / closing_velocity` carries the shape *into*
    /// overlap, and `Gjk::distance` returns a meaningless positive distance for overlapping
    /// shapes instead of signalling penetration — so the closing velocity flips sign and the
    /// routine reports no hit for a sweep that plainly connects. Two unit boxes 5 apart with
    /// a 0.2 lateral offset reproduce it. Tracked in docs/FIXPLAN.md; this query does not
    /// wait on it.
    ///
    /// The march step is derived from the smaller of the two shapes rather than being a
    /// fixed subdivision, so the sweep cannot step over a thin target — the classic failure
    /// of a naive fixed-sample sweep.
    #[allow(clippy::too_many_arguments)]
    fn sweep_against(
        &self,
        shape: &ColliderShape,
        pos: Vec3,
        rot: Quat,
        dir: Vec3,
        max_distance: f32,
        target_shape: &ColliderShape,
        target_pos: Vec3,
        target_rot: Quat,
    ) -> Option<f32> {
        let hits_at = |d: f32| -> bool {
            NarrowPhase::test_collision(shape, pos + dir * d, rot, target_shape, target_pos, target_rot)
                .is_some()
        };

        // Step by half the smallest extent involved, so nothing thinner than a step can be
        // skipped. Clamped so a huge sweep against a tiny target stays bounded.
        let probe_aabb = gizmo_physics_core::Collider::from_shape(shape.clone()).compute_aabb(pos, rot);
        let target_aabb =
            gizmo_physics_core::Collider::from_shape(target_shape.clone()).compute_aabb(target_pos, target_rot);
        let smallest = probe_aabb
            .size()
            .min_element()
            .min(target_aabb.size().min_element())
            .max(SWEEP_EPS);
        let step = (smallest * 0.5).max(max_distance / MAX_SWEEP_SAMPLES as f32);

        let mut prev_clear = 0.0f32;
        let mut d = step;
        let mut first_hit = None;
        while d <= max_distance {
            if hits_at(d) {
                first_hit = Some(d);
                break;
            }
            prev_clear = d;
            d += step;
        }
        // The endpoint itself may be the only touching sample.
        if first_hit.is_none() {
            if hits_at(max_distance) {
                first_hit = Some(max_distance);
            } else {
                return None;
            }
        }

        // Bisect between the last clear sample and the first touching one.
        let (mut lo, mut hi) = (prev_clear, first_hit.unwrap());
        for _ in 0..SWEEP_BISECTIONS {
            let mid = 0.5 * (lo + hi);
            if hits_at(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Some(hi)
    }

    /// Convenience: sweep the collider a body already has.
    ///
    /// Excludes the body itself, so a character can sweep its own capsule without
    /// immediately hitting it.
    pub fn cast_body(
        &self,
        body: BodyHandle,
        dir: Vec3,
        max_distance: f32,
        filter: QueryFilter<'_>,
    ) -> Option<ShapeCastHit> {
        let i = self.index_of(body)?;
        let shape = self.colliders[i].shape.clone();
        let Transform {
            position, rotation, ..
        } = self.transforms[i];

        // Splice the body into the exclusion list without allocating when it is already
        // the only exclusion the caller wanted.
        let mut exclude: Vec<BodyHandle> = filter.exclude.to_vec();
        if !exclude.contains(&body) {
            exclude.push(body);
        }
        let filter = QueryFilter {
            exclude: &exclude,
            ..filter
        };
        self.cast_shape(&shape, position, rotation, dir, max_distance, filter)
    }
}
