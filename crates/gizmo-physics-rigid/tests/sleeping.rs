//! Islands & sleeping sağlamlaştırma regresyon testleri (Faz 4).
//!
//! Denetimde bulunan gerçek bug'ları kilitler:
//!  * apply_impulse/apply_force uyuyan cismi UYANDIRMALI (yoksa impuls yutulur),
//!  * joint-coupled uyuyan cisim, eklemin diğer ucu hareketliyse UYANMALI,
//!  * island inşa sırası DETERMINISTIK (en küçük manifold indisine göre sıralı),
//!  * uyuyan yığının üstüne düşen cisim yığını uyandırıp ÜSTÜNDE durmalı (tünelleme yok).

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, ContactManifold, ContactPoint, Transform};
use gizmo_physics_rigid::{IslandManager, Joint, PhysicsWorld, RigidBody, Velocity};

fn add_ground(world: &mut PhysicsWorld) {
    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        Entity::new(0, 0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
}

fn add_box(world: &mut PhysicsWorld, id: u32, pos: Vec3) {
    let mut rb = RigidBody::new(1.0, 0.0, 0.6, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.5));
    rb.update_inertia_from_collider(&col);
    world.add_body(Entity::new(id, 0), rb, Transform::new(pos), Velocity::default(), col);
}

#[test]
fn apply_impulse_wakes_sleeping_body() {
    // Uyuyan bir cisme impuls uygulanınca cisim UYANMALI ve hızı değişmeli.
    // (Eski hata: apply_impulse `&RigidBody` alıyordu → uyandıramıyor → is_sleeping
    //  true kalıyor → position_integration cismi atlıyor → impuls SESSİZCE yutuluyor.)
    let world = PhysicsWorld::new();
    let mut rb = RigidBody::new(1.0, 0.0, 0.5, true);
    rb.is_sleeping = true;
    rb.sleep_counter = 100;
    let t = Transform::new(Vec3::ZERO);
    let mut v = Velocity::default();

    world.apply_impulse(&mut rb, &t, &mut v, Vec3::new(0.0, 5.0, 0.0), Vec3::ZERO);

    assert!(!rb.is_sleeping, "impuls uyuyan cismi uyandırmadı");
    assert!(v.linear.y > 0.0, "impuls uygulanmadı: v={:?}", v.linear);
}

#[test]
fn apply_force_wakes_sleeping_body() {
    let world = PhysicsWorld::new();
    let mut rb = RigidBody::new(1.0, 0.0, 0.5, true);
    rb.is_sleeping = true;
    rb.sleep_counter = 100;
    let mut v = Velocity::default();

    world.apply_force(&mut rb, &mut v, Vec3::new(10.0, 0.0, 0.0), 1.0 / 60.0);

    assert!(!rb.is_sleeping, "kuvvet uyuyan cismi uyandırmadı");
    assert!(v.linear.x > 0.0, "kuvvet uygulanmadı: v={:?}", v.linear);
}

#[test]
fn joint_couples_wake_to_sleeping_body() {
    // Fixed joint'le bağlı iki dinamik cisim; ikisi de uyusun. Sonra BİRİ uyandırılıp
    // disturb edilince, eklemin diğer (uyuyan) ucu da UYANMALI (joint-coupled wake).
    // (Eski hata: joint_solver `&[RigidBody]` alıyor → uyuyan ucu sessizce hareket
    //  ettirip is_sleeping'i bırakıyordu → düzeltme yutuluyor, mekanizma kopuk.)
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    add_box(&mut world, 1, Vec3::new(0.0, 0.0, 0.0)); // index 0
    add_box(&mut world, 2, Vec3::new(1.5, 0.0, 0.0)); // index 1 (uzak → temas yok)
    world.joints.push(Joint::fixed(
        Entity::new(1, 0),
        Entity::new(2, 0),
        Vec3::new(1.5, 0.0, 0.0),
        Vec3::ZERO,
    ));

    // Yerçekimi yok, dinlenmede → ikisi de uyusun.
    for _ in 0..120 {
        world.step(1.0 / 60.0).ok();
    }
    assert!(
        world.rigid_bodies[0].is_sleeping && world.rigid_bodies[1].is_sleeping,
        "ön koşul: iki cisim de uyumalıydı (A={} B={})",
        world.rigid_bodies[0].is_sleeping,
        world.rigid_bodies[1].is_sleeping
    );

    // A'yı uyandır + disturb et (dış etki simülasyonu).
    world.rigid_bodies[0].wake_up();
    world.velocities[0].linear = Vec3::new(0.0, 1.0, 0.0);

    world.step(1.0 / 60.0).ok();

    assert!(
        !world.rigid_bodies[1].is_sleeping,
        "joint-coupled uyandırma çalışmadı: B hâlâ uyuyor"
    );
}

#[test]
fn island_build_order_is_deterministic_sorted() {
    // build_islands çıktısı, island'ların en küçük manifold indisine göre SIRALI olmalı
    // (HashMap into_values süreç-bağlı sıradan bağımsız determinizm).
    let mk = |ea: u32, eb: u32| -> ContactManifold {
        let mut m = ContactManifold::new(Entity::new(ea, 0), Entity::new(eb, 0));
        m.contacts.push(ContactPoint {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            penetration: 0.01,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        });
        m
    };
    // 3 ayrı island: {0:(1-2)} {1:(3-4)} {2:(5-6)} — karışık entity id'leriyle.
    let manifolds = vec![mk(1, 2), mk(3, 4), mk(5, 6)];
    let is_dyn = |e: Entity| e.id() != 0;
    let islands = IslandManager::build_islands(&manifolds, &is_dyn);
    assert_eq!(islands.len(), 3);
    // Her island tek manifold; sıra 0,1,2 olmalı (min-index artan).
    let firsts: Vec<usize> = islands
        .iter()
        .map(|i| *i.manifold_indices.first().unwrap())
        .collect();
    let mut sorted = firsts.clone();
    sorted.sort_unstable();
    assert_eq!(firsts, sorted, "island sırası min-indise göre sıralı değil: {firsts:?}");
}

#[test]
fn dropped_box_wakes_and_rests_on_sleeping_stack() {
    // Sağlamlaştırma: yerleşip UYUYAN bir yığının üstüne yeni kutu düşünce, yığın
    // uyanıp yeni kutu ÜSTÜNDE durmalı (içinden geçmemeli). Per-body uyku + island
    // uyandırma (island_awake + wake_updates) bunu garanti etmeli.
    let mut world = PhysicsWorld::new();
    add_ground(&mut world);
    add_box(&mut world, 1, Vec3::new(0.0, 0.5, 0.0)); // alt
    add_box(&mut world, 2, Vec3::new(0.0, 1.5, 0.0)); // üst

    // Yerleşip uyusunlar.
    for _ in 0..180 {
        world.step(1.0 / 60.0).ok();
    }
    assert!(
        world.rigid_bodies[1].is_sleeping && world.rigid_bodies[2].is_sleeping,
        "ön koşul: yığın uyumalıydı"
    );

    // Üstüne 3. kutu düşür (y=3'ten).
    add_box(&mut world, 3, Vec3::new(0.0, 3.0, 0.0)); // index 3
    for _ in 0..180 {
        world.step(1.0 / 60.0).ok();
    }

    // 3. kutu yığının üstünde (~y=2.5) durmalı; içinden geçip alta düşmemeli.
    let y3 = world.transforms[3].position.y;
    assert!(y3.is_finite(), "3. kutu NaN");
    assert!(
        y3 > 2.0,
        "düşen kutu uyuyan yığını uyandıramadı / içinden geçti: y={y3}"
    );
    // Sıra korundu (tünelleme yok).
    let (y1, y2) = (world.transforms[1].position.y, world.transforms[2].position.y);
    assert!(
        y2 > y1 + 0.3 && y3 > y2 + 0.3,
        "yığın sırası bozuldu: y1={y1} y2={y2} y3={y3}"
    );
}
