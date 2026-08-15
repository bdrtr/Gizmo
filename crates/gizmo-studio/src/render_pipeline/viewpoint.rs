//! Which camera the viewport draws from, and which frustum it culls against.
//!
//! # Why this is its own file
//!
//! Two decisions were buried in the middle of `execute_render_pipeline`, forty lines apart, and
//! both are the kind that a reader has to reconstruct from `if`s:
//!
//! - **The editor draws from the game camera in play mode and the editor camera otherwise** — but
//!   only if the game camera actually exists, which is the branch nobody thinks about until a
//!   scene without one goes black.
//! - **It culls against the GAME camera's frustum even in edit mode.** That is deliberate: it lets
//!   you fly the editor camera around and watch what the game camera would and would not draw.
//!   Nothing said so except one line of Turkish in the middle of a 600-line function, and nothing
//!   checked it.
//!
//! Both are pure functions of the world and two entity ids — no GPU, no `StudioState`, no
//! `Renderer` — which is what makes the tests at the bottom possible. They are the first tests the
//! editor's render path has ever had for its own camera behaviour.

use gizmo::prelude::*;
use gizmo::renderer::{CameraFrame, Frustum};

/// The camera this frame is drawn from, plus what the rest of the pipeline needs from it.
pub(super) struct Viewpoint {
    /// Ready for the uniform blocks; carries the jitter-free view-projection.
    pub(super) camera: CameraFrame,
    /// Vertical FOV in radians — the cascade fit needs the angle, and `CameraFrame` only carries
    /// the matrix it produced.
    pub(super) fov: f32,
    /// True when the editor is playing or paused, i.e. showing the game rather than the scene.
    pub(super) is_playing_mode: bool,
}

/// The camera the frame is drawn from.
///
/// Falls back, in order: the game camera when playing and it exists → the editor camera → a
/// default 0.1..2000 perspective at the origin looking down -Z. That last one is not a nicety: a
/// freshly created scene has no camera at all for a few frames, and the alternative to a default
/// is a panic in the render loop.
pub(super) fn resolve(
    world: &World,
    editor_camera: u32,
    game_camera: u32,
    aspect: f32,
    exposure: f32,
) -> Viewpoint {
    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<Transform>();

    let is_playing_mode = world
        .get_resource::<gizmo::editor::EditorState>()
        .map(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
        .unwrap_or(false);

    let active = if is_playing_mode && cameras.get(game_camera).is_some() {
        game_camera
    } else {
        editor_camera
    };

    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut position = Vec3::ZERO;
    let mut near = 0.1f32;
    let mut far = 2000.0f32;
    let mut fov = std::f32::consts::FRAC_PI_4;
    let mut forward = Vec3::new(0.0, 0.0, -1.0);

    if let (Some(cam), Some(trans)) = (cameras.get(active), transforms.get(active)) {
        proj = cam.get_projection(aspect);
        view_mat = cam.get_view(trans.position);
        position = trans.position;
        near = cam.near;
        far = cam.far;
        fov = cam.fov;
        forward = cam.get_front();
    }

    Viewpoint {
        camera: CameraFrame { view_proj: proj * view_mat, position, forward, near, far, exposure },
        fov,
        is_playing_mode,
    }
}

/// The frustum this frame culls against.
///
/// **The game camera's, even in edit mode.** Flying the editor camera around therefore does not
/// change what is culled, which is the point: you can watch what the game would draw from where it
/// stands. In play mode the two cameras are the same one, so the question does not arise; with no
/// game camera in the scene it falls back to the drawing camera's own frustum.
pub(super) fn culling_frustum(
    world: &World,
    game_camera: u32,
    aspect: f32,
    viewpoint: &Viewpoint,
) -> Frustum {
    let drawn = Frustum::from_matrix(&viewpoint.camera.view_proj);
    if viewpoint.is_playing_mode {
        return drawn;
    }
    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<Transform>();
    match (cameras.get(game_camera), transforms.get(game_camera)) {
        (Some(cam), Some(trans)) => {
            let vp = cam.get_projection(aspect) * cam.get_view(trans.position);
            Frustum::from_matrix(&vp)
        }
        _ => drawn,
    }
}

/// The game camera's view-projection, for the wire box the gizmo pass draws around what the game
/// would see. `None` in play mode (you are already looking through it) or with no game camera.
pub(super) fn game_view_proj(
    world: &World,
    game_camera: u32,
    aspect: f32,
    viewpoint: &Viewpoint,
) -> Option<Mat4> {
    if viewpoint.is_playing_mode {
        return None;
    }
    let cameras = world.borrow::<Camera>();
    let transforms = world.borrow::<Transform>();
    match (cameras.get(game_camera), transforms.get(game_camera)) {
        (Some(cam), Some(trans)) => {
            Some(cam.get_projection(aspect) * cam.get_view(trans.position))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ASPECT: f32 = 16.0 / 9.0;

    /// Spawns a camera at `pos` with a distinctive far plane, so which one was picked is readable
    /// from the result.
    fn camera_at(world: &mut World, pos: Vec3, far: f32) -> u32 {
        let e = world.spawn();
        world.add_component(e, Transform::new(pos));
        world.add_component(
            e,
            Camera::new(std::f32::consts::FRAC_PI_4, 0.1, far, -std::f32::consts::FRAC_PI_2, 0.0, false),
        );
        e.id()
    }

    fn playing(world: &mut World, yes: bool) {
        let mut ed = gizmo::editor::EditorState::default();
        if yes {
            ed.mode = gizmo::editor::EditorMode::Play;
        }
        world.insert_resource(ed);
    }

    #[test]
    fn edit_mode_draws_from_the_editor_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0), 111.0);
        let game = camera_at(&mut world, Vec3::new(9.0, 0.0, 0.0), 999.0);
        playing(&mut world, false);

        let v = resolve(&world, editor, game, ASPECT, 1.0);
        assert_eq!(v.camera.far, 111.0, "the editor camera should be the one drawn from");
        assert_eq!(v.camera.position, Vec3::new(1.0, 0.0, 0.0));
        assert!(!v.is_playing_mode);
    }

    #[test]
    fn play_mode_draws_from_the_game_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0), 111.0);
        let game = camera_at(&mut world, Vec3::new(9.0, 0.0, 0.0), 999.0);
        playing(&mut world, true);

        let v = resolve(&world, editor, game, ASPECT, 1.0);
        assert_eq!(v.camera.far, 999.0, "play mode should show what the game camera sees");
        assert!(v.is_playing_mode);
    }

    /// The branch that decides whether a scene without a game camera goes black.
    #[test]
    fn play_mode_without_a_game_camera_falls_back_to_the_editor_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0), 111.0);
        playing(&mut world, true);

        // An id no entity has.
        let v = resolve(&world, editor, 4242, ASPECT, 1.0);
        assert_eq!(v.camera.far, 111.0, "a missing game camera must not blank the viewport");
        assert!(v.is_playing_mode, "the mode is still play — only the camera fell back");
    }

    /// A scene with no cameras at all still renders something rather than panicking.
    #[test]
    fn no_camera_at_all_yields_the_documented_default() {
        let world = World::new();
        let v = resolve(&world, 1, 2, ASPECT, 1.25);
        assert_eq!((v.camera.near, v.camera.far), (0.1, 2000.0));
        assert_eq!(v.camera.position, Vec3::ZERO);
        assert_eq!(v.camera.forward, Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(v.fov, std::f32::consts::FRAC_PI_4);
        assert_eq!(v.camera.exposure, 1.25, "exposure comes from the caller, not the camera");
    }

    /// The editor's deliberate oddity, tested for the first time: culling follows the GAME camera
    /// while the picture follows the editor camera.
    #[test]
    fn edit_mode_culls_against_the_game_camera_not_the_one_it_draws_from() {
        let mut world = World::new();
        // Editor camera at the origin looking down -Z; game camera far away looking the same way.
        let editor = camera_at(&mut world, Vec3::ZERO, 500.0);
        let game = camera_at(&mut world, Vec3::new(0.0, 0.0, 900.0), 500.0);
        playing(&mut world, false);

        let v = resolve(&world, editor, game, ASPECT, 1.0);
        let culling = culling_frustum(&world, game, ASPECT, &v);
        let drawn = Frustum::from_matrix(&v.camera.view_proj);

        // A point just in front of the EDITOR camera: drawn from, but nowhere near the game one.
        let near_editor = gizmo::math::Aabb::new(Vec3::new(-1.0, -1.0, -11.0), Vec3::new(1.0, 1.0, -9.0));
        assert!(
            drawn.intersects_aabb(near_editor),
            "the point should be inside the camera actually being drawn from"
        );
        assert!(
            !culling.intersects_aabb(near_editor),
            "culling must follow the GAME camera in edit mode — that is what makes the editor \
             able to show you what the game would drop"
        );
    }

    #[test]
    fn play_mode_culls_against_the_camera_it_draws_from() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::ZERO, 500.0);
        let game = camera_at(&mut world, Vec3::new(0.0, 0.0, 900.0), 500.0);
        playing(&mut world, true);

        let v = resolve(&world, editor, game, ASPECT, 1.0);
        let culling = culling_frustum(&world, game, ASPECT, &v);
        // Both are the game camera now, so a box in front of it is visible to both.
        let in_front = gizmo::math::Aabb::new(
            Vec3::new(-1.0, -1.0, 889.0),
            Vec3::new(1.0, 1.0, 891.0),
        );
        assert_eq!(
            culling.intersects_aabb(in_front),
            Frustum::from_matrix(&v.camera.view_proj).intersects_aabb(in_front),
            "in play mode the culling and drawing frusta are the same camera"
        );
    }

    /// With no game camera to borrow a frustum from, culling falls back to what is drawn.
    #[test]
    fn without_a_game_camera_culling_follows_the_drawn_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::ZERO, 500.0);
        playing(&mut world, false);

        let v = resolve(&world, editor, 4242, ASPECT, 1.0);
        let culling = culling_frustum(&world, 4242, ASPECT, &v);
        let boxed = gizmo::math::Aabb::new(Vec3::new(-1.0, -1.0, -11.0), Vec3::new(1.0, 1.0, -9.0));
        assert!(culling.intersects_aabb(boxed));
    }
}
