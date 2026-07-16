use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::{CollisionLayer, PhysicsMaterial, Transform};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Collider {
    pub shape: ColliderShape,
    pub is_trigger: bool,
    pub material: PhysicsMaterial,
    pub collision_layer: CollisionLayer,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius: 0.5 }),
            is_trigger: false,
            material: PhysicsMaterial::default(),
            collision_layer: CollisionLayer::default(),
        }
    }
}

impl Collider {
    /// Build a collider from a raw [`ColliderShape`], using the default
    /// material and collision layer with `is_trigger = false`.
    ///
    /// This is the canonical constructor for turning a bare shape into a
    /// [`Collider`]: the struct is `#[non_exhaustive]`, so it cannot be built
    /// with a struct literal from outside this crate. Combine with the
    /// `with_*` builder methods to override defaults.
    pub fn from_shape(shape: ColliderShape) -> Self {
        Self {
            shape,
            ..Default::default()
        }
    }

    /// Calculate AABB for this collider at given transform
    pub fn compute_aabb(&self, position: Vec3, rotation: Quat) -> gizmo_math::Aabb {
        match &self.shape {
            ColliderShape::Sphere(s) => {
                let radius_vec = Vec3::splat(s.radius);
                gizmo_math::Aabb::from_center_half_extents(position, radius_vec)
            }
            ColliderShape::Box(b) => {
                // Rotate the half extents to get world-space AABB
                let corners = [
                    Vec3::new(b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, b.half_extents.z),
                    Vec3::new(-b.half_extents.x, b.half_extents.y, -b.half_extents.z),
                    Vec3::new(b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                    Vec3::new(-b.half_extents.x, -b.half_extents.y, -b.half_extents.z),
                ];

                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);

                for corner in &corners {
                    let rotated = rotation * (*corner);
                    let world_pos = position + rotated;
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }

                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::Capsule(c) => {
                let axis = rotation * Vec3::Y;
                let half_height_vec = axis * c.half_height;
                let radius_vec = Vec3::splat(c.radius);
                let extent = half_height_vec.abs() + radius_vec;
                gizmo_math::Aabb::from_center_half_extents(position, extent)
            }
            ColliderShape::Plane(_) => {
                // Infinite plane - use a very large AABB
                let large = 10000.0;
                gizmo_math::Aabb::new(position - Vec3::splat(large), position + Vec3::splat(large))
            }
            ColliderShape::TriMesh(tm) => {
                if tm.vertices.is_empty() {
                    // No vertices ⇒ the min/max fold below stays at ±INFINITY, producing an
                    // inverted (degenerate) AABB that silently breaks broadphase overlap tests.
                    tracing::warn!(
                        "TriMesh collider has no vertices; computed AABB is degenerate (inverted)"
                    );
                }
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for v in tm.vertices.iter() {
                    let world_pos = position + rotation * (*v);
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }
                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::ConvexHull(ch) => {
                if ch.vertices.is_empty() {
                    tracing::warn!(
                        "ConvexHull collider has no vertices; computed AABB is degenerate (inverted)"
                    );
                }
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for v in ch.vertices.iter() {
                    let world_pos = position + rotation * (*v);
                    min = min.min(world_pos);
                    max = max.max(world_pos);
                }
                gizmo_math::Aabb::new(min, max)
            }
            ColliderShape::Compound(shapes) => {
                if shapes.is_empty() {
                    tracing::warn!(
                        "Compound collider has no sub-shapes; computed AABB is degenerate (inverted)"
                    );
                }
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for (local_t, sub_shape) in shapes {
                    let world_pos = position + rotation.mul_vec3(local_t.position);
                    let world_rot = rotation * local_t.rotation;

                    let temp_col = Collider {
                        shape: (**sub_shape).clone(),
                        ..Default::default()
                    };
                    let sub_aabb = temp_col.compute_aabb(world_pos, world_rot);
                    min = min.min(sub_aabb.min.into());
                    max = max.max(sub_aabb.max.into());
                }
                gizmo_math::Aabb::new(min, max)
            }
        }
    }

    pub fn plane(normal: Vec3, distance: f32) -> Self {
        Self {
            shape: ColliderShape::Plane(PlaneShape { normal, distance }),
            ..Default::default()
        }
    }

    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius }),
            ..Default::default()
        }
    }

    pub fn box_collider(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(BoxShape { half_extents }),
            ..Default::default()
        }
    }

    pub fn offset_box(offset: Vec3, half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Compound(vec![(
                Transform::new(offset),
                Box::new(ColliderShape::Box(BoxShape { half_extents })),
            )]),
            ..Default::default()
        }
    }

    pub fn capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(CapsuleShape {
                radius,
                half_height,
            }),
            ..Default::default()
        }
    }

    pub fn convex_hull(points: &[Vec3]) -> Self {
        let hull = crate::quickhull::compute_convex_hull(points);
        Self {
            shape: ColliderShape::ConvexHull(ConvexHullShape {
                vertices: std::sync::Arc::new(hull.vertices),
                faces: std::sync::Arc::new(hull.faces),
            }),
            ..Default::default()
        }
    }

    pub fn with_trigger(mut self, is_trigger: bool) -> Self {
        self.is_trigger = is_trigger;
        self
    }

    pub fn with_material(mut self, material: PhysicsMaterial) -> Self {
        self.material = material;
        self
    }

    /// Zıplaklık (restitution) kısayolu — tam malzeme kurmadan tek satırda ayarla.
    /// Örn: `Collider::sphere(0.5).with_restitution(0.9)` (defalarca zıplayan top).
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.material.restitution = restitution.clamp(0.0, 1.0);
        self
    }

    /// Sürtünme kısayolu (statik = dinamik = `friction`).
    pub fn with_friction(mut self, friction: f32) -> Self {
        let f = friction.max(0.0);
        self.material.static_friction = f;
        self.material.dynamic_friction = f;
        self
    }

    // Backwards compatibility wrappers
    pub fn aabb(half_extents: Vec3) -> Self {
        Self::box_collider(half_extents)
    }

    pub fn new_sphere(radius: f32) -> Self {
        Self::sphere(radius)
    }

    pub fn new_aabb(x: f32, y: f32, z: f32) -> Self {
        Self::box_collider(Vec3::new(x, y, z))
    }

    pub fn new_capsule(radius: f32, half_height: f32) -> Self {
        Self::capsule(radius, half_height)
    }

    pub fn with_layer(mut self, layer: CollisionLayer) -> Self {
        self.collision_layer = layer;
        self
    }

    pub fn volume(&self) -> f32 {
        match &self.shape {
            ColliderShape::Sphere(s) => (4.0 / 3.0) * std::f32::consts::PI * s.radius.powi(3),
            ColliderShape::Box(b) => 8.0 * b.half_extents.x * b.half_extents.y * b.half_extents.z,
            ColliderShape::Capsule(c) => {
                let cylinder_vol = std::f32::consts::PI * c.radius.powi(2) * (c.half_height * 2.0);
                let sphere_vol = (4.0 / 3.0) * std::f32::consts::PI * c.radius.powi(3);
                cylinder_vol + sphere_vol
            }
            ColliderShape::Plane(_) => f32::MAX, // Safe value instead of INFINITY for inertia calculations
            ColliderShape::TriMesh(_)
            | ColliderShape::ConvexHull(_)
            | ColliderShape::Compound(_) => {
                let aabb = self.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
                let e = aabb.max - aabb.min;
                e.x * e.y * e.z * 0.5 // Approximate volume from AABB
            }
        }
    }

    pub fn extents_y(&self) -> f32 {
        match &self.shape {
            ColliderShape::Sphere(s) => s.radius,
            ColliderShape::Box(b) => b.half_extents.y,
            ColliderShape::Capsule(c) => c.half_height + c.radius,
            ColliderShape::Plane(_) => 0.0,
            ColliderShape::TriMesh(_)
            | ColliderShape::ConvexHull(_)
            | ColliderShape::Compound(_) => {
                let aabb = self.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
                (aabb.max.y - aabb.min.y) * 0.5
            }
        }
    }
}

// NOT `#[non_exhaustive]`: the engine's own crates (gizmo-physics-rigid) match
// this exhaustively to compute inertia / AABB / narrowphase dispatch. Adding a
// new collider shape is inherently a breaking change (it needs solver support),
// so a major version bump is appropriate — and exhaustive matching lets the
// compiler flag every site that must handle the new shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    Sphere(SphereShape),
    Box(BoxShape),
    Capsule(CapsuleShape),
    Plane(PlaneShape),
    TriMesh(TriMeshShape),
    ConvexHull(ConvexHullShape),
    Compound(Vec<(Transform, Box<ColliderShape>)>),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SphereShape {
    pub radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShape {
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShape {
    pub radius: f32,
    pub half_height: f32, // Height of cylindrical part (not including hemispheres)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaneShape {
    pub normal: Vec3,
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "TriMeshShapeData", from = "TriMeshShapeData")]
pub struct TriMeshShape {
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    pub indices: std::sync::Arc<Vec<u32>>,
    #[serde(skip)]
    pub bvh: std::sync::Arc<crate::bvh::BvhTree>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TriMeshShapeData {
    vertices: Vec<Vec3>,
    indices: Vec<u32>,
}

impl From<TriMeshShapeData> for TriMeshShape {
    fn from(mut data: TriMeshShapeData) -> Self {
        let bvh = crate::bvh::BvhTree::build(&data.vertices, &mut data.indices).unwrap_or_else(|e| {
            // A failed build (out-of-range index / u32 overflow) previously vanished into
            // `unwrap_or_default()`, leaving an empty BVH so trimesh collision & raycasts
            // silently degrade to a naive O(n) triangle scan. Surface it.
            tracing::warn!(
                error = %e,
                vertex_count = data.vertices.len(),
                index_count = data.indices.len(),
                "TriMesh BVH build failed; falling back to an empty BVH (naive O(n) triangle scan)"
            );
            crate::bvh::BvhTree::default()
        });
        Self {
            vertices: std::sync::Arc::new(data.vertices),
            indices: std::sync::Arc::new(data.indices),
            bvh: std::sync::Arc::new(bvh),
        }
    }
}

impl From<TriMeshShape> for TriMeshShapeData {
    fn from(shape: TriMeshShape) -> Self {
        Self {
            vertices: (*shape.vertices).clone(),
            indices: (*shape.indices).clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "ConvexHullShapeData", from = "ConvexHullShapeData")]
pub struct ConvexHullShape {
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    pub faces: std::sync::Arc<Vec<[u32; 3]>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ConvexHullShapeData {
    points: Vec<Vec3>, // These are raw points, we rebuild the hull on load
}

impl From<ConvexHullShapeData> for ConvexHullShape {
    fn from(data: ConvexHullShapeData) -> Self {
        let hull = crate::quickhull::compute_convex_hull(&data.points);
        Self {
            vertices: std::sync::Arc::new(hull.vertices),
            faces: std::sync::Arc::new(hull.faces),
        }
    }
}

impl From<ConvexHullShape> for ConvexHullShapeData {
    fn from(shape: ConvexHullShape) -> Self {
        Self {
            points: (*shape.vertices).clone(),
        }
    }
}

gizmo_core::impl_component!(Collider);

// ─────────────────────────────────────────────────────────────────────────────
// Tests — AABB per shape variant, volume/extent closed forms, serde round-trips
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn sphere_aabb_is_rotation_invariant() {
        let c = Collider::sphere(0.5);
        let pos = Vec3::new(1.0, 2.0, 3.0);
        let a0 = c.compute_aabb(pos, Quat::IDENTITY);
        let a1 = c.compute_aabb(pos, Quat::from_rotation_x(1.3) * Quat::from_rotation_y(0.7));
        // A sphere's AABB cannot depend on orientation.
        assert!((Vec3::from(a0.min) - Vec3::from(a1.min)).length() < EPS);
        assert!((Vec3::from(a0.max) - Vec3::from(a1.max)).length() < EPS);
        assert!((Vec3::from(a0.min) - Vec3::new(0.5, 1.5, 2.5)).length() < EPS);
        assert!((Vec3::from(a0.max) - Vec3::new(1.5, 2.5, 3.5)).length() < EPS);
    }

    #[test]
    fn box_aabb_grows_under_45deg_rotation() {
        let c = Collider::box_collider(Vec3::splat(1.0));
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let a = c.compute_aabb(Vec3::ZERO, rot);
        let he = Vec3::from(a.half_extents());
        // 45° about Z: the X/Y half-extents grow to √2; Z is unaffected.
        assert!((he.x - std::f32::consts::SQRT_2).abs() < 1e-3, "x he {}", he.x);
        assert!((he.y - std::f32::consts::SQRT_2).abs() < 1e-3, "y he {}", he.y);
        assert!((he.z - 1.0).abs() < 1e-3, "z he {}", he.z);
        // Still centred on the origin (symmetric corners).
        assert!(Vec3::from(a.center()).length() < EPS);
    }

    #[test]
    fn box_aabb_identity_equals_half_extents() {
        let c = Collider::box_collider(Vec3::new(1.0, 2.0, 3.0));
        let a = c.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
        assert!((Vec3::from(a.min) - Vec3::new(-1.0, -2.0, -3.0)).length() < EPS);
        assert!((Vec3::from(a.max) - Vec3::new(1.0, 2.0, 3.0)).length() < EPS);
    }

    #[test]
    fn capsule_aabb_tracks_axis_rotation() {
        let c = Collider::capsule(0.5, 2.0);
        // Upright: half-height along Y, plus the radius on every axis.
        let up = Vec3::from(c.compute_aabb(Vec3::ZERO, Quat::IDENTITY).half_extents());
        assert!((up - Vec3::new(0.5, 2.5, 0.5)).length() < 1e-3, "{up:?}");
        // Rotated 90° about Z → the capsule axis now lies along X.
        let side = Vec3::from(
            c.compute_aabb(Vec3::ZERO, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
                .half_extents(),
        );
        assert!((side - Vec3::new(2.5, 0.5, 0.5)).length() < 1e-3, "{side:?}");
    }

    #[test]
    fn plane_aabb_is_effectively_unbounded() {
        let c = Collider::plane(Vec3::Y, 0.0);
        let a = c.compute_aabb(Vec3::new(3.0, 0.0, 0.0), Quat::IDENTITY);
        assert!(Vec3::from(a.min).x <= 3.0 - 10000.0);
        assert!(Vec3::from(a.max).x >= 3.0 + 10000.0);
    }

    #[test]
    fn compound_offset_box_aabb_centres_on_offset() {
        let c = Collider::offset_box(Vec3::new(5.0, 0.0, 0.0), Vec3::splat(1.0));
        // No parent rotation: the sub-box sits at its local offset.
        let a = c.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
        assert!((Vec3::from(a.center()) - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-3);
        assert!((Vec3::from(a.half_extents()) - Vec3::splat(1.0)).length() < 1e-3);

        // A 90° parent rotation about Z carries the offset from +X to +Y.
        let ar = c.compute_aabb(Vec3::ZERO, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        assert!(
            (Vec3::from(ar.center()) - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-3,
            "offset must rotate with the compound, got {:?}",
            Vec3::from(ar.center())
        );
    }

    #[test]
    fn volume_matches_closed_forms() {
        use std::f32::consts::PI;
        assert!((Collider::sphere(1.0).volume() - (4.0 / 3.0) * PI).abs() < 1e-4);
        // Box: 8 · hx · hy · hz.
        assert!((Collider::box_collider(Vec3::new(1.0, 2.0, 3.0)).volume() - 48.0).abs() < 1e-4);
        // Capsule: cylinder (πr²·2h) + full sphere (4/3 πr³).
        let cap = Collider::capsule(1.0, 2.0).volume();
        let expected = PI * 1.0 * 4.0 + (4.0 / 3.0) * PI;
        assert!((cap - expected).abs() < 1e-3, "cap {cap}");
        // Plane returns a finite sentinel (not INF) so inertia math stays defined.
        assert_eq!(Collider::plane(Vec3::Y, 0.0).volume(), f32::MAX);
    }

    #[test]
    fn extents_y_per_shape() {
        assert!((Collider::sphere(0.75).extents_y() - 0.75).abs() < EPS);
        assert!((Collider::box_collider(Vec3::new(1.0, 2.0, 3.0)).extents_y() - 2.0).abs() < EPS);
        // Capsule half-height + radius.
        assert!((Collider::capsule(0.5, 2.0).extents_y() - 2.5).abs() < EPS);
        assert_eq!(Collider::plane(Vec3::Y, 0.0).extents_y(), 0.0);
    }

    #[test]
    fn collider_material_shortcuts_clamp() {
        // with_restitution clamps to [0,1]; with_friction clamps to >= 0 and mirrors
        // static == dynamic.
        let c = Collider::sphere(1.0).with_restitution(1.5).with_friction(-2.0);
        assert_eq!(c.material.restitution, 1.0);
        assert_eq!(c.material.static_friction, 0.0);
        assert_eq!(c.material.dynamic_friction, 0.0);
        let c2 = Collider::sphere(1.0).with_restitution(-0.5);
        assert_eq!(c2.material.restitution, 0.0);
    }

    #[test]
    fn trimesh_serde_roundtrip_rebuilds_bvh() {
        // A single triangle. TriMeshShape uses `#[serde(into/from = TriMeshShapeData)]`
        // with the BVH marked `#[serde(skip)]`, so deserialization must rebuild it.
        let data = TriMeshShapeData {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            indices: vec![0, 1, 2],
        };
        let shape = TriMeshShape::from(data);
        assert_eq!(*shape.vertices, vec![Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert_eq!(*shape.indices, vec![0, 1, 2]);
        assert!(
            !shape.bvh.nodes.is_empty(),
            "BVH is #[serde(skip)] and must be rebuilt on load"
        );

        // Back to the serialized form: geometry is preserved exactly.
        let back = TriMeshShapeData::from(shape);
        assert_eq!(back.vertices, vec![Vec3::ZERO, Vec3::X, Vec3::Y]);
        assert_eq!(back.indices, vec![0, 1, 2]);
    }

    #[test]
    fn convexhull_serde_roundtrip_is_idempotent() {
        // The serialized form keeps only raw points and rebuilds the hull on load.
        // Rebuilding a hull from its OWN vertices must reproduce the same vertex set.
        let corners: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                )
            })
            .collect();
        let hull = crate::quickhull::compute_convex_hull(&corners);
        let shape = ConvexHullShape {
            vertices: std::sync::Arc::new(hull.vertices),
            faces: std::sync::Arc::new(hull.faces),
        };
        assert_eq!(shape.vertices.len(), 8);

        let data = ConvexHullShapeData::from(shape.clone());
        let rebuilt = ConvexHullShape::from(data);
        assert_eq!(
            rebuilt.vertices.len(),
            shape.vertices.len(),
            "rebuilding a hull from its own vertices must be idempotent"
        );
    }

    #[test]
    fn from_shape_uses_defaults() {
        // `from_shape` is the canonical bare-shape constructor for the #[non_exhaustive]
        // struct: default material + layer, not a trigger.
        let c = Collider::from_shape(ColliderShape::Sphere(SphereShape { radius: 2.0 }));
        assert!(!c.is_trigger);
        assert_eq!(c.material, PhysicsMaterial::default());
        assert_eq!(c.collision_layer, CollisionLayer::default());
    }
}
