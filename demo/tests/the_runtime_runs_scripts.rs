//! Does a script attached to an entity actually run, and when does its effect land?
//!
//! The exported game's whole contract is "what Play mode does", and half of that is scripting —
//! but nothing anywhere drove a real `.lua` file through the loop. There is not one in the
//! repository: the scripting tests all write their source inline into a temp file, and the two
//! callers of [`gizmo::systems::PlayLoop`] were verified by rendering a scene, which needs no
//! script at all. This is the other half.
//!
//! It lives in `demo` because that is where the runtime binary lives and because this crate turns
//! the `scripting` feature on — the facade's default feature set does not, so a test placed there
//! would compile in the all-features lint job and never actually run.

use gizmo::core::component::EntityName;
use gizmo::prelude::*;
use gizmo::systems::PlayLoop;

/// A script that moves whatever entity it is attached to, on its per-entity hook.
const MOVER: &str = r#"
function on_entity_update(id, dt, props)
    entity.set_position(id, 5.0, 0.0, 0.0)
end
"#;

/// A script that asks for a sound. The whole Lua audio API is three calls, and this is the one a
/// game makes most.
const NOISY: &str = r#"
function on_entity_update(id, dt, props)
    audio.play("beep")
end
"#;

struct TempScript(std::path::PathBuf);

impl TempScript {
    fn new(name: &str, source: &str) -> Self {
        let path = std::env::temp_dir().join(format!("gizmo_{}_{}.lua", name, std::process::id()));
        std::fs::write(&path, source).expect("script dosyası yazılamadı");
        Self(path)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().to_string()
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn world_with_scripted_entity(script_path: &str) -> (World, u32) {
    let mut world = World::new();
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
    world.insert_resource(gizmo::scripting::ScriptEngine::new().expect("Lua VM"));

    let e = world.spawn();
    world.add_component(e, EntityName::new("Oyuncu"));
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, gizmo::scripting::Script::new(script_path));
    (world, e.id())
}

fn position_of(world: &World, id: u32) -> Vec3 {
    world
        .borrow::<Transform>()
        .get(id)
        .map(|t| t.position)
        .expect("entity has a Transform")
}

/// **A script attached to an entity moves it, and it does so in the frame it asked to.**
///
/// The second half is the one that had to be measured rather than assumed. The loop ran
/// `update` → `flush_commands` → per-entity `on_entity_update`, and `entity.set_position` does not
/// write the world, it queues a command — so everything a per-entity script asked for waited for
/// the *next* frame's flush. Measured before the fix: the entity was still at the origin after one
/// frame and only moved on the second. That is a frame of latency on every per-entity script in
/// the engine, in the editor and in every exported game, and nothing was watching for it because
/// the movement did happen — just late.
#[test]
fn a_scripted_entity_moves_in_the_frame_its_script_asked() {
    let script = TempScript::new("mover", MOVER);
    let (mut world, id) = world_with_scripted_entity(&script.path());
    let input = Input::default();
    let mut play = PlayLoop::new();

    assert_eq!(position_of(&world, id), Vec3::ZERO, "başlangıç");

    play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});

    assert_eq!(
        position_of(&world, id),
        Vec3::new(5.0, 0.0, 0.0),
        "the script ran on this frame and its command must land on this frame; a queued command \
         that waits for the next flush is a frame of input latency"
    );
}

/// **The whole chain an exported game walks:** a scene file on disk → a world → a script that
/// runs and moves something.
///
/// Each link had a test and the chain had none, which is the arrangement that let the export ship
/// a binary that read no scene for as long as it did. `Script` in particular is registered by
/// `full_scene_registry` and by nothing below it — `gizmo-scene` cannot see the scripting crate —
/// so "the scene saved from the editor still has its scripts when the game opens it" is a claim
/// about the registry the runtime uses, and this drives that registry.
#[test]
fn a_scene_saved_with_a_script_loads_and_that_script_runs() {
    let script = TempScript::new("chain", MOVER);
    let (source_world, _) = world_with_scripted_entity(&script.path());

    let scene = std::env::temp_dir().join(format!("gizmo_chain_{}.scene", std::process::id()));
    let scene_path = scene.to_string_lossy().to_string();
    let registry = gizmo::full_scene_registry();
    gizmo::scene::SceneData::save(&source_world, &scene_path, &registry).expect("sahne kaydı");

    // A fresh world, as a launched game has: nothing but what the file carries.
    let mut world = World::new();
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
    world.insert_resource(gizmo::scripting::ScriptEngine::new().expect("Lua VM"));
    gizmo::scene::SceneData::load_into(&scene_path, &mut world, &registry).expect("sahne yükleme");
    let _ = std::fs::remove_file(&scene);

    let id = {
        let names = world.borrow::<EntityName>();
        names
            .iter()
            .find(|(_, n)| n.0 == "Oyuncu")
            .map(|(id, _)| id)
            .expect("the entity survived the file")
    };
    assert!(
        world.borrow::<gizmo::scripting::Script>().get(id).is_some(),
        "the Script component did not survive the scene file — an exported game would open a \
         world that looks right and does nothing"
    );
    assert_eq!(position_of(&world, id), Vec3::ZERO, "yüklendiği yer");

    PlayLoop::new().step(&mut world, 1.0 / 60.0, &Input::default(), &mut |_| {});

    assert_eq!(
        position_of(&world, id),
        Vec3::new(5.0, 0.0, 0.0),
        "the script that came out of the scene file did not run"
    );
}

/// A script whose file cannot be read is reported once — and the entity keeps its position rather
/// than the frame dying.
#[test]
fn a_missing_script_is_reported_once_and_does_not_stop_the_frame() {
    let missing = std::env::temp_dir()
        .join(format!("gizmo_absent_{}.lua", std::process::id()))
        .to_string_lossy()
        .to_string();
    let _ = std::fs::remove_file(&missing);

    let (mut world, id) = world_with_scripted_entity(&missing);
    let input = Input::default();
    let mut play = PlayLoop::new();

    let mut breaks = 0;
    for _ in 0..30 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |report| {
            if matches!(report, gizmo::systems::PlayReport::ScriptBroke { .. }) {
                breaks += 1;
            }
        });
    }

    assert_eq!(
        breaks, 1,
        "thirty frames of a missing script must produce one message, not thirty"
    );
    assert_eq!(
        position_of(&world, id),
        Vec3::ZERO,
        "a broken script must not move anything, and must not take the frame down with it"
    );
}

/// **A script can make a sound.** On real hardware, because silence is exactly the symptom.
///
/// `audio.play` queues a `ScriptCommand::PlaySound`, and `ScriptEngine::flush_commands` cannot
/// apply it — the scripting crate does not depend on the audio subsystem — so it *returns* it to
/// the host. Both call sites in `PlayLoop` discarded that return value (`let _unhandled = …`) and
/// no other consumer existed in the workspace. So the Lua audio API was complete except for the
/// end: the call worked, the command was queued, `api_audio.rs`'s unit test asserted the queueing,
/// and nothing anywhere ever played a sound. Found 2026-08-18 by following the return value.
///
/// The assertion goes through `stop_by_name` because that is the only question a script's
/// vocabulary can ask: it stops every live sink started from that name and reports how many, so a
/// `1` here is "the sound the script asked for is playing".
///
/// `#[ignore]` for the usual reason: CI runners have no sound card, where `AudioManager::new`
/// legitimately fails. `cargo test -p demo --test the_runtime_runs_scripts -- --ignored`
#[test]
#[ignore = "needs a real audio output device — run with --ignored"]
fn a_script_can_make_a_sound() {
    let script = TempScript::new("noisy", NOISY);
    let (mut world, _id) = world_with_scripted_entity(&script.path());

    let mut audio = match AudioManager::new() {
        Ok(a) => a,
        Err(e) => panic!("no audio output device on this machine: {e}"),
    };
    audio
        .load_sound("beep", concat!(env!("CARGO_MANIFEST_DIR"), "/assets/audio/engine.wav"))
        .expect("the demo's own engine.wav must be loadable");
    world.insert_resource(audio);

    let input = Input::default();
    let mut play = PlayLoop::new();
    play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});

    let mut audio = world
        .get_resource_mut::<AudioManager>()
        .expect("the manager is still a resource");
    assert_eq!(
        audio.stop_by_name("beep"),
        1,
        "the script asked for a sound and nothing is playing — the host is dropping the audio \
         commands `flush_commands` hands back again"
    );
}
