use super::PhysicsWorld;
use crate::{
    components::{RigidBody, Velocity},
    integrator::Integrator,
};
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::raycast::{Ray, Raycast, RaycastHit};
use gizmo_physics_core::BodyHandle;

impl PhysicsWorld {
    /// Apply an impulse to a body at a point.
    ///
    /// `rb` alınır `&mut` çünkü uyuyan bir cisme impuls uygulamak onu UYANDIRMALIDIR;
    /// aksi halde hız değişir ama `is_sleeping` true kalır → position_integration cismi
    /// atlar ve impuls SESSİZCE YUTULUR (cisim hiç hareket etmez).
    pub fn apply_impulse(
        &self,
        rb: &mut RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        impulse: gizmo_math::Vec3,
        point: gizmo_math::Vec3,
    ) {
        if rb.is_dynamic() {
            rb.wake_up();
        }
        Integrator::apply_impulse_at_point(rb, transform, vel, impulse, point);
    }

    /// Apply a force to a body. `rb` `&mut` — uyuyan cismi uyandırır (bkz. apply_impulse).
    pub fn apply_force(
        &self,
        rb: &mut RigidBody,
        vel: &mut Velocity,
        force: gizmo_math::Vec3,
        dt: f32,
    ) {
        if rb.is_dynamic() {
            rb.wake_up();
        }
        Integrator::apply_force(rb, vel, force, dt);
    }

    /// Perform a raycast against all bodies
    pub fn raycast(&self, ray: &Ray, max_distance: f32) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        let potential_hits = self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) =
                    Raycast::ray_shape(ray, &collider.shape, transform)
                {
                    if distance < closest_distance {
                        closest_distance = distance;
                        closest_hit = Some(RaycastHit {
                            entity,
                            point: ray.point_at(distance),
                            normal,
                            distance,
                        });
                    }
                }
            }
        }

        closest_hit
    }

    /// Perform a raycast, ignoring one body (e.g. a vehicle raycasting its own
    /// wheels must not hit its own chassis).
    ///
    /// [`raycast`](Self::raycast) returns only the CLOSEST hit; a caller that
    /// wanted to exclude itself by post-filtering (`hit.entity != me`) would drop
    /// the hit entirely and never see the ground BEHIND its own collider — so a
    /// wheel ray starting inside/near the chassis collider reported "not grounded"
    /// and the vehicle fell through. Excluding during the sweep returns the closest
    /// hit among the *other* bodies, which is the correct ground contact.
    pub fn raycast_excluding(
        &self,
        ray: &Ray,
        max_distance: f32,
        exclude: BodyHandle,
    ) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        let potential_hits = self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if entity == exclude {
                continue;
            }
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                if let Some((distance, normal)) =
                    Raycast::ray_shape(ray, &collider.shape, transform)
                {
                    if distance < closest_distance {
                        closest_distance = distance;
                        closest_hit = Some(RaycastHit {
                            entity,
                            point: ray.point_at(distance),
                            normal,
                            distance,
                        });
                    }
                }
            }
        }

        closest_hit
    }

    /// Perform a raycast and return all hits
    pub fn raycast_all(&self, ray: &Ray, max_distance: f32) -> Vec<RaycastHit> {
        let mut hits = Vec::new();

        let potential_hits = self
            .spatial_hash
            .query_ray(ray.origin, ray.direction, max_distance);

        for (entity, _aabb_t) in potential_hits {
            if let Some(&i) = self.entity_index_map.get(&entity.id()) {
                let transform = &self.transforms[i];
                let collider = &self.colliders[i];

                // Detailed shape test
                if let Some((distance, normal)) =
                    Raycast::ray_shape(ray, &collider.shape, transform)
                {
                    hits.push(RaycastHit {
                        entity,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }

        // Sort by distance
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }
}
