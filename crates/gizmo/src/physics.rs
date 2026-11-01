//! Gizmo Physics Module Re-exports
//! This module provides backward-compatible exports for the split physics crates.

pub use gizmo_physics_core::*;

/// Physics components from both halves of the split: the core's `Transform`, `Collider` and
/// materials, plus the rigid crate's `RigidBody` and `Velocity`.
pub mod components {
    pub use gizmo_physics_core::components::*;
    pub use gizmo_physics_rigid::components::*;
}

// Some legacy usages accessed RigidBody directly from gizmo::physics
pub use components::{RigidBody, Velocity, GpuPhysicsLink, GlobalTransform};
pub use gizmo_physics_core::Transform;

/// The rigid-body world — `PhysicsWorld`, its step, and the scene queries over it.
pub mod world {
    pub use gizmo_physics_rigid::world::*;
}

/// Joints and the joint solver: hinges, sliders, ball sockets, D6 and the rest.
pub mod joints {
    pub use gizmo_physics_rigid::joints::*;
}

#[cfg(feature = "physics-soft")]
/// Soft bodies: the FEM solver and the deformable primitives built on it.
pub mod soft_body {
    pub use gizmo_physics_soft::*;
}

/// The ECS bridge — `physics_step_system` and the systems around it.
pub mod system {
    pub use gizmo_physics_rigid::system::*;
}

#[cfg(feature = "physics-dynamics")]
/// Vehicle dynamics: the wheel model, the drivetrain and the tyre curves.
pub mod vehicle {
    pub use gizmo_physics_dynamics::vehicle::*;
}

#[cfg(feature = "physics-dynamics")]
/// The kinematic character controller, including its swimming path.
pub mod character {
    pub use gizmo_physics_dynamics::character::*;
}

#[cfg(feature = "physics-dynamics")]
/// Ragdolls — a joint skeleton built from a set of bodies.
pub mod ragdoll {
    pub use gizmo_physics_dynamics::ragdoll::*;
}

/// Just [`ColliderShape`](gizmo_physics_core::ColliderShape), for code that only needs the
/// shape vocabulary.
pub mod shape {
    pub use gizmo_physics_core::ColliderShape;
}

#[cfg(feature = "physics-soft")]
/// Ropes and cables, from the soft-body crate.
pub mod rope {
    pub use gizmo_physics_soft::rope::*;
}

/// Fracture: Voronoi shattering and the debris it produces.
pub mod fracture {
    pub use gizmo_physics_rigid::fracture::*;
}


#[cfg(feature = "physics-soft")]
/// Cloth simulation, from the soft-body crate.
pub mod cloth {
    pub use gizmo_physics_soft::cloth::*;
}

pub use system::{physics_fracture_system, physics_explosion_system, physics_step_system};

// Gameplay controller systems (Pacejka vehicle + kinematic character). These drive
// `VehicleController` / `CharacterController` and must run *before* `physics_step_system`
// so the rigid step integrates the forces they write into `Velocity`. Demos that step
// physics manually (car_demo, vehicle_scene, hill_climb) call these directly.
//
// `fighter_frame_system` is in the same list for a different reason: it integrates nothing, it
// counts frames on `FighterController`, and it needs the same one-call-per-fixed-step placement.
// `PlayLoop` calls it, so the editor's ▶ and every exported game have the fight clock; a game
// stepping physics by hand calls it the way the demos above call theirs.
#[cfg(feature = "physics-dynamics")]
pub use gizmo_physics_dynamics::{
    character_controller_system, fighter_frame_system, hit_detection_system, oxygen_system,
    vehicle_controller_system, Oxygen,
};

/// A box collider that covers a world-space [`Sprite`](crate::renderer::components::Sprite)
/// exactly — its size, and where its [`anchor`](crate::renderer::components::Sprite::anchor) puts
/// it relative to the entity — with `depth` world units of thickness along local Z.
///
/// Without this a sprite and its collider are two numbers that only agree if the game keeps them
/// agreeing: `kule_yikici` wrote a `spawn_box` whose one job was to derive both from the same
/// half-size (`docs/CAPABILITY_GAPS.md` §C9). Here the collider is derived *from the sprite*, so
/// changing the sprite's size or anchor and re-deriving cannot leave them apart.
///
/// - A centred sprite (the default anchor) gets a plain [`Collider::box_collider`]. Any other
///   anchor gets a [`Collider::offset_box`], because the quad is then not centred on the
///   entity's origin — `(0.5, 1.0)` stands a character on its feet, and the box then sits above
///   the origin, not around it. An offset box also moves the body's centre of mass; see
///   `offset_box`.
/// - **Scale:** the renderer applies the entity's `Transform::scale` to a sprite and the physics
///   does **not** apply it to a collider. For a scaled entity use [`sprite_collider_scaled`].
/// - `None` for a [`SpriteSpace::Screen`](crate::renderer::components::SpriteSpace) sprite (it
///   has no place in the world) and for a sprite with no area.
///
/// The collider takes the usual defaults (material, layer); set those on the result.
#[cfg(feature = "render")]
pub fn sprite_collider(
    sprite: &crate::renderer::components::Sprite,
    depth: f32,
) -> Option<components::Collider> {
    sprite_collider_scaled(sprite, gizmo_math::Vec3::ONE, depth)
}

/// [`sprite_collider`] for an entity whose `Transform::scale` is `scale`: the box is sized the
/// way the renderer will draw the sprite. `scale.z` multiplies `depth`. Negative scale components
/// (a mirrored entity) size the box by their magnitude.
#[cfg(feature = "render")]
pub fn sprite_collider_scaled(
    sprite: &crate::renderer::components::Sprite,
    scale: gizmo_math::Vec3,
    depth: f32,
) -> Option<components::Collider> {
    if !sprite.space.needs_world_transform() {
        return None;
    }
    box_for_quad(sprite.size, sprite.anchor, scale, depth)
}

/// The geometry of [`sprite_collider_scaled`], on plain numbers.
#[cfg(feature = "render")]
fn box_for_quad(
    size: gizmo_math::Vec2,
    anchor: gizmo_math::Vec2,
    scale: gizmo_math::Vec3,
    depth: f32,
) -> Option<components::Collider> {
    use gizmo_math::{Vec2, Vec3};
    let scale = scale.abs();
    let size = size.abs() * scale.truncate();
    if !(size.x > 0.0 && size.y > 0.0 && size.is_finite()) {
        return None;
    }
    let half = Vec3::new(size.x * 0.5, size.y * 0.5, (depth.abs() * scale.z * 0.5).max(1e-3));
    // The renderer places a corner `c ∈ [0,1]²` at `(c − anchor) · (w, −h)` (top-left origin, Y
    // down in the image, Y up in the world). The centre is `c = (0.5, 0.5)`.
    let centre = (Vec2::splat(0.5) - anchor) * Vec2::new(size.x, -size.y);
    Some(if centre == Vec2::ZERO {
        components::Collider::box_collider(half)
    } else {
        components::Collider::offset_box(centre.extend(0.0), half)
    })
}

#[cfg(all(test, feature = "render"))]
mod sprite_collider_tests {
    use super::*;
    use gizmo_math::{Vec2, Vec3};

    fn box_of(c: &components::Collider) -> (Vec3, Vec3) {
        match &c.shape {
            components::ColliderShape::Box(b) => (Vec3::ZERO, b.half_extents),
            components::ColliderShape::Compound(parts) => match (parts.len(), &*parts[0].1) {
                (1, components::ColliderShape::Box(b)) => (parts[0].0.position, b.half_extents),
                _ => panic!("expected one box part"),
            },
            other => panic!("expected a box, got {other:?}"),
        }
    }

    #[test]
    fn a_centred_sprite_gets_a_centred_box_of_its_size() {
        let c = box_for_quad(Vec2::new(3.0, 1.0), Vec2::splat(0.5), Vec3::ONE, 1.0).unwrap();
        assert!(matches!(c.shape, components::ColliderShape::Box(_)), "no compound when centred");
        assert_eq!(box_of(&c), (Vec3::ZERO, Vec3::new(1.5, 0.5, 0.5)));
    }

    /// A character anchored at its feet: the box sits above the origin, bottom edge on it — and
    /// a top-left anchor puts it right and below. Each case is the renderer's own corner rule.
    #[test]
    fn the_anchor_moves_the_box_where_the_renderer_moves_the_quad() {
        let feet = box_for_quad(Vec2::new(1.0, 2.0), Vec2::new(0.5, 1.0), Vec3::ONE, 0.2);
        let (centre, half) = box_of(&feet.unwrap());
        assert_eq!(centre, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(centre.y - half.y, 0.0, "the feet are on the origin");

        let top_left = box_for_quad(Vec2::new(4.0, 2.0), Vec2::ZERO, Vec3::ONE, 1.0);
        let (centre, _) = box_of(&top_left.unwrap());
        assert_eq!(centre, Vec3::new(2.0, -1.0, 0.0));
    }

    #[test]
    fn scale_sizes_the_box_the_way_the_renderer_draws_the_sprite() {
        let scaled = box_for_quad(Vec2::new(2.0, 2.0), Vec2::new(0.5, 1.0), Vec3::new(-3.0, 0.5, 2.0), 1.0);
        let (centre, half) = box_of(&scaled.unwrap());
        assert_eq!(half, Vec3::new(3.0, 0.5, 1.0));
        assert_eq!(centre, Vec3::new(0.0, 0.5, 0.0));
    }

    #[test]
    fn a_quad_with_no_area_has_no_collider() {
        assert!(box_for_quad(Vec2::new(0.0, 1.0), Vec2::splat(0.5), Vec3::ONE, 1.0).is_none());
        assert!(box_for_quad(Vec2::new(f32::NAN, 1.0), Vec2::splat(0.5), Vec3::ONE, 1.0).is_none());
        assert!(box_for_quad(Vec2::ONE, Vec2::splat(0.5), Vec3::new(1.0, 0.0, 1.0), 1.0).is_none());
    }
}
