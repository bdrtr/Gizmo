use std::f32::consts::{FRAC_PI_2, PI};
use gizmo::prelude::*;
use gizmo::simple::{SceneBuilder, SimpleSceneState};
use gizmo::systems;
use gizmo::physics::world::PhysicsWorld;
use gizmo::core::system::{Res, ResMut, IntoSystemConfig, Phase};
use gizmo::core::query::{Query, Mut, With};
use gizmo::core::input::Input;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Front,
    Middle,
    Rear,
}

impl Row {
    fn z(self) -> f32 {
        match self {
            Row::Front => 4.0,
            Row::Middle => 0.0,
            Row::Rear => -4.0,
        }
    }

    fn advance(self) -> Self {
        match self {
            Row::Front => Row::Rear,
            Row::Middle => Row::Front,
            Row::Rear => Row::Middle,
        }
    }
}

// Marker component for shapes
#[derive(Clone, Copy)]
pub struct Shape;

impl gizmo::core::component::Component for Shape {
    fn storage_type() -> gizmo::core::component::StorageType {
        gizmo::core::component::StorageType::Table
    }
}

impl gizmo::core::component::Component for Row {
    fn storage_type() -> gizmo::core::component::StorageType {
        gizmo::core::component::StorageType::Table
    }
}

pub struct DemoState {
    simple: SimpleSceneState,
    rotate: bool,
    time: f32,
}

fn main() {
    let mut app = App::new("Demo2 Gizmo", 1280, 720);

    app = app
        .set_setup(|world, renderer| {
            world.register_component_type::<Shape>();
            world.register_component_type::<Row>();

            let mut asset_manager = AssetManager::new();
            // Yerçekimini sıfırlıyoruz ki cisimler havada asılı kalsın
            let phys_world = PhysicsWorld::new().with_gravity(Vec3::ZERO);

            let mut state = SimpleSceneState {
                camera_speed: 15.0,
                camera_pitch: 0.0,
                camera_yaw: 0.0,
                camera_pos: Vec3::new(0.0, 2.0, 5.0),
            };

            let mut scene = SceneBuilder {
                world,
                renderer,
                asset_manager: &mut asset_manager,
            };

            // Ground plane (radius 25.0 gives 50.0 diameter)
            scene.spawn_ground(25.0);

            // Point Light
            let light_ent = scene.world.spawn();
            let mut bundle = gizmo::bundles::PointLightBundle::default();
            bundle.position = Vec3::new(8.0, 16.0, 8.0);
            bundle.color = Vec3::new(1.0, 1.0, 1.0);
            bundle.intensity = 200.0;
            bundle.apply(scene.world, light_ent);

            // Camera setup
            scene.spawn_camera(&mut state, Vec3::new(0.0, 7.0, 14.0), Vec3::new(0.0, 1.0, 0.0));

            let shapes_x_extent = 14.0;
            let z_extent = 8.0;

            // Front Row (Various Shapes)
            let num_front = 11;
            for i in 0..num_front {
                let x = -shapes_x_extent / 2.0 + (i as f32 / (num_front - 1) as f32) * shapes_x_extent;
                let pos = Vec3::new(x, 2.0, z_extent / 2.0);
                let e = match i {
                    0 => scene.spawn_textured_cube(pos, 1.0),
                    1 => scene.spawn_textured_tetrahedron(pos, 0.5),
                    2 => scene.spawn_textured_capsule(pos, 0.5, 1.0),
                    3 => scene.spawn_textured_torus(pos, 0.4, 0.15),
                    4 => scene.spawn_textured_cylinder(pos, 0.5, 1.0),
                    5 => scene.spawn_textured_cone(pos, 0.5, 1.0),
                    6 => scene.spawn_textured_conical_frustum(pos, 0.5, 0.25, 1.0),
                    7 => scene.spawn_textured_sphere(pos, 0.5),
                    8 => scene.spawn_textured_sphere(pos, 0.5),
                    9 => scene.spawn_textured_cylinder(pos, 0.1, 1.0), // Segment3d approx
                    _ => scene.spawn_textured_cylinder(pos, 0.1, 1.0), // Polyline3d approx
                };
                scene.world.add_component(e, Shape);
                scene.world.add_component(e, Row::Front);
            }

            // Middle Row (Extrusions)
            let num_middle = 8;
            for i in 0..num_middle {
                let x = -shapes_x_extent / 2.0 + (i as f32 / (num_middle - 1) as f32) * shapes_x_extent;
                let pos = Vec3::new(x, 2.0, 0.0);
                let depth = 1.0;
                let e = match i {
                    // Rectangle
                    0 => scene.spawn_textured_convex_extrusion(pos, &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]], depth),
                    // Capsule2d (Approximate with polygon)
                    1 => {
                        let mut pts = Vec::new();
                        for j in 0..8 { let a = j as f32 * PI / 7.0; pts.push([0.5 + 0.5 * a.cos(), 0.5 * a.sin()]); }
                        for j in 0..8 { let a = PI + j as f32 * PI / 7.0; pts.push([-0.5 + 0.5 * a.cos(), 0.5 * a.sin()]); }
                        scene.spawn_textured_convex_extrusion(pos, &pts, depth)
                    },
                    // Annulus
                    2 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..16 {
                            let a = j as f32 * 2.0 * PI / 16.0;
                            i_pts.push([0.25 * a.cos(), 0.25 * a.sin()]);
                            o_pts.push([0.5 * a.cos(), 0.5 * a.sin()]);
                        }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // Circle
                    3 => {
                        let mut pts = Vec::new();
                        for j in 0..16 { let a = j as f32 * 2.0 * PI / 16.0; pts.push([0.5 * a.cos(), 0.5 * a.sin()]); }
                        scene.spawn_textured_convex_extrusion(pos, &pts, depth)
                    },
                    // Ellipse
                    4 => {
                        let mut pts = Vec::new();
                        for j in 0..16 { let a = j as f32 * 2.0 * PI / 16.0; pts.push([0.5 * a.cos(), 0.25 * a.sin()]); }
                        scene.spawn_textured_convex_extrusion(pos, &pts, depth)
                    },
                    // RegularPolygon (Hexagon)
                    5 => {
                        let mut pts = Vec::new();
                        for j in 0..6 { let a = j as f32 * 2.0 * PI / 6.0; pts.push([0.5 * a.cos(), 0.5 * a.sin()]); }
                        scene.spawn_textured_convex_extrusion(pos, &pts, depth)
                    },
                    // Triangle2d
                    6 => scene.spawn_textured_convex_extrusion(pos, &[[-0.5, -0.4], [0.5, -0.4], [0.0, 0.5]], depth),
                    // ConvexPolygon
                    _ => scene.spawn_textured_convex_extrusion(pos, &[[0.0, 0.8], [-0.47, 0.25], [-0.47, -0.65], [0.47, -0.65], [0.47, 0.25]], depth),
                };
                scene.world.add_component(e, Shape);
                scene.world.add_component(e, Row::Middle);
            }

            // Rear Row (Ring Extrusions)
            let num_rear = 7;
            for i in 0..num_rear {
                let x = -shapes_x_extent / 2.0 + (i as f32 / (num_rear - 1) as f32) * shapes_x_extent;
                let pos = Vec3::new(x, 2.0, -z_extent / 2.0);
                let depth = 1.0;
                let e = match i {
                    // Rectangle Ring
                    0 => scene.spawn_textured_ring_extrusion(pos, 
                        &[[-0.4, -0.4], [0.4, -0.4], [0.4, 0.4], [-0.4, 0.4]],
                        &[[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]], depth),
                    // Capsule2d Ring
                    1 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..8 { let a = j as f32 * PI / 7.0; o_pts.push([0.5 + 0.5 * a.cos(), 0.5 * a.sin()]); i_pts.push([0.5 + 0.4 * a.cos(), 0.4 * a.sin()]); }
                        for j in 0..8 { let a = PI + j as f32 * PI / 7.0; o_pts.push([-0.5 + 0.5 * a.cos(), 0.5 * a.sin()]); i_pts.push([-0.5 + 0.4 * a.cos(), 0.4 * a.sin()]); }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // Ring (Circle-Circle)
                    2 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..16 {
                            let a = j as f32 * 2.0 * PI / 16.0;
                            i_pts.push([0.5 * a.cos(), 0.5 * a.sin()]);
                            o_pts.push([1.0 * a.cos(), 1.0 * a.sin()]);
                        }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // Circle Ring
                    3 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..16 {
                            let a = j as f32 * 2.0 * PI / 16.0;
                            i_pts.push([0.4 * a.cos(), 0.4 * a.sin()]);
                            o_pts.push([0.5 * a.cos(), 0.5 * a.sin()]);
                        }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // Ellipse Ring
                    4 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..16 {
                            let a = j as f32 * 2.0 * PI / 16.0;
                            i_pts.push([0.4 * a.cos(), 0.15 * a.sin()]);
                            o_pts.push([0.5 * a.cos(), 0.25 * a.sin()]);
                        }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // RegularPolygon Ring
                    5 => {
                        let mut i_pts = Vec::new(); let mut o_pts = Vec::new();
                        for j in 0..6 { let a = j as f32 * 2.0 * PI / 6.0; i_pts.push([0.4 * a.cos(), 0.4 * a.sin()]); o_pts.push([0.5 * a.cos(), 0.5 * a.sin()]); }
                        scene.spawn_textured_ring_extrusion(pos, &i_pts, &o_pts, depth)
                    },
                    // Triangle2d Ring
                    _ => scene.spawn_textured_ring_extrusion(pos, 
                        &[[-0.3, -0.2], [0.3, -0.2], [0.0, 0.3]],
                        &[[-0.5, -0.4], [0.5, -0.4], [0.0, 0.5]], depth),
                };
                scene.world.add_component(e, Shape);
                scene.world.add_component(e, Row::Rear);
            }

            world.insert_resource(phys_world);
            world.insert_resource(asset_manager);
            world.insert_resource(gizmo::systems::render::WireframeConfig { global: false });

            world.insert_resource(DemoState {
                simple: state,
                rotate: true,
                time: 0.0,
            });
        })
        .add_system(toggle_rotation.in_phase(Phase::Update))
        .add_system(toggle_wireframes.in_phase(Phase::Update))
        .add_system(advance_rows.in_phase(Phase::Update))
        .add_system(rotate_shapes.in_phase(Phase::Update))
        .add_system(camera_movement.in_phase(Phase::Update))
        .add_system((Box::new(PhysicsSystem) as Box<dyn gizmo::core::system::System>).in_phase(Phase::Physics))
        .add_system((Box::new(TransformUpdateSystem) as Box<dyn gizmo::core::system::System>).in_phase(Phase::PostUpdate))
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            
            systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|_world, _state, ctx| {
            gizmo::egui::Area::new("demo2_ui".into())
                .fixed_pos(gizmo::egui::pos2(12.0, 12.0))
                .show(ctx, |ui| {
                    ui.label(gizmo::egui::RichText::new("Press 'R' to pause/resume rotation").color(gizmo::egui::Color32::WHITE));
                    ui.label(gizmo::egui::RichText::new("Press 'Tab' to cycle through rows").color(gizmo::egui::Color32::WHITE));
                    ui.label(gizmo::egui::RichText::new("Press 'Space' to toggle wireframes").color(gizmo::egui::Color32::WHITE));
                });
        });

    app.run();
}

// =================== SYSTEMS ===================

fn toggle_rotation(mut state: ResMut<DemoState>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyR as u32) {
        state.rotate = !state.rotate;
    }
}

fn toggle_wireframes(mut cfg: ResMut<gizmo::systems::render::WireframeConfig>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        cfg.global = !cfg.global;
    }
}

fn advance_rows(mut q: Query<(Mut<Row>, Mut<Transform>, With<Shape>)>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Tab as u32) {
        for (_id, (mut row, mut trans, _)) in q.iter_mut() {
            *row = row.advance();
            trans.position.z = row.z();
        }
    }
}

fn rotate_shapes(mut q: Query<(Mut<Transform>, With<Shape>)>, mut state: ResMut<DemoState>, time: Res<gizmo::core::time::Time>) {
    let dt = time.dt();
    if state.rotate {
        state.time += dt;
        for (_id, (mut trans, _)) in q.iter_mut() {
            trans.rotation = Quat::from_rotation_y(state.time);
            trans.rotation *= Quat::from_rotation_x(state.time * 0.5);
        }
    }
}

fn camera_movement(mut q: Query<(Mut<Transform>, Mut<gizmo::renderer::components::Camera>)>, mut state: ResMut<DemoState>, input: Res<Input>, time: Res<gizmo::core::time::Time>) {
    let dt = time.dt();
    if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
        let delta = input.mouse_delta();
        state.simple.camera_yaw -= delta.0 * 0.005;
        state.simple.camera_pitch -= delta.1 * 0.005;
        state.simple.camera_pitch = state.simple.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }

    let fx = state.simple.camera_yaw.cos() * state.simple.camera_pitch.cos();
    let fy = state.simple.camera_pitch.sin();
    let fz = state.simple.camera_yaw.sin() * state.simple.camera_pitch.cos();
    let forward = Vec3::new(fx, fy, fz).normalize();
    let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
    let up = Vec3::new(0.0, 1.0, 0.0);

    let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) {
        state.simple.camera_speed * 3.0
    } else {
        state.simple.camera_speed
    };

    let mut cam_move = Vec3::ZERO;
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) { cam_move += forward; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) { cam_move -= forward; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) { cam_move += right; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) { cam_move -= right; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) { cam_move += up; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) { cam_move -= up; }

    if cam_move.length_squared() > 0.0 {
        state.simple.camera_pos += cam_move.normalize() * speed * dt;
    }

    let yaw_rot = Quat::from_rotation_y(-state.simple.camera_yaw + FRAC_PI_2);
    let pitch_rot = Quat::from_rotation_x(state.simple.camera_pitch);
    let rot = yaw_rot * pitch_rot;

    for (_id, (mut trans, mut cam)) in q.iter_mut() {
        trans.position = state.simple.camera_pos;
        trans.rotation = rot;
        cam.yaw = state.simple.camera_yaw;
        cam.pitch = state.simple.camera_pitch;
    }
}

// Exclusive system because physics uses `&World` everywhere and modifies things internally.
struct PhysicsSystem;
impl gizmo::core::system::System for PhysicsSystem {
    fn run(&mut self, world: &gizmo::core::world::World, dt: f32) {
        let mut physics_dt = dt.min(0.1);
        while physics_dt > 0.0 {
            let step = physics_dt.min(0.016);
            systems::cpu_physics_step_system(world, step);
            physics_dt -= step;
        }
    }
    
    fn access_info(&self) -> gizmo::core::system::AccessInfo {
        gizmo::core::system::AccessInfo::new() // Not strictly accurate, but fine for exclusive
    }
}

// Exclusive system because TransformSyncSystem has its own `run(&mut self, world, dt)` method
struct TransformUpdateSystem;
impl gizmo::core::system::System for TransformUpdateSystem {
    fn run(&mut self, world: &gizmo::core::world::World, dt: f32) {
        let mut transform_sync = systems::transform::TransformSyncSystem;
        let mut transform_propagate = systems::transform::TransformPropagateSystem;
        transform_sync.run(world, dt);
        transform_propagate.run(world, dt);
    }
    
    fn access_info(&self) -> gizmo::core::system::AccessInfo {
        gizmo::core::system::AccessInfo::new()
    }
}
