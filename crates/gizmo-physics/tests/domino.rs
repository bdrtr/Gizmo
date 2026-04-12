use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::*;
use gizmo_physics::shape::*;
use gizmo_physics::system::*;

/// Setup a fresh physics world state
fn create_physics_world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsSolverState::new());
    world
}

/// Simulate steps helper
fn simulate(world: &mut World, steps: u32, dt: f32) {
    for _ in 0..steps {
        gizmo_physics::physics_apply_forces_system(world, dt);
        gizmo_physics::physics_movement_system(world, dt);
        physics_collision_system(world, dt);
    }
}

// ========================================================
// DOMİNO ETKİSİ - ZİNCİRLEME REAKSİYON
// Fiziğin zamanlamasını, ardışık çarpışma kuyruklarını ve
// enerji aktarımındaki kayıpları (damping, restitution) ölçer.
// ========================================================
#[test]
fn domino_chain_reaction() {
    let mut world = create_physics_world();
    let dt = 1.0 / 60.0;

    // 1. ZEMİN (Statik)
    let floor = world.spawn();
    world.add_component(floor, Transform::new(Vec3::new(0.0, -0.5, 50.0)));
    world.add_component(floor, RigidBody::new_static());
    // Koca bir zemin parçası (Yarı boyutları)
    world.add_component(
        floor,
        Collider {
            shape: ColliderShape::Aabb(Aabb {
                half_extents: Vec3::new(10.0, 0.5, 120.0),
            }),
        },
    );
    world.add_component(floor, Velocity::new(Vec3::ZERO));

    // 2. DOMİNO TAŞLARI (Dinamik)
    let num_dominoes = 100;
    let spacing = 0.8; // Her bir domino arası mesafe

    // Domino ebatları (Yarı boyut: en, boy, kalınlık) => G:0.2, Y:1.0, K:0.1
    let (dx, dy, dz) = (0.2, 1.0, 0.1);
    let mut last_domino_id = None;

    for i in 0..num_dominoes {
        let domino = world.spawn();

        // Art arda Z ekseni boyunca diz. Y ekseni tam zeminin üstüne oturacak şekilde. (zemin y=0, domino_half_y=1.0)
        let pos = Vec3::new(0.0, dy, i as f32 * spacing);
        world.add_component(domino, Transform::new(pos));

        let mut rb = RigidBody::new(0.5, 0.1, 0.3, true); // Kütle, Sekme(Restitution), Sürtünme(Friction)
        rb.calculate_box_inertia(dx * 2.0, dy * 2.0, dz * 2.0); // OBB olarak eylemsizliği hesapla ki dönebilsin
        world.add_component(domino, rb);

        world.add_component(
            domino,
            Collider {
                shape: ColliderShape::Aabb(Aabb {
                    half_extents: Vec3::new(dx, dy, dz),
                }),
            },
        );
        world.add_component(domino, Velocity::new(Vec3::ZERO));

        if i == num_dominoes - 1 {
            last_domino_id = Some(domino.id());
        }
    }

    // 3. AĞIR KÜRE (Tetikleyici)
    // En baştaki dominoya Z yönünden yuvarlanarak (hızla) çarpacak
    let heavy_ball = world.spawn();
    // Z ekseninde ilk dominonun (-1.5) arkasında
    world.add_component(heavy_ball, Transform::new(Vec3::new(0.0, 0.5, -1.5)));

    let mut ball_rb = RigidBody::new(50.0, 0.2, 0.5, true); // 50 birim ağır gülle
                                                            // Sphere inertia calculation: I = 2/5 * m * r^2
    let r = 0.5;
    let inertia = (2.0 / 5.0) * ball_rb.mass * (r * r);
    ball_rb.local_inertia = Vec3::new(inertia, inertia, inertia);
    ball_rb.inverse_inertia_local =
        gizmo_math::Mat3::from_diagonal(Vec3::splat(1.0 / inertia));

    world.add_component(heavy_ball, ball_rb);
    world.add_component(
        heavy_ball,
        Collider {
            shape: ColliderShape::Sphere(Sphere { radius: 0.5 }),
        },
    );

    // Güçlü bir itme kuvveti verelim (gerçekçi sürtünmeyi aşması için artırıldı)
    world.add_component(heavy_ball, Velocity::new(Vec3::new(0.0, 0.0, 25.0)));

    // 4. SİMÜLASYON ADIMLARI
    // 100 domino = ~80 mt uzunluk. Gülle çarptıktan sonra zincir reaksiyonun sona ulaşması zaman alacaktır.
    // 60fps * 30 saniye = 1800 kare
    simulate(&mut world, 1800, dt);

    // 5. SONUÇLARI DOĞRULA (ASSERTION)
    // Son dominonun pozisyonuna bakalım.
    let last_id = last_domino_id.expect("Son domino oluşturulamadı");
    let transforms = world.borrow::<Transform>().unwrap();

    let last_transform = transforms.get(last_id).expect("Son dominoyu bulamadık");

    assert!(
        last_transform.position.y < dy - 0.2,
        "Domino zinciri yarıda kesilmiş! Enerji son dominoya kadar gidemedi. Çarpışma ve ağırlık aktarım sönümlemesi hatalı. Son domino yüksekliği: {}",
        last_transform.position.y
    );

    println!(
        "Zincirleme Reaksiyon Başarılı! Son domino düştü. (Son Yüksekliği: {:.2})",
        last_transform.position.y
    );
}
