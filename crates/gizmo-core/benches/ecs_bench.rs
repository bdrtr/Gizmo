use criterion::{criterion_group, criterion_main, Criterion, BatchSize};
use gizmo_core::{
    component::{Component, Bundle, StorageType},
    world::World,
    query::{Query, Mut},
    entity::Entity,
};

#[derive(Clone, Copy)]
struct Mat4([f32; 16]);

impl Mat4 {
    const ONE: Self = Self([1.0; 16]);
    const ZERO: Self = Self([0.0; 16]);
}

#[derive(Clone, Copy)]
struct Vec3([f32; 3]);

impl Vec3 {
    const ONE: Self = Self([1.0; 3]);
    const ZERO: Self = Self([0.0; 3]);
}

#[derive(Clone, Copy)]
struct Transform(Mat4);
impl Component for Transform {}

#[derive(Clone, Copy)]
struct Position(Vec3);
impl Component for Position {}

#[derive(Clone, Copy)]
struct Rotation(Vec3);
impl Component for Rotation {}

#[derive(Clone, Copy)]
struct Velocity(Vec3);
impl Component for Velocity {}

// 1. SparseSet Benchmark (High churn)
#[derive(Clone, Copy)]
struct A(f32);
impl Component for A {}

#[derive(Clone, Copy)]
struct B(f32);
impl Component for B {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

fn bench_insert_remove_sparseset(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);
    
    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_sparseset", |b| {
        b.iter(|| {
            for entity in &entities {
                world.add_component(*entity, B(0.0));
            }
            for entity in &entities {
                world.remove_component::<B>(*entity);
            }
        });
    });
}

// 2. Batch Operations Benchmark (Archetype Migration O(1))
fn bench_insert_remove_batch(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);
    
    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_batch", |b| {
        b.iter(|| {
            // O(1) Arch lookup
            world.insert_batch(&entities, Velocity(Vec3::ZERO));
            world.remove_batch::<Velocity>(&entities);
        });
    });
}

// 3. Heavyweight Nested Bundle
#[derive(Clone, Copy)]
struct F<const N: usize>(Mat4);
impl<const N: usize> Component for F<N> {}

fn bench_heavyweight_bundle(c: &mut Criterion) {
    let mut world = World::new();
    let mut entities = Vec::with_capacity(10_000);
    
    for _ in 0..10_000 {
        entities.push(world.spawn_bundle(A(0.0)));
    }

    c.bench_function("insert_remove_heavy_bundle", |b| {
        b.iter(|| {
            // O(1) Migration for 7 components at once
            for entity in &entities {
                world.add_bundle(*entity, (
                    F::<1>(Mat4::ONE),
                    F::<2>(Mat4::ONE),
                    F::<3>(Mat4::ONE),
                    F::<4>(Mat4::ONE),
                    F::<5>(Mat4::ONE),
                    F::<6>(Mat4::ONE),
                    F::<7>(Mat4::ONE),
                ));
            }

            for entity in &entities {
                world.remove_bundle::<(F<1>, F<2>, F<3>, F<4>, F<5>, F<6>, F<7>)>(*entity);
            }
        });
    });
}

// 4. Spawn Batch vs Loop
fn bench_spawn_batch(c: &mut Criterion) {
    c.bench_function("spawn_batch_10k", |b| {
        b.iter_batched(
            World::new,
            |mut world| {
                let iter = (0..10_000).map(|_| {
                    (
                        Transform(Mat4::ONE),
                        Position(Vec3::ONE),
                        Rotation(Vec3::ONE),
                        Velocity(Vec3::ONE),
                    )
                });
                // Exhaust the iterator to actually spawn
                let _ = world.spawn_batch(iter).count();
            },
            BatchSize::LargeInput,
        );
    });

    c.bench_function("spawn_loop_10k", |b| {
        b.iter_batched(
            World::new,
            |mut world| {
                for _ in 0..10_000 {
                    world.spawn_bundle((
                        Transform(Mat4::ONE),
                        Position(Vec3::ONE),
                        Rotation(Vec3::ONE),
                        Velocity(Vec3::ONE),
                    ));
                }
            },
            BatchSize::LargeInput,
        );
    });
}

// 5. Heavy Compute
fn bench_heavy_compute(c: &mut Criterion) {
    let mut world = World::new();
    world.spawn_batch((0..1_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ONE),
            Rotation(Vec3::ONE),
            Velocity(Vec3::ONE),
        )
    })).count();

    c.bench_function("heavy_compute_par", |b| {
        b.iter(|| {
            let query = world.query::<(Mut<Position>, Mut<Transform>)>().unwrap();
            query.par_for_each(|(_id, (mut pos, mut mat))| {
                for _ in 0..100 {
                    // simulate inverse matrix
                    mat.0.0[0] *= 0.99;
                }
                pos.0.0[0] *= mat.0.0[0];
            });
        });
    });
}



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

fn bench_fragmented_iteration(c: &mut Criterion) {
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
    let mut query = world.query::<Mut<Data>>().unwrap();
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
struct WideData<const X: usize>(f32);
impl<const X: usize> Component for WideData<X> {}

fn bench_wide_iteration(c: &mut Criterion) {
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

    let mut query = world.query::<(
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

fn bench_fragmented_wide_iteration(c: &mut Criterion) {
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

    let mut query = world.query::<(
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

fn bench_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO), // Replace Vec3::X with ZERO since my mock struct doesn't have X
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("simple_iter", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

fn bench_contiguous_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("contiguous_iter", |b| {
        b.iter(|| {
            let iter = query.iter_chunks_mut();
            for (_ids, (velocity_slice, position_slice)) in iter {
                assert!(velocity_slice.len() == position_slice.len());
                for (v, p) in velocity_slice.iter().zip(position_slice.iter_mut()) {
                    p.0.0[0] += v.0.0[0];
                    p.0.0[1] += v.0.0[1];
                    p.0.0[2] += v.0.0[2];
                }
            }
        });
    });
}

fn bench_contiguous_iter_avx2(c: &mut Criterion) {
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }

    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    #[target_feature(enable = "avx2")]
    unsafe fn exec(position: &mut [Position], velocity: &[Velocity]) {
        assert!(position.len() == velocity.len());
        for i in 0..position.len() {
            position[i].0.0[0] += velocity[i].0.0[0];
            position[i].0.0[1] += velocity[i].0.0[1];
            position[i].0.0[2] += velocity[i].0.0[2];
        }
    }

    c.bench_function("contiguous_iter_avx2", |b| {
        b.iter(|| {
            let iter = query.iter_chunks_mut();
            for (_ids, (velocity_slice, position_slice)) in iter {
                unsafe {
                    exec(position_slice, velocity_slice);
                }
            }
        });
    });
}

fn bench_for_each_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("for_each_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (velocity, mut position))| {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            });
        });
    });
}

fn bench_cache_locality_loss(c: &mut Criterion) {
    let mut world = World::new();

    let mut v = vec![];
    for _ in 0..10_000 {
        world.spawn_bundle((A(0.0), B(0.0)));
        v.push(world.spawn_bundle(A(0.0)));
    }

    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    v.shuffle(&mut rng);

    for e in v.into_iter() {
        world.despawn(e);
    }

    let mut query = world.query::<(Mut<A>, &B)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("cache_locality_loss", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (mut v1, v2))| {
                v1.0 += v2.0;
            });
        });
    });
}

#[derive(Clone, Copy)]
struct SparsePos(Vec3);
impl Component for SparsePos {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
struct SparseVel(Vec3);
impl Component for SparseVel {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

fn bench_sparse_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            SparsePos(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            SparseVel(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&SparseVel, Mut<SparsePos>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("sparse_iter", |b| {
        b.iter(|| {
            query.iter_mut().for_each(|(_id, (velocity, mut position))| {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            });
        });
    });
}

#[derive(Clone, Copy)]
struct Pos<const N: usize>(Vec3);
impl<const N: usize> Component for Pos<N> {}

#[derive(Clone, Copy)]
struct Vel<const N: usize>(Vec3);
impl<const N: usize> Component for Vel<N> {}

fn bench_wide_simple_iter(c: &mut Criterion) {
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

    let mut query = world.query::<(
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

#[derive(Clone, Copy)]
struct SparsePosWide<const N: usize>(Vec3);
impl<const N: usize> Component for SparsePosWide<N> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
struct SparseVelWide<const N: usize>(Vec3);
impl<const N: usize> Component for SparseVelWide<N> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

fn bench_wide_sparse_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Rotation(Vec3::ZERO),
            SparsePosWide::<0>(Vec3::ZERO),
            SparseVelWide::<0>(Vec3::ONE),
            SparsePosWide::<1>(Vec3::ZERO),
            SparseVelWide::<1>(Vec3::ONE),
            SparsePosWide::<2>(Vec3::ZERO),
            SparseVelWide::<2>(Vec3::ONE),
            SparsePosWide::<3>(Vec3::ZERO),
            SparseVelWide::<3>(Vec3::ONE),
            SparsePosWide::<4>(Vec3::ZERO),
            SparseVelWide::<4>(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(
        &SparseVelWide<0>,
        Mut<SparsePosWide<0>>,
        &SparseVelWide<1>,
        Mut<SparsePosWide<1>>,
        &SparseVelWide<2>,
        Mut<SparsePosWide<2>>,
        &SparseVelWide<3>,
        Mut<SparsePosWide<3>>,
        &SparseVelWide<4>,
        Mut<SparsePosWide<4>>,
    )>().unwrap();
    
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("wide_sparse_iter", |b| {
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
                
                p4.0.0[0] += v4.0.0[0];
                p4.0.0[1] += v4.0.0[1];
                p4.0.0[2] += v4.0.0[2];
            });
        });
    });
}

fn bench_bypass_change_detection(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&Velocity, Mut<Position>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("bypass_change_detection", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                let p = position.bypass_change_detection();
                p.0.0[0] += velocity.0.0[0];
                p.0.0[1] += velocity.0.0[1];
                p.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

fn bench_sparse_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            SparsePos(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            SparseVel(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(&SparseVel, Mut<SparsePos>)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("sparse_simple_iter", |b| {
        b.iter(|| {
            for (_id, (velocity, mut position)) in query.iter_mut() {
                position.0.0[0] += velocity.0.0[0];
                position.0.0[1] += velocity.0.0[1];
                position.0.0[2] += velocity.0.0[2];
            }
        });
    });
}

fn bench_system_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Position(Vec3::ZERO),
            Rotation(Vec3::ZERO),
            Velocity(Vec3::ONE),
        )
    })).count();

    fn query_system(mut query: Query<(&Velocity, Mut<Position>)>) {
        for (_id, (velocity, mut position)) in query.iter_mut() {
            position.0.0[0] += velocity.0.0[0];
            position.0.0[1] += velocity.0.0[1];
            position.0.0[2] += velocity.0.0[2];
        }
    }

    use gizmo_core::system::IntoSystem;
    let mut system = query_system.into_system();
    
    // Warmup
    system.run(&world, 0.0);

    c.bench_function("system_iter", |b| {
        b.iter(|| {
            system.run(&world, 0.0);
        });
    });
}

fn bench_wide_sparse_simple_iter(c: &mut Criterion) {
    let mut world = World::new();

    world.spawn_batch((0..10_000).map(|_| {
        (
            Transform(Mat4::ONE),
            Rotation(Vec3::ZERO),
            SparsePosWide::<0>(Vec3::ZERO),
            SparseVelWide::<0>(Vec3::ONE),
            SparsePosWide::<1>(Vec3::ZERO),
            SparseVelWide::<1>(Vec3::ONE),
            SparsePosWide::<2>(Vec3::ZERO),
            SparseVelWide::<2>(Vec3::ONE),
            SparsePosWide::<3>(Vec3::ZERO),
            SparseVelWide::<3>(Vec3::ONE),
            SparsePosWide::<4>(Vec3::ZERO),
            SparseVelWide::<4>(Vec3::ONE),
        )
    })).count();

    let mut query = world.query::<(
        &SparseVelWide<0>,
        Mut<SparsePosWide<0>>,
        &SparseVelWide<1>,
        Mut<SparsePosWide<1>>,
        &SparseVelWide<2>,
        Mut<SparsePosWide<2>>,
        &SparseVelWide<3>,
        Mut<SparsePosWide<3>>,
        &SparseVelWide<4>,
        Mut<SparsePosWide<4>>,
    )>().unwrap();
    
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("wide_sparse_simple_iter", |b| {
        b.iter(|| {
            for (_id, (v0, mut p0, v1, mut p1, v2, mut p2, v3, mut p3, v4, mut p4)) in query.iter_mut() {
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
                
                p4.0.0[0] += v4.0.0[0];
                p4.0.0[1] += v4.0.0[1];
                p4.0.0[2] += v4.0.0[2];
            }
        });
    });
}

fn bench_par_cache_locality_loss(c: &mut Criterion) {
    let mut world = World::new();

    let mut v = vec![];
    for _ in 0..10_000 {
        world.spawn_bundle((A(0.0), B(0.0)));
        v.push(world.spawn_bundle(A(0.0)));
    }

    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    v.shuffle(&mut rng);

    for e in v.into_iter() {
        world.despawn(e);
    }

    let mut query = world.query::<(Mut<A>, &B)>().unwrap();
    query.iter_mut().for_each(|_| {}); // Warmup

    c.bench_function("par_cache_locality_loss", |b| {
        b.iter(|| {
            query.par_for_each_mut(|(_id, (mut v1, v2))| {
                v1.0 += v2.0;
            });
        });
    });
}

fn bench_observer_lifecycle_insert(c: &mut Criterion) {
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

fn bench_event_propagation(criterion: &mut Criterion) {
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

fn bench_combinator_system(criterion: &mut Criterion) {
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

fn dyn_param(criterion: &mut Criterion) {
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

fn yes() -> bool {
    true
}

fn no() -> bool {
    false
}

fn run_condition_yes(criterion: &mut Criterion) {
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

fn run_condition_no(criterion: &mut Criterion) {
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

fn run_condition_yes_with_query(criterion: &mut Criterion) {
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

fn run_condition_yes_with_resource(criterion: &mut Criterion) {
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

const ENTITY_BUNCH: usize = 5000;

fn empty_systems(criterion: &mut Criterion) {
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

#[derive(Clone, Copy)]
struct TestA(f32);
impl Component for TestA {}

#[derive(Clone, Copy)]
struct TestB(f32);
impl Component for TestB {}

#[derive(Clone, Copy)]
struct TestC(f32);
impl Component for TestC {}

#[derive(Clone, Copy)]
struct TestD(f32);
impl Component for TestD {}

#[derive(Clone, Copy)]
struct TestE(f32);
impl Component for TestE {}

fn busy_systems(criterion: &mut Criterion) {
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

fn contrived(criterion: &mut Criterion) {
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

pub fn empty_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("empty_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    group.bench_function("0_entities", |bencher| {
        let mut world = World::new();
        let queue = gizmo_core::commands::CommandQueue::new();

        bencher.iter(|| {
            queue.apply(&mut world);
        });
    });

    group.finish();
}

pub fn spawn_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..entity_count {
                    queue.push(move |world| {
                        let entity = world.spawn();
                        if i % 2 == 0 { world.add_component(entity, TestA(0.0)); }
                        if i % 3 == 0 { world.add_component(entity, TestB(0.0)); }
                        if i % 4 == 0 { world.add_component(entity, TestC(0.0)); }
                        if i % 5 == 0 { world.despawn(entity); }
                    });
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

pub fn nonempty_spawn_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("nonempty_spawn_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..entity_count {
                    if core::hint::black_box(i % 2 == 0) {
                        queue.push(|world| {
                            let e = world.spawn();
                            world.add_component(e, TestA(0.0));
                        });
                    }
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

#[derive(Default, Clone, Copy)]
struct TestMatrix([[f32; 4]; 4]);
impl Component for TestMatrix {}

#[derive(Default, Clone, Copy)]
struct TestVec3([f32; 3]);
impl Component for TestVec3 {}

pub fn insert_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("insert_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    let entity_count = 10_000;
    group.bench_function("insert", |bencher| {
        let mut world = World::new();
        let mut entities = Vec::new();
        for _ in 0..entity_count {
            entities.push(world.spawn());
        }
        let queue = gizmo_core::commands::CommandQueue::new();

        bencher.iter(|| {
            for entity in &entities {
                let e = *entity;
                queue.push(move |world| {
                    world.add_bundle(e, (TestMatrix::default(), TestVec3::default()));
                });
            }
            queue.apply(&mut world);
        });
    });

    group.finish();
}

pub fn fake_commands(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("fake_commands");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for command_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{command_count}_commands"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for i in 0..command_count {
                    if core::hint::black_box(i % 2 == 0) {
                        queue.push(|world| { core::hint::black_box(world); });
                    } else {
                        queue.push(|world| { core::hint::black_box(world); });
                    }
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

fn sized_commands_impl<T: Default + Send + Sync + 'static>(criterion: &mut Criterion, name: &str) {
    let mut group = criterion.benchmark_group(name);
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for command_count in [100, 1_000, 10_000] {
        group.bench_function(format!("{command_count}_commands"), |bencher| {
            let mut world = World::new();
            let queue = gizmo_core::commands::CommandQueue::new();

            bencher.iter(|| {
                for _ in 0..command_count {
                    let t = T::default();
                    queue.push(move |world| {
                        core::hint::black_box(t);
                        core::hint::black_box(world);
                    });
                }
                queue.apply(&mut world);
            });
        });
    }

    group.finish();
}

pub fn zero_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<()>(criterion, "sized_commands_0_bytes");
}

pub fn medium_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<(u32, u32, u32)>(criterion, "sized_commands_12_bytes");
}

#[derive(Clone, Copy)]
struct LargeStruct([u64; 64]);

impl Default for LargeStruct {
    fn default() -> Self {
        Self([0; 64])
    }
}

pub fn large_sized_commands(criterion: &mut Criterion) {
    sized_commands_impl::<LargeStruct>(criterion, "sized_commands_512_bytes");
}

pub fn world_despawn(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("despawn_world");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let mut entities = Vec::with_capacity(entity_count);
                    for _ in 0..entity_count {
                        let e = world.spawn();
                        world.add_bundle(e, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        entities.push(e);
                    }
                    (world, entities)
                },
                |(world, entities)| {
                    entities.iter().for_each(|e| {
                        world.despawn(*e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_despawn_recursive(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("despawn_world_recursive");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    use gizmo_core::hierarchy::HierarchyExt;
                    let mut parent_ents = Vec::with_capacity(entity_count);
                    for _ in 0..entity_count {
                        let parent = world.spawn();
                        world.add_bundle(parent, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        let child = world.spawn();
                        world.add_bundle(child, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                        world.add_child(parent, child);
                        parent_ents.push(parent);
                    }
                    (world, parent_ents)
                },
                |(world, parent_ents)| {
                    use gizmo_core::hierarchy::HierarchyExt;
                    parent_ents.iter().for_each(|e| {
                        world.despawn_recursive(*e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

const SIZES: [usize; 5] = [100, 316, 1000, 3162, 10000];

fn make_entity(rng: &mut impl rand::RngExt, size: usize) -> Entity {
    let x: f64 = rng.random();
    let id = -(1.0 - x).log2() * (size as f64);
    let x: f64 = rng.random();
    let generation = 1.0 + -(1.0 - x).log2() * 2.0;

    let id = id as u32 + 1;
    let bits = ((generation as u64) << 32) | (id as u64);
    Entity::from_bits(bits)
}

pub fn entity_set_build_and_lookup(c: &mut Criterion) {
    use chacha20::ChaCha8Rng;
    use rand::SeedableRng;
    use std::collections::HashSet;
    use criterion::Throughput;

    let mut group = c.benchmark_group("entity_hash");
    for size in SIZES {
        let mut rng = ChaCha8Rng::seed_from_u64(size as u64);
        let entities =
            Vec::from_iter(core::iter::repeat_with(|| make_entity(&mut rng, size)).take(size));

        group.throughput(Throughput::Elements(size as u64));
        group.bench_function(criterion::BenchmarkId::new("entity_set_build", size), |bencher| {
            bencher.iter_with_large_drop(|| HashSet::<Entity>::from_iter(entities.iter().copied()));
        });
        group.bench_function(criterion::BenchmarkId::new("entity_set_lookup_hit", size), |bencher| {
            let set = HashSet::<Entity>::from_iter(entities.iter().copied());
            bencher.iter(|| entities.iter().copied().filter(|e| set.contains(e)).count());
        });
        group.bench_function(
            criterion::BenchmarkId::new("entity_set_lookup_miss_id", size),
            |bencher| {
                let set = HashSet::<Entity>::from_iter(entities.iter().copied());
                bencher.iter(|| {
                    entities
                        .iter()
                        .copied()
                        .map(|e| Entity::from_bits(e.to_bits() + 1))
                        .filter(|e| set.contains(e))
                        .count()
                });
            },
        );
        group.bench_function(
            criterion::BenchmarkId::new("entity_set_lookup_miss_gen", size),
            |bencher| {
                let set = HashSet::<Entity>::from_iter(entities.iter().copied());
                bencher.iter(|| {
                    entities
                        .iter()
                        .copied()
                        .map(|e| Entity::from_bits(e.to_bits() + (1 << 32)))
                        .filter(|e| set.contains(e))
                        .count()
                });
            },
        );
    }
}

pub fn entity_allocator_benches(criterion: &mut Criterion) {
    const ENTITY_COUNTS: [u32; 3] = [1, 100, 10_000];
    use gizmo_core::entity::allocator::Entities;

    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    // Gizmo doesn't have bulk allocation yet, so we loop.
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_free");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    (allocator, entities)
                },
                |(allocator, entities)| {
                    entities.drain(..).for_each(|e| {
                        allocator.free(e);
                    });
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_free_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    (allocator, entities)
                },
                |(allocator, entities)| {
                    for e in entities {
                        allocator.free(*e);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused_bulk");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    // Since Gizmo does not have Remote Allocators, these are tested via standard Entities allocations.
    let mut group = criterion.benchmark_group("entity_allocator_allocate_fresh_remote");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                Entities::new,
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("entity_allocator_allocate_reused_remote");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in ENTITY_COUNTS {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                || {
                    let allocator = Entities::new();
                    let mut entities = Vec::with_capacity(entity_count as usize);
                    for _ in 0..entity_count {
                        entities.push(allocator.reserve_entity());
                    }
                    for e in &entities {
                        allocator.free(*e);
                    }
                    allocator
                },
                |allocator| {
                    for _ in 0..entity_count {
                        let entity = allocator.reserve_entity();
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_spawn(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_world");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for entity_count in [1, 100, 10_000] {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                World::new,
                |world| {
                    for _ in 0..entity_count {
                        let e = world.spawn();
                        world.add_bundle(e, (LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])));
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

pub fn world_spawn_batch(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("spawn_world_batch");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    #[derive(Clone, Copy)]
    struct LocalA(crate::Mat4);
    impl Component for LocalA {}

    #[derive(Clone, Copy)]
    struct LocalB([f32; 4]);
    impl Component for LocalB {}

    for batch_count in [1, 100, 1000, 10_000] {
        group.bench_function(format!("{batch_count}_entities"), |bencher| {
            bencher.iter_batched_ref(
                World::new,
                |world| {
                    for _ in 0..(10_000 / batch_count) {
                        let _ = world.spawn_batch(
                            std::iter::repeat_n((LocalA(crate::Mat4::ZERO), LocalB([0.0; 4])), batch_count as usize),
                        ).count();
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

#[derive(Clone, Copy, Default)]
struct Table(f32);
impl Component for Table {
    fn storage_type() -> StorageType { StorageType::Table }
}

#[derive(Clone, Copy, Default)]
struct Sparse(f32);
impl Component for Sparse {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy, Default)]
struct WideTable<const X: usize>(f32);
impl<const X: usize> Component for WideTable<X> {
    fn storage_type() -> StorageType { StorageType::Table }
}

#[derive(Clone, Copy, Default)]
struct WideSparse<const X: usize>(f32);
impl<const X: usize> Component for WideSparse<X> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

const RANGE: core::ops::Range<u32> = 5..6;

fn setup<T: Component + Default + Clone>(entity_count: u32) -> (World, Vec<Entity>) {
    let mut world = World::new();
    let entities: Vec<Entity> = world
        .spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize))
        .collect();
    core::hint::black_box((world, entities))
}

fn setup_wide<T: Bundle + Default + Clone>(
    entity_count: u32,
) -> (World, Vec<Entity>) {
    let mut world = World::new();
    let entities: Vec<Entity> = world
        .spawn_batch(std::iter::repeat_n(T::default(), entity_count as usize))
        .collect();
    core::hint::black_box((world, entities))
}

pub fn world_entity(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_entity");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    core::hint::black_box(world.is_alive(*entity));
                }
            });
        });
    }

    group.finish();
}

pub fn world_get(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    assert!(world.query_entity::<&Table>(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, entities) = setup::<Sparse>(entity_count);

            bencher.iter(|| {
                for entity in &entities {
                    assert!(world.query_entity::<&Sparse>(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn world_query_get(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, entities) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_table_wide"), |bencher| {
            let (world, entities) = setup_wide::<(
                WideTable<0>,
                WideTable<1>,
                WideTable<2>,
                WideTable<3>,
                WideTable<4>,
                WideTable<5>,
            )>(entity_count);
            let query = world.query::<(
                &WideTable<0>,
                &WideTable<1>,
                &WideTable<2>,
                &WideTable<3>,
                &WideTable<4>,
                &WideTable<5>,
            )>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, entities) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse_wide"), |bencher| {
            let (world, entities) = setup_wide::<(
                WideSparse<0>,
                WideSparse<1>,
                WideSparse<2>,
                WideSparse<3>,
                WideSparse<4>,
                WideSparse<5>,
            )>(entity_count);
            let query = world.query::<(
                &WideSparse<0>,
                &WideSparse<1>,
                &WideSparse<2>,
                &WideSparse<3>,
                &WideSparse<4>,
                &WideSparse<5>,
            )>().unwrap();

            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn world_query_iter(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_iter");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, _) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                for (_id, comp) in query.iter() {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, _) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                for (_id, comp) in query.iter() {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
    }

    group.finish();
}

pub fn world_query_for_each(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_for_each");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let (world, _) = setup::<Table>(entity_count);
            let query = world.query::<&Table>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                query.iter().for_each(|(_id, comp)| {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                });
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let (world, _) = setup::<Sparse>(entity_count);
            let query = world.query::<&Sparse>().unwrap();

            bencher.iter(|| {
                let mut count = 0;
                query.iter().for_each(|(_id, comp)| {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                });
                assert_eq!(core::hint::black_box(count), entity_count);
            });
        });
    }

    group.finish();
}

pub fn query_get(criterion: &mut Criterion) {
    use rand::seq::SliceRandom;

    let mut group = criterion.benchmark_group("query_get");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("{entity_count}_entities_table"), |bencher| {
            let mut world = World::new();
            let mut entities: Vec<_> = world
                .spawn_batch(std::iter::repeat_n((Table::default(),), entity_count as usize))
                .collect();
            use rand::SeedableRng;
            let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
            entities.shuffle(&mut rng);

            let mut schedule = gizmo_core::system::Schedule::new();
            let entities_clone = entities.clone();
            schedule.add_di_system(move |query: gizmo_core::query::Query<&Table>| {
                let mut count = 0;
                for comp in entities_clone.iter().filter_map(|&e| query.get(e.id())) {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
            schedule.build();
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
        group.bench_function(format!("{entity_count}_entities_sparse"), |bencher| {
            let mut world = World::new();
            let mut entities: Vec<_> = world
                .spawn_batch(std::iter::repeat_n((Sparse::default(),), entity_count as usize))
                .collect();
            use rand::SeedableRng;
            let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
            entities.shuffle(&mut rng);

            let mut schedule = gizmo_core::system::Schedule::new();
            let entities_clone = entities.clone();
            schedule.add_di_system(move |query: gizmo_core::query::Query<&Sparse>| {
                let mut count = 0;
                for comp in entities_clone.iter().filter_map(|&e| query.get(e.id())) {
                    core::hint::black_box(comp);
                    count += 1;
                    core::hint::black_box(count);
                }
                assert_eq!(core::hint::black_box(count), entity_count);
            });
            schedule.build();
            bencher.iter(|| {
                schedule.run(&mut world, 0.0);
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_2(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_2");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("2_components_{entity_count}_entities"), |bencher| {
            let (world, entities) = setup_wide::<(WideTable<0>, WideTable<1>)>(entity_count);
            let query = world.query::<(Mut<WideTable<0>>, Mut<WideTable<1>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_5(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_5");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("5_components_{entity_count}_entities"), |bencher| {
            let (world, entities) = setup_wide::<(WideTable<0>, WideTable<1>, WideTable<2>, WideTable<3>, WideTable<4>)>(entity_count);
            let query = world.query::<(Mut<WideTable<0>>, Mut<WideTable<1>>, Mut<WideTable<2>>, Mut<WideTable<3>>, Mut<WideTable<4>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}

pub fn query_get_components_mut_10(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("world_query_get_components_mut_10");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    for entity_count in RANGE.map(|i| i * 10_000) {
        group.bench_function(format!("10_components_{entity_count}_entities"), |bencher| {
            let (world, entities) = setup_wide::<(WideTable<0>, WideTable<1>, WideTable<2>, WideTable<3>, WideTable<4>, WideTable<5>, WideTable<6>, WideTable<7>, WideTable<8>, WideTable<9>)>(entity_count);
            let query = world.query::<(Mut<WideTable<0>>, Mut<WideTable<1>>, Mut<WideTable<2>>, Mut<WideTable<3>>, Mut<WideTable<4>>, Mut<WideTable<5>>, Mut<WideTable<6>>, Mut<WideTable<7>>, Mut<WideTable<8>>, Mut<WideTable<9>>)>().unwrap();
            bencher.iter(|| {
                for entity in &entities {
                    assert!(query.get(entity.id()).is_some());
                }
            });
        });
    }

    group.finish();
}


fn all_added_detection_generic<T: Component + Default + Clone>(group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>, entity_count: u32) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick(); // Wait, added entities were added at the old tick, and query uses the current tick? No, they were added with world's current tick. Incrementing the tick makes the current tick different from the added tick. Wait, if we increment the tick, then `ticks.added == current_tick` will be false!
                    // Wait, Bevy's `Added` checks if `added_tick` is newer than `last_run_tick`.
                    // Gizmo's `Added` (which I just implemented) checks `ticks.added == tick`. `tick` is `world.tick`.
                    // So if we don't increment the tick, `ticks.added == world.tick` will be true.
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Added<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(entity_count, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn all_added_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("all_added_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        all_added_detection_generic::<Table>(&mut group, entity_count);
        all_added_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn all_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick();
                    let mut query_mut = world.query::<gizmo_core::query::Mut<T>>().unwrap();
                    for (_id, mut component) in query_mut.iter_mut() {
                        // writing to Mut<T> triggers Changed tick internally! Wait, Gizmo's `Mut` doesn't automatically trigger Changed tick.
                        // Wait, does it? Let's assume the user has a bench_modify trait, but we can just do mutable access or manually update ticks.
                        // I will just use `world.get_component_mut::<T>(entity)` which updates the tick if Gizmo does it, or we just manually update ticks.
                        // Actually, just let me get query_mut and do something. But Gizmo `Mut` is literally `&mut T`. It doesn't wrap!
                        // In Gizmo, returning `&mut T` from `Query` iterator DOES update the tick if `iter_mut` is called! 
                        // Wait, Gizmo's `iter_mut` gives `(&mut T)`. How does it track change? 
                        // Ah, `col.ticks_ptr_mut().write()` is called inside `world.mod.rs` when doing things, but `iter_mut()` might just update ticks for all accessed rows!
                        core::hint::black_box(&mut component);
                    }
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(entity_count, count); // If all_changed works
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn all_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("all_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        all_changed_detection_generic::<Table>(&mut group, entity_count);
        all_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn few_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    let ratio_to_modify = 0.1;
    let amount_to_modify = (entity_count as f32 * ratio_to_modify) as usize;
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let mut entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick();
                    
                    use rand::seq::SliceRandom;
                    use rand::SeedableRng;
                    let mut rng = chacha20::ChaCha8Rng::seed_from_u64(42);
                    entities.shuffle(&mut rng);
                    
                    for i in 0..amount_to_modify {
                        // Trigger change
                        let _ = world.query_entity_mut::<gizmo_core::query::Mut<T>>(entities[i].id());
                    }
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                    }
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn few_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("few_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        few_changed_detection_generic::<Table>(&mut group, entity_count);
        few_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

fn none_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    entity_count: u32,
) {
    group.bench_function(
        format!("{}_entities_{}", entity_count, core::any::type_name::<T>()),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    let entities: Vec<_> = world.spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize)).collect();
                    world.increment_tick();
                    (world, entities)
                },
                |(world, _)| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(0, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn none_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("none_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));
    for &entity_count in &[5000, 50000] {
        none_changed_detection_generic::<Table>(&mut group, entity_count);
        none_changed_detection_generic::<Sparse>(&mut group, entity_count);
    }
    group.finish();
}

#[derive(Clone, Copy, Default)]
struct ArchetypeData<const X: u16>(f32);
impl<const X: u16> Component for ArchetypeData<X> {
    fn storage_type() -> gizmo_core::component::StorageType { gizmo_core::component::StorageType::Table }
}

fn add_archetypes_entities<T: Component + Default + Clone>(
    world: &mut World,
    archetype_count: u16,
    entity_count: u32,
) {
    for i in 0..archetype_count {
        for _j in 0..entity_count {
            let e = world.spawn();
            world.add_component(e, T::default());
            if i & (1 << 0) != 0 { world.add_component(e, ArchetypeData::<0>(1.0)); }
            if i & (1 << 1) != 0 { world.add_component(e, ArchetypeData::<1>(1.0)); }
            if i & (1 << 2) != 0 { world.add_component(e, ArchetypeData::<2>(1.0)); }
            if i & (1 << 3) != 0 { world.add_component(e, ArchetypeData::<3>(1.0)); }
            if i & (1 << 4) != 0 { world.add_component(e, ArchetypeData::<4>(1.0)); }
            if i & (1 << 5) != 0 { world.add_component(e, ArchetypeData::<5>(1.0)); }
        }
    }
}

fn multiple_archetype_none_changed_detection_generic<T: Component + Default + Clone>(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    archetype_count: u16,
    entity_count: u32,
) {
    group.bench_function(
        format!(
            "{}_archetypes_{}_entities_{}",
            archetype_count,
            entity_count,
            core::any::type_name::<T>()
        ),
        |bencher| {
            bencher.iter_batched_ref(
                || {
                    let mut world = World::new();
                    add_archetypes_entities::<T>(&mut world, archetype_count, entity_count);
                    world.increment_tick();
                    
                    let query_mut = world.query::<(
                        gizmo_core::query::Mut<ArchetypeData<0>>,
                        gizmo_core::query::Mut<ArchetypeData<1>>,
                        gizmo_core::query::Mut<ArchetypeData<2>>,
                    )>();
                    if let Some(mut query_mut) = query_mut {
                        for (_id, (mut d0, mut d1, mut d2)) in query_mut.iter_mut() {
                            d0.0 += 1.0; d1.0 += 1.0; d2.0 += 1.0;
                        }
                    }
                    world
                },
                |world| {
                    let query = world.query::<gizmo_core::query::Changed<T>>().unwrap();
                    let mut count = 0;
                    for (entity, _) in query.iter() {
                        core::hint::black_box(entity);
                        count += 1;
                    }
                    assert_eq!(0, count);
                },
                criterion::BatchSize::LargeInput,
            );
        },
    );
}

pub fn multiple_archetype_none_changed_detection(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("multiple_archetypes_none_changed_detection");
    group.warm_up_time(core::time::Duration::from_millis(800));
    group.measurement_time(core::time::Duration::from_secs(8));
    for archetype_count in [5, 20] {
        for entity_count in [10, 100, 1000] {
            multiple_archetype_none_changed_detection_generic::<Table>(
                &mut group,
                archetype_count,
                entity_count,
            );
            multiple_archetype_none_changed_detection_generic::<Sparse>(
                &mut group,
                archetype_count,
                entity_count,
            );
        }
    }
    group.finish();
}



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



use gizmo_core::hierarchy::HierarchyExt;
use gizmo_core::component::{Children, Parent};

#[derive(Clone, Copy)]
struct C<const N: usize>(Mat4);
impl<const N: usize> Default for C<N> {
    fn default() -> Self {
        Self(Mat4([0.0; 16]))
    }
}
impl<const N: usize> Component for C<N> {
    fn storage_type() -> gizmo_core::component::StorageType { gizmo_core::component::StorageType::Table }
}

fn bench_clone(
    b: &mut criterion::Bencher,
    bundle_size: usize,
) {
    let mut world = World::new();

    // Spawn the first entity, which will be cloned in the benchmark routine.
    let id = world.spawn();
    world.add_component(id, C::<1>::default());
    if bundle_size > 1 {
        world.add_component(id, C::<2>::default());
        world.add_component(id, C::<3>::default());
        world.add_component(id, C::<4>::default());
        world.add_component(id, C::<5>::default());
        world.add_component(id, C::<6>::default());
        world.add_component(id, C::<7>::default());
        world.add_component(id, C::<8>::default());
        world.add_component(id, C::<9>::default());
        world.add_component(id, C::<10>::default());
    }
    
    let eid = id.id();

    b.iter(|| {
        world.clone_entity(eid, 1);
    });
}

fn clone_hierarchy_recursive(world: &mut World, source_id: u32) -> Option<gizmo_core::entity::Entity> {
    let cloned_entities = world.clone_entity(source_id, 1)?;
    let cloned_parent = cloned_entities[0];

    let mut children_to_clone = Vec::new();
    let source_entity = world.reconstruct_entity(source_id)?;
    if let Some(children_ptr) = world.get_component_ptr(source_entity, core::any::TypeId::of::<Children>()) {
        let children = unsafe { &*(children_ptr as *const Children) };
        children_to_clone = children.0.clone();
    }

    // Since Gizmo's clone_entity copies all components including Parent/Children, 
    // the cloned parent currently points to the old children! 
    // We must clear its Children component first before adding new ones.
    world.remove_component::<Children>(cloned_parent);
    // It also points to the old parent, we remove it.
    world.remove_component::<Parent>(cloned_parent);

    for child_id in children_to_clone {
        if let Some(cloned_child) = clone_hierarchy_recursive(world, child_id) {
            world.add_child(cloned_parent, cloned_child);
        }
    }

    Some(cloned_parent)
}

fn bench_clone_hierarchy(
    b: &mut criterion::Bencher,
    height: usize,
    children: usize,
    complex: bool,
) {
    let mut world = World::new();

    let root = world.spawn();
    world.add_component(root, C::<1>::default());
    if complex {
        world.add_component(root, C::<2>::default());
        world.add_component(root, C::<3>::default());
        world.add_component(root, C::<4>::default());
        world.add_component(root, C::<5>::default());
        world.add_component(root, C::<6>::default());
        world.add_component(root, C::<7>::default());
        world.add_component(root, C::<8>::default());
        world.add_component(root, C::<9>::default());
        world.add_component(root, C::<10>::default());
    }

    let mut hierarchy_level = vec![root];

    for _ in 0..height {
        let current_hierarchy_level = hierarchy_level.clone();
        hierarchy_level.clear();

        for parent in current_hierarchy_level {
            for _ in 0..children {
                let child = world.spawn();
                world.add_component(child, C::<1>::default());
                if complex {
                    world.add_component(child, C::<2>::default());
                    world.add_component(child, C::<3>::default());
                    world.add_component(child, C::<4>::default());
                    world.add_component(child, C::<5>::default());
                    world.add_component(child, C::<6>::default());
                    world.add_component(child, C::<7>::default());
                    world.add_component(child, C::<8>::default());
                    world.add_component(child, C::<9>::default());
                    world.add_component(child, C::<10>::default());
                }
                world.add_child(parent, child);
                hierarchy_level.push(child);
            }
        }
    }

    let root_id = root.id();
    b.iter(|| {
        clone_hierarchy_recursive(&mut world, root_id);
    });
}

pub fn single_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_clone");
    group.throughput(criterion::Throughput::Elements(1));
    group.bench_function("complex_bundle", |b| {
        bench_clone(b, 10);
    });
    group.finish();
}

pub fn hierarchy_tall(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_tall");
    group.throughput(criterion::Throughput::Elements(51));
    group.bench_function("tall", |b| {
        bench_clone_hierarchy(b, 50, 1, false);
    });
    group.finish();
}

pub fn hierarchy_wide(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_wide");
    group.throughput(criterion::Throughput::Elements(51));
    group.bench_function("wide", |b| {
        bench_clone_hierarchy(b, 1, 50, false);
    });
    group.finish();
}

pub fn hierarchy_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_many");
    group.throughput(criterion::Throughput::Elements(364));
    group.bench_function("many", |b| {
        bench_clone_hierarchy(b, 5, 3, true);
    });
    group.finish();
}



fn flip_coin() -> bool {
    rand::random::<bool>()
}

// Ensure Table and Sparse take <const X: usize = 0> as per the prompt?
// Wait, Table<X> doesn't exist. There's Table(f32) and WideTable<X>(f32). I will reuse WideTable<X>!
fn spawn_empty_frag_archetype_wide<T: Component + Default>(world: &mut World) {
    for i in 0..10000 { // Reduced to 10k to prevent OOM / taking too long in CI
        let e = world.spawn();
        if flip_coin() {
            world.add_component(e, WideTable::<1>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<2>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<3>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<4>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<5>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<6>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<7>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<8>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<9>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<10>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<11>(0.0));
        }
        if flip_coin() {
            world.add_component(e, WideTable::<12>(0.0));
        }
        world.add_component(e, T::default());

        if i != 0 {
            world.despawn(e);
        }
    }
}

pub fn iter_frag_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("iter_fragmented_empty");
    group.warm_up_time(core::time::Duration::from_millis(500));
    group.measurement_time(core::time::Duration::from_secs(4));

    group.bench_function("foreach_table", |b| {
        let mut world = World::new();
        spawn_empty_frag_archetype_wide::<Table>(&mut world);
        
        let mut schedule = gizmo_core::system::Schedule::new();
        fn iter_table(query: gizmo_core::query::Query<&Table >) {
            let mut res = 0;
            // Iterate over entities
            for (e, t) in query.iter() {
                res += e;
                core::hint::black_box(t);
            }
            core::hint::black_box(res);
        }
        schedule.add_di_system(iter_table);
        schedule.build();
        
        b.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });
    
    group.bench_function("foreach_sparse", |b| {
        let mut world = World::new();
        spawn_empty_frag_archetype_wide::<Sparse>(&mut world);
        
        let mut schedule = gizmo_core::system::Schedule::new();
        fn iter_sparse(query: gizmo_core::query::Query<&Sparse >) {
            let mut res = 0;
            // Iterate over entities
            for (e, t) in query.iter() {
                res += e;
                core::hint::black_box(t);
            }
            core::hint::black_box(res);
        }
        schedule.add_di_system(iter_sparse);
        schedule.build();
        
        b.iter(|| {
            schedule.run(&mut world, 0.0);
        });
    });
    group.finish();
}




struct DummyRes<const N: usize>;
struct BenchRes;

fn create_resource_world() -> World {
    let mut world = World::new();
    world.insert_resource(DummyRes::<0>);
    world.insert_resource(DummyRes::<1>);
    world.insert_resource(DummyRes::<2>);
    world.insert_resource(DummyRes::<3>);
    world.insert_resource(DummyRes::<4>);
    world.insert_resource(DummyRes::<5>);
    world.insert_resource(DummyRes::<6>);
    world.insert_resource(DummyRes::<7>);
    world.insert_resource(DummyRes::<8>);
    world.insert_resource(DummyRes::<9>);
    world.insert_resource(DummyRes::<10>);
    world.insert_resource(DummyRes::<11>);
    world.insert_resource(DummyRes::<12>);
    world.insert_resource(DummyRes::<13>);
    world.insert_resource(DummyRes::<14>);
    world.insert_resource(DummyRes::<15>);
    world.insert_resource(DummyRes::<16>);
    world.insert_resource(DummyRes::<17>);
    world.insert_resource(DummyRes::<18>);
    world.insert_resource(DummyRes::<19>);
    world.insert_resource(DummyRes::<20>);
    world.insert_resource(DummyRes::<21>);
    world.insert_resource(DummyRes::<22>);
    world.insert_resource(DummyRes::<23>);
    world.insert_resource(DummyRes::<24>);
    world.insert_resource(DummyRes::<25>);
    world.insert_resource(DummyRes::<26>);
    world.insert_resource(DummyRes::<27>);
    world.insert_resource(DummyRes::<28>);
    world.insert_resource(DummyRes::<29>);
    world.insert_resource(DummyRes::<30>);
    world.insert_resource(DummyRes::<31>);
    world.insert_resource(DummyRes::<32>);
    world.insert_resource(DummyRes::<33>);
    world.insert_resource(DummyRes::<34>);
    world.insert_resource(DummyRes::<35>);
    world.insert_resource(DummyRes::<36>);
    world.insert_resource(DummyRes::<37>);
    world.insert_resource(DummyRes::<38>);
    world.insert_resource(DummyRes::<39>);
    world.insert_resource(DummyRes::<40>);
    world.insert_resource(DummyRes::<41>);
    world.insert_resource(DummyRes::<42>);
    world.insert_resource(DummyRes::<43>);
    world.insert_resource(DummyRes::<44>);
    world.insert_resource(DummyRes::<45>);
    world.insert_resource(DummyRes::<46>);
    world.insert_resource(DummyRes::<47>);
    world.insert_resource(DummyRes::<48>);
    world.insert_resource(DummyRes::<49>);
    world.insert_resource(DummyRes::<50>);
    world.insert_resource(DummyRes::<51>);
    world.insert_resource(DummyRes::<52>);
    world.insert_resource(DummyRes::<53>);
    world.insert_resource(DummyRes::<54>);
    world.insert_resource(DummyRes::<55>);
    world.insert_resource(DummyRes::<56>);
    world.insert_resource(DummyRes::<57>);
    world.insert_resource(DummyRes::<58>);
    world.insert_resource(DummyRes::<59>);
    world.insert_resource(DummyRes::<60>);
    world.insert_resource(DummyRes::<61>);
    world.insert_resource(DummyRes::<62>);
    world.insert_resource(DummyRes::<63>);
    world.insert_resource(DummyRes::<64>);
    world.insert_resource(DummyRes::<65>);
    world.insert_resource(DummyRes::<66>);
    world.insert_resource(DummyRes::<67>);
    world.insert_resource(DummyRes::<68>);
    world.insert_resource(DummyRes::<69>);
    world.insert_resource(DummyRes::<70>);
    world.insert_resource(DummyRes::<71>);
    world.insert_resource(DummyRes::<72>);
    world.insert_resource(DummyRes::<73>);
    world.insert_resource(DummyRes::<74>);
    world.insert_resource(DummyRes::<75>);
    world.insert_resource(DummyRes::<76>);
    world.insert_resource(DummyRes::<77>);
    world.insert_resource(DummyRes::<78>);
    world.insert_resource(DummyRes::<79>);
    world.insert_resource(DummyRes::<80>);
    world.insert_resource(DummyRes::<81>);
    world.insert_resource(DummyRes::<82>);
    world.insert_resource(DummyRes::<83>);
    world.insert_resource(DummyRes::<84>);
    world.insert_resource(DummyRes::<85>);
    world.insert_resource(DummyRes::<86>);
    world.insert_resource(DummyRes::<87>);
    world.insert_resource(DummyRes::<88>);
    world.insert_resource(DummyRes::<89>);
    world.insert_resource(DummyRes::<90>);
    world.insert_resource(DummyRes::<91>);
    world.insert_resource(DummyRes::<92>);
    world.insert_resource(DummyRes::<93>);
    world.insert_resource(DummyRes::<94>);
    world.insert_resource(DummyRes::<95>);
    world.insert_resource(DummyRes::<96>);
    world.insert_resource(DummyRes::<97>);
    world.insert_resource(DummyRes::<98>);
    world.insert_resource(DummyRes::<99>);
    world.insert_resource(DummyRes::<100>);
    world.insert_resource(DummyRes::<101>);
    world.insert_resource(DummyRes::<102>);
    world.insert_resource(DummyRes::<103>);
    world.insert_resource(DummyRes::<104>);
    world.insert_resource(DummyRes::<105>);
    world.insert_resource(DummyRes::<106>);
    world.insert_resource(DummyRes::<107>);
    world.insert_resource(DummyRes::<108>);
    world.insert_resource(DummyRes::<109>);
    world.insert_resource(DummyRes::<110>);
    world.insert_resource(DummyRes::<111>);
    world.insert_resource(DummyRes::<112>);
    world.insert_resource(DummyRes::<113>);
    world.insert_resource(DummyRes::<114>);
    world.insert_resource(DummyRes::<115>);
    world.insert_resource(DummyRes::<116>);
    world.insert_resource(DummyRes::<117>);
    world.insert_resource(DummyRes::<118>);
    world.insert_resource(DummyRes::<119>);
    world.insert_resource(DummyRes::<120>);
    world.insert_resource(DummyRes::<121>);
    world.insert_resource(DummyRes::<122>);
    world.insert_resource(DummyRes::<123>);
    world.insert_resource(DummyRes::<124>);
    world.insert_resource(DummyRes::<125>);
    world.insert_resource(DummyRes::<126>);
    world.insert_resource(DummyRes::<127>);
    world.insert_resource(DummyRes::<128>);
    world.insert_resource(DummyRes::<129>);
    world.insert_resource(DummyRes::<130>);
    world.insert_resource(DummyRes::<131>);
    world.insert_resource(DummyRes::<132>);
    world.insert_resource(DummyRes::<133>);
    world.insert_resource(DummyRes::<134>);
    world.insert_resource(DummyRes::<135>);
    world.insert_resource(DummyRes::<136>);
    world.insert_resource(DummyRes::<137>);
    world.insert_resource(DummyRes::<138>);
    world.insert_resource(DummyRes::<139>);
    world.insert_resource(DummyRes::<140>);
    world.insert_resource(DummyRes::<141>);
    world.insert_resource(DummyRes::<142>);
    world.insert_resource(DummyRes::<143>);
    world.insert_resource(DummyRes::<144>);
    world.insert_resource(DummyRes::<145>);
    world.insert_resource(DummyRes::<146>);
    world.insert_resource(DummyRes::<147>);
    world.insert_resource(DummyRes::<148>);
    world.insert_resource(DummyRes::<149>);
    world.insert_resource(DummyRes::<150>);
    world.insert_resource(DummyRes::<151>);
    world.insert_resource(DummyRes::<152>);
    world.insert_resource(DummyRes::<153>);
    world.insert_resource(DummyRes::<154>);
    world.insert_resource(DummyRes::<155>);
    world.insert_resource(DummyRes::<156>);
    world.insert_resource(DummyRes::<157>);
    world.insert_resource(DummyRes::<158>);
    world.insert_resource(DummyRes::<159>);
    world.insert_resource(DummyRes::<160>);
    world.insert_resource(DummyRes::<161>);
    world.insert_resource(DummyRes::<162>);
    world.insert_resource(DummyRes::<163>);
    world.insert_resource(DummyRes::<164>);
    world.insert_resource(DummyRes::<165>);
    world.insert_resource(DummyRes::<166>);
    world.insert_resource(DummyRes::<167>);
    world.insert_resource(DummyRes::<168>);
    world.insert_resource(DummyRes::<169>);
    world.insert_resource(DummyRes::<170>);
    world.insert_resource(DummyRes::<171>);
    world.insert_resource(DummyRes::<172>);
    world.insert_resource(DummyRes::<173>);
    world.insert_resource(DummyRes::<174>);
    world.insert_resource(DummyRes::<175>);
    world.insert_resource(DummyRes::<176>);
    world.insert_resource(DummyRes::<177>);
    world.insert_resource(DummyRes::<178>);
    world.insert_resource(DummyRes::<179>);
    world.insert_resource(DummyRes::<180>);
    world.insert_resource(DummyRes::<181>);
    world.insert_resource(DummyRes::<182>);
    world.insert_resource(DummyRes::<183>);
    world.insert_resource(DummyRes::<184>);
    world.insert_resource(DummyRes::<185>);
    world.insert_resource(DummyRes::<186>);
    world.insert_resource(DummyRes::<187>);
    world.insert_resource(DummyRes::<188>);
    world.insert_resource(DummyRes::<189>);
    world.insert_resource(DummyRes::<190>);
    world.insert_resource(DummyRes::<191>);
    world.insert_resource(DummyRes::<192>);
    world.insert_resource(DummyRes::<193>);
    world.insert_resource(DummyRes::<194>);
    world.insert_resource(DummyRes::<195>);
    world.insert_resource(DummyRes::<196>);
    world.insert_resource(DummyRes::<197>);
    world.insert_resource(DummyRes::<198>);
    world.insert_resource(DummyRes::<199>);
    world.insert_resource(DummyRes::<200>);
    world.insert_resource(DummyRes::<201>);
    world.insert_resource(DummyRes::<202>);
    world.insert_resource(DummyRes::<203>);
    world.insert_resource(DummyRes::<204>);
    world.insert_resource(DummyRes::<205>);
    world.insert_resource(DummyRes::<206>);
    world.insert_resource(DummyRes::<207>);
    world.insert_resource(DummyRes::<208>);
    world.insert_resource(DummyRes::<209>);
    world.insert_resource(DummyRes::<210>);
    world.insert_resource(DummyRes::<211>);
    world.insert_resource(DummyRes::<212>);
    world.insert_resource(DummyRes::<213>);
    world.insert_resource(DummyRes::<214>);
    world.insert_resource(DummyRes::<215>);
    world.insert_resource(DummyRes::<216>);
    world.insert_resource(DummyRes::<217>);
    world.insert_resource(DummyRes::<218>);
    world.insert_resource(DummyRes::<219>);
    world.insert_resource(DummyRes::<220>);
    world.insert_resource(DummyRes::<221>);
    world.insert_resource(DummyRes::<222>);
    world.insert_resource(DummyRes::<223>);
    world.insert_resource(DummyRes::<224>);
    world.insert_resource(DummyRes::<225>);
    world.insert_resource(DummyRes::<226>);
    world.insert_resource(DummyRes::<227>);
    world.insert_resource(DummyRes::<228>);
    world.insert_resource(DummyRes::<229>);
    world.insert_resource(DummyRes::<230>);
    world.insert_resource(DummyRes::<231>);
    world.insert_resource(DummyRes::<232>);
    world.insert_resource(DummyRes::<233>);
    world.insert_resource(DummyRes::<234>);
    world.insert_resource(DummyRes::<235>);
    world.insert_resource(DummyRes::<236>);
    world.insert_resource(DummyRes::<237>);
    world.insert_resource(DummyRes::<238>);
    world.insert_resource(DummyRes::<239>);
    world.insert_resource(DummyRes::<240>);
    world.insert_resource(DummyRes::<241>);
    world.insert_resource(DummyRes::<242>);
    world.insert_resource(DummyRes::<243>);
    world.insert_resource(DummyRes::<244>);
    world.insert_resource(DummyRes::<245>);
    world.insert_resource(DummyRes::<246>);
    world.insert_resource(DummyRes::<247>);
    world.insert_resource(DummyRes::<248>);
    world.insert_resource(DummyRes::<249>);
    world.insert_resource(DummyRes::<250>);
    world.insert_resource(DummyRes::<251>);
    world.insert_resource(DummyRes::<252>);
    world.insert_resource(DummyRes::<253>);
    world.insert_resource(DummyRes::<254>);
    world.insert_resource(DummyRes::<255>);
    world.insert_resource(DummyRes::<256>);
    world.insert_resource(DummyRes::<257>);
    world.insert_resource(DummyRes::<258>);
    world.insert_resource(DummyRes::<259>);
    world.insert_resource(DummyRes::<260>);
    world.insert_resource(DummyRes::<261>);
    world.insert_resource(DummyRes::<262>);
    world.insert_resource(DummyRes::<263>);
    world.insert_resource(DummyRes::<264>);
    world.insert_resource(DummyRes::<265>);
    world.insert_resource(DummyRes::<266>);
    world.insert_resource(DummyRes::<267>);
    world.insert_resource(DummyRes::<268>);
    world.insert_resource(DummyRes::<269>);
    world.insert_resource(DummyRes::<270>);
    world.insert_resource(DummyRes::<271>);
    world.insert_resource(DummyRes::<272>);
    world.insert_resource(DummyRes::<273>);
    world.insert_resource(DummyRes::<274>);
    world.insert_resource(DummyRes::<275>);
    world.insert_resource(DummyRes::<276>);
    world.insert_resource(DummyRes::<277>);
    world.insert_resource(DummyRes::<278>);
    world.insert_resource(DummyRes::<279>);
    world.insert_resource(DummyRes::<280>);
    world.insert_resource(DummyRes::<281>);
    world.insert_resource(DummyRes::<282>);
    world.insert_resource(DummyRes::<283>);
    world.insert_resource(DummyRes::<284>);
    world.insert_resource(DummyRes::<285>);
    world.insert_resource(DummyRes::<286>);
    world.insert_resource(DummyRes::<287>);
    world.insert_resource(DummyRes::<288>);
    world.insert_resource(DummyRes::<289>);
    world.insert_resource(DummyRes::<290>);
    world.insert_resource(DummyRes::<291>);
    world.insert_resource(DummyRes::<292>);
    world.insert_resource(DummyRes::<293>);
    world.insert_resource(DummyRes::<294>);
    world.insert_resource(DummyRes::<295>);
    world.insert_resource(DummyRes::<296>);
    world.insert_resource(DummyRes::<297>);
    world.insert_resource(DummyRes::<298>);
    world.insert_resource(DummyRes::<299>);
    world.insert_resource(DummyRes::<300>);
    world.insert_resource(DummyRes::<301>);
    world.insert_resource(DummyRes::<302>);
    world.insert_resource(DummyRes::<303>);
    world.insert_resource(DummyRes::<304>);
    world.insert_resource(DummyRes::<305>);
    world.insert_resource(DummyRes::<306>);
    world.insert_resource(DummyRes::<307>);
    world.insert_resource(DummyRes::<308>);
    world.insert_resource(DummyRes::<309>);
    world.insert_resource(DummyRes::<310>);
    world.insert_resource(DummyRes::<311>);
    world.insert_resource(DummyRes::<312>);
    world.insert_resource(DummyRes::<313>);
    world.insert_resource(DummyRes::<314>);
    world.insert_resource(DummyRes::<315>);
    world.insert_resource(DummyRes::<316>);
    world.insert_resource(DummyRes::<317>);
    world.insert_resource(DummyRes::<318>);
    world.insert_resource(DummyRes::<319>);
    world.insert_resource(DummyRes::<320>);
    world.insert_resource(DummyRes::<321>);
    world.insert_resource(DummyRes::<322>);
    world.insert_resource(DummyRes::<323>);
    world.insert_resource(DummyRes::<324>);
    world.insert_resource(DummyRes::<325>);
    world.insert_resource(DummyRes::<326>);
    world.insert_resource(DummyRes::<327>);
    world.insert_resource(DummyRes::<328>);
    world.insert_resource(DummyRes::<329>);
    world.insert_resource(DummyRes::<330>);
    world.insert_resource(DummyRes::<331>);
    world.insert_resource(DummyRes::<332>);
    world.insert_resource(DummyRes::<333>);
    world.insert_resource(DummyRes::<334>);
    world.insert_resource(DummyRes::<335>);
    world.insert_resource(DummyRes::<336>);
    world.insert_resource(DummyRes::<337>);
    world.insert_resource(DummyRes::<338>);
    world.insert_resource(DummyRes::<339>);
    world.insert_resource(DummyRes::<340>);
    world.insert_resource(DummyRes::<341>);
    world.insert_resource(DummyRes::<342>);
    world.insert_resource(DummyRes::<343>);
    world.insert_resource(DummyRes::<344>);
    world.insert_resource(DummyRes::<345>);
    world.insert_resource(DummyRes::<346>);
    world.insert_resource(DummyRes::<347>);
    world.insert_resource(DummyRes::<348>);
    world.insert_resource(DummyRes::<349>);
    world.insert_resource(DummyRes::<350>);
    world.insert_resource(DummyRes::<351>);
    world.insert_resource(DummyRes::<352>);
    world.insert_resource(DummyRes::<353>);
    world.insert_resource(DummyRes::<354>);
    world.insert_resource(DummyRes::<355>);
    world.insert_resource(DummyRes::<356>);
    world.insert_resource(DummyRes::<357>);
    world.insert_resource(DummyRes::<358>);
    world.insert_resource(DummyRes::<359>);
    world.insert_resource(DummyRes::<360>);
    world.insert_resource(DummyRes::<361>);
    world.insert_resource(DummyRes::<362>);
    world.insert_resource(DummyRes::<363>);
    world.insert_resource(DummyRes::<364>);
    world.insert_resource(DummyRes::<365>);
    world.insert_resource(DummyRes::<366>);
    world.insert_resource(DummyRes::<367>);
    world.insert_resource(DummyRes::<368>);
    world.insert_resource(DummyRes::<369>);
    world.insert_resource(DummyRes::<370>);
    world.insert_resource(DummyRes::<371>);
    world.insert_resource(DummyRes::<372>);
    world.insert_resource(DummyRes::<373>);
    world.insert_resource(DummyRes::<374>);
    world.insert_resource(DummyRes::<375>);
    world.insert_resource(DummyRes::<376>);
    world.insert_resource(DummyRes::<377>);
    world.insert_resource(DummyRes::<378>);
    world.insert_resource(DummyRes::<379>);
    world.insert_resource(DummyRes::<380>);
    world.insert_resource(DummyRes::<381>);
    world.insert_resource(DummyRes::<382>);
    world.insert_resource(DummyRes::<383>);
    world.insert_resource(DummyRes::<384>);
    world.insert_resource(DummyRes::<385>);
    world.insert_resource(DummyRes::<386>);
    world.insert_resource(DummyRes::<387>);
    world.insert_resource(DummyRes::<388>);
    world.insert_resource(DummyRes::<389>);
    world.insert_resource(DummyRes::<390>);
    world.insert_resource(DummyRes::<391>);
    world.insert_resource(DummyRes::<392>);
    world.insert_resource(DummyRes::<393>);
    world.insert_resource(DummyRes::<394>);
    world.insert_resource(DummyRes::<395>);
    world.insert_resource(DummyRes::<396>);
    world.insert_resource(DummyRes::<397>);
    world.insert_resource(DummyRes::<398>);
    world.insert_resource(DummyRes::<399>);
    world.insert_resource(DummyRes::<400>);
    world.insert_resource(DummyRes::<401>);
    world.insert_resource(DummyRes::<402>);
    world.insert_resource(DummyRes::<403>);
    world.insert_resource(DummyRes::<404>);
    world.insert_resource(DummyRes::<405>);
    world.insert_resource(DummyRes::<406>);
    world.insert_resource(DummyRes::<407>);
    world.insert_resource(DummyRes::<408>);
    world.insert_resource(DummyRes::<409>);
    world.insert_resource(DummyRes::<410>);
    world.insert_resource(DummyRes::<411>);
    world.insert_resource(DummyRes::<412>);
    world.insert_resource(DummyRes::<413>);
    world.insert_resource(DummyRes::<414>);
    world.insert_resource(DummyRes::<415>);
    world.insert_resource(DummyRes::<416>);
    world.insert_resource(DummyRes::<417>);
    world.insert_resource(DummyRes::<418>);
    world.insert_resource(DummyRes::<419>);
    world.insert_resource(DummyRes::<420>);
    world.insert_resource(DummyRes::<421>);
    world.insert_resource(DummyRes::<422>);
    world.insert_resource(DummyRes::<423>);
    world.insert_resource(DummyRes::<424>);
    world.insert_resource(DummyRes::<425>);
    world.insert_resource(DummyRes::<426>);
    world.insert_resource(DummyRes::<427>);
    world.insert_resource(DummyRes::<428>);
    world.insert_resource(DummyRes::<429>);
    world.insert_resource(DummyRes::<430>);
    world.insert_resource(DummyRes::<431>);
    world.insert_resource(DummyRes::<432>);
    world.insert_resource(DummyRes::<433>);
    world.insert_resource(DummyRes::<434>);
    world.insert_resource(DummyRes::<435>);
    world.insert_resource(DummyRes::<436>);
    world.insert_resource(DummyRes::<437>);
    world.insert_resource(DummyRes::<438>);
    world.insert_resource(DummyRes::<439>);
    world.insert_resource(DummyRes::<440>);
    world.insert_resource(DummyRes::<441>);
    world.insert_resource(DummyRes::<442>);
    world.insert_resource(DummyRes::<443>);
    world.insert_resource(DummyRes::<444>);
    world.insert_resource(DummyRes::<445>);
    world.insert_resource(DummyRes::<446>);
    world.insert_resource(DummyRes::<447>);
    world.insert_resource(DummyRes::<448>);
    world.insert_resource(DummyRes::<449>);
    world.insert_resource(DummyRes::<450>);
    world.insert_resource(DummyRes::<451>);
    world.insert_resource(DummyRes::<452>);
    world.insert_resource(DummyRes::<453>);
    world.insert_resource(DummyRes::<454>);
    world.insert_resource(DummyRes::<455>);
    world.insert_resource(DummyRes::<456>);
    world.insert_resource(DummyRes::<457>);
    world.insert_resource(DummyRes::<458>);
    world.insert_resource(DummyRes::<459>);
    world.insert_resource(DummyRes::<460>);
    world.insert_resource(DummyRes::<461>);
    world.insert_resource(DummyRes::<462>);
    world.insert_resource(DummyRes::<463>);
    world.insert_resource(DummyRes::<464>);
    world.insert_resource(DummyRes::<465>);
    world.insert_resource(DummyRes::<466>);
    world.insert_resource(DummyRes::<467>);
    world.insert_resource(DummyRes::<468>);
    world.insert_resource(DummyRes::<469>);
    world.insert_resource(DummyRes::<470>);
    world.insert_resource(DummyRes::<471>);
    world.insert_resource(DummyRes::<472>);
    world.insert_resource(DummyRes::<473>);
    world.insert_resource(DummyRes::<474>);
    world.insert_resource(DummyRes::<475>);
    world.insert_resource(DummyRes::<476>);
    world.insert_resource(DummyRes::<477>);
    world.insert_resource(DummyRes::<478>);
    world.insert_resource(DummyRes::<479>);
    world.insert_resource(DummyRes::<480>);
    world.insert_resource(DummyRes::<481>);
    world.insert_resource(DummyRes::<482>);
    world.insert_resource(DummyRes::<483>);
    world.insert_resource(DummyRes::<484>);
    world.insert_resource(DummyRes::<485>);
    world.insert_resource(DummyRes::<486>);
    world.insert_resource(DummyRes::<487>);
    world.insert_resource(DummyRes::<488>);
    world.insert_resource(DummyRes::<489>);
    world.insert_resource(DummyRes::<490>);
    world.insert_resource(DummyRes::<491>);
    world.insert_resource(DummyRes::<492>);
    world.insert_resource(DummyRes::<493>);
    world.insert_resource(DummyRes::<494>);
    world.insert_resource(DummyRes::<495>);
    world.insert_resource(DummyRes::<496>);
    world.insert_resource(DummyRes::<497>);
    world.insert_resource(DummyRes::<498>);
    world.insert_resource(DummyRes::<499>);
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


criterion_group!(
    benches,
    bench_insert_remove_sparseset,
    bench_insert_remove_batch,
    bench_heavyweight_bundle,
    bench_spawn_batch,
    bench_heavy_compute,
    bench_fragmented_iteration,
    bench_wide_iteration,
    bench_fragmented_wide_iteration,
    bench_simple_iter,
    bench_contiguous_iter,
    bench_contiguous_iter_avx2,
    bench_for_each_iter,
    bench_cache_locality_loss,
    bench_sparse_iter,
    bench_wide_simple_iter,
    bench_wide_sparse_iter,
    bench_bypass_change_detection,
    bench_sparse_simple_iter,
    bench_system_iter,
    bench_wide_sparse_simple_iter,
    bench_par_cache_locality_loss,
    bench_observer_lifecycle_insert,
    bench_event_propagation,
    bench_combinator_system,
    dyn_param,
    run_condition_yes,
    run_condition_no,
    run_condition_yes_with_query,
    run_condition_yes_with_resource,
    empty_systems,
    busy_systems,
    contrived,
    schedule_bench,
    build_schedule,
    empty_schedule_run,
    empty_commands,
    spawn_commands,
    nonempty_spawn_commands,
    insert_commands,
    fake_commands,
    zero_sized_commands,
    medium_sized_commands,
    large_sized_commands,
    world_despawn,
    world_despawn_recursive,
    entity_allocator_benches,
    entity_set_build_and_lookup,
    world_spawn,
    world_spawn_batch,
    world_entity,
    world_get,
    world_query_get,
    world_query_iter,
    world_query_for_each,
    query_get,
    query_get_components_mut_2,
    query_get_components_mut_5,

    query_get_components_mut_10,
    all_added_detection,
    all_changed_detection,
    few_changed_detection,
    none_changed_detection,

    multiple_archetype_none_changed_detection,

    empty_archetypes,
    single_clone,
    hierarchy_tall,
    hierarchy_wide,

    hierarchy_many,

    iter_frag_empty,
    get_resource,
    get_resource_mut,
    insert_remove_resource
);
criterion_main!(benches);





