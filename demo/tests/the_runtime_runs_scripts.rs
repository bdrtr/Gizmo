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

/// A script that drives whatever vehicle it is attached to.
const DRIVER: &str = r#"
function on_entity_update(id, dt, props)
    vehicle.set_engine_force(id, -0.6)
    vehicle.set_steering(id, 0.5)
    vehicle.set_brake(id, 0.25)
end
"#;

/// A script that asks for a sound. The whole Lua audio API is three calls, and this is the one a
/// game makes most.
const NOISY: &str = r#"
function on_entity_update(id, dt, props)
    audio.play("beep")
end
"#;

/// A script that freezes its fighter for three frames, once, on the first frame it runs.
const FREEZER: &str = r#"
frames = 0
function on_entity_update(id, dt, props)
    frames = frames + 1
    if frames == 1 then
        fighter.apply_hitstop(id, 3)
    end
end
"#;

/// A script that drives an animation, then **reads back which clip is playing** and switches on
/// what it saw. Write, read, write — the loop that proves both halves of the API.
const DANCER: &str = r#"
frames = 0
function on_entity_update(id, dt, props)
    frames = frames + 1
    if frames == 1 then
        animation.play(id, "run", 0.1, true)
    elseif frames == 2 then
        animation.set_speed(id, 2.0)
    elseif animation.is_playing(id, "run") then
        animation.play(id, "idle", 0.0, true)
    end
end
"#;

/// A script that throws a jab and then **spends the hits the engine reports**: it reads
/// `fighter.hits()`, takes the damage off the victim's health and stuns them. This is the whole
/// contract of the event-based design — the engine resolves the hit, the game decides the cost.
const BRAWLER: &str = r#"
frames = 0
function on_entity_update(id, dt, props)
    frames = frames + 1
    if frames == 1 then
        fighter.set_move(id, "Jab", 2, 2, 1, 8.0, 20, 5)
    end
    for _, hit in ipairs(fighter.hits()) do
        local victim = fighter.state(hit.victim)
        if victim ~= nil then
            fighter.set_health(hit.victim, victim.health - hit.damage)
            fighter.apply_hitstun(hit.victim, hit.hitstun)
        end
    end
end
"#;

/// A script that throws one 5/3/2 jab, once, on the first frame it runs.
const JABBER: &str = r#"
frames = 0
function on_entity_update(id, dt, props)
    frames = frames + 1
    if frames == 1 then
        fighter.set_move(id, "jab", 5, 3, 2, 8.0)
    end
end
"#;

/// A script that throws a jab and then **watches its own move**: it counts the frames on which the
/// engine says it is hitting, and remembers the highest move frame it ever saw. Both numbers are
/// reported through the only channel a script has that a test can read — its position.
const WATCHER: &str = r#"
frames = 0
hits = 0
last_frame = 0
stun = 0
function on_entity_update(id, dt, props)
    frames = frames + 1
    if frames == 1 then
        -- 30 frames of stun and 7 of freeze: this move's own numbers, not the engine's defaults.
        fighter.set_move(id, "jab", 5, 3, 2, 8.0, 30, 7)
    elseif fighter.is_attacking(id) then
        hits = hits + 1
    end
    local f = fighter.move_frame(id)
    if f ~= nil then last_frame = f end
    local s = fighter.state(id)
    if s ~= nil and s.move ~= nil then stun = s.move.hitstun_on_hit end
    entity.set_position(id, hits, last_frame, stun)
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

/// Returns the entity handle, not just its id: a caller that wants to add another component —
/// a vehicle, say — needs the handle, and reconstructing one from an id means guessing a
/// generation.
fn world_with_scripted_entity(script_path: &str) -> (World, gizmo::core::Entity) {
    let mut world = World::new();
    world.insert_resource(gizmo::physics::world::PhysicsWorld::new());
    world.insert_resource(gizmo::scripting::ScriptEngine::new().expect("Lua VM"));

    let e = world.spawn();
    world.add_component(e, EntityName::new("Oyuncu"));
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, gizmo::scripting::Script::new(script_path));
    (world, e)
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
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();
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

    let (mut world, entity) = world_with_scripted_entity(&missing);
    let id = entity.id();
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
    let (mut world, _entity) = world_with_scripted_entity(&script.path());

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

/// **A script can drive a car, through the whole chain.** No device, so this one runs on CI.
///
/// The unit tests for the handler call it directly; this drives the path a game actually takes —
/// Lua → command queue → `flush_commands` → *its return value* → `PlayLoop` → the component. That
/// return value is the link that was missing, and a test that skips it would not have noticed.
///
/// It also pins the reverse mapping at the far end of the chain: `set_engine_force(id, -0.6)` is
/// documented as "negative drives it backwards", while `VehicleController::throttle_input` is
/// documented as ignoring the sign — so the correct answer is reverse engaged and 0.6 of throttle,
/// and the tempting one (assign −0.6 straight through) is full speed forwards.
#[test]
fn a_script_drives_a_car_through_the_whole_chain() {
    let script = TempScript::new("driver", DRIVER);
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();
    world.add_component(entity, gizmo::physics::vehicle::VehicleController::new());

    let input = Input::default();
    let mut play = PlayLoop::new();
    play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});

    let vehicles = world.borrow::<gizmo::physics::vehicle::VehicleController>();
    let car = vehicles.get(id).expect("the car is still there");
    assert_eq!(car.steering_input, 0.5, "steering must reach the controller");
    assert_eq!(car.brake_input, 0.25, "and so must the brake");
    assert!(car.reverse_input, "a negative engine force is reverse, not forwards");
    assert_eq!(car.throttle_input, 0.6, "and its magnitude is the throttle");
}

/// Reads a fighter's `(is_locked, is_in_active_window, has_a_move)` back out of the world.
fn fighter_state(world: &World, id: u32) -> (bool, bool, bool) {
    let fighters = world.borrow::<gizmo::physics::components::FighterController>();
    let f = fighters.get(id).expect("the fighter is still there");
    (f.is_locked(), f.is_in_active_window(), f.active_move.is_some())
}

/// **A hitstop asked for from Lua ends.** Three frames must cost three frames.
///
/// It cost forever. `fighter.apply_hitstop` queues a command, `flush_commands` applies it to the
/// component, and `FighterController::is_locked` then answered `true` for the rest of the
/// process, because *nothing in the engine counted the frames down* — not a system, not the play
/// loop, not the studio. The component's own documentation said "the game (or a script) must tick
/// them once per fixed frame" and no game, script or system anywhere did. A script that used the
/// engine's own fighting API to add three frames of hit-freeze froze its fighter permanently.
///
/// The frames are counted where they are spent: `PlayLoop::physics_pass`, once per **fixed** step.
/// Each `step` below carries exactly one `FIXED_DT`, so one call is one frame.
#[test]
fn a_hitstop_from_a_script_ends_after_the_frames_it_asked_for() {
    let script = TempScript::new("freezer", FREEZER);
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();
    world.add_component(
        entity,
        gizmo::physics::components::FighterController::new(1),
    );

    let input = Input::default();
    let mut play = PlayLoop::new();

    let mut locked_after = Vec::new();
    for frame in 1..=5 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});
        if fighter_state(&world, id).0 {
            locked_after.push(frame);
        }
    }

    assert_eq!(
        locked_after,
        vec![1, 2],
        "three frames of hitstop must be spent over three frames and then be over — a fighter \
         still locked on frame 5 is the defect this test exists for (it used to be locked forever)"
    );
}

/// **A move started from Lua reaches its active window, and then ends.**
///
/// The other half of the same missing clock, and the half a fighting game is built on:
/// `is_in_active_window` — the function that says "this attack is hitting right now" — had never
/// once answered `true` in this engine, because `current_move_frame` was only ever *assigned*
/// (to 0, by `set_move`) and never advanced.
///
/// The numbers are the contract, not a smoke test: a 5/3/2 jab must hit on exactly three frames,
/// they must be the three that follow five frames of startup, and the fighter must be back to
/// neutral after all ten — recovery included, which is the phase nothing used to honour.
#[test]
fn a_move_from_a_script_reaches_its_active_window_and_ends() {
    let script = TempScript::new("jabber", JABBER);
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();
    world.add_component(
        entity,
        gizmo::physics::components::FighterController::new(1),
    );

    let input = Input::default();
    let mut play = PlayLoop::new();

    let mut hitting_on = Vec::new();
    let mut still_committed_after = Vec::new();
    for frame in 1..=12 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});
        let (_, in_window, has_move) = fighter_state(&world, id);
        if in_window {
            hitting_on.push(frame);
        }
        if has_move {
            still_committed_after.push(frame);
        }
    }

    assert_eq!(
        hitting_on,
        vec![5, 6, 7],
        "a 5/3/2 jab hits on the three frames after its five of startup — an empty list here is \
         the engine never advancing a move at all"
    );
    assert_eq!(
        still_committed_after,
        (1..=9).collect::<Vec<_>>(),
        "the move must occupy startup+active+recovery = 10 frames and be over on the tenth"
    );
}

/// **A script can watch the move it started.** The read side of the fighting API used to be one
/// boolean wide — `is_locked` — so a script could ask the engine to throw a jab and then had no
/// way to learn what frame it was on, whether it was hitting, or when it ended. Frame data exists
/// to be read on the frame it matters.
///
/// The script reports through its own position because that is the one channel a Lua script has
/// that a test can read: `x` counts the frames the engine said it was hitting, `y` is the highest
/// move frame it ever saw.
///
/// The numbers are the contract. `x == 3`: a 5/3/2 jab hits on exactly three frames, and the
/// script saw all three. `y == 9`: the last frame index it observed before the move ended, one
/// behind the tenth and final tick — because the mirror is taken at the top of the frame and the
/// clock is spent at the bottom of it, which is exactly the ordering a script reacting to its own
/// move needs.
#[test]
fn a_script_can_read_the_move_it_started() {
    let script = TempScript::new("watcher", WATCHER);
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();
    world.add_component(
        entity,
        gizmo::physics::components::FighterController::new(1),
    );

    let input = Input::default();
    let mut play = PlayLoop::new();
    for _ in 0..12 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});
    }

    let reported = position_of(&world, id);
    assert_eq!(
        reported.x, 3.0,
        "the script must see its own hitting window — all three frames of it. 0 here is a script \
         that cannot read the move it authored"
    );
    assert_eq!(
        reported.y, 9.0,
        "and it must be able to read the frame counter advancing, up to the frame before the \
         move ended"
    );
    assert_eq!(
        reported.z, 30.0,
        "the stun the script authored must survive the whole round trip — Lua argument, command, \
         component, mirror, Lua again. 20 here is `FrameData`'s default winning, which is what \
         every Lua-authored move used to get whatever it asked for"
    );
}

/// **A script fights: the engine reports the hit, the script spends it.**
///
/// The end of the chain, and every link is a thing that did not exist at the start of the day: the
/// fight clock advances the move, the active window drives the fist's hitbox, `hit_detection_system`
/// resolves the overlap and reports a `HitEvent`, `PlayLoop` rotates the queue so the event is
/// readable at all, the Lua mirror hands it to the script, and the script takes the health off
/// with `fighter.set_health`. Nothing in the engine subtracts it — that is the design.
///
/// The numbers are the contract: **92**, not 100 (the hit landed and was spent) and not 84 (one
/// hit per move, however many frames the window is open).
#[test]
fn a_script_spends_the_hits_the_engine_reports() {
    let script = TempScript::new("brawler", BRAWLER);
    let (mut world, attacker) = world_with_scripted_entity(&script.path());
    world.add_component(
        attacker,
        gizmo::physics::components::FighterController::new(1),
    );

    // The attacker's fist, parented to it: hit detection composes the pose through the link.
    let fist = world.spawn();
    world.add_component(fist, Transform::new(Vec3::new(0.0, 0.0, -0.8)));
    world.add_component(fist, gizmo::core::component::Parent(attacker.id()));
    world.add_component(
        fist,
        gizmo::physics::components::Hitbox::new(Vec3::splat(0.2), 99.0),
    );

    let defender = world.spawn();
    world.add_component(defender, Transform::new(Vec3::new(0.0, 0.0, -1.0)));
    world.add_component(
        defender,
        gizmo::physics::components::FighterController::new(2),
    );
    world.add_component(
        defender,
        gizmo::physics::components::Hurtbox::new(Vec3::new(0.3, 0.5, 0.3)),
    );

    let input = Input::default();
    let mut play = PlayLoop::new();
    for _ in 0..10 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});
    }

    let fighters = world.borrow::<gizmo::physics::components::FighterController>();
    let hit = fighters.get(defender.id()).expect("the defender is still there");
    assert_eq!(
        hit.health, 92.0,
        "the script must have spent exactly one 8-damage hit: 100 means the event never reached \
         it, 84 means the same hit was reported once per active frame"
    );
    assert!(
        hit.is_locked(),
        "and the stun the script applied from the event must be running"
    );
}

/// **A script can animate, and can see what it animated.**
///
/// `ScriptCommand::PlayAnimation` and `SetAnimationSpeed` had working handlers and no producer:
/// `flush_commands` has applied both to real `AnimationPlayer`s for as long as they have existed,
/// and nothing anywhere pushed either one. The engine could animate on request and had no way to
/// be asked — `gizmo-animation`'s own docs even name "the Lua `PlayAnimation` command" while
/// explaining a warning.
///
/// The script switches to `run`, sets the speed, then *reads back* that `run` is playing and
/// switches to `idle`. The final state proves all three: the clip changed twice (so the write
/// works), the speed stuck (so the second command works), and the second switch happened at all
/// (so the read works — with no mirror, `is_playing` is false forever and it never fires).
#[test]
fn a_script_can_drive_an_animation_and_read_it_back() {
    use gizmo::renderer::components::AnimationPlayer;

    fn clip(name: &str) -> gizmo::animation::skeletal::AnimationClip {
        gizmo::animation::skeletal::AnimationClip {
            name: name.to_string(),
            duration: 1.0,
            translations: Vec::new(),
            rotations: Vec::new(),
            scales: Vec::new(),
        }
    }

    let script = TempScript::new("dancer", DANCER);
    let (mut world, entity) = world_with_scripted_entity(&script.path());
    let id = entity.id();

    world.add_component(
        entity,
        AnimationPlayer {
            animations: std::sync::Arc::new([clip("idle"), clip("run")]),
            ..Default::default()
        },
    );

    let input = Input::default();
    let mut play = PlayLoop::new();
    for _ in 0..4 {
        play.step(&mut world, 1.0 / 60.0, &input, &mut |_| {});
    }

    let players = world.borrow::<AnimationPlayer>();
    let player = players.get(id).expect("the player is still there");
    assert_eq!(
        player.current_clip().map(|c| c.name.as_str()),
        Some("idle"),
        "the script read that `run` was playing and switched back — a script that cannot read \
         which clip is active never makes that second switch"
    );
    assert_eq!(
        player.prev_animation,
        Some(1),
        "and it came from `run`, so the first switch happened too"
    );
    assert_eq!(player.speed, 2.0, "the speed the script set must have landed");
}
