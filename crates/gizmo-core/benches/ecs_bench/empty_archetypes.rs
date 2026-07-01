use criterion::Criterion;
use gizmo_core::world::World;
use super::common::*;

fn iter(
    query: gizmo_core::query::Query<(
        &ArchetypeData<0>,
        &ArchetypeData<1>,
        &ArchetypeData<2>,
        &ArchetypeData<3>,
        &ArchetypeData<4>,
        &ArchetypeData<5>,
        &ArchetypeData<6>,
        &ArchetypeData<7>,
        &ArchetypeData<8>,
        &ArchetypeData<9>,
        &ArchetypeData<10>,
        &ArchetypeData<11>,
    )>,
) {
    for comp in query.iter() {
        core::hint::black_box(comp);
    }
}

fn par_for_each(
    query: gizmo_core::query::Query<(
        &ArchetypeData<0>,
        &ArchetypeData<1>,
        &ArchetypeData<2>,
        &ArchetypeData<3>,
        &ArchetypeData<4>,
        &ArchetypeData<5>,
        &ArchetypeData<6>,
        &ArchetypeData<7>,
        &ArchetypeData<8>,
        &ArchetypeData<9>,
        &ArchetypeData<10>,
        &ArchetypeData<11>,
    )>,
) {
    query.par_for_each(|(_id, comp)| {
        core::hint::black_box(comp);
    });
}

fn setup_empty_archetypes(setup_sys: impl FnOnce(&mut gizmo_core::system::Schedule)) -> (World, gizmo_core::system::Schedule) {
    let world = World::new();
    let mut schedule = gizmo_core::system::Schedule::new();
    setup_sys(&mut schedule);
    (world, schedule)
}

fn add_archetypes(world: &mut World, count: u16) {
    for i in 0..count {
        let e = world.spawn();
        world.add_component(e, ArchetypeData::<0>(1.0));
        world.add_component(e, ArchetypeData::<1>(1.0));
        world.add_component(e, ArchetypeData::<2>(1.0));
        world.add_component(e, ArchetypeData::<3>(1.0));
        world.add_component(e, ArchetypeData::<4>(1.0));
        world.add_component(e, ArchetypeData::<5>(1.0));
        world.add_component(e, ArchetypeData::<6>(1.0));
        world.add_component(e, ArchetypeData::<7>(1.0));
        world.add_component(e, ArchetypeData::<8>(1.0));
        world.add_component(e, ArchetypeData::<9>(1.0));
        world.add_component(e, ArchetypeData::<10>(1.0));
        world.add_component(e, ArchetypeData::<11>(1.0));
        if i & (1 << 1) != 0 { world.add_component(e, ArchetypeData::<12>(1.0)); }
        if i & (1 << 2) != 0 { world.add_component(e, ArchetypeData::<13>(1.0)); }
        if i & (1 << 3) != 0 { world.add_component(e, ArchetypeData::<14>(1.0)); }
        if i & (1 << 4) != 0 { world.add_component(e, ArchetypeData::<15>(1.0)); }
        if i & (1 << 5) != 0 { world.add_component(e, ArchetypeData::<16>(1.0)); }
        if i & (1 << 6) != 0 { world.add_component(e, ArchetypeData::<17>(1.0)); }
        if i & (1 << 7) != 0 { world.add_component(e, ArchetypeData::<18>(1.0)); }
        if i & (1 << 8) != 0 { world.add_component(e, ArchetypeData::<19>(1.0)); }
        if i & (1 << 9) != 0 { world.add_component(e, ArchetypeData::<20>(1.0)); }
        if i & (1 << 10) != 0 { world.add_component(e, ArchetypeData::<21>(1.0)); }
    }
}

pub fn empty_archetypes(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("empty_archetypes");
    for archetype_count in [10, 100, 1_000] { // reduced max archetypes for bench speed
        let (mut world, mut schedule) = setup_empty_archetypes(|schedule| {
            schedule.add_di_system(iter);
        });
        add_archetypes(&mut world, archetype_count);
        world.clear_entities(); // Test the clear_entities implementation

        let e = world.spawn();
        world.add_component(e, ArchetypeData::<0>(1.0));
        world.add_component(e, ArchetypeData::<1>(1.0));
        world.add_component(e, ArchetypeData::<2>(1.0));
        world.add_component(e, ArchetypeData::<3>(1.0));
        world.add_component(e, ArchetypeData::<4>(1.0));
        world.add_component(e, ArchetypeData::<5>(1.0));
        world.add_component(e, ArchetypeData::<6>(1.0));
        world.add_component(e, ArchetypeData::<7>(1.0));
        world.add_component(e, ArchetypeData::<8>(1.0));
        world.add_component(e, ArchetypeData::<9>(1.0));
        world.add_component(e, ArchetypeData::<10>(1.0));
        world.add_component(e, ArchetypeData::<11>(1.0));

        schedule.build();
        schedule.run(&mut world, 0.0);

        group.bench_function(format!("iter_{}", archetype_count), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }

    for archetype_count in [10, 100, 1_000] {
        let (mut world, mut schedule) = setup_empty_archetypes(|schedule| {
            schedule.add_di_system(par_for_each);
        });
        add_archetypes(&mut world, archetype_count);
        world.clear_entities();

        let e = world.spawn();
        world.add_component(e, ArchetypeData::<0>(1.0));
        world.add_component(e, ArchetypeData::<1>(1.0));
        world.add_component(e, ArchetypeData::<2>(1.0));
        world.add_component(e, ArchetypeData::<3>(1.0));
        world.add_component(e, ArchetypeData::<4>(1.0));
        world.add_component(e, ArchetypeData::<5>(1.0));
        world.add_component(e, ArchetypeData::<6>(1.0));
        world.add_component(e, ArchetypeData::<7>(1.0));
        world.add_component(e, ArchetypeData::<8>(1.0));
        world.add_component(e, ArchetypeData::<9>(1.0));
        world.add_component(e, ArchetypeData::<10>(1.0));
        world.add_component(e, ArchetypeData::<11>(1.0));

        schedule.build();
        schedule.run(&mut world, 0.0);

        group.bench_function(format!("par_for_each_{}", archetype_count), |bencher| {
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }
    group.finish();
}
