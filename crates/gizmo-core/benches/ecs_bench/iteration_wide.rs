use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
    query::Mut,
};
use super::common::*;
use super::iteration_frag::WideData;

pub fn bench_wide_iteration(c: &mut Criterion) {
    let mut world = World::new();

    macro_rules! create_entities {
        ($w:ident; $( $variants:ident ),*) => {
            $(
                #[derive(Clone, Copy)]
                struct $variants(f32);
                impl Component for $variants {}

                for _ in 0..20 {
                    $w.spawn_bundle((
                        $variants(0.0),
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
            )*
        };
    }

    create_entities!(world; A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z);

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

    c.bench_function("wide_iteration", |b| {
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

pub fn bench_wide_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Rotation(Vec3::ZERO),
            Pos::<0>(Vec3::ZERO),
            Vel::<0>(Vec3::ONE),
            Pos::<1>(Vec3::ZERO),
            Vel::<1>(Vec3::ONE),
            Pos::<2>(Vec3::ZERO),
            Vel::<2>(Vec3::ONE),
            Pos::<3>(Vec3::ZERO),
            Vel::<3>(Vec3::ONE),
            Pos::<4>(Vec3::ZERO),
            Vel::<4>(Vec3::ONE),
        )
    })).count();

    let mut query = world.query_mut::<(
        &Vel<0>,
        Mut<Pos<0>>,
        &Vel<1>,
        Mut<Pos<1>>,
        &Vel<2>,
        Mut<Pos<2>>,
        &Vel<3>,
        Mut<Pos<3>>,
        &Vel<4>,
        Mut<Pos<4>>,
    )>().unwrap();

    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("wide_simple_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (v0, mut p0, v1, mut p1, v2, mut p2, v3, mut p3, v4, mut p4))| {
                p0.0.0[0] += v0.0.0[0];
                p0.0.0[1] += v0.0.0[1];
                p0.0.0[2] += v0.0.0[2];

                p1.0.0[0] += v1.0.0[0];
                p1.0.0[1] += v1.0.0[1];
                p1.0.0[2] += v1.0.0[2];

                p2.0.0[0] += v2.0.0[0];
                p2.0.0[1] += v2.0.0[1];
                p2.0.0[2] += v2.0.0[2];

                p3.0.0[0] += v3.0.0[0];
                p3.0.0[1] += v3.0.0[1];
                p3.0.0[2] += v3.0.0[2];

                // the user's snippet stops here, but I will do p4 as well to be consistent with the 10 item fetch
                p4.0.0[0] += v4.0.0[0];
                p4.0.0[1] += v4.0.0[1];
                p4.0.0[2] += v4.0.0[2];
            });
        });
    });
}
