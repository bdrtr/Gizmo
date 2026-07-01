use criterion::Criterion;
use gizmo_core::world::World;
use super::common::*;

const ENTITY_BUNCH: usize = 5000;

pub fn empty_systems(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("empty_systems");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    for amount in [0, 2, 4] {
        let mut schedule = gizmo_core::system::Schedule::new();
        for _ in 0..amount {
            schedule.add_di_system(|| {});
        }
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    for amount in [10, 100, 1_000] {
        let mut schedule = gizmo_core::system::Schedule::new();
        for _ in 0..(amount / 5) {
            schedule.add_systems((|| {}, || {}, || {}, || {}, || {}));
        }
        schedule.run(&mut world, 0.0);
        group.bench_function(format!("{amount}_systems"), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}

pub fn busy_systems(criterion: &mut Criterion) {
    let ab = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestA>, gizmo_core::query::Mut<TestB>)>| {
        q.iter_mut().for_each(|(_, (mut a, mut b))| {
            core::mem::swap(&mut a.0, &mut b.0);
        });
    };
    let cd = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestC>, gizmo_core::query::Mut<TestD>)>| {
        q.iter_mut().for_each(|(_, (mut c, mut d))| {
            core::mem::swap(&mut c.0, &mut d.0);
        });
    };
    let ce = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestC>, gizmo_core::query::Mut<TestE>)>| {
        q.iter_mut().for_each(|(_, (mut c, mut e))| {
            core::mem::swap(&mut c.0, &mut e.0);
        });
    };
    let mut world = World::new();
    let mut group = criterion.benchmark_group("busy_systems");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    for entity_bunches in [1, 3, 5] {
        for _ in 0..4 * ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0)));
        }
        for _ in 0..4 * ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0)));
        }
        for _ in 0..ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0), TestD(0.0)));
        }
        for _ in 0..ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0), TestE(0.0)));
        }
        for system_amount in [3, 9, 15] {
            let mut schedule = gizmo_core::system::Schedule::new();
            for _ in 0..(system_amount / 3) {
                schedule.add_systems((ab, cd, ce));
            }
            schedule.run(&mut world, 0.0);
            group.bench_function(
                format!("{entity_bunches:02}x_entities_{system_amount:02}_systems"),
                |bencher| {
                    bencher.iter(|| {
                        schedule.run(&mut world, 0.0);
                    });
                },
            );
        }
    }
    group.finish();
}

pub fn contrived(criterion: &mut Criterion) {
    let s_0 = |mut q_0: gizmo_core::query::Query<(gizmo_core::query::Mut<TestA>, gizmo_core::query::Mut<TestB>)>| {
        q_0.iter_mut().for_each(|(_, (mut c_0, mut c_1))| {
            core::mem::swap(&mut c_0.0, &mut c_1.0);
        });
    };
    let s_1 = |mut q_0: gizmo_core::query::Query<(gizmo_core::query::Mut<TestA>, gizmo_core::query::Mut<TestC>)>, mut q_1: gizmo_core::query::Query<(gizmo_core::query::Mut<TestB>, gizmo_core::query::Mut<TestD>)>| {
        q_0.iter_mut().for_each(|(_, (mut c_0, mut c_1))| {
            core::mem::swap(&mut c_0.0, &mut c_1.0);
        });
        q_1.iter_mut().for_each(|(_, (mut c_0, mut c_1))| {
            core::mem::swap(&mut c_0.0, &mut c_1.0);
        });
    };
    let s_2 = |mut q_0: gizmo_core::query::Query<(gizmo_core::query::Mut<TestC>, gizmo_core::query::Mut<TestD>)>| {
        q_0.iter_mut().for_each(|(_, (mut c_0, mut c_1))| {
            core::mem::swap(&mut c_0.0, &mut c_1.0);
        });
    };
    let mut world = World::new();
    let mut group = criterion.benchmark_group("contrived");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));
    for entity_bunches in [1, 3, 5] {
        for _ in 0..ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0), TestD(0.0)));
        }
        for _ in 0..ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0)));
        }
        for _ in 0..ENTITY_BUNCH {
            let e = world.spawn();
            world.add_bundle(e, (TestC(0.0), TestD(0.0)));
        }
        for system_amount in [3, 9, 15] {
            let mut schedule = gizmo_core::system::Schedule::new();
            for _ in 0..(system_amount / 3) {
                schedule.add_di_system(s_0);
                schedule.add_di_system(s_1);
                schedule.add_di_system(s_2);
            }
            schedule.run(&mut world, 0.0);
            group.bench_function(
                format!("{entity_bunches:02}x_entities_{system_amount:02}_systems"),
                |bencher| {
                    bencher.iter(|| {
                        schedule.run(&mut world, 0.0);
                    });
                },
            );
        }
    }
    group.finish();
}

pub fn schedule_bench(c: &mut Criterion) {
    let ab = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestA>, gizmo_core::query::Mut<TestB>)>| {
        q.iter_mut().for_each(|(_, (mut a, mut b))| {
            core::mem::swap(&mut a.0, &mut b.0);
        });
    };
    let cd = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestC>, gizmo_core::query::Mut<TestD>)>| {
        q.iter_mut().for_each(|(_, (mut c, mut d))| {
            core::mem::swap(&mut c.0, &mut d.0);
        });
    };
    let ce = |mut q: gizmo_core::query::Query<(gizmo_core::query::Mut<TestC>, gizmo_core::query::Mut<TestE>)>| {
        q.iter_mut().for_each(|(_, (mut c, mut e))| {
            core::mem::swap(&mut c.0, &mut e.0);
        });
    };

    let mut group = c.benchmark_group("schedule");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    group.bench_function("base", |b| {
        let mut world = World::new();

        for _ in 0..10000 {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0)));
        }
        for _ in 0..10000 {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0)));
        }
        for _ in 0..10000 {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0), TestD(0.0)));
        }
        for _ in 0..10000 {
            let e = world.spawn();
            world.add_bundle(e, (TestA(0.0), TestB(0.0), TestC(0.0), TestE(0.0)));
        }

        let mut schedule = gizmo_core::system::Schedule::new();
        schedule.add_di_system(ab);
        schedule.add_di_system(cd);
        schedule.add_di_system(ce);
        schedule.run(&mut world, 0.0);

        b.iter(move || schedule.run(&mut world, 0.0));
    });
    group.finish();
}

pub fn build_schedule(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("build_schedule");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(15));

    let labels: Vec<_> = (0..1000)
        .map(|i| Box::leak(format!("sys_{}", i).into_boxed_str()) as &'static str)
        .collect();

    for graph_size in [100, 500, 1000] {
        group.bench_function(format!("{graph_size}_schedule_no_constraints"), |bencher| {
            bencher.iter(|| {
                let mut schedule = gizmo_core::system::Schedule::new();
                for _ in 0..graph_size {

                    schedule.add_di_system(|| {});
                }
                schedule.build();
            });
        });

        group.bench_function(format!("{graph_size}_schedule"), |bencher| {
            bencher.iter(|| {
                let mut schedule = gizmo_core::system::Schedule::new();
                use gizmo_core::system::IntoSystemConfig;
                schedule.add_di_system((|| {}).label("Dummy"));

                for i in 0..graph_size {
                    let mut sys = (|| {}).label(labels[i]).before("Dummy");
                    for label in labels.iter().take(i) {
                        sys = sys.after(label);
                    }
                    for label in &labels[i + 1..graph_size] {
                        sys = sys.before(label);
                    }
                    schedule.add_di_system(sys);
                }
                schedule.build();
            });
        });
    }

    group.finish();
}

pub fn empty_schedule_run(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("run_empty_schedule");
    let mut schedule = gizmo_core::system::Schedule::new();
    group.bench_function("default", |bencher| {
        bencher.iter(|| schedule.run(&mut world, 0.0));
    });
    group.finish();
}
