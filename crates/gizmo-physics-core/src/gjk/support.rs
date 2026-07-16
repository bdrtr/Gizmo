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
