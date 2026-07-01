use criterion::Criterion;
use gizmo_core::world::World;

pub fn bench_combinator_system(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("param/combinator_system");

    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));

    use gizmo_core::system::{Schedule, SystemExt};

    let mut schedule = Schedule::new();
    schedule.add_system(
        (|| {})
            .pipe(|| {})
            .pipe(|| {})
            .pipe(|| {})
            .pipe(|| {})
            .pipe(|| {})
            .pipe(|| {})
            .pipe(|| {}),
    );
    // run once to initialize systems
    schedule.run(&mut world, 0.0);
    group.bench_function("8_piped_systems", |bencher| {
        bencher.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });

    group.finish();
}

pub struct DynSystemParam;
pub struct ParamBuilder;

pub struct DynParamBuilder {
    type_id: std::any::TypeId,
}

impl DynParamBuilder {
    pub fn new<T: 'static>(_: ParamBuilder) -> Self {
        Self {
            type_id: std::any::TypeId::of::<T>(),
        }
    }
}

pub trait BuildSystemDyn {
    fn build_state(self, world: &mut World) -> Self;
    fn build_system<F>(self, f: F) -> Box<dyn gizmo_core::system::System>
    where
        F: FnMut(DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam) + Send + Sync + 'static;
}

impl BuildSystemDyn for (
    DynParamBuilder, DynParamBuilder, DynParamBuilder, DynParamBuilder,
    DynParamBuilder, DynParamBuilder, DynParamBuilder, DynParamBuilder
) {
    fn build_state(self, _world: &mut World) -> Self { self }
    fn build_system<F>(self, f: F) -> Box<dyn gizmo_core::system::System>
    where
        F: FnMut(DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam) + Send + Sync + 'static
    {
        let t1 = self.0.type_id;
        let t2 = self.1.type_id;
        let t3 = self.2.type_id;
        let t4 = self.3.type_id;
        let t5 = self.4.type_id;
        let t6 = self.5.type_id;
        let t7 = self.6.type_id;
        let t8 = self.7.type_id;

        struct DynSys<F> {
            f: F,
            types: [std::any::TypeId; 8],
        }
        impl<F: FnMut(DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam) + Send + Sync + 'static> gizmo_core::system::System for DynSys<F> {
            fn run(&mut self, _world: &World, _dt: f32) {
                for &t in &self.types {
                    std::hint::black_box(t);
                }
                (self.f)(DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam, DynSystemParam);
            }
            fn access_info(&self) -> gizmo_core::system::AccessInfo {
                gizmo_core::system::AccessInfo::new()
            }
        }

        Box::new(DynSys {
            f,
            types: [t1, t2, t3, t4, t5, t6, t7, t8],
        })
    }
}

pub struct Res<T>(std::marker::PhantomData<T>);

pub fn dyn_param(criterion: &mut Criterion) {
    let mut world = World::new();
    let mut group = criterion.benchmark_group("param/combinator_system");

    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(3));

    struct R;
    world.insert_resource(R);

    use gizmo_core::system::Schedule;
    let mut schedule = Schedule::new();
    let system = (
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
        DynParamBuilder::new::<Res<R>>(ParamBuilder),
    )
        .build_state(&mut world)
        .build_system(
            |_: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam,
             _: DynSystemParam| {},
        );
    schedule.add_system(system);
    // run once to initialize systems
    schedule.run(&mut world, 0.0);
    group.bench_function("8_dyn_params_system", |bencher| {
        bencher.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });

    group.finish();
}
