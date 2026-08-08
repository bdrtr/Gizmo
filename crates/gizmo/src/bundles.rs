//! Bevy-style predefined Bundle structures.
//!
//! Used to add more than one component to an entity in a single go.
//!
//! Working examples live on the bundles' own pages: `CameraBundle`, `MeshBundle`,
//! `Prefab`.

// The examples deliberately live on the items, not here: this module is gated
// `any(render, physics)` while every individual bundle needs only one half of that gate, so a
// module-level doctest naming any of them fails to compile under a single-feature build
// (`--no-default-features --features physics`). On the item, the doctest is cfg'd out with the
// item itself.

use crate::core::{Bundle, Entity, EntityName, World};
use crate::math::{Quat, Vec3, Vec4};
use gizmo_physics_core::Transform;
// Light/camera/mesh bundles are renderer components; `RigidBodyBundle` below is not, so the
// module stays available with `physics` alone.
#[cfg(feature = "render")]
use crate::renderer::components::{
    Camera, DirectionalLight, LightRole, Material, Mesh, MeshRenderer, PointLight, SpotLight,
};

// ============================================================
//  DirectionalLightBundle
// ============================================================

/// Ready-made bundle for a directional light (the sun).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(feature = "render")]
pub struct DirectionalLightBundle {
    pub rotation: Quat,
    pub color: Vec3,
    pub intensity: f32,
    pub role: LightRole,
}

#[cfg(feature = "render")]
impl Default for DirectionalLightBundle {
    fn default() -> Self {
        Self {
            rotation: Quat::from_rotation_x(-std::f32::consts::PI / 4.0),
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 3.0,
            role: LightRole::Sun,
        }
    }
}

#[cfg(feature = "render")]
impl Bundle for DirectionalLightBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(Vec3::ZERO).with_rotation(self.rotation),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            DirectionalLight::new(self.color, self.intensity, self.role),
        );
    }
}

// ============================================================
//  PointLightBundle
// ============================================================

/// Ready-made bundle for a point light.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(feature = "render")]
pub struct PointLightBundle {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
}

#[cfg(feature = "render")]
impl Default for PointLightBundle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 5.0,
            radius: 20.0,
        }
    }
}

#[cfg(feature = "render")]
impl Bundle for PointLightBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            PointLight::new(self.color, self.intensity, self.radius),
        );
    }
}

// ============================================================
//  SpotLightBundle
// ============================================================

/// Ready-made bundle for a spot light.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(feature = "render")]
pub struct SpotLightBundle {
    pub position: Vec3,
    pub rotation: Quat,
    pub color: Vec3,
    pub intensity: f32,
    pub radius: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

#[cfg(feature = "render")]
impl Default for SpotLightBundle {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            color: Vec3::new(1.0, 1.0, 1.0),
            intensity: 10.0,
            radius: 30.0,
            inner_angle: 0.4,
            outer_angle: 0.6,
        }
    }
}

#[cfg(feature = "render")]
impl Bundle for SpotLightBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position).with_rotation(self.rotation),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(
            entity,
            SpotLight::new(
                self.color,
                self.intensity,
                self.radius,
                self.inner_angle,
                self.outer_angle,
            ),
        );
    }
}

// ============================================================
//  CameraBundle
// ============================================================

/// Ready-made bundle for a camera.
///
/// ```
/// # use gizmo::prelude::*;
/// # let mut world = World::new();
/// let cam = world.spawn_bundle(CameraBundle {
///     position: Vec3::new(0.0, 3.0, 10.0),
///     fov: 60.0_f32.to_radians(),
///     ..Default::default()
/// });
/// // A single call attaches Transform + GlobalTransform + Camera together.
/// assert_eq!(
///     world.borrow::<Transform>().get_entity(cam).unwrap().position,
///     Vec3::new(0.0, 3.0, 10.0)
/// );
/// assert!(world.borrow::<Camera>().get_entity(cam).unwrap().primary);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg(feature = "render")]
pub struct CameraBundle {
    pub position: Vec3,
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub exposure: f32,
    pub primary: bool,
}

#[cfg(feature = "render")]
impl Default for CameraBundle {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 5.0, 10.0),
            fov: std::f32::consts::FRAC_PI_3,
            near: 0.1,
            far: 1500.0,
            yaw: 0.0,
            pitch: 0.0,
            exposure: 1.0,
            primary: true,
        }
    }
}

#[cfg(feature = "render")]
impl Bundle for CameraBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(entity, Transform::new(self.position));
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        let mut cam = Camera::new(
            self.fov,
            self.near,
            self.far,
            self.yaw,
            self.pitch,
            self.primary,
        );
        cam.exposure = self.exposure;
        world.add_component(entity, cam);
    }
}

// ============================================================
//  MeshBundle
// ============================================================

/// Ready-made bundle for Mesh + Material + MeshRenderer.
///
/// `mesh`/`material` are each a [`Handle`](crate::core::asset::Handle) — not the mesh/material
/// ITSELF: register it with [`Assets::add`](crate::core::asset::Assets::add) and give the
/// returned handle here.
///
/// ```
/// # use gizmo::prelude::*;
/// # let mut world = World::new();
/// # let cube: Handle<Mesh> = Handle::default();
/// # let my_material: Handle<Material> = Handle::default();
/// let player = world.spawn_bundle(
///     MeshBundle::new(cube, my_material)
///         .with_name("Oyuncu")
///         .at(Vec3::new(0.0, 5.0, 0.0))
/// );
/// assert_eq!(
///     world.borrow::<Transform>().get_entity(player).unwrap().position,
///     Vec3::new(0.0, 5.0, 0.0)
/// );
/// assert_eq!(world.borrow::<EntityName>().get_entity(player).unwrap().0, "Oyuncu");
/// ```
#[cfg(feature = "render")]
pub struct MeshBundle {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub mesh: crate::core::asset::Handle<Mesh>,
    pub material: crate::core::asset::Handle<Material>,
    pub name: Option<String>,
}

#[cfg(feature = "render")]
impl MeshBundle {
    /// Creates a new MeshBundle (mesh and material are mandatory).
    pub fn new(
        mesh: crate::core::asset::Handle<Mesh>,
        material: crate::core::asset::Handle<Material>,
    ) -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            mesh,
            material,
            name: None,
        }
    }

    /// Sets the position.
    pub fn at(mut self, position: Vec3) -> Self {
        self.position = position;
        self
    }

    /// Sets the rotation.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Sets the scale.
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Gives a name.
    pub fn with_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }
}

#[cfg(feature = "render")]
impl Bundle for MeshBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> { vec![] }
    unsafe fn write_to_archetype(self, _arch: &mut gizmo_core::archetype::Archetype, _row: usize, _tick: u32) {}
    fn apply(self, world: &mut World, entity: Entity) {
        world.add_component(
            entity,
            Transform::new(self.position)
                .with_rotation(self.rotation)
                .with_scale(self.scale),
        );
        world.add_component(entity, gizmo_physics_core::components::GlobalTransform::default());
        world.add_component(entity, self.mesh);
        world.add_component(entity, self.material);
        world.add_component(entity, MeshRenderer::new());
        if let Some(name) = self.name {
            world.add_component(entity, EntityName(name));
        }
    }
}

// ============================================================
//  RigidBodyBundle
// ============================================================

use gizmo_physics_core::{BoxShape, Collider, ColliderShape};
#[cfg(feature = "physics")]
use gizmo_physics_rigid::components::{RigidBody, Velocity};

/// Zero-overhead Bundle for creating a physics object.
/// Prevents the mistakes of forgetting to add Velocity or Collider.
#[cfg(feature = "physics")]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RigidBodyBundle {
    pub rigid_body: RigidBody,
    pub velocity: Velocity,
    pub collider: Collider,
}


#[cfg(feature = "physics")]
impl RigidBodyBundle {
    pub fn dynamic(mass: f32) -> Self {
        Self {
            rigid_body: RigidBody::new(mass, true),
            ..Default::default()
        }
    }

    pub fn static_body() -> Self {
        Self {
            rigid_body: RigidBody::new_static(),
            ..Default::default()
        }
    }

    /// Kinematic body — user-driven motion (moving platforms, scripted blades).
    /// `new_kinematic` turns CCD on by default, so fast kinematic movers get
    /// tunnelling prevention without a separate `.with_ccd()`.
    pub fn kinematic() -> Self {
        Self {
            rigid_body: RigidBody::new_kinematic(),
            ..Default::default()
        }
    }

    pub fn with_collider(mut self, collider: Collider) -> Self {
        self.collider = collider;
        self
    }

    /// Give the body an initial linear velocity (the bundle otherwise spawns at rest).
    pub fn with_velocity(mut self, linear: Vec3) -> Self {
        self.velocity = Velocity::new(linear);
        self
    }

    /// Enable Continuous Collision Detection: the body is swept against obstacles
    /// each substep so it can't tunnel through thin/other geometry at high speed.
    /// Off by default (discrete detection) — turn it on for bullets, fast balls,
    /// anything that moves more than its own thickness per frame.
    pub fn with_ccd(mut self) -> Self {
        self.rigid_body.ccd_enabled = true;
        self
    }

    /// Turns on physical air drag (½·ρ·Cd·A·v²) → a falling/flying body settles at its natural
    /// terminal velocity. `cd` is the drag coefficient (sphere ~0.47, cube ~1.05), `area` the
    /// frontal area (m²). E.g.: `RigidBodyBundle::dynamic(2.0).with_air_drag(0.47, 0.5)`.
    pub fn with_air_drag(mut self, cd: f32, area: f32) -> Self {
        self.rigid_body = self.rigid_body.with_air_drag(cd, area);
        self
    }

    /// Sets the collider's bounciness (restitution).
    /// E.g.: `RigidBodyBundle::dynamic(1.0).with_collider(Collider::sphere(0.5)).with_restitution(0.9)`.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.collider = self.collider.with_restitution(restitution);
        self
    }

    /// Sets the collider's friction (static = dynamic).
    pub fn with_friction(mut self, friction: f32) -> Self {
        self.collider = self.collider.with_friction(friction);
        self
    }

    /// Sets linear + angular damping (coarse energy loss). For realistic air drag use
    /// `with_air_drag`.
    pub fn with_damping(mut self, linear: f32, angular: f32) -> Self {
        self.rigid_body = self.rigid_body.with_damping(linear, angular);
        self
    }

    /// Turn gravity on/off.
    pub fn with_gravity(mut self, enabled: bool) -> Self {
        self.rigid_body = self.rigid_body.with_gravity(enabled);
        self
    }

    /// Sets the center of mass (body-local).
    pub fn with_center_of_mass(mut self, com: Vec3) -> Self {
        self.rigid_body = self.rigid_body.with_center_of_mass(com);
        self
    }

    /// Locks the three rotation axes — the body does not topple (characters, upright objects).
    pub fn lock_rotation(mut self) -> Self {
        self.rigid_body = self.rigid_body.lock_rotation();
        self
    }

    /// Gives an initial angular velocity (rad/s).
    pub fn with_angular_velocity(mut self, angular: Vec3) -> Self {
        self.velocity.angular = angular;
        self
    }
}

#[cfg(feature = "physics")]
impl Bundle for RigidBodyBundle {
    fn get_infos() -> Vec<gizmo_core::archetype::ComponentInfo> {
        <(RigidBody, Velocity, Collider)>::get_infos()
    }

    unsafe fn write_to_archetype(self, arch: &mut gizmo_core::archetype::Archetype, row: usize, tick: u32) {
        let mut rb = self.rigid_body;
        rb.update_inertia_from_collider(&self.collider);
        (rb, self.velocity, self.collider).write_to_archetype(arch, row, tick)
    }

    fn apply(self, world: &mut World, entity: Entity) {
        let mut rb = self.rigid_body;
        // Derive rotational inertia from the collider shape so callers don't have to
        // remember `calculate_*_inertia` — the default inertia otherwise gives wrong
        // spin dynamics. No-op for static/kinematic bodies (the calculators guard on
        // `is_dynamic`), and idempotent if the caller already set a matching inertia.
        rb.update_inertia_from_collider(&self.collider);
        (rb, self.velocity, self.collider).apply(world, entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ergonomic_collider_and_bundle_builders() {
        // Collider zıplaklık/sürtünme kısayolları — tam PhysicsMaterial kurmadan.
        let c = Collider::sphere(0.5).with_restitution(0.9).with_friction(0.3);
        assert_eq!(c.material.restitution, 0.9);
        assert_eq!(c.material.static_friction, 0.3);
        assert_eq!(c.material.dynamic_friction, 0.3);

        // Bundle: collider + hava direnci + zıplaklık TEK zincirde.
        let b = RigidBodyBundle::dynamic(2.0)
            .with_collider(Collider::sphere(0.5))
            .with_air_drag(0.47, 0.8)
            .with_restitution(0.85);
        assert_eq!(b.rigid_body.drag_coefficient, 0.47);
        assert_eq!(b.rigid_body.drag_area, 0.8);
        assert_eq!(b.collider.material.restitution, 0.85);

        // Genişletilmiş akıcı set: damping + gravity + lock + COM + açısal hız tek zincirde.
        let b2 = RigidBodyBundle::dynamic(1.0)
            .with_damping(0.1, 0.2)
            .with_gravity(false)
            .lock_rotation()
            .with_center_of_mass(Vec3::new(0.0, 0.5, 0.0))
            .with_angular_velocity(Vec3::new(0.0, 3.0, 0.0));
        assert_eq!(b2.rigid_body.linear_damping, 0.1);
        assert!(!b2.rigid_body.use_gravity);
        assert!(b2.rigid_body.lock_rotation_x);
        assert_eq!(b2.rigid_body.center_of_mass, Vec3::new(0.0, 0.5, 0.0));
        assert_eq!(b2.velocity.angular, Vec3::new(0.0, 3.0, 0.0));
    }

    #[test]
    fn rigid_body_bundle_derives_inertia_from_collider() {
        // Spawn purely via the bundle — no manual calculate_*_inertia.
        let mut world = World::new();
        let e = world
            .spawn_bundle(RigidBodyBundle::dynamic(2.0).with_collider(Collider::sphere(0.5)));

        let rbs = world.borrow::<RigidBody>();
        let rb = rbs.get(e.id()).expect("rigid body spawned");

        // Solid sphere: I = 0.4·m·r² = 0.4·2·0.25 = 0.2 per axis (calculate_sphere_inertia).
        assert!(
            (rb.local_inertia - Vec3::splat(0.2)).length() < 1e-6,
            "bundle must derive sphere inertia from the collider, got {:?}",
            rb.local_inertia
        );
        // Regression: must NOT be the un-derived default Vec3::splat(1.0).
        assert!(
            (rb.local_inertia - Vec3::splat(1.0)).length() > 1e-3,
            "inertia must be derived, not left at the default"
        );
    }

    #[test]
    fn rigid_body_bundle_static_body_inertia_derivation_is_noop() {
        // Static bodies must spawn fine; inertia derivation is a no-op for them.
        let mut world = World::new();
        let e = world
            .spawn_bundle(RigidBodyBundle::static_body().with_collider(Collider::sphere(0.5)));
        assert!(world.borrow::<RigidBody>().get(e.id()).is_some());
    }
}

// ============================================================
//  Prefab — reusable spawn blueprint
// ============================================================

/// Reusable SPAWN BLUEPRINT: define a mesh + material + (optional) physics body ONCE, spawn it
/// MANY TIMES with a `transform`/color — it prevents the demos from repeating the
/// `spawn_bundle((Transform, mesh.clone(), material, MeshRenderer, RigidBodyBundle…))` chain by
/// hand for every object. Mesh/Material are Arc-backed → clone is cheap.
///
/// ```no_run
/// # use gizmo::prelude::*;
/// # fn build(renderer: &Renderer, stone_mat: Material, positions: Vec<Vec3>, color: Vec4) {
/// # let mut world = World::new();
/// // Define it once (physics config included):
/// let block = Prefab::new(renderer.create_cube(), stone_mat)
///     .with_body(RigidBodyBundle::dynamic(1.0)
///         .with_collider(Collider::box_collider(Vec3::splat(0.5)))
///         .with_friction(0.85));
/// // Spawn it many times (only position + colour change):
/// for pos in positions {
///     block.clone().with_pbr(color, 0.8, 0.05).spawn(&mut world, Transform::new(pos));
/// }
/// # }
/// ```
/// Reusable mesh+material(+body) template. Renderer-backed, hence gated on `render`;
/// `RigidBodyBundle` above is the renderer-free half of this module.
#[cfg(all(feature = "render", feature = "physics"))]
#[derive(Clone)]
pub struct Prefab {
    mesh: Mesh,
    material: Material,
    body: Option<RigidBodyBundle>,
    /// If `Some(base)`, the box collider is derived at SPAWN time from `transform.scale · base`
    /// (don't write the size twice). See [`auto_box_collider`](Self::auto_box_collider).
    auto_box: Option<Vec3>,
}

#[cfg(all(feature = "render", feature = "physics"))]
impl Prefab {
    /// Visual-only prefab (no physics). For physics, chain [`with_body`](Self::with_body).
    pub fn new(mesh: Mesh, material: Material) -> Self {
        Self {
            mesh,
            material,
            body: None,
            auto_box: None,
        }
    }

    /// Add a physics body (`RigidBodyBundle`: dynamic/static + collider + friction…). When the
    /// prefab is cloned the body config is cloned too (each instance gets its own rigid body).
    pub fn with_body(mut self, body: RigidBodyBundle) -> Self {
        self.body = Some(body);
        self
    }

    /// Change the material's color/PBR (per-instance tint — the base texture is preserved). To
    /// spawn the same blueprint with different colors, `prefab.clone().with_pbr(...)`.
    pub fn with_pbr(mut self, albedo: Vec4, roughness: f32, metallic: f32) -> Self {
        self.material = self.material.clone().with_pbr(albedo, roughness, metallic);
        self
    }

    /// Derive the box collider from `transform.scale` on every spawn (base = `Vec3::ONE`, for
    /// `create_cube`, whose mesh half-width == the scale) — so a SINGLE blueprint covers a block
    /// of any size and you don't write the size twice. The collider in
    /// [`with_body`](Self::with_body) is a placeholder; at spawn it is OVERWRITTEN according to
    /// the scale (material/friction are preserved). If the prefab's body was set to a NON-box
    /// collider it is still replaced with a box (this builder means "this prefab is a box").
    pub fn auto_box_collider(self) -> Self {
        self.auto_box_collider_scaled(Vec3::ONE)
    }

    /// [`auto_box_collider`](Self::auto_box_collider) but with a custom per-axis base multiplier
    /// (e.g. `Vec3::splat(0.5)` for a 0.5-factor mesh family → half-width = scale/2).
    pub fn auto_box_collider_scaled(mut self, base: Vec3) -> Self {
        self.auto_box = Some(base);
        self
    }

    /// Spawn one instance at the `transform` position; return the entity (e.g. to add a
    /// name/marker component).
    pub fn spawn(&self, world: &mut World, transform: Transform) -> Entity {
        self.spawn_inner(world, transform, None)
    }

    /// Convenience: spawn at the given position (default rotation/scale).
    pub fn spawn_at(&self, world: &mut World, position: Vec3) -> Entity {
        self.spawn(world, Transform::new(position))
    }

    /// [`spawn`](Self::spawn) but with a mass specific to this instance — the inertia is
    /// automatically re-derived from the scaled box + the new mass. For spawning instances of
    /// differing mass from the same blueprint (e.g. a light block vs a heavy beam).
    pub fn spawn_with_mass(&self, world: &mut World, transform: Transform, mass: f32) -> Entity {
        self.spawn_inner(world, transform, Some(mass))
    }

    fn spawn_inner(
        &self,
        world: &mut World,
        transform: Transform,
        mass_override: Option<f32>,
    ) -> Entity {
        let scale = transform.scale;
        let e = world.spawn_bundle((
            transform,
            self.mesh.clone(),
            self.material.clone(),
            MeshRenderer::new(),
        ));
        if let Some(body) = &self.body {
            let mut body = body.clone();
            if let Some(base) = self.auto_box {
                // Collider'ı bu örneğin ölçeğinden türet — TEK boyut kaynağı.
                let he = crate::systems::auto_collider::derived_box_half_extents(scale, base);
                body.collider.shape = ColliderShape::Box(BoxShape { half_extents: he });
            }
            if let Some(m) = mass_override {
                body.rigid_body.mass = m;
            }
            // add_bundle → RigidBodyBundle::write_to_archetype, ölçekli kutu + kütleden
            // ataleti yeniden türetir → shape/atalet tutarlı, yeni atalet kodu yok.
            world.add_bundle(e, body);
        }
        e
    }
}
