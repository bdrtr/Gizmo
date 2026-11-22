//! Which camera the Scene panel draws from, and the game camera's frustum it outlines.
//!
//! # Why this is its own file
//!
//! **The editor draws from the game camera in play mode and the editor camera otherwise** — but
//! only if the game camera actually exists, which is the branch nobody thinks about until a scene
//! without one goes black. It was buried in the middle of a 600-line function once, and is a pure
//! function of the world and two entity ids, which is what makes the tests at the bottom possible.
//!
//! This file also decided what the Scene panel culled against: the game camera's frustum even in
//! edit mode, so flying the editor camera around showed what the game would and would not draw.
//! The panel draws through the game path since 2026-11-22, which culls by the camera it draws from,
//! as other editors' scene views do (the owner's choice); the game camera's frustum is still
//! outlined, in green (`EditorLines`).

use gizmo::prelude::*;

/// Whether the editor is playing or paused, i.e. showing the game rather than the scene.
pub(super) fn is_playing(world: &World) -> bool {
    world
        .get_resource::<gizmo::editor::EditorState>()
        .is_some_and(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
}

/// The camera the Scene panel is drawn from: the game camera when playing and it exists, the
/// editor camera otherwise.
pub(super) fn scene_camera(world: &World, editor_camera: Entity, game_camera: Entity) -> Entity {
    let has_camera = world.query::<&Camera>().get(game_camera).is_some();
    if is_playing(world) && has_camera {
        game_camera
    } else {
        editor_camera
    }
}

/// The game camera's view-projection at `aspect`, as this frame draws it (its `CameraShake`
/// applied by the game path's own `shake_view`, which writes nothing). `None` with no game camera.
pub(super) fn game_view_proj(world: &World, game_camera: Entity, aspect: f32) -> Option<Mat4> {
    let cameras = world.query::<&Camera>();
    let transforms = world.query::<&Transform>();
    let (cam, trans) = (cameras.get(game_camera)?, transforms.get(game_camera)?);
    let (at, turn) = gizmo::systems::render::shake_view(world, game_camera, trans.position, trans.rotation);
    Some(cam.get_projection(aspect) * Camera::view(at, turn))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawns a camera at `pos`.
    fn camera_at(world: &mut World, pos: Vec3) -> Entity {
        let e = world.spawn();
        world.add_component(e, Transform::new(pos));
        world.add_component(e, Camera::new(std::f32::consts::FRAC_PI_4, 0.1, 100.0, false));
        e
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
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0));
        let game = camera_at(&mut world, Vec3::new(9.0, 0.0, 0.0));
        playing(&mut world, false);
        assert_eq!(scene_camera(&world, editor, game), editor);
        assert!(!is_playing(&world));
    }

    #[test]
    fn play_mode_draws_from_the_game_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0));
        let game = camera_at(&mut world, Vec3::new(9.0, 0.0, 0.0));
        playing(&mut world, true);
        assert_eq!(scene_camera(&world, editor, game), game, "play mode shows what the game camera sees");
        assert!(is_playing(&world));
    }

    /// The branch that decides whether a scene without a game camera goes black.
    #[test]
    fn play_mode_without_a_game_camera_falls_back_to_the_editor_camera() {
        let mut world = World::new();
        let editor = camera_at(&mut world, Vec3::new(1.0, 0.0, 0.0));
        playing(&mut world, true);
        // An id no entity has.
        assert_eq!(scene_camera(&world, editor, Entity::new(4242, 0)), editor, "a missing game camera must not blank the viewport");
    }

    /// The frustum outline is the game camera's, from where it stands, and there is none without one.
    #[test]
    fn the_outlined_frustum_is_the_game_cameras() {
        let mut world = World::new();
        let game = camera_at(&mut world, Vec3::new(9.0, 0.0, 0.0));
        let vp = game_view_proj(&world, game, 16.0 / 9.0).expect("a game camera");
        // The camera's own position maps to the near plane's centre line: w = 0 there is behind,
        // so test a point straight ahead of it instead — inside the frustum.
        let ahead = vp * Vec4::new(9.0, 0.0, -10.0, 1.0);
        let ndc = ahead.truncate() / ahead.w;
        assert!(ahead.w > 0.0 && ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "a point ahead of the game camera is outside its frustum: {ndc:?}");
        assert!(game_view_proj(&world, Entity::new(4242, 0), 1.0).is_none());
    }
}
