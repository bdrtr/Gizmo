use crate::components::{RigidBody, Transform, Velocity};
use gizmo_core::World;
use gizmo_math::{Mat3, Mat4, Quat, Vec3};

/// Uyku eşikleri — her biri kendi biriminde ayrı değerlendirilir.
///
/// Lineer: (m/s)² — obje bu hızın altına düştüğünde uyku sayıcısı artar.
/// Açısal: (rad/s)² — ayrı eşik; dönerken lineer sıfır olsa da uyanmalı.
/// İkkisini toplamak boyutsel olarak anlamsız (m/s ≠ rad/s) — ayrı kontrol zorunlu.
const SLEEP_LINEAR_SQ: f32 = 0.05 * 0.05;  // 5 cm/s — Bullet Physics standardı
const SLEEP_ANGULAR_SQ: f32 = 0.15 * 0.15;  // 0.15 rad/s (~8.6°/s)
const SLEEP_TIMER_THRESHOLD: f32 = 1.0;     // 1 saniye — 2s çok uzun, jitter uzar

/// Dünya uzayında tork `τ` için açısal ivme benzeri vektör `I⁻¹ τ` (burada `I⁻¹` dünya uzayında
/// `R I_body⁻¹ Rᵀ`). `inverse_inertia_local` gövde çerçevesinde simetrik ters eylemsizlik matrisidir.
pub fn apply_inv_inertia(torque: Vec3, inverse_inertia_local: Mat3, rot: Quat) -> Vec3 {
    let local_t = rot.inverse().mul_vec3(torque);
    let local_ang = inverse_inertia_local * local_t;
    rot.mul_vec3(local_ang)
}

/// Tek iş parçacığı, skalar f32 — SIMD/AVX2 yok; `PhysicsConfig::deterministic_simulation` ile seçilir.
fn physics_apply_forces_scalar(world: &World, dt: f32) {
    if let (Some(mut vel_storage), Some(mut rbs)) = (
        world.borrow_mut::<Velocity>(),
        world.borrow_mut::<RigidBody>(),
    ) {
        let entities: Vec<u32> = vel_storage.dense.iter().map(|e| e.entity).collect();
        let mut active_ents = Vec::with_capacity(entities.len());
        for &entity in &entities {
            if let Some(rb) = rbs.get_mut(entity) {
                if let Some(v) = vel_storage.get_mut(entity) {
                    if rb.mass > 0.0 {
                        let lin_sq = v.linear.length_squared();
                        let ang_sq = v.angular.length_squared();
                        rb.avg_linear_sq = rb.avg_linear_sq * 0.9 + lin_sq * 0.1;
                        rb.avg_angular_sq = rb.avg_angular_sq * 0.9 + ang_sq * 0.1;
                        let is_still =
                            rb.avg_linear_sq < SLEEP_LINEAR_SQ && rb.avg_angular_sq < SLEEP_ANGULAR_SQ;
                        if is_still {
                            rb.sleep_timer += dt;
                            if rb.sleep_timer > SLEEP_TIMER_THRESHOLD {
                                rb.is_sleeping = true;
                                v.linear = Vec3::ZERO;
                                v.angular = Vec3::ZERO;
                            }
                        } else {
                            rb.wake_up();
                        }
                    }
                    if !rb.is_sleeping && rb.mass > 0.0 {
                        active_ents.push(entity);
                    }
                }
            }
        }

        let linear_drag = (-0.1 * dt).exp();   // k=0.1: ~10%/s kayıp (hafif hava direnci)
        let angular_drag = (-0.5 * dt).exp();   // k=0.5: ~40%/s kayıp (dönme sönümleme)

        for &e in &active_ents {
            if let Some(v) = vel_storage.get_mut(e) {
                let g = rbs
                    .get(e)
                    .map(|rb| if rb.use_gravity { 9.81 } else { 0.0 })
                    .unwrap_or(0.0);
                let mut lx = v.linear.x;
                let mut ly = v.linear.y;
                let mut lz = v.linear.z;
                let mut ax = v.angular.x;
                let mut ay = v.angular.y;
                let mut az = v.angular.z;

                ly -= g * dt;
                lx *= linear_drag;
                ly *= linear_drag;
                lz *= linear_drag;
                ax *= angular_drag;
                ay *= angular_drag;
                az *= angular_drag;

                v.linear = Vec3::new(
                    lx.clamp(-200.0, 200.0),
                    ly.clamp(-200.0, 200.0),
                    lz.clamp(-200.0, 200.0),
                );
                v.angular = Vec3::new(
                    ax.clamp(-100.0, 100.0),
                    ay.clamp(-100.0, 100.0),
                    az.clamp(-100.0, 100.0),
                );
            }
        }
    }
}

#[inline(always)]
fn physics_apply_forces_system_impl(world: &World, dt: f32) {
    if let (Some(mut vel_storage), Some(mut rbs)) = (
        world.borrow_mut::<Velocity>(),
        world.borrow_mut::<RigidBody>(),
    ) {
        use wide::f32x8;

        let entities: Vec<u32> = vel_storage.dense.iter().map(|e| e.entity).collect();
        let mut active_ents = Vec::with_capacity(entities.len());
        for &entity in &entities {
            if let Some(rb) = rbs.get_mut(entity) {
                if let Some(v) = vel_storage.get_mut(entity) {
                    if rb.mass > 0.0 {
                        // Lineer ve açısal hızı AYRI AYRI değerlendir:
                        let lin_sq = v.linear.length_squared();
                        let ang_sq = v.angular.length_squared();
                        
                        // Rolling average: Çok ufak dt ve sönümleme katsayısı kullanılarak titreşim (jitter) filtrelemesi
                        rb.avg_linear_sq = rb.avg_linear_sq * 0.9 + lin_sq * 0.1;
                        rb.avg_angular_sq = rb.avg_angular_sq * 0.9 + ang_sq * 0.1;

                        let is_still = rb.avg_linear_sq < SLEEP_LINEAR_SQ && rb.avg_angular_sq < SLEEP_ANGULAR_SQ;

                        if is_still {
                            rb.sleep_timer += dt;
                            if rb.sleep_timer > SLEEP_TIMER_THRESHOLD {
                                rb.is_sleeping = true;
                                v.linear = Vec3::ZERO;
                                v.angular = Vec3::ZERO;
                            }
                        } else {
                            rb.wake_up();
                        }
                    }
                    if !rb.is_sleeping && rb.mass > 0.0 {
                        active_ents.push(entity);
                    }
                }
            }
        }

        let mut index = 0;
        while index < active_ents.len() {
            let mut chunk_ents = [0u32; 8];
            let mut vx = [0.0; 8];
            let mut vy = [0.0; 8];
            let mut vz = [0.0; 8];
            let mut ax = [0.0; 8];
            let mut ay = [0.0; 8];
            let mut az = [0.0; 8];
            let mut grav = [0.0; 8];

            let end = (index + 8).min(active_ents.len());
            let valid_count = end - index;
            for i in 0..valid_count {
                let e = active_ents[index + i];
                chunk_ents[i] = e;
                if let Some(v) = vel_storage.get(e) {
                    vx[i] = v.linear.x;
                    vy[i] = v.linear.y;
                    vz[i] = v.linear.z;
                    ax[i] = v.angular.x;
                    ay[i] = v.angular.y;
                    az[i] = v.angular.z;
                }
                if let Some(rb) = rbs.get(e) {
                    grav[i] = if rb.use_gravity { 9.81 } else { 0.0 };
                }
            }

            let mut x_v = f32x8::new(vx);
            let mut y_v = f32x8::new(vy);
            let mut z_v = f32x8::new(vz);
            let mut x_a = f32x8::new(ax);
            let mut y_a = f32x8::new(ay);
            let mut z_a = f32x8::new(az);
            let g_v = f32x8::new(grav);
            let wf_dt = f32x8::splat(dt);

            // 1. YERÇEKİMİ UYGULASI (Daha Kararlı)
            y_v -= g_v * wf_dt;

            // 2. DOĞRU DAMPING / DRAG HESABI (Gerçek frame-rate bağımsız üstel sönümleme)
            // e^(-k·dt) formülü: fps'ten bağımsız, her zaman aynı saniyesel sönümleme oranını verir.
            //   k=0.1 → lineer drag: saniyede e^(-0.1) ≈ %90 hız kalır (hafif hava direnci)
            //   k=0.5 → angular drag: saniyede e^(-0.5) ≈ %60 hız kalır (dönme sönümleme)
            let linear_drag = f32x8::splat((-0.1 * dt).exp());
            let angular_drag = f32x8::splat((-0.5 * dt).exp());
            x_v *= linear_drag;
            y_v *= linear_drag;
            z_v *= linear_drag;
            x_a *= angular_drag;
            y_a *= angular_drag;
            z_a *= angular_drag;

            // 3. VELOCITY CLAMP
            let max_lin = f32x8::splat(200.0);
            let min_lin = f32x8::splat(-200.0);
            let max_ang = f32x8::splat(100.0);
            let min_ang = f32x8::splat(-100.0);
            x_v = x_v.max(min_lin).min(max_lin);
            y_v = y_v.max(min_lin).min(max_lin);
            z_v = z_v.max(min_lin).min(max_lin);
            x_a = x_a.max(min_ang).min(max_ang);
            y_a = y_a.max(min_ang).min(max_ang);
            z_a = z_a.max(min_ang).min(max_ang);

            let xv_arr = x_v.to_array();
            let yv_arr = y_v.to_array();
            let zv_arr = z_v.to_array();
            let xa_arr = x_a.to_array();
            let ya_arr = y_a.to_array();
            let za_arr = z_a.to_array();

            for i in 0..valid_count {
                let e = chunk_ents[i];
                if let Some(v) = vel_storage.get_mut(e) {
                    v.linear = Vec3::new(xv_arr[i], yv_arr[i], zv_arr[i]);
                    v.angular = Vec3::new(xa_arr[i], ya_arr[i], za_arr[i]);
                }
            }
            index += 8;
        }
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn physics_apply_forces_system_avx2(world: &World, dt: f32) {
    physics_apply_forces_system_impl(world, dt);
}

pub fn physics_apply_forces_system(world: &World, dt: f32) {
    let deterministic = world
        .get_resource::<crate::components::PhysicsConfig>()
        .map(|c| c.deterministic_simulation)
        .unwrap_or(false);

    if deterministic {
        physics_apply_forces_scalar(world, dt);
        return;
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                physics_apply_forces_system_avx2(world, dt);
                return;
            }
        }
    }
    // Fallback: SSE, NEON veya skalar `wide` yolu
    physics_apply_forces_system_impl(world, dt);
}

pub fn physics_movement_system(world: &World, dt: f32) {
    if let (Some(mut trans_storage), Some(vel_storage), Some(rbs)) = (
        world.borrow_mut::<Transform>(),
        world.borrow::<Velocity>(),
        world.borrow::<RigidBody>(),
    ) {
        let entities: Vec<u32> = trans_storage.dense.iter().map(|e| e.entity).collect();
        let mut active_ents = Vec::with_capacity(entities.len());
        for &entity in &entities {
            if let Some(rb) = rbs.get(entity) {
                if !rb.is_sleeping && rb.mass > 0.0 {
                    active_ents.push(entity);
                }
            }
        }

        // BATCH 3: Pozisyon Entegrasyonu & CCD (Continuous Collision Detection) - Skalar Loop
        for &e in &active_ents {
            let _rb = rbs.get(e).unwrap();
            let v = match vel_storage.get(e) {
                Some(v) => *v,
                None => continue, // Velocity yoksa hareketi es geç
            };
            let t = match trans_storage.get_mut(e) {
                Some(t) => t,
                None => continue,
            };
            let mut is_dirty = false;
            
            // Eğer objenin lineer hızı kayda değer değilse pozisyonu rölantide tut (mikro-jitter engeller)
            if v.linear.length_squared() > 0.000001 {
                t.position += v.linear * dt;
                is_dirty = true;
            }

            if v.angular.length_squared() > 0.0001 {
                let w_quat = Quat::from_xyzw(v.angular.x, v.angular.y, v.angular.z, 0.0);
                let q = t.rotation;
                let dq = w_quat * q;
                t.rotation = Quat::from_xyzw(
                    q.x + 0.5 * dt * dq.x,
                    q.y + 0.5 * dt * dq.y,
                    q.z + 0.5 * dt * dq.z,
                    q.w + 0.5 * dt * dq.w,
                )
                .normalize();
                is_dirty = true;
            }

            // Performans Optimizasyonu: Sadece matrisini değiştirmesi "gereken" objelerde
            // Mat4 hesaplaması yaparız, gereksiz sin/cos/mul matris inşasını önleriz.
            if is_dirty {
                t.update_local_matrix();
            }
        }
    }
}

// O(N^2) Çarpışma Tespit ve Fizik (Impulse/Sekme/Tork) Çözümleyici Sistem

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Mat3, Mat4};

    #[test]
    fn apply_inv_inertia_diagonal_mat3_matches_principal_formula() {
        let rot = Quat::from_rotation_y(0.4) * Quat::from_rotation_z(-0.2);
        let inv_diag = Vec3::new(2.0, 0.5, 1.25);
        let inv = Mat3::from_diagonal(inv_diag);
        let tau = Vec3::new(0.3, -1.1, 0.8);
        let local_t = rot.inverse().mul_vec3(tau);
        let expected = rot.mul_vec3(Vec3::new(
            local_t.x * inv_diag.x,
            local_t.y * inv_diag.y,
            local_t.z * inv_diag.z,
        ));
        let got = apply_inv_inertia(tau, inv, rot);
        assert!((got - expected).length() < 1e-5, "got {got:?} expected {expected:?}");
    }

    #[test]
    fn test_physics_dirty_flag_matrix_update() {
        let mut world = World::new();

        // 1. Static/Stationary Entity (Awake, but zero velocity)
        let e_static = world.spawn();
        let mut t_static = Transform::new(Vec3::ZERO);
        let broken_matrix = Mat4::from_scale_rotation_translation(
            Vec3::new(9.0, 9.0, 9.0),
            Quat::IDENTITY,
            Vec3::new(9.0, 9.0, 9.0),
        );
        t_static.global_matrix = broken_matrix; // Bilerek bozuyoruz
        world.add_component(e_static, t_static);
        world.add_component(e_static, Velocity::new(Vec3::ZERO));
        
        let mut rb1 = RigidBody::new(1.0, 0.5, 0.5, true);
        rb1.is_sleeping = false;
        world.add_component(e_static, rb1);

        // 2. Moving Entity
        let e_moving = world.spawn();
        let mut t_moving = Transform::new(Vec3::ZERO);
        t_moving.global_matrix = broken_matrix; // Bilerek bozuyoruz
        world.add_component(e_moving, t_moving);
        world.add_component(e_moving, Velocity::new(Vec3::new(10.0, 0.0, 0.0)));
        
        let mut rb2 = RigidBody::new(1.0, 0.5, 0.5, true);
        rb2.is_sleeping = false;
        world.add_component(e_moving, rb2);

        // Run movement
        physics_movement_system(&world, 0.1);

        let transforms = world.borrow::<Transform>().unwrap();

        // Stationary -> Hız < 1e-6, update_local_matrix çağrılmamalı! Bozuk matris korunmalı.
        let t1 = transforms.get(e_static.id()).unwrap();
        assert_eq!(t1.global_matrix, broken_matrix, "Stationary Matrix recalculated without movement!");

        // Moving -> Hız kayda değer, dirty flag true oldu, Matris yeniden hesaplanmalı ve düzelmeli!
        let t2 = transforms.get(e_moving.id()).unwrap();
        assert_ne!(t2.global_matrix, broken_matrix, "Moving Matrix did NOT recalculate!");
        assert_eq!(
            t2.global_matrix,
            Mat4::from_scale_rotation_translation(Vec3::new(1.0, 1.0, 1.0), Quat::IDENTITY, Vec3::new(1.0, 0.0, 0.0))
        );
    }
}

pub fn update_transform_hierarchy(world: &World) {
    use gizmo_core::component::{Parent, Children};
    if let Some(mut transforms) = world.borrow_mut::<Transform>() {
        let parents = world.borrow::<Parent>();
        let children = world.borrow::<Children>();
        
        let mut stack = Vec::new();
        // find root nodes (entities WITH Transform but NO Parent)
        for e in transforms.dense.iter() {
            let id = e.entity;
            let has_parent = parents.as_ref().map(|p| p.contains(id)).unwrap_or(false);
            if !has_parent {
                stack.push((id, Mat4::IDENTITY));
            }
        }
        
        while let Some((id, parent_mat)) = stack.pop() {
            let mut current_mat;
            if let Some(t) = transforms.get_mut(id) {
                current_mat = parent_mat * t.local_matrix();
                if current_mat.is_nan() {
                    current_mat = Mat4::IDENTITY; // Safe fallback
                }
                t.global_matrix = current_mat;
            } else {
                continue;
            }
            
            if let Some(ch_store) = &children {
                if let Some(ch) = ch_store.get(id) {
                    for &c in &ch.0 {
                        stack.push((c, current_mat));
                    }
                }
            }
        }
    }
}
