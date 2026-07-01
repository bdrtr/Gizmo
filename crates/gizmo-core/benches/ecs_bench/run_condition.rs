use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};

fn yes() -> bool {
    true
}

fn no() -> bool {
    false
}

pub fn run_condition_yes(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("run_condition/yes");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    fn empty() {}

    use gizmo_core::system::{Schedule, DistributiveRunIfExt};
    for amount in [10, 100, 1_000] {
        let mut schedule = Schedule::new();
        for _ in 0..(amount / 5) {
            schedule.add_system((empty, empty, empty, empty, empty).distributive_run_if(yes));
        }
        // run once to initialize systems
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}

pub fn run_condition_no(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("run_condition/no");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    fn empty() {}
    use gizmo_core::system::{Schedule, DistributiveRunIfExt};
    for amount in [10, 100, 1_000] {
        let mut schedule = Schedule::new();
        for _ in 0..(amount / 5) {
            schedule.add_system((empty, empty, empty, empty, empty).distributive_run_if(no));
        }
        // run once to initialize systems
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}

#[derive(Clone, Copy)]
struct TestBool(pub bool);
impl Component for TestBool {}

pub fn run_condition_yes_with_query(criterion: &mut Criterion) {
    let mut world = World::new();
    world.spawn_bundle(TestBool(true));
    let mut group = criterion.benchmark_group("run_condition/yes_using_query");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    fn empty() {}
    let yes_with_query = |query: gizmo_core::query::Query<&TestBool>| -> bool {
        query.iter().next().map(|q| q.1.0).unwrap_or(false)
    };
    use gizmo_core::system::{Schedule, DistributiveRunIfExt};
    for amount in [10, 100, 1_000] {
        let mut schedule = Schedule::new();
        for _ in 0..(amount / 5) {
            schedule.add_system(
                (empty, empty, empty, empty, empty).distributive_run_if::<(gizmo_core::query::Query<&TestBool>,), _>(yes_with_query),
            );
        }
        // run once to initialize systems
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}

struct TestResource(pub bool);

pub fn run_condition_yes_with_resource(criterion: &mut Criterion) {
    let mut world = World::new();
    world.insert_resource(TestResource(true));
    let mut group = criterion.benchmark_group("run_condition/yes_using_resource");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    fn empty() {}
    let yes_with_resource = |res: gizmo_core::system::Res<TestResource>| -> bool {
        res.0
    };
    use gizmo_core::system::{Schedule, DistributiveRunIfExt};
    for amount in [10, 100, 1_000] {
        let mut schedule = Schedule::new();
        for _ in 0..(amount / 5) {
            schedule.add_system(
                (empty, empty, empty, empty, empty).distributive_run_if::<(gizmo_core::system::Res<TestResource>,), _>(yes_with_resource),
            );
        }
        // run once to initialize systems
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}
