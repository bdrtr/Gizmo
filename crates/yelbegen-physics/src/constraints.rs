use yelbegen_math::Vec3;

/// Fiziksel Kısıtlayıcı Türleri (Joints & Constraints)
/// İki entity arasında fiziksel bağlantı oluşturur

/// Kısıtlayıcı Bileşeni — Entity'ye eklenir, fizik sistemi tarafından uygulanır
#[derive(Clone, Debug)]
pub struct Joint {
    pub kind: JointKind,
    pub entity_a: u32,
    pub entity_b: u32,
    /// Kısıtlayıcının göreceli bağlantı noktası (Entity A'nın yerel koordinatlarında)
    pub anchor_a: Vec3,
    /// Kısıtlayıcının göreceli bağlantı noktası (Entity B'nin yerel koordinatlarında)
    pub anchor_b: Vec3,
    /// Kısıtlayıcı sertliği (0 = yumuşak, 1 = katı)
    pub stiffness: f32,
    /// Sönümleme (damping) — titreşimi azaltır
    pub damping: f32,
}

/// Kısıtlayıcı Türleri
#[derive(Clone, Debug)]
pub enum JointKind {
    /// Top Mafsal (Ball Socket) — Her yöne serbest döner, konum kısıtlı
    BallSocket,
    /// Menteşe (Hinge) — Tek bir eksen etrafında döner
    Hinge {
        axis: Vec3, // Dönme ekseni (Entity A'nın yerel koordinatlarında)
    },
    /// Mesafe Kısıtı — İki nokta arasındaki mesafe sabit kalır (ip/çubuk)
    Distance {
        length: f32,
    },
    /// Yay (Spring) — İki nokta arasında elastik bağlantı
    Spring {
        rest_length: f32,
        spring_constant: f32, // k (Newton/metre)
    },
}

impl Joint {
    /// Yeni bir Ball Socket (top mafsal) oluşturur
    pub fn ball_socket(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        Self {
            kind: JointKind::BallSocket,
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Yeni bir menteşe (hinge) oluşturur
    pub fn hinge(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            kind: JointKind::Hinge { axis: axis.normalize() },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Mesafe kısıtı oluşturur (ip/çubuk)
    pub fn distance(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, length: f32) -> Self {
        Self {
            kind: JointKind::Distance { length },
            entity_a, entity_b,
            anchor_a, anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Yay kısıtı oluşturur
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

/// Kısıtlayıcı havuzu — Tüm aktif joint'lerin listesi (ECS dışı, global)
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

/// Kısıtlayıcıları fizik adımında çözen sistem
/// Position-based constraint solving (Baumgarte stabilization)
/// World referansı alır — storage yapısından bağımsız çalışır
pub fn solve_constraints(
    joint_world: &JointWorld,
    world: &yelbegen_core::World,
    dt: f32,
) {
    let beta = 0.2; // Baumgarte stabilizasyon faktörü

    for joint in &joint_world.joints {
        let pos_a = match world.borrow::<crate::components::Transform>() {
            Some(t) => match t.get(joint.entity_a) {
                Some(ta) => ta.position + joint.anchor_a,
                None => continue,
            },
            None => continue,
        };
        let pos_b = match world.borrow::<crate::components::Transform>() {
            Some(t) => match t.get(joint.entity_b) {
                Some(tb) => tb.position + joint.anchor_b,
                None => continue,
            },
            None => continue,
        };

        let (inv_mass_a, inv_mass_b) = {
            let rbs = match world.borrow::<crate::components::RigidBody>() {
                Some(r) => r,
                None => continue,
            };
            let ima = rbs.get(joint.entity_a).map_or(0.0, |rb| if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 });
            let imb = rbs.get(joint.entity_b).map_or(0.0, |rb| if rb.mass > 0.0 { 1.0 / rb.mass } else { 0.0 });
            (ima, imb)
        };
        let total_inv_mass = inv_mass_a + inv_mass_b;
        if total_inv_mass == 0.0 { continue; }

        match &joint.kind {
            JointKind::BallSocket => {
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
            }
            JointKind::Distance { length } => {
                let diff = pos_b - pos_a;
                let current_len = diff.length();
                if current_len < 0.0001 { continue; }
                
                let dir = diff / current_len;
                let error = current_len - length;
                let correction = dir * error * (beta / dt) * joint.stiffness;
                
                if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                    if let Some(v_a) = vels.get_mut(joint.entity_a) {
                        v_a.linear += correction * (inv_mass_a / total_inv_mass);
                    }
                    if let Some(v_b) = vels.get_mut(joint.entity_b) {
                        v_b.linear -= correction * (inv_mass_b / total_inv_mass);
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
            JointKind::Hinge { axis } => {
                let diff = pos_b - pos_a;
                let correction = diff * (beta / dt) * joint.stiffness;
                
                if let Some(mut vels) = world.borrow_mut::<crate::components::Velocity>() {
                    if let Some(v_a) = vels.get_mut(joint.entity_a) {
                        v_a.linear += correction * (inv_mass_a / total_inv_mass);
                        let angular_on_axis = *axis * v_a.angular.dot(*axis);
                        v_a.angular = angular_on_axis;
                    }
                    if let Some(v_b) = vels.get_mut(joint.entity_b) {
                        v_b.linear -= correction * (inv_mass_b / total_inv_mass);
                        let angular_on_axis = *axis * v_b.angular.dot(*axis);
                        v_b.angular = angular_on_axis;
                    }
                }
            }
        }
    }
}
