use criterion::Criterion;
use gizmo_core::{
    world::World,
    entity::Entity,
};
use super::common::*;

pub fn bench_observer_lifecycle_insert(c: &mut Criterion) {
    use std::hint::black_box;
    use gizmo_core::observer::{On, Insert};

    let mut world = World::new();

    fn on_insert(event: On<Insert, A>) {
        black_box(event);
    }

    world.add_observer(on_insert);
    let entity = world.spawn_bundle((A(0.0),));

    c.bench_function("observer_lifecycle_insert", |b| {
        b.iter(|| {
            for _ in 0..10_000 {
                world.add_component(entity, A(0.0));
            }
        });
    });
}

const DENSITY: usize = 20; // percent of nodes with listeners
const ENTITY_DEPTH: usize = 64;
const ENTITY_WIDTH: usize = 200;
const N_EVENTS: usize = 500;

pub fn bench_event_propagation(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("event_propagation");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    group.bench_function("single_event_type", |bencher| {
        let mut world = World::new();
        let (roots, leaves, nodes) = spawn_listener_hierarchy(&mut world);
        add_listeners_to_hierarchy::<DENSITY, 1>(&roots, &leaves, &nodes, &mut world);

        bencher.iter(|| {
            send_events::<1, N_EVENTS>(&mut world, &leaves);
        });
    });

    group.bench_function("single_event_type_no_listeners", |bencher| {
        let mut world = World::new();
        let (roots, leaves, nodes) = spawn_listener_hierarchy(&mut world);
        add_listeners_to_hierarchy::<DENSITY, 1>(&roots, &leaves, &nodes, &mut world);

        bencher.iter(|| {
            send_events::<9, N_EVENTS>(&mut world, &leaves);
        });
    });

    group.bench_function("four_event_types", |bencher| {
        let mut world = World::new();
        let (roots, leaves, nodes) = spawn_listener_hierarchy(&mut world);
        const FRAC_N_EVENTS_4: usize = N_EVENTS / 4;
        const FRAC_DENSITY_4: usize = DENSITY / 4;
        add_listeners_to_hierarchy::<FRAC_DENSITY_4, 1>(&roots, &leaves, &nodes, &mut world);
        add_listeners_to_hierarchy::<FRAC_DENSITY_4, 2>(&roots, &leaves, &nodes, &mut world);
        add_listeners_to_hierarchy::<FRAC_DENSITY_4, 3>(&roots, &leaves, &nodes, &mut world);
        add_listeners_to_hierarchy::<FRAC_DENSITY_4, 4>(&roots, &leaves, &nodes, &mut world);

        bencher.iter(|| {
            send_events::<1, FRAC_N_EVENTS_4>(&mut world, &leaves);
            send_events::<2, FRAC_N_EVENTS_4>(&mut world, &leaves);
            send_events::<3, FRAC_N_EVENTS_4>(&mut world, &leaves);
            send_events::<4, FRAC_N_EVENTS_4>(&mut world, &leaves);
        });
    });

    group.finish();
}

#[derive(Clone)]
struct TestEvent<const N: usize> {
    entity: Entity,
}

impl<const N: usize> gizmo_core::observer::EntityEvent for TestEvent<N> {
    fn target(&self) -> Entity {
        self.entity
    }
    fn can_propagate(&self) -> bool {
        true
    }
}

fn send_events<const N: usize, const N_EVENTS_PARAM: usize>(world: &mut World, leaves: &[Entity]) {
    let idx = 42 % leaves.len();
    let entity = leaves[idx];

    for _ in 0..N_EVENTS_PARAM {
        world.trigger(TestEvent::<N> { entity });
    }
}

fn spawn_listener_hierarchy(world: &mut World) -> (Vec<Entity>, Vec<Entity>, Vec<Entity>) {
    use gizmo_core::hierarchy::HierarchyExt;
    let mut roots = vec![];
    let mut leaves = vec![];
    let mut nodes = vec![];
    for _ in 0..ENTITY_WIDTH {
        let mut parent = world.spawn();
        roots.push(parent);
        for _ in 0..ENTITY_DEPTH {
            let child = world.spawn();
            nodes.push(child);

            world.add_child(parent, child);
            parent = child;
        }
        nodes.pop();
        leaves.push(parent);
    }
    (roots, leaves, nodes)
}

fn add_listeners_to_hierarchy<const DENSITY_PARAM: usize, const N: usize>(
    roots: &[Entity],
    leaves: &[Entity],
    nodes: &[Entity],
    world: &mut World,
) {
    for e in roots.iter() {
        world.observe(*e, empty_listener::<N>);
    }
    for e in leaves.iter() {
        world.observe(*e, empty_listener::<N>);
    }
    for (i, e) in nodes.iter().enumerate() {
        if i % 100 < DENSITY_PARAM {
            world.observe(*e, empty_listener::<N>);
        }
    }
}

fn empty_listener<const N: usize>(event: gizmo_core::observer::On<TestEvent<N>>) {
    std::hint::black_box(event);
}
