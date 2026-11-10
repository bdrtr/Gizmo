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
/// Draws the physics debug view: every collider's shape, plus the velocity and contact gizmos
/// on top of it.
pub fn physics_debug_system(world: &crate::core::World) {
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        fn draw_collider(
            trans: &Transform,
            col: &Collider,
            color: [f32; 4],
            gizmos: &mut crate::renderer::Gizmos,
        ) {
            match &col.shape {
                gizmo_physics_core::ColliderShape::Box(b) => {
                    let h = b.half_extents;
                    let r = trans.rotation;
                    let p = trans.position;
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
                gizmo_physics_core::ColliderShape::Sphere(s) => {
                    let r = s.radius;
                    let min = trans.position - Vec3::new(r, r, r);
                    let max = trans.position + Vec3::new(r, r, r);
                    gizmos.draw_box(min, max, color);
                }
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
                    let (lo, hi) = (hf.local_aabb.min, hf.local_aabb.max);
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
                _ => {
                    let min = trans.position - Vec3::new(1.0, 1.0, 1.0);
                    let max = trans.position + Vec3::new(1.0, 1.0, 1.0);
                    gizmos.draw_box(min, max, color);
                }
            }
        }

        {
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
                draw_collider(trans, col, color, &mut gizmos);
            }
        }

        {
            let q = world.query_filtered::<(&gizmo_physics_core::Transform, &gizmo_physics_core::Collider), crate::core::query::Without<gizmo_physics_rigid::components::RigidBody>>();
            for (trans, col) in q.iter() {
                draw_collider(trans, col, [0.5, 0.5, 0.5, 1.0], &mut gizmos);
            }
        }

        // --- Phase 5.5: Hitbox / Hurtbox Visuals ---
        let draw_oriented_box = |trans: &gizmo_physics_core::Transform,
                                 offset: gizmo_math::Vec3,
                                 h: gizmo_math::Vec3,
                                 color: [f32; 4],
                                 gizmos: &mut crate::renderer::Gizmos| {
            let p = trans.position + trans.rotation.mul_vec3(offset);
            let r = trans.rotation;
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
        };

        {
            let q = world.query::<(&gizmo_physics_core::Transform, &gizmo_physics_core::components::Hitbox)>();
            for (trans, hitbox) in q.iter() {
                if hitbox.active {
                    draw_oriented_box(trans, hitbox.offset, hitbox.half_extents, [1.0, 0.0, 0.0, 1.0], &mut gizmos); // RED
                }
            }
        }

        {
            let q = world.query::<(&gizmo_physics_core::Transform, &gizmo_physics_core::components::Hurtbox)>();
            for (trans, hurtbox) in q.iter() {
                draw_oriented_box(trans, hurtbox.offset, hurtbox.half_extents, [0.0, 1.0, 0.0, 1.0], &mut gizmos); // GREEN
            }
        }

        #[cfg(feature = "physics-soft")]
        let soft_color = [1.0, 0.4, 0.8, 1.0]; // Pinkish for soft body
        #[cfg(feature = "physics-soft")]
        {
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

        // --- Phase 6.1: Süspansiyon Raycast Çizgisi + Kuvvet Okları ---
        #[cfg(feature = "physics-dynamics")]
        {
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

        // (Ölü rigid `Vehicle` süspansiyon debug-draw bloğu kaldırıldı — o araç sistemi silindi;
        // kanonik dynamics `VehicleController` debug-draw'u yukarıda.)

        // --- Phase 6.2: Temas Normalleri ve Penetrasyon Derinliği ---
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
        tracing::trace!(vertices = gizmos.vertex_count(), "physics_debug_system: gizmo lines");
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
/// The step includes **fracture**: a [`Breakable`](gizmo_physics_rigid::components::Breakable)
/// hit hard enough shatters, its pieces age and are removed after its `debris_lifetime`, and —
/// with the `render` feature — each piece is drawn as its own Voronoi cell in the broken body's
/// material. Until 2026-11-03 neither this step nor anything else ran the fracture system, so a
/// `Breakable` in a game never broke unless the game called
/// `physics_fracture_system` itself, and the debris it then got had no geometry to draw.
pub fn cpu_physics_step_system(world: &crate::core::World, dt: f32) {
    gizmo_physics_rigid::system::physics_step_system(world, dt);
    gizmo_physics_rigid::system::physics_fracture_system(world, dt);
    #[cfg(feature = "render")]
    queue_debris_visuals(world);

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
            .filter(|(entity, _)| drawn.get(*entity).is_none())
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

/// Schedulable wrapper around [`cpu_physics_step_system`].
///
/// [`crate::plugins::PhysicsPlugin`] adds this to the app schedule so the physics
/// step runs automatically at the app's **fixed timestep** (the accumulator loop
/// driven by `PhysicsTime`) — callers no longer hand-call `cpu_physics_step_system`
/// every frame, and physics gets a stable fixed step for free.
pub struct PhysicsStepSystem;

impl gizmo_core::system::System for PhysicsStepSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        // Exclusive barrier — mirrors `physics_step_system`'s whole-World access.
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &crate::core::World, dt: f32) {
        cpu_physics_step_system(world, dt);
    }
}

#[cfg(test)]
mod physics_step_tests {
    use gizmo_core::Entity;
    use super::*;
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
        let mut sys = PhysicsStepSystem;
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
    fn debris_ids(world: &World) -> Vec<Entity> {
        world.query::<(Entity, &gizmo_physics_rigid::components::Debris)>().iter().map(|(entity, _)| entity).collect()
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
