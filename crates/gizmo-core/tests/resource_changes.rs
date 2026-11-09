//! A resource keeps the tick of its last write, as a component does: inserting it and writing
//! through a write guard (`ResMut<T>`) stamp it, reading does not, and a system can ask
//! `Res::is_changed` the way a query asks `Changed<T>`.

use gizmo_core::prelude::*;

#[derive(Clone, Default)]
struct Score(u32);

#[derive(Clone, Default)]
struct Seen(Vec<bool>);

#[test]
fn inserting_and_writing_stamp_the_tick_and_reading_does_not() {
    let mut world = World::new();
    assert_eq!(world.resource_change_tick::<Score>(), None);

    world.insert_resource(Score(0));
    let inserted = world.resource_change_tick::<Score>().expect("just inserted");
    assert_eq!(inserted, world.tick);

    world.begin_change_frame(world.tick);
    // A write guard taken and only read through is not a write.
    {
        let guard = world.get_resource_mut::<Score>().unwrap();
        assert_eq!(guard.0, 0);
    }
    assert_eq!(world.resource_change_tick::<Score>(), Some(inserted));
    assert!(!world.is_resource_changed::<Score>());

    world.get_resource_mut::<Score>().unwrap().0 = 5;
    assert_eq!(world.resource_change_tick::<Score>(), Some(world.tick));
    assert!(world.is_resource_changed::<Score>());

    world.begin_change_frame(world.tick);
    assert!(!world.is_resource_changed::<Score>(), "a write is news for one frame");
}

fn watch(score: Res<Score>, mut seen: ResMut<Seen>) {
    seen.0.push(score.is_changed());
}

fn bump(mut score: ResMut<Score>) {
    score.0 += 1;
}

fn look(score: ResMut<Score>, mut seen: ResMut<Seen>) {
    let _ = score.0;
    seen.0.push(score.is_changed());
}

#[test]
fn a_system_sees_a_resource_written_between_its_runs_once() {
    let mut world = World::new();
    world.insert_resource(Score(0));
    world.insert_resource(Seen::default());
    let mut schedule = Schedule::new();
    schedule.add_systems(watch);

    schedule.run(&mut world, 0.016); // the insertion is news to the first run
    schedule.run(&mut world, 0.016);
    world.get_resource_mut::<Score>().unwrap().0 = 1;
    schedule.run(&mut world, 0.016);
    schedule.run(&mut world, 0.016);

    assert_eq!(world.get_resource::<Seen>().unwrap().0, vec![true, false, true, false]);
}

#[test]
fn res_mut_reports_its_own_write_and_not_a_read() {
    let mut world = World::new();
    world.insert_resource(Score(0));
    world.insert_resource(Seen::default());
    let mut reads = Schedule::new();
    reads.add_systems(look);
    let mut writes = Schedule::new();
    writes.add_systems(bump);

    reads.run(&mut world, 0.016);
    reads.run(&mut world, 0.016);
    writes.run(&mut world, 0.016);
    reads.run(&mut world, 0.016);
    reads.run(&mut world, 0.016);

    // A `ResMut` the system only read through left the resource unchanged on the second run;
    // `bump`'s write is seen once by the next run of `look`, then not again.
    assert_eq!(world.get_resource::<Seen>().unwrap().0, vec![true, false, true, false]);
}
