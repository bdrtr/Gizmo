use crate::app::App;
use crate::core::world::World;
use crate::core::Bundle;
use crate::bundles::{RigidBodyBundle, CameraBundle};
use crate::physics::components::{Collider, Transform};
use crate::physics::world::PhysicsWorld;
use crate::renderer::asset::AssetManager;
use crate::renderer::components::{Camera, Material, MeshRenderer};
use crate::renderer::Renderer;
use std::f32::consts::{FRAC_PI_2, PI};
use crate::math::{Quat, Vec3, Vec4};
use crate::systems;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleSceneState {
    pub camera_speed: f32,
    pub camera_pitch: f32,
    pub camera_yaw: f32,
    pub camera_pos: Vec3,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct CameraSettings {
    pub speed: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub pos: Vec3,
    pub exposure: f32,
    pub bloom_intensity: f32,
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            speed: 15.0,
            pitch: 0.0,
            yaw: 0.0,
            pos: Vec3::new(0.0, 2.0, 5.0),
            exposure: 1.0,
            bloom_intensity: 0.05,
        }
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct LightingSettings {
    pub preset: u32,
    pub preset_2: u32,
    pub blend_t: f32,
    pub auto_cycle: bool,
    pub rotation_speed: f32,
    pub direct_intensity: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            preset: 0,
            preset_2: 1,
            blend_t: 0.0,
            auto_cycle: false,
            rotation_speed: 1.0,
            direct_intensity: 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum CameraState {
    Orbiting,
    Stationary,
    #[default]
    Manual,
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
pub enum EditorState {
    #[default]
    PlayMode,
    EditMode,
    Paused,
}



pub struct SceneBuilder<'a> {
    pub world: &'a mut World,
    pub renderer: &'a Renderer,
    pub asset_manager: &'a mut AssetManager,
}

impl<'a> SceneBuilder<'a> {
    pub fn spawn_cube(&mut self, position: Vec3, size: f32, color: Vec3) {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let tex = self.asset_manager.create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(color.x, color.y, color.z, 1.0), 1.0, 0.0);

        // Gizmo cube is from -1.0 to 1.0 (size 2.0).
        // To get a cube of `size`, we scale by `size / 2.0`.
        let half_extents = size / 2.0;

        let ent = self.world.spawn();
        self.world.add_component(
            ent,
            Transform::new(position).with_scale(Vec3::splat(half_extents)),
        );
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::box_collider(Vec3::splat(half_extents))));
    }

    pub fn spawn_sphere(&mut self, position: Vec3, radius: f32, color: Vec3) {
        let mesh = AssetManager::create_sphere(&self.renderer.device, radius, 32, 32);
        let tex = self.asset_manager.create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(color.x, color.y, color.z, 1.0), 1.0, 0.0);

        let ent = self.world.spawn();
        self.world.add_component(
            ent,
            Transform::new(position), // Mesh is created with exactly the given radius
        );
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius)));
    }

    pub fn spawn_textured_cube(&mut self, position: Vec3, size: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_cube(&self.renderer.device);
        let tex = self.asset_manager.create_uv_debug_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);

        let half_extents = size / 2.0;
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position).with_scale(Vec3::splat(half_extents)));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::box_collider(Vec3::splat(half_extents))));
        ent
    }

    pub fn spawn_textured_sphere(&mut self, position: Vec3, radius: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_sphere(&self.renderer.device, radius, 32, 32);
        let tex = self.asset_manager.create_uv_debug_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);

        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius)));
        ent
    }

    pub fn spawn_textured_cylinder(&mut self, position: Vec3, radius: f32, height: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_cylinder(&self.renderer.device, radius, height, 32);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius)));
        ent
    }

    pub fn spawn_textured_cone(&mut self, position: Vec3, radius: f32, height: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_cone(&self.renderer.device, radius, height, 32);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius))); // approximation
        ent
    }

    pub fn spawn_textured_torus(&mut self, position: Vec3, radius: f32, tube_radius: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_torus(&self.renderer.device, radius, tube_radius, 32, 16);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius + tube_radius))); // approximation
        ent
    }

    pub fn spawn_textured_capsule(&mut self, position: Vec3, radius: f32, depth: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_capsule(&self.renderer.device, radius, depth, 16, 32);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius)));
        ent
    }

    pub fn spawn_textured_tetrahedron(&mut self, position: Vec3, size: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_tetrahedron(&self.renderer.device, size);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(size)));
        ent
    }

    pub fn spawn_textured_conical_frustum(&mut self, position: Vec3, radius_bottom: f32, radius_top: f32, height: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_conical_frustum(&self.renderer.device, radius_bottom, radius_top, height, 32);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(radius_bottom.max(radius_top))));
        ent
    }

    pub fn spawn_textured_convex_extrusion(&mut self, position: Vec3, points: &[[f32; 2]], depth: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_convex_extrusion(&self.renderer.device, points, depth);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        // Since we are in XZ plane initially in demo2, let's allow it to just spawn.
        // Or actually the generated extrusion might be in XZ plane already. 
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(1.0)));
        ent
    }

    pub fn spawn_textured_ring_extrusion(&mut self, position: Vec3, inner_points: &[[f32; 2]], outer_points: &[[f32; 2]], depth: f32) -> crate::core::entity::Entity {
        let mesh = AssetManager::create_ring_extrusion(&self.renderer.device, inner_points, outer_points, depth);
        let tex = self.asset_manager.create_uv_debug_texture(&self.renderer.device, &self.renderer.queue, &self.renderer.scene.texture_bind_group_layout);
        let mat = Material::new(tex).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.6, 0.0);
        let ent = self.world.spawn();
        self.world.add_component(ent, Transform::new(position));
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::sphere(1.0)));
        ent
    }

    pub fn spawn_ground(&mut self, radius: f32) {
        let mesh = AssetManager::create_plane(&self.renderer.device, radius * 2.0);
        let tex = self.asset_manager.create_white_texture(
            &self.renderer.device,
            &self.renderer.queue,
            &self.renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.2, 0.2, 0.2, 1.0), 0.9, 0.0);

        let ent = self.world.spawn();
        self.world.add_component(
            ent,
            Transform::new(Vec3::new(0.0, 0.0, 0.0)),
        );
        self.world.add_component(ent, crate::physics::components::GlobalTransform::default());
        self.world.add_component(ent, mesh);
        self.world.add_component(ent, mat);
        self.world.add_component(ent, MeshRenderer::new());
        self.world.add_bundle(ent, RigidBodyBundle::static_body().with_collider(Collider::plane(Vec3::new(0.0, 1.0, 0.0), 0.0)));
    }

    pub fn spawn_point_light(&mut self, position: Vec3) {
        let light_ent = self.world.spawn();
        let bundle = crate::bundles::PointLightBundle {
            position,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 20.0,
            ..Default::default()
        };

        bundle.apply(self.world, light_ent);
    }
    
    pub fn spawn_camera(&mut self, state: &mut SimpleSceneState, pos: Vec3, look_at: Vec3) {
        let look_dir = (look_at - pos).normalize_or_zero();
        state.camera_pos = pos;
        if look_dir != Vec3::ZERO {
            state.camera_yaw = look_dir.z.atan2(look_dir.x);
            state.camera_pitch = look_dir.y.asin();
        }

        let camera_ent = self.world.spawn();
        let bundle = CameraBundle {
            position: state.camera_pos,
            yaw: state.camera_yaw,
            pitch: state.camera_pitch,
            ..Default::default()
        };

        bundle.apply(self.world, camera_ent);
    }
}

pub trait SimpleAppExt {
    fn with_simple_scene<F>(self, setup_fn: F) -> Self
    where
        F: FnOnce(&mut SceneBuilder, &mut SimpleSceneState) + 'static;
}

impl SimpleAppExt for App<SimpleSceneState> {
    fn with_simple_scene<F>(self, setup_fn: F) -> Self
    where
        F: FnOnce(&mut SceneBuilder, &mut SimpleSceneState) + 'static,
    {
        self.set_setup(move |world, renderer| {
            let mut asset_manager = AssetManager::new();
            let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

            let mut state = SimpleSceneState {
                camera_speed: 15.0,
                camera_pitch: 0.0,
                camera_yaw: 0.0,
                camera_pos: Vec3::new(0.0, 2.0, 5.0),
            };

            let mut builder = SceneBuilder {
                world,
                renderer,
                asset_manager: &mut asset_manager,
            };

            setup_fn(&mut builder, &mut state);

            world.insert_resource(phys_world);
            world.insert_resource(asset_manager);
            state
        })
        .set_update(|world, state, dt, input| {
            if input.is_mouse_button_pressed(1) {
                let delta = input.mouse_delta();
                state.camera_yaw -= delta.0 * 0.005;
                state.camera_pitch -= delta.1 * 0.005;
                state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
            }

            let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
            let fy = state.camera_pitch.sin();
            let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
            let forward = Vec3::new(fx, fy, fz).normalize();
            let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
            let up = Vec3::new(0.0, 1.0, 0.0);

            let speed = if input.is_key_pressed(crate::winit::keyboard::KeyCode::ShiftLeft as u32) {
                state.camera_speed * 3.0
            } else {
                state.camera_speed
            };

            let mut cam_move = Vec3::ZERO;
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyW as u32) { cam_move += forward; }
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyS as u32) { cam_move -= forward; }
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyD as u32) { cam_move += right; }
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyA as u32) { cam_move -= right; }
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyE as u32) { cam_move += up; }
            if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyQ as u32) { cam_move -= up; }

            if cam_move.length_squared() > 0.0 {
                state.camera_pos += cam_move.normalize() * speed * dt;
            }

            if let Some(mut q) = world.query::<(
                crate::core::query::Mut<Transform>,
                crate::core::query::Mut<Camera>,
            )>() {
                let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + FRAC_PI_2);
                let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
                let rot = yaw_rot * pitch_rot;

                for (_, (mut trans, mut cam)) in q.iter_mut() {
                    trans.position = state.camera_pos;
                    trans.rotation = rot;
                    cam.yaw = state.camera_yaw;
                    cam.pitch = state.camera_pitch;
                }
            }

            let mut physics_dt = dt.min(0.1);
            while physics_dt > 0.0 {
                let step = physics_dt.min(0.016);
                systems::cpu_physics_step_system(world, step);
                physics_dt -= step;
            }

            use crate::core::system::System;
            let mut transform_sync = systems::transform::TransformSyncSystem;
            let mut transform_propagate = systems::transform::TransformPropagateSystem;
            transform_sync.run(world, dt);
            transform_propagate.run(world, dt);
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            // Basit sahnelerde varsayılan olarak gelen GPU compute sistemlerini ve reflection'ları kapatıyoruz
            // Böylece sadece bizim eklediğimiz küp ve ışık temiz bir şekilde render edilecek.
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None; // Arkadaki istenmeyen yansımaları (Screen Space Reflections) kapatır
            renderer.ssgi = None; // SSGI kapatır
            
            systems::default_render_pass(world, encoder, view, renderer);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_settings_default() {
        let settings = CameraSettings::default();
        assert_eq!(settings.speed, 15.0);
        assert_eq!(settings.pitch, 0.0);
        assert_eq!(settings.yaw, 0.0);
        assert_eq!(settings.pos, Vec3::new(0.0, 2.0, 5.0));
        assert_eq!(settings.exposure, 1.0);
        assert_eq!(settings.bloom_intensity, 0.05);
    }

    #[test]
    fn test_lighting_settings_default() {
        let settings = LightingSettings::default();
        assert_eq!(settings.preset, 0);
        assert_eq!(settings.preset_2, 1);
        assert_eq!(settings.blend_t, 0.0);
        assert!(!settings.auto_cycle);
        assert_eq!(settings.rotation_speed, 1.0);
        assert_eq!(settings.direct_intensity, 4.0);
    }

    #[test]
    fn test_camera_state_transitions() {
        let mut state = CameraState::default();
        assert_eq!(state, CameraState::Manual);
        
        state = CameraState::Orbiting;
        assert_eq!(state, CameraState::Orbiting);
        
        state = CameraState::Stationary;
        assert_eq!(state, CameraState::Stationary);
    }

    #[test]
    fn test_editor_state_transitions() {
        let mut state = EditorState::default();
        assert_eq!(state, EditorState::PlayMode);
        
        state = EditorState::EditMode;
        assert_eq!(state, EditorState::EditMode);
        
        state = EditorState::Paused;
        assert_eq!(state, EditorState::Paused);
    }
}
