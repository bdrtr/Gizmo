use super::*;

impl Gjk {
    /// Get support point for a shape in a given direction
    pub fn support_point(
        shape: &ColliderShape,
        pos: Vec3,
        rot: gizmo_math::Quat,
        dir: Vec3,
    ) -> Vec3 {
        let local_dir = rot.inverse() * dir;

        let local_support = match shape {
            ColliderShape::Sphere(s) => Self::sphere_support(s, local_dir),
            ColliderShape::Box(b) => Self::box_support(b, local_dir),
            ColliderShape::Capsule(c) => Self::capsule_support(c, local_dir),
            ColliderShape::Cylinder(c) => Self::cylinder_support(c, local_dir),
            ColliderShape::Plane(_) => {
                // Contract violation: planes are handled by dedicated analytic paths and must
                // never reach GJK support. In debug this aborts; in release it would silently
                // return ZERO and corrupt the collision result, so log it either way.
                tracing::error!(
                    "Plane shape reached GJK support_point; planes require dedicated collision detection (returning ZERO)"
                );
                debug_assert!(false, "Plane shapes must use separate collision detection");
                Vec3::ZERO
            }
            ColliderShape::Heightfield(_) => {
                // Same contract as `Plane`, for the opposite reason: a heightfield is CONCAVE,
                // so its support function would describe the convex hull of the terrain — a lid
                // over every valley. Contact generation dispatches it per cell instead
                // (`NarrowPhase::shape_heightfield`), and reaching here means a pair slipped
                // past that dispatch.
                tracing::error!(
                    "Heightfield reached GJK support_point; terrain is dispatched per cell (returning ZERO)"
                );
                debug_assert!(false, "Heightfield must be dispatched per cell, not through GJK");
                Vec3::ZERO
            }
            ColliderShape::TriMesh(tm) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;

                if !tm.bvh.nodes.is_empty() {
                    let mut stack = Vec::with_capacity(64);
                    stack.push(0);

                    let abs_dir = gizmo_math::Vec3A::new(
                        local_dir.x.abs(),
                        local_dir.y.abs(),
                        local_dir.z.abs(),
                    );
                    let dir_a = gizmo_math::Vec3A::new(local_dir.x, local_dir.y, local_dir.z);

                    while let Some(node_idx) = stack.pop() {
                        let node = &tm.bvh.nodes[node_idx];

                        let center = node.aabb.center();
                        let half_extents = node.aabb.half_extents();

                        let max_node_dot = center.dot(dir_a)
                            + half_extents.x * abs_dir.x
                            + half_extents.y * abs_dir.y
                            + half_extents.z * abs_dir.z;

                        if max_node_dot < best_dot {
                            continue;
                        }

                        if node.is_leaf() {
                            let start = (node.first_tri_index * 3) as usize;
                            let end = start + (node.tri_count * 3) as usize;
                            for i in start..end {
                                let v = tm.vertices[tm.indices[i] as usize];
                                let d = v.dot(local_dir);
                                if d > best_dot {
                                    best_dot = d;
                                    best_pt = v;
                                }
                            }
                        } else {
                            if node.left_child >= 0 {
                                stack.push(node.left_child as usize);
                            }
                            if node.right_child >= 0 {
                                stack.push(node.right_child as usize);
                            }
                        }
                    }
                } else {
                    for v in tm.vertices.iter() {
                        let d = v.dot(local_dir);
                        if d > best_dot {
                            best_dot = d;
                            best_pt = *v;
                        }
                    }
                }
                best_pt
            }
            ColliderShape::ConvexHull(ch) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;
                for v in ch.vertices.iter() {
                    let d = v.dot(local_dir);
                    if d > best_dot {
                        best_dot = d;
                        best_pt = *v;
                    }
                }
                best_pt
            }
            crate::components::ColliderShape::Compound(_) => {
                // Contract violation: compounds are decomposed into sub-shapes before GJK.
                // Log so a release build does not silently return ZERO (a corrupt support).
                tracing::error!(
                    "Compound shape reached GJK support_point; compounds must be decomposed before collision detection (returning ZERO)"
                );
                debug_assert!(
                    false,
                    "Compound shapes must use separate collision detection"
                );
                Vec3::ZERO
            }
        };

        pos + rot * local_support
    }

    fn sphere_support(sphere: &SphereShape, dir: Vec3) -> Vec3 {
        dir.try_normalize().unwrap_or(Vec3::X) * sphere.radius
    }

    fn box_support(box_shape: &BoxShape, dir: Vec3) -> Vec3 {
        Vec3::new(
            if dir.x > 0.0 {
                box_shape.half_extents.x
            } else {
                -box_shape.half_extents.x
            },
            if dir.y > 0.0 {
                box_shape.half_extents.y
            } else {
                -box_shape.half_extents.y
            },
            if dir.z > 0.0 {
                box_shape.half_extents.z
            } else {
                -box_shape.half_extents.z
            },
        )
    }

    /// Support point of a solid cylinder about local +Y.
    ///
    /// The two halves are independent, which is the whole difference from a capsule: the axial
    /// coordinate jumps to whichever flat end `dir` points at, and the radial part goes to the
    /// rim in the direction's own XZ plane. Their combination is the **rim**, and that is what
    /// makes a cylinder stand on its end where a capsule of the same size rocks: GJK can return
    /// any point of the circular edge, so the contact manifold spreads around it.
    ///
    /// A direction with no XZ component (straight up or down) leaves the radial part at zero —
    /// the centre of the flat end. Any point of that face is an equally valid support point;
    /// picking the centre is the one choice that is continuous as the direction tilts.
    fn cylinder_support(cylinder: &CylinderShape, dir: Vec3) -> Vec3 {
        let y = if dir.y >= 0.0 {
            cylinder.half_height
        } else {
            -cylinder.half_height
        };
        let radial = Vec3::new(dir.x, 0.0, dir.z);
        let rim = radial
            .try_normalize()
            .map(|n| n * cylinder.radius)
            .unwrap_or(Vec3::ZERO);
        Vec3::new(rim.x, y, rim.z)
    }

    fn capsule_support(capsule: &CapsuleShape, dir: Vec3) -> Vec3 {
        let dir_normalized = dir.try_normalize().unwrap_or(Vec3::X);
        let sphere_center = if dir_normalized.y > 0.0 {
            Vec3::new(0.0, capsule.half_height, 0.0)
        } else {
            Vec3::new(0.0, -capsule.half_height, 0.0)
        };
        sphere_center + dir_normalized * capsule.radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Quat;

    #[test]
    fn box_support_picks_the_aligned_corner() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(1.0, 2.0, 3.0),
        });
        // +X+Y+Z → the (+1,+2,+3) corner.
        let s = Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        assert!((s - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5, "{s:?}");
        // -X+Y-Z → the (-1,+2,-3) corner.
        let s2 =
            Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, Vec3::new(-1.0, 0.5, -0.2));
        assert!((s2 - Vec3::new(-1.0, 2.0, -3.0)).length() < 1e-5, "{s2:?}");
    }

    #[test]
    fn box_support_respects_translation_and_rotation() {
        let shape = ColliderShape::Box(BoxShape {
            half_extents: Vec3::splat(1.0),
        });
        let pos = Vec3::new(5.0, 0.0, 0.0);
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2); // 90° about Z
        // Farthest point along world +X must sit one half-extent beyond the centre.
        let s = Gjk::support_point(&shape, pos, rot, Vec3::X);
        assert!(
            (s.x - 6.0).abs() < 1e-5,
            "support along +X must reach the far face, got {s:?}"
        );
    }

    #[test]
    fn sphere_support_is_center_plus_radius_and_ignores_rotation() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 2.0 });
        let pos = Vec3::new(1.0, 1.0, 1.0);
        let rot = Quat::from_rotation_x(0.9); // must not matter for a sphere
        let s = Gjk::support_point(&shape, pos, rot, Vec3::Z);
        assert!((s - (pos + Vec3::new(0.0, 0.0, 2.0))).length() < 1e-5, "{s:?}");
    }

    #[test]
    fn cylinder_support_reaches_the_rim_and_the_flat_ends() {
        let shape = ColliderShape::Cylinder(CylinderShape {
            radius: 0.5,
            half_height: 2.0,
        });
        // Straight up → the centre of the top face, not a point 0.5 above it: a cylinder has no
        // cap. (A capsule of the same numbers answers (0, 2.5, 0) here.)
        let up = Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, Vec3::Y);
        assert!((up - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-5, "got {up:?}");

        // Sideways → the rim: full radius out, and on one of the two ends.
        let side = Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, Vec3::X);
        assert!((side.x - 0.5).abs() < 1e-5, "radius out, got {side:?}");
        assert!((side.y.abs() - 2.0).abs() < 1e-5, "and on an end, got {side:?}");

        // Diagonally → the rim on the end the direction points at.
        let diag = Gjk::support_point(
            &shape,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(1.0, -1.0, 0.0).normalize(),
        );
        assert!(
            (diag - Vec3::new(0.5, -2.0, 0.0)).length() < 1e-5,
            "got {diag:?}"
        );
    }

    /// The support point defines the shape the narrowphase actually sees, so this is where a
    /// cylinder that is secretly a capsule (or a box) would show up.
    #[test]
    fn a_cylinder_is_not_its_capsule_or_its_box() {
        let cylinder = ColliderShape::Cylinder(CylinderShape {
            radius: 0.5,
            half_height: 2.0,
        });
        let capsule = ColliderShape::Capsule(CapsuleShape {
            radius: 0.5,
            half_height: 2.0,
        });
        let up = Vec3::Y;
        let cyl_up = Gjk::support_point(&cylinder, Vec3::ZERO, Quat::IDENTITY, up);
        let cap_up = Gjk::support_point(&capsule, Vec3::ZERO, Quat::IDENTITY, up);
        assert!(
            cap_up.y - cyl_up.y > 0.4,
            "the capsule reaches a radius further up: {cap_up:?} vs {cyl_up:?}"
        );

        // And it is round, not square: 45° in the XZ plane stays on the circle.
        let diag = Gjk::support_point(
            &cylinder,
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(1.0, 0.0, 1.0).normalize(),
        );
        let radial = Vec3::new(diag.x, 0.0, diag.z).length();
        assert!((radial - 0.5).abs() < 1e-5, "on the circle, got {radial}");
    }

    #[test]
    fn capsule_support_reaches_the_correct_cap() {
        let shape = ColliderShape::Capsule(CapsuleShape {
            radius: 0.5,
            half_height: 2.0,
        });
        // +Y → top cap centre (0,2,0) + radius = (0, 2.5, 0).
        let up = Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, Vec3::Y);
        assert!((up - Vec3::new(0.0, 2.5, 0.0)).length() < 1e-5, "{up:?}");
        // -Y → bottom cap (0,-2,0) - radius = (0,-2.5,0).
        let down = Gjk::support_point(&shape, Vec3::ZERO, Quat::IDENTITY, -Vec3::Y);
        assert!((down - Vec3::new(0.0, -2.5, 0.0)).length() < 1e-5, "{down:?}");
    }

    #[test]
    fn box_support_maximises_projection_over_all_corners() {
        // The support point must have the largest dot(·, dir) among all 8 corners,
        // for an arbitrary rotated/translated box and direction.
        let h = Vec3::new(1.0, 2.0, 0.5);
        let shape = ColliderShape::Box(BoxShape { half_extents: h });
        let rot = Quat::from_rotation_y(0.6) * Quat::from_rotation_x(0.3);
        let pos = Vec3::new(-2.0, 3.0, 1.0);
        let dir = Vec3::new(0.4, -0.7, 0.5);
        let best = Gjk::support_point(&shape, pos, rot, dir).dot(dir);
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let corner = pos + rot * Vec3::new(sx * h.x, sy * h.y, sz * h.z);
                    assert!(
                        corner.dot(dir) <= best + 1e-5,
                        "support not maximal: corner {corner:?} projects further than support"
                    );
                }
            }
        }
    }
}
