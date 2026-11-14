//! A [`CameraShake`] in the frame being drawn — and only there.
//!
//! Two halves, on the two sides of the frame. [`advance_camera_shakes`] moves every shake's clock
//! once a frame, with the other clocks a game can see ([`advance_frame_clocks`](super::advance_frame_clocks)):
//! the trauma wearing off is game state, and `CameraShake::trauma` reads it. [`shake_view`] is
//! the draw's: it takes where a camera stands and how it is turned and returns the shaken pose, **without
//! writing anything**. The camera's `Camera`, `Transform` and `GlobalTransform` are never moved —
//! the draw used to move them, draw, and put them back, which is a write into the world the game
//! owns and the reason the draw could not take that world read-only.

use crate::core::World;
use crate::renderer::components::{Camera, CameraShake};
use gizmo_math::{Quat, Vec3};
use gizmo_core::Entity;
use gizmo_physics_core::components::Transform;

/// Advances every [`CameraShake`] by `dt` — its noise and the trauma wearing off. Called once a
/// frame by [`advance_frame_clocks`](super::advance_frame_clocks), on the animation clock, so a
/// paused game's shake pauses with it and a split screen does not shake twice as fast.
pub fn advance_camera_shakes(world: &mut World, dt: f32) {
    for mut shake in world.query_mut::<&mut CameraShake>().iter_mut() {
        shake.advance(dt);
    }
}

/// A camera at `position` turned by `rotation`, as this instant's shake of entity `id` moves it:
/// turned by the shake's yaw (about world `+Y`) and pitch (about its own right), and shifted along
/// its own right/up axes. Unchanged when `id` has no [`CameraShake`] or its trauma is gone.
///
/// The shift applies only to a camera with a `Transform` — the shake is a movement of the
/// camera's place, and one with no place of its own (a bare `GlobalTransform`) was never moved.
/// Reads the world and nothing else: every draw path asks this, so none of them writes the camera.
#[must_use]
pub fn shake_view(world: &World, id: Entity, position: Vec3, rotation: Quat) -> (Vec3, Quat) {
    let Some((offset, yaw, pitch)) = world.query::<&CameraShake>().get(id).map(|s| s.sample()) else {
        return (position, rotation);
    };
    if offset == Vec3::ZERO && yaw == 0.0 && pitch == 0.0 {
        return (position, rotation);
    }
    // The axes of the camera as the game left it, before this instant's yaw and pitch.
    let right = Camera::right(rotation);
    let up = right.cross(Camera::front(rotation));
    // A yaw is about world +Y (positive turns +X toward +Z, `forward_from`'s sense), a pitch about
    // the camera's own right — what adding them to the old `Camera::yaw`/`pitch` fields did.
    let turned = (Quat::from_rotation_y(-yaw) * rotation * Quat::from_rotation_x(pitch)).normalize();
    if world.query::<&Transform>().get(id).is_none() {
        return (position, turned);
    }
    (position + (right * offset.x + up * offset.y), turned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::GlobalTransform;

    /// The view is shaken; the camera the game owns is not touched — position, angles and world
    /// matrix stay bit-for-bit what the game left, however many frames shake.
    #[test]
    fn the_shake_is_in_the_view_and_never_in_the_camera() {
        let mut world = World::new();
        let cam = Camera::new(1.0, 0.1, 100.0, true);
        let aim = Camera::rotation_from(0.3, -0.2);
        let start = Transform::new(Vec3::new(1.0, 2.0, 3.0)).with_rotation(aim);
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.decay = 0.0;
        let global = GlobalTransform::default();
        let id = world.spawn_bundle((cam, start, global, shake));

        for _ in 0..50 {
            advance_camera_shakes(&mut world, 1.0 / 60.0);
            let (at, turned) = shake_view(&world, id, start.position, start.rotation);
            assert!(at != start.position || turned != aim, "nothing shook");
            assert_eq!(*world.query::<&Transform>().get(id).unwrap(), start);
            assert_eq!(*world.query::<&Camera>().get(id).unwrap(), cam);
            assert_eq!(*world.query::<&GlobalTransform>().get(id).unwrap(), global);
        }
    }

    /// The offset is in the camera's own plane: it never moves the camera along its look axis.
    #[test]
    fn the_offset_is_across_the_view_not_along_it() {
        let mut world = World::new();
        let cam = Camera::new(1.0, 0.1, 100.0, true);
        let aim = Camera::rotation_from(1.1, 0.4);
        let front = Camera::front(aim);
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.max_angle = 0.0;
        let id = world.spawn_bundle((cam, Transform::new(Vec3::ZERO).with_rotation(aim), shake));
        for _ in 0..20 {
            advance_camera_shakes(&mut world, 0.05);
            let (p, _) = shake_view(&world, id, Vec3::ZERO, aim);
            assert!(p.dot(front).abs() < 1e-5, "moved {} along the view", p.dot(front));
        }
    }
}
