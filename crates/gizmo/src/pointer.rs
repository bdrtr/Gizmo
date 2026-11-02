//! The mouse cursor in the world.
//!
//! Every game that is played with a mouse on a 3D scene — placing a tower, selecting a unit,
//! aiming at the ground — needs "what is under the cursor", and the engine had the pieces but not
//! the answer: [`Camera::screen_to_ray`](crate::renderer::components::Camera::screen_to_ray) wants
//! the window size, the camera's world position and the cursor, each from a different place, and
//! the ray then has to be cut with the ground by hand. `kale` wrote those fifteen lines; this is
//! them, once.

use crate::core::input::Input;
use crate::core::window::WindowInfo;
use crate::core::World;
use crate::math::{Ray, Vec3};
use crate::renderer::components::{active_camera, Camera};

/// The ray from the camera through the mouse cursor, or `None` when there is no camera, no
/// window size or no [`Input`] resource.
///
/// The camera is the one **under the cursor**: in split screen, the camera whose
/// [`viewport`](Camera::viewport) contains it (highest [`order`](Camera::order) wins where they
/// overlap), and the ray is cast through that rectangle at its own aspect ratio. Otherwise —
/// one camera, or a cursor outside every viewport — it is
/// [`active_camera`], the camera the frame is rendered through, over the whole window.
///
/// The camera's position is its `GlobalTransform` (falling back to its `Transform`), the same
/// position the frame is rendered from.
pub fn cursor_ray(world: &World) -> Option<Ray> {
    let input = world.get_resource::<Input>()?;
    let info = world.get_resource::<WindowInfo>()?;
    let window = (info.width.max(1.0), info.height.max(1.0));
    let cursor = input.mouse_position();

    let cameras = world.borrow::<Camera>();
    // A viewport camera under the cursor, else the camera the frame is drawn through.
    let in_viewport = cameras
        .iter()
        .filter_map(|(id, c)| c.viewport.map(|v| (id, c.order, v)))
        .filter(|(_, _, v)| {
            let (x, y) = (cursor.0 / window.0, cursor.1 / window.1);
            x >= v.x && x < v.x + v.width && y >= v.y && y < v.y + v.height
        })
        .max_by_key(|(_, order, _)| *order);
    let (id, screen, size) = match in_viewport {
        Some((id, _, v)) => (
            id,
            (cursor.0 - v.x * window.0, cursor.1 - v.y * window.1),
            (v.width * window.0, v.height * window.1),
        ),
        None => (active_camera(world)?, cursor, window),
    };
    let camera = *cameras.get(id)?;
    drop(cameras);
    Some(camera.screen_to_ray(screen, size, camera_position(world, id)?))
}

/// Where the ray under the cursor meets the plane through `point` with normal `normal` — the
/// ground under the mouse is `cursor_on_plane(world, Vec3::ZERO, Vec3::Y)`. `None` without a ray
/// (see [`cursor_ray`]), or when the cursor looks away from the plane or along it.
pub fn cursor_on_plane(world: &World, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let ray = cursor_ray(world)?;
    let t = ray.intersect_plane(point, normal)?;
    Some(Vec3::from(ray.at(t)))
}

fn camera_position(world: &World, id: u32) -> Option<Vec3> {
    world
        .borrow::<gizmo_physics_core::components::GlobalTransform>()
        .get(id)
        .map(|g| g.matrix.to_scale_rotation_translation().2)
        .or_else(|| world.borrow::<gizmo_physics_core::components::Transform>().get(id).map(|t| t.position))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundles::CameraBundle;
    use crate::renderer::components::Viewport;

    /// A camera 10 m up and 10 m back, pitched 45° down — the centre of its view is the origin.
    fn scene(width: f32, height: f32, cursor: (f32, f32)) -> World {
        let mut world = World::new();
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 10.0, 10.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -std::f32::consts::FRAC_PI_4,
            primary: true,
            ..Default::default()
        });
        world.insert_resource(WindowInfo { width, height });
        let mut input = Input::new();
        input.set_mouse_position(cursor.0, cursor.1);
        world.insert_resource(input);
        propagate(&mut world);
        world
    }

    /// `GlobalTransform` is what the frame renders from, and it is written by propagation — run
    /// it, as the app loop does every frame.
    fn propagate(world: &mut World) {
        use crate::core::system::System;
        crate::systems::transform::TransformPropagateSystem.run(world, 0.0);
    }

    #[test]
    fn the_centre_of_the_window_looks_at_what_the_camera_looks_at() {
        for (w, h) in [(1280.0, 720.0), (472.0, 492.0)] {
            let world = scene(w, h, (w * 0.5, h * 0.5));
            let hit = cursor_on_plane(&world, Vec3::ZERO, Vec3::Y).expect("the ground is in view");
            assert!(hit.length() < 1e-3, "{w}×{h}: the centre ray should hit the origin, got {hit:?}");
        }
    }

    /// Up the screen is further away on the ground, right is right.
    #[test]
    fn moving_the_cursor_moves_the_ground_point_the_right_way() {
        let up = cursor_on_plane(&scene(1280.0, 720.0, (640.0, 200.0)), Vec3::ZERO, Vec3::Y).unwrap();
        let right = cursor_on_plane(&scene(1280.0, 720.0, (1000.0, 360.0)), Vec3::ZERO, Vec3::Y).unwrap();
        assert!(up.z < -0.5, "higher on screen is further from the camera (−Z), got {up:?}");
        assert!(right.x > 0.5, "right on screen is +X, got {right:?}");
    }

    /// Looking at the sky: no ground point.
    #[test]
    fn a_cursor_above_the_horizon_has_no_ground_point() {
        assert!(cursor_on_plane(&scene(1280.0, 720.0, (640.0, 0.0)), Vec3::new(0.0, 20.0, 0.0), Vec3::Y).is_none());
    }

    /// In split screen the camera under the cursor casts the ray, through its own rectangle: the
    /// centre of the right half looks where the right camera looks.
    #[test]
    fn in_split_screen_the_camera_under_the_cursor_casts_the_ray() {
        let mut world = scene(1280.0, 720.0, (960.0, 360.0));
        // A second camera on the right half, 20 m to the side of the first.
        let right = world.spawn_bundle(CameraBundle {
            position: Vec3::new(20.0, 10.0, 10.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: -std::f32::consts::FRAC_PI_4,
            ..Default::default()
        });
        if let Some(mut c) = world.borrow_mut::<Camera>().get_mut(right.id()) {
            c.viewport = Some(Viewport::right_half());
        }
        propagate(&mut world);
        let hit = cursor_on_plane(&world, Vec3::ZERO, Vec3::Y).unwrap();
        assert!((hit - Vec3::new(20.0, 0.0, 0.0)).length() < 1e-2, "the right camera's centre is (20, 0, 0), got {hit:?}");
    }
}
