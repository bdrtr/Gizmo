use crate::math::Vec3;
use crate::physics::{Collider, ColliderShape, GpuPhysicsLink, RigidBody, Transform};
use crate::renderer::Renderer;


pub fn physics_debug_system(world: &crate::core::World) {
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        // Renk: Parlak Yeşil (R, G, B, A)
        let color = [0.1, 0.9, 0.1, 1.0];

        if let Some(q) = world.query::<(&crate::physics::Transform, &gizmo_physics::Collider)>() {
            for (_, (trans, col)) in q.iter() {
                // To support proper rotation, we should draw the 8 corners of the box.
                match &col.shape {
                    gizmo_physics::ColliderShape::Box(b) => {
                        let h = b.half_extents;
                        let p0 = trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y, -h.z));
                        let p1 = trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y, -h.z));
                        let p2 = trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y, -h.z));
                        let p3 = trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y, -h.z));
                        let p4 = trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y,  h.z));
                        let p5 = trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y,  h.z));
                        let p6 = trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y,  h.z));
                        let p7 = trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y,  h.z));
                        
                        gizmos.draw_line(p0, p1, color); gizmos.draw_line(p1, p2, color);
                        gizmos.draw_line(p2, p3, color); gizmos.draw_line(p3, p0, color);
                        gizmos.draw_line(p4, p5, color); gizmos.draw_line(p5, p6, color);
                        gizmos.draw_line(p6, p7, color); gizmos.draw_line(p7, p4, color);
                        gizmos.draw_line(p0, p4, color); gizmos.draw_line(p1, p5, color);
                        gizmos.draw_line(p2, p6, color); gizmos.draw_line(p3, p7, color);
                    }
                    gizmo_physics::ColliderShape::Sphere(s) => {
                        let r = s.radius;
                        let min = trans.position - Vec3::new(r, r, r);
                        let max = trans.position + Vec3::new(r, r, r);
                        gizmos.draw_box(min, max, color);
                    }
                    _ => {
                        let min = trans.position - Vec3::new(1.0, 1.0, 1.0);
                        let max = trans.position + Vec3::new(1.0, 1.0, 1.0);
                        gizmos.draw_box(min, max, color);
                    }
                }
            }
        }
        
        let soft_color = [1.0, 0.4, 0.8, 1.0]; // Pinkish for soft body
        if let Some(q) = world.query::<&gizmo_physics::soft_body::SoftBodyMesh>() {
            for (_, sm) in q.iter() {
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
        if let Some(q) = world.query::<(&crate::physics::Transform, &gizmo_physics::vehicle::VehicleController)>() {
            for (_, (trans, vehicle)) in q.iter() {
                for wheel in &vehicle.wheels {
                    let attach_world = trans.position + trans.rotation.mul_vec3(wheel.attachment_local_pos);
                    let ray_dir = trans.rotation.mul_vec3(wheel.direction_local).normalize();
                    let ray_end = attach_world + ray_dir * (wheel.suspension_rest_length + wheel.suspension_max_travel + wheel.radius);
                    
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

        // --- Phase 6.2: Temas Normalleri ve Penetrasyon Derinliği ---
        if let Some(phys_world) = world.get_resource::<gizmo_physics::world::PhysicsWorld>() {
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
}

/// ECS'deki yeni yaratılmış Fiziksel Objeleri (RigidBody + Transform + Collider)
/// GPU Physics çekirdeğinin otoyoluna (GpuPhysicsSystem::spheres_buffer) kaydeder.
/// Statik collider'lar için ayrı sayaç. İlk 3 slot başlangıç collider'larına ayrılmıştır.
static NEXT_STATIC_COLLIDER_SLOT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(3);

pub fn gpu_physics_submit_system(world: &mut crate::core::World, renderer: &Renderer) {
    use crate::physics::Velocity;

    if let Some(physics) = &renderer.gpu_physics {
        let mut unlinked_entities = Vec::new();
        if let Some(q) = world.query::<(&RigidBody, &Transform, &Collider)>() {
            let links = world.borrow::<GpuPhysicsLink>();
            let velocities = world.borrow::<Velocity>();
            for (e, (rb, trans, col)) in q.iter() {
                if links.get(e).is_none() {
                    let vel = velocities.get(e).map(|v| *v).unwrap_or_default();
                    unlinked_entities.push((e, *rb, *trans, col.clone(), vel));
                }
            }
        }

        let mut next_dynamic_id = world
            .query::<&GpuPhysicsLink>()
            .map(|q| q.iter().count() as u32)
            .unwrap_or(0);

        for (e, rb, trans, col, vel) in unlinked_entities {
            if matches!(col.shape, ColliderShape::Plane(_)) {
                // Statik engel — ayrı slot sayacı kullan
                let slot =
                    NEXT_STATIC_COLLIDER_SLOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if slot >= 100 {
                    eprintln!("[GpuPhysics] Statik collider slot limiti (100) aşıldı, collider atlanıyor.");
                    NEXT_STATIC_COLLIDER_SLOT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }

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
                physics.update_collider(&renderer.queue, slot, &gpu_col);
            } else {
                // Dinamik Kutu (AABB)
                let id = next_dynamic_id;
                next_dynamic_id += 1;

                let extents = match &col.shape {
                    ColliderShape::Box(b) => {
                        [b.half_extents.x, b.half_extents.y, b.half_extents.z]
                    }
                    _ => [0.5, 0.5, 0.5],
                };

                let gpu_box = gizmo_renderer::gpu_physics::GpuBox {
                    position: [trans.position.x, trans.position.y, trans.position.z],
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
                physics.update_box(&renderer.queue, id, &gpu_box);

                world.add_component(world.get_entity(e).unwrap(), GpuPhysicsLink { id });
            }
        }
    }
}

/// GPU'dan Asenkron (0ms) çekilen devasa Fizik lokasyon durumlarını,
/// Ekrandaki objelerin render edilmesi için ECS'deki Transform'larına kopyalar.
pub fn gpu_physics_readback_system(world: &mut crate::core::World, renderer: &Renderer) {
    if let Some(physics) = &renderer.gpu_physics {
        if let Some(gpu_data) = physics.poll_readback_data(&renderer.device) {
            if let Some(mut q) =
                world.query::<(gizmo_core::prelude::Mut<Transform>, &GpuPhysicsLink)>()
            {
                for (_, (mut trans, link)) in q.iter_mut() {
                    let idx = link.id as usize;
                    if idx < gpu_data.len() {
                        let box_data = &gpu_data[idx];
                        trans.position = gizmo_math::Vec3::new(
                            box_data.position[0],
                            box_data.position[1],
                            box_data.position[2],
                        );
                        trans.rotation = gizmo_math::Quat::from_xyzw(
                            box_data.rotation[0],
                            box_data.rotation[1],
                            box_data.rotation[2],
                            box_data.rotation[3],
                        );
                        trans.update_local_matrix();
                    }
                }
            }
        }
    }
}

/// Phase 7.1: Fluid-Rigid Coupling
/// Senkronize eder: GpuPhysicsLink sahibi objeleri FluidCollider buffer'ına yazar.

pub fn cpu_physics_step_system(world: &crate::core::World, dt: f32) {
    gizmo_physics::system::physics_step_system(world, dt);
}
