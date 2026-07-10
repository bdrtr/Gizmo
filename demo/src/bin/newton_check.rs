//! Headless doğrulama: Newton sarkacı fizik mantığını GÖRSEL olmadan sınar.
//! Topları + hinge'leri kurar, adımlar, uç topların hız/konumunu basar — momentum
//! dalgasının gerçekten aktarıldığını (top0 durur, topN-1 fırlar) sayısal görürüz.

use gizmo::physics::components::{Collider, CombineMode, PhysicsMaterial, RigidBody, Velocity};
use gizmo::physics::joints::Joint;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{BodyHandle, Transform};
use gizmo::core::world::World;
use gizmo::math::{Quat, Vec3};

const N: usize = 5;
const R: f32 = 0.5;
const L: f32 = 4.0;
const PIVOT_Y: f32 = 6.0;
const MASS: f32 = 1.0;
const GAP: f32 = 0.01;
const RELEASE_DEG: f32 = 55.0;

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

fn main() {
    let mut world = World::new();
    let gap: f32 = std::env::var("GAP").ok().and_then(|s| s.parse().ok()).unwrap_or(GAP);
    println!("(gap = {gap})");
    let n: usize = std::env::var("NBALLS").ok().and_then(|s| s.parse().ok()).unwrap_or(N);
    let spacing = 2.0 * R + gap;
    let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

    // Sabit çapa gövdesi (kiriş).
    let beam = world.spawn();
    world.add_component(beam, Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0)));
    world.add_component(beam, RigidBody::new_static());
    world.add_component(beam, Velocity::default());
    world.add_component(beam, Collider::box_collider(Vec3::new(N as f32 * spacing, 0.05, 0.05)));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let iters: usize = std::env::var("ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    phys.solver.iterations = iters;
    println!("(solver.iterations = {iters})");
    let mut balls = Vec::new();

    for i in 0..n {
        let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
        let (center, rot) = if i == 0 {
            let a = RELEASE_DEG.to_radians();
            (pivot + L * Vec3::new(-a.sin(), -a.cos(), 0.0), Quat::from_rotation_z(-a))
        } else {
            (pivot - Vec3::new(0.0, L, 0.0), Quat::IDENTITY)
        };

        let mut rb = RigidBody::new(MASS, true);
        rb.calculate_sphere_inertia(R);

        let ball = world.spawn();
        world.add_component(ball, Transform::new(center).with_rotation(rot));
        world.add_component(ball, rb);
        world.add_component(ball, Velocity::default());
        world.add_component(ball, Collider::sphere(R).with_material(elastic()));
        balls.push(ball.id());

        phys.joints.push(Joint::hinge(
            BodyHandle::from_id(beam.id()),
            BodyHandle::from_id(ball.id()),
            pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
            Vec3::new(0.0, L, 0.0),
            Vec3::Z,
        ));
    }
    world.insert_resource(phys);

    let dt = 1.0 / 120.0;
    let read = |world: &World, id: u32| -> (Vec3, Vec3) {
        let ts = world.borrow::<Transform>();
        let vs = world.borrow::<Velocity>();
        (ts.get(id).unwrap().position, vs.get(id).map(|v| v.linear).unwrap_or(Vec3::ZERO))
    };

    println!("frame  t     vx[0..N] (her topun yatay hızı)                  ΣKE   Σpx");
    for frame in 0..=360 {
        if frame % 10 == 0 {
            let mut ke = 0.0;
            let mut px = 0.0;
            let mut s = String::new();
            for &b in &balls {
                let (_, v) = read(&world, b);
                ke += 0.5 * MASS * v.length_squared();
                px += MASS * v.x;
                s += &format!("{:+.2} ", v.x);
            }
            println!("{frame:>4}  {:.2}  [{s}]  {ke:.2}  {px:+.2}", frame as f32 * dt);
        }
        gizmo::systems::cpu_physics_step_system(&world, dt);
    }

    // Basit sağlık kontrolleri.
    let mut nan = false;
    let mut max_drop = 0.0_f32;
    for &b in &balls {
        let (p, _) = read(&world, b);
        if !p.is_finite() { nan = true; }
        max_drop = max_drop.max(PIVOT_Y - L - p.y); // dinlenme y'sinden ne kadar düştü
    }
    println!("\nNaN={nan}  max_sag(dinlenmeden düşüş)={max_drop:.2} (büyükse toplar kopmuş/düşmüş demek)");
}
