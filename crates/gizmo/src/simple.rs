use crate::app::App;
use crate::core::input::Input;
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
/// The state a `SimpleApp` scene carries between frames: where the fly camera is and how fast
/// it moves.
pub struct SimpleSceneState {
    /// Fly speed in metres per second.
    pub camera_speed: f32,
    /// Pitch in radians.
    pub camera_pitch: f32,
    /// Yaw in radians.
    pub camera_yaw: f32,
    /// Camera position in world space.
    pub camera_pos: Vec3,
}

impl Default for SimpleSceneState {
    /// Where [`with_simple_scene`](SimpleAppExt::with_simple_scene) starts the fly camera:
    /// 2 m up, 5 m back, looking down −Z, at 15 m/s.
    fn default() -> Self {
        Self { camera_speed: 15.0, camera_pitch: 0.0, camera_yaw: 0.0, camera_pos: Vec3::new(0.0, 2.0, 5.0) }
    }
}

impl SimpleSceneState {
    /// Advances the built-in fly camera by one frame: look, then move.
    ///
    /// Extracted from the update closure so it can be tested at all — a closure inside
    /// `set_update` needs a window, a renderer and an event loop to reach, which is why the
    /// engine's own camera was the last mover in the workspace nothing had ever asserted on.
    ///
    /// **Looking** is the right mouse button dragging, in pixels; **moving** is
    /// [`Input::move_axis`] (WASD *and* the left stick) horizontally, with Q/E for the vertical
    /// axis the stick has no counterpart for. The total is clamped to unit length rather than
    /// normalised: for keys the two are identical, and a normalise would push a half-tilted
    /// stick back up to full speed, which is the one thing a stick expresses and a key cannot.
    pub fn fly_step(&mut self, input: &Input, dt: f32) {
        if input.is_mouse_button_pressed(1) {
            let delta = input.mouse_delta();
            self.camera_yaw -= delta.0 * 0.005;
            self.camera_pitch -= delta.1 * 0.005;
            self.camera_pitch = self.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
        }

        let fx = self.camera_yaw.cos() * self.camera_pitch.cos();
        let fy = self.camera_pitch.sin();
        let fz = self.camera_yaw.sin() * self.camera_pitch.cos();
        let forward = Vec3::new(fx, fy, fz).normalize();
        let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
        let up = Vec3::new(0.0, 1.0, 0.0);

        let speed = if input.is_key_pressed(crate::winit::keyboard::KeyCode::ShiftLeft as u32) {
            self.camera_speed * 3.0
        } else {
            self.camera_speed
        };

        let (mx, my) = input.move_axis();
        let mut cam_move = right * mx + forward * my;
        if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyE as u32) {
            cam_move += up;
        }
        if input.is_key_pressed(crate::winit::keyboard::KeyCode::KeyQ as u32) {
            cam_move -= up;
        }

        let len = cam_move.length();
        if len > 1.0 {
            cam_move /= len;
        }
        self.camera_pos += cam_move * speed * dt;
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
/// Serialisable camera settings — the same numbers as [`SimpleSceneState`] plus the two
/// post-process knobs, in a form a scene or a settings file can hold.
pub struct CameraSettings {
    /// Fly speed in metres per second.
    pub speed: f32,
    /// Pitch in radians.
    pub pitch: f32,
    /// Yaw in radians.
    pub yaw: f32,
    /// Camera position in world space.
    pub pos: Vec3,
    /// Exposure applied at tone-mapping; 1.0 leaves the HDR scene as rendered.
    pub exposure: f32,
    /// How much bloom is mixed back in, 0 for none.
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
/// The lighting a `SimpleApp` scene runs with: two presets and the blend between them, which is
/// what makes a time-of-day sweep a single number.
pub struct LightingSettings {
    /// The preset being blended *from*.
    pub preset: u32,
    /// The preset being blended *to*.
    pub preset_2: u32,
    /// Where between the two the scene currently sits, 0 → `preset`, 1 → `preset_2`.
    pub blend_t: f32,
    /// Whether `blend_t` advances on its own.
    pub auto_cycle: bool,
    /// How fast the sun rotates, in radians per second.
    pub rotation_speed: f32,
    /// The directional light's intensity multiplier.
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
#[non_exhaustive]
/// What the demo camera is doing.
pub enum CameraState {
    /// Circling the scene on its own.
    Orbiting,
    /// Parked; neither input nor the orbit moves it.
    Stationary,
    /// Driven by the player's input — the default.
    #[default]
    Manual,
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[derive(Default)]
#[non_exhaustive]
/// Whether the scene is being played or edited.
pub enum EditorState {
    /// Running: physics steps and scripts execute.
    #[default]
    PlayMode,
    /// Being edited: the simulation is stopped and entities can be moved freely.
    EditMode,
    /// Play mode, held still — the world keeps its state and nothing advances.
    Paused,
}



/// The handle a `with_simple_scene` setup closure builds its scene through.
///
/// Every `spawn_*` on it creates the mesh, the material and the physics body together, so a demo
/// scene is a list of calls rather than a bundle assembly.
pub struct SceneBuilder<'a> {
    /// The world being populated.
    pub world: &'a mut World,
    /// The live renderer, for creating meshes and textures — and, since it is `&mut`, for
    /// registering things that live on it.
    ///
    /// Mutable because a scene may need to *add* to the renderer, not only read from it:
    /// [`Renderer::custom_materials`](gizmo_renderer::Renderer::custom_materials) is registered
    /// here, and a `&Renderer` made the registry reachable only from `set_render`, i.e. from
    /// inside the frame loop, one frame after the entities that use it were spawned.
    pub renderer: &'a mut Renderer,
    /// The asset manager the created meshes and textures are registered with.
    pub asset_manager: &'a mut AssetManager,
}

impl<'a> SceneBuilder<'a> {
    /// Spawns a coloured cube with a matching box collider.
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

    /// Spawns a coloured sphere with a matching sphere collider.
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

    /// Spawns a cube with the default checker texture, and returns its entity.
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

    /// Spawns a textured sphere, and returns its entity.
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

    /// Spawns a textured cylinder, and returns its entity.
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
        // `Collider::cylinder` landed 2026-08-17 and this call site was not updated with it — a
        // cylinder mesh wearing a sphere rolls in every direction and rests a radius too high.
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::cylinder(radius, height / 2.0)));
        ent
    }

    /// Spawns a textured cone, and returns its entity.
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
        // The real cone, not a sphere. `create_cone`'s mesh has its base at `-height/2` and its
        // apex at `+height/2`, which is exactly `ConeShape`'s convention, so the collider is the
        // silhouette rather than something near it.
        self.world.add_bundle(ent, RigidBodyBundle::dynamic(10.0).with_collider(Collider::cone(radius, height / 2.0)));
        ent
    }

    /// Spawns a textured torus, and returns its entity.
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

    /// Spawns a textured capsule, and returns its entity.
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

    /// Spawns a textured tetrahedron, and returns its entity.
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

    /// Spawns a textured truncated cone, and returns its entity.
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

    /// Extrudes a convex 2-D outline into a textured solid, and returns its entity.
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

    /// Extrudes a ring — an outer outline with an inner one cut out — into a textured solid.
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

    /// Spawns a static ground disc of `radius` metres, with a collider to match.
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

    /// Spawns a point light at `position`, with the demo defaults.
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
    
    /// Spawns the scene's camera at `pos` looking at `look_at`, and records it in `state`.
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

/// Everything the simple scene does every frame, as a callable function.
///
/// [`SimpleAppExt::with_simple_scene`] installs this as its update hook. It is public because
/// [`App::set_update`](crate::app::App::set_update) **replaces** that hook rather than chaining
/// onto it: a game that needs its own exclusive `&mut World` pass — which a scheduled system can
/// never have — would otherwise silently lose all four jobs below. Call this at the top of your
/// own hook to keep them.
///
/// [`App::add_update_hook`](crate::app::App::add_update_hook) is the direct answer to the same
/// problem and needs no call to this function; what the loss actually costs, job by job, is
/// measured under `set_update`'s own docs and in the `update_hooks` demo.
///
/// ```no_run
/// # use gizmo::prelude::*;
/// # use gizmo::simple::{simple_scene_update, SimpleAppExt, SimpleSceneState};
/// # fn my_exclusive_work(_: &mut gizmo::core::World) {}
/// # let app = App::<SimpleSceneState>::new("x", 1, 1).with_simple_scene(|_, _| {});
/// app.set_update(|world, state, dt, input| {
///     simple_scene_update(world, state, dt, input);
///     my_exclusive_work(world);
/// })
/// # ;
/// ```
///
/// The four jobs, in order:
///
///   1. turn mouse/WASD into a camera pose ([`SimpleSceneState::fly_step`]);
///   2. write that pose onto **the scene's camera** — see below;
///   3. step the CPU physics in ≤16 ms slices;
///   4. run `TransformSyncSystem` then `TransformPropagateSystem`.
///
/// # Which camera step 2 writes to
///
/// Exactly the one the frame renders from, resolved through
/// [`active_camera`](crate::renderer::components::active_camera) — the camera flagged `primary`,
/// falling back to the first when none is.
///
/// It did not always. Until 2026-08-23 this loop wrote the pose to **every** entity matching
/// `(Transform, Camera)` with no filter at all, so a scene holding a second camera had it stomped
/// once per frame and could not keep its own pose. Measured while building
/// `demo/src/bin/render_to_texture.rs`: a camera spawned at `(0, 7.0, 0.2)` with pitch −1.15 read
/// back as `yaw −1.57 pitch −0.21 T(0, 1.4, 6.5)` at draw time — the *first* camera's pose. Using
/// the same rule as the renderer is what makes "the camera the player flies" and "the camera the
/// frame is drawn from" the same sentence.
pub fn simple_scene_update(
    world: &mut crate::core::World,
    state: &mut SimpleSceneState,
    dt: f32,
    input: &Input,
) {
    state.fly_step(input, dt);

    // The renderer's own rule, not a second copy of it: whichever camera `active_camera` names is
    // the one the fly controls.
    let target = crate::renderer::components::active_camera(world);
    if let (Some(target), Some(mut q)) = (
        target,
        world.query_mut::<(
            crate::core::query::Mut<Transform>,
            crate::core::query::Mut<Camera>,
        )>(),
    ) {
        let rot = Quat::from_rotation_y(-state.camera_yaw + FRAC_PI_2)
            * Quat::from_rotation_x(state.camera_pitch);

        for (id, (mut trans, mut cam)) in q.iter_mut() {
            if id != target {
                continue;
            }
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

    sync_transforms(world);
}

/// Sets the content of the [`Text`](crate::renderer::components::Text) on entity `id` —
/// **only if it differs**, so a HUD written every frame is laid out again only when it changes.
/// Returns whether it changed. Nothing happens to an entity without a `Text`.
///
/// Sixteen of the demos carried this same function, which is the reason it is here.
pub fn set_text(world: &mut World, id: u32, content: &str) -> bool {
    let mut texts = world.borrow_mut::<crate::renderer::components::Text>();
    match texts.get_mut(id) {
        Some(mut t) if t.content != content => {
            t.content = content.to_string();
            true
        }
        _ => false,
    }
}

/// Copies physics poses into `Transform`s and propagates them down the hierarchy into
/// `GlobalTransform` — the last job of [`simple_scene_update`], on its own.
///
/// A hook that replaces `simple_scene_update` and steps physics itself (with [`FixedPhysics`],
/// say) calls this once a frame after the step, or every sprite and mesh is drawn where it stood
/// a frame ago — or, for a body spawned this frame, at the origin.
pub fn sync_transforms(world: &mut crate::core::World) {
    use crate::core::system::System;
    // Neither system reads its `dt`.
    systems::transform::TransformSyncSystem.run(world, 0.0);
    systems::transform::TransformPropagateSystem.run(world, 0.0);
}

/// A fixed-step physics driver for a game that runs its own update hook — one that owns its
/// camera, and so cannot use [`simple_scene_update`], which also flies the camera with WASD and
/// the mouse.
///
/// Every step advances the simulation by exactly [`step`](Self::step) seconds whatever the frame
/// rate, so the same shot lands in the same place on every machine. `simple_scene_update` does
/// **not** promise that: it slices the frame delta into ≤16 ms pieces, so the steps it takes
/// depend on the frame rate. Until this existed, a game that needed the fixed step wrote the
/// accumulator itself (`kule_yikici`'s `FIXED_DT`/`MAX_STEPS`, `docs/CAPABILITY_GAPS.md` §C9).
///
/// The game owns the value — in its state or a resource — rather than the world holding one,
/// because the windowed runtime already feeds a [`PhysicsTime`](crate::core::time::PhysicsTime)
/// resource for its fixed schedule, and two drivers spending one accumulator would each see half
/// the time.
///
/// ```no_run
/// # use gizmo::prelude::*;
/// # use gizmo::simple::{FixedPhysics, SimpleAppExt, SimpleSceneState, sync_transforms};
/// # let app = App::<SimpleSceneState>::new("x", 1, 1).with_simple_scene(|_, _| {});
/// let mut physics = FixedPhysics::new(120);
/// app.set_update(move |world, _state, dt, _input| {
///     // …game logic that sets velocities…
///     physics.advance(world, dt);
///     sync_transforms(world);
/// })
/// # ;
/// ```
///
/// # When a frame takes too long
///
/// At most [`max_steps`](Self::with_max_steps) steps run per call (8 by default). When a frame
/// hits that ceiling the rest of the debt is **dropped**, not carried: a stall — a breakpoint, a
/// window drag — then costs one slow frame instead of a run of them, each slower than the last
/// as it tries to catch up. The game falls behind wall-clock instead, which for a game is the
/// right failure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedPhysics {
    step: f32,
    max_steps: u32,
    accumulator: f32,
}

impl FixedPhysics {
    /// The per-call step ceiling [`new`](Self::new) starts with.
    pub const DEFAULT_MAX_STEPS: u32 = 8;

    /// A driver that steps `hz` times per simulated second. `hz` of 0 is taken as 1.
    #[must_use]
    pub fn new(hz: u32) -> Self {
        Self::with_step(1.0 / hz.max(1) as f32)
    }

    /// A driver with an explicit step length in seconds — for a step that is not `1/hz` of an
    /// integer, or one already stored as seconds. Non-positive or non-finite values become 1/60.
    #[must_use]
    pub fn with_step(step: f32) -> Self {
        let step = if step.is_finite() && step > 0.0 { step } else { 1.0 / 60.0 };
        Self { step, max_steps: Self::DEFAULT_MAX_STEPS, accumulator: 0.0 }
    }

    /// Replaces the per-call step ceiling — see the type's docs. 0 is taken as 1.
    #[must_use]
    pub fn with_max_steps(mut self, max_steps: u32) -> Self {
        self.max_steps = max_steps.max(1);
        self
    }

    /// Spends `dt` seconds of frame time in whole steps of physics, and returns how many ran.
    ///
    /// Time short of a full step is kept for the next call; a negative or non-finite `dt` adds
    /// nothing. This steps physics only — call [`sync_transforms`] afterwards to make the result
    /// visible.
    ///
    /// Each step runs the gameplay controllers first, in the order `GameplayPhysicsPlugin` runs
    /// them — vehicles, characters, the fight clock, hit detection — and then the rigid step
    /// they feed. They did not run here at all, so a car on this driver sat still with its
    /// throttle down. Do not also add `GameplayPhysicsPlugin`: that is a second driver of the same
    /// controllers, as `PhysicsPlugin` is of the rigid step.
    pub fn advance(&mut self, world: &mut crate::core::World, dt: f32) -> u32 {
        let steps = self.plan(dt);
        for _ in 0..steps {
            #[cfg(feature = "physics-dynamics")]
            {
                gizmo_physics_dynamics::vehicle_controller_system(world, self.step);
                gizmo_physics_dynamics::character_controller_system(world, self.step);
                gizmo_physics_dynamics::fighter_frame_system(world, self.step);
                gizmo_physics_dynamics::hit_detection_system(world, self.step);
            }
            systems::cpu_physics_step_system(world, self.step);
        }
        steps
    }

    /// The bookkeeping half of [`advance`](Self::advance), without a world: adds `dt`, takes the
    /// whole steps out, applies the ceiling.
    fn plan(&mut self, dt: f32) -> u32 {
        if dt.is_finite() && dt > 0.0 {
            self.accumulator += dt;
        }
        let mut steps = 0;
        while self.accumulator >= self.step && steps < self.max_steps {
            self.accumulator -= self.step;
            steps += 1;
        }
        if steps == self.max_steps {
            self.accumulator = 0.0;
        }
        steps
    }

    /// The step length, in seconds.
    #[must_use]
    pub fn step(&self) -> f32 {
        self.step
    }

    /// How far the simulation is into the next step, `0.0..1.0` — the coefficient for
    /// `lerp(previous_pose, current_pose, alpha)` if the game keeps previous poses to blend.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.step).clamp(0.0, 1.0)
    }

    /// Forgets the time owed — for a pause, or a level restart, where the time that passed
    /// should not be simulated afterwards.
    pub fn reset(&mut self) {
        self.accumulator = 0.0;
    }
}

impl Default for FixedPhysics {
    /// 60 steps per second.
    fn default() -> Self {
        Self::new(60)
    }
}

/// Plays a `SimpleApp` game's own update hook frame by frame, with no window, GPU or event loop —
/// what a game's headless `--test` mode runs on.
///
/// Build the game's scene into a plain [`World`] (meshes and materials need no GPU to exist as
/// components; [`HeadlessFrames::font`] gives HUD text a real font), hand the world over, and
/// call [`frame`](Self::frame) with the same `update` function the game gives
/// [`replace_update`](crate::app::App::replace_update):
///
/// ```no_run
/// # use gizmo::prelude::*;
/// # use gizmo::core::input::Input;
/// # use gizmo::simple::{HeadlessFrames, SimpleSceneState};
/// # struct Game { won: bool }
/// # fn setup(world: &mut World, _font: gizmo::renderer::text::FontId) -> Game { Game { won: false } }
/// # fn update(world: &mut World, _s: &mut SimpleSceneState, _dt: f32, _input: &Input) {}
/// let mut world = World::new();
/// let game = setup(&mut world, HeadlessFrames::font());
/// world.insert_resource(game);
/// let mut run = HeadlessFrames::new(world);
/// while run.elapsed < 60.0 && !run.world.get_resource::<Game>().is_some_and(|g| g.won) {
///     run.frame(1.0 / 60.0, update);
/// }
/// ```
///
/// Each frame does what the windowed loop does around the hook: the hook runs with the frame's
/// `dt` and an [`Input`] in which nothing is held, then the deferred commands it (and the systems
/// it drove) queued are applied. That last part is the reason this exists rather than a loop in
/// every game: the first headless test written that way left it out, and a building whose blocks
/// shattered on screen reported none — they were latched broken and never despawned, because
/// fracture despawns through the command queue (2026-11-04, `vinc`).
///
/// What it does **not** do: run plugins' schedules, fire the event double-buffer swap, load
/// assets or draw. A game driven through `FixedPhysics` and its own update needs none of them.
#[non_exhaustive]
pub struct HeadlessFrames {
    /// The game's world. Read its resources between frames to see how the game is going.
    pub world: World,
    /// The scene state handed to the hook, as `with_simple_scene` would start it.
    pub state: SimpleSceneState,
    /// The input handed to the hook: nothing pressed, unless the caller writes into it.
    pub input: Input,
    /// Frames played.
    pub frames: u64,
    /// Seconds played: the sum of every `dt` given to [`frame`](Self::frame).
    pub elapsed: f32,
}

impl HeadlessFrames {
    /// Takes over a world that already holds the game's scene and resources.
    #[must_use]
    pub fn new(world: World) -> Self {
        Self { world, state: SimpleSceneState::default(), input: Input::default(), frames: 0, elapsed: 0.0 }
    }

    /// Plays one frame of `dt` seconds through `update`, then applies the deferred commands.
    pub fn frame(&mut self, dt: f32, update: impl FnOnce(&mut World, &mut SimpleSceneState, f32, &Input)) {
        update(&mut self.world, &mut self.state, dt, &self.input);
        self.world.apply_commands();
        self.frames += 1;
        self.elapsed += dt;
    }

    /// A font for HUD text, loaded without a renderer: the engine's built-in synthetic face.
    /// Text spawned with it is a real `Text` component; it is simply never drawn.
    #[must_use]
    pub fn font() -> crate::renderer::text::FontId {
        crate::renderer::text::FontLibrary::new()
            .load(crate::renderer::text::synthetic::synthetic_face())
            .expect("the built-in synthetic face always parses")
    }
}

/// Adds [`with_simple_scene`](SimpleAppExt::with_simple_scene) to an `App<SimpleSceneState>`.
pub trait SimpleAppExt {
    /// Installs a setup closure that builds the scene through a [`SceneBuilder`], and wires the
    /// fly camera, the physics world and the default lighting around it.
    fn with_simple_scene<F>(self, setup_fn: F) -> Self
    where
        F: FnOnce(&mut SceneBuilder, &mut SimpleSceneState) + 'static;
}

/// What [`SimpleAppExt::with_simple_scene`] adds to the world once the game's setup has run:
/// the asset manager, and — **unless the setup inserted its own** — a `PhysicsWorld` with Earth
/// gravity and an empty [`Gizmos`](crate::renderer::Gizmos) for the frame's debug lines.
///
/// Without `Gizmos` a game's `get_resource_mut::<Gizmos>()` found nothing and its
/// `draw_line` calls went nowhere, with no word said: the crane's rope in `vinc` was simply not
/// there (`docs/CAPABILITY_GAPS.md` §C16q). Nothing draws into it on its own, so an empty one
/// changes no picture.
///
/// The physics world used to be inserted unconditionally, after the setup, so a game that set
/// its own gravity there had it silently replaced — found writing `lobut`
/// (`docs/CAPABILITY_GAPS.md` §C11).
fn insert_scene_defaults(world: &mut crate::core::World, asset_manager: AssetManager) {
    if world.get_resource::<PhysicsWorld>().is_none() {
        world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    }
    if world.get_resource::<crate::renderer::Gizmos>().is_none() {
        world.insert_resource(crate::renderer::Gizmos::default());
    }
    world.insert_resource(asset_manager);
}

impl SimpleAppExt for App<SimpleSceneState> {
    fn with_simple_scene<F>(self, setup_fn: F) -> Self
    where
        F: FnOnce(&mut SceneBuilder, &mut SimpleSceneState) + 'static,
    {
        self.set_setup(move |world, renderer| {
            let mut asset_manager = AssetManager::new();
            // The simple scene's defaults, set ONCE, before the game's setup — so a game that
            // wants one of them back turns it on there. They used to be forced off in the render
            // closure every frame, which also switched off the GPU particle system and so made
            // `ParticleEmitter` do nothing in every `SimpleApp` game; the particle system is now
            // free when idle and stays on.
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let mut state = SimpleSceneState::default();

            let mut builder = SceneBuilder {
                world,
                renderer,
                asset_manager: &mut asset_manager,
            };

            setup_fn(&mut builder, &mut state);

            insert_scene_defaults(world, asset_manager);
            state
        })
        .set_update(simple_scene_update)
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            systems::default_render_pass(world, encoder, view, renderer);
        })
    }
}

#[cfg(test)]
mod fly_camera_tests {
    //! The engine's own fly camera — the one every `SimpleApp` gets, and the one nothing had
    //! ever asserted on, because it lived inside an `set_update` closure that needs a window to
    //! reach. [`SimpleSceneState::fly_step`] is that closure's body, and this is what it does.

    use super::*;
    use crate::core::input::{GamepadAxis, GamepadId, MoveKeys};

    fn state() -> SimpleSceneState {
        SimpleSceneState {
            camera_speed: 10.0,
            camera_pitch: 0.0,
            camera_yaw: 0.0,
            camera_pos: Vec3::ZERO,
        }
    }

    /// The defect this camera did NOT have — pinned so it cannot acquire it. Four demos in this
    /// repository did, and each of them was one `if` away from this shape.
    #[test]
    fn a_diagonal_is_not_faster_than_a_straight_push() {
        let mut straight = state();
        let mut input = Input::new();
        input.on_key_pressed(MoveKeys::WASD.forward);
        straight.fly_step(&input, 1.0);

        let mut diagonal = state();
        input.on_key_pressed(MoveKeys::WASD.right);
        diagonal.fly_step(&input, 1.0);

        let (a, b) = (straight.camera_pos.length(), diagonal.camera_pos.length());
        assert!((a - 10.0).abs() < 1e-4, "one key should travel speed × dt, got {a}");
        assert!(
            (b - a).abs() < 1e-4,
            "the diagonal travelled {b} against {a} — the 41 % bug"
        );
    }

    /// What a pad buys: the amount, which a key cannot express. Half a tilt is a walk.
    #[test]
    fn the_left_stick_flies_the_camera_and_carries_its_amount() {
        let mut input = Input::new();
        let id = GamepadId::new(0);
        input.on_gamepad_connected(id, "test pad");
        input.on_gamepad_axis(id, GamepadAxis::LeftStickY, 1.0);

        let mut full = state();
        full.fly_step(&input, 1.0);
        assert!(
            full.camera_pos.length() > 9.0,
            "a full stick did not fly the camera: {:?}",
            full.camera_pos
        );

        // Past the 0.15 deadzone, well short of the rim.
        input.on_gamepad_axis(id, GamepadAxis::LeftStickY, 0.5);
        let mut half = state();
        half.fly_step(&input, 1.0);
        assert!(
            half.camera_pos.length() < full.camera_pos.length() * 0.8,
            "half a tilt must walk, not run: {} against {}",
            half.camera_pos.length(),
            full.camera_pos.length()
        );
        assert!(half.camera_pos.length() > 0.1, "…but it must move at all");
    }

    /// The vertical axis has no stick counterpart, so it stays on Q/E — and adding it must not
    /// let the total exceed full speed, nor re-normalise a half-tilted stick back up to it.
    #[test]
    fn the_vertical_axis_neither_exceeds_nor_inflates_the_speed() {
        let mut input = Input::new();
        input.on_key_pressed(MoveKeys::WASD.forward);
        input.on_key_pressed(crate::winit::keyboard::KeyCode::KeyE as u32);
        let mut keys = state();
        keys.fly_step(&input, 1.0);
        assert!(
            (keys.camera_pos.length() - 10.0).abs() < 1e-4,
            "forward + up should still be one speed, got {}",
            keys.camera_pos.length()
        );

        let mut input = Input::new();
        let id = GamepadId::new(0);
        input.on_gamepad_connected(id, "test pad");
        input.on_gamepad_axis(id, GamepadAxis::LeftStickY, 0.5);
        let mut stick = state();
        stick.fly_step(&input, 1.0);
        assert!(
            stick.camera_pos.length() < 9.0,
            "a half-tilted stick was normalised back up to full speed: {}",
            stick.camera_pos.length()
        );
    }

    /// The fly camera writes to the camera the frame renders from, and to no other.
    ///
    /// Before 2026-08-23 this loop had no filter, so a scene holding a second camera had its pose
    /// overwritten once per frame. Reverting the `id != target` guard turns this test red.
    #[test]
    fn the_fly_camera_writes_only_to_the_rendered_camera() {
        use crate::core::World;
        use crate::renderer::components::Camera;

        let mut world = World::new();

        // The scene's camera: marked primary, so `active_camera` names it.
        let primary = world.spawn_bundle((
            Transform::new(Vec3::ZERO),
            Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true),
        ));
        // A second camera with a pose of its own that nothing should touch.
        let other_pose = Vec3::new(0.0, 7.0, 0.2);
        let other = world.spawn_bundle((
            Transform::new(other_pose),
            Camera::new(1.0, 0.1, 100.0, 0.0, -1.15, false),
        ));

        let mut state = SimpleSceneState {
            camera_speed: 15.0,
            camera_pitch: 0.25,
            camera_yaw: 0.5,
            camera_pos: Vec3::new(3.0, 4.0, 5.0),
        };
        simple_scene_update(&mut world, &mut state, 1.0 / 60.0, &Input::new());

        let read = |entity: crate::core::Entity| {
            let q = world
                .query::<(&Transform, &Camera)>()
                .expect("query buildable");
            q.iter()
                .find(|(id, _)| *id == entity.id())
                .map(|(_, (t, c))| (t.position, c.pitch))
                .expect("camera still present")
        };

        let (primary_pos, primary_pitch) = read(primary);
        assert_eq!(
            primary_pos, state.camera_pos,
            "the rendered camera must follow the fly state"
        );
        assert!(
            (primary_pitch - state.camera_pitch).abs() < 1e-6,
            "the rendered camera must take the fly pitch, got {primary_pitch}"
        );

        let (other_pos, other_pitch) = read(other);
        assert_eq!(
            other_pos, other_pose,
            "a second camera must keep its own position, got {other_pos:?}"
        );
        assert!(
            (other_pitch - (-1.15)).abs() < 1e-6,
            "a second camera must keep its own pitch, got {other_pitch}"
        );
    }

    #[test]
    fn nothing_held_does_not_move_the_camera() {
        let mut s = state();
        s.fly_step(&Input::new(), 1.0);
        assert_eq!(s.camera_pos, Vec3::ZERO);
    }

    #[test]
    fn the_right_mouse_button_is_what_looks_around() {
        let mut input = Input::new();
        input.on_mouse_delta(100.0, 0.0);

        let mut ignored = state();
        ignored.fly_step(&input, 1.0);
        assert_eq!(ignored.camera_yaw, 0.0, "a drag with no button must not turn the camera");

        input.on_mouse_button_pressed(1);
        let mut looked = state();
        looked.fly_step(&input, 1.0);
        assert!(looked.camera_yaw != 0.0, "right-drag must turn the camera");
    }

    /// Pitch is clamped just short of vertical: at exactly ±90° the forward/right basis is
    /// degenerate and `right` comes out as a zero vector, which loses the camera's heading.
    #[test]
    fn pitch_cannot_reach_straight_up() {
        let mut input = Input::new();
        input.on_mouse_button_pressed(1);
        input.on_mouse_delta(0.0, -10_000.0);
        let mut s = state();
        s.fly_step(&input, 1.0);
        assert!(s.camera_pitch < PI / 2.0, "pitch reached {}", s.camera_pitch);
        assert!(s.camera_pitch > PI / 2.0 - 0.2);
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

#[cfg(test)]
mod fixed_physics_tests {
    use super::*;

    /// The step count is fixed by the time spent, not by how it was split into frames: 1 s at
    /// 120 Hz is 120 steps whether it arrives as 60 frames, 144 frames or 37 uneven ones.
    #[test]
    fn the_same_time_takes_the_same_steps_at_any_frame_rate() {
        for frames in [60u32, 144, 240] {
            let mut p = FixedPhysics::new(120);
            let total: u32 = (0..frames).map(|_| p.plan(1.0 / frames as f32)).sum();
            assert!((119..=120).contains(&total), "{frames} fps: {total} steps");
        }
        // Uneven frames that add up to one second.
        let mut p = FixedPhysics::new(100);
        let pattern: [f32; 5] = [0.004, 0.031, 0.017, 0.009, 0.039];
        let mut t = 0.0;
        let mut total = 0;
        while t < 0.999 {
            let dt = pattern[total as usize % pattern.len()].min(1.0 - t);
            t += dt;
            total += p.plan(dt);
        }
        assert!((99..=100).contains(&total), "{total}");
    }

    /// A stall runs the ceiling and then forgets the rest instead of owing it.
    #[test]
    fn a_stall_runs_the_ceiling_and_drops_the_rest() {
        let mut p = FixedPhysics::new(60).with_max_steps(4);
        assert_eq!(p.plan(10.0), 4);
        assert_eq!(p.alpha(), 0.0, "the debt was dropped");
        assert_eq!(p.plan(1.0 / 60.0), 1, "the next ordinary frame is ordinary");
    }

    #[test]
    fn short_frames_bank_their_time_and_bad_input_adds_none() {
        let mut p = FixedPhysics::new(10);
        assert_eq!(p.plan(0.06), 0);
        assert!((p.alpha() - 0.6).abs() < 1e-5);
        assert_eq!(p.plan(f32::NAN), 0);
        assert_eq!(p.plan(-1.0), 0);
        assert_eq!(p.plan(0.05), 1);
        p.reset();
        assert_eq!(p.alpha(), 0.0);
        assert_eq!(FixedPhysics::with_step(0.0).step(), 1.0 / 60.0);
        assert_eq!(FixedPhysics::new(0).step(), 1.0);
    }

    /// The render closure draws and does nothing else: renderer settings forced there every frame
    /// overrode the game's own (and switched particles off for every game). Source guard — the
    /// closure needs a window to run.
    #[test]
    fn the_simple_scene_does_not_override_the_renderer_every_frame() {
        let src = include_str!("simple.rs");
        let render = src.split(".set_render(").nth(1).and_then(|r| r.split("})").next()).unwrap_or("");
        assert!(!render.contains("= None"), "the render closure switches something off every frame again:\n{render}");
        assert!(!src.contains(concat!("renderer.gpu_particles", " = None")), "particles are switched off for every simple scene again");
    }

    /// A `PhysicsWorld` the game's setup inserted survives; a world without one gets Earth's.
    #[test]
    fn the_setups_own_physics_world_is_kept() {
        let mut w = crate::core::World::new();
        w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -1.62, 0.0)));
        super::insert_scene_defaults(&mut w, AssetManager::new());
        let g = w.get_resource::<PhysicsWorld>().map(|p| p.integrator.gravity);
        assert_eq!(g, Some(Vec3::new(0.0, -1.62, 0.0)), "the setup's Moon gravity was replaced");

        let mut empty = crate::core::World::new();
        super::insert_scene_defaults(&mut empty, AssetManager::new());
        let g = empty.get_resource::<PhysicsWorld>().map(|p| p.integrator.gravity);
        assert_eq!(g, Some(Vec3::new(0.0, -9.81, 0.0)));
        assert!(empty.get_resource::<AssetManager>().is_some());
    }

    /// `set_text` writes only a different content, says whether it did, and ignores an entity
    /// without a `Text`.
    #[test]
    fn set_text_writes_only_a_change() {
        use crate::renderer::components::{Text, TextAnchor};
        let mut w = crate::core::World::new();
        let font = crate::renderer::text::FontLibrary::default()
            .load(crate::renderer::text::synthetic::synthetic_face())
            .expect("the synthetic face parses");
        let e = w.spawn();
        w.add_component(e, Text::pinned("a", font, 12.0, TextAnchor::TopLeft, crate::math::Vec2::ZERO));
        assert!(super::set_text(&mut w, e.id(), "b"));
        assert!(!super::set_text(&mut w, e.id(), "b"), "the same content is not a change");
        assert_eq!(w.borrow::<Text>().get(e.id()).unwrap().content, "b");
        let bare = w.spawn();
        assert!(!super::set_text(&mut w, bare.id(), "x"));
    }

    /// A simple scene has somewhere for debug lines to go, and a setup's own `Gizmos` survives.
    #[test]
    fn a_simple_scene_has_gizmos_and_keeps_the_setups() {
        let mut w = crate::core::World::new();
        super::insert_scene_defaults(&mut w, AssetManager::new());
        let mut g = w.get_resource_mut::<crate::renderer::Gizmos>().expect("no Gizmos: draw_line goes nowhere");
        g.draw_line(Vec3::ZERO, Vec3::X, [1.0; 4]);
        assert_eq!(g.lines.len(), 2);
        drop(g);

        let mut own = crate::core::World::new();
        own.insert_resource(crate::renderer::Gizmos { depth_test: true, ..Default::default() });
        super::insert_scene_defaults(&mut own, AssetManager::new());
        assert!(own.get_resource::<crate::renderer::Gizmos>().unwrap().depth_test, "the setup's Gizmos was replaced");
    }

    /// `advance` actually moves the world: a falling body falls exactly `steps` steps' worth, and
    /// the same total time split two ways leaves it bit-identical.
    #[test]
    fn advance_steps_the_world_independently_of_the_frame_split() {
        fn world_with_ball() -> (crate::core::World, u32) {
            let mut w = crate::core::World::new();
            w.insert_resource(
                PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)),
            );
            let e = w.spawn_bundle((
                Transform::new(Vec3::new(0.0, 10.0, 0.0)),
                crate::physics::RigidBody::new(1.0, true),
                crate::physics::Velocity::default(),
                crate::physics::Collider::sphere(0.5),
            ));
            (w, e.id())
        }
        let y = |w: &crate::core::World, id: u32| w.borrow::<Transform>().get(id).unwrap().position.y;

        // Powers of two, so the frame times are exact in f32 and both splits owe the same steps.
        let (mut a, ida) = world_with_ball();
        let mut pa = FixedPhysics::new(128);
        let steps_a: u32 = (0..64).map(|_| pa.advance(&mut a, 1.0 / 64.0)).sum();

        let (mut b, idb) = world_with_ball();
        let mut pb = FixedPhysics::new(128);
        let steps_b: u32 = (0..256).map(|_| pb.advance(&mut b, 1.0 / 256.0)).sum();

        assert_eq!((steps_a, steps_b), (128, 128));
        assert!(y(&a, ida) < 9.0, "the ball fell: {}", y(&a, ida));
        assert_eq!(y(&a, ida).to_bits(), y(&b, idb).to_bits(), "same steps, same world");
    }

    /// A car driven by `FixedPhysics` moves under its throttle: the vehicle controller runs in
    /// each step, before the rigid step it feeds. It did not run at all — a game on this driver
    /// had cars that sat still with the pedal down.
    #[cfg(feature = "physics-dynamics")]
    #[test]
    fn fixed_physics_drives_a_vehicle() {
        use crate::physics::vehicle::{Axle, VehicleController, Wheel};
        use crate::physics::world::PhysicsWorld;
        use crate::physics::{Collider, RigidBody, Velocity};
        let mut w = crate::core::World::new();
        w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
        w.spawn_bundle((
            Transform::new(Vec3::new(0.0, -1.0, 0.0)),
            RigidBody::new_static(),
            Velocity::default(),
            Collider::box_collider(Vec3::new(100.0, 1.0, 100.0)),
        ));
        let collider = Collider::box_collider(Vec3::new(0.9, 0.3, 2.0));
        let mut body = RigidBody::new(900.0, true);
        body.update_inertia_from_collider(&collider);
        let mut car = VehicleController::new();
        for (x, z, axle, is_left) in [
            (-0.8, -1.4, Axle::Front, true),
            (0.8, -1.4, Axle::Front, false),
            (-0.8, 1.4, Axle::Rear, true),
            (0.8, 1.4, Axle::Rear, false),
        ] {
            car.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.0, z),
                axle_type: axle,
                is_left,
                radius: 0.35,
                suspension_rest_length: 0.4,
                suspension_max_travel: 0.3,
                ..Default::default()
            });
        }
        car.throttle_input = 1.0;
        let id = w
            .spawn_bundle((Transform::new(Vec3::new(0.0, 0.6, 0.0)), body, Velocity::default(), collider, car))
            .id();
        let mut physics = FixedPhysics::new(120);
        for _ in 0..150 {
            physics.advance(&mut w, 1.0 / 60.0);
        }
        let z = w.borrow::<Transform>().get(id).unwrap().position.z;
        assert!(z < -0.5, "the throttled car went forward (−Z): z = {z}");
    }
}

#[cfg(test)]
mod headless_frames_tests {
    use super::*;

    #[test]
    fn a_frame_runs_the_hook_with_nothing_pressed_and_applies_what_it_queued() {
        let mut world = World::new();
        let doomed = world.spawn();
        let mut run = HeadlessFrames::new(world);
        let mut seen = None;
        run.frame(0.25, |world, _state, dt, input| {
            seen = Some((dt, input.move_axis()));
            world
                .get_resource::<crate::core::commands::CommandQueue>()
                .expect("every World has a command queue")
                .push(move |w: &mut World| w.despawn(doomed));
        });
        assert_eq!(seen, Some((0.25, (0.0, 0.0))));
        assert!(!run.world.is_alive(doomed), "the despawn the hook queued was left in the queue");
        run.frame(0.5, |_, _, _, _| {});
        assert_eq!((run.frames, run.elapsed), (2, 0.75));
    }

    #[test]
    fn the_headless_font_is_a_real_font() {
        let font = HeadlessFrames::font();
        let mut world = World::new();
        let id = world
            .spawn_bundle((crate::renderer::components::Text::pinned("puan 3", font, 22.0, crate::renderer::components::TextAnchor::TopLeft, crate::math::Vec2::ZERO),))
            .id();
        assert!(set_text(&mut world, id, "puan 4"));
    }

    #[test]
    fn the_default_scene_state_is_where_with_simple_scene_starts() {
        let s = SimpleSceneState::default();
        assert_eq!((s.camera_speed, s.camera_pos), (15.0, Vec3::new(0.0, 2.0, 5.0)));
    }
}
