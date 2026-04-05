use gizmo_math::Vec3;

/// Fiziksel Kısıtlayıcı Türleri (Joints & Constraints)
/// İki entity arasında fiziksel bağlantı oluşturur

/// Kısıtlayıcı Bileşeni — Entity'ye eklenir, fizik sistemi tarafından uygulanır
#[derive(Clone, Debug)]
pub struct Joint {
    pub kind: JointKind,
    pub entity_a: u32,
    pub entity_b: u32,
    pub anchor_a: Vec3,
    pub anchor_b: Vec3,
    pub stiffness: f32,
    pub damping: f32,
}

/// Kısıtlayıcı Türleri
#[derive(Clone, Debug)]
pub enum JointKind {
    /// Top Mafsal (Ball Socket) — Her yöne serbest döner, konum kısıtlı
    BallSocket,
    /// Menteşe (Hinge) — Tek bir eksen etrafında döner, açı limitli
    Hinge {
        axis: Vec3,
        min_angle: f32, // Minimum açı (radyan). f32::NEG_INFINITY = limitsiz
        max_angle: f32, // Maximum açı (radyan). f32::INFINITY = limitsiz
    },
    /// Sabit Bağlantı (Fixed) — İki obje yapışık, ne hareket ne dönüş
    Fixed {
        /// Bağlantı anındaki B'nin A'ya göre göreli rotasyonu (başlangıç offset)
        relative_rotation: gizmo_math::Quat,
    },
    /// Mesafe Kısıtı — İki nokta arasındaki mesafe sabit kalır (ip/çubuk)
    Distance {
        length: f32,
    },
    /// Yay (Spring) — İki nokta arasında elastik bağlantı
    Spring {
        rest_length: f32,
        spring_constant: f32,
    },
}

impl Joint {
    pub fn ball_socket(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        Self {
            kind: JointKind::BallSocket,
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    pub fn hinge(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            kind: JointKind::Hinge { axis: axis.normalize(), min_angle: f32::NEG_INFINITY, max_angle: f32::INFINITY },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Açı limitli menteşe
    pub fn hinge_limited(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, axis: Vec3, min_angle: f32, max_angle: f32) -> Self {
        Self {
            kind: JointKind::Hinge { axis: axis.normalize(), min_angle, max_angle },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Sabit bağlantı (iki obje yapışık)
    pub fn fixed(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        Self {
            kind: JointKind::Fixed { relative_rotation: gizmo_math::Quat::IDENTITY },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.3,
        }
    }

    pub fn distance(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, length: f32) -> Self {
        Self {
            kind: JointKind::Distance { length },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    pub fn spring(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, rest_length: f32, k: f32) -> Self {
        Self {
            kind: JointKind::Spring { rest_length, spring_constant: k },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.5,
        }
    }
}

/// Kısıtlayıcı havuzu — Tüm aktif joint'lerin listesi
pub struct JointWorld {
    pub joints: Vec<Joint>,
}

impl JointWorld {
    pub fn new() -> Self {
        Self { joints: Vec::new() }
    }

    pub fn add(&mut self, joint: Joint) -> usize {
        let idx = self.joints.len();
        self.joints.push(joint);
        idx
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.joints.len() {
            self.joints.swap_remove(index);
        }
    }
}

impl Default for JointWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Kısıtlayıcıları fizik adımında çözen sistem
/// 4 iterasyon ile kararlılık sağlar (Gauss-Seidel)
pub fn solve_constraints(
    joint_world: &JointWorld,
    world: &gizmo_core::World,
    dt: f32,
) {
    let beta = 0.2; // Baumgarte stabilizasyon faktörü
    let iterations = 15; // 4 İterasyon uzun zincirler için yetersizdir. En az eleman sayısı kadar olmalı.

    for _iter in 0..iterations {
        for joint in &joint_world.joints {
            let (pos_a, pos_b, rot_a, rot_b) = match world.borrow::<crate::components::Transform>() {
                Some(t) => {
                    let ta = match t.get(joint.entity_a) { Some(ta) => *ta, None => continue };
                    let tb = match t.get(joint.entity_b) { Some(tb) => *tb, None => continue };
                    (
                        ta.position + ta.rotation.mul_vec3(joint.anchor_a),
                        tb.position + tb.rotation.mul_vec3(joint.anchor_b),
                        ta.rotation,
                        tb.rotation,
                    )
                },
                None => continue,
            };

            let (inv_mass_a, inv_inertia_a, inv_mass_b, inv_inertia_b) = {
                let rbs = match world.borrow::<crate::components::RigidBody>() {
                    Some(r) => r,
                    None => continue,
                };
                let (ima, iia) = rbs.get(joint.entity_a).map_or((0.0, gizmo_math::Vec3::ZERO), |rb| {
                    if rb.mass > 0.0 { (1.0 / rb.mass, rb.inverse_inertia) } else { (0.0, gizmo_math::Vec3::ZERO) }
                });
                let (imb, iib) = rbs.get(joint.entity_b).map_or((0.0, gizmo_math::Vec3::ZERO), |rb| {
                    if rb.mass > 0.0 { (1.0 / rb.mass, rb.inverse_inertia) } else { (0.0, gizmo_math::Vec3::ZERO) }
                });
                (ima, iia, imb, iib)
            };
            let total_inv_mass = inv_mass_a + inv_mass_b;
            if total_inv_mass == 0.0 { continue; }

            match &joint.kind {
                JointKind::BallSocket => {
                    let r_a = rot_a.mul_vec3(joint.anchor_a);
                    let r_b = rot_b.mul_vec3(joint.anchor_b);
                    let diff = pos_b - pos_a;
                    
                    if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                        let mut va_lin = if let Some(v) = vels.get(joint.entity_a) { v.linear } else { Vec3::ZERO };
                        let mut va_ang = if let Some(v) = vels.get(joint.entity_a) { v.angular } else { Vec3::ZERO };
                        let mut vb_lin = if let Some(v) = vels.get(joint.entity_b) { v.linear } else { Vec3::ZERO };
                        let mut vb_ang = if let Some(v) = vels.get(joint.entity_b) { v.angular } else { Vec3::ZERO };
                        
                        let bias = diff * (beta / dt) * joint.stiffness;

                        // 3D Jacobian Hız Tahminleme: 3 eksen boyunca teker teker çöz (Sequential Impulse)
                        let axes = [Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0)];
                        for &axis in &axes {
                            let vel_a_anchor = va_lin + va_ang.cross(r_a);
                            let vel_b_anchor = vb_lin + vb_ang.cross(r_b);
                            let rel_vel = vel_b_anchor - vel_a_anchor;
                            
                            let rel_vel_n = rel_vel.dot(axis);
                            let bias_n = bias.dot(axis);
                            
                            let r_a_cross_n = r_a.cross(axis);
                            let r_b_cross_n = r_b.cross(axis);
                            
                            // Vec3 element-wise çarpım (Mul) tanımlı olmadığı için manuel çarpım:
                            let r_a_cross_n_i = Vec3::new(r_a_cross_n.x * inv_inertia_a.x, r_a_cross_n.y * inv_inertia_a.y, r_a_cross_n.z * inv_inertia_a.z);
                            let r_b_cross_n_i = Vec3::new(r_b_cross_n.x * inv_inertia_b.x, r_b_cross_n.y * inv_inertia_b.y, r_b_cross_n.z * inv_inertia_b.z);
                            
                            // Tam Jacobian Efektif Kütle Projeksiyonu (Angular katkı ile)
                            let eff_mass_inv = inv_mass_a + inv_mass_b 
                                + r_a_cross_n_i.dot(r_a_cross_n)
                                + r_b_cross_n_i.dot(r_b_cross_n);
                                
                            if eff_mass_inv > 0.0001 {
                                let lambda = -(rel_vel_n + bias_n) / eff_mass_inv;
                                let impulse = axis * lambda;
                                
                                va_lin -= impulse * inv_mass_a;
                                
                                let va_torque = r_a.cross(impulse);
                                va_ang -= Vec3::new(va_torque.x * inv_inertia_a.x, va_torque.y * inv_inertia_a.y, va_torque.z * inv_inertia_a.z);
                                
                                vb_lin += impulse * inv_mass_b;
                                
                                let vb_torque = r_b.cross(impulse);
                                vb_ang += Vec3::new(vb_torque.x * inv_inertia_b.x, vb_torque.y * inv_inertia_b.y, vb_torque.z * inv_inertia_b.z);
                            }
                        }

                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.linear = va_lin;
                            v_a.angular = va_ang;
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.linear = vb_lin;
                            v_b.angular = vb_ang;
                        }
                    }
                }
                JointKind::Fixed { relative_rotation } => {
                    // 1. Pozisyon kısıtı (BallSocket gibi)
                    let diff = pos_b - pos_a;
                    let correction = diff * (beta / dt) * joint.stiffness;
                    
                    if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.linear += correction * (inv_mass_a / total_inv_mass);
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.linear -= correction * (inv_mass_b / total_inv_mass);
                        }
                    }
                    
                    // 2. Rotasyon kısıtı — B'nin rotasyonu A*relative_rotation olmalı
                    let target_rot = rot_a * *relative_rotation;
                    // Hata quaternion'u: target'tan mevcut'a
                    let error_rot = target_rot * rot_b.conjugate();
                    // Quaternion'dan açısal hız hatası çıkar
                    let (axis, angle) = error_rot.to_axis_angle();
                    if angle.abs() > 0.001 {
                        let angular_correction = axis * angle * (beta / dt) * joint.stiffness;
                        if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                            if let Some(v_a) = vels.get_mut(joint.entity_a) {
                                v_a.angular -= angular_correction * 0.5;
                            }
                            if let Some(v_b) = vels.get_mut(joint.entity_b) {
                                v_b.angular += angular_correction * 0.5;
                            }
                        }
                    }
                }
                JointKind::Distance { length } => {
                    let diff = pos_b - pos_a;
                    let current_len = diff.length();
                    if current_len < 0.0001 { continue; }
                    
                    let dir = diff / current_len;
                    let error = current_len - length;
                    
                    if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                        let va = if let Some(v) = vels.get(joint.entity_a) { v.linear } else { Vec3::ZERO };
                        let vb = if let Some(v) = vels.get(joint.entity_b) { v.linear } else { Vec3::ZERO };
                        
                        let rel_vel = vb - va;
                        let rel_vel_along_dir = rel_vel.dot(dir);
                        
                        // Doğru Gauss-Seidel Distance Solver:
                        let lambda = -(rel_vel_along_dir + error * (beta / dt) * joint.stiffness) / total_inv_mass;
                        let impulse = dir * lambda;
                        
                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.linear -= impulse * inv_mass_a;
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.linear += impulse * inv_mass_b;
                        }
                    }
                }
                JointKind::Spring { rest_length, spring_constant } => {
                    let diff = pos_b - pos_a;
                    let current_len = diff.length();
                    if current_len < 0.0001 { continue; }
                    
                    let dir = diff / current_len;
                    let displacement = current_len - rest_length;
                    let spring_force = dir * displacement * (*spring_constant);
                    
                    let rel_vel = {
                        let vels = world.borrow::<crate::components::Velocity>();
                        let va = vels.as_ref().and_then(|v| v.get(joint.entity_a)).map_or(Vec3::ZERO, |v| v.linear);
                        let vb = vels.as_ref().and_then(|v| v.get(joint.entity_b)).map_or(Vec3::ZERO, |v| v.linear);
                        vb - va
                    };
                    let damping_force = dir * rel_vel.dot(dir) * joint.damping;
                    let total_force = spring_force + damping_force;
                    
                    if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.linear += total_force * inv_mass_a * dt;
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.linear -= total_force * inv_mass_b * dt;
                        }
                    }
                }
                JointKind::Hinge { axis, min_angle, max_angle } => {
                    // 1. Pozisyon kısıtı (BallSocket gibi)
                    let diff = pos_b - pos_a;
                    let correction = diff * (beta / dt) * joint.stiffness;
                    
                    if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.linear += correction * (inv_mass_a / total_inv_mass);
                            // Açısal hızı sadece menteşe ekseni üzerine projeksiyonla
                            let angular_on_axis = *axis * v_a.angular.dot(*axis);
                            v_a.angular = angular_on_axis;
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.linear -= correction * (inv_mass_b / total_inv_mass);
                            let angular_on_axis = *axis * v_b.angular.dot(*axis);
                            v_b.angular = angular_on_axis;
                        }
                    }
                    
                    // 2. Açı limiti kontrolü
                    if *min_angle > f32::NEG_INFINITY || *max_angle < f32::INFINITY {
                        // B'nin A'ya göre açısını hesapla
                        let rel_rot = rot_a.conjugate() * rot_b;
                        let (hinge_axis, angle) = rel_rot.to_axis_angle();
                        // Eksen yönüne göre açıyı işaretle
                        let signed_angle = if hinge_axis.dot(*axis) >= 0.0 { angle } else { -angle };
                        
                        if signed_angle < *min_angle {
                            let error = *min_angle - signed_angle;
                            let angular_correction = *axis * error * (beta / dt);
                            if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                                if let Some(v_b) = vels.get_mut(joint.entity_b) {
                                    v_b.angular += angular_correction;
                                }
                            }
                        } else if signed_angle > *max_angle {
                            let error = signed_angle - *max_angle;
                            let angular_correction = *axis * error * (beta / dt);
                            if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                                if let Some(v_b) = vels.get_mut(joint.entity_b) {
                                    v_b.angular -= angular_correction;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

