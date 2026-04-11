use gizmo::physics::components::{RigidBody, Velocity};
use gizmo::physics::integration::{physics_apply_forces_system, physics_movement_system};
use gizmo::physics::shape::{Aabb, Collider, ColliderShape};
use gizmo::physics::system::{physics_collision_system, PhysicsSolverState};
use gizmo::prelude::*;

fn main() {
    App::<()>::new("Gizmo Tutorial", 800, 600)
        .set_setup(|world, renderer| {
            // Fizik Solver Durumunu ekle (Baumgarte ve Contact algoritmaları için şart)
            world.insert_resource(PhysicsSolverState::new());

            let mut cmd = Commands::new(world, renderer);

            // Zemin (Statik Fiziksel)
            let floor_id = cmd
                .spawn_plane(Vec3::new(0.0, 0.0, 0.0), 20.0, Color::DARK_GRAY)
                .with_name("Zemin");
            cmd.world.add_component(floor_id, RigidBody::new_static());
            cmd.world.add_component(
                floor_id,
                Collider {
                    shape: ColliderShape::Aabb(Aabb {
                        half_extents: Vec3::new(10.0, 0.5, 10.0),
                    }),
                },
            );
            cmd.world.add_component(floor_id, Velocity::new(Vec3::ZERO));

            // Domino Taşlarını Dizelim
            for i in 0..15 {
                let dx = 0.2;
                let dy = 1.0;
                let dz = 0.1;
                let pos = Vec3::new(0.0, dy, -8.0 + (i as f32 * 0.8));

                let domino_id = cmd
                    .spawn_cube(pos, Color::RED)
                    .with_name(&format!("Domino_{}", i));

                let mut rb = RigidBody::new(0.5, 0.1, 0.3, true);
                rb.calculate_box_inertia(dx * 2.0, dy * 2.0, dz * 2.0);

                cmd.world.add_component(domino_id, rb);
                cmd.world.add_component(
                    domino_id,
                    Collider {
                        shape: ColliderShape::Aabb(Aabb {
                            half_extents: Vec3::new(dx, dy, dz),
                        }),
                    },
                );
                cmd.world
                    .add_component(domino_id, Velocity::new(Vec3::ZERO));

                // İlk taşı devirecek başlangıç ivmesi (zinciri tetikle)
                if i == 0 {
                    cmd.world
                        .add_component(domino_id, Velocity::new(Vec3::new(0.0, 0.0, 5.0)));
                }

                // Oyuncu modelimizi gösterelim (Sadece görsel basitlik için oyuncu kutusunu en son dominonun arkasına koyabiliriz, ama gerek yok)
            }

            // Kamera — varsayılan ayarlarla hazır (Yandan bakalım dominolara)
            cmd.spawn_camera(Vec3::new(-10.0, 5.0, -2.0));
        })
        .set_update(|world, _state, dt, input| {
            // Fizik sabit adımlarla (Fixed Timestep) ilerletilir, burada görsel tut için basitçe tick atıyoruz
            physics_apply_forces_system(world, dt);
            physics_collision_system(world, dt);
            physics_movement_system(world, dt);

            // Kamera dönüşü
            let hiz = 8.0 * dt;
            world.move_entity_named("Camera", |trans| {
                if input.pressed(Key::KeyW) {
                    trans.position.y += hiz;
                }
                if input.pressed(Key::KeyS) {
                    trans.position.y -= hiz;
                }
            });
        })
        .set_render(|world, _state, encoder, view, renderer, _t| {
            default_render_pass(world, encoder, view, renderer);
        })
        .run();
}
