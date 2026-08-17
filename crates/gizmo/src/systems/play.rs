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
//! watcher drives it), and the editor's default `ActionMap` scaffolding, which exists for a
//! fighter system that is currently commented out. Both stay on the studio side.

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
            // true.) Unhandled ones — audio, scene switching — are dropped: the editor must not
            // switch scenes under the author, and the runtime has no consumer for them yet.
            let _unhandled = engine.flush_commands(world, dt);

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
            let _unhandled = engine.flush_commands(world, dt);

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
    fn physics_pass(&mut self, world: &World, dt: f32) -> u32 {
        let (steps, remaining) = plan_steps(self.accumulator, dt);
        for _ in 0..steps {
            crate::physics::system::physics_step_system(world, FIXED_DT);
        }
        self.accumulator = remaining;
        steps
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
