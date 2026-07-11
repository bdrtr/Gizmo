use gizmo_math::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

use super::Velocity;
use gizmo_physics_core::{Collider, ColliderShape};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub enum BodyType {
    Dynamic,   // Fully simulated
    Kinematic, // Moved by user, affects others
    Static,    // Never moves
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct RigidBody {
    pub body_type: BodyType,
    pub mass: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Aerodinamik sürükleme katsayısı (Cd). `drag_area` ile birlikte >0 olduğunda
    /// integrator fiziksel hava direnci uygular: F = ½·ρ·Cd·A·|v|², hıza KARŞI. 0 =
    /// kapalı (varsayılan). `linear_damping` (hıza-doğrusal exp sönüm) kaba bir proxy
    /// iken bu gerçekçi v² sürüklemesidir → cisim doğal terminal hıza oturur
    /// (v_term = √(2·m·g / (ρ·Cd·A))). Hava yoğunluğu `Integrator::air_density`'den.
    /// `#[serde(default)]`: bu alan eklenmeden önce kaydedilmiş sahneler `0.0` (kapalı)
    /// olarak yüklenir.
    #[serde(default)]
    pub drag_coefficient: f32,
    /// Sürüklemeye maruz referans (frontal) alan, m². `drag_coefficient` ile birlikte
    /// >0 olduğunda hava direnci aktif olur.
    #[serde(default)]
    pub drag_area: f32,
    pub use_gravity: bool,
    pub is_sleeping: bool,
    pub ccd_enabled: bool,
    pub local_inertia: Vec3,
    pub lock_rotation_x: bool,
    pub lock_rotation_y: bool,
    pub lock_rotation_z: bool,
    pub lock_translation_x: bool,
    pub lock_translation_y: bool,
    pub lock_translation_z: bool,
    pub sleep_counter: u32, // Frames below sleep threshold
    pub center_of_mass: Vec3,
    pub fracture_threshold: Option<f32>, // Impulse threshold for fracturing
    pub force_accumulator: Vec3,
    pub torque_accumulator: Vec3,
}

impl Default for RigidBody {
    fn default() -> Self {
        Self {
            body_type: BodyType::Dynamic,
            mass: 1.0,
            linear_damping: 0.01,
            angular_damping: 0.05,
            drag_coefficient: 0.0, // opt-in: 0 = hava direnci kapalı
            drag_area: 0.0,
            use_gravity: true,
            is_sleeping: false,
            ccd_enabled: false,
            local_inertia: Vec3::splat(1.0),
            lock_rotation_x: false,
            lock_rotation_y: false,
            lock_rotation_z: false,
            lock_translation_x: false,
            lock_translation_y: false,
            lock_translation_z: false,
            sleep_counter: 0,
            center_of_mass: Vec3::ZERO,
            fracture_threshold: None,
            force_accumulator: Vec3::ZERO,
            torque_accumulator: Vec3::ZERO,
        }
    }
}

impl RigidBody {
    /// Creates a dynamic body of the given `mass`. Contact **friction** and
    /// **restitution** are NOT stored on the body — they are taken from the
    /// colliders' [`PhysicsMaterial`](gizmo_physics_core::PhysicsMaterial)
    /// (combined per contact), so configure them there.
    pub fn new(mass: f32, use_gravity: bool) -> Self {
        Self {
            mass,
            use_gravity,
            ..Default::default()
        }
    }

    /// Fiziksel hava direncini açar: F = ½·ρ·Cd·A·|v|² (hıza karşı). `cd` sürükleme
    /// katsayısı (küre ~0.47, küp ~1.05, akıcı gövde ~0.04), `area` frontal alan (m²).
    /// Yerçekimi altında cisim doğal terminal hıza oturur. Zincirlenebilir.
    pub fn with_air_drag(mut self, cd: f32, area: f32) -> Self {
        self.drag_coefficient = cd.max(0.0);
        self.drag_area = area.max(0.0);
        self
    }

    pub fn new_static() -> Self {
        Self {
            body_type: BodyType::Static,
            mass: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            use_gravity: false,
            is_sleeping: true,
            local_inertia: Vec3::ZERO,
            lock_rotation_x: true,
            lock_rotation_y: true,
            lock_rotation_z: true,
            lock_translation_x: true,
            lock_translation_y: true,
            lock_translation_z: true,
            ..Default::default()
        }
    }

    pub fn new_kinematic() -> Self {
        Self {
            body_type: BodyType::Kinematic,
            mass: 0.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            use_gravity: false,
            ccd_enabled: true,
            local_inertia: Vec3::ZERO,
            ..Default::default()
        }
    }

    pub fn with_fracture_threshold(mut self, threshold: f32) -> Self {
        self.fracture_threshold = Some(threshold);
        self
    }

    /// Lineer + açısal sönümü ayarlar (kaba, hıza-doğrusal enerji kaybı). Gerçekçi
    /// v² hava direnci için [`with_air_drag`](Self::with_air_drag) kullan. Zincirlenebilir.
    pub fn with_damping(mut self, linear: f32, angular: f32) -> Self {
        self.linear_damping = linear.max(0.0);
        self.angular_damping = angular.max(0.0);
        self
    }

    /// Yerçekimini aç/kapat (uçan/asılı cisimler için). Zincirlenebilir.
    pub fn with_gravity(mut self, enabled: bool) -> Self {
        self.use_gravity = enabled;
        self
    }

    /// Sürekli Çarpışma Tespiti'ni (CCD) açar — hızlı/ince cisimler tünellemez. Zincirlenebilir.
    pub fn with_ccd(mut self) -> Self {
        self.ccd_enabled = true;
        self
    }

    /// Kütle merkezini (gövde-yerel) ayarlar. Zincirlenebilir.
    pub fn with_center_of_mass(mut self, com: Vec3) -> Self {
        self.center_of_mass = com;
        self
    }

    /// Üç dönme eksenini de kilitler — cisim devrilmez/dönmez (karakter kapsülü, dik
    /// duran nesneler). Zincirlenebilir.
    pub fn lock_rotation(mut self) -> Self {
        self.lock_rotation_x = true;
        self.lock_rotation_y = true;
        self.lock_rotation_z = true;
        self
    }

    pub fn wake_up(&mut self) {
        self.is_sleeping = false;
        self.sleep_counter = 0;
    }

    pub fn can_sleep(&self, velocity: &Velocity) -> bool {
        if self.is_kinematic() {
            return false; // Kinematic bodies never sleep — user controls their motion
        }
        if !self.is_dynamic() {
            return true; // Static bodies are always "asleep"
        }

        const SLEEP_LINEAR_THRESHOLD: f32 = 0.05;
        const SLEEP_ANGULAR_THRESHOLD: f32 = 0.05;

        velocity.linear.length_squared() < SLEEP_LINEAR_THRESHOLD * SLEEP_LINEAR_THRESHOLD
            && velocity.angular.length_squared() < SLEEP_ANGULAR_THRESHOLD * SLEEP_ANGULAR_THRESHOLD
    }

    pub fn update_sleep_state(&mut self, velocity: &Velocity) {
        const SLEEP_FRAMES_REQUIRED: u32 = 60; // ~1 second at 60fps

        if self.can_sleep(velocity) {
            self.sleep_counter += 1;
            if self.sleep_counter >= SLEEP_FRAMES_REQUIRED {
                self.is_sleeping = true;
            }
        } else {
            self.sleep_counter = 0;
            self.is_sleeping = false;
        }
    }

    #[inline]
    pub fn is_dynamic(&self) -> bool {
        matches!(self.body_type, BodyType::Dynamic)
    }

    #[inline]
    pub fn is_kinematic(&self) -> bool {
        matches!(self.body_type, BodyType::Kinematic)
    }

    #[inline]
    pub fn is_static(&self) -> bool {
        matches!(self.body_type, BodyType::Static)
    }

    #[inline]
    pub fn enforce_locks(&self, vel: &mut Velocity) {
        if self.lock_translation_x {
            vel.linear.x = 0.0;
        }
        if self.lock_translation_y {
            vel.linear.y = 0.0;
        }
        if self.lock_translation_z {
            vel.linear.z = 0.0;
        }
        if self.lock_rotation_x {
            vel.angular.x = 0.0;
        }
        if self.lock_rotation_y {
            vel.angular.y = 0.0;
        }
        if self.lock_rotation_z {
            vel.angular.z = 0.0;
        }
    }

    #[inline]
    pub fn inv_mass(&self) -> f32 {
        if self.mass == 0.0 || !self.is_dynamic() {
            0.0
        } else {
            1.0 / self.mass
        }
    }

    #[inline]
    pub fn inv_local_inertia(&self) -> Vec3 {
        if self.mass == 0.0 || !self.is_dynamic() {
            Vec3::ZERO
        } else {
            Vec3::new(
                if self.local_inertia.x == 0.0 {
                    0.0
                } else {
                    1.0 / self.local_inertia.x
                },
                if self.local_inertia.y == 0.0 {
                    0.0
                } else {
                    1.0 / self.local_inertia.y
                },
                if self.local_inertia.z == 0.0 {
                    0.0
                } else {
                    1.0 / self.local_inertia.z
                },
            )
        }
    }

    /// Get inverse world-space inertia tensor
    pub fn inv_world_inertia_tensor_identity(&self) -> Mat3 {
        Mat3::from_diagonal(self.inv_local_inertia())
    }

    /// Get world-space inertia tensor from local inertia and rotation
    pub fn world_inertia_tensor(&self, rotation: Quat) -> Mat3 {
        let rot_mat = Mat3::from_quat(rotation);
        let local_inertia_mat = Mat3::from_diagonal(self.local_inertia);
        rot_mat * local_inertia_mat * rot_mat.transpose()
    }

    /// Get inverse world-space inertia tensor
    pub fn inv_world_inertia_tensor(&self, rotation: Quat) -> Mat3 {
        if self.mass == 0.0 || !self.is_dynamic() {
            return Mat3::ZERO;
        }
        let rot_mat = Mat3::from_quat(rotation);
        let inv_local = Mat3::from_diagonal(self.inv_local_inertia());
        let mut inv_world = rot_mat * inv_local * rot_mat.transpose();

        // Zero out locked world axes
        if self.lock_rotation_x {
            inv_world.x_axis = Vec3::ZERO;
            inv_world.y_axis.x = 0.0;
            inv_world.z_axis.x = 0.0;
        }
        if self.lock_rotation_y {
            inv_world.y_axis = Vec3::ZERO;
            inv_world.x_axis.y = 0.0;
            inv_world.z_axis.y = 0.0;
        }
        if self.lock_rotation_z {
            inv_world.z_axis = Vec3::ZERO;
            inv_world.x_axis.z = 0.0;
            inv_world.y_axis.z = 0.0;
        }

        inv_world
    }

    pub fn clear_forces(&mut self) {
        self.force_accumulator = Vec3::ZERO;
        self.torque_accumulator = Vec3::ZERO;
    }

    pub fn calculate_box_inertia(&mut self, w: f32, h: f32, d: f32) {
        let m = self.mass;
        self.local_inertia = Vec3::new(
            (m / 12.0) * (h * h + d * d),
            (m / 12.0) * (w * w + d * d),
            (m / 12.0) * (w * w + h * h),
        );
    }

    pub fn calculate_sphere_inertia(&mut self, r: f32) {
        let i = 0.4 * self.mass * r * r;
        self.local_inertia = Vec3::splat(i);
    }

    pub fn calculate_capsule_inertia(&mut self, r: f32, half_h: f32) {
        let m = self.mass;
        let h = half_h * 2.0;
        let vol_cyl = std::f32::consts::PI * r * r * h;
        let vol_sph = 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
        let total_vol = vol_cyl + vol_sph;

        let m_cyl = if total_vol > 0.0 {
            m * vol_cyl / total_vol
        } else {
            0.0
        };
        let m_sph = if total_vol > 0.0 {
            m * vol_sph / total_vol
        } else {
            0.0
        };

        let i_y = m_cyl * (r * r) / 2.0 + m_sph * 2.0 * (r * r) / 5.0;
        let i_cyl_xz = m_cyl * (3.0 * r * r + h * h) / 12.0;
        // İki yarımküre kabın enine (i_xz) katkısı. Yarımkürenin düz-yüzey
        // merkezindeki enine ataleti 2/5·m·r²'dir; bunu kapsül merkezine paralel-
        // eksen ile taşırken yarımküre COM-offset terimi (9/64·r²) SADELEŞİR.
        // Eski kod ayrıca bir +9/64·r² (=0.140625·r²) ekliyordu → çift sayım
        // (enine atalet ~%5–35 fazla, kapsül devrilmeye aşırı dirençli).
        let i_sph_xz = m_sph * (0.4 * r * r + half_h * half_h + 0.75 * r * half_h);
        let i_xz = i_cyl_xz + i_sph_xz;

        self.local_inertia = Vec3::new(i_xz, i_y, i_xz);
    }

    pub fn update_inertia_from_collider(&mut self, collider: &Collider) {
        match &collider.shape {
            ColliderShape::Box(b) => {
                let w = b.half_extents.x * 2.0;
                let h = b.half_extents.y * 2.0;
                let d = b.half_extents.z * 2.0;
                self.calculate_box_inertia(w, h, d);
            }
            ColliderShape::Sphere(s) => {
                self.calculate_sphere_inertia(s.radius);
            }
            ColliderShape::Capsule(c) => {
                self.calculate_capsule_inertia(c.radius, c.half_height);
            }
            ColliderShape::Plane(_) => {
                self.local_inertia = Vec3::splat(f32::INFINITY);
            }
            ColliderShape::ConvexHull(hull) => {
                // AABB'den kutu ataleti türet (eskiden sabit 1×1×1 idi → tüm fracture
                // parçaları boyuttan bağımsız aynı atalete sahip oluyordu, yanlış takla).
                let mut mn = Vec3::splat(f32::INFINITY);
                let mut mx = Vec3::splat(f32::NEG_INFINITY);
                for &v in hull.vertices.iter() {
                    mn = mn.min(v);
                    mx = mx.max(v);
                }
                if mn.x <= mx.x {
                    let e = mx - mn;
                    self.calculate_box_inertia(e.x.max(1e-3), e.y.max(1e-3), e.z.max(1e-3));
                } else {
                    self.calculate_box_inertia(1.0, 1.0, 1.0);
                }
            }
            ColliderShape::TriMesh(_) => {
                self.calculate_box_inertia(1.0, 1.0, 1.0);
            }
            ColliderShape::Compound(shapes) => {
                let mut total_vol = 0.0;
                let mut vols = Vec::with_capacity(shapes.len());
                for (_, sub_shape) in shapes {
                    let temp_col = Collider::from_shape((**sub_shape).clone());
                    let v = temp_col.volume();
                    vols.push(v);
                    total_vol += v;
                }

                if total_vol > 0.0 {
                    let mut com = Vec3::ZERO;
                    for (i, (local_t, _)) in shapes.iter().enumerate() {
                        let mass_i = (vols[i] / total_vol) * self.mass;
                        com += local_t.position * mass_i;
                    }
                    if self.mass > 0.0 {
                        self.center_of_mass = com / self.mass;
                    }

                    let mut inertia = Vec3::ZERO;
                    for (i, (local_t, sub_shape)) in shapes.iter().enumerate() {
                        let mass_i = (vols[i] / total_vol) * self.mass;

                        let mut temp_rb = RigidBody {
                            mass: mass_i,
                            ..Default::default()
                        };
                        let temp_col = Collider::from_shape((**sub_shape).clone());
                        temp_rb.update_inertia_from_collider(&temp_col);

                        let d = local_t.position - self.center_of_mass;
                        let d_sq = d.length_squared();

                        inertia.x += temp_rb.local_inertia.x + mass_i * (d_sq - d.x * d.x);
                        inertia.y += temp_rb.local_inertia.y + mass_i * (d_sq - d.y * d.y);
                        inertia.z += temp_rb.local_inertia.z + mass_i * (d_sq - d.z * d.z);
                    }
                    self.local_inertia = inertia;
                } else {
                    self.calculate_box_inertia(1.0, 1.0, 1.0);
                }
            }
        }
    }
}

gizmo_core::impl_component!(RigidBody);

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::collider::ConvexHullShape;
    use std::sync::Arc;

    /// Akıcı builder'lar: alan erişimi yerine tek zincirde yapılandırma.
    #[test]
    fn ergonomic_rigid_body_builders() {
        let rb = RigidBody::new(5.0, true)
            .with_damping(0.2, 0.4)
            .with_gravity(false)
            .with_ccd()
            .with_center_of_mass(Vec3::new(0.0, 0.3, 0.0))
            .lock_rotation()
            .with_air_drag(0.5, 1.2);
        assert_eq!(rb.linear_damping, 0.2);
        assert_eq!(rb.angular_damping, 0.4);
        assert!(!rb.use_gravity);
        assert!(rb.ccd_enabled);
        assert_eq!(rb.center_of_mass, Vec3::new(0.0, 0.3, 0.0));
        assert!(rb.lock_rotation_x && rb.lock_rotation_y && rb.lock_rotation_z);
        assert_eq!(rb.drag_coefficient, 0.5);
        assert_eq!(rb.drag_area, 1.2);
        // Clamp: negatif damping 0'a.
        assert_eq!(RigidBody::default().with_damping(-1.0, -2.0).linear_damping, 0.0);
    }

    /// ConvexHull ataleti AABB'den türetilmeli (eskiden sabit 1×1×1 idi → fracture
    /// parçaları boyuttan bağımsız aynı atalete sahipti).
    #[test]
    fn convex_hull_inertia_uses_aabb_extents() {
        // 4×2×6 kutuyu kapsayan köşeler.
        let verts = vec![
            Vec3::new(-2.0, -1.0, -3.0),
            Vec3::new(2.0, -1.0, -3.0),
            Vec3::new(2.0, 1.0, -3.0),
            Vec3::new(-2.0, 1.0, -3.0),
            Vec3::new(-2.0, -1.0, 3.0),
            Vec3::new(2.0, -1.0, 3.0),
            Vec3::new(2.0, 1.0, 3.0),
            Vec3::new(-2.0, 1.0, 3.0),
        ];
        let hull_col = Collider::from_shape(ColliderShape::ConvexHull(ConvexHullShape {
            vertices: Arc::new(verts),
            faces: Arc::new(vec![]),
        }));

        let mut rb_hull = RigidBody::new(8.0, true);
        rb_hull.update_inertia_from_collider(&hull_col);

        // Aynı boyutlu kutu ataletiyle eşleşmeli.
        let mut rb_box = RigidBody::new(8.0, true);
        rb_box.calculate_box_inertia(4.0, 2.0, 6.0);
        assert!(
            (rb_hull.local_inertia - rb_box.local_inertia).length() < 1e-3,
            "hull ataleti AABB kutusuyla eşleşmeli: {:?} vs {:?}",
            rb_hull.local_inertia,
            rb_box.local_inertia
        );

        // ve 1×1×1'den belirgin farklı olmalı.
        let mut rb_unit = RigidBody::new(8.0, true);
        rb_unit.calculate_box_inertia(1.0, 1.0, 1.0);
        assert!(
            (rb_hull.local_inertia - rb_unit.local_inertia).length() > 1e-3,
            "hull ataleti 1×1×1'den farklı olmalı"
        );
    }

    /// Kapsül enine ataleti (i_xz) analitik değerle eşleşmeli. Regresyon: eskiden
    /// yarımküre paralel-eksen COM-offset terimi (9/64·r² = 0.140625·r²) çift
    /// sayılıyordu → enine atalet fazla hesaplanıyordu (kapsül devrilmeye aşırı
    /// dirençliydi). Doğru yarımküre-çifti katkısı: m_sph·(2/5·r² + half_h² +
    /// 3/4·r·half_h) — fazladan COM-offset terimi YOK.
    #[test]
    fn capsule_transverse_inertia_has_no_spurious_com_term() {
        let r = 0.5_f32;
        let half_h = 1.0_f32;
        let mass = 4.0_f32;

        let mut rb = RigidBody::new(mass, true);
        rb.calculate_capsule_inertia(r, half_h);

        // Kütle, hacme göre silindir ve küre (iki yarımküre) arasında paylaştırılır.
        let h = half_h * 2.0;
        let vol_cyl = std::f32::consts::PI * r * r * h;
        let vol_sph = 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
        let total = vol_cyl + vol_sph;
        let m_cyl = mass * vol_cyl / total;
        let m_sph = mass * vol_sph / total;

        let i_cyl_xz = m_cyl * (3.0 * r * r + h * h) / 12.0;
        // Analitik olarak doğru katkı (9/64·r² terimi OLMADAN):
        let correct_i_sph_xz = m_sph * (0.4 * r * r + half_h * half_h + 0.75 * r * half_h);
        let expected_i_xz = i_cyl_xz + correct_i_sph_xz;

        assert!(
            (rb.local_inertia.x - expected_i_xz).abs() < 1e-6,
            "kapsül enine ataleti analitik değerle eşleşmeli: {} vs {}",
            rb.local_inertia.x,
            expected_i_xz
        );
        assert_eq!(rb.local_inertia.x, rb.local_inertia.z, "i_xz simetrik olmalı");

        // Spin ekseni (i_y) değişmemeli.
        let expected_i_y = m_cyl * (r * r) / 2.0 + m_sph * 2.0 * (r * r) / 5.0;
        assert!(
            (rb.local_inertia.y - expected_i_y).abs() < 1e-6,
            "kapsül spin ekseni ataleti korunmalı"
        );

        // Eski (hatalı) formül belirgin şekilde daha büyüktü → yeni değer onun altında.
        let buggy_i_sph_xz =
            m_sph * (0.4 * r * r + half_h * half_h + 0.75 * r * half_h + 0.140625 * r * r);
        assert!(
            correct_i_sph_xz < buggy_i_sph_xz,
            "doğru enine atalet eski hatalı değerin altında olmalı"
        );
    }
}
