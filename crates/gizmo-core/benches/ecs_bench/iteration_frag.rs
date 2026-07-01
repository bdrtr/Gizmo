use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
    query::Mut,
};

// 5. Query Iteration Fragmentation
macro_rules! create_variants {
    ($($name:ident),*) => {
        $(
            #[derive(Clone, Copy)]
            struct $name(f32);
            impl Component for $name {}
        )*
    };
}

create_variants!(
    C00, C01, C02, C03, C04, C05, C06, C07, C08, C09,
    C10, C11, C12, C13, C14, C15, C16, C17, C18, C19,
    C20, C21, C22, C23, C24, C25, C26, C27, C28, C29,
    C30, C31, C32, C33, C34, C35, C36, C37, C38, C39,
    C40, C41, C42, C43, C44, C45, C46, C47, C48, C49,
    C50, C51, C52, C53, C54, C55, C56, C57, C58, C59,
    C60, C61, C62, C63, C64, C65, C66, C67, C68, C69,
    C70, C71, C72, C73, C74, C75, C76, C77, C78, C79,
    C80, C81, C82, C83, C84, C85, C86, C87, C88, C89,
    C90, C91, C92, C93, C94, C95, C96, C97, C98, C99
);

#[derive(Clone, Copy)]
struct Data(f32);
impl Component for Data {}

pub fn bench_fragmented_iteration(c: &mut Criterion) {
    let mut world = World::new();

    for _ in 0..5 {
        world.spawn_bundle(Data(1.0));
    }

    macro_rules! spawn_variants {
        ($w:ident; $($name:ident),*) => {
            $(
                for _ in 0..5 {
                    $w.spawn_bundle($name(0.0));
                }
            )*
        };
    }

    spawn_variants!(world; C00, C01, C02, C03, C04, C05, C06, C07, C08, C09);
    spawn_variants!(world; C10, C11, C12, C13, C14, C15, C16, C17, C18, C19);
    spawn_variants!(world; C20, C21, C22, C23, C24, C25, C26, C27, C28, C29);
    spawn_variants!(world; C30, C31, C32, C33, C34, C35, C36, C37, C38, C39);
    spawn_variants!(world; C40, C41, C42, C43, C44, C45, C46, C47, C48, C49);
    spawn_variants!(world; C50, C51, C52, C53, C54, C55, C56, C57, C58, C59);
    spawn_variants!(world; C60, C61, C62, C63, C64, C65, C66, C67, C68, C69);
    spawn_variants!(world; C70, C71, C72, C73, C74, C75, C76, C77, C78, C79);
    spawn_variants!(world; C80, C81, C82, C83, C84, C85, C86, C87, C88, C89);
    spawn_variants!(world; C90, C91, C92, C93, C94, C95, C96, C97, C98, C99);

    // Warm up the query cache
    let mut query = world.query_mut::<Mut<Data>>().unwrap();
    query.iter_mut().for_each(|_| {});

    c.bench_function("fragmented_iteration", |b| {
        b.iter(|| {
            for (_id, mut data) in query.iter_mut() {
                data.0 *= 2.0;
            }
        });
    });
}

#[derive(Clone, Copy)]
pub struct WideData<const X: usize>(pub f32);
impl<const X: usize> Component for WideData<X> {}

pub fn bench_fragmented_wide_iteration(c: &mut Criterion) {
    let mut world = World::new();

    for _ in 0..5 {
        world.spawn_bundle((
            WideData::<0>(1.0),
            WideData::<1>(1.0),
            WideData::<2>(1.0),
            WideData::<3>(1.0),
            WideData::<4>(1.0),
            WideData::<5>(1.0),
            WideData::<6>(1.0),
            WideData::<7>(1.0),
            WideData::<8>(1.0),
            WideData::<9>(1.0),
            WideData::<10>(1.0),
        ));
    }

    macro_rules! create_noise_entities {
        ($w:ident; $( $variants:ident ),*) => {
            $(
                for _ in 0..5 {
                    $w.spawn_bundle($variants(0.0));
                }
            )*
        };
    }

    create_noise_entities!(world; C00, C01, C02, C03, C04, C05, C06, C07, C08, C09);
    create_noise_entities!(world; C10, C11, C12, C13, C14, C15, C16, C17, C18, C19);
    create_noise_entities!(world; C20, C21, C22, C23, C24, C25, C26, C27, C28, C29);
    create_noise_entities!(world; C30, C31, C32, C33, C34, C35, C36, C37, C38, C39);
    create_noise_entities!(world; C40, C41, C42, C43, C44, C45, C46, C47, C48, C49);
    create_noise_entities!(world; C50, C51, C52, C53, C54, C55, C56, C57, C58, C59);
    create_noise_entities!(world; C60, C61, C62, C63, C64, C65, C66, C67, C68, C69);
    create_noise_entities!(world; C70, C71, C72, C73, C74, C75, C76, C77, C78, C79);
    create_noise_entities!(world; C80, C81, C82, C83, C84, C85, C86, C87, C88, C89);
    create_noise_entities!(world; C90, C91, C92, C93, C94, C95, C96, C97, C98, C99);

    let mut query = world.query_mut::<(
        Mut<WideData<0>>,
        Mut<WideData<1>>,
        Mut<WideData<2>>,
        Mut<WideData<3>>,
        Mut<WideData<4>>,
        Mut<WideData<5>>,
        Mut<WideData<6>>,
        Mut<WideData<7>>,
        Mut<WideData<8>>,
        Mut<WideData<9>>,
        Mut<WideData<10>>,
    )>().unwrap();

    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("fragmented_wide_iteration", |b| {
        b.iter(|| {
            for (_id, (mut d0, mut d1, mut d2, mut d3, mut d4, mut d5, mut d6, mut d7, mut d8, mut d9, mut d10)) in query.iter_mut() {
                d0.0 *= 2.0;
                d1.0 *= 2.0;
                d2.0 *= 2.0;
                d3.0 *= 2.0;
                d4.0 *= 2.0;
                d5.0 *= 2.0;
                d6.0 *= 2.0;
                d7.0 *= 2.0;
                d8.0 *= 2.0;
                d9.0 *= 2.0;
                d10.0 *= 2.0;
            }
        });
    });
}
