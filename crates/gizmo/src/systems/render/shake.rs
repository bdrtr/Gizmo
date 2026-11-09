//! A [`CameraShake`] in the frame being drawn — and only there.
//!
//! Two halves, on the two sides of the frame. [`advance_camera_shakes`] moves every shake's clock
//! once a frame, with the other clocks a game can see ([`advance_frame_clocks`](super::advance_frame_clocks)):
//! the trauma wearing off is game state, and `CameraShake::trauma` reads it. [`shake_view`] is
//! the draw's: it takes a camera and where it stands and returns the shaken pair, **without
//! writing anything**. The camera's `Camera`, `Transform` and `GlobalTransform` are never moved —
//! the draw used to move them, draw, and put them back, which is a write into the world the game
//! owns and the reason the draw could not take that world read-only.

use crate::core::World;
use crate::renderer::components::{Camera, CameraShake};
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

/// `camera`, standing at `position`, as this instant's shake of entity `id` moves it: yaw and
/// pitch added to a copy of the camera, and the position shifted along the camera's own right/up
/// axes. Unchanged when `id` has no [`CameraShake`] or its trauma is gone.
///
/// The shift applies only to a camera with a `Transform` — the shake is a movement of the
/// camera's place, and one with no place of its own (a bare `GlobalTransform`) was never moved.
/// Reads the world and nothing else: every draw path asks this, so none of them writes the camera.
#[must_use]
pub fn shake_view(world: &World, id: Entity, camera: &Camera, position: gizmo_math::Vec3) -> (Camera, gizmo_math::Vec3) {
    let mut camera = *camera;
    let Some((offset, yaw, pitch)) = world.query::<&CameraShake>().get(id).map(|s| s.sample()) else {
        return (camera, position);
    };
    if offset == gizmo_math::Vec3::ZERO && yaw == 0.0 && pitch == 0.0 {
        return (camera, position);
    }
    // The axes of the camera as the game left it, before this instant's yaw and pitch.
    let right = camera.get_right();
    let up = right.cross(camera.get_front());
    camera.yaw += yaw;
    camera.pitch += pitch;
    if world.query::<&Transform>().get(id).is_none() {
        return (camera, position);
    }
    (camera, position + (right * offset.x + up * offset.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::GlobalTransform;

    /// The view is shaken; the camera the game owns is not touched — position, angles and world
    /// matrix stay bit-for-bit what the game left, however many frames shake.
    #[test]
    fn the_shake_is_in_the_view_and_never_in_the_camera() {
        let mut world = World::new();
        let cam = Camera::new(1.0, 0.1, 100.0, 0.3, -0.2, true);
        let start = Transform::new(Vec3::new(1.0, 2.0, 3.0));
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.decay = 0.0;
        let global = GlobalTransform::default();
        let id = world.spawn_bundle((cam, start, global, shake));

        for _ in 0..50 {
            advance_camera_shakes(&mut world, 1.0 / 60.0);
            let (shaken, at) = shake_view(&world, id, &cam, start.position);
            assert!(at != start.position || shaken.yaw != 0.3 || shaken.pitch != -0.2, "nothing shook");
            assert_eq!(world.query::<&Transform>().get(id).unwrap().position, start.position);
            let c = *world.query::<&Camera>().get(id).unwrap();
            assert_eq!((c.yaw, c.pitch), (0.3, -0.2));
            assert_eq!(*world.query::<&GlobalTransform>().get(id).unwrap(), global);
        }
    }

    /// The offset is in the camera's own plane: it never moves the camera along its look axis.
    #[test]
    fn the_offset_is_across_the_view_not_along_it() {
        let mut world = World::new();
        let cam = Camera::new(1.0, 0.1, 100.0, 1.1, 0.4, true);
        let front = cam.get_front();
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.max_angle = 0.0;
        let id = world.spawn_bundle((cam, Transform::new(Vec3::ZERO), shake));
        for _ in 0..20 {
            advance_camera_shakes(&mut world, 0.05);
            let (_, p) = shake_view(&world, id, &cam, Vec3::ZERO);
            assert!(p.dot(front).abs() < 1e-5, "moved {} along the view", p.dot(front));
        }
    }
}
