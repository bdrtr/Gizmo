use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use super::{CollisionLayer, PhysicsMaterial, Transform};

/// The collision geometry attached to a body, together with the surface response and
/// the filtering rules that apply to it.
///
/// Everything is metres, in the collider's own frame, whose origin is the body's
/// [`Transform`] origin — **not** its centre of mass. `Transform::scale` is not applied
/// to collider geometry anywhere in `gizmo-physics-core` or `gizmo-physics-rigid`, so
/// resizing means building a new shape, not scaling the entity.
///
/// The struct is `#[non_exhaustive]` — see [`Collider::from_shape`] for how to build one
/// from outside this crate. [`Default`] is a 0.5 m sphere with the default material and
/// layer and `is_trigger = false`; it is a placeholder to spread with `..`, not a
/// recommendation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Collider {
    /// The geometry, in the collider's local frame.
    ///
    /// Nothing re-derives from this behind your back. In particular a rigid body's
    /// inertia tensor and centre of mass come from an explicit
    /// `RigidBody::update_inertia_from_collider` call in `gizmo-physics-rigid` (made at
    /// construction time by its callers), so swapping the shape at runtime leaves those
    /// stale until you call it again.
    pub shape: ColliderShape,
    /// Report overlaps, but never push anything.
    ///
    /// A pair where *either* side sets this is diverted in the rigid-body pipeline: it
    /// emits `TriggerEvent`s (`Started` / `Persisting` / `Ended`) and no contact
    /// manifold is built, so the solver never sees it. Layer filtering still applies
    /// first — a trigger is not exempt from `collision_layer`.
    pub is_trigger: bool,
    /// Friction, restitution and density for this collider's surface.
    ///
    /// A contact never uses one collider's material on its own: the pair is folded
    /// through [`PhysicsMaterial::combine`], where the friction and restitution
    /// [`CombineMode`](crate::components::CombineMode)s of *both* materials are resolved
    /// against each other (density is averaged unconditionally). So a slippery collider
    /// does not by itself make a slippery contact.
    pub material: PhysicsMaterial,
    /// Which layer this collider sits on, and which layers it is willing to touch — see
    /// [`CollisionLayer`] for the filtering rule and
    /// [`CollisionLayer::can_collide_with`] for the test itself.
    ///
    /// A [`Collider`] built by any of this type's constructors gets
    /// [`CollisionLayer::default`] — layer 0, mask `u32::MAX` — so filtering is opt-in:
    /// until you call [`Collider::with_layer`], this field excludes nothing.
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

/// Axis-aligned bounds of a vertex list, in its own space. Empty gives a degenerate box at the
/// origin rather than the inverted infinities a bare fold would leave, because an inverted AABB
/// silently overlaps nothing in the broadphase.
fn local_bounds(vertices: &[Vec3]) -> gizmo_math::Aabb {
    if vertices.is_empty() {
        return gizmo_math::Aabb::new(Vec3::ZERO, Vec3::ZERO);
    }
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for v in vertices {
        min = min.min(*v);
        max = max.max(*v);
    }
    gizmo_math::Aabb::new(min, max)
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

    /// Calculate AABB for this collider at given transform.
    ///
    /// `position` / `rotation` are the body's [`Transform`] placement — the collider's
    /// local origin, not its centre of mass — and the returned box is world-space. Scale
    /// is not a parameter and is never applied.
    ///
    /// The box is **conservative**: it always contains the shape, but it is not guaranteed
    /// to be the tightest such box — a rotated [`ColliderShape::TriMesh`] in particular is
    /// bounded through the corners of its cached local box (see [`TriMeshShape::local_aabb`])
    /// rather than through its triangles. That is the safe direction for a broadphase — too
    /// big loses time, too small loses contacts.
    ///
    /// Degenerate cases, none of which panic:
    /// - a [`ColliderShape::Plane`] is not actually unbounded here; it reports a
    ///   ±10 000 m cube centred on `position`, which is finite and so, unlike an
    ///   `INFINITY` bound, still participates in min/max folds and overlap tests;
    /// - an empty [`ColliderShape::TriMesh`] gives a single point at `position` (a warn
    ///   is logged), because an inverted `(+inf, -inf)` box would silently overlap
    ///   nothing at all;
    /// - an empty [`ColliderShape::ConvexHull`] or [`ColliderShape::Compound`] *does*
    ///   return that inverted box, and logs a warning — check before feeding it onward.
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
                    // Was an inverted (+inf, -inf) box, which this file already recorded as
                    // "silently breaks broadphase overlap tests". A degenerate box at the body's
                    // position overlaps nothing either, and does it without poisoning a min/max
                    // fold somewhere upstream.
                    tracing::warn!("TriMesh collider has no vertices; its AABB is a point");
                    return gizmo_math::Aabb::new(position, position);
                }
                // Eight corners of the box measured when the collider was built, rather than
                // every vertex transformed — see `TriMeshShape::local_aabb`. Exact for an
                // axis-aligned rotation and a conservative superset otherwise, which is what a
                // broadphase bound is allowed to be.
                let (lo, hi) = (Vec3::from(tm.local_aabb.min), Vec3::from(tm.local_aabb.max));
                let mut min = Vec3::splat(f32::INFINITY);
                let mut max = Vec3::splat(f32::NEG_INFINITY);
                for i in 0..8 {
                    let corner = Vec3::new(
                        if i & 1 == 0 { lo.x } else { hi.x },
                        if i & 2 == 0 { lo.y } else { hi.y },
                        if i & 4 == 0 { lo.z } else { hi.z },
                    );
                    let p = position + rotation * corner;
                    min = min.min(p);
                    max = max.max(p);
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

    /// A half-space bounded by the plane `dot(x, normal) == distance`, with `normal`
    /// pointing out of the solid side.
    ///
    /// Both arguments are consumed in **world space**: the plane paths in this crate do
    /// not apply the body's position or rotation to them (see [`PlaneShape`]), so a
    /// plane collider does not follow its entity's [`Transform`].
    ///
    /// Pass a unit-length `normal` — nothing normalizes it, and `distance` is then
    /// metres along it from the world origin.
    pub fn plane(normal: Vec3, distance: f32) -> Self {
        Self {
            shape: ColliderShape::Plane(PlaneShape { normal, distance }),
            ..Default::default()
        }
    }

    /// A sphere of `radius` metres centred on the collider's local origin.
    ///
    /// The cheapest shape in every phase, and its AABB does not depend on the body's
    /// orientation. `radius` is not validated; zero or negative collapses the shape
    /// rather than erroring.
    pub fn sphere(radius: f32) -> Self {
        Self {
            shape: ColliderShape::Sphere(SphereShape { radius }),
            ..Default::default()
        }
    }

    /// A box centred on the collider's local origin, given as **half**-extents in metres
    /// — the full size is `2 * half_extents`.
    ///
    /// Box–box and box–plane contacts have dedicated analytic paths rather than going
    /// through GJK/EPA, so this is the shape to prefer for crates, walls and floors.
    pub fn box_collider(half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Box(BoxShape { half_extents }),
            ..Default::default()
        }
    }

    /// A box whose centre sits `offset` metres from the collider's local origin, built as
    /// a one-part [`ColliderShape::Compound`].
    ///
    /// Use it when the body's origin has to stay where it is — a wheel hub, a mesh pivot
    /// at the feet — and the collider belongs somewhere else. `offset` rotates with the
    /// body.
    ///
    /// The offset is not free: `RigidBody::update_inertia_from_collider` in
    /// `gizmo-physics-rigid` derives a compound's centre of mass from its parts'
    /// positions, so a single offset part moves the body's centre of mass to `offset`.
    /// That is usually what you want, and it is why an offset box is not the same thing
    /// as moving the entity.
    pub fn offset_box(offset: Vec3, half_extents: Vec3) -> Self {
        Self {
            shape: ColliderShape::Compound(vec![(
                Transform::new(offset),
                Box::new(ColliderShape::Box(BoxShape { half_extents })),
            )]),
            ..Default::default()
        }
    }

    /// A capsule along the collider's local **+Y** axis, centred on the local origin.
    ///
    /// `half_height` is the half-length of the *cylindrical* section only, so the total
    /// length is `2 * (half_height + radius)` metres. `half_height == 0.0` degenerates to
    /// a sphere of `radius` rather than to nothing.
    ///
    /// The +Y convention is baked in: to lay a capsule on its side, rotate the body.
    pub fn capsule(radius: f32, half_height: f32) -> Self {
        Self {
            shape: ColliderShape::Capsule(CapsuleShape {
                radius,
                half_height,
            }),
            ..Default::default()
        }
    }

    /// The convex hull of `points`, computed now via [`crate::quickhull`].
    ///
    /// `points` are in the collider's local frame (metres) and need no ordering: interior
    /// points are discarded, so only hull vertices survive into the shape. Fewer than four
    /// points — or a degenerate, near-coplanar set — produces a shape with the points but
    /// **no faces**, which GJK/EPA still works with, while [`crate::raycast::Raycast`]
    /// falls back to a bounding-box approximation of it.
    ///
    /// Quickhull here deliberately uses ordered containers, so a given input produces the
    /// same vertex and face ordering on every run of the same build. As everywhere in
    /// this engine, that is a same-platform guarantee only.
    ///
    /// The hull is built once, here, and stored behind `Arc`s, so cloning the resulting
    /// [`Collider`] afterwards does not copy the geometry.
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

    /// A **static triangle mesh** collider: terrain, a track surface, a level's geometry.
    ///
    /// Takes the mesh by value because building the BVH **reorders the indices** — the tree's
    /// `first_tri_index` addresses that reordered array, so the two must travel together and a
    /// caller keeping its own copy would be keeping the wrong one.
    ///
    /// The mesh is the one shape that is genuinely *concave*, which is the whole reason it exists
    /// as a shape rather than as a convex hull. See [`crate::narrowphase::NarrowPhase`]: a pair
    /// involving one is dispatched per-triangle, not through GJK over the whole thing.
    ///
    /// Meant for **static** bodies. There is no inertia tensor for an arbitrary mesh here, and a
    /// dynamic one would be asking the solver a question this shape cannot answer.
    ///
    /// A failed BVH build (an index past the end, a mesh past `u32::MAX` triangles) leaves an
    /// empty tree and logs it. That degrades to "collides with nothing" rather than to a panic or,
    /// worse, a silent O(n) scan nobody notices until the frame budget is gone.
    pub fn trimesh(vertices: Vec<Vec3>, mut indices: Vec<u32>) -> Self {
        let bvh = crate::bvh::BvhTree::build(&vertices, &mut indices).unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                vertex_count = vertices.len(),
                index_count = indices.len(),
                "TriMesh collider BVH build failed; this mesh will collide with nothing"
            );
            crate::bvh::BvhTree::default()
        });
        let local_aabb = local_bounds(&vertices);
        Self {
            shape: ColliderShape::TriMesh(TriMeshShape {
                vertices: std::sync::Arc::new(vertices),
                indices: std::sync::Arc::new(indices),
                bvh: std::sync::Arc::new(bvh),
                local_aabb,
            }),
            ..Default::default()
        }
    }

    /// Sets [`Collider::is_trigger`] in a builder chain — see that field for what a
    /// trigger pair does instead of colliding. Takes the flag as an argument rather than
    /// asserting it, so a caller can forward a config value without branching.
    pub fn with_trigger(mut self, is_trigger: bool) -> Self {
        self.is_trigger = is_trigger;
        self
    }

    /// Replaces the whole [`PhysicsMaterial`] — including `density` and both
    /// [`CombineMode`](crate::components::CombineMode)s, which the narrower
    /// [`Collider::with_restitution`] / [`Collider::with_friction`] shortcuts leave
    /// alone. Call this first if you mean to combine the two.
    ///
    /// Unlike those shortcuts, nothing is clamped here: the material is stored verbatim.
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
    /// Back-compatibility alias for [`Collider::box_collider`], kept for older call
    /// sites. The name is historical and misleading: the shape is an oriented box that
    /// rotates with the body, not an axis-aligned one.
    pub fn aabb(half_extents: Vec3) -> Self {
        Self::box_collider(half_extents)
    }

    /// Back-compatibility alias for [`Collider::sphere`], identical in every respect.
    pub fn new_sphere(radius: f32) -> Self {
        Self::sphere(radius)
    }

    /// Back-compatibility alias for [`Collider::box_collider`] with the extents spread
    /// over three arguments. `x`, `y`, `z` are **half**-extents, so this is a box of full
    /// size `2x × 2y × 2z` — the name reads like full dimensions and is not.
    pub fn new_aabb(x: f32, y: f32, z: f32) -> Self {
        Self::box_collider(Vec3::new(x, y, z))
    }

    /// Back-compatibility alias for [`Collider::capsule`]; same local +Y axis and the
    /// same cylinder-only `half_height`.
    pub fn new_capsule(radius: f32, half_height: f32) -> Self {
        Self::capsule(radius, half_height)
    }

    /// Replaces both halves of [`Collider::collision_layer`] — the layer this collider
    /// is on *and* its acceptance mask. To change only one, build the [`CollisionLayer`]
    /// yourself; the default `CollisionLayer::new(n)` accepts every layer.
    pub fn with_layer(mut self, layer: CollisionLayer) -> Self {
        self.collision_layer = layer;
        self
    }

    /// Volume in cubic metres, used for buoyancy and for weighting the parts of a
    /// compound when its mass is distributed.
    ///
    /// Exact for the primitives (sphere, box, capsule as cylinder + one full sphere).
    /// **Approximate** for [`ColliderShape::TriMesh`], [`ColliderShape::ConvexHull`] and
    /// [`ColliderShape::Compound`]: half the volume of the unrotated local AABB, which
    /// for a compound spans the empty space between its parts as well.
    ///
    /// A [`ColliderShape::Plane`] returns `f32::MAX`, not `INFINITY` — a finite sentinel,
    /// deliberately chosen so downstream arithmetic stays defined where `INFINITY` would
    /// have produced a `NaN` (`INFINITY * 0.0`). It is not a measurement; a half-space
    /// has no finite volume.
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

    /// Half the shape's extent along the collider's local **Y** axis, in metres.
    ///
    /// Orientation is not a parameter and is not accounted for, so this is the *unrotated*
    /// half-height: a body lying on its side still reports its upright figure. It is what
    /// the buoyancy path uses to estimate how deep a body is submerged, where the
    /// approximation is tolerable; it may not be elsewhere.
    ///
    /// For the primitives the shape is centred on the local origin, so `y ± extents_y()`
    /// brackets it. For [`ColliderShape::TriMesh`], [`ColliderShape::ConvexHull`] and
    /// [`ColliderShape::Compound`] this is half the local AABB's Y span, which need not be
    /// centred on the origin — an offset compound's true top and bottom are not
    /// symmetric about it.
    ///
    /// A [`ColliderShape::Plane`] reports `0.0`: no thickness. Note that the same shape
    /// reports `f32::MAX` from [`Collider::volume`] — the two functions do not treat a
    /// half-space the same way.
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

/// The geometry of a [`Collider`], in the collider's local frame (metres).
///
/// Not every variant travels the same road through the narrowphase. Sphere–sphere,
/// sphere–plane, box–plane and box–box have dedicated analytic paths; every other pair —
/// `Capsule` and `ConvexHull` included — goes through the generic support-point and
/// GJK/EPA routes (the module table on [`crate::narrowphase`] lists them). `TriMesh` is
/// dispatched per triangle, because the GJK fallback would collide against its convex hull
/// instead of the mesh. `Plane` and `Compound` are handled entirely by dedicated paths, and
/// reaching [`Gjk::support_point`](crate::gjk::Gjk::support_point) with either is a contract
/// violation — see [`Gjk`](crate::gjk::Gjk) for what that costs.
///
/// Which variant you pick therefore decides more than the silhouette: see
/// [`Collider::trimesh`] for the one shape that is genuinely concave, and
/// [`Collider::volume`] for which variants only approximate their own mass.
// NOT `#[non_exhaustive]`: the engine's own crates (gizmo-physics-rigid) match
// this exhaustively to compute inertia / AABB / narrowphase dispatch. Adding a
// new collider shape is inherently a breaking change (it needs solver support),
// so a major version bump is appropriate — and exhaustive matching lets the
// compiler flag every site that must handle the new shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ColliderShape {
    /// A sphere about the local origin — the only variant with no orientation of its own,
    /// so the body's rotation cancels out of its geometry. See [`Collider::sphere`].
    Sphere(SphereShape),
    /// A box that turns with the body — an oriented box, despite what the
    /// [`Collider::aabb`] alias name suggests.
    Box(BoxShape),
    /// A capsule whose axis is the local +Y direction; rotate the body to lay it down.
    Capsule(CapsuleShape),
    /// An infinite half-space — see [`PlaneShape`] for the frame its two fields live in.
    /// Its AABB is a large finite box, not an unbounded one.
    Plane(PlaneShape),
    /// A static triangle mesh — the only variant whose own geometry may be concave,
    /// which is why it is dispatched per triangle through its BVH instead of as one
    /// solid. Meant for static bodies; there is no real inertia tensor for it. Build
    /// with [`Collider::trimesh`].
    TriMesh(TriMeshShape),
    /// A convex polyhedron, handled by GJK/EPA over its vertices. Build with
    /// [`Collider::convex_hull`], which discards interior points.
    ConvexHull(ConvexHullShape),
    /// Several sub-shapes, each placed by a [`Transform`] relative to the *parent
    /// collider's* origin.
    ///
    /// Only `position` and `rotation` of that transform are read — `scale` is ignored,
    /// as it is everywhere else in collision. The narrowphase and AABB paths recurse
    /// into each part rather than treating the union as one solid, so a compound is
    /// never reduced to its convex hull; parts are free to overlap, and where they do,
    /// each of them can contribute its own contacts.
    Compound(Vec<(Transform, Box<ColliderShape>)>),
}

/// A sphere centred on the collider's local origin. See [`Collider::sphere`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SphereShape {
    /// Radius in metres. Not validated at construction or on deserialize: zero and
    /// negative values are stored as given and yield a degenerate shape rather than an
    /// error.
    pub radius: f32,
}

/// A box centred on the collider's local origin, oriented by the body's rotation.
/// See [`Collider::box_collider`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxShape {
    /// Half the box's size on each local axis, in metres — the box spans
    /// `-half_extents ..= +half_extents`, so its full size is twice this.
    pub half_extents: Vec3,
}

/// A capsule aligned with the collider's local +Y axis, centred on the local origin.
/// See [`Collider::capsule`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CapsuleShape {
    /// Radius of the cylinder and of the two hemispherical caps, in metres. The caps are
    /// not extra geometry you can shape independently — this one number rounds both ends.
    pub radius: f32,
    /// Distance in metres from the local origin to either cap-sphere centre — that is, half
    /// the cylindrical section, caps excluded; see [`Collider::capsule`] for the total
    /// length. Not validated at construction or on deserialize.
    pub half_height: f32, // Height of cylindrical part (not including hemispheres)
}

/// An infinite half-space: everything with `dot(x, normal) < distance` is inside.
///
/// Both fields are **world**-space and are read as-is by the plane paths in
/// [`crate::narrowphase`] and [`crate::raycast`] — the owning body's position and
/// rotation are not applied to them, so moving or turning the entity does not move the
/// surface. Build one with [`Collider::plane`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaneShape {
    /// Outward direction of the solid half-space. Assumed unit length: nothing normalizes
    /// it, and a non-unit normal rescales both the meaning of `distance` and the
    /// penetration depths reported for contacts against this plane.
    pub normal: Vec3,
    /// Signed offset of the plane from the world origin along `normal`, in metres — the
    /// plane is `dot(x, normal) == distance`. A ground plane at y = 0 is `normal = +Y`,
    /// `distance = 0.0`; raising the ground to y = 2 means `distance = 2.0`.
    pub distance: f32,
}

/// An indexed triangle mesh with a BVH over it — the one collider shape that is allowed
/// to be concave. Build it with [`Collider::trimesh`], which is the only way to get the
/// four fields into a consistent state.
///
/// `vertices`, `indices` and `bvh` are a package: the tree addresses `indices`, `indices`
/// addresses `vertices`, and the build **reorders** `indices` in place. Constructing this
/// struct by hand from a mesh and a separately-built tree is how you get a mesh that
/// collides against the wrong triangles.
///
/// Everything is stored behind `Arc`, so cloning a mesh collider shares the geometry
/// rather than copying it. Serialization keeps only `vertices` and `indices`; the tree
/// and the cached bounds are rebuilt on load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "TriMeshShapeData", from = "TriMeshShapeData")]
pub struct TriMeshShape {
    /// Mesh vertices in the collider's local frame, in metres. Positions only — no
    /// normals, no UVs; this is collision geometry, not a renderable mesh.
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    /// Triangle corners, three consecutive entries per triangle, indexing `vertices`.
    ///
    /// **Not in the order you passed in.** The BVH build permutes whole triangles so
    /// each leaf owns a contiguous run, and `bvh`'s leaf ranges are offsets into *this*
    /// array. Any index `>= vertices.len()` makes the build fail rather than panic
    /// later, and a length that is not a multiple of three rounds down — the trailing
    /// one or two entries are left out of the tree.
    pub indices: std::sync::Arc<Vec<u32>>,
    /// Bounding-volume hierarchy over the triangles, never stored in a scene file and
    /// rebuilt from `vertices`/`indices` when the shape is deserialized.
    ///
    /// An **empty** tree is the failure state, and the two consumers disagree about what
    /// it means: mesh contact generation queries the tree and so finds nothing at all —
    /// the mesh collides with nothing — while [`crate::raycast::Raycast`] and
    /// [`Gjk::support_point`](crate::gjk::Gjk::support_point) notice the empty tree and
    /// fall back to a linear scan over every triangle or vertex, staying correct but
    /// losing the acceleration. A mesh that raycasts fine yet nothing lands on is the
    /// signature of a failed build.
    #[serde(skip)]
    pub bvh: std::sync::Arc<crate::bvh::BvhTree>,
    /// The mesh's bounds in its own space, measured once when the collider is built.
    ///
    /// [`Collider::compute_aabb`] used to fold over every vertex, transforming each one. The
    /// broadphase asks for that AABB several times per body per substep — the vehicle alone asks
    /// four times per wheel per step — so a static mesh with real triangle counts spent the frame
    /// budget re-deriving a number that cannot change. With the local box cached, the world AABB
    /// is eight corners rotated, whatever the mesh.
    #[serde(skip)]
    pub local_aabb: gizmo_math::Aabb,
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
        let local_aabb = local_bounds(&data.vertices);
        Self {
            vertices: std::sync::Arc::new(data.vertices),
            indices: std::sync::Arc::new(data.indices),
            bvh: std::sync::Arc::new(bvh),
            local_aabb,
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

/// A convex polyhedron: the hull vertices, plus the triangles that bound them.
///
/// Build it with [`Collider::convex_hull`] rather than by hand — nothing here checks that
/// the vertices are actually in convex position, and GJK/EPA will happily produce contacts
/// for a non-convex vertex set as though it were its own hull.
///
/// Serialization keeps only the vertices; the faces are recomputed by re-running
/// Quickhull on load, which is idempotent for a real hull because re-hulling a hull's own
/// vertices reproduces them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(into = "ConvexHullShapeData", from = "ConvexHullShapeData")]
pub struct ConvexHullShape {
    /// Hull vertices in the collider's local frame, in metres, with interior points
    /// already discarded by the hull build.
    ///
    /// This is what the narrowphase actually uses: [`crate::gjk`] takes its support point
    /// by scanning every entry, so contact cost grows with the vertex count. Simplify the
    /// point cloud before building, not after.
    pub vertices: std::sync::Arc<Vec<Vec3>>,
    /// Bounding triangles as index triples into `vertices`, wound so the triangle normal
    /// points out of the hull.
    ///
    /// Used only by [`crate::raycast::Raycast`], which needs real triangles to hit;
    /// GJK/EPA ignores them. **Legitimately empty** for a hull built from fewer than four
    /// points or from a degenerate (collinear/coplanar) set — the raycast path detects
    /// that and falls back to the vertices' bounding box, which reports hits on the
    /// corners of a box the hull does not fill.
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

    /// `Collider::trimesh` builds a usable tree, and keeps the indices the tree was built
    /// against — the build reorders them, so handing back the caller's original would address the
    /// wrong triangles.
    #[test]
    fn a_trimesh_collider_keeps_the_indices_its_tree_was_built_on() {
        // Two quads far apart, so the builder actually splits and reorders.
        let mut v = Vec::new();
        let mut i = Vec::new();
        for cx in [0.0f32, 50.0] {
            let b = v.len() as u32;
            v.extend([
                Vec3::new(cx - 1.0, 0.0, -1.0),
                Vec3::new(cx + 1.0, 0.0, -1.0),
                Vec3::new(cx + 1.0, 0.0, 1.0),
                Vec3::new(cx - 1.0, 0.0, 1.0),
            ]);
            i.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        let c = Collider::trimesh(v.clone(), i.clone());
        let ColliderShape::TriMesh(tm) = &c.shape else { panic!("not a trimesh") };
        assert!(!tm.bvh.nodes.is_empty(), "the tree was built");
        assert_eq!(tm.vertices.len(), v.len());
        assert_eq!(tm.indices.len(), i.len());
        // Every triangle the tree addresses resolves through the indices it kept.
        let mut tris = Vec::new();
        tm.bvh.query_aabb(gizmo_math::Aabb::new(Vec3::splat(-100.0), Vec3::splat(100.0)), &mut tris);
        assert_eq!(tris.len(), 4, "all four triangles are reachable");
        for t in tris {
            for k in 0..3 {
                let idx = tm.indices[t as usize * 3 + k] as usize;
                assert!(idx < tm.vertices.len(), "index {idx} is past the vertices");
            }
        }
    }

    /// A mesh the BVH cannot be built for collides with nothing, and does not panic.
    #[test]
    fn a_broken_trimesh_degrades_to_an_empty_tree() {
        let v = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        let c = Collider::trimesh(v, vec![0, 1, 9]); // 9 is past the end
        let ColliderShape::TriMesh(tm) = &c.shape else { panic!("not a trimesh") };
        assert!(tm.bvh.nodes.is_empty(), "a failed build leaves an empty tree, not a bad one");
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

#[cfg(test)]
mod trimesh_aabb_tests {
    use super::*;
    use gizmo_math::Quat;

    /// A tetrahedron with distinct extents on every axis, so a swapped axis cannot pass.
    fn tetra() -> (Vec<Vec3>, Vec<u32>) {
        let v = vec![
            Vec3::new(-1.0, -2.0, -3.0),
            Vec3::new(4.0, -2.0, -3.0),
            Vec3::new(-1.0, 5.0, -3.0),
            Vec3::new(-1.0, -2.0, 6.0),
        ];
        (v, vec![0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3])
    }

    #[test]
    fn an_unrotated_trimesh_reports_its_own_bounds() {
        let (v, i) = tetra();
        let c = Collider::trimesh(v, i);
        let aabb = c.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
        assert_eq!(aabb.min, Vec3::new(-1.0, -2.0, -3.0).into());
        assert_eq!(aabb.max, Vec3::new(4.0, 5.0, 6.0).into());
    }

    #[test]
    fn translation_moves_the_box_and_nothing_else() {
        let (v, i) = tetra();
        let c = Collider::trimesh(v, i);
        let at = Vec3::new(100.0, -50.0, 7.5);
        let aabb = c.compute_aabb(at, Quat::IDENTITY);
        assert_eq!(aabb.min, (Vec3::new(-1.0, -2.0, -3.0) + at).into());
        assert_eq!(aabb.max, (Vec3::new(4.0, 5.0, 6.0) + at).into());
    }

    /// The contract the cached box has to keep: a broadphase bound may be conservative, but it may
    /// never miss a vertex. Folding the corners of a box is a superset of folding the vertices;
    /// this is what says the superset is a real one and not a shifted box.
    #[test]
    fn a_rotated_box_still_contains_every_vertex() {
        let (v, i) = tetra();
        let c = Collider::trimesh(v.clone(), i);
        for (axis, angle) in [
            (Vec3::Y, 0.7_f32),
            (Vec3::X, -1.2),
            (Vec3::new(1.0, 1.0, 1.0).normalize(), 2.4),
        ] {
            let q = Quat::from_axis_angle(axis, angle);
            let at = Vec3::new(3.0, -4.0, 5.0);
            let aabb = c.compute_aabb(at, q);
            for p in &v {
                let w = at + q * *p;
                assert!(
                    w.x >= aabb.min.x - 1e-4
                        && w.y >= aabb.min.y - 1e-4
                        && w.z >= aabb.min.z - 1e-4
                        && w.x <= aabb.max.x + 1e-4
                        && w.y <= aabb.max.y + 1e-4
                        && w.z <= aabb.max.z + 1e-4,
                    "{w:?} escaped {aabb:?} under {axis:?}/{angle}"
                );
            }
        }
    }

    /// An empty mesh must not produce the inverted box a bare fold leaves, which overlaps nothing.
    #[test]
    fn an_empty_trimesh_is_degenerate_rather_than_inverted() {
        let c = Collider::trimesh(Vec::new(), Vec::new());
        let aabb = c.compute_aabb(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY);
        assert!(aabb.min.x <= aabb.max.x && aabb.min.y <= aabb.max.y && aabb.min.z <= aabb.max.z);
    }

    /// The cached box is measured once, so a mesh with a great many vertices costs the same as a
    /// small one. Asserted as a ratio rather than an absolute time: the point is that it stopped
    /// scaling, not how fast the machine is.
    #[test]
    fn the_cost_no_longer_scales_with_the_mesh() {
        let build = |n: u32| {
            let v: Vec<Vec3> = (0..n).map(|k| Vec3::new(k as f32, (k % 7) as f32, (k % 13) as f32)).collect();
            let i: Vec<u32> = (0..n.saturating_sub(2)).flat_map(|k| [k, k + 1, k + 2]).collect();
            Collider::trimesh(v, i)
        };
        let small = build(64);
        let large = build(200_000);
        let q = Quat::from_axis_angle(Vec3::Y, 0.3);

        let time = |c: &Collider| {
            let t = std::time::Instant::now();
            for _ in 0..2_000 {
                std::hint::black_box(c.compute_aabb(Vec3::ZERO, q));
            }
            t.elapsed()
        };
        let (a, b) = (time(&small), time(&large));
        // Generous, because this runs on whatever CI has: before the cache the large mesh was
        // three thousand times the work, so anything under 10x is the difference being measured.
        assert!(
            b.as_nanos() < a.as_nanos().max(1) * 10,
            "200k vertices cost {b:?} against {a:?} for 64 — the AABB is being recomputed"
        );
    }
}
