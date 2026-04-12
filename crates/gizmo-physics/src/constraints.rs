use gizmo_math::{Mat3, Vec3};
use crate::integration::apply_inv_inertia;

// ─── Yardımcı: JointBodies ───────────────────────────────────────────────────
//
// Her joint kolunda tekrarlanan 6 satırlık hazırlık kodu:
//
//   let pos_a       = ta.position + ta.rotation.mul_vec3(anchor_a);
//   let pos_b       = tb.position + tb.rotation.mul_vec3(anchor_b);
//   let inv_mass_a  = ...;
//   let inv_mass_b  = ...;
//   let total       = inv_mass_a + inv_mass_b;
//   if total == 0.0 { continue; }
//
// `JointBodies::resolve` bu bloğu bir kez çözer ve `None` döndürerek
// `continue` mantığını çağırana bırakır.

#[derive(Clone, Debug)]
pub struct JointBodies {
    pub pos_a:        Vec3,
    pub pos_b:        Vec3,
    pub rot_a:        gizmo_math::Quat,
    pub rot_b:        gizmo_math::Quat,
    pub inv_mass_a:   f32,
    pub inv_mass_b:   f32,
    pub inv_inertia_a: Mat3,
    pub inv_inertia_b: Mat3,
    pub total_inv_mass: f32,
}

impl JointBodies {
    /// Transforms ve RigidBody'lerden joint verilerini çıkarır.
    /// İki statik nesne veya eksik bileşen durumunda `None` döndürür.
    pub fn resolve(
        joint:      &Joint,
        transforms: &gizmo_core::SparseSet<crate::components::Transform>,
        rbs:        &gizmo_core::SparseSet<crate::components::RigidBody>,
    ) -> Option<Self> {
        let ta = *transforms.get(joint.entity_a)?;
        let tb = *transforms.get(joint.entity_b)?;

        let inv_mass_of = |rb: &crate::components::RigidBody| -> (f32, Mat3) {
            if rb.mass > 0.0 {
                (1.0 / rb.mass, rb.inverse_inertia_local)
            } else {
                (0.0, Mat3::ZERO)
            }
        };

        let (inv_mass_a, inv_inertia_a) =
            rbs.get(joint.entity_a).map_or((0.0, Mat3::ZERO), inv_mass_of);
        let (inv_mass_b, inv_inertia_b) =
            rbs.get(joint.entity_b).map_or((0.0, Mat3::ZERO), inv_mass_of);
        let total_inv_mass = inv_mass_a + inv_mass_b;

        if total_inv_mass == 0.0 {
            return None; // İki statik nesne — solver etkisiz
        }

        Some(Self {
            pos_a:         ta.position + ta.rotation.mul_vec3(joint.anchor_a),
            pos_b:         tb.position + tb.rotation.mul_vec3(joint.anchor_b),
            rot_a:         ta.rotation,
            rot_b:         tb.rotation,
            inv_mass_a,
            inv_mass_b,
            inv_inertia_a,
            inv_inertia_b,
            total_inv_mass,
        })
    }
}

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
        /// Menteşe ekseni — entity_a'nın **yerel (local)** uzayında saklanır.
        /// Fizik solverında her frame `rot_a.mul_vec3(axis)` ile world-space'e dönüştürülmelidir.
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
    Distance { length: f32 },
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
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    pub fn hinge(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3, axis: Vec3) -> Self {
        Self {
            kind: JointKind::Hinge {
                axis: axis.normalize(),
                min_angle: f32::NEG_INFINITY,
                max_angle: f32::INFINITY,
            },
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Açı limitli menteşe
    pub fn hinge_limited(
        entity_a: u32,
        entity_b: u32,
        anchor_a: Vec3,
        anchor_b: Vec3,
        axis: Vec3,
        min_angle: f32,
        max_angle: f32,
    ) -> Self {
        Self {
            kind: JointKind::Hinge {
                axis: axis.normalize(),
                min_angle,
                max_angle,
            },
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    /// Sabit bağlantı (iki obje yapışık)
    pub fn fixed(
        entity_a: u32, 
        entity_b: u32, 
        anchor_a: Vec3, 
        anchor_b: Vec3,
        rot_a: gizmo_math::Quat,
        rot_b: gizmo_math::Quat
    ) -> Self {
        // FIX #15: Anlık rotasyon farkını kaydet
        let relative_rotation = rot_a.inverse() * rot_b;
        Self {
            kind: JointKind::Fixed {
                relative_rotation,
            },
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.3,
        }
    }

    pub fn distance(
        entity_a: u32,
        entity_b: u32,
        anchor_a: Vec3,
        anchor_b: Vec3,
        length: f32,
    ) -> Self {
        Self {
            kind: JointKind::Distance { length },
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.1,
        }
    }

    pub fn spring(
        entity_a: u32,
        entity_b: u32,
        anchor_a: Vec3,
        anchor_b: Vec3,
        rest_length: f32,
        k: f32,
    ) -> Self {
        Self {
            kind: JointKind::Spring {
                rest_length,
                spring_constant: k,
            },
            entity_a,
            entity_b,
            anchor_a,
            anchor_b,
            stiffness: 1.0,
            damping: 0.5,
        }
    }
}

/// Kısıtlayıcı havuzu — Tüm aktif joint'lerın opaque-ID tabanlı listesi.
///
/// # ID Garantileri
/// - [`add`] her çağrıda benzersiz, monoton artan bir `usize` ID döndürür.
/// - ID'ler asla yeniden kullanılmaz; silinen ID sonsuza kadar geçersizdir.
/// - Çağıran kod **daima ID'yi** saklamalıdır, `Vec` indeksini değil.
///
/// # Güvenlik Tasarımı
/// `joints` alanı `pub(crate)` yapılmıştır; dışarıdan ham indeksle erişim
/// derleme aşamasında engellenir. Tüm dış erişim [`get`] / [`get_mut`] /
/// [`iter`] / [`contains`] arayüzleri üzerinden yapılır.
pub struct JointWorld {
    /// (id, joint) çiftleri. Dışarıdan indeks erişimini engellemek için
    /// `pub(crate)` — solver bu alanı doğrudan okur.
    pub(crate) joints: Vec<(usize, Joint)>,
    next_id: usize,
}

impl JointWorld {
    pub fn new() -> Self {
        Self {
            joints: Vec::new(),
            next_id: 1,
        }
    }

    /// Yeni bir joint ekler; çağırana özgü opaque ID döndürür.
    ///
    /// Dönen ID'yi saklayın — silme ve sorgulama işlemlerinde bu ID kullanılır.
    /// Ham `Vec` indeksi **geçersiz** bir tanımlayıcıdır; kullanmayın.
    pub fn add(&mut self, joint: Joint) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.joints.push((id, joint));
        id
    }

    /// Verilen ID'ye sahip joint'i **kararlı sıralamayı koruyarak** siler.
    ///
    /// Dönüş değeri:
    /// - `true`  → ID bulundu ve silindi.
    /// - `false` → ID zaten geçersizdi (çift silme / stale handle); sessiz hata yok.
    ///
    /// # Neden `swap_remove` değil?
    /// `swap_remove` son elemanı silinen yere taşır; bu, taşınan joint'in
    /// Vec indeksini değiştirir. Çağıran kod eski indeksi saklıyorsa farklı
    /// bir joint'i yanlışlıkla hedefler — tespit edilmesi güç sessiz bug.
    /// `retain` ile sıralama sabit kalır ve tüm ID'ler geçerliliğini korur.
    pub fn remove(&mut self, id: usize) -> bool {
        let before = self.joints.len();
        self.joints.retain(|(i, _)| *i != id);
        self.joints.len() < before
    }

    /// Verilen ID'nin hâlâ geçerli olup olmadığını kontrol eder.
    ///
    /// Çağıran kod silme işleminden önce/sonra ID'yi doğrulamak istiyorsa
    /// kullanılır. `remove` sonrası aynı ID için `false` döner.
    #[inline]
    pub fn contains(&self, id: usize) -> bool {
        self.joints.iter().any(|(i, _)| *i == id)
    }

    /// ID'ye göre joint'e salt-okunur erişim.
    ///
    /// ID geçersizse `None` döner — asla paniklemez.
    #[inline]
    pub fn get(&self, id: usize) -> Option<&Joint> {
        self.joints.iter().find(|(i, _)| *i == id).map(|(_, j)| j)
    }

    /// ID'ye göre joint'e değiştirilebilir erişim.
    ///
    /// ID geçersizse `None` döner — asla paniklemez.
    #[inline]
    pub fn get_mut(&mut self, id: usize) -> Option<&mut Joint> {
        self.joints
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, j)| j)
    }

    /// Tüm (id, joint) çiftleri üzerinde iterator döndürür.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Joint)> {
        self.joints.iter().map(|(id, j)| (*id, j))
    }

    /// Tüm (id, joint) çiftleri üzerinde değiştirilebilir iterator döndürür.
    #[inline]
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (usize, &mut Joint)> {
        self.joints.iter_mut().map(|(id, j)| (*id, j))
    }

    /// Kayıtlı joint sayısını döndürür.
    #[inline]
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// Hiç joint yoksa `true` döner.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }
}

impl Default for JointWorld {
    fn default() -> Self {
        Self::new()
    }
}

/// Kısıtlayıcıları fizik adımında çözen sistem
/// 15 iterasyon ile kararlılık sağlar (Gauss-Seidel)

/// Hız çözümü iterasyonu. Sequential Impulse döngüsü içinde çağrılacaktır.
pub fn solve_joint_velocity(
    dt: f32,
    joint: &Joint,
    jb: &JointBodies,
    va_lin: &mut Vec3,
    va_ang: &mut Vec3,
    vb_lin: &mut Vec3,
    vb_ang: &mut Vec3,
) {
    let beta = 0.2;
    let pos_a = jb.pos_a;
    let pos_b = jb.pos_b;
    let rot_a = jb.rot_a;
    let rot_b = jb.rot_b;
    let inv_mass_a = jb.inv_mass_a;
    let inv_mass_b = jb.inv_mass_b;
    let inv_inertia_a = jb.inv_inertia_a;
    let inv_inertia_b = jb.inv_inertia_b;
    let total_inv_mass = jb.total_inv_mass;

    match &joint.kind {
        JointKind::BallSocket => {
            let r_a = rot_a.mul_vec3(joint.anchor_a);
            let r_b = rot_b.mul_vec3(joint.anchor_b);
            let diff = pos_b - pos_a;

            let bias = diff * (beta / dt) * joint.stiffness;

            let axes = [
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ];
            for &axis in &axes {
                let vel_a_anchor = *va_lin + va_ang.cross(r_a);
                let vel_b_anchor = *vb_lin + vb_ang.cross(r_b);
                let rel_vel = vel_b_anchor - vel_a_anchor;
                let rel_vel_n = rel_vel.dot(axis);
                let bias_n = bias.dot(axis);
                let r_a_cross_n = r_a.cross(axis);
                let r_b_cross_n = r_b.cross(axis);
                let r_a_cross_n_i = apply_inv_inertia(r_a_cross_n, inv_inertia_a, rot_a);
                let r_b_cross_n_i = apply_inv_inertia(r_b_cross_n, inv_inertia_b, rot_b);
                let eff_mass_inv = inv_mass_a
                    + inv_mass_b
                    + r_a_cross_n_i.dot(r_a_cross_n)
                    + r_b_cross_n_i.dot(r_b_cross_n);
                if eff_mass_inv > 0.0001 {
                    let lambda = -(rel_vel_n + bias_n) / eff_mass_inv;
                    let impulse = axis * lambda;
                    *va_lin -= impulse * inv_mass_a;
                    *va_ang -= apply_inv_inertia(r_a.cross(impulse), inv_inertia_a, rot_a);
                    *vb_lin += impulse * inv_mass_b;
                    *vb_ang += apply_inv_inertia(r_b.cross(impulse), inv_inertia_b, rot_b);
                }
            }
        }
        JointKind::Fixed { relative_rotation } => {
            let diff = pos_b - pos_a;
            let correction = diff * (beta / dt) * joint.stiffness;
            *va_lin += correction * (inv_mass_a / total_inv_mass);
            *vb_lin -= correction * (inv_mass_b / total_inv_mass);
            
            let target_rot = rot_a * *relative_rotation;
            let error_rot = target_rot * rot_b.conjugate();
            let (axis, angle) = error_rot.to_axis_angle();
            if angle.abs() > 0.001 {
                let angular_correction = axis * angle * (beta / dt) * joint.stiffness;
                *va_ang -= angular_correction * 0.5;
                *vb_ang += angular_correction * 0.5;
            }
        }
        JointKind::Distance { length } => {
            let r_a = rot_a.mul_vec3(joint.anchor_a);
            let r_b = rot_b.mul_vec3(joint.anchor_b);

            let diff = pos_b - pos_a;
            let current_len = diff.length();
            if current_len >= 0.0001 {
                let dir = diff / current_len;
                let error = current_len - length;

                let vel_a_anchor = *va_lin + va_ang.cross(r_a);
                let vel_b_anchor = *vb_lin + vb_ang.cross(r_b);
                let rel_vel_along_dir = (vel_b_anchor - vel_a_anchor).dot(dir);

                let r_a_cross_dir = r_a.cross(dir);
                let r_b_cross_dir = r_b.cross(dir);
                let ang_a = apply_inv_inertia(r_a_cross_dir, inv_inertia_a, rot_a);
                let ang_b = apply_inv_inertia(r_b_cross_dir, inv_inertia_b, rot_b);
                let eff_mass_inv = inv_mass_a
                    + inv_mass_b
                    + ang_a.dot(r_a_cross_dir)
                    + ang_b.dot(r_b_cross_dir);

                if eff_mass_inv >= 1e-8 {
                    let bias = error * (beta / dt) * joint.stiffness;
                    let lambda = -(rel_vel_along_dir + bias) / eff_mass_inv;
                    let impulse = dir * lambda;

                    *va_lin -= impulse * inv_mass_a;
                    *va_ang -= apply_inv_inertia(r_a.cross(impulse), inv_inertia_a, rot_a);
                    *vb_lin += impulse * inv_mass_b;
                    *vb_ang += apply_inv_inertia(r_b.cross(impulse), inv_inertia_b, rot_b);
                }
            }
        }
        JointKind::Spring {
            rest_length,
            spring_constant,
        } => {
            let diff = pos_b - pos_a;
            let current_len = diff.length();
            if current_len >= 0.0001 {
                let dir = diff / current_len;
                let displacement = current_len - rest_length;
                let spring_force = dir * displacement * (*spring_constant);
                let damping_force = dir * (*vb_lin - *va_lin).dot(dir) * joint.damping;
                let total_force = spring_force + damping_force;
                *va_lin += total_force * inv_mass_a * dt;
                *vb_lin -= total_force * inv_mass_b * dt;
            }
        }
        JointKind::Hinge {
            axis,
            min_angle,
            max_angle,
        } => {
            let diff = pos_b - pos_a;
            let bias = diff * (beta / dt) * joint.stiffness;

            let r_a = rot_a.mul_vec3(joint.anchor_a);
            let r_b = rot_b.mul_vec3(joint.anchor_b);

            for &constraint_axis in &[Vec3::X, Vec3::Y, Vec3::Z] {
                let vel_a_anchor = *va_lin + va_ang.cross(r_a);
                let vel_b_anchor = *vb_lin + vb_ang.cross(r_b);
                let rel_vel_n = (vel_b_anchor - vel_a_anchor).dot(constraint_axis);
                let bias_n = bias.dot(constraint_axis);
                let ra_x_n = r_a.cross(constraint_axis);
                let rb_x_n = r_b.cross(constraint_axis);
                let ia = apply_inv_inertia(ra_x_n, inv_inertia_a, rot_a);
                let ib = apply_inv_inertia(rb_x_n, inv_inertia_b, rot_b);
                let eff = inv_mass_a + inv_mass_b + ia.dot(ra_x_n) + ib.dot(rb_x_n);
                if eff > 1e-6 {
                    let lambda = -(rel_vel_n + bias_n) / eff;
                    let impulse = constraint_axis * lambda;
                    *va_lin -= impulse * inv_mass_a;
                    *va_ang -= apply_inv_inertia(r_a.cross(impulse), inv_inertia_a, rot_a);
                    *vb_lin += impulse * inv_mass_b;
                    *vb_ang += apply_inv_inertia(r_b.cross(impulse), inv_inertia_b, rot_b);
                }
            }

            let hinge_axis_world = rot_a.mul_vec3(*axis);
            let perp1 = {
                let candidate = if hinge_axis_world.x.abs() < 0.9 {
                    Vec3::X
                } else {
                    Vec3::Y
                };
                hinge_axis_world.cross(candidate).normalize()
            };
            let perp2 = hinge_axis_world.cross(perp1);

            for &perp in &[perp1, perp2] {
                let rel_ang_n = (*vb_ang - *va_ang).dot(perp);
                let ia = apply_inv_inertia(perp, inv_inertia_a, rot_a);
                let ib = apply_inv_inertia(perp, inv_inertia_b, rot_b);
                let eff_ang = ia.dot(perp) + ib.dot(perp);
                if eff_ang > 1e-8 {
                    let lambda_ang = -rel_ang_n / eff_ang;
                    let ang_impulse = perp * lambda_ang;
                    *va_ang -= apply_inv_inertia(ang_impulse, inv_inertia_a, rot_a);
                    *vb_ang += apply_inv_inertia(ang_impulse, inv_inertia_b, rot_b);
                }
            }

            if *min_angle > f32::NEG_INFINITY || *max_angle < f32::INFINITY {
                let hinge_world = rot_a.mul_vec3(*axis);
                let rel_rot = rot_a.conjugate() * rot_b;
                let (rel_axis_local, angle) = rel_rot.to_axis_angle();
                let rel_axis_world = rot_a.mul_vec3(rel_axis_local);
                let signed_angle = if rel_axis_world.dot(hinge_world) >= 0.0 {
                    angle
                } else {
                    -angle
                };

                let mut error = 0.0;
                if signed_angle < *min_angle {
                    error = *min_angle - signed_angle;
                } else if signed_angle > *max_angle {
                    error = signed_angle - *max_angle;
                }
                
                if error != 0.0 {
                    let constraint_axis = if signed_angle < *min_angle {
                        hinge_world
                    } else {
                        -hinge_world
                    };
                    
                    let rel_ang_n = (*vb_ang - *va_ang).dot(constraint_axis);
                    let bias_n = error * (beta / dt);
                    
                    let ia = apply_inv_inertia(constraint_axis, inv_inertia_a, rot_a);
                    let ib = apply_inv_inertia(constraint_axis, inv_inertia_b, rot_b);
                    let eff_ang = ia.dot(constraint_axis) + ib.dot(constraint_axis);
                    
                    // Debug kaldırılarak temizlendi
                    
                    if eff_ang > 1e-8 {
                        let lambda_ang = -(rel_ang_n + bias_n) / eff_ang;
                        let ang_impulse = constraint_axis * lambda_ang;
                        *va_ang -= apply_inv_inertia(ang_impulse, inv_inertia_a, rot_a);
                        *vb_ang += apply_inv_inertia(ang_impulse, inv_inertia_b, rot_b);
                    }
                }
            }
        }
    }
}

/// Pozisyonların son düzeltmesi. Island solver bittikten sonra çalıştırılır.
pub fn solve_joint_position(
    joint: &Joint,
    jb: &JointBodies,
    pos_a_mut: &mut Vec3,
    pos_b_mut: &mut Vec3,
) {
    const POSITION_CORRECTION_FACTOR: f32 = 0.8;
    const POSITION_SLOP: f32 = 0.001;

    let pos_a = jb.pos_a;
    let pos_b = jb.pos_b;
    let inv_mass_a = jb.inv_mass_a;
    let inv_mass_b = jb.inv_mass_b;
    let total_inv_mass = jb.total_inv_mass;
    if total_inv_mass == 0.0 {
        return;
    }

    match &joint.kind {
        JointKind::BallSocket | JointKind::Fixed { .. } | JointKind::Hinge { .. } => {
            let error = pos_b - pos_a;
            let error_len = error.length();
            if error_len > POSITION_SLOP {
                let correction = error * (POSITION_CORRECTION_FACTOR / total_inv_mass);
                *pos_a_mut += correction * inv_mass_a;
                *pos_b_mut -= correction * inv_mass_b;
            }
        }
        JointKind::Distance { length } => {
            let diff = pos_b - pos_a;
            let current_len = diff.length();
            if current_len >= 0.0001 {
                let dir = diff / current_len;
                let error = current_len - length;
                if error.abs() > POSITION_SLOP {
                    let correction = dir * (error * POSITION_CORRECTION_FACTOR / total_inv_mass);
                    *pos_a_mut += correction * inv_mass_a;
                    *pos_b_mut -= correction * inv_mass_b;
                }
            }
        }
        JointKind::Spring { .. } => {}
    }
}
