use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gizmo_physics::solver::ConstraintSolver;
use gizmo_physics::collision::{ContactManifold, ContactPoint};
use gizmo_physics::components::{RigidBody, Transform, Velocity, ColliderShape, BoxShape};
use gizmo_physics::broadphase::SpatialHash;
use gizmo_physics::gjk::Gjk;
use gizmo_math::{Vec3, Quat, Aabb, Mat4, Frustum};
use gizmo_core::entity::Entity;
use gizmo_renderer::frustum_cull::visible_in_frustum;
use gizmo_renderer::csm::{cascade_split_distances, directional_cascade_view_projs, CASCADE_COUNT};

fn setup_solver_data(count: usize) -> (Vec<ContactManifold>, Vec<(RigidBody, Transform, Velocity)>, Vec<(RigidBody, Transform, Velocity)>) {
    let mut manifolds = Vec::with_capacity(count);
    let mut bodies_a = Vec::with_capacity(count);
    let mut bodies_b = Vec::with_capacity(count);

    for i in 0..count {
        let entity_a = Entity::new(i as u32, 0);
        let entity_b = Entity::new((i + count) as u32, 0);
        
        let mut manifold = ContactManifold::new(entity_a, entity_b);
        manifold.add_contact(ContactPoint {
            point: Vec3::new(0.0, 0.0, 0.0),
            normal: Vec3::new(0.0, 1.0, 0.0),
            penetration: 0.1,
            local_point_a: Vec3::new(0.0, -0.5, 0.0),
            local_point_b: Vec3::new(0.0, 0.5, 0.0),
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        });
        manifolds.push(manifold);

        let mut rb_a = RigidBody::default();
        rb_a.mass = 1.0;
        let t_a = Transform::new(Vec3::new(0.0, 1.0, 0.0));
        let v_a = Velocity::new(Vec3::new(0.0, -1.0, 0.0));
        bodies_a.push((rb_a, t_a, v_a));

        let mut rb_b = RigidBody::default();
        rb_b.mass = 1.0;
        let t_b = Transform::new(Vec3::new(0.0, -1.0, 0.0));
        let v_b = Velocity::new(Vec3::new(0.0, 1.0, 0.0));
        bodies_b.push((rb_b, t_b, v_b));
    }

    (manifolds, bodies_a, bodies_b)
}

fn solver_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Physics Solver");
    let count = 1000;
    
    group.bench_function(format!("solve_contacts_{}", count), |b| {
        b.iter_batched(
            || {
                let (manifolds, bodies_a, bodies_b) = setup_solver_data(count);
                let solver = ConstraintSolver::new(10);
                (solver, manifolds, bodies_a, bodies_b)
            },
            |(solver, mut manifolds, mut bodies_a, mut bodies_b)| {
                solver.solve_contacts(
                    black_box(&mut manifolds),
                    black_box(&mut bodies_a),
                    black_box(&mut bodies_b),
                    black_box(0.016),
                );
            },
            criterion::BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn broadphase_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Physics Broadphase");
    let count = 10_000;

    group.bench_function(format!("spatial_hash_insert_and_query_{}", count), |b| {
        b.iter_batched(
            || {
                let mut hash = SpatialHash::new(5.0);
                let mut entities = Vec::with_capacity(count);
                for i in 0..count {
                    let e = Entity::new(i as u32, 0);
                    let x = (i % 100) as f32;
                    let y = ((i / 100) % 100) as f32;
                    let z = (i / 10000) as f32;
                    let aabb = Aabb::from_center_half_extents(Vec3::new(x, y, z), Vec3::splat(1.0));
                    entities.push((e, aabb));
                }
                (hash, entities)
            },
            |(hash, entities)| {
                use rayon::prelude::*;
                entities.into_par_iter().for_each(|(e, aabb)| {
                    hash.insert(e, aabb);
                });
                black_box(hash.query_pairs());
            },
            criterion::BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn gjk_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Physics Narrowphase");
    
    let shape_a = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(1.0) });
    let shape_b = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(1.0) });
    
    let pos_a = Vec3::new(0.0, 0.0, 0.0);
    let rot_a = Quat::IDENTITY;
    let pos_b = Vec3::new(1.5, 0.5, 0.0);
    let rot_b = Quat::IDENTITY;

    group.bench_function("gjk_get_contact", |b| {
        b.iter(|| {
            black_box(Gjk::get_contact(
                black_box(&shape_a), black_box(pos_a), black_box(rot_a),
                black_box(&shape_b), black_box(pos_b), black_box(rot_b)
            ));
        })
    });
    group.finish();
}

fn renderer_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Renderer CPU");

    // 1. Frustum Culling Benchmark
    let count = 100_000;
    let view_proj = Mat4::look_at_rh(Vec3::new(0.0, 0.0, -10.0), Vec3::ZERO, Vec3::Y) 
        * Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0/9.0, 0.1, 100.0);
    let frustum = Frustum::from_matrix(&view_proj);
    
    let mut aabbs = Vec::with_capacity(count);
    let mut models = Vec::with_capacity(count);
    for i in 0..count {
        aabbs.push(Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(1.0)));
        models.push(Mat4::from_translation(Vec3::new((i % 100) as f32, 0.0, (i / 100) as f32)));
    }

    group.bench_function(format!("frustum_cull_{}", count), |b| {
        b.iter(|| {
            let mut visible = 0;
            for i in 0..count {
                if visible_in_frustum(&frustum, &models[i], aabbs[i]) {
                    visible += 1;
                }
            }
            black_box(visible);
        })
    });

    // 2. CSM Splitting and Projection Generation Benchmark
    group.bench_function("csm_matrix_generation", |b| {
        b.iter(|| {
            let splits = cascade_split_distances(0.1, 1000.0, 0.5);
            let mats = directional_cascade_view_projs(
                Vec3::new(0.0, 10.0, 0.0), 
                Vec3::new(0.0, 0.0, -1.0), 
                16.0 / 9.0, 
                std::f32::consts::FRAC_PI_4, 
                0.1, 
                &splits, 
                Vec3::new(1.0, -1.0, 1.0), 
                2048
            );
            black_box(mats);
        })
    });

    group.finish();
}

fn fracture_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Physics Fracture");
    
    let extents = Vec3::splat(1.0);
    let chunk_counts = [50, 200];
    
    for &num_pieces in &chunk_counts {
        group.bench_function(format!("voronoi_shatter_{}", num_pieces), |b| {
            b.iter(|| {
                black_box(gizmo_physics::fracture::voronoi_shatter(
                    black_box(extents), 
                    black_box(num_pieces), 
                    black_box(12345)
                ));
            })
        });
        
        group.bench_function(format!("generate_fracture_chunks_{}", num_pieces), |b| {
            let transform = Transform::new(Vec3::ZERO);
            let rb = RigidBody::new(10.0, 0.5, 0.5, true);
            let vel = Velocity::new(Vec3::ZERO);
            let impact_point = Vec3::new(0.5, 0.5, 0.5);
            
            b.iter(|| {
                black_box(gizmo_physics::fracture::generate_fracture_chunks(
                    black_box(&transform),
                    black_box(&rb),
                    black_box(&vel),
                    black_box(extents),
                    black_box(num_pieces),
                    black_box(impact_point),
                    black_box(5000.0)
                ));
            })
        });
    }
    
    group.finish();
}

fn world_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Physics World Full Step");
    let count = 500;
    
    group.bench_function(format!("world_step_{}_bodies", count), |b| {
        b.iter_batched(
            || {
                let mut world = gizmo_physics::world::PhysicsWorld::new();
                world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);
                
                let mut bodies = Vec::with_capacity(count);
                for i in 0..count {
                    let ent = Entity::new(i as u32, 0);
                    let rb = RigidBody::new(1.0, 0.5, 0.5, true);
                    // Stack them
                    let transform = Transform::new(Vec3::new(0.0, (i as f32) * 2.0, 0.0));
                    let vel = Velocity::default();
                    let collider = gizmo_physics::components::Collider::box_collider(Vec3::splat(0.5));
                    bodies.push((ent, rb, transform, vel, collider));
                }
                
                // Add a ground plane
                let ground_ent = Entity::new(9999, 0);
                let ground_rb = RigidBody::new_static();
                let ground_transform = Transform::new(Vec3::ZERO);
                let ground_vel = Velocity::default();
                let ground_collider = gizmo_physics::components::Collider::plane(Vec3::new(0.0, 1.0, 0.0), 0.0);
                bodies.push((ground_ent, ground_rb, ground_transform, ground_vel, ground_collider));
                
                (world, bodies)
            },
            |(mut world, mut bodies)| {
                // Measure one full sub-step equivalent (1/120 dt)
                // World metrics can be observed if needed: black_box(&world.metrics);
                world.step(&mut bodies, &mut [], 1.0 / 120.0);
                black_box(bodies);
                black_box(world);
            },
            criterion::BatchSize::SmallInput,
        )
    });
    
    group.finish();
}

criterion_group!(benches, solver_benchmark, broadphase_benchmark, gjk_benchmark, renderer_benchmark, fracture_benchmark, world_benchmark);
criterion_main!(benches);
