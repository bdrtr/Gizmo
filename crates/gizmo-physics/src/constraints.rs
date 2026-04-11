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
    pub fn fixed(entity_a: u32, entity_b: u32, anchor_a: Vec3, anchor_b: Vec3) -> Self {
        Self {
            kind: JointKind::Fixed {
                relative_rotation: gizmo_math::Quat::IDENTITY,
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
pub fn solve_constraints(joint_world: &JointWorld, world: &gizmo_core::World, dt: f32) {
    if joint_world.joints.is_empty() {
        return;
    }

    let beta = 0.2;
    let iterations = 15;

    // Tüm borrow'ları DÖNGÜ DIŞINDA bir kez al — RefCell overhead ortadan kalkar
    let transforms = match world.borrow::<crate::components::Transform>() {
        Some(t) => t,
        None => return,
    };
    let rbs = match world.borrow::<crate::components::RigidBody>() {
        Some(r) => r,
        None => return,
    };
    let mut vels = match world.borrow_mut::<crate::components::Velocity>() {
        Some(v) => v,
        None => return,
    };

    for _iter in 0..iterations {
        for (_, joint) in &joint_world.joints {
            let ta = match transforms.get(joint.entity_a) {
                Some(ta) => *ta,
                None => continue,
            };
            let tb = match transforms.get(joint.entity_b) {
                Some(tb) => *tb,
                None => continue,
            };

            let pos_a = ta.position + ta.rotation.mul_vec3(joint.anchor_a);
            let pos_b = tb.position + tb.rotation.mul_vec3(joint.anchor_b);
            let rot_a = ta.rotation;
            let rot_b = tb.rotation;

            let (inv_mass_a, inv_inertia_a) =
                rbs.get(joint.entity_a).map_or((0.0, Vec3::ZERO), |rb| {
                    if rb.mass > 0.0 {
                        (1.0 / rb.mass, rb.inverse_inertia)
                    } else {
                        (0.0, Vec3::ZERO)
                    }
                });
            let (inv_mass_b, inv_inertia_b) =
                rbs.get(joint.entity_b).map_or((0.0, Vec3::ZERO), |rb| {
                    if rb.mass > 0.0 {
                        (1.0 / rb.mass, rb.inverse_inertia)
                    } else {
                        (0.0, Vec3::ZERO)
                    }
                });
            let total_inv_mass = inv_mass_a + inv_mass_b;
            if total_inv_mass == 0.0 {
                continue;
            }

            match &joint.kind {
                JointKind::BallSocket => {
                    let r_a = rot_a.mul_vec3(joint.anchor_a);
                    let r_b = rot_b.mul_vec3(joint.anchor_b);
                    let diff = pos_b - pos_a;

                    let mut va_lin = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.linear);
                    let mut va_ang = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.angular);
                    let mut vb_lin = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.linear);
                    let mut vb_ang = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.angular);

                    let bias = diff * (beta / dt) * joint.stiffness;

                    let axes = [
                        Vec3::new(1.0, 0.0, 0.0),
                        Vec3::new(0.0, 1.0, 0.0),
                        Vec3::new(0.0, 0.0, 1.0),
                    ];
                    for &axis in &axes {
                        let vel_a_anchor = va_lin + va_ang.cross(r_a);
                        let vel_b_anchor = vb_lin + vb_ang.cross(r_b);
                        let rel_vel = vel_b_anchor - vel_a_anchor;
                        let rel_vel_n = rel_vel.dot(axis);
                        let bias_n = bias.dot(axis);
                        let r_a_cross_n = r_a.cross(axis);
                        let r_b_cross_n = r_b.cross(axis);
                        let r_a_cross_n_i = Vec3::new(
                            r_a_cross_n.x * inv_inertia_a.x,
                            r_a_cross_n.y * inv_inertia_a.y,
                            r_a_cross_n.z * inv_inertia_a.z,
                        );
                        let r_b_cross_n_i = Vec3::new(
                            r_b_cross_n.x * inv_inertia_b.x,
                            r_b_cross_n.y * inv_inertia_b.y,
                            r_b_cross_n.z * inv_inertia_b.z,
                        );
                        let eff_mass_inv = inv_mass_a
                            + inv_mass_b
                            + r_a_cross_n_i.dot(r_a_cross_n)
                            + r_b_cross_n_i.dot(r_b_cross_n);
                        if eff_mass_inv > 0.0001 {
                            let lambda = -(rel_vel_n + bias_n) / eff_mass_inv;
                            let impulse = axis * lambda;
                            va_lin -= impulse * inv_mass_a;
                            let va_torque = r_a.cross(impulse);
                            va_ang -= Vec3::new(
                                va_torque.x * inv_inertia_a.x,
                                va_torque.y * inv_inertia_a.y,
                                va_torque.z * inv_inertia_a.z,
                            );
                            vb_lin += impulse * inv_mass_b;
                            let vb_torque = r_b.cross(impulse);
                            vb_ang += Vec3::new(
                                vb_torque.x * inv_inertia_b.x,
                                vb_torque.y * inv_inertia_b.y,
                                vb_torque.z * inv_inertia_b.z,
                            );
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
                JointKind::Fixed { relative_rotation } => {
                    let diff = pos_b - pos_a;
                    let correction = diff * (beta / dt) * joint.stiffness;
                    if let Some(v_a) = vels.get_mut(joint.entity_a) {
                        v_a.linear += correction * (inv_mass_a / total_inv_mass);
                    }
                    if let Some(v_b) = vels.get_mut(joint.entity_b) {
                        v_b.linear -= correction * (inv_mass_b / total_inv_mass);
                    }
                    let target_rot = rot_a * *relative_rotation;
                    let error_rot = target_rot * rot_b.conjugate();
                    let (axis, angle) = error_rot.to_axis_angle();
                    if angle.abs() > 0.001 {
                        let angular_correction = axis * angle * (beta / dt) * joint.stiffness;
                        if let Some(v_a) = vels.get_mut(joint.entity_a) {
                            v_a.angular -= angular_correction * 0.5;
                        }
                        if let Some(v_b) = vels.get_mut(joint.entity_b) {
                            v_b.angular += angular_correction * 0.5;
                        }
                    }
                }
                JointKind::Distance { length } => {
                    // Anchor'lardan merkeze lever arm (world-space)
                    let r_a = rot_a.mul_vec3(joint.anchor_a);
                    let r_b = rot_b.mul_vec3(joint.anchor_b);

                    let diff = pos_b - pos_a;
                    let current_len = diff.length();
                    if current_len < 0.0001 {
                        continue;
                    }
                    let dir = diff / current_len;
                    let error = current_len - length;

                    // Anchor hızlarını hesapla (linear + angular katkısıyla)
                    let va_lin = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.linear);
                    let va_ang = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.angular);
                    let vb_lin = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.linear);
                    let vb_ang = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.angular);

                    let vel_a_anchor = va_lin + va_ang.cross(r_a);
                    let vel_b_anchor = vb_lin + vb_ang.cross(r_b);
                    let rel_vel_along_dir = (vel_b_anchor - vel_a_anchor).dot(dir);

                    // Etkin kütle: Lineer + Rotasyonel katkı
                    // Önceki hata: total_inv_mass yalnızca lineer kütleyi kapsıyordu →
                    // impulse fazla büyük, angular kol hiç güncellenmiyordu
                    let r_a_cross_dir = r_a.cross(dir);
                    let r_b_cross_dir = r_b.cross(dir);
                    let ang_a = Vec3::new(
                        r_a_cross_dir.x * inv_inertia_a.x,
                        r_a_cross_dir.y * inv_inertia_a.y,
                        r_a_cross_dir.z * inv_inertia_a.z,
                    );
                    let ang_b = Vec3::new(
                        r_b_cross_dir.x * inv_inertia_b.x,
                        r_b_cross_dir.y * inv_inertia_b.y,
                        r_b_cross_dir.z * inv_inertia_b.z,
                    );
                    let eff_mass_inv = inv_mass_a
                        + inv_mass_b
                        + ang_a.dot(r_a_cross_dir)
                        + ang_b.dot(r_b_cross_dir);

                    if eff_mass_inv < 1e-8 {
                        continue;
                    }

                    // Baumgarte konum düzeltmesi + hız düzeltmesi
                    let bias = error * (beta / dt) * joint.stiffness;
                    let lambda = -(rel_vel_along_dir + bias) / eff_mass_inv;
                    let impulse = dir * lambda;

                    // Linear VE Angular impulse uygula
                    // Önceki kod sadece linear uyguluyordu → sarkık obje sallanmak yerine dönüyordu
                    if let Some(v_a) = vels.get_mut(joint.entity_a) {
                        v_a.linear -= impulse * inv_mass_a;
                        // angular -= I⁻¹ · (r_a × impulse)
                        let torque_a = r_a.cross(impulse);
                        v_a.angular -= Vec3::new(
                            torque_a.x * inv_inertia_a.x,
                            torque_a.y * inv_inertia_a.y,
                            torque_a.z * inv_inertia_a.z,
                        );
                    }
                    if let Some(v_b) = vels.get_mut(joint.entity_b) {
                        v_b.linear += impulse * inv_mass_b;
                        // angular += I⁻¹ · (r_b × impulse)
                        let torque_b = r_b.cross(impulse);
                        v_b.angular += Vec3::new(
                            torque_b.x * inv_inertia_b.x,
                            torque_b.y * inv_inertia_b.y,
                            torque_b.z * inv_inertia_b.z,
                        );
                    }
                }
                JointKind::Spring {
                    rest_length,
                    spring_constant,
                } => {
                    let diff = pos_b - pos_a;
                    let current_len = diff.length();
                    if current_len < 0.0001 {
                        continue;
                    }
                    let dir = diff / current_len;
                    let displacement = current_len - rest_length;
                    let spring_force = dir * displacement * (*spring_constant);
                    let va = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.linear);
                    let vb = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.linear);
                    let damping_force = dir * (vb - va).dot(dir) * joint.damping;
                    let total_force = spring_force + damping_force;
                    if let Some(v_a) = vels.get_mut(joint.entity_a) {
                        v_a.linear += total_force * inv_mass_a * dt;
                    }
                    if let Some(v_b) = vels.get_mut(joint.entity_b) {
                        v_b.linear -= total_force * inv_mass_b * dt;
                    }
                }
                JointKind::Hinge {
                    axis,
                    min_angle,
                    max_angle,
                } => {
                    // 1. Pozisyon kısıtlayıcısı (BallSocket gibi çalışır)
                    let diff = pos_b - pos_a;
                    let bias = diff * (beta / dt) * joint.stiffness;

                    let mut va_lin = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.linear);
                    let mut va_ang = vels.get(joint.entity_a).map_or(Vec3::ZERO, |v| v.angular);
                    let mut vb_lin = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.linear);
                    let mut vb_ang = vels.get(joint.entity_b).map_or(Vec3::ZERO, |v| v.angular);

                    let r_a = rot_a.mul_vec3(joint.anchor_a);
                    let r_b = rot_b.mul_vec3(joint.anchor_b);

                    // a — çeviri kısıtlayıcısı (3 eksen)
                    for &constraint_axis in &[Vec3::X, Vec3::Y, Vec3::Z] {
                        let vel_a_anchor = va_lin + va_ang.cross(r_a);
                        let vel_b_anchor = vb_lin + vb_ang.cross(r_b);
                        let rel_vel_n = (vel_b_anchor - vel_a_anchor).dot(constraint_axis);
                        let bias_n = bias.dot(constraint_axis);
                        let ra_x_n = r_a.cross(constraint_axis);
                        let rb_x_n = r_b.cross(constraint_axis);
                        let ia = Vec3::new(
                            ra_x_n.x * inv_inertia_a.x,
                            ra_x_n.y * inv_inertia_a.y,
                            ra_x_n.z * inv_inertia_a.z,
                        );
                        let ib = Vec3::new(
                            rb_x_n.x * inv_inertia_b.x,
                            rb_x_n.y * inv_inertia_b.y,
                            rb_x_n.z * inv_inertia_b.z,
                        );
                        let eff = inv_mass_a + inv_mass_b + ia.dot(ra_x_n) + ib.dot(rb_x_n);
                        if eff > 1e-6 {
                            let lambda = -(rel_vel_n + bias_n) / eff;
                            let impulse = constraint_axis * lambda;
                            va_lin -= impulse * inv_mass_a;
                            va_ang -= Vec3::new(
                                (r_a.cross(impulse)).x * inv_inertia_a.x,
                                (r_a.cross(impulse)).y * inv_inertia_a.y,
                                (r_a.cross(impulse)).z * inv_inertia_a.z,
                            );
                            vb_lin += impulse * inv_mass_b;
                            vb_ang += Vec3::new(
                                (r_b.cross(impulse)).x * inv_inertia_b.x,
                                (r_b.cross(impulse)).y * inv_inertia_b.y,
                                (r_b.cross(impulse)).z * inv_inertia_b.z,
                            );
                        }
                    }

                    // b — Dönüş kısıtlayıcısı: menteseye dik 2 eksende açısal hızı sıfırla
                    // axis'e dik iki vektörü bul
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
                        let rel_ang_n = (vb_ang - va_ang).dot(perp);
                        // Etkin atalet tahmini: her eksenin ters atalet toplamı / 2
                        let eff_ang = (inv_inertia_a.x
                            + inv_inertia_a.y
                            + inv_inertia_a.z
                            + inv_inertia_b.x
                            + inv_inertia_b.y
                            + inv_inertia_b.z)
                            / 6.0;
                        if eff_ang > 1e-8 {
                            let lambda_ang = -rel_ang_n / eff_ang;
                            let ang_impulse = perp * lambda_ang;
                            va_ang -= Vec3::new(
                                ang_impulse.x * inv_inertia_a.x,
                                ang_impulse.y * inv_inertia_a.y,
                                ang_impulse.z * inv_inertia_a.z,
                            );
                            vb_ang += Vec3::new(
                                ang_impulse.x * inv_inertia_b.x,
                                ang_impulse.y * inv_inertia_b.y,
                                ang_impulse.z * inv_inertia_b.z,
                            );
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

                    // c — Açı limiti
                    if *min_angle > f32::NEG_INFINITY || *max_angle < f32::INFINITY {
                        // `axis` entity_a'nın local-space'inde saklanır.
                        // Anlık world-space ekseni her frame yeniden hesaplanmalıdır.
                        let hinge_world = rot_a.mul_vec3(*axis);

                        // rel_rot: B'nin A'ya göre göreli dönüşü (A-local uzayında)
                        let rel_rot = rot_a.conjugate() * rot_b;
                        let (rel_axis_local, angle) = rel_rot.to_axis_angle();

                        // rel_axis_local, A'nın yerel uzayında — world-space'e çevir
                        let rel_axis_world = rot_a.mul_vec3(rel_axis_local);

                        // Dönüş yönü, world-space hinge_world eksenine göre işaretlenir
                        let signed_angle = if rel_axis_world.dot(hinge_world) >= 0.0 {
                            angle
                        } else {
                            -angle
                        };

                        if signed_angle < *min_angle {
                            let correction = hinge_world * (*min_angle - signed_angle) * (beta / dt);
                            if let Some(v_b) = vels.get_mut(joint.entity_b) {
                                v_b.angular += correction;
                            }
                        } else if signed_angle > *max_angle {
                            let correction = hinge_world * (signed_angle - *max_angle) * (beta / dt);
                            if let Some(v_b) = vels.get_mut(joint.entity_b) {
                                v_b.angular -= correction;
                            }
                        }
                    }
                }
            }
        }
    }

    // === POSITION PROJECTION PASS ===
    // Velocity solver sonrası, kalan konumsal hatayı (drift) doğrudan Transform'a uygula.
    // Bu, Baumgarte stabilization'ın yakınsayamadığı büyük hataları düzeltir.
    drop(vels); // velocity borrow'unu bırak
    drop(rbs); // rbs borrow'unu bırak
    drop(transforms); // transforms borrow'unu bırak

    let mut transforms = match world.borrow_mut::<crate::components::Transform>() {
        Some(t) => t,
        None => return,
    };
    let rbs = match world.borrow::<crate::components::RigidBody>() {
        Some(r) => r,
        None => return,
    };

    const POSITION_CORRECTION_FACTOR: f32 = 0.8; // %80 düzeltme (overshooting koruması)
    const POSITION_SLOP: f32 = 0.001; // 1mm'den küçük hataları yoksay

    for (_, joint) in &joint_world.joints {
        let ta = match transforms.get(joint.entity_a) {
            Some(ta) => *ta,
            None => continue,
        };
        let tb = match transforms.get(joint.entity_b) {
            Some(tb) => *tb,
            None => continue,
        };

        let pos_a = ta.position + ta.rotation.mul_vec3(joint.anchor_a);
        let pos_b = tb.position + tb.rotation.mul_vec3(joint.anchor_b);

        let (inv_mass_a, _) = rbs.get(joint.entity_a).map_or((0.0, Vec3::ZERO), |rb| {
            if rb.mass > 0.0 {
                (1.0 / rb.mass, rb.inverse_inertia)
            } else {
                (0.0, Vec3::ZERO)
            }
        });
        let (inv_mass_b, _) = rbs.get(joint.entity_b).map_or((0.0, Vec3::ZERO), |rb| {
            if rb.mass > 0.0 {
                (1.0 / rb.mass, rb.inverse_inertia)
            } else {
                (0.0, Vec3::ZERO)
            }
        });
        let total_inv_mass = inv_mass_a + inv_mass_b;
        if total_inv_mass == 0.0 {
            continue;
        }

        match &joint.kind {
            JointKind::BallSocket | JointKind::Fixed { .. } | JointKind::Hinge { .. } => {
                let error = pos_b - pos_a;
                let error_len = error.length();
                if error_len > POSITION_SLOP {
                    let correction = error * (POSITION_CORRECTION_FACTOR / total_inv_mass);
                    if let Some(t_a) = transforms.get_mut(joint.entity_a) {
                        t_a.position += correction * inv_mass_a;
                    }
                    if let Some(t_b) = transforms.get_mut(joint.entity_b) {
                        t_b.position -= correction * inv_mass_b;
                    }
                }
            }
            JointKind::Distance { length } => {
                let diff = pos_b - pos_a;
                let current_len = diff.length();
                if current_len < 0.0001 {
                    continue;
                }
                let dir = diff / current_len;
                let error = current_len - length;
                if error.abs() > POSITION_SLOP {
                    let correction = dir * (error * POSITION_CORRECTION_FACTOR / total_inv_mass);
                    if let Some(t_a) = transforms.get_mut(joint.entity_a) {
                        t_a.position += correction * inv_mass_a;
                    }
                    if let Some(t_b) = transforms.get_mut(joint.entity_b) {
                        t_b.position -= correction * inv_mass_b;
                    }
                }
            }
            JointKind::Spring { .. } => {
                // Spring'ler esnek bağlantı — pozisyon düzeltmesi yapılmaz
            }
        }
    }
}
