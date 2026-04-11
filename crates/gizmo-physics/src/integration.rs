use crate::components::{RigidBody, Transform, Velocity};
use gizmo_core::World;
use gizmo_math::{Quat, Vec3};

pub fn apply_inv_inertia(torque: Vec3, inv_inertia: Vec3, rot: Quat) -> Vec3 {
    let local_t = rot.inverse().mul_vec3(torque);
    let local_ang = Vec3::new(
        local_t.x * inv_inertia.x,
        local_t.y * inv_inertia.y,
        local_t.z * inv_inertia.z,
    );
    rot.mul_vec3(local_ang)
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
                        let speed_sq = v.linear.length_squared() + v.angular.length_squared();
                        if speed_sq < 0.0001 {
                            rb.sleep_timer += dt;
                            if rb.sleep_timer > 2.0 {
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

            // 2. DOĞRU DAMPING / DRAG HESABI (Üstel sönümleme yaklaşımı ile kararlı)
            let linear_drag = f32x8::splat(1.0 / (1.0 + dt * 0.5));
            let angular_drag = f32x8::splat(1.0 / (1.0 + dt * 3.0));
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
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                physics_apply_forces_system_avx2(world, dt);
                return;
            }
        }
    }
    // Fallback: SSE, NEON veya safcalar LLVM auto-vectorization
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
            let v = *vel_storage.get(e).unwrap();
            let t = match trans_storage.get_mut(e) {
                Some(t) => t,
                None => continue,
            };
            t.position += v.linear * dt;

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
            }

            t.update_local_matrix();
        }
    }
}

// O(N^2) Çarpışma Tespit ve Fizik (Impulse/Sekme/Tork) Çözümleyici Sistem
