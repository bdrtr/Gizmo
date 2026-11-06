//! Applying a [`CameraShake`] to the frame being drawn — and only to it.

use gizmo_core::Mut;
use gizmo_core::Entity;
use crate::core::World;
use crate::renderer::components::{Camera, CameraShake};
use gizmo_physics_core::components::{GlobalTransform, Transform};

/// What [`apply_camera_shake`] moved, to be put back by [`undo_camera_shake`].
#[must_use = "a shake that is not undone moves the camera for good"]
#[derive(Debug, Default)]
pub struct ShakeUndo(Vec<Moved>);

#[derive(Debug)]
struct Moved {
    id: u32,
    yaw: f32,
    pitch: f32,
    position: Option<gizmo_math::Vec3>,
    global: Option<GlobalTransform>,
}

/// Advances every [`CameraShake`] by `dt` and moves its camera by this instant's shake — its
/// `Transform` along the camera's own right/up axes, its `Camera` yaw/pitch — remembering exactly
/// what each was. Both draw paths call this before the frame's transform pass and
/// [`undo_camera_shake`] after drawing, so the shake is in the picture and nowhere else.
///
/// Restoring the remembered values rather than subtracting the offset is deliberate: `p + o - o`
/// is not `p` in floating point, and a camera that drifts by an ulp a frame drifts for good.
pub fn apply_camera_shake(world: &mut World, dt: f32) -> ShakeUndo {
    let mut undo = ShakeUndo::default();
    let shaking: Vec<(u32, (gizmo_math::Vec3, f32, f32))> = {
        let mut shakes = world.query_mut::<(Entity, Mut<CameraShake>)>().unwrap();
        shakes
            .iter_mut()
            .map(|(entity, mut s)| {
                s.advance(dt);
                (entity.id(), s.sample())
            })
            .collect()
    };
    for (id, (offset, yaw, pitch)) in shaking {
        if offset == gizmo_math::Vec3::ZERO && yaw == 0.0 && pitch == 0.0 {
            continue;
        }
        let Some((right, up, old_yaw, old_pitch)) = ({
            let mut cams = world.borrow_mut::<Camera>();
            cams.get_mut(id).map(|mut c| {
                let right = c.get_right();
                let up = right.cross(c.get_front());
                let old = (c.yaw, c.pitch);
                c.yaw += yaw;
                c.pitch += pitch;
                (right, up, old.0, old.1)
            })
        }) else {
            continue;
        };
        let shift = right * offset.x + up * offset.y;
        let position = world.borrow_mut::<Transform>().get_mut(id).map(|mut t| {
            let old = t.position;
            t.position += shift;
            old
        });
        let global = world.borrow::<GlobalTransform>().get(id).copied();
        undo.0.push(Moved { id, yaw: old_yaw, pitch: old_pitch, position, global });
    }
    undo
}

/// Puts every camera [`apply_camera_shake`] moved back exactly where it was — `Transform`,
/// `Camera` yaw/pitch and the `GlobalTransform` the game will read before the next frame's
/// transform pass.
pub fn undo_camera_shake(world: &mut World, undo: ShakeUndo) {
    for m in undo.0 {
        if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(m.id) {
            c.yaw = m.yaw;
            c.pitch = m.pitch;
        }
        if let Some(p) = m.position {
            if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(m.id) {
                t.position = p;
            }
        }
        if let Some(g) = m.global {
            if let Some(mut gt) = world.borrow_mut::<GlobalTransform>().get_mut(m.id) {
                *gt = g;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    /// The frame sees the camera moved; after it, the camera is bit-for-bit where the game left
    /// it — position, angles and world matrix — however many frames shake.
    #[test]
    fn the_shake_is_in_the_frame_and_gone_after_it() {
        let mut world = World::new();
        let cam = Camera::new(1.0, 0.1, 100.0, 0.3, -0.2, true);
        let start = Transform::new(Vec3::new(1.0, 2.0, 3.0));
        let mut shake = CameraShake::default();
        shake.add(1.0);
        shake.decay = 0.0;
        let id = world.spawn_bundle((cam, start, GlobalTransform::default(), shake)).id();
        let global_before = *world.borrow::<GlobalTransform>().get(id).unwrap();

        for _ in 0..50 {
            let undo = apply_camera_shake(&mut world, 1.0 / 60.0);
            let moved = world.borrow::<Transform>().get(id).unwrap().position;
            let c = *world.borrow::<Camera>().get(id).unwrap();
            assert!(moved != start.position || c.yaw != 0.3 || c.pitch != -0.2, "nothing shook");
            // The draw path's transform pass runs here and writes the world matrix.
            world.borrow_mut::<GlobalTransform>().get_mut(id).unwrap().matrix =
                gizmo_math::Mat4::from_translation(moved);
            undo_camera_shake(&mut world, undo);
            assert_eq!(world.borrow::<Transform>().get(id).unwrap().position, start.position);
            let c = *world.borrow::<Camera>().get(id).unwrap();
            assert_eq!((c.yaw, c.pitch), (0.3, -0.2));
            assert_eq!(*world.borrow::<GlobalTransform>().get(id).unwrap(), global_before);
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
        let id = world.spawn_bundle((cam, Transform::new(Vec3::ZERO), shake)).id();
        for _ in 0..20 {
            let undo = apply_camera_shake(&mut world, 0.05);
            let p = world.borrow::<Transform>().get(id).unwrap().position;
            assert!(p.dot(front).abs() < 1e-5, "moved {} along the view", p.dot(front));
            undo_camera_shake(&mut world, undo);
        }
    }
}
