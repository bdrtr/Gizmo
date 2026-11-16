//! **Play in a world of its own.** ▶ builds a fresh world from the edited scene and runs the game
//! there; ⏹ throws that world away and puts the edited one back, untouched.
//!
//! Until 2026-11-16 ▶ ran the game in the edited world itself and ⏹ wrote a snapshot back over it —
//! a lossy one: only registered components came back, an entity deleted under ▶ returned without its
//! mesh, its material or its parent and often with another id, the physics world (joints, contacts)
//! was not restored at all, and a game's own state (a resource holding entity ids) was left
//! pointing at whatever those ids had become. A game written in Rust that builds its world in code
//! and streams it in and out under ▶ — `dunya` despawns and spawns hundreds of entities a minute —
//! could not be played in the editor and come back to what was being edited. Unreal's
//! Play-In-Editor works this way, for the same reason.
//!
//! What ▶ does ([`start_play`]):
//!
//! 1. The edited scene — every entity but the editor's own furniture — is taken as a scene and
//!    spawned into a new world, with the components the scene registry knows (a game's own among
//!    them, `register_editor_component`).
//! 2. The edited world's resources are **moved** there — the input, the clock, the asset caches,
//!    the editor's own state — except the two a play session starts afresh: the physics world (a
//!    new one, with the edited one's gravity) and the script engine (a new Lua state).
//! 3. The editor camera and the grid are copied over, so the Scene view goes on working; the
//!    editor's selection, undo history and clipboard are set aside
//!    ([`EditorState::enter_play`]), since their entity ids mean nothing in the new world.
//! 4. The two worlds are swapped: the app's world is the play world from now on, the edited one
//!    waits in the [`PlaySession`].
//! 5. A game hooked into the studio ([`crate::StudioGame`]) has its setup run on the play world by
//!    the next render hook (it needs the renderer), and from then on its own frame instead of
//!    `PlayLoop`'s — the frame its exported binary runs.
//!
//! ⏹ ([`stop_play`]) moves the same resources back, swaps the worlds again and drops the play
//! world, then gives the editor back its selection and history.

use crate::state::StudioState;
use gizmo::core::component::EntityName;
use gizmo::editor::EditorState;
use gizmo::prelude::*;
use std::any::TypeId;

/// A play session: the edited world, set aside while the play world runs — see the [module](self).
pub struct PlaySession {
    /// The edited world, as ▶ left it.
    edited: World,
    /// The resource types moved from it into the play world, to move back.
    moved: Vec<TypeId>,
    /// The edited world's editor and game cameras.
    editor_camera: Entity,
    game_camera: Entity,
    /// The editor's selection, history and clipboard of the edited world.
    kept: Option<gizmo::editor::editor_state::EditedWorldState>,
}

/// In the play world until the next render hook has run the game's setup on it (the setup needs the
/// renderer, which the update hook does not have). The game's frame waits for it.
pub struct GameSetupPending;

/// The resource types a play session starts afresh rather than borrowing from the edited world.
fn starts_afresh(type_id: TypeId) -> bool {
    type_id == TypeId::of::<gizmo::physics::world::PhysicsWorld>() || type_id == TypeId::of::<gizmo::scripting::ScriptEngine>()
}

/// ▶ — builds the play world and swaps it in. See the [module](self).
pub fn start_play(world: &mut World, state: &mut StudioState) {
    let registry = gizmo::scene_registry_for(world);
    let protected = crate::render::collect_protected_ids(world, state.editor_camera);
    let scene_entities: Vec<Entity> = world.iter_alive_entities().into_iter().filter(|e| !protected.contains(e)).collect();
    let scene = gizmo::scene::SceneData::of_entities(world, scene_entities, &registry);

    let mut play = World::new();
    let ids = scene.instantiate(&mut play, &registry);
    let moved = world.move_resources_to(&mut play, |t| !starts_afresh(t));
    let gravity = world
        .get_resource::<gizmo::physics::world::PhysicsWorld>()
        .map_or(Vec3::new(0.0, -9.81, 0.0), |p| p.integrator.gravity);
    play.insert_resource(gizmo::physics::world::PhysicsWorld::new().with_gravity(gravity));
    if let Ok(engine) = gizmo::scripting::ScriptEngine::new() {
        play.insert_resource(engine);
    }

    let editor_camera = copy_furniture(world, &mut play, state.editor_camera);
    if let Some(grid) = find_named(world, "Editor Grid") {
        copy_furniture(world, &mut play, grid);
    }
    let game_camera = ids.get(&state.game_camera.id()).copied().unwrap_or(state.game_camera);

    let kept = play.get_resource_mut::<EditorState>().map(|mut ed| ed.enter_play());
    if state.game.is_some() {
        play.insert_resource(GameSetupPending);
    }

    std::mem::swap(world, &mut play);
    state.session = Some(PlaySession {
        edited: play,
        moved,
        editor_camera: state.editor_camera,
        game_camera: state.game_camera,
        kept,
    });
    state.editor_camera = editor_camera;
    state.game_camera = game_camera;
}

/// ⏹ — puts the edited world back as ▶ left it and drops the play world. See the [module](self).
pub fn stop_play(world: &mut World, state: &mut StudioState) {
    let Some(mut session) = state.session.take() else { return };
    world.move_resources_to(&mut session.edited, |t| session.moved.contains(&t));
    std::mem::swap(world, &mut session.edited);
    if let (Some(kept), Some(mut ed)) = (session.kept.take(), world.get_resource_mut::<EditorState>()) {
        ed.leave_play(kept);
    }
    state.editor_camera = session.editor_camera;
    state.game_camera = session.game_camera;
    // `session.edited` is the play world now, and goes with the session.
}

/// The camera a hooked game draws its picture with, in the play world: its primary one — not the
/// editor's camera, which is primary too and was copied over, and not one with a viewport (a
/// minimap) or a texture of its own. `None` until the game has made one.
pub fn game_camera(world: &World, editor_camera: Entity) -> Option<Entity> {
    use gizmo::renderer::components::{Camera, CameraTarget};
    let cameras = world.query::<(Entity, &Camera)>();
    let targets = world.query::<&CameraTarget>();
    cameras
        .iter()
        .find(|(e, c)| *e != editor_camera && c.primary && c.viewport.is_none() && targets.get(*e).is_none())
        .map(|(e, _)| e)
}

/// The first entity called `name`.
fn find_named(world: &World, name: &str) -> Option<Entity> {
    let names = world.query::<&EntityName>();
    world.iter_alive_entities().into_iter().find(|e| names.get(*e).is_some_and(|n| n.0 == name))
}

/// Copies a piece of the editor's furniture (its camera, its grid) into the play world: the
/// components it is drawn and placed with. Returns the copy.
fn copy_furniture(from: &World, to: &mut World, entity: Entity) -> Entity {
    let copy = to.spawn();
    macro_rules! copy {
        ($($ty:ty),*) => {$(
            if let Some(c) = from.get::<$ty>(entity) {
                to.add_component(copy, (*c).clone());
            }
        )*};
    }
    copy!(
        EntityName,
        gizmo::physics::components::Transform,
        gizmo::physics::components::GlobalTransform,
        gizmo::renderer::components::Camera,
        gizmo::renderer::components::Mesh,
        gizmo::renderer::components::Material,
        gizmo::renderer::components::MeshRenderer,
        gizmo::core::component::EditorOnly
    );
    copy
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo::physics::components::Transform;

    fn studio_state(editor_camera: Entity, game_camera: Entity) -> StudioState {
        StudioState {
            current_fps: 0.0,
            actual_dt: 0.016,
            editor_camera,
            game_camera,
            do_raycast: false,
            play: gizmo::systems::PlayLoop::new(),
            asset_watcher: None,
            gc_timer: 0.0,
            autosave_timer: 0.0,
            visible_entity_count: 0,
            draw_call_count: 0,
            session: None,
            game: None,
        }
    }

    /// **⏹ comes back to the edited world exactly as ▶ left it.** A scene of three named crates, the
    /// editor's camera, a selection, a resource of the host's: ▶ swaps in a world built from the
    /// scene, with the host's resources and an empty selection; under ▶ every crate is despawned,
    /// new ones spawned and the gravity turned upside down; ⏹ returns the same ids with the same
    /// components, the same gravity, the selection and the resource. With the old snapshot, a
    /// despawned crate came back with another id, if at all whole, and a joint or a gravity changed
    /// under ▶ stayed changed.
    #[test]
    fn stop_returns_to_the_edited_world_untouched() {
        let mut world = World::new();
        world.insert_resource(EditorState::new());
        world.insert_resource(gizmo::physics::world::PhysicsWorld::new().with_gravity(Vec3::new(0.0, -3.0, 0.0)));
        world.insert_resource(String::from("the host's"));
        let crates: Vec<Entity> = (0..3)
            .map(|i| {
                let e = world.spawn();
                world.add_component(e, EntityName(format!("Crate {i}")));
                world.add_component(e, Transform::new(Vec3::new(i as f32, 0.0, 0.0)));
                e
            })
            .collect();
        let editor_camera = world.spawn();
        world.add_component(editor_camera, EntityName("Editor Camera".into()));
        world.add_component(editor_camera, Transform::new(Vec3::new(0.0, 8.0, 18.0)));
        if let Some(mut ed) = world.get_resource_mut::<EditorState>() {
            ed.selection.entities.insert(crates[1]);
            ed.selection.primary = Some(crates[1]);
        }
        let mut state = studio_state(editor_camera, crates[0]);

        start_play(&mut world, &mut state);
        assert!(state.session.is_some());
        let names = |w: &World| {
            let q = w.query::<&EntityName>();
            let mut v: Vec<String> = w.iter_alive_entities().into_iter().filter_map(|e| q.get(e).map(|n| n.0.clone())).collect();
            v.sort();
            v
        };
        assert_eq!(names(&world), vec!["Crate 0", "Crate 1", "Crate 2", "Editor Camera"], "the scene, and a copy of the editor's camera");
        assert_eq!(world.get_resource::<String>().map(|s| s.clone()).as_deref(), Some("the host's"), "the host's resources came along");
        assert!(world.get_resource::<EditorState>().is_some_and(|ed| ed.selection.entities.is_empty()), "nothing selected in the play world");
        assert_eq!(world.get::<EntityName>(state.game_camera).map(|n| n.0.clone()).as_deref(), Some("Crate 0"), "the game camera is its copy there");

        // Under ▶: everything goes, other things come, the gravity turns.
        for e in world.iter_alive_entities() {
            world.despawn(e);
        }
        for _ in 0..5 {
            let e = world.spawn();
            world.add_component(e, EntityName("Debris".into()));
        }
        if let Some(mut p) = world.get_resource_mut::<gizmo::physics::world::PhysicsWorld>() {
            p.integrator.gravity = Vec3::new(0.0, 9.81, 0.0);
        }

        stop_play(&mut world, &mut state);
        assert!(state.session.is_none());
        assert_eq!((state.editor_camera, state.game_camera), (editor_camera, crates[0]));
        for (i, e) in crates.iter().enumerate() {
            assert_eq!(world.get::<EntityName>(*e).map(|n| n.0.clone()), Some(format!("Crate {i}")), "the same id, the same crate");
            assert_eq!(world.get::<Transform>(*e).map(|t| t.position.x), Some(i as f32));
        }
        assert_eq!(names(&world).len(), 4, "no debris");
        assert_eq!(
            world.get_resource::<gizmo::physics::world::PhysicsWorld>().map(|p| p.integrator.gravity),
            Some(Vec3::new(0.0, -3.0, 0.0)),
            "the edited world's own physics, untouched"
        );
        assert_eq!(world.get_resource::<String>().map(|s| s.clone()).as_deref(), Some("the host's"));
        assert!(world.get_resource::<EditorState>().is_some_and(|ed| ed.selection.primary == Some(crates[1])), "the selection is back");
    }
}
