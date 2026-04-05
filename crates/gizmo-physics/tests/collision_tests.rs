use gizmo_math::{Vec3, Quat};
use gizmo_physics::shape::{ColliderShape, Capsule, Aabb, Sphere, ConvexHull};
use gizmo_physics::collision::{
    check_aabb_aabb_manifold, 
    check_capsule_aabb_manifold,
    check_capsule_capsule_manifold,
    check_capsule_sphere_manifold
};

// ============================================
// 1. ONSHAPE & BROADPHASE SUPPORT TESTS
// ============================================

#[test]
fn test_capsule_support_points() {
    let capsule = ColliderShape::Capsule(Capsule {
        radius: 0.5,
        half_height: 1.0,
    });
    
    // Kapsül yukarı doğru yönelmiş olmalı
    let pos = Vec3::new(0.0, 0.0, 0.0);
    let rot = Quat::IDENTITY;
    
    // Y ekseni boyunca support point (tepe) -> origin(0,0,0) + half_height(1.0) + radius(0.5) = 1.5
    let up = Vec3::new(0.0, 1.0, 0.0);
    let p_up = capsule.support_point(pos, rot, up);
    assert_eq!(p_up.x, 0.0);
    assert!((p_up.y - 1.5).abs() < 0.001);
    assert_eq!(p_up.z, 0.0);
    
    // X ekseni boyunca support point (yan duvar) -> origin + radius = 0.5
    let right = Vec3::new(1.0, 0.0, 0.0);
    let p_right = capsule.support_point(pos, rot, right);
    assert!((p_right.x - 0.5).abs() < 0.001);
}

// ============================================
// 2. NARROW-PHASE MANIFOLD TESTS
// ============================================

#[test]
fn test_aabb_vs_aabb_collision() {
    let aabb1 = Aabb { half_extents: Vec3::new(1.0, 1.0, 1.0) };
    let aabb2 = Aabb { half_extents: Vec3::new(1.0, 1.0, 1.0) };
    
    let pos1 = Vec3::new(0.0, 0.0, 0.0);
    let pos2 = Vec3::new(1.5, 0.0, 0.0); // X'de 0.5 penetrasyon: (1.0 + 1.0) - 1.5 = 0.5
    
    let manifold = check_aabb_aabb_manifold(pos1, &aabb1, pos2, &aabb2);
    
    assert!(manifold.is_colliding, "Kutular birbiri içine geçmiş olmalı!");
    assert!((manifold.penetration - 0.5).abs() < 0.001, "Penetrasyon 0.5 olmalı");
    assert!((manifold.normal.x - -1.0).abs() < 0.001 || (manifold.normal.x - 1.0).abs() < 0.001, "Normal X ekseninde olmalı");
}

#[test]
fn test_aabb_vs_aabb_no_collision() {
    let aabb1 = Aabb { half_extents: Vec3::new(1.0, 1.0, 1.0) };
    let aabb2 = Aabb { half_extents: Vec3::new(1.0, 1.0, 1.0) };
    
    let pos1 = Vec3::new(0.0, 0.0, 0.0);
    let pos2 = Vec3::new(2.1, 0.0, 0.0); // Kesinlikle ayrıklar
    
    let manifold = check_aabb_aabb_manifold(pos1, &aabb1, pos2, &aabb2);
    assert!(!manifold.is_colliding, "Kutular birbirine değmemeli!");
}

#[test]
fn test_capsule_vs_floor_aabb() {
    let capsule = Capsule { radius: 0.5, half_height: 1.0 };
    let floor = Aabb { half_extents: Vec3::new(10.0, 0.5, 10.0) };
    
    // Kapsül ayak ucu tam 0.0. Yer Orijini (0.0), zemin ise Y: -0.4'te
    // Zemin Y limits: -0.9 to 0.1
    // Kapsül Y limits: -1.5 (if pos=0, wait, base is pos.y - half_height - radius = 0 - 1 - 0.5 = -1.5)
    // Kapsülü (0, 1.4, 0) seviyesine koyalım. Y limit: 1.4 - 1.5 = -0.1
    let pos_cap = Vec3::new(0.0, 1.4, 0.0);
    let pos_floor = Vec3::new(0.0, -0.4, 0.0); // Y limit -0.4 + 0.5 = 0.1
    // Penetrasyon: Zemin üst = 0.1, Kapsül alt = -0.1. Penetration = 0.2
    
    let rot = Quat::IDENTITY;
    let manifold = check_capsule_aabb_manifold(pos_cap, rot, &capsule, pos_floor, &floor);
    
    assert!(manifold.is_colliding);
    assert!((manifold.penetration - 0.2).abs() < 0.001);
    assert!((manifold.normal.y - -1.0).abs() < 0.001, "Normal A'dan B'ye doğru, yani karakterden zemine doğru Y=-1 bakmalı");
}

#[test]
fn test_capsule_vs_capsule() {
    let c1 = Capsule { radius: 0.5, half_height: 1.0 };
    let c2 = Capsule { radius: 0.5, half_height: 1.0 };
    
    // Toplam yarıçap 1.0. Yan yana 0.8 mesafede koyalım (0.2 kesişim)
    let p1 = Vec3::new(0.0, 0.0, 0.0);
    let p2 = Vec3::new(0.8, 0.0, 0.0);
    
    let rot = Quat::IDENTITY;
    let manifold = check_capsule_capsule_manifold(p1, rot, &c1, p2, rot, &c2);
    
    assert!(manifold.is_colliding);
    assert!((manifold.penetration - 0.2).abs() < 0.001);
}

#[test]
fn test_capsule_vs_sphere() {
    let cap = Capsule { radius: 0.5, half_height: 1.0 };
    let sph = Sphere { radius: 0.5 };
    
    // Kapsül ucu Y = 1.5. Küre Y = 1.8 koyalım. 
    // Mesafe = 1.8 - 1.0(segment tip) = 0.8. Toplam yarıçap = 1.0. 
    // Penetration = 1.0 - 0.8 = 0.2
    let p_cap = Vec3::new(0.0, 0.0, 0.0);
    let p_sph = Vec3::new(0.0, 1.8, 0.0);
    
    let manifold = check_capsule_sphere_manifold(p_cap, Quat::IDENTITY, &cap, p_sph, &sph);
    
    assert!(manifold.is_colliding);
    assert!((manifold.penetration - 0.2).abs() < 0.001);
}
