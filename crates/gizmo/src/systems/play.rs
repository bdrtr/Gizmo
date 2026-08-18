//! One frame of a *running* game — the step the editor's ▶ Play takes, and the step an exported
//! game takes.
//!
//! **Why it is one function.** Studio's Build/Export ships `gizmo_runtime`, and that binary's
//! contract is "what Play mode does": it is how the export can promise that the game you shipped
//! behaves like the game you were just looking at. A contract written down in prose drifts the
//! first time one of the two sides is edited and nobody notices for a release. Written as a
//! shared function it cannot: there is one accumulator, one script order, one step size.
//!
//! What legitimately differs between the two is not the work but **who hears about it** — a
//! broken script is a red line in the editor's console and a line on stderr in a shipped game —
//! so the reporting is injected ([`PlayReport`]) and none of the decisions are.
//!
//! What is deliberately *not* here: hot-reload (an authoring tool, and the editor's own asset
//! watcher drives it), and the editor's default `ActionMap` scaffolding — bindings a game
//! actually needs belong in its scene. Both stay on the studio side. The fighter system that
//! scaffolding was written for is no longer commented out anywhere: the fight clock
//! (`fighter_frame_system`) runs here, on the fixed step, for both paths.

use crate::core::World;

/// The fixed simulation step: 60 Hz, and the value both paths have always used.
pub const FIXED_DT: f32 = 1.0 / 60.0;

/// Ceiling on the steps one frame may take **and** on the debt the accumulator may carry.
///
/// Both halves matter. Without the per-frame cap a slow frame runs the simulation forever;
/// without the cap on the debt itself, a stall that is not the simulation's fault — a breakpoint,
/// a window drag, a laptop lid — leaves the next frame owing hundreds of steps, which makes the
/// frame after that slower still. That is the death spiral, and it is why the accumulator is
/// clamped rather than merely drained.
pub const MAX_STEPS: u32 = 16;

/// Something the step wants a human to know.
///
/// Borrowed, not owned: a report is read and formatted by the caller inside the callback, and the
/// step should not allocate a `String` per frame for a console nobody may be watching.
#[derive(Debug)]
pub enum PlayReport<'a> {
    /// The shared script pass failed. The world is intact; this frame's script logic is not.
    ScriptError {
        /// What the shared pass reported.
        error: &'a str,
    },
    /// One entity's `on_update` failed. Others still ran.
    EntityScriptError {
        /// The entity whose script failed.
        entity: u32,
        /// The script file it was running.
        path: &'a str,
        /// What it reported.
        error: &'a str,
    },
    /// A script file has just *started* failing to load — reported on the edge, not every frame.
    ///
    /// The edge is the point: the editor stamps `scripts/new_script.lua`, a path nothing creates,
    /// so a script component pointing at a missing file is the common case. Announcing it per
    /// frame is sixty identical lines a second; announcing it never is what used to happen.
    ScriptBroke {
        /// The file that has started failing.
        path: &'a str,
        /// Why it will not load.
        error: &'a str,
    },
    /// A script that was failing now loads.
    ScriptRecovered {
        /// The file that loads again.
        path: &'a str,
    },
    /// A line the script itself printed, with the level it chose.
    ScriptLog {
        /// The level the script chose: `info`, `warn`, `error`.
        level: &'a str,
        /// The line itself.
        message: &'a str,
    },
}

/// The edge a script's load attempt just crossed, if any.
///
/// The decision needs nothing from Lua — a set and a `Result` — so it is compiled whenever its
/// tests are, which is what keeps them running in a default (scripting-off) build. It still has
/// to be gated: without `test` in this list a wasm build, where `scripting` is off, carries an
/// enum and a function nobody calls, and `-D warnings` is a CI gate. Measured the hard way.
#[cfg(any(feature = "scripting", test))]
#[derive(Debug, PartialEq, Eq)]
enum ReloadEdge {
    Broke(String),
    Recovered,
}

/// Answer everything `flush_commands` handed back that this side of the engine can reach.
///
/// The scripting crate depends on `gizmo-core`, the physics components and `gizmo-ai` — not on
/// audio, not on the renderer, not on the vehicle dynamics — so what it cannot apply it returns.
/// This is where those subsystems are, and each handler passes on what it does not speak for.
///
/// **What still goes unanswered, and why it is a list rather than a shrug.** Of `ScriptCommand`'s
/// 43 variants, 23 are applied inside the scripting crate and 20 came back here. This chain now
/// takes seven of those (three audio, three vehicle, the camera's field of view). The remaining
/// thirteen are scene load/save, the camera *follow* commands, dialogue, cutscenes and the race
/// subsystem — see docs/ENGINE.md's Scripting section, which carries the enumerated list and what
/// each one is waiting for.
#[cfg(feature = "scripting")]
fn apply_host_commands(
    world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    let rest = apply_script_audio(world, unhandled);
    let rest = apply_script_vehicle(world, rest);
    apply_script_camera(world, rest)
}

/// Drive a vehicle from Lua: `vehicle.set_engine_force`, `set_steering`, `set_brake`.
///
/// **Two unit traps here, and both were live.** `SetVehicleEngineForce`'s documentation says
/// "negative drives it backwards", while `VehicleController::throttle_input`'s says the opposite in
/// as many words: *"Only its magnitude is used, so a negative value is **not** reverse — use
/// `set_reverse` for that."* Assigning the command's value straight into the field would have
/// honoured neither: `vehicle.set_engine_force(id, -1)` would have driven the car **forwards**.
/// The mapping below keeps the Lua promise by engaging reverse, which is idempotent and safe to
/// call every frame from a held input.
///
/// Steering and brake do line up with their fields (−1..1 and 0..1), and are passed through
/// unclamped exactly as the controller's own documentation says it treats them.
#[cfg(all(feature = "scripting", feature = "physics-dynamics"))]
fn apply_script_vehicle(
    world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    use crate::scripting::ScriptCommand as Cmd;

    let mut rest = Vec::new();
    let mut vehicles = world.borrow_mut::<gizmo_physics_dynamics::VehicleController>();
    for cmd in unhandled {
        match cmd {
            Cmd::SetVehicleEngineForce(id, force) => {
                if let Some(mut vehicle) = vehicles.get_mut(id) {
                    vehicle.set_reverse(force < 0.0);
                    vehicle.throttle_input = force.abs();
                } else {
                    tracing::trace!(entity = id, "[Scripting] set_engine_force: hedefte VehicleController yok");
                }
            }
            Cmd::SetVehicleSteering(id, steering) => {
                if let Some(mut vehicle) = vehicles.get_mut(id) {
                    vehicle.steering_input = steering;
                } else {
                    tracing::trace!(entity = id, "[Scripting] set_steering: hedefte VehicleController yok");
                }
            }
            Cmd::SetVehicleBrake(id, brake) => {
                if let Some(mut vehicle) = vehicles.get_mut(id) {
                    vehicle.brake_input = brake;
                } else {
                    tracing::trace!(entity = id, "[Scripting] set_brake: hedefte VehicleController yok");
                }
            }
            other => rest.push(other),
        }
    }
    rest
}

/// Without the vehicle dynamics there is nothing to steer.
#[cfg(all(feature = "scripting", not(feature = "physics-dynamics")))]
fn apply_script_vehicle(
    _world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    unhandled
}

/// `camera.set_fov` from Lua, applied to the primary camera.
///
/// **The third unit trap of the afternoon.** `ScriptCommand::SetCameraFov`'s documentation says
/// *degrees*; `Camera::fov`'s says *radians*. A script asking for a 60° field of view would have
/// got 60 radians — and `Camera::new` clamps only the bottom (`fov.max(0.001)`), so nothing would
/// have caught it. This is the same shape as the keymap defect the scripting section already
/// records: two real units, not the same unit, and nothing comparing them.
///
/// The camera's *follow* commands (`SetCameraTarget`, `SetFightCamera`) are still handed back:
/// they ask for a behaviour over time, not a value, and the engine ships no follow system for a
/// script to point at. **Trigger:** one.
#[cfg(all(feature = "scripting", feature = "render"))]
fn apply_script_camera(
    world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    use crate::scripting::ScriptCommand as Cmd;

    let mut rest = Vec::new();
    for cmd in unhandled {
        match cmd {
            Cmd::SetCameraFov(degrees) => {
                let radians = degrees.to_radians().max(0.001);
                // The primary is found through a read borrow and written through a write one:
                // the mutable query is not iterable, and holding both at once is a panic.
                let primary = {
                    let cameras = world.borrow::<crate::renderer::Camera>();
                    cameras.iter().find(|(_, cam)| cam.primary).map(|(entity, _)| entity)
                };
                match primary {
                    Some(entity) => {
                        let mut cameras = world.borrow_mut::<crate::renderer::Camera>();
                        if let Some(mut cam) = cameras.get_mut(entity) {
                            cam.fov = radians;
                        }
                    }
                    None => tracing::trace!("[Scripting] set_fov: sahnede birincil kamera yok"),
                }
            }
            other => rest.push(other),
        }
    }
    rest
}

/// Without a renderer there is no camera to point.
#[cfg(all(feature = "scripting", not(feature = "render")))]
fn apply_script_camera(
    _world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    unhandled
}

/// One thing a script asked the audio subsystem for.
///
/// The scripting crate cannot name `AudioManager` — it depends on `gizmo-core` and the physics
/// components, not on audio — so what it can do is queue a command and hand it back. This is the
/// vocabulary on this side of that handover.
#[cfg(all(feature = "scripting", feature = "audio"))]
#[derive(Debug, PartialEq)]
enum AudioAction {
    /// `audio.play(name)`
    Play(String),
    /// `audio.play_3d(name, x, y, z)`
    Play3d(String, crate::math::Vec3),
    /// `audio.stop(name)`
    Stop(String),
}

/// Split what `flush_commands` handed back into audio actions and everything else.
///
/// Separated from the doing so the *decision* is testable without an audio device: the defect this
/// closes was not that the sound was wrong, it was that the command was never looked at.
#[cfg(all(feature = "scripting", feature = "audio"))]
fn split_audio_actions(
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> (Vec<AudioAction>, Vec<crate::scripting::ScriptCommand>) {
    use crate::scripting::ScriptCommand as Cmd;
    let mut actions = Vec::new();
    let mut rest = Vec::new();
    for cmd in unhandled {
        match cmd {
            Cmd::PlaySound(name) => actions.push(AudioAction::Play(name)),
            Cmd::PlaySound3D(name, pos) => actions.push(AudioAction::Play3d(name, pos)),
            Cmd::StopSound(name) => actions.push(AudioAction::Stop(name)),
            other => rest.push(other),
        }
    }
    (actions, rest)
}

/// The ears a script-placed 3D sound is measured against.
///
/// The same listener the spatial system uses, so a sound placed by `audio.play_3d` does not jump
/// on the frame after it starts.
#[cfg(all(feature = "scripting", feature = "audio", feature = "render", feature = "physics"))]
fn script_listener_ears(world: &World) -> ([f32; 3], [f32; 3]) {
    crate::systems::audio::listener(world).ears()
}

/// Without a renderer there is no camera to listen from, and without physics no `Transform` to
/// read one off. A game in that shape (a headless server with sound, a text game) still gets its
/// sound: it is placed at the origin, which is the only listening position such a world has.
#[cfg(all(
    feature = "scripting",
    feature = "audio",
    not(all(feature = "render", feature = "physics"))
))]
fn script_listener_ears(_world: &World) -> ([f32; 3], [f32; 3]) {
    ([-0.1, 0.0, 0.0], [0.1, 0.0, 0.0])
}

/// Answer the audio commands a script queued; hand back the ones this does not speak for.
///
/// **What this closes.** `ScriptEngine::flush_commands` applies everything the scripting crate can
/// reach and *returns* the rest. Both call sites in this file discarded that return value with
/// `let _unhandled = …`, and no other consumer existed anywhere in the workspace — so
/// `audio.play("jump")` from Lua queued a command, passed a unit test asserting the command was
/// queued, and made no sound in the editor's Play mode or in any exported game. The API existed
/// end to end except for the end.
///
/// Scene, dialogue, race and camera commands are still returned unhandled, and that is deliberate:
/// the editor must not switch scenes under the author.
#[cfg(all(feature = "scripting", feature = "audio"))]
fn apply_script_audio(
    world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    let (actions, rest) = split_audio_actions(unhandled);
    if actions.is_empty() {
        return rest;
    }

    // Read the listener before taking the manager: both borrow the world.
    let (left_ear, right_ear) = script_listener_ears(world);

    let Some(mut manager) = world.get_resource_mut::<gizmo_audio::AudioManager>() else {
        // A game built without an audio device — or one whose `AudioManager::new` failed, which is
        // a `Result` the demos handle by continuing silently. Say it at debug level: a script
        // asking for sound in a world with no audio is a configuration, not an error.
        tracing::debug!(
            count = actions.len(),
            "[Scripting] ses komutu geldi ama dünyada AudioManager yok"
        );
        return rest;
    };

    for action in actions {
        match action {
            AudioAction::Play(name) => {
                if let Err(e) = manager.play(&name) {
                    tracing::warn!(sound = %name, error = %e, "[Scripting] audio.play başarısız");
                }
            }
            AudioAction::Play3d(name, pos) => {
                if let Err(e) =
                    manager.play_3d(&name, [pos.x, pos.y, pos.z], left_ear, right_ear)
                {
                    tracing::warn!(sound = %name, error = %e, "[Scripting] audio.play_3d başarısız");
                }
            }
            AudioAction::Stop(name) => {
                let stopped = manager.stop_by_name(&name);
                tracing::trace!(sound = %name, stopped, "[Scripting] audio.stop");
            }
        }
    }
    rest
}

/// Without the `audio` feature there is nothing to hand them to, so they stay unhandled.
#[cfg(all(feature = "scripting", not(feature = "audio")))]
fn apply_script_audio(
    _world: &mut World,
    unhandled: Vec<crate::scripting::ScriptCommand>,
) -> Vec<crate::scripting::ScriptCommand> {
    unhandled
}

/// Decide what to say about one reload result, and remember the answer.
///
/// The studio used to run `let _ = engine.reload_if_changed(&path);` — result discarded — and the
/// only other call, `update_entity`, returns `Ok(())` for a script it never loaded. So a `Script`
/// component pointing at a file that is not there did nothing, forever, and said nothing; the
/// editor's own `➕ Bileşen Ekle ▸ Script` stamps `scripts/new_script.lua`, a path no part of the
/// editor creates. Reporting it naively is no better: this runs per entity per frame.
///
/// `failed` is the memory that turns a stream into two edges — the moment it breaks and the
/// moment it comes back, which are the two a person needs.
#[cfg(any(feature = "scripting", test))]
fn reload_edge(
    failed: &mut std::collections::BTreeSet<String>,
    path: &str,
    result: Result<bool, String>,
) -> Option<ReloadEdge> {
    match result {
        Ok(_) => failed.remove(path).then_some(ReloadEdge::Recovered),
        Err(e) => failed
            .insert(path.to_string())
            .then_some(ReloadEdge::Broke(e)),
    }
}

/// The state a play loop carries between frames.
#[derive(Debug, Default)]
pub struct PlayLoop {
    /// Simulation time owed, in seconds.
    accumulator: f32,
    /// Scripts that are currently failing to load — the memory that makes [`PlayReport::ScriptBroke`]
    /// an edge rather than a stream.
    #[cfg(feature = "scripting")]
    failed_scripts: std::collections::BTreeSet<String>,
}

impl PlayLoop {
    /// A loop with no accumulated debt and nothing recorded as broken.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the accumulated debt.
    ///
    /// The editor calls this while it is *not* playing: simulated time does not pass in a stopped
    /// game, and carrying a paused editor's debt into the next ▶ would spend it all in one frame.
    pub fn reset(&mut self) {
        self.accumulator = 0.0;
    }

    /// Seconds of simulation currently owed. Exposed for tests and for a profiler that wants to
    /// show the loop falling behind.
    pub fn debt(&self) -> f32 {
        self.accumulator
    }

    /// One frame of the running game: scripts first, then fixed-step physics. Returns how many
    /// physics steps ran, which is what a profiler wants and what a test can assert.
    ///
    /// Scripts before physics, because a script that sets a velocity this frame expects the
    /// solver to act on it this frame — the reverse order delays every input by a frame.
    pub fn step(
        &mut self,
        world: &mut World,
        dt: f32,
        input: &crate::core::input::Input,
        report: &mut dyn FnMut(PlayReport<'_>),
    ) -> u32 {
        #[cfg(feature = "scripting")]
        self.script_pass(world, dt, input, report);
        #[cfg(not(feature = "scripting"))]
        {
            let _ = (input, report);
        }
        self.physics_pass(world, dt)
    }

    /// The script half: the shared pass, the queued commands, then each entity's own update.
    #[cfg(feature = "scripting")]
    fn script_pass(
        &mut self,
        world: &mut World,
        dt: f32,
        input: &crate::core::input::Input,
        report: &mut dyn FnMut(PlayReport<'_>),
    ) {
        if world
            .try_get_resource::<crate::scripting::ScriptEngine>()
            .is_err()
        {
            // No Lua VM (it failed to start, or this game has no scripts). Physics still runs —
            // a game whose scripts cannot load is still a game that should draw and fall.
            return;
        }

        let failed = &mut self.failed_scripts;
        world.resource_scope(|world, engine: &mut crate::scripting::ScriptEngine| {
            if let Err(e) = engine.update(world, input, dt) {
                report(PlayReport::ScriptError { error: &e });
            }

            // Commands the shared pass queued, applied before the per-entity hooks run so those
            // hooks read a world that already reflects them.
            //
            // (`entity.set_position` and its neighbours do not write the world; they push a
            // command. So "flush" is not bookkeeping — it is when a script's decision becomes
            // true.) What `flush_commands` hands back is what the scripting crate cannot reach
            // from where it stands; the audio half is answered here, and scene switching is still
            // dropped on purpose — the editor must not switch scenes under the author.
            let unhandled = engine.flush_commands(world, dt);
            let _unhandled = apply_host_commands(world, unhandled);

            // Per-entity `on_update`. The entity's own property overrides ride along: scripts are
            // cached per path, so a per-entity value cannot live in the shared Lua environment.
            let mut entity_calls = Vec::new();
            {
                let scripts = world.borrow::<crate::scripting::Script>();
                for (entity_id, script) in scripts.iter() {
                    entity_calls.push((
                        entity_id,
                        script.file_path.clone(),
                        script.properties.clone(),
                    ));
                }
            }

            for (entity_id, path, properties) in entity_calls {
                match reload_edge(failed, &path, engine.reload_if_changed(&path)) {
                    Some(ReloadEdge::Broke(e)) => report(PlayReport::ScriptBroke {
                        path: &path,
                        error: &e,
                    }),
                    Some(ReloadEdge::Recovered) => {
                        report(PlayReport::ScriptRecovered { path: &path })
                    }
                    None => {}
                }
                if let Err(e) = engine.update_entity(entity_id, &path, dt, &properties) {
                    report(PlayReport::EntityScriptError {
                        entity: entity_id,
                        path: &path,
                        error: &e,
                    });
                }
            }

            // **The second flush, and the reason it exists.** Everything an `on_entity_update`
            // just asked for is sitting in the queue, and with only the flush above it would wait
            // for the *next* frame — measured: an entity whose script sets its position on frame
            // 1 was still at the origin at the end of frame 1 and moved at the end of frame 2.
            // That is a frame of latency on every per-entity script in the engine, in the editor
            // and in every exported game, and nothing caught it because the movement did happen.
            // Draining an empty queue is a `Vec` swap, so the frame that queued nothing pays
            // nothing.
            let unhandled = engine.flush_commands(world, dt);
            let _unhandled = apply_host_commands(world, unhandled);

            if let Ok(mut logs) = engine.log_queue.lock() {
                for (level, message) in logs.drain(..) {
                    report(PlayReport::ScriptLog {
                        level: &level,
                        message: &message,
                    });
                }
            }
        });
    }

    /// The physics half: spend the debt in fixed steps.
    ///
    /// The fight systems ride on the same steps, and this is the only place they can: a
    /// fighting move's timing is measured in **frames**, so the clock has to be spent by the same
    /// accumulator that spends the physics steps — not once per rendered frame, which would tie a
    /// jab's startup to the frame rate. Everything the fight subsystem promises (a hitstop that
    /// ends, a move that reaches its active window) hangs off this line, in the editor's ▶ and in
    /// every exported game alike.
    fn physics_pass(&mut self, world: &mut World, dt: f32) -> u32 {
        let (steps, remaining) = plan_steps(self.accumulator, dt);
        for _ in 0..steps {
            #[cfg(feature = "physics-dynamics")]
            {
                crate::physics::fighter_frame_system(world, FIXED_DT);
                // After the clock: the active window it resolves has to be this step's.
                crate::physics::hit_detection_system(world, FIXED_DT);
            }
            crate::physics::system::physics_step_system(world, FIXED_DT);
        }
        self.accumulator = remaining;
        pump_event_queues(world);
        steps
    }
}

/// End of frame for the event queues this loop's own systems fill.
///
/// `Events<T>` is double-buffered: `send` writes the current frame and `iter` reads the previous
/// one, and **something must call `update()` once a frame** or nothing ever becomes readable.
/// Nothing did on this path — `App::add_event` is the windowed runtime's pump and neither the
/// editor's ▶ nor an exported game goes through it. So `physics_step_system` had been sending
/// collision and trigger events into queues that never rotated, and the fight events would have
/// joined them.
///
/// The `HitEvent` queue is **created** if it is missing, the way
/// [`run_fixed_and_update`](gizmo_app::frame::run_fixed_and_update) creates a `PhysicsTime`: it is
/// the only one of the three with no other way in, and a fight whose hits are unreadable is not a
/// fight. The other two are only rotated **if the game asked for them**, and that asymmetry is
/// measured rather than tidy: `physics_step_system` clones every collision event it produced into
/// that queue, which in a scene like the 200-box tower is thousands of contact lists a frame, and
/// nobody should pay that for a resource they never asked for.
fn pump_event_queues(world: &mut World) {
    #[cfg(feature = "physics-dynamics")]
    {
        use gizmo_physics_core::components::HitEvent;
        if world
            .try_get_resource::<crate::core::event::Events<HitEvent>>()
            .is_err()
        {
            world.insert_resource(crate::core::event::Events::<HitEvent>::new());
        }
        if let Ok(mut hits) = world.try_get_resource_mut::<crate::core::event::Events<HitEvent>>() {
            hits.update();
        }
    }

    if let Ok(mut collisions) = world
        .try_get_resource_mut::<crate::core::event::Events<gizmo_physics_core::CollisionEvent>>()
    {
        collisions.update();
    }
    if let Ok(mut triggers) = world
        .try_get_resource_mut::<crate::core::event::Events<gizmo_physics_core::TriggerEvent>>()
    {
        triggers.update();
    }
}

/// How many fixed steps this frame runs, and the debt left over.
///
/// Split out from the loop that uses it so the rule is testable without a physics world — the
/// interesting cases are the ones a running game rarely reaches and a stalled one always does.
fn plan_steps(accumulator: f32, dt: f32) -> (u32, f32) {
    let mut debt = (accumulator + dt).min(FIXED_DT * MAX_STEPS as f32);
    let mut steps = 0;
    while debt >= FIXED_DT && steps < MAX_STEPS {
        debt -= FIXED_DT;
        steps += 1;
    }
    (steps, debt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The audio commands must be *recognised* — the half of the defect that has nothing to do
    /// with sound, and the only half that can be asserted without a device.
    ///
    /// What was broken was never the playing; it was that `flush_commands`' return value went
    /// into `let _unhandled` and no one ever looked at it. So this asserts the looking: the three
    /// audio commands leave the unhandled list, and everything else — scene switching, which is
    /// dropped on purpose — stays in it.
    #[cfg(all(feature = "scripting", feature = "audio"))]
    #[test]
    fn the_audio_commands_are_taken_out_of_the_unhandled_pile() {
        use crate::math::Vec3;
        use crate::scripting::ScriptCommand as Cmd;

        let (actions, rest) = split_audio_actions(vec![
            Cmd::PlaySound("jump".into()),
            Cmd::LoadScene("level2".into()),
            Cmd::PlaySound3D("explosion".into(), Vec3::new(1.0, 2.0, 3.0)),
            Cmd::StopSound("music".into()),
        ]);

        assert_eq!(
            actions,
            vec![
                AudioAction::Play("jump".into()),
                AudioAction::Play3d("explosion".into(), Vec3::new(1.0, 2.0, 3.0)),
                AudioAction::Stop("music".into()),
            ],
            "all three of the Lua audio API's calls must be answered"
        );
        assert_eq!(rest.len(), 1, "and the scene command must still be handed back, not eaten");
    }

    /// **A script can drive a car** — the second of the silent Lua APIs.
    ///
    /// `vehicle.set_engine_force` / `set_steering` / `set_brake` queued commands nobody applied,
    /// exactly like `audio.play`. Unlike audio these need no device, so the effect itself is
    /// assertable here.
    #[cfg(all(feature = "scripting", feature = "physics-dynamics"))]
    #[test]
    fn a_script_drives_the_vehicle_it_names() {
        use crate::scripting::ScriptCommand as Cmd;

        let mut world = World::new();
        let car = world.spawn();
        world.add_component(car, gizmo_physics_dynamics::VehicleController::new());
        let id = car.id();

        let rest = apply_script_vehicle(
            &mut world,
            vec![
                Cmd::SetVehicleEngineForce(id, 0.75),
                Cmd::SetVehicleSteering(id, -0.4),
                Cmd::SetVehicleBrake(id, 0.2),
                Cmd::LoadScene("level2".into()),
            ],
        );

        let vehicles = world.borrow::<gizmo_physics_dynamics::VehicleController>();
        let car = vehicles.get(id).expect("the car is still there");
        assert_eq!(car.throttle_input, 0.75);
        assert_eq!(car.steering_input, -0.4);
        assert_eq!(car.brake_input, 0.2);
        assert!(!car.reverse_input, "a positive force is not reverse");
        assert_eq!(rest.len(), 1, "the scene command is not the vehicle handler's to eat");
    }

    /// **Reverse is a gear, not a negative throttle** — and the two sides of this command said
    /// opposite things.
    ///
    /// `SetVehicleEngineForce`'s documentation: *"Negative drives it backwards."*
    /// `VehicleController::throttle_input`'s: *"Only its magnitude is used, so a negative value is
    /// **not** reverse."* Assigning the value straight through would have honoured neither —
    /// `vehicle.set_engine_force(id, -1)` would have driven the car **forwards** at full throttle,
    /// which is the worst possible reading of "backwards".
    #[cfg(all(feature = "scripting", feature = "physics-dynamics"))]
    #[test]
    fn a_negative_engine_force_engages_reverse_rather_than_driving_forwards() {
        use crate::scripting::ScriptCommand as Cmd;

        let mut world = World::new();
        let car = world.spawn();
        world.add_component(car, gizmo_physics_dynamics::VehicleController::new());
        let id = car.id();

        apply_script_vehicle(&mut world, vec![Cmd::SetVehicleEngineForce(id, -0.8)]);
        {
            let vehicles = world.borrow::<gizmo_physics_dynamics::VehicleController>();
            let car = vehicles.get(id).expect("the car");
            assert!(car.reverse_input, "a negative force must engage reverse");
            assert_eq!(car.throttle_input, 0.8, "and spend its magnitude as throttle");
            assert_eq!(car.current_gear, 0, "reverse is gear 0");
        }

        // And it must come back out again, or a script that taps reverse strands the car in it.
        apply_script_vehicle(&mut world, vec![Cmd::SetVehicleEngineForce(id, 0.5)]);
        let vehicles = world.borrow::<gizmo_physics_dynamics::VehicleController>();
        let car = vehicles.get(id).expect("the car");
        assert!(!car.reverse_input);
        assert_eq!(car.throttle_input, 0.5);
    }

    /// **The fov a script asks for is in degrees; the camera's is in radians.**
    ///
    /// `SetCameraFov`'s documentation says degrees and `Camera::fov`'s says radians, so a script
    /// asking for 60 would have got 60 *radians* — and `Camera::new` clamps only the bottom, so
    /// nothing downstream would have objected. Same shape as the keymap defect this crate already
    /// records: two real units, not the same unit, nothing comparing them.
    #[cfg(all(feature = "scripting", feature = "render"))]
    #[test]
    fn the_field_of_view_a_script_asks_for_is_converted_not_copied() {
        use crate::scripting::ScriptCommand as Cmd;

        let mut world = World::new();
        let eye = world.spawn();
        world.add_component(
            eye,
            crate::renderer::Camera::new(std::f32::consts::FRAC_PI_4, 0.1, 100.0, 0.0, 0.0, true),
        );

        apply_script_camera(&mut world, vec![Cmd::SetCameraFov(60.0)]);

        let cameras = world.borrow::<crate::renderer::Camera>();
        let fov = cameras.get(eye.id()).expect("the camera").fov;
        assert!(
            (fov - std::f32::consts::FRAC_PI_3).abs() < 1e-5,
            "60 degrees is {} radians, not {fov}",
            std::f32::consts::FRAC_PI_3
        );
    }

    #[test]
    fn a_frame_worth_of_time_buys_exactly_one_step() {
        let (steps, debt) = plan_steps(0.0, FIXED_DT);
        assert_eq!(steps, 1);
        assert!(debt < FIXED_DT, "a spent frame leaves no whole step behind");
    }

    #[test]
    fn a_short_frame_buys_nothing_and_keeps_its_change() {
        let (steps, debt) = plan_steps(0.0, FIXED_DT / 3.0);
        assert_eq!(steps, 0, "a 180 Hz frame must not step the solver three times a second late");
        assert!(
            (debt - FIXED_DT / 3.0).abs() < 1e-6,
            "the unspent time is owed, not dropped: {debt}"
        );
    }

    #[test]
    fn three_short_frames_add_up_to_one_step() {
        let (s1, d1) = plan_steps(0.0, FIXED_DT / 3.0);
        let (s2, d2) = plan_steps(d1, FIXED_DT / 3.0);
        let (s3, _) = plan_steps(d2, FIXED_DT / 3.0);
        assert_eq!((s1, s2, s3), (0, 0, 1));
    }

    /// The death-spiral guard, and the reason the cap is on the *debt* and not only on the steps.
    ///
    /// A ten-second stall — a breakpoint, a dragged window, a laptop lid — must not buy six
    /// hundred steps, and must not leave 590 of them owed to the next frame either. Capping only
    /// the per-frame count would do the first and not the second, and the loop would then run 16
    /// steps every frame for the next forty seconds while the game appeared to run in fast
    /// forward.
    #[test]
    fn a_long_stall_does_not_buy_hundreds_of_steps_or_leave_them_owed() {
        let (steps, debt) = plan_steps(0.0, 10.0);
        assert_eq!(steps, MAX_STEPS);
        assert!(
            debt < FIXED_DT,
            "the stall's remaining debt survived the cap: {debt}"
        );

        // And the frame after the stall is an ordinary frame again.
        let (next, _) = plan_steps(debt, FIXED_DT);
        assert_eq!(next, 1);
    }

    #[test]
    fn reset_forgets_the_debt() {
        let mut loop_state = PlayLoop::new();
        loop_state.accumulator = FIXED_DT * 4.0;
        loop_state.reset();
        assert_eq!(loop_state.debt(), 0.0);
    }

    /// A script that cannot load is reported once, and its recovery once.
    ///
    /// Both halves are defects that happened. Saying nothing is what the code did before anyone
    /// looked: the reload result was discarded and `update_entity` returns `Ok(())` for a script
    /// it never loaded, so an entity whose script pointed at a missing file did nothing and said
    /// nothing. Saying it every frame is the other failure — this runs per entity per frame, so
    /// an unconditional line is sixty copies a second on top of whatever the user is reading.
    #[test]
    fn a_broken_script_is_reported_once_and_its_recovery_too() {
        let mut failed = std::collections::BTreeSet::new();
        let path = "scripts/new_script.lua";

        assert_eq!(
            reload_edge(&mut failed, path, Err("Script okunamadı".into())),
            Some(ReloadEdge::Broke("Script okunamadı".into())),
            "the first failure has to reach the console; nothing else in the chain says a word"
        );

        for _ in 0..120 {
            assert_eq!(
                reload_edge(&mut failed, path, Err("Script okunamadı".into())),
                None,
                "a failure already on screen must not be printed again every frame"
            );
        }

        assert_eq!(
            reload_edge(&mut failed, path, Ok(true)),
            Some(ReloadEdge::Recovered),
            "coming back has to be reported too — otherwise the last word on screen is an error \
             about a script that is now running fine"
        );
        assert_eq!(reload_edge(&mut failed, path, Ok(false)), None);
        assert!(failed.is_empty(), "a recovered path must not stay in the set");
    }

    /// Two broken scripts are two reports, not one: the memory is keyed by path.
    #[test]
    fn each_script_is_tracked_on_its_own() {
        let mut failed = std::collections::BTreeSet::new();
        assert!(matches!(
            reload_edge(&mut failed, "a.lua", Err("yok".into())),
            Some(ReloadEdge::Broke(_))
        ));
        assert!(
            matches!(
                reload_edge(&mut failed, "b.lua", Err("yok".into())),
                Some(ReloadEdge::Broke(_))
            ),
            "a second broken script was swallowed because the first one had already reported"
        );
        assert_eq!(
            reload_edge(&mut failed, "a.lua", Ok(true)),
            Some(ReloadEdge::Recovered)
        );
        assert_eq!(failed.iter().collect::<Vec<_>>(), ["b.lua"]);
    }
}
