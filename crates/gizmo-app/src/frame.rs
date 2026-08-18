//! Per-frame simulation stepping: the fixed-timestep loop and the once-per-frame update.
//!
//! # Why there are two schedules
//!
//! Until 0.9 the windowed runtime had exactly one [`Schedule`], and it ran **only** inside
//! the fixed-timestep loop. That loop executes zero or more times per rendered frame,
//! depending on how much simulated time has accumulated — so with the renderer's default
//! `PresentMode::AutoNoVsync` pushing several hundred frames per second against a 60 Hz
//! accumulator, the *majority* of rendered frames ran no systems at all.
//!
//! That is fine for physics, which is what the loop is for. It is wrong for everything
//! else, and it broke input in a way that is hard to see and easy to blame on hardware:
//! `Input` is captured once per rendered frame and its edges (`is_key_just_pressed`,
//! `mouse_delta`) are cleared once per rendered frame, but were consumed 0..N times. On a
//! frame with no fixed step, a keypress was written and cleared without any system
//! observing it. Taps went missing; mouse motion arrived in fragments.
//!
//! So there are now two:
//!
//! - **fixed** — runs `0..N` times per frame at a constant `dt`. Physics, and anything
//!   that must be deterministic or frame-rate independent. This is the existing
//!   `App::schedule`; nothing moved out of it.
//! - **update** — runs **exactly once** per rendered frame with the real frame `dt`.
//!   Gameplay, cameras, UI, input edges. This is `App::update_schedule`, reached through
//!   [`App::add_update_system`](crate::windowed::App::add_update_system).
//!
//! The split is the same one Bevy draws between `Update` and `FixedUpdate`, and for the
//! same reason.
//!
//! # Ordering
//!
//! [`run_fixed_and_update`] runs the fixed steps first, then computes the interpolation
//! alpha, then runs update. Update therefore observes the simulation *after* this frame's
//! physics, and can read [`PhysicsTime::alpha`] — which is the fraction of a step that has
//! accumulated but not been simulated.
//!
//! **Nothing in the engine spends that alpha**, and the wording above used to imply otherwise
//! by promising "the last two simulated states": the engine keeps exactly one. There is no
//! previous `Transform` anywhere in the workspace, so the renderer draws the pose the last fixed
//! step left behind and holds it — measured at 144 Hz against a 60 Hz step, for two or three
//! frames in an irregular pattern. A game that wants interpolation snapshots its own poses each
//! fixed step and blends them with this alpha; see [`PhysicsTime::alpha`] for the numbers.

use gizmo_core::system::Schedule;
use gizmo_core::time::PhysicsTime;
use gizmo_core::world::World;

/// What [`run_fixed_and_update`] did this frame. Useful for tracing and for tests that
/// need to assert the loop actually behaved like a fixed-timestep accumulator.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct FrameSteps {
    /// How many times the fixed schedule ran. Zero is normal and expected on a frame
    /// shorter than the fixed step; a large number means the accumulator is catching up
    /// after a stall.
    pub fixed_steps: u32,
    /// The fixed timestep used, in seconds.
    pub fixed_dt: f32,
    /// Interpolation factor left in the accumulator after the last step, `0.0..1.0`:
    /// `render_pos = lerp(prev_physics_pos, curr_physics_pos, alpha)` — a lerp the engine does
    /// not perform, since it stores no `prev_physics_pos`. See [`PhysicsTime::alpha`].
    ///
    /// A **copy** of what [`PhysicsTime`] holds, for tracing and tests; the resource is the
    /// authority, and is where a game should read it from.
    pub alpha: f32,
}

/// Advance the world's [`Time`] by one frame of wall clock and return the **simulated** delta.
///
/// The return value is `Time::dt()` — the raw delta scaled by `time_scale` and then clamped to
/// `max_dt` — which is what a runtime should feed the fixed-timestep loop, while the update hook
/// keeps getting the raw delta so cameras and UI stay smooth through a pause. Creates the
/// resource if the world has none, the same way [`run_fixed_and_update`] creates a
/// [`PhysicsTime`].
///
/// **It exists because the two runtimes disagreed about whether `Time` exists at all.** The
/// windowed loop did this inline; the headless one never touched `Time` and handed the raw
/// wall-clock delta straight to the fixed loop. So a headless game had no `Res<Time>` — no
/// `dt()`, no `elapsed()`, no `frame()` — and `set_time_scale` did nothing there, including
/// `0.0`, which is supposed to be pause. That is the same defect the headless loop's own comment
/// records having fixed one layer up ("a plugin could not be written once and behave the same in
/// both"), left standing one layer down. One function now, called by both.
pub fn advance_time(world: &mut World, raw_dt: f32) -> f32 {
    if world.get_resource::<gizmo_core::time::Time>().is_some() {
        let mut time = world
            .get_resource_mut::<gizmo_core::time::Time>()
            .expect("just checked");
        time.update(raw_dt);
        time.dt()
    } else {
        let mut time = gizmo_core::time::Time::new();
        time.update(raw_dt);
        let sim_dt = time.dt();
        world.insert_resource(time);
        sim_dt
    }
}

/// Advance one rendered frame: drain the fixed-timestep accumulator, then run the
/// per-frame update schedule exactly once.
///
/// `sim_dt` is the *simulated* delta — `Time::dt()`, already scaled by `time_scale` and
/// clamped — and feeds the accumulator, so `set_time_scale(0.0)` genuinely stops physics.
/// `frame_dt` is the raw wall-clock frame delta handed to the update schedule, so cameras
/// and UI keep moving smoothly even while the simulation is paused.
///
/// `after_each_fixed_step` runs inside the loop after every fixed step, before the
/// accumulator is consumed. The windowed runtime uses it to snapshot rollback state; tests
/// and headless callers can pass `|_| {}`.
///
/// Inserts a default [`PhysicsTime`] if the world has none, so a caller that never
/// configured one still gets a working fixed step.
pub fn run_fixed_and_update<F>(
    world: &mut World,
    fixed_schedule: &mut Schedule,
    update_schedule: &mut Schedule,
    sim_dt: f32,
    frame_dt: f32,
    mut after_each_fixed_step: F,
) -> FrameSteps
where
    F: FnMut(&mut World),
{
    if world.get_resource::<PhysicsTime>().is_none() {
        world.insert_resource(PhysicsTime::default());
    }

    let fixed_dt = {
        let mut pt = world
            .get_resource_mut::<PhysicsTime>()
            .expect("PhysicsTime was just ensured");
        pt.accumulate(sim_dt);
        pt.fixed_dt()
    };

    let mut fixed_steps = 0u32;
    loop {
        let should = world
            .get_resource::<PhysicsTime>()
            .map(|pt| pt.should_step())
            .unwrap_or(false);
        if !should {
            break;
        }

        fixed_schedule.run(world, fixed_dt);
        after_each_fixed_step(world);

        world
            .get_resource_mut::<PhysicsTime>()
            .expect("PhysicsTime cannot vanish mid-loop")
            .consume_step();
        fixed_steps += 1;
    }

    let alpha = {
        let mut pt = world
            .get_resource_mut::<PhysicsTime>()
            .expect("PhysicsTime cannot vanish");
        pt.compute_alpha();
        pt.alpha()
    };

    // Exactly once, no matter how many (or how few) fixed steps ran above. This is the
    // line the whole module exists for.
    update_schedule.run(world, frame_dt);

    if fixed_steps > 0 {
        tracing::debug!(
            steps = fixed_steps,
            fixed_dt,
            alpha,
            "[frame] fixed-timestep physics steps"
        );
    }

    FrameSteps {
        fixed_steps,
        fixed_dt,
        alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A system that bumps a shared counter and records the `dt` it was handed.
    fn counting_system(
        runs: Arc<AtomicU32>,
        dts: Arc<std::sync::Mutex<Vec<f32>>>,
    ) -> impl FnMut(&World, f32) + Send + Sync + 'static {
        move |_world: &World, dt: f32| {
            runs.fetch_add(1, Ordering::Relaxed);
            dts.lock().unwrap().push(dt);
        }
    }

    struct Harness {
        world: World,
        fixed: Schedule,
        update: Schedule,
        fixed_runs: Arc<AtomicU32>,
        update_runs: Arc<AtomicU32>,
        fixed_dts: Arc<std::sync::Mutex<Vec<f32>>>,
        update_dts: Arc<std::sync::Mutex<Vec<f32>>>,
    }

    impl Harness {
        fn new(hz: u32) -> Self {
            let mut world = World::new();
            world.insert_resource(PhysicsTime::new(hz));

            let fixed_runs = Arc::new(AtomicU32::new(0));
            let update_runs = Arc::new(AtomicU32::new(0));
            let fixed_dts = Arc::new(std::sync::Mutex::new(Vec::new()));
            let update_dts = Arc::new(std::sync::Mutex::new(Vec::new()));

            let mut fixed = Schedule::new();
            fixed.add_system(counting_system(fixed_runs.clone(), fixed_dts.clone()));
            let mut update = Schedule::new();
            update.add_system(counting_system(update_runs.clone(), update_dts.clone()));

            Self {
                world,
                fixed,
                update,
                fixed_runs,
                update_runs,
                fixed_dts,
                update_dts,
            }
        }

        fn frame(&mut self, dt: f32) -> FrameSteps {
            run_fixed_and_update(
                &mut self.world,
                &mut self.fixed,
                &mut self.update,
                dt,
                dt,
                |_| {},
            )
        }
    }

    /// The regression this module exists for.
    ///
    /// At 600 fps against a 60 Hz accumulator, nine frames out of ten take no fixed step.
    /// Before the split, a system registered on the only schedule ran on just those tenth
    /// frames — and because `Input` is captured and cleared once per *rendered* frame, a
    /// keypress landing on any of the other nine was never observed by anything.
    #[test]
    fn update_runs_every_frame_even_when_no_fixed_step_does() {
        let mut h = Harness::new(60);
        let dt = 1.0 / 600.0; // ten rendered frames per fixed step

        for _ in 0..9 {
            let steps = h.frame(dt);
            assert_eq!(steps.fixed_steps, 0, "accumulator should not be full yet");
        }

        assert_eq!(
            h.fixed_runs.load(Ordering::Relaxed),
            0,
            "nine short frames must not advance the fixed schedule"
        );
        assert_eq!(
            h.update_runs.load(Ordering::Relaxed),
            9,
            "the update schedule must run on every rendered frame regardless"
        );

        // The tenth frame tips the accumulator over.
        let steps = h.frame(dt);
        assert_eq!(steps.fixed_steps, 1);
        assert_eq!(h.fixed_runs.load(Ordering::Relaxed), 1);
        assert_eq!(h.update_runs.load(Ordering::Relaxed), 10);
    }

    /// The mirror case: one long frame runs the fixed schedule several times, and update
    /// still exactly once. A system that assumed one-run-per-frame on the old single
    /// schedule was silently running N times here.
    #[test]
    fn update_runs_once_even_when_several_fixed_steps_do() {
        let mut h = Harness::new(60);

        let steps = h.frame(4.5 / 60.0); // four and a half fixed steps' worth
        assert_eq!(steps.fixed_steps, 4, "four whole steps fit");
        assert_eq!(h.fixed_runs.load(Ordering::Relaxed), 4);
        assert_eq!(
            h.update_runs.load(Ordering::Relaxed),
            1,
            "update is once per rendered frame, not once per simulation step"
        );
        assert!(
            (steps.alpha - 0.5).abs() < 1e-3,
            "half a step should remain in the accumulator, got alpha={}",
            steps.alpha
        );
    }

    /// The two schedules are handed different deltas on purpose: fixed gets the constant
    /// step, update gets the real frame time. Feeding update the fixed dt would make
    /// camera motion lurch whenever the step count changed.
    #[test]
    fn fixed_gets_the_constant_step_and_update_gets_the_frame_delta() {
        let mut h = Harness::new(60);
        let frame_dt = 2.5 / 60.0;
        h.frame(frame_dt);

        let fixed_dts = h.fixed_dts.lock().unwrap().clone();
        let update_dts = h.update_dts.lock().unwrap().clone();

        assert_eq!(fixed_dts.len(), 2);
        for d in &fixed_dts {
            assert!(
                (d - 1.0 / 60.0).abs() < 1e-6,
                "fixed schedule must always see the constant step, saw {d}"
            );
        }
        assert_eq!(update_dts.len(), 1);
        assert!(
            (update_dts[0] - frame_dt).abs() < 1e-6,
            "update schedule must see the real frame delta, saw {}",
            update_dts[0]
        );
    }

    /// A paused simulation (`time_scale = 0` upstream, which arrives here as `sim_dt = 0`)
    /// must still run update, or the camera and UI freeze along with the physics.
    #[test]
    fn a_paused_simulation_still_runs_update() {
        let mut h = Harness::new(60);
        let steps = run_fixed_and_update(
            &mut h.world,
            &mut h.fixed,
            &mut h.update,
            0.0,        // sim_dt — paused
            1.0 / 60.0, // frame_dt — wall clock keeps going
            |_| {},
        );
        assert_eq!(steps.fixed_steps, 0);
        assert_eq!(h.fixed_runs.load(Ordering::Relaxed), 0);
        assert_eq!(
            h.update_runs.load(Ordering::Relaxed),
            1,
            "pausing the simulation must not pause the frame"
        );
    }

    /// The per-step hook is what carries rollback snapshotting, so it must fire once per
    /// fixed step — not once per frame, and not at all when no step ran.
    ///
    /// Asserted against the reported step count rather than a literal, deliberately: at
    /// 60 Hz the fixed step is not representable in binary, so a `sim_dt` of exactly
    /// `3.0 / 60.0` yields 2.9999994 steps' worth and the loop runs twice. Hard-coding
    /// "3" here would be testing f32 rounding, not the contract.
    #[test]
    fn the_per_step_hook_fires_once_per_fixed_step() {
        let mut h = Harness::new(60);
        let hook_calls = Arc::new(AtomicU32::new(0));
        let hc = hook_calls.clone();

        let steps = run_fixed_and_update(
            &mut h.world,
            &mut h.fixed,
            &mut h.update,
            3.5 / 60.0,
            3.5 / 60.0,
            move |_| {
                hc.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert!(steps.fixed_steps >= 3, "expected several steps, got {}", steps.fixed_steps);
        assert_eq!(
            hook_calls.load(Ordering::Relaxed),
            steps.fixed_steps,
            "the hook must fire exactly as many times as the fixed schedule ran"
        );
        assert_eq!(
            h.fixed_runs.load(Ordering::Relaxed),
            steps.fixed_steps,
            "and the fixed schedule's own run count must agree"
        );
    }

    /// No fixed step means no hook call — rollback must not snapshot a frame in which the
    /// simulation did not advance.
    #[test]
    fn the_per_step_hook_does_not_fire_on_a_frame_with_no_step() {
        let mut h = Harness::new(60);
        let hook_calls = Arc::new(AtomicU32::new(0));
        let hc = hook_calls.clone();

        let steps = run_fixed_and_update(
            &mut h.world,
            &mut h.fixed,
            &mut h.update,
            1.0 / 600.0,
            1.0 / 600.0,
            move |_| {
                hc.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert_eq!(steps.fixed_steps, 0);
        assert_eq!(hook_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            h.update_runs.load(Ordering::Relaxed),
            1,
            "but update still ran"
        );
    }

    /// A world that never configured `PhysicsTime` still gets a working fixed step rather
    /// than silently never stepping.
    #[test]
    fn a_missing_physics_time_resource_is_created_rather_than_skipped() {
        let mut world = World::new();
        assert!(world.get_resource::<PhysicsTime>().is_none());

        let mut fixed = Schedule::new();
        let mut update = Schedule::new();
        let steps = run_fixed_and_update(&mut world, &mut fixed, &mut update, 1.0, 1.0, |_| {});

        assert!(world.get_resource::<PhysicsTime>().is_some());
        assert!(
            steps.fixed_steps > 0,
            "a full second of simulated time must produce fixed steps"
        );
    }

    /// **`advance_time` creates the clock if the world has none** — which is the whole of what the
    /// headless runtime was missing. A game there had no `Res<Time>` at all: no `dt()`, no
    /// `elapsed()`, no `frame()`.
    #[test]
    fn advance_time_creates_the_clock_and_returns_the_simulated_delta() {
        use gizmo_core::time::Time;

        let mut world = World::new();
        assert!(
            world.get_resource::<Time>().is_none(),
            "the fixture starts without one"
        );

        let sim_dt = advance_time(&mut world, 1.0 / 60.0);
        assert!(
            (sim_dt - 1.0 / 60.0).abs() < 1e-6,
            "an ordinary frame passes through untouched: {sim_dt}"
        );

        let time = world.get_resource::<Time>().expect("it was created");
        assert_eq!(time.frame(), 1, "and the frame counter started");
        assert!((time.dt() - sim_dt).abs() < 1e-9, "the resource agrees with the return value");
    }

    /// It keeps using the clock the game configured rather than replacing it, so `set_time_scale`
    /// and a custom `max_dt` survive from frame to frame.
    #[test]
    fn advance_time_honours_the_clock_the_game_configured() {
        use gizmo_core::time::Time;

        let mut world = World::new();
        let mut time = Time::new();
        time.set_time_scale(0.0); // pause
        world.insert_resource(time);

        let sim_dt = advance_time(&mut world, 1.0 / 60.0);
        assert_eq!(
            sim_dt, 0.0,
            "time_scale 0.0 is pause, and this is the value that feeds the fixed loop — it did \
             nothing at all on the headless path, where no Time existed to read"
        );

        {
            let mut time = world.get_resource_mut::<Time>().expect("still there");
            time.set_time_scale(0.5);
        }
        let half = advance_time(&mut world, 1.0 / 60.0);
        assert!(
            (half - (1.0 / 120.0)).abs() < 1e-6,
            "half speed halves the simulated delta: {half}"
        );

        let elapsed = world.get_resource::<Time>().expect("still there").elapsed();
        assert!(
            (elapsed - (1.0 / 120.0)).abs() < 1e-6,
            "and elapsed counts SIMULATED time, so the paused frame added nothing: {elapsed}"
        );
    }

    /// A stalled frame is clamped by `max_dt`, which is the ceiling that actually governs the
    /// fixed loop's catch-up — see `PhysicsTime`'s docs on the three of them.
    #[test]
    fn advance_time_clamps_a_stall() {
        use gizmo_core::time::Time;

        let mut world = World::new();
        let sim_dt = advance_time(&mut world, 1.0); // a one-second stall
        assert!(
            (sim_dt - 0.05).abs() < 1e-6,
            "clamped to the default max_dt of 50 ms, not passed through: {sim_dt}"
        );
        assert!(
            world.get_resource::<Time>().expect("created").raw_dt() > 0.9,
            "the raw delta is still readable — the clamp is on the simulated one"
        );
    }

    /// **Both runtimes take the simulated delta from the same place.** A source-shape guard,
    /// because neither loop can be driven from a test: the windowed one needs a window and the
    /// headless one never returns. Comments are cut first — this file's prose names the call.
    #[test]
    fn both_runtimes_feed_the_fixed_loop_the_simulated_delta() {
        fn code_only(src: &str) -> String {
            src.lines()
                .map(|line| {
                    let bytes = line.as_bytes();
                    let mut end = line.len();
                    let mut i = 0;
                    while i + 1 < bytes.len() {
                        if bytes[i] == b'/'
                            && bytes[i + 1] == b'/'
                            && (i == 0 || bytes[i - 1] != b':')
                        {
                            end = i;
                            break;
                        }
                        i += 1;
                    }
                    &line[..end]
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        for (name, src) in [
            ("headless", include_str!("headless.rs")),
            ("windowed", include_str!("windowed/event.rs")),
        ] {
            let code: String = code_only(src).chars().filter(|c| !c.is_whitespace()).collect();
            assert!(
                code.contains("advance_time(&mutself.world,dt)"),
                "{name} must take its simulated delta from `advance_time`"
            );
            assert!(
                !code.contains("run_fixed_and_update(&mutself.world,&mutself.schedule,&mutself.update_schedule,dt,dt,"),
                "{name} is feeding the RAW delta to the fixed loop again — that is the defect \
                 where a headless game had no clock and `set_time_scale` did nothing"
            );
        }
    }
}
