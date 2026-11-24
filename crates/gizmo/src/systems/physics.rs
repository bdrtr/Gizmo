// Everything these four lines name is used by the `render`-gated items below — the debug view and
// the two GPU-physics systems. Without that feature the module still exists (it is gated on
// `physics`) and they are unused.
#[cfg(feature = "render")]
use crate::math::Vec3;
#[cfg(feature = "render")]
use crate::renderer::Renderer;
#[cfg(feature = "render")]
use gizmo_physics_core::{Collider, ColliderShape, Transform, components::GpuPhysicsLink};
#[cfg(feature = "render")]
use gizmo_physics_rigid::components::RigidBody;

#[cfg(feature = "render")]
/// Draws the physics debug view into the world's `Gizmos`: every collider's shape, plus the
/// velocity and contact gizmos on top of it. [`physics_debug_lines`] draws the same into lines of
/// the caller's.
pub fn physics_debug_system(world: &crate::core::World) {
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        physics_debug_lines(world, &mut gizmos);
    }
}

#[cfg(feature = "render")]
/// The physics debug view of [`physics_debug_system`], drawn into `gizmos`: an editor's overlay
/// (`gizmo::renderer::EditorLines`), which is drawn into the editor's own view and not the game's.
pub fn physics_debug_lines(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    rigid_body_colliders(world, gizmos);
    static_colliders(world, gizmos);
    fight_boxes(world, gizmos);
    #[cfg(feature = "physics-soft")]
    soft_body_tetrahedra(world, gizmos);
    #[cfg(feature = "physics-dynamics")]
    vehicle_suspension(world, gizmos);
    contact_normals(world, gizmos);
    tracing::trace!(vertices = gizmos.vertex_count(), "physics_debug_system: gizmo lines");
}

#[cfg(feature = "render")]
/// A collider's outline: its real shape, or for a heightfield or a triangle mesh its bounds — a
/// terrain's or a level's every edge would bury the rest of the frame. Every shape has an arm: the
/// enum is exhaustive so that a new shape is flagged here, and the catch-all box it had drew a
/// capsule and a triangle mesh as a 2 m cube at the origin.
fn draw_collider(trans: &Transform, col: &Collider, color: [f32; 4], gizmos: &mut crate::renderer::Gizmos) {
    match &col.shape {
        gizmo_physics_core::ColliderShape::Box(b) => wire_box(gizmos, trans.position, trans.rotation, b.half_extents, color),
        gizmo_physics_core::ColliderShape::Sphere(s) => wire_sphere(gizmos, trans, s.radius, color),
        gizmo_physics_core::ColliderShape::Capsule(c) => wire_capsule(gizmos, trans, c.radius, c.half_height, color),
        gizmo_physics_core::ColliderShape::ConvexHull(ch) => {
            let r = trans.rotation;
            let p = trans.position;
            for face in ch.faces.iter() {
                let p0 = p + r.mul_vec3(ch.vertices[face[0] as usize]);
                let p1 = p + r.mul_vec3(ch.vertices[face[1] as usize]);
                let p2 = p + r.mul_vec3(ch.vertices[face[2] as usize]);
                gizmos.draw_line(p0, p1, color);
                gizmos.draw_line(p1, p2, color);
                gizmos.draw_line(p2, p0, color);
            }
        }
        gizmo_physics_core::ColliderShape::Compound(shapes) => {
            for (local_t, sub_shape) in shapes {
                let world_pos = trans.position + trans.rotation.mul_vec3(local_t.position);
                let world_rot = trans.rotation * local_t.rotation;
                let sub_trans = gizmo_physics_core::Transform {
                    position: world_pos,
                    rotation: world_rot,
                    scale: trans.scale,
                    ..*trans
                };
                let temp_col = gizmo_physics_core::Collider::from_shape(*sub_shape.clone());
                draw_collider(&sub_trans, &temp_col, color, gizmos);
            }
        }
        gizmo_physics_core::ColliderShape::Plane(p) => {
            // Sonsuz zemin düzlemini büyük düz tel-ızgara olarak çiz ki hitbox görünür olsun.
            let n = p.normal.normalize();
            let up = if n.y.abs() < 0.99 {
                Vec3::new(0.0, 1.0, 0.0)
            } else {
                Vec3::new(1.0, 0.0, 0.0)
            };
            let t1 = n.cross(up).normalize();
            let t2 = n.cross(t1).normalize();
            // Izgara merkezini trans.position'ı düzleme izdüşürerek bul (düzlem: dot(n,x)=distance).
            let signed = n.dot(trans.position) - p.distance;
            let center = trans.position - n * signed;
            let half = 40.0_f32;
            let step = 4.0_f32;
            let n_lines = (half * 2.0 / step) as i32;
            for i in 0..=n_lines {
                let o = -half + i as f32 * step;
                gizmos.draw_line(center + t1 * o - t2 * half, center + t1 * o + t2 * half, color);
                gizmos.draw_line(center + t2 * o - t1 * half, center + t2 * o + t1 * half, color);
            }
        }
        gizmo_physics_core::ColliderShape::Cylinder(c) => {
            // Two rims and the lines between them. Drawn properly rather than left to
            // the fallback box below, because the shape exists for wheels and barrels
            // — the cases where seeing the real silhouette is the point.
            const SEGMENTS: usize = 16;
            let axis = trans.rotation.mul_vec3(Vec3::Y) * c.half_height;
            let u = trans.rotation.mul_vec3(Vec3::X) * c.radius;
            let w = trans.rotation.mul_vec3(Vec3::Z) * c.radius;
            let rim = |i: usize, end: f32| {
                let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                trans.position + axis * end + u * a.cos() + w * a.sin()
            };
            for i in 0..SEGMENTS {
                for end in [1.0, -1.0] {
                    gizmos.draw_line(rim(i, end), rim(i + 1, end), color);
                }
                gizmos.draw_line(rim(i, 1.0), rim(i, -1.0), color);
            }
        }
        gizmo_physics_core::ColliderShape::Cone(c) => {
            // One rim and the lines up to the apex — the silhouette that tells a cone
            // from the cylinder next to it. The fallback box below would draw the same
            // outline for both, which is exactly the confusion this shape was added to
            // remove.
            const SEGMENTS: usize = 16;
            let axis = trans.rotation.mul_vec3(Vec3::Y) * c.half_height;
            let u = trans.rotation.mul_vec3(Vec3::X) * c.radius;
            let w = trans.rotation.mul_vec3(Vec3::Z) * c.radius;
            let apex = trans.position + axis;
            let rim = |i: usize| {
                let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                trans.position - axis + u * a.cos() + w * a.sin()
            };
            for i in 0..SEGMENTS {
                gizmos.draw_line(rim(i), rim(i + 1), color);
                gizmos.draw_line(rim(i), apex, color);
            }
        }
        gizmo_physics_core::ColliderShape::Heightfield(hf) => {
            // The field's bounds, not its surface. A terrain's cell edges are tens of
            // thousands of lines and would bury the rest of the frame; the box is what a
            // debug view of a static body is actually for — is it where I put it, and is
            // it as big as I think. The surface itself is on screen already.
            wire_local_bounds(gizmos, trans, hf.local_aabb.min.into(), hf.local_aabb.max.into(), color);
        }
        gizmo_physics_core::ColliderShape::TriMesh(mesh) => {
            // Its bounds, as for a heightfield: a level's every triangle would bury the frame.
            let lo = mesh.vertices.iter().fold(Vec3::splat(f32::INFINITY), |a, &v| a.min(v));
            let hi = mesh.vertices.iter().fold(Vec3::splat(f32::NEG_INFINITY), |a, &v| a.max(v));
            if lo.x <= hi.x {
                wire_local_bounds(gizmos, trans, lo, hi, color);
            }
        }
    }
}

#[cfg(feature = "render")]
/// The twelve edges of a box of half-extents `h` centred at `p`, turned by `r`.
fn wire_box(gizmos: &mut crate::renderer::Gizmos, p: Vec3, r: gizmo_math::Quat, h: Vec3, color: [f32; 4]) {
    let p0 = p + r.mul_vec3(Vec3::new(-h.x, -h.y, -h.z));
    let p1 = p + r.mul_vec3(Vec3::new(h.x, -h.y, -h.z));
    let p2 = p + r.mul_vec3(Vec3::new(h.x, h.y, -h.z));
    let p3 = p + r.mul_vec3(Vec3::new(-h.x, h.y, -h.z));
    let p4 = p + r.mul_vec3(Vec3::new(-h.x, -h.y, h.z));
    let p5 = p + r.mul_vec3(Vec3::new(h.x, -h.y, h.z));
    let p6 = p + r.mul_vec3(Vec3::new(h.x, h.y, h.z));
    let p7 = p + r.mul_vec3(Vec3::new(-h.x, h.y, h.z));

    gizmos.draw_line(p0, p1, color);
    gizmos.draw_line(p1, p2, color);
    gizmos.draw_line(p2, p3, color);
    gizmos.draw_line(p3, p0, color);
    gizmos.draw_line(p4, p5, color);
    gizmos.draw_line(p5, p6, color);
    gizmos.draw_line(p6, p7, color);
    gizmos.draw_line(p7, p4, color);
    gizmos.draw_line(p0, p4, color);
    gizmos.draw_line(p1, p5, color);
    gizmos.draw_line(p2, p6, color);
    gizmos.draw_line(p3, p7, color);
}

#[cfg(feature = "render")]
/// A box in the body's own frame, from `lo` to `hi`, taken to the world by `trans`.
fn wire_local_bounds(gizmos: &mut crate::renderer::Gizmos, trans: &Transform, lo: Vec3, hi: Vec3, color: [f32; 4]) {
    let mut corners = [Vec3::ZERO; 8];
    for (i, c) in corners.iter_mut().enumerate() {
        let local = Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        );
        *c = trans.position + trans.rotation.mul_vec3(local);
    }
    for (a, b) in [
        (0, 1), (2, 3), (4, 5), (6, 7),
        (0, 2), (1, 3), (4, 6), (5, 7),
        (0, 4), (1, 5), (2, 6), (3, 7),
    ] {
        gizmos.draw_line(corners[a], corners[b], color);
    }
}

#[cfg(feature = "render")]
/// A circle about `center` in the plane of `u` and `w`, whose lengths are its radii.
fn wire_circle(gizmos: &mut crate::renderer::Gizmos, center: Vec3, u: Vec3, w: Vec3, color: [f32; 4]) {
    const SEGMENTS: usize = 24;
    let at = |i: usize| {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        center + u * a.cos() + w * a.sin()
    };
    for i in 0..SEGMENTS {
        gizmos.draw_line(at(i), at(i + 1), color);
    }
}

#[cfg(feature = "render")]
/// A sphere as three great circles, turning with the body so a rolling ball is seen to roll.
fn wire_sphere(gizmos: &mut crate::renderer::Gizmos, trans: &Transform, radius: f32, color: [f32; 4]) {
    let [x, y, z] = [Vec3::X, Vec3::Y, Vec3::Z].map(|axis| trans.rotation.mul_vec3(axis) * radius);
    wire_circle(gizmos, trans.position, x, y, color);
    wire_circle(gizmos, trans.position, y, z, color);
    wire_circle(gizmos, trans.position, z, x, color);
}

#[cfg(feature = "render")]
/// A capsule about its local +Y axis: the two rims where the caps meet the side, four side lines,
/// and each cap as two half circles over its pole.
fn wire_capsule(gizmos: &mut crate::renderer::Gizmos, trans: &Transform, radius: f32, half_height: f32, color: [f32; 4]) {
    const HALF: usize = 12;
    let up = trans.rotation.mul_vec3(Vec3::Y);
    let u = trans.rotation.mul_vec3(Vec3::X) * radius;
    let w = trans.rotation.mul_vec3(Vec3::Z) * radius;
    let (top, bottom) = (trans.position + up * half_height, trans.position - up * half_height);
    wire_circle(gizmos, top, u, w, color);
    wire_circle(gizmos, bottom, u, w, color);
    for side in [u, -u, w, -w] {
        gizmos.draw_line(bottom + side, top + side, color);
    }
    for (pole, out) in [(top, up), (bottom, -up)] {
        for side in [u, w] {
            let at = |i: usize| {
                let a = i as f32 / HALF as f32 * std::f32::consts::PI;
                pole + side * a.cos() + out * (radius * a.sin())
            };
            for i in 0..HALF {
                gizmos.draw_line(at(i), at(i + 1), color);
            }
        }
    }
}

#[cfg(feature = "render")]
/// Every rigid body's collider, coloured by state: grey static, red asleep, green awake.
fn rigid_body_colliders(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    let q = world.query::<(
    &gizmo_physics_core::Transform,
    &gizmo_physics_core::Collider,
    &gizmo_physics_rigid::components::RigidBody,
)>();
    for (trans, col, rb) in q.iter() {
        let color = if rb.is_static() {
            [0.5, 0.5, 0.5, 1.0]
        } else if rb.is_sleeping {
            [0.9, 0.1, 0.1, 1.0]
        } else {
            [0.1, 0.9, 0.1, 1.0]
        };
        draw_collider(trans, col, color, gizmos);
    }
}

#[cfg(feature = "render")]
/// The colliders without a rigid body, in grey.
fn static_colliders(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    let q = world.query_filtered::<(&gizmo_physics_core::Transform, &gizmo_physics_core::Collider), crate::core::query::Without<gizmo_physics_rigid::components::RigidBody>>();
    for (trans, col) in q.iter() {
        draw_collider(trans, col, [0.5, 0.5, 0.5, 1.0], gizmos);
    }
}

#[cfg(feature = "render")]
/// The fighters' active hitboxes in red and their hurtboxes in green.
fn fight_boxes(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    {
        let q = world.query::<(&gizmo_physics_core::Transform, &gizmo_physics_core::components::Hitbox)>();
        for (trans, hitbox) in q.iter() {
            if hitbox.active {
                wire_box(gizmos, trans.position + trans.rotation.mul_vec3(hitbox.offset), trans.rotation, hitbox.half_extents, [1.0, 0.0, 0.0, 1.0]); // RED
            }
        }
    }
    let q = world.query::<(&gizmo_physics_core::Transform, &gizmo_physics_core::components::Hurtbox)>();
    for (trans, hurtbox) in q.iter() {
        wire_box(gizmos, trans.position + trans.rotation.mul_vec3(hurtbox.offset), trans.rotation, hurtbox.half_extents, [0.0, 1.0, 0.0, 1.0]); // GREEN
    }
}

#[cfg(all(feature = "render", feature = "physics-soft"))]
/// Every soft body's tetrahedra, in pink.
fn soft_body_tetrahedra(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    let soft_color = [1.0, 0.4, 0.8, 1.0]; // Pinkish for soft body
    let q = world.query::<&gizmo_physics_soft::SoftBodyMesh>();
    for sm in q.iter() {
        for elem in &sm.elements {
            let p0 = sm.nodes[elem.node_indices[0] as usize].position;
            let p1 = sm.nodes[elem.node_indices[1] as usize].position;
            let p2 = sm.nodes[elem.node_indices[2] as usize].position;
            let p3 = sm.nodes[elem.node_indices[3] as usize].position;

            // 6 edges of a tetrahedron
            gizmos.draw_line(p0, p1, soft_color);
            gizmos.draw_line(p0, p2, soft_color);
            gizmos.draw_line(p0, p3, soft_color);
            gizmos.draw_line(p1, p2, soft_color);
            gizmos.draw_line(p1, p3, soft_color);
            gizmos.draw_line(p2, p3, soft_color);
        }
    }
}

#[cfg(all(feature = "render", feature = "physics-dynamics"))]
/// Each vehicle wheel's suspension ray (yellow), and on the ground its force (blue) and length (orange).
fn vehicle_suspension(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    let q = world.query::<(
    &gizmo_physics_core::Transform,
    &gizmo_physics_dynamics::VehicleController,
)>();
    for (trans, vehicle) in q.iter() {
        for wheel in &vehicle.wheels {
            let attach_world =
                trans.position + trans.rotation.mul_vec3(wheel.attachment_local_pos);
            let ray_dir = trans.rotation.mul_vec3(wheel.direction_local).normalize();
            let ray_end = attach_world
                + ray_dir
                    * (wheel.suspension_rest_length
                        + wheel.suspension_max_travel
                        + wheel.radius);

            // Draw raycast maximum extent (Yellow line)
            gizmos.draw_line(attach_world, ray_end, [1.0, 1.0, 0.0, 1.0]);

            if wheel.is_grounded {
                if let Some(hit) = &wheel.ground_hit {
                    // Kuvvet oku (Mavi) - sadece uzunluğu normalize edip görselleştirmek için / 10000 kullanıyoruz
                    let force_dir = -ray_dir;
                    let force_len = (wheel.suspension_force / 10000.0).clamp(0.1, 2.0);
                    let arrow_end = hit.point + force_dir * force_len;
                    gizmos.draw_line(hit.point, arrow_end, [0.0, 0.0, 1.0, 1.0]);

                    // Mevcut süspansiyon uzunluğu + tekerlek merkezi çizgisi (Turuncu)
                    let wheel_center = attach_world + ray_dir * wheel.suspension_length;
                    gizmos.draw_line(wheel_center, hit.point, [1.0, 0.5, 0.0, 1.0]);
                }
            }
        }
    }
}

#[cfg(feature = "render")]
/// This step's contact normals (red) and penetration depths (magenta).
fn contact_normals(world: &crate::core::World, gizmos: &mut crate::renderer::Gizmos) {
    if let Some(phys_world) = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
        for event in phys_world.collision_events() {
            for contact in &event.contact_points {
                let p1 = contact.point;
                let p2 = contact.point + contact.normal * 0.5; // Normal arrow
                gizmos.draw_line(p1, p2, [1.0, 0.0, 0.0, 1.0]); // Red normal

                let p_pen = contact.point - contact.normal * contact.penetration;
                gizmos.draw_line(p1, p_pen, [1.0, 0.0, 1.0, 1.0]); // Magenta penetration depth
            }
        }
    }
}

/// The static-collider slot a plane-collider entity holds in the GPU bodies' table. Internal
/// bookkeeping of [`gpu_physics_submit_system`]: an entity carries it once uploaded, as a body
/// carries its [`GpuPhysicsLink`], and the slot is freed once the entity no longer carries it.
#[cfg(feature = "render")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GpuStaticCollider(pub(crate) u32);
#[cfg(feature = "render")]
gizmo_core::impl_component!(GpuStaticCollider);

#[cfg(feature = "render")]
/// Keeps the renderer's GPU bodies the entities with a rigid body: each uploaded once — a plane
/// collider as a static collider, anything else as a box it is linked to ([`GpuPhysicsLink`]) — and
/// its slot freed once the entity is gone or no longer carries the link.
///
/// **A slot is the table's to give** ([`add_box`](gizmo_renderer::gpu_physics::GpuPhysicsSystem::add_box),
/// [`add_collider`](gizmo_renderer::gpu_physics::GpuPhysicsSystem::add_collider)). A body's used to be
/// the count of linked entities, so a body despawned and another spawned shared a slot with one
/// still alive, and nothing ever took a body out. A plane took the next number of a counter shared
/// by the whole process, and was never marked as uploaded — planes take no link — so it was
/// uploaded again into the next slot every frame, filling the table with copies of itself.
///
/// A body the table has no room for is not linked, and asked about again next frame: room is made
/// as other bodies go.
pub fn gpu_physics_submit_system(world: &mut crate::core::World, renderer: &mut Renderer) {
    use gizmo_physics_rigid::components::Velocity;

    if let Some(physics) = renderer.gpu_physics.as_mut() {
        // First the way out: a slot whose entity is gone, or no longer claims it, is freed — the body
        // in it stops being simulated and drawn, and the slot goes to the next body in.
        physics.retain_owned_boxes(&renderer.queue, |entity, slot| {
            world.get::<GpuPhysicsLink>(entity).is_some_and(|link| link.id == slot)
        });
        physics.retain_owned_colliders(&renderer.queue, |entity, slot| {
            world.get::<GpuStaticCollider>(entity) == Some(&GpuStaticCollider(slot))
        });
        // And the other way: a link the table does not back — copied onto another entity, loaded
        // with a scene, its slot freed through `remove_box` — is dropped, and the entity uploaded
        // again below. Kept, it made the entity mirror whichever body the slot held next.
        let unbacked: Vec<(gizmo_core::Entity, bool)> = {
            let links = world.query::<(gizmo_core::Entity, &GpuPhysicsLink)>();
            let statics = world.query::<(gizmo_core::Entity, &GpuStaticCollider)>();
            links
                .iter()
                .filter(|(e, link)| !physics.holds_box(link.id, *e))
                .map(|(e, _)| (e, true))
                .chain(statics.iter().filter(|(e, c)| !physics.holds_collider(c.0, *e)).map(|(e, _)| (e, false)))
                .collect()
        };
        for (entity, is_box) in unbacked {
            if is_box {
                world.remove_component::<GpuPhysicsLink>(entity);
            } else {
                world.remove_component::<GpuStaticCollider>(entity);
            }
        }

        let mut unlinked_entities = Vec::new();
        {
            let q = world.query::<(gizmo_core::Entity, &RigidBody, &Transform, &Collider)>();
            let links = world.query::<&GpuPhysicsLink>();
            let statics = world.query::<&GpuStaticCollider>();
            let velocities = world.query::<&Velocity>();
            for (e, rb, trans, col) in q.iter() {
                if links.get(e).is_none() && statics.get(e).is_none() {
                    let vel = velocities.get(e).copied().unwrap_or_default();
                    unlinked_entities.push((e, *rb, *trans, col.clone(), vel));
                }
            }
        }

        for (e, rb, trans, col, vel) in unlinked_entities {
            if matches!(col.shape, ColliderShape::Plane(_)) {
                let gpu_col = gizmo_renderer::gpu_physics::GpuCollider {
                    shape_type: match col.shape {
                        ColliderShape::Plane(_) => 1,
                        _ => 0, // Varsayılan Box (AABB)
                    },
                    _pad1: [0; 3],
                    data1: match &col.shape {
                        ColliderShape::Plane(p) => [p.normal.x, p.normal.y, p.normal.z, 0.0],
                        ColliderShape::Box(b) => {
                            let min = trans.position - b.half_extents;
                            [min.x, min.y, min.z, 0.0]
                        }
                        _ => [0.0; 4],
                    },
                    data2: match &col.shape {
                        ColliderShape::Plane(p) => [p.distance, 0.0, 0.0, 0.0],
                        ColliderShape::Box(b) => {
                            let max = trans.position + b.half_extents;
                            [max.x, max.y, max.z, 0.0]
                        }
                        _ => [0.0; 4],
                    },
                };
                // A full table is logged by the table, once; the plane is asked about again later.
                if let Some(slot) = physics.add_collider(&renderer.queue, &gpu_col, Some(e)) {
                    world.add_component(e, GpuStaticCollider(slot));
                }
            } else {
                // Dinamik Kutu (AABB)

                let (extents, offset) = match &col.shape {
                    ColliderShape::Box(b) => ([b.half_extents.x, b.half_extents.y, b.half_extents.z], Vec3::ZERO),
                    ColliderShape::Compound(shapes) => {
                        if let Some((local_t, box_shape)) = shapes.first() {
                            if let ColliderShape::Box(b) = &**box_shape {
                                ([b.half_extents.x, b.half_extents.y, b.half_extents.z], local_t.position)
                            } else {
                                ([0.5, 0.5, 0.5], Vec3::ZERO)
                            }
                        } else {
                            ([0.5, 0.5, 0.5], Vec3::ZERO)
                        }
                    }
                    _ => ([0.5, 0.5, 0.5], Vec3::ZERO),
                };

                let world_pos = trans.position + trans.rotation.mul_vec3(offset);

                let gpu_box = gizmo_renderer::gpu_physics::GpuBox {
                    position: [world_pos.x, world_pos.y, world_pos.z],
                    mass: rb.mass,
                    velocity: [vel.linear.x, vel.linear.y, vel.linear.z],
                    state: 0,
                    rotation: [
                        trans.rotation.x,
                        trans.rotation.y,
                        trans.rotation.z,
                        trans.rotation.w,
                    ],
                    angular_velocity: [vel.angular.x, vel.angular.y, vel.angular.z],
                    sleep_counter: if rb.is_sleeping { 60 } else { 0 },
                    color: [0.3, 0.8, 1.0, 1.0],
                    half_extents: extents,
                    _pad: 0,
                };
                if let Some(id) = physics.add_box(&renderer.queue, &gpu_box, Some(e)) {
                    world.add_component(e, GpuPhysicsLink { id });
                }
            }
        }
    }
}

/// Copies the enormous Physics location states pulled Asynchronously (0 ms) from the GPU
/// into their Transforms in the ECS, so that the objects on screen can be rendered.
#[cfg(feature = "render")]
pub fn gpu_physics_readback_system(world: &mut crate::core::World, renderer: &Renderer) {
    if let Some(physics) = &renderer.gpu_physics {
        if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
            {
                let mut q = world.query_mut::<(&mut Transform, &GpuPhysicsLink, &gizmo_physics_core::Collider)>();
                for (mut trans, link, col) in q.iter_mut() {
                    let idx = link.id as usize;
                    // Copied before this body was put in: the slot there holds nothing yet, or the
                    // body that left it. The next readback has it.
                    if !physics.readback_holds(link.id) {
                        continue;
                    }
                    if idx < gpu_data.len() {
                        let box_data = &gpu_data[idx];
                        
                        let offset = match &col.shape {
                            gizmo_physics_core::ColliderShape::Compound(shapes) => {
                                if let Some((local_t, _)) = shapes.first() {
                                    local_t.position
                                } else {
                                    gizmo_math::Vec3::ZERO
                                }
                            }
                            _ => gizmo_math::Vec3::ZERO,
                        };

                        let new_rot = gizmo_math::Quat::from_xyzw(
                            box_data.rotation[0],
                            box_data.rotation[1],
                            box_data.rotation[2],
                            box_data.rotation[3],
                        );

                        trans.position = gizmo_math::Vec3::new(
                            box_data.position[0],
                            box_data.position[1],
                            box_data.position[2],
                        ) - new_rot.mul_vec3(offset);
                        
                        trans.rotation = new_rot;
                        trans.update_local_matrix();
                    }
                }
            }
        }
    }
}

// Phase 7.1: Fluid-Rigid Coupling
// Senkronize eder: GpuPhysicsLink sahibi objeleri FluidCollider buffer'ına yazar.
/// Steps the CPU physics world by `dt` and writes the results back into the ECS.
///
/// The step includes **explosions** and **fracture**: an active
/// [`Explosion`](gizmo_physics_rigid::components::Explosion) pushes and damages what is in its
/// reach, once; a [`Breakable`](gizmo_physics_rigid::components::Breakable) hit hard enough
/// shatters, its pieces age and are removed after its `debris_lifetime`, its `break_sound` plays
/// at it (with the `audio` and `render` features), and — with `render` — each piece is drawn as
/// its own Voronoi cell in the broken body's material. Until 2026-11-03 nothing ran the fracture
/// system, and until 2026-11-22 nothing ran the explosion system: an `Explosion` did nothing unless
/// the game called `physics_explosion_system` itself.
pub fn cpu_physics_step_system(world: &crate::core::World, dt: f32) {
    // Before the step, so the blast's push is integrated by it.
    let mut breaks = gizmo_physics_rigid::system::explosion_step(world, dt);
    gizmo_physics_rigid::system::physics_step_system(world, dt);
    breaks.extend(gizmo_physics_rigid::system::fracture_step(world, dt));
    #[cfg(feature = "render")]
    queue_debris_visuals(world);
    #[cfg(all(feature = "audio", feature = "render"))]
    play_break_sounds(world, &breaks);
    #[cfg(not(all(feature = "audio", feature = "render")))]
    let _ = breaks;

    #[cfg(feature = "physics-soft")]
    {
        // Obtain gravity from the PhysicsWorld if it exists
        let gravity = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>()
            .map(|w| w.integrator.gravity)
            .unwrap_or(gizmo_math::Vec3::new(0.0, -9.81, 0.0));
            
        gizmo_physics_soft::system::soft_body_step_system(world, dt, gravity);
        gizmo_physics_soft::system::cloth_step_system(world, dt, gravity);
        gizmo_physics_soft::system::rope_step_system(world, dt, gravity);
    }
}

/// Plays each break's `break_sound` at the body that broke, heard from the frame's camera — what
/// `Breakable::break_sound` asks for, which until 2026-11-22 nothing read.
#[cfg(all(feature = "audio", feature = "render"))]
fn play_break_sounds(world: &crate::core::World, breaks: &[gizmo_physics_rigid::components::BreakEvent]) {
    if breaks.iter().all(|b| b.sound.is_none()) {
        return;
    }
    let Some(mut audio) = world.get_resource_mut::<gizmo_audio::AudioManager>() else { return };
    let (left, right) = crate::systems::audio::listener(world).ears();
    for b in breaks {
        if let Some(name) = &b.sound {
            if let Err(e) = audio.play_3d(name, b.position.to_array(), left, right) {
                tracing::warn!(sound = %name, error = %e, "[Physics] a break's sound could not be played");
            }
        }
    }
}

/// Queues, behind the fracture step's own commands, the job of giving each new debris piece a
/// mesh and the material of the body it broke off.
///
/// The broken body is despawned by those commands, before its pieces are spawned, so its
/// material is read **now**, while it is still there: a `Breakable` that has just latched
/// `is_broken` and still exists is one whose despawn is pending.
#[cfg(feature = "render")]
fn queue_debris_visuals(world: &crate::core::World) {
    use crate::renderer::components::MaterialDesc;
    use gizmo_physics_rigid::components::Breakable;
    let breakables = world.query::<(gizmo_core::Entity, &Breakable)>();
    let descs = world.query::<&MaterialDesc>();
    let looks: Vec<(gizmo_core::Entity, Option<MaterialDesc>)> = breakables
        .iter()
        .filter(|(_, b)| b.is_broken)
        .map(|(entity, _)| (entity, descs.get(entity).cloned()))
        .collect();
    if looks.is_empty() {
        return;
    }
    drop(descs);
    drop(breakables);
    let Some(queue) = world.get_resource::<gizmo_core::commands::CommandQueue>() else { return };
    queue.push(move |world: &mut crate::core::World| attach_debris_visuals(world, &looks));
}

/// Gives every [`Debris`](gizmo_physics_rigid::components::Debris) piece that has no
/// `MeshRenderer` yet its cell as a [`MeshData`](crate::renderer::components::MeshData), the
/// material of the body it came from (`looks`, by source id; a plain grey without one), a
/// renderer and a `GlobalTransform`.
#[cfg(feature = "render")]
pub(crate) fn attach_debris_visuals(world: &mut crate::core::World, looks: &[(gizmo_core::Entity, Option<crate::renderer::components::MaterialDesc>)]) {
    use crate::renderer::components::{MaterialDesc, MeshData, MeshRenderer};
    use gizmo_physics_rigid::components::Debris;
    let pending: Vec<(gizmo_core::Entity, MeshData, u32)> = {
        let pieces = world.query::<(gizmo_core::Entity, &Debris)>();
        let drawn = world.query::<&MeshRenderer>();
        pieces
            .iter()
            // A part that broke loose (no cell of its own) is drawn by its own mesh, or not at all.
            .filter(|(entity, d)| drawn.get(*entity).is_none() && !d.indices.is_empty())
            .map(|(entity, d)| (entity, debris_mesh(d), d.source))
            .collect()
    };
    for (id, mesh, source) in pending {
        let Some(e) = world.live(id) else { continue };
        let material = looks
            .iter()
            .find(|(s, _)| s.id() == source)
            .and_then(|(_, m)| m.clone())
            .unwrap_or_else(|| MaterialDesc::new().with_pbr(gizmo_math::Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.0));
        world.add_component(e, mesh);
        world.add_component(e, material);
        world.add_component(e, MeshRenderer::new());
        world.add_component(e, gizmo_physics_core::components::GlobalTransform::default());
    }
}

/// A debris cell as a flat-shaded triangle list, each triangle wound counter-clockwise about
/// its face's stored outward normal (the cell's own winding is not reliable).
#[cfg(feature = "render")]
fn debris_mesh(d: &gizmo_physics_rigid::components::Debris) -> crate::renderer::components::MeshData {
    use crate::renderer::Vertex;
    let mut out = Vec::with_capacity(d.indices.len());
    for &[a, b, c] in d.indices.as_chunks::<3>().0 {
        let [a, b, c] = [a as usize, b as usize, c as usize];
        let (Some(&pa), Some(&pb), Some(&pc)) = (d.vertices.get(a), d.vertices.get(b), d.vertices.get(c)) else {
            continue;
        };
        let n = d.normals.get(a).copied().unwrap_or(gizmo_math::Vec3::Y);
        let order = if (pb - pa).cross(pc - pa).dot(n) >= 0.0 { [pa, pb, pc] } else { [pa, pc, pb] };
        for p in order {
            out.push(Vertex { position: p.to_array(), normal: n.to_array(), ..Default::default() });
        }
    }
    crate::renderer::components::MeshData::new(out)
}

/// How much of a substep a fixed tick may be off a whole number of them and still count as one:
/// f32 rounding (`1/60` is `4 × 1/240` to a few parts in ten million), nothing a game chose. The
/// world's own step snaps by the same fraction.
const TICK_SNAP: f64 = 1e-4;

/// A fixed tick that is not a whole number of a physics world's substeps — what
/// [`substeps_per_tick`] reports, and what every driver of a [`PhysicsWorld`] here logs.
///
/// [`PhysicsWorld`]: gizmo_physics_rigid::world::PhysicsWorld
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum TickMismatch {
    /// The tick is shorter than one substep: some ticks run no physics at all and the next runs
    /// what they owed.
    ShorterThanSubstep {
        /// The tick's rate, in hertz.
        tick_hz: f32,
        /// The world's substep rate, in hertz.
        substep_rate: u32,
    },
    /// The tick is longer than a substep but not a whole number of them: ticks run uneven numbers
    /// of substeps (at 100 Hz under a 60 Hz tick: 1, 2, 2, 1, 2, 2, …).
    NotWholeSubsteps {
        /// The tick's rate, in hertz.
        tick_hz: f32,
        /// The world's substep rate, in hertz.
        substep_rate: u32,
    },
}

impl TickMismatch {
    /// What would fit: the world rates that are whole multiples of the tick, nearest the world's
    /// own, and the tick rates that divide the world's.
    fn remedy(tick_hz: f32, rate: u32) -> String {
        let mut out = String::new();
        let rates = gizmo_physics_rigid::SUBSTEP_RATES;
        let tick = tick_hz.round() as u32;
        if tick > 0 && (tick_hz - tick as f32).abs() < 1e-3 {
            let below = (rate / tick) * tick;
            let fits: Vec<String> = [below, below + tick]
                .into_iter()
                .filter(|r| *r > 0 && rates.contains(r))
                .map(|r| format!("{r} Hz"))
                .collect();
            if !fits.is_empty() {
                out += &format!("build the world at a whole multiple of the tick ({}, with `with_substep_rate`)", fits.join(" or "));
            }
        }
        let divisors: Vec<u32> = (1..=rate).filter(|k| rate.is_multiple_of(*k)).map(|k| rate / k).collect();
        let under = divisors.iter().copied().filter(|d| (*d as f32) <= tick_hz).max();
        let over = divisors.iter().copied().filter(|d| (*d as f32) > tick_hz).min();
        let ticks: Vec<String> = [under, over].into_iter().flatten().map(|d| format!("{d} Hz")).collect();
        if !ticks.is_empty() {
            if !out.is_empty() {
                out += ", or ";
            }
            out += &format!("tick at a rate that divides {rate} Hz ({})", ticks.join(" or "));
        }
        out
    }
}

impl std::fmt::Display for TickMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::ShorterThanSubstep { tick_hz, substep_rate } => write!(
                f,
                "a fixed tick of {tick_hz:.2} Hz is shorter than one substep of the physics world's \
                 {substep_rate} Hz, so some ticks simulate nothing: {}",
                Self::remedy(tick_hz, substep_rate)
            ),
            Self::NotWholeSubsteps { tick_hz, substep_rate } => write!(
                f,
                "a fixed tick of {tick_hz:.2} Hz is not a whole number of the physics world's \
                 {substep_rate} Hz substeps, so ticks run uneven numbers of them: {}",
                Self::remedy(tick_hz, substep_rate)
            ),
        }
    }
}

impl std::error::Error for TickMismatch {}

/// How many of `world`'s substeps a fixed tick of `tick_dt` seconds is: a whole number, one or
/// more, or the [`TickMismatch`] that says why not.
///
/// Every driver of a world — the app's fixed schedule ([`PhysicsStepSystem`]), the play loop and
/// [`FixedPhysics`](crate::simple::FixedPhysics) — hands [`PhysicsWorld::step`] one tick at a time,
/// and the world spends it in its own substeps. A tick that is a whole number `k` of them runs
/// exactly `k` every time. Any other tick runs some ticks with none or with an uneven count: a
/// kinematic target cut per tick teleports, a force written once a tick acts for a different time,
/// and the poses drawn between ticks judder.
///
/// [`PhysicsWorld::step`]: gizmo_physics_rigid::world::PhysicsWorld::step
pub fn substeps_per_tick(
    world: &gizmo_physics_rigid::world::PhysicsWorld,
    tick_dt: f32,
) -> Result<u32, TickMismatch> {
    let substep_rate = world.substep_rate();
    let ratio = f64::from(tick_dt) * f64::from(substep_rate);
    let tick_hz = 1.0 / tick_dt;
    // A NaN tick (a zero or non-finite one) lands here too.
    if ratio.is_nan() || ratio < 1.0 - TICK_SNAP {
        return Err(TickMismatch::ShorterThanSubstep { tick_hz, substep_rate });
    }
    let k = ratio.round();
    if !k.is_finite() || (ratio - k).abs() > TICK_SNAP {
        return Err(TickMismatch::NotWholeSubsteps { tick_hz, substep_rate });
    }
    Ok(k as u32)
}

/// A driver's memory of the tick and world rate it last checked, so a mismatch is logged once per
/// pairing rather than once a tick — and again if the game changes either.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct TickCheck {
    checked: Option<(u32, u32)>,
}

impl TickCheck {
    /// Checks `tick_dt` against `world` ([`substeps_per_tick`]) the first time this pairing is
    /// seen, and logs an error naming `driver` when it does not fit. Returns the mismatch it
    /// logged, if it logged one.
    ///
    /// **Reported, not refused.** Refusing to step would freeze the game on a configuration error,
    /// which is a worse failure than uneven substeps: the world still receives every second of
    /// simulated time and stays deterministic, only unevenly cut. The error is loud where it counts
    /// — every `--test` run goes through `run_self_test`, which fails a run that logged one, and the
    /// editor's console shows it in red.
    pub(crate) fn check(
        &mut self,
        world: &gizmo_physics_rigid::world::PhysicsWorld,
        tick_dt: f32,
        driver: &str,
    ) -> Option<TickMismatch> {
        let key = (tick_dt.to_bits(), world.substep_rate());
        if self.checked == Some(key) {
            return None;
        }
        self.checked = Some(key);
        let mismatch = substeps_per_tick(world, tick_dt).err()?;
        tracing::error!(driver, "[Physics] {driver}: {mismatch}");
        Some(mismatch)
    }
}

/// Schedulable wrapper around [`cpu_physics_step_system`].
///
/// [`crate::plugins::PhysicsPlugin`] adds this to the app schedule so the physics
/// step runs automatically at the app's **fixed timestep** (the accumulator loop
/// driven by `PhysicsTime`) — callers no longer hand-call `cpu_physics_step_system`
/// every frame, and physics gets a stable fixed step for free.
///
/// The fixed tick has to be a whole number of the world's substeps (60 Hz is 4 of the default
/// 240): where it first runs on a world it checks, and logs an error when the two do not fit
/// ([`substeps_per_tick`]).
#[derive(Debug, Default)]
pub struct PhysicsStepSystem {
    tick: TickCheck,
    /// The rate the plugin built its world at, when the game asked for one.
    asked_rate: Option<u32>,
    /// Whether a world at another rate than `asked_rate` has been reported.
    rate_reported: bool,
}

impl PhysicsStepSystem {
    /// A step system for a world built anywhere.
    pub fn new() -> Self {
        Self::default()
    }

    /// The one [`PhysicsPlugin`](crate::plugins::PhysicsPlugin) registers: it also says, once, when
    /// the world it steps is not at the rate the plugin was asked for — a world the game's setup
    /// inserted after the plugin built its own replaces that one, rate and all.
    pub(crate) fn for_rate(asked_rate: Option<u32>) -> Self {
        Self { asked_rate, ..Self::default() }
    }
}

impl gizmo_core::system::System for PhysicsStepSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // Exclusive barrier — mirrors `physics_step_system`'s whole-World access.
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &crate::core::World, dt: f32) {
        if let Some(physics) = world.get_resource::<gizmo_physics_rigid::world::PhysicsWorld>() {
            self.tick.check(&physics, dt, "the app's fixed tick (PhysicsTime)");
            if let Some(asked) = self.asked_rate.filter(|asked| *asked != physics.substep_rate()) {
                if !self.rate_reported {
                    self.rate_reported = true;
                    tracing::error!(
                        "[Physics] PhysicsPlugin was asked for {asked} Hz substeps, but the PhysicsWorld it \
                         steps runs at {} Hz: a world inserted after the plugin (a setup's own) replaced \
                         the plugin's. Build that world with `with_substep_rate({asked})`, or leave the \
                         rate to it and ask the plugin for none.",
                        physics.substep_rate()
                    );
                }
            }
        }
        cpu_physics_step_system(world, dt);
    }
}

/// The messages of the errors logged on this thread while `f` runs — what a test asks of a report
/// that is a log line and nothing else.
#[cfg(test)]
pub(crate) fn errors_during(f: impl FnOnce()) -> Vec<String> {
    use std::sync::{Arc, Mutex};
    struct Errors(Arc<Mutex<Vec<String>>>);
    struct Message(String);
    impl tracing::field::Visit for Message {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }
    impl tracing::Subscriber for Errors {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::ERROR
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0.lock().expect("not poisoned").push(message.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }
    let errors = Arc::new(Mutex::new(Vec::new()));
    tracing::subscriber::with_default(Errors(errors.clone()), f);
    let logged = errors.lock().expect("not poisoned").clone();
    logged
}

#[cfg(test)]
mod tick_tests {
    use super::*;
    use gizmo_physics_rigid::world::PhysicsWorld;

    fn world(hz: u32) -> PhysicsWorld {
        PhysicsWorld::new().with_substep_rate(hz).expect("a rate a world accepts")
    }

    /// A 60 Hz tick is 1, 2 and 4 substeps of a 60, 120 and 240 Hz world; a 90, 100 or 144 Hz
    /// world does not divide it, and a 30 Hz world's substep is longer than it.
    #[test]
    fn a_tick_is_a_whole_number_of_substeps_or_says_why_not() {
        let tick = 1.0 / 60.0;
        for (hz, k) in [(60, 1), (120, 2), (180, 3), (240, 4), (300, 5), (480, 8), (960, 16)] {
            assert_eq!(substeps_per_tick(&world(hz), tick), Ok(k), "{hz} Hz");
        }
        for hz in [90, 100, 144] {
            assert!(matches!(substeps_per_tick(&world(hz), tick), Err(TickMismatch::NotWholeSubsteps { substep_rate, .. }) if substep_rate == hz), "{hz} Hz");
        }
        assert!(matches!(substeps_per_tick(&world(30), tick), Err(TickMismatch::ShorterThanSubstep { .. })));
        assert!(matches!(substeps_per_tick(&world(240), f32::NAN), Err(TickMismatch::ShorterThanSubstep { .. })));
        assert!(matches!(substeps_per_tick(&world(240), 0.0), Err(TickMismatch::ShorterThanSubstep { .. })));
        assert!(matches!(substeps_per_tick(&world(240), f32::INFINITY), Err(TickMismatch::NotWholeSubsteps { .. })));
        // A step built from the world's own substep, at a rate a 1/hz would not hit exactly.
        assert_eq!(substeps_per_tick(&world(300), 3.0 * world(300).substep_dt()), Ok(3));
    }

    /// The report names both rates and what would fit: the world rates around the tick's multiples,
    /// and the tick rates that divide the world's.
    #[test]
    fn the_report_names_both_rates_and_what_fits() {
        let text = substeps_per_tick(&world(90), 1.0 / 60.0).unwrap_err().to_string();
        for expected in ["60.00 Hz", "90 Hz", "60 Hz or 120 Hz", "45 Hz or 90 Hz"] {
            assert!(text.contains(expected), "{expected:?} is not in: {text}");
        }
        let text = substeps_per_tick(&world(30), 1.0 / 60.0).unwrap_err().to_string();
        assert!(text.contains("shorter than one substep") && text.contains("60 Hz") && text.contains("30 Hz"), "{text}");
    }

    /// **The app's fixed tick is checked against the world it steps, once.** A 60 Hz tick on a 90 Hz
    /// world logs one error however many ticks run, and none on a 120 Hz world; a tick changed to
    /// one that does not fit is reported again.
    #[test]
    fn the_step_system_reports_a_tick_that_does_not_fit_once() {
        use gizmo_core::system::System;
        let run = |hz: u32, ticks: &[f32]| {
            let mut w = crate::core::World::new();
            w.insert_resource(world(hz));
            let mut system = PhysicsStepSystem::new();
            errors_during(|| {
                for &dt in ticks {
                    system.run(&w, dt);
                }
            })
        };
        let sixty = [1.0 / 60.0; 10];
        let errors = run(90, &sixty);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("90 Hz") && errors[0].contains("PhysicsTime"), "{}", errors[0]);
        assert!(run(120, &sixty).is_empty(), "a 60 Hz tick fits a 120 Hz world");
        let errors = run(120, &[1.0 / 60.0, 1.0 / 60.0, 1.0 / 100.0, 1.0 / 100.0]);
        assert_eq!(errors.len(), 1, "the tick changed to one that does not fit: {errors:?}");
    }

    /// The plugin's step says when the world it steps is not at the rate the plugin was asked for.
    #[test]
    fn the_plugins_step_reports_a_world_at_another_rate_than_it_asked() {
        use gizmo_core::system::System;
        let mut w = crate::core::World::new();
        w.insert_resource(world(240));
        let mut system = PhysicsStepSystem::for_rate(Some(120));
        let errors = errors_during(|| {
            for _ in 0..5 {
                system.run(&w, 1.0 / 60.0);
            }
        });
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("asked for 120 Hz") && errors[0].contains("240 Hz"), "{}", errors[0]);
        let mut system = PhysicsStepSystem::for_rate(Some(240));
        assert!(errors_during(|| system.run(&w, 1.0 / 60.0)).is_empty());
    }
}

#[cfg(test)]
mod physics_step_tests {
    use super::*;
    // Named here rather than taken from the module's imports, which are the renderer's: the step
    // tests need none of it, and a physics-only build compiles them too.
    use crate::math::Vec3;
    use gizmo_physics_core::{Collider, Transform};
    use gizmo_physics_rigid::components::RigidBody;
    use crate::core::World;
    use gizmo_core::system::System;
    use gizmo_physics_rigid::components::Velocity;
    use gizmo_physics_rigid::world::PhysicsWorld;

    #[test]
    fn physics_step_system_advances_the_world_under_gravity() {
        // Proves the schedulable wrapper actually steps physics (what PhysicsPlugin
        // now runs automatically at the fixed timestep).
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

        let body = world.spawn();
        world.add_component(body, Transform::new(Vec3::new(0.0, 10.0, 0.0)));
        world.add_component(body, RigidBody::new(1.0, true));
        world.add_component(body, Velocity::default());
        world.add_component(body, Collider::sphere(0.5));

        let y0 = world.query::<&Transform>().get(body).unwrap().position.y;
        let mut sys = PhysicsStepSystem::new();
        for _ in 0..30 {
            sys.run(&world, 1.0 / 60.0);
        }
        let y1 = world.query::<&Transform>().get(body).unwrap().position.y;
        assert!(y1 < y0 - 0.5, "body must fall under gravity via the scheduled step: {y0} -> {y1}");
    }
    /// A `Breakable` box with a heavy ball dropped on it, stepped by the facade's own step.
    /// Returns the world and the box.
    #[cfg(feature = "render")]
    fn breakable_under_a_ball(lifetime: f32) -> (World, crate::core::Entity) {
        use crate::renderer::components::MaterialDesc;
        use gizmo_physics_rigid::components::Breakable;
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
        let ground = world.spawn();
        world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
        world.add_component(ground, RigidBody::new_static());
        world.add_component(ground, Velocity::default());
        world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)));
        let target = world.spawn();
        // Scaled to its half-extents, the way a game sizes a `standard_cube`.
        world.add_component(target, Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)));
        world.add_component(target, RigidBody::new(5.0, true));
        world.add_component(target, Velocity::default());
        world.add_component(target, Collider::box_collider(Vec3::splat(0.5)));
        world.add_component(target, MaterialDesc::new().with_unlit(gizmo_math::Vec4::new(0.9, 0.1, 0.1, 1.0)));
        world.add_component(target, Breakable {
            max_pieces: 6,
            threshold: 1.0,
            current_health: 1.0,
            debris_lifetime: lifetime,
            ..Default::default()
        });
        let ball = world.spawn();
        world.add_component(ball, Transform::new(Vec3::new(0.0, 2.0, 0.0)));
        world.add_component(ball, RigidBody::new(50.0, true));
        world.add_component(ball, Velocity::new(Vec3::new(0.0, -10.0, 0.0)));
        world.add_component(ball, Collider::sphere(0.4));
        (world, target)
    }

    #[cfg(feature = "render")]
    fn debris_ids(world: &World) -> Vec<gizmo_core::Entity> {
        world.query::<(gizmo_core::Entity, &gizmo_physics_rigid::components::Debris)>().iter().map(|(entity, _)| entity).collect()
    }

    #[test]
    #[cfg(feature = "render")]
    fn a_breakable_shatters_in_the_facade_step_into_pieces_drawn_in_its_material() {
        use crate::renderer::components::{MaterialDesc, MeshData, MeshRenderer};
        let (mut world, target) = breakable_under_a_ball(0.0);
        for _ in 0..30 {
            cpu_physics_step_system(&world, 1.0 / 60.0);
            world.apply_commands();
        }
        assert!(!world.is_alive(target), "the box was hit by 50 kg at 10 m/s and is still whole");
        let pieces = debris_ids(&world);
        assert!(pieces.len() >= 2, "{} pieces", pieces.len());
        let red = MaterialDesc::new().with_unlit(gizmo_math::Vec4::new(0.9, 0.1, 0.1, 1.0));
        for id in pieces {
            let mesh = world.query::<&MeshData>().get(id).map(|m| m.vertices().len()).unwrap_or(0);
            assert!(mesh >= 12 && mesh % 3 == 0, "piece {id} has {mesh} vertices to draw");
            assert!(world.query::<&MeshRenderer>().get(id).is_some(), "piece {id} is not drawn");
            assert_eq!(world.query::<&MaterialDesc>().get(id).cloned(), Some(red.clone()), "piece {id}'s material");
            let scale = world.query::<&Transform>().get(id).unwrap().scale;
            assert_eq!(scale, Vec3::ONE, "piece {id} would be sized twice");
        }
    }

    #[test]
    #[cfg(feature = "render")]
    fn debris_is_removed_after_its_lifetime() {
        let (mut world, _) = breakable_under_a_ball(0.5);
        let mut most = 0;
        let mut broke_at = None;
        for step in 0..120 {
            cpu_physics_step_system(&world, 1.0 / 60.0);
            world.apply_commands();
            let n = debris_ids(&world).len();
            if n > 0 && broke_at.is_none() {
                broke_at = Some(step);
            }
            most = most.max(n);
        }
        let broke_at = broke_at.expect("the box never broke");
        assert!(most >= 2, "{most} pieces");
        assert!(broke_at + 32 < 120, "the run is too short to see the lifetime pass");
        assert_eq!(debris_ids(&world).len(), 0, "pieces outlived their 0.5 s lifetime");
    }
}

#[cfg(all(test, feature = "render"))]
mod debug_line_tests {
    use super::*;
    use gizmo_physics_core::Collider;

    /// Every line end the overlay draws for `collider` alone at `at`.
    fn outline(collider: Collider, at: Transform) -> Vec<Vec3> {
        let mut world = crate::core::World::new();
        let e = world.spawn();
        world.add_component(e, at);
        world.add_component(e, collider);
        let mut gizmos = crate::renderer::Gizmos::default();
        physics_debug_lines(&world, &mut gizmos);
        gizmos.lines.iter().map(|v| Vec3::from(v.position)).collect()
    }

    /// A sphere collider is outlined on its surface. It was an axis-aligned box round it.
    #[test]
    fn a_sphere_collider_is_outlined_on_its_surface() {
        let centre = Vec3::new(1.0, 2.0, 3.0);
        let points = outline(Collider::sphere(0.5), Transform::new(centre));
        assert!(!points.is_empty());
        for p in points {
            assert!(((p - centre).length() - 0.5).abs() < 1e-4, "{p:?} is not on the sphere");
        }
    }

    /// A capsule collider is outlined as one: within its radius of its axis, and reaching
    /// `half_height + radius` along it. It fell to the catch-all, a 2 m box at the origin.
    #[test]
    fn a_capsule_collider_is_outlined_as_a_capsule() {
        let mut at = Transform::new(Vec3::new(0.0, 3.0, 0.0));
        at.rotation = gizmo_math::Quat::from_rotation_z(0.6);
        let (radius, half_height) = (0.4, 0.9);
        let points = outline(Collider::capsule(radius, half_height), at);
        let axis = at.rotation.mul_vec3(Vec3::Y);
        let mut reach: f32 = 0.0;
        for p in &points {
            let v = *p - at.position;
            let along = v.dot(axis);
            let off_axis = (v - axis * along).length();
            assert!(off_axis <= radius + 1e-4, "{p:?} is {off_axis} from the axis");
            let to_core = (along.abs() - half_height).max(0.0);
            assert!((to_core * to_core + off_axis * off_axis).sqrt() <= radius + 1e-4, "{p:?} is outside the capsule");
            reach = reach.max(along.abs());
        }
        assert!((reach - (half_height + radius)).abs() < 1e-4, "the outline reaches {reach} along the axis");
    }

    /// A triangle mesh is outlined by its bounds, where it is. It was a 2 m box at the origin.
    #[test]
    fn a_trimesh_collider_is_outlined_by_its_bounds() {
        let at = Transform::new(Vec3::new(10.0, 0.0, 0.0));
        let mesh = Collider::trimesh(vec![Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 3.0)], vec![0, 1, 2]);
        let points = outline(mesh, at);
        assert_eq!(points.len(), 24, "twelve edges");
        let lo = points.iter().fold(Vec3::splat(f32::INFINITY), |a, &p| a.min(p));
        let hi = points.iter().fold(Vec3::splat(f32::NEG_INFINITY), |a, &p| a.max(p));
        assert!((lo - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-4 && (hi - Vec3::new(14.0, 1.0, 3.0)).length() < 1e-4, "{lo:?}..{hi:?}");
    }
}
