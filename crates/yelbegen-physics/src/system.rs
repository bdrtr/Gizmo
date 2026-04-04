use yelbegen_core::World;
use crate::components::{Transform, Velocity, RigidBody};
use crate::shape::{Collider, ColliderShape};
use crate::collision::check_aabb_aabb_manifold;
use yelbegen_math::Vec3;

// Varlıkların fiziksel hareketlerini, yerçekimi ve sürtünme etkileriyle uygulayan sistem
pub fn physics_movement_system(world: &World) {
    let mut transforms = world.borrow_mut::<Transform>().expect("Transform yok");
    let mut velocities = world.borrow_mut::<Velocity>().expect("Velocity yok");
    let rigidbodies = world.borrow::<RigidBody>();

    let dt = 0.016; // FPS sabitleyicimize (16ms) tam uyumlu Delta Time

    for entity in transforms.entity_dense.clone() {
        if let (Some(trans), Some(vel)) = (transforms.get_mut(entity), velocities.get_mut(entity)) {
            // Kuvvetleri Uygula (Eğer Katı Cisim ise)
            if let Some(rb_list) = &rigidbodies {
                if let Some(rb) = rb_list.get(entity) {
                    if rb.use_gravity && rb.mass > 0.0 {
                        vel.linear.y -= 9.81 * dt; // Yerçekimi ivmesi
                    }
                    
                    if rb.friction > 0.0 && rb.mass > 0.0 {
                        vel.linear.x *= 1.0 - (rb.friction * dt);
                        vel.linear.z *= 1.0 - (rb.friction * dt);
                    }
                }
            }
            
            // Hızı pozisyona uygula
            trans.position += vel.linear * dt;
        }
    }
}

// O(N^2) Çarpışma Tespit ve Fizik (Impulse/Sekme) Çözümleyici Sistem
pub fn physics_collision_system(world: &World) {
    let mut transforms = world.borrow_mut::<Transform>().expect("Transform yok");
    let mut velocities = world.borrow_mut::<Velocity>().expect("Velocity yok");
    let colliders = world.borrow::<Collider>().expect("Collider yok");
    let rigidbodies = world.borrow::<RigidBody>().expect("RigidBody yok");

    let entities = transforms.entity_dense.clone();

    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let ent_a = entities[i];
            let ent_b = entities[j];

            let (rb_a, rb_b) = match (rigidbodies.get(ent_a), rigidbodies.get(ent_b)) {
                (Some(a), Some(b)) => (a, b),
                _ => continue, // Rigidbody'si olmayan çarpışıp güç aktaramaz
            };

            // İkisinin de kütlesi yoksa çarpışma çözümüne gerek yok (İki duvar çarpışmaz)
            if rb_a.mass == 0.0 && rb_b.mass == 0.0 { continue; }

            if let (Some(col_a), Some(col_b)) = (colliders.get(ent_a), colliders.get(ent_b)) {
                let pos_a = transforms.get(ent_a).unwrap().position;
                let pos_b = transforms.get(ent_b).unwrap().position;

                let manifold = match (&col_a.shape, &col_b.shape) {
                    (ColliderShape::Aabb(a), ColliderShape::Aabb(b)) => {
                        check_aabb_aabb_manifold(pos_a, a, pos_b, b)
                    }
                    _ => continue,
                };

                // Eğer kutular birbirine geçiyorsa:
                if manifold.is_colliding {
                    // -- 1. POZİSYON DÜZELTMESİ (Positional Correction) --
                    // Objelerin birbirinin içinden sızmasını (Sinking) engellemek için hafifçe ayırıyoruz
                    let inv_mass_a = if rb_a.mass == 0.0 { 0.0 } else { 1.0 / rb_a.mass };
                    let inv_mass_b = if rb_b.mass == 0.0 { 0.0 } else { 1.0 / rb_b.mass };
                    let sum_inv_mass = inv_mass_a + inv_mass_b;

                    if let Some(t_a) = transforms.get_mut(ent_a) {
                        t_a.position -= manifold.normal * (manifold.penetration * (inv_mass_a / sum_inv_mass));
                    }
                    if let Some(t_b) = transforms.get_mut(ent_b) {
                        t_b.position += manifold.normal * (manifold.penetration * (inv_mass_b / sum_inv_mass));
                    }

                    // -- 2. MOMENTUM & İTME (Impulse & Restitution) --
                    let vel_a = velocities.get(ent_a).map(|v| v.linear).unwrap_or(Vec3::ZERO);
                    let vel_b = velocities.get(ent_b).map(|v| v.linear).unwrap_or(Vec3::ZERO);

                    let relative_vel = vel_b - vel_a;
                    let vel_along_normal = relative_vel.dot(manifold.normal);

                    // Objeler zaten ayrılıyorsa tekrar itme
                    if vel_along_normal > 0.0 { continue; }

                    // Objelerin zıplama oranının en esnek olmayanını alıyoruz
                    let e = rb_a.restitution.min(rb_b.restitution);

                    // Güç katsayısı J = -(1 + e) * V / Toplam_Ters_Kütle
                    let mut j = -(1.0 + e) * vel_along_normal;
                    j /= sum_inv_mass;

                    let impulse = manifold.normal * j;

                    // Hızlara (Velocity) yansıtma
                    if let Some(v_a) = velocities.get_mut(ent_a) {
                        v_a.linear -= impulse * inv_mass_a;
                    }
                    if let Some(v_b) = velocities.get_mut(ent_b) {
                        v_b.linear += impulse * inv_mass_b;
                    }
                }
            }
        }
    }
}
