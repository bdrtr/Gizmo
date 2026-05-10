use crate::physics::{Collider, ColliderShape, Transform};
use crate::renderer::Renderer;
use bytemuck;

pub fn gpu_fluid_coupling_system(world: &crate::core::World, renderer: &mut Renderer) {
    use gizmo_renderer::gpu_fluid::types::FluidCollider;
    use gizmo_renderer::gpu_fluid::types::MAX_FLUID_COLLIDERS;

    if let Some(fluid) = &mut renderer.gpu_fluid {
        let mut colliders = vec![
            FluidCollider {
                position: [0.0; 3],
                radius: 0.0,
                velocity: [0.0; 3],
                shape_type: 0,
                half_extents: [0.0; 3],
                _pad: 0.0,
            };
            MAX_FLUID_COLLIDERS
        ];

        let mut count = 0;

        if let Some(q) = world.query::<(&Transform, &crate::physics::Velocity, &Collider)>() {
            for (_, (trans, vel, col)) in q.iter() {
                if count >= MAX_FLUID_COLLIDERS {
                    break;
                }

                // Sadece belli y altındaki veya dinamik olanları eklemek isteyebiliriz, ama şimdilik hepsini ekleyelim
                let shape_type;
                let mut radius = 0.0;
                let mut half_extents = [0.0; 3];

                match &col.shape {
                    ColliderShape::Sphere(s) => {
                        shape_type = 0;
                        radius = s.radius;
                    }
                    ColliderShape::Box(b) => {
                        shape_type = 1;
                        half_extents = [b.half_extents.x, b.half_extents.y, b.half_extents.z];
                    }
                    _ => continue, // Sadece Sphere ve Box destekliyoruz
                }

                colliders[count] = FluidCollider {
                    position: [trans.position.x, trans.position.y, trans.position.z],
                    radius,
                    velocity: [vel.linear.x, vel.linear.y, vel.linear.z],
                    shape_type,
                    half_extents,
                    _pad: 0.0,
                };
                count += 1;
            }
        }

        // GPU'ya yaz
        renderer
            .queue
            .write_buffer(&fluid.colliders_buffer, 0, bytemuck::cast_slice(&colliders));

        // Fluid Params num_colliders güncelle
        fluid.update_colliders_count(&renderer.queue, count as u32);
    }
}
