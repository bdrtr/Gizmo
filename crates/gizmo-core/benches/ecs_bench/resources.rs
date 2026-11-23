use criterion::Criterion;
use gizmo_core::world::World;

struct DummyRes<const N: usize>;
struct BenchRes;

/// Inserts the hundred `DummyRes` types `100 × $hundred` to `100 × $hundred + 99`, digit by digit:
/// each is a type of its own, which is what the world's resource lookup is measured among.
macro_rules! insert_hundred {
    ($world:ident, $hundred:literal) => {
        insert_hundred!(@tens $world, $hundred, [0 1 2 3 4 5 6 7 8 9]);
    };
    (@tens $world:ident, $hundred:literal, [$($ten:literal)*]) => {
        $( insert_hundred!(@ones $world, $hundred, $ten, [0 1 2 3 4 5 6 7 8 9]); )*
    };
    (@ones $world:ident, $hundred:literal, $ten:literal, [$($one:literal)*]) => {
        $( $world.insert_resource(DummyRes::<{ $hundred * 100 + $ten * 10 + $one }>); )*
    };
}

/// A world holding 500 resources of distinct types, `DummyRes<0>` to `DummyRes<499>`.
fn create_resource_world() -> World {
    let mut world = World::new();
    let before = world.resource_count();
    insert_hundred!(world, 0);
    insert_hundred!(world, 1);
    insert_hundred!(world, 2);
    insert_hundred!(world, 3);
    insert_hundred!(world, 4);
    assert_eq!(world.resource_count() - before, 500, "five hundred types, each once");
    world
}

pub fn get_resource(criterion: &mut Criterion) {
    let mut world = create_resource_world();
    world.insert_resource(BenchRes);
    criterion.bench_function("get_resource", |bencher| {
        bencher.iter(|| world.get_resource::<BenchRes>());
    });
}

pub fn get_resource_mut(criterion: &mut Criterion) {
    let mut world = create_resource_world();
    world.insert_resource(BenchRes);
    criterion.bench_function("get_resource_mut", |bencher| {
        bencher.iter(|| {
            core::hint::black_box(world.get_resource_mut::<BenchRes>());
        });
    });
}

pub fn insert_remove_resource(criterion: &mut Criterion) {
    let mut world = create_resource_world();
    criterion.bench_function("insert_remove_resource", |bencher| {
        bencher.iter(|| {
            world.insert_resource(BenchRes);
            core::hint::black_box(&mut world);
            world.remove_resource::<BenchRes>()
        });
    });
}
