use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{Transform, RigidBody, Velocity};
use gizmo_physics::shape::Collider;
use gizmo_physics::system::{physics_movement_system, physics_collision_system};

fn setup_world() -> World {
    World::new()
}

#[test]
fn test_tunneling_without_ccd() {
    let mut world = setup_world();
    
    // Devasa hızla giden (mermi gibi) bir obje
    // CCD KAPALI (use_ccd = false)
    let bullet = world.spawn();
    world.add_component(bullet, Transform::new(Vec3::new(-5.0, 0.0, 0.0)));
    let mut rb = RigidBody::new(1.0, 0.5, 0.5, false);
    rb.ccd_enabled = false;
    world.add_component(bullet, rb);
    // Hız saniyede 200 metre (1 karede (0.016s) = 3.2 metre ilerler)
    world.add_component(bullet, Velocity::new(Vec3::new(200.0, 0.0, 0.0)));
    world.add_component(bullet, Collider::new_sphere(0.1));
    
    // Tam önünde (-0.5'ten 0.5'e kadar) ince duvar (Kalınlık 1.0)
    let wall = world.spawn();
    world.add_component(wall, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(wall, RigidBody::new_static());
    world.add_component(wall, Collider::new_aabb(0.5, 5.0, 5.0)); // Yarı X=0.5 -> Toplam X kalınlığı = 1.0
    
    // 0.1 saniye işlet => Mermi 20 metre gidecek
    // -5.0 + 20.0 = 15.0'e ışınlanmalı ve duvara HİÇ ÇARPMADAN geçmeli
    // (Çünkü CCD kapalı ve discrete test 1 kareden diğerine duvarı atlar)
    physics_movement_system(&world, 0.1);
    physics_collision_system(&world); // Çarpışmayı kontrol et ama nafile, çünkü duvardan çoktan geçti
    
    let t = world.borrow::<Transform>().unwrap().get(bullet.id()).unwrap().clone();
    
    // Eğer tunneling engellenemeseydi, X > 5.0 olurdu!
    assert!(t.position.x > 5.0, "Tünelleme gerçekleşmeliydi ancak mermi takıldı! Pos: {}", t.position.x);
}

#[test]
fn test_tunneling_prevention_with_ccd() {
    let mut world = setup_world();
    
    // Devasa hızla giden (mermi gibi) bir obje
    // CCD AÇIK (use_ccd = true)
    let bullet = world.spawn();
    world.add_component(bullet, Transform::new(Vec3::new(-5.0, 0.0, 0.0)));
    let mut rb = RigidBody::new(1.0, 0.5, 0.5, false);
    rb.ccd_enabled = true;
    world.add_component(bullet, rb);
    
    world.add_component(bullet, Velocity::new(Vec3::new(200.0, 0.0, 0.0)));
    world.add_component(bullet, Collider::new_sphere(0.1));
    
    // Tam önünde ince duvar
    let wall = world.spawn();
    world.add_component(wall, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(wall, RigidBody::new_static());
    world.add_component(wall, Collider::new_aabb(0.5, 5.0, 5.0)); 
    
    // 0.1 saniye işlet
    physics_movement_system(&world, 0.1);
    
    let t = world.borrow::<Transform>().unwrap().get(bullet.id()).unwrap().clone();
    let v = world.borrow::<Velocity>().unwrap().get(bullet.id()).unwrap().clone();
    
    // CCD Açıkken merminin duvarın ÖNÜNDE (yaklaşık x = -0.6 civarı) kalması gerekir
    // X = 0.0 duvar merkezi. -0.5 duvar sonu. -0.1 yarıçap. O halde tahmini pos: X=-0.6
    assert!(t.position.x < 0.0, "CCD başarısız oldu, obje duvardan geçti veya içine girdi! Pos: {}", t.position.x);
    assert!((t.position.x - -0.6).abs() < 0.1, "CCD objeyi tam duvarın dibinde durduramadı! Pos: {}", t.position.x);
    
    // Ayrıca CCD duvarın içine girmesini engellediği için hızı da normal yönünde (X) sıfırlamış olmalıdır
    assert!(v.linear.x < 1.0, "Çarpışma hızı kesmedi! Kalan X hız: {}", v.linear.x);
}
