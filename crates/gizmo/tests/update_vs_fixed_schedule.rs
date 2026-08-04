//! Which schedule a plugin registers on is a behavioural contract, not a detail.
//!
//! `App` has two: `schedule` runs `0..N` times per rendered frame at a constant `dt`, and
//! `update_schedule` runs exactly once with the real frame delta. Putting a system on the
//! wrong one is invisible in review and invisible at runtime until someone notices their
//! camera stuttering or their keypress not registering — so it gets a test.
//!
//! These drive the two schedules directly rather than through the event loop, which needs a
//! window and a GPU.

use gizmo::core::world::World;
use gizmo::math::Vec3;
use gizmo::prelude::*;
use gizmo::systems::fps_look::{FpsLook, FpsLookPlugin};

/// Build an `App` far enough to apply plugins. No window and no renderer is created until
/// `run()`, so this is cheap and headless.
fn app_with_plugin<P: gizmo::app::Plugin<()>>(plugin: P) -> gizmo::app::App<()> {
    gizmo::app::App::<()>::new("schedule-placement-test", 64, 64).add_plugin(plugin)
}

/// Spawn a camera driven by `FpsLook`, with mouse motion pending on the input resource.
fn camera_with_pending_mouse_motion(world: &mut World) -> gizmo::core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::ZERO));
    world.add_component(e, FpsLook::new().with_sensitivity(0.01));
    // `FpsLookSystem` queries (FpsLook, Camera, Transform) together — without the camera the
    // entity simply does not match and the test would pass for the wrong reason.
    world.add_component(e, Camera::new(60.0_f32.to_radians(), 0.1, 1000.0, 0.0, 0.0, true));

    let mut input = gizmo::core::input::Input::new();
    input.on_mouse_delta(200.0, 0.0);
    world.insert_resource(input);
    e
}

fn yaw_of(world: &World, e: gizmo::core::entity::Entity) -> f32 {
    world
        .query::<&FpsLook>()
        .unwrap()
        .get(e.id())
        .expect("entity still has FpsLook")
        .yaw
}

/// `FpsLookSystem` reads `Input::mouse_delta`, which is accumulated from window events and
/// cleared once per *rendered* frame. On the fixed schedule it therefore saw a fraction of
/// the motion — or none at all on a frame where the accumulator did not fill, which with
/// vsync off is most frames. It belongs on the per-frame schedule.
#[test]
fn fps_look_runs_on_the_per_frame_schedule_not_the_fixed_one() {
    let mut app = app_with_plugin(FpsLookPlugin);
    let cam = camera_with_pending_mouse_motion(&mut app.world);

    let before = yaw_of(&app.world, cam);

    // The fixed schedule must not own this system.
    app.schedule.run(&mut app.world, 1.0 / 60.0);
    assert_eq!(
        yaw_of(&app.world, cam),
        before,
        "FpsLookSystem must not be registered on the fixed-timestep schedule — mouse deltas \
         are per-frame state and the fixed loop may run zero times in a frame"
    );

    // The per-frame schedule must.
    app.update_schedule.run(&mut app.world, 1.0 / 60.0);
    assert_ne!(
        yaw_of(&app.world, cam),
        before,
        "FpsLookSystem must be registered on the per-frame update schedule"
    );
}

/// The two schedules are genuinely separate storage: registering on one must not leak into
/// the other. A shared or aliased schedule would make the test above pass for the wrong
/// reason.
#[test]
fn the_two_schedules_are_independent() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let fixed_runs = Arc::new(AtomicU32::new(0));
    let update_runs = Arc::new(AtomicU32::new(0));

    let f = fixed_runs.clone();
    let u = update_runs.clone();

    let mut app = gizmo::app::App::<()>::new("independence-test", 64, 64);
    app.schedule.add_system(move |_w: &World, _dt: f32| {
        f.fetch_add(1, Ordering::Relaxed);
    });
    app.update_schedule.add_system(move |_w: &World, _dt: f32| {
        u.fetch_add(1, Ordering::Relaxed);
    });

    app.schedule.run(&mut app.world, 0.016);
    assert_eq!(fixed_runs.load(Ordering::Relaxed), 1);
    assert_eq!(
        update_runs.load(Ordering::Relaxed),
        0,
        "running the fixed schedule must not run update systems"
    );

    app.update_schedule.run(&mut app.world, 0.016);
    assert_eq!(
        fixed_runs.load(Ordering::Relaxed),
        1,
        "running the update schedule must not re-run fixed systems"
    );
    assert_eq!(update_runs.load(Ordering::Relaxed), 1);
}

/// Physics stays on the fixed schedule — the other half of the contract. The split exists
/// so simulation is frame-rate independent; moving the step to the per-frame schedule would
/// silently tie it to the display refresh rate.
///
/// Asserted structurally: `PhysicsPlugin` installs its world and registers on `app.schedule`,
/// and `crate::frame`'s tests already prove the two schedules run at different cadences.
#[test]
fn the_physics_plugin_installs_its_world_and_leaves_the_frame_schedule_alone() {
    use gizmo::physics::world::PhysicsWorld;

    let mut app = app_with_plugin(gizmo::plugins::PhysicsPlugin::default());
    assert!(
        app.world.get_resource::<PhysicsWorld>().is_some(),
        "PhysicsPlugin should have installed a PhysicsWorld"
    );

    // Running only the per-frame schedule must be a no-op for a world with no entities and
    // no per-frame systems registered by this plugin.
    app.update_schedule.run(&mut app.world, 1.0 / 60.0);
    assert!(app.world.get_resource::<PhysicsWorld>().is_some());
}

/// Transform propagation belongs on the per-frame schedule, and its placement carries an
/// ordering guarantee that used to be only an aspiration.
///
/// `PhysicsPlugin` labels its step `physics_step` with a comment saying transform systems
/// "can order themselves after it" — but no `.after("physics_step")` edge was ever wired, so
/// within a fixed step the order was whatever the batcher picked. Running the transforms on
/// the update schedule makes "after physics" structural instead: `run_fixed_and_update`
/// drains every fixed step before update runs at all.
///
/// It is also a cost fix. The result is consumed once, at draw; propagating it `0..N` times
/// per frame in the fixed loop was work nobody read.
#[test]
fn transform_propagation_runs_on_the_per_frame_schedule() {
    use gizmo::physics::components::GlobalTransform;

    let mut app = app_with_plugin(gizmo::plugins::TransformPlugin);

    let e = app.world.spawn();
    app.world
        .add_component(e, Transform::new(Vec3::new(3.0, 4.0, 5.0)));
    app.world.add_component(e, GlobalTransform::default());

    let world_y = |w: &World| -> f32 {
        w.query::<&GlobalTransform>()
            .unwrap()
            .get(e.id())
            .expect("entity keeps its GlobalTransform")
            .matrix
            .to_scale_rotation_translation()
            .2
            .y
    };

    // The fixed schedule must not own transform propagation any more.
    app.schedule.run(&mut app.world, 1.0 / 60.0);
    assert_eq!(
        world_y(&app.world),
        0.0,
        "the fixed-timestep schedule must not propagate transforms — the result is read once \
         per frame, not once per simulation step"
    );

    // The per-frame schedule must.
    app.update_schedule.run(&mut app.world, 1.0 / 60.0);
    assert!(
        (world_y(&app.world) - 4.0).abs() < 1e-5,
        "the per-frame schedule must propagate Transform into GlobalTransform, got y={}",
        world_y(&app.world)
    );
}

/// The whole point of the split: a system on the update schedule observes the world *after*
/// this frame's fixed steps. Anything that reads simulation output for presentation —
/// transform propagation, cameras, UI — depends on that ordering.
#[test]
fn update_systems_observe_the_world_after_the_frame_s_fixed_steps() {
    use gizmo_app::frame::run_fixed_and_update;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    let order = Arc::new(std::sync::Mutex::new(Vec::<&'static str>::new()));
    let fixed_count = Arc::new(AtomicU32::new(0));

    let mut world = World::new();
    world.insert_resource(gizmo::core::time::PhysicsTime::new(60));

    let o1 = order.clone();
    let fc = fixed_count.clone();
    let mut fixed = gizmo::core::system::Schedule::new();
    fixed.add_system(move |_w: &World, _dt: f32| {
        fc.fetch_add(1, Ordering::Relaxed);
        o1.lock().unwrap().push("fixed");
    });

    let o2 = order.clone();
    let mut update = gizmo::core::system::Schedule::new();
    update.add_system(move |_w: &World, _dt: f32| {
        o2.lock().unwrap().push("update");
    });

    // A frame long enough for several fixed steps.
    run_fixed_and_update(&mut world, &mut fixed, &mut update, 3.5 / 60.0, 3.5 / 60.0, |_| {});

    let seq = order.lock().unwrap().clone();
    let steps = fixed_count.load(Ordering::Relaxed) as usize;
    assert!(steps >= 3, "expected several fixed steps, got {steps}");
    assert_eq!(
        seq.last(),
        Some(&"update"),
        "update must run last, after every fixed step of the frame; sequence was {seq:?}"
    );
    assert_eq!(
        seq.iter().filter(|s| **s == "update").count(),
        1,
        "and exactly once; sequence was {seq:?}"
    );
}
