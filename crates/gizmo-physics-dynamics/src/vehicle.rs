use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_core::raycast::{Ray, Raycast, RaycastHit};
use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};

// ============================================================
// PACEJKA MF — Kombine Slip Modeli (MF 5.2 benzeri)
// F_x = Fx_pure(σx) * Gx(σy)   — uzunlamasına weighting
// F_y = Fy_pure(σy) * Gy(σx)   — yanal weighting
// ============================================================

#[derive(Clone, Debug)]
pub struct PacejkaParams {
    pub b: f32, // Stiffness factor
    pub c: f32, // Shape factor
    pub d: f32, // Peak factor (normal load ile ölçeklenir)
    pub e: f32, // Curvature factor
}

impl Default for PacejkaParams {
    fn default() -> Self {
        Self {
            b: 10.0,
            c: 1.9,
            d: 1.0,
            e: 0.97,
        }
    }
}

impl PacejkaParams {
    /// Tek eksen için saf Pacejka değeri ([-∞,+∞] slip → kuvvet)
    pub fn calculate_force(&self, slip: f32, normal_load: f32) -> f32 {
        let bx = self.b * slip;
        let d = self.d * normal_load;
        let inner = self.c * (bx - self.e * (bx - bx.atan())).atan();
        d * inner.sin()
    }

    /// Kombine slip weighting fonksiyonu (Lorentzian falloff)
    /// σ_other = dikey yöndeki normalize kayma miktarı
    fn weighting_lorentzian(&self, sigma_other: f32) -> f32 {
        let k = self.b * sigma_other;
        // Lorentzian falloff, KİNETİK sürtünme tabanıyla (0.35).
        // Neden taban: `sigma_other` (özellikle boyuna slip_ratio) SINIRSIZ; patinajda
        // slip_ratio ~10 olabilir → k=100 → çıplak Lorentzian ≈0.01, yani DÖNEN tekerlek
        // yanal tutuşunu ~%99 kaybediyordu. Bu, düz tam gazda arka aksın yanal
        // restoring kuvvetini sıfırlayıp en ufak yük asimetrisinde aracı KENDİLİĞİNDEN
        // spin ettiriyordu (ve sürtünme çemberinin boş bütçesini hiç kullanmadan). Tam
        // kayan lastik gerçekte kinetik sürtünmeyle yanal kapasitesinin bir kısmını
        // korur; tabanı bu artık grip'i modelliyor. Kombine limiti hâlâ aşağıdaki
        // sürtünme çemberi kırpması belirler (çift-bastırma değil). sigma_other=0 →
        // taban bağlanmaz (weighting=1) → saf-eksen kuvvet testleri korunur.
        (1.0 / (1.0 + k * k).sqrt()).max(0.35)
    }
}

/// Geriye uyum için PacejkaLat alias
pub type PacejkaLat = PacejkaParams;

/// Kombine Pacejka: uzunlamasına ve yanal kuvvetleri birlikte hesapla
/// Sürtünme çemberi dahilinde tutulur.
pub fn pacejka_combined(
    long: &PacejkaParams,
    lat: &PacejkaParams,
    slip_ratio: f32, // longitudinal (σx)
    slip_angle: f32, // lateral (radyan, σy)
    normal_load: f32,
) -> (f32, f32) {
    let fx_pure = long.calculate_force(slip_ratio, normal_load);
    let fy_pure = lat.calculate_force(slip_angle, normal_load);

    // Kombine weighting: her eksen diğerini kısmen bastırır
    let gx = long.weighting_lorentzian(slip_angle);
    let gy = lat.weighting_lorentzian(slip_ratio);

    let fx = fx_pure * gx;
    let fy = fy_pure * gy;

    // Sürtünme çemberi: μ * Fz sınırı
    let mu_peak = long.d.max(lat.d) * 1.2;
    let limit = normal_load * mu_peak;
    let mag = (fx * fx + fy * fy).sqrt();
    if mag > limit && mag > 0.0 {
        let scale = limit / mag;
        (fx * scale, fy * scale)
    } else {
        (fx, fy)
    }
}

// ============================================================
// WHEEL & VEHICLE STRUCTS
// ============================================================

#[derive(Clone, Debug, PartialEq)]
pub enum Axle {
    Front,
    Rear,
}

#[derive(Clone, Debug)]
pub struct Wheel {
    pub attachment_local_pos: Vec3,
    pub direction_local: Vec3,
    pub axle_type: Axle,
    pub is_left: bool,

    pub radius: f32,
    pub suspension_rest_length: f32,
    pub suspension_max_travel: f32,
    pub suspension_stiffness: f32,
    pub suspension_damping: f32,

    pub pacejka_long: PacejkaParams,
    pub pacejka_lat: PacejkaLat,
    pub wheel_mass: f32,

    // Inputs (set by VehicleController each frame)
    pub steering_angle: f32,
    pub drive_torque: f32,
    pub brake_torque: f32,

    // State
    pub is_grounded: bool,
    pub ground_hit: Option<RaycastHit>,
    pub suspension_length: f32,
    pub rotation_angle: f32,
    pub angular_velocity: f32,
    pub suspension_force: f32,
}

impl Default for Wheel {
    fn default() -> Self {
        Self {
            attachment_local_pos: Vec3::ZERO,
            direction_local: Vec3::new(0.0, -1.0, 0.0),
            axle_type: Axle::Front,
            is_left: true,
            radius: 0.35,
            suspension_rest_length: 0.5,
            suspension_max_travel: 0.2,
            suspension_stiffness: 25000.0,
            suspension_damping: 3000.0,
            pacejka_long: PacejkaParams::default(),
            pacejka_lat: PacejkaLat::default(),
            wheel_mass: 20.0,
            steering_angle: 0.0,
            drive_torque: 0.0,
            brake_torque: 0.0,
            is_grounded: false,
            ground_hit: None,
            suspension_length: 0.5,
            rotation_angle: 0.0,
            angular_velocity: 0.0,
            suspension_force: 0.0,
        }
    }
}

/// Eski uyum için `pacejka` getter
impl Wheel {
    #[inline]
    pub fn pacejka(&self) -> &PacejkaParams {
        &self.pacejka_long
    }
}

/// Aerodinamik paket
#[derive(Clone, Debug)]
pub struct AeroPackage {
    pub drag_coefficient: f32,     // Cd
    pub lift_coefficient: f32,     // Cl (negatif = downforce)
    pub frontal_area: f32,         // m²
    pub center_of_pressure: Vec3,  // CoM'dan offset (yerel)
    pub ground_effect_height: f32, // Bu yüksekliğin altında zemin etkisi devreye girer
    pub ground_effect_multiplier: f32,
}

impl Default for AeroPackage {
    fn default() -> Self {
        Self {
            drag_coefficient: 0.32,
            lift_coefficient: -0.8, // downforce
            frontal_area: 2.2,      // m²
            center_of_pressure: Vec3::new(0.0, 0.3, 0.2),
            ground_effect_height: 0.15,
            ground_effect_multiplier: 1.8,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VehicleTuning {
    pub idle_rpm: f32,
    pub max_rpm: f32,
    /// [0]=Geri, [1]=Nötr, [2..]=İleri vitesler
    pub gear_ratios: Vec<f32>,
    pub final_drive_ratio: f32,
    /// Otomatik vites: upshift RPM eşiği
    pub upshift_rpm: f32,
    /// Otomatik vites: downshift RPM eşiği
    pub downshift_rpm: f32,
    pub wheelbase: f32,
    pub track_width: f32,
    pub anti_roll_stiffness: f32,
    pub max_engine_torque: f32,
    pub max_brake_torque: f32,
    pub aero: AeroPackage,
}

impl Default for VehicleTuning {
    fn default() -> Self {
        Self {
            idle_rpm: 800.0,
            max_rpm: 7000.0,
            gear_ratios: vec![-2.5, 0.0, 3.0, 2.0, 1.4, 1.0, 0.75],
            final_drive_ratio: 3.73,
            upshift_rpm: 6200.0,
            downshift_rpm: 2200.0,
            wheelbase: 2.8,
            track_width: 1.6,
            anti_roll_stiffness: 3000.0,
            max_engine_torque: 350.0,
            max_brake_torque: 1500.0,
            aero: AeroPackage::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct VehicleController {
    pub wheels: Vec<Wheel>,
    pub tuning: VehicleTuning,

    pub throttle_input: f32, // 0..1
    pub brake_input: f32,    // 0..1
    pub steering_input: f32, // -1..1
    pub reverse_input: bool,
    pub auto_shift: bool, // Otomatik vites etkin mi?

    pub current_gear: usize, // gear_ratios index
    pub max_steering_angle: f32,
    pub shift_cooldown: f32, // Vites değişimi sonrası bekleme süresi (s)

    pub engine_rpm: f32,
    pub current_speed_kmh: f32,
    pub engine_angular_vel: f32, // rad/s — şanzıman simülasyonu için
    pub flywheel_inertia: f32,   // kg·m²
}

impl gizmo_core::component::Component for VehicleController {}

impl Default for VehicleController {
    fn default() -> Self {
        Self {
            wheels: Vec::new(),
            tuning: VehicleTuning::default(),
            throttle_input: 0.0,
            brake_input: 0.0,
            steering_input: 0.0,
            reverse_input: false,
            auto_shift: true,
            current_gear: 2,
            max_steering_angle: 0.52,
            shift_cooldown: 0.0,
            engine_rpm: 800.0,
            current_speed_kmh: 0.0,
            engine_angular_vel: 800.0 / 9.549,
            flywheel_inertia: 0.25, // kg·m²
        }
    }
}

impl VehicleController {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }

    /// Motor tork eğrisi — parametrik çan eğrisi
    pub fn engine_torque(&self) -> f32 {
        let t = &self.tuning;
        let ratio = (self.engine_rpm - t.idle_rpm).max(0.0) / (t.max_rpm - t.idle_rpm).max(1.0);
        let curve = (1.0 - (ratio - 0.4).powi(2) * 2.5).clamp(0.05, 1.0);
        t.max_engine_torque * curve * self.throttle_input.abs()
    }

    pub fn set_reverse(&mut self, on: bool) {
        self.current_gear = if on {
            0
        } else if self.current_gear == 0 {
            2
        } else {
            self.current_gear
        };
        self.reverse_input = on;
    }

    /// Otomatik vites — RPM eşiğine göre upshift/downshift
    pub fn auto_shift_tick(&mut self, dt: f32) {
        if !self.auto_shift || self.reverse_input {
            return;
        }
        self.shift_cooldown = (self.shift_cooldown - dt).max(0.0);
        if self.shift_cooldown > 0.0 {
            return;
        }
        // Guard: kullanıcı gear_ratios'u boş bırakabilir; `len() - 1` usize
        // underflow panic'ini önle. Boşsa vites değişimi anlamsız → erken dön.
        if self.tuning.gear_ratios.is_empty() {
            return;
        }
        let max_gear = self.tuning.gear_ratios.len().saturating_sub(1);
        if self.engine_rpm > self.tuning.upshift_rpm && self.current_gear < max_gear {
            self.current_gear += 1;
            self.shift_cooldown = 0.4; // 400ms bekleme
        } else if self.engine_rpm < self.tuning.downshift_rpm && self.current_gear > 2 {
            self.current_gear -= 1;
            self.shift_cooldown = 0.3;
        }
    }
}

// ============================================================
// ARAÇ GÜNCELLEME FONKSİYONU
// ============================================================

/// Per-wheel Ackermann steering angle.
///
/// `turn_radius = wheelbase / tan(steer_angle)` is *signed*: with `+Y` up, `-Z`
/// forward and `+X` right, a positive `steer_angle` steers the wheels left, giving
/// a positive `turn_radius` and putting the turn centre on the car's left, so the
/// **left** wheel is the inner one. The inner wheel traces the smaller radius
/// (`turn_radius - track/2`) and therefore steers *more*; the outer wheel uses
/// `turn_radius + track/2`. Beyond a near-straight threshold the nominal angle is
/// returned unchanged.
#[inline]
fn ackermann_steering_angle(
    steer_angle: f32,
    turn_radius: f32,
    wheelbase: f32,
    track_width: f32,
    is_left: bool,
) -> f32 {
    if turn_radius.abs() < 1e4 {
        // Inner wheel (left on a left turn) subtracts half-track → tighter angle.
        let sign = if is_left { -1.0 } else { 1.0 };
        (wheelbase / (turn_radius + sign * track_width * 0.5)).atan()
    } else {
        steer_angle
    }
}

/// Advances a vehicle by one fixed step.
///
/// Runs the drivetrain, aerodynamics, Ackermann steering, suspension raycasts and
/// combined-slip tire forces, mutating the rigid body and velocity in place.
/// `all_colliders` must contain every scene collider (static and dynamic); the
/// entry matching `vehicle_entity` is ignored so the vehicle does not raycast
/// against itself.
/// Anti-roll bar (sway bar) corrective vertical force for one wheel on an axle.
///
/// `diff = travel_left - travel_right`, where `travel` is suspension
/// *compression*. A positive `diff` means the left corner is more compressed
/// (lower) than the right — i.e. the body is rolling toward the left. The
/// returned value is *added* into `suspension_force`, which pushes the chassis
/// **up** at that corner. To RESIST the roll, the bar must add up-force on the
/// lower (more-compressed) corner and remove it from the higher corner, so the
/// left wheel gets `+diff` and the right `-diff`.
///
/// The previous code had both signs flipped (`-diff` on the left, `+diff` on the
/// right), which *amplified* roll: it pushed the already-high inner corner up and
/// the already-low outer corner down, so raising `anti_roll_stiffness` — which a
/// user does to reduce roll — made cornering roll and stability worse.
#[inline]
fn anti_roll_force(is_left: bool, diff: f32, stiffness: f32) -> f32 {
    let signed = if is_left { diff } else { -diff };
    signed * stiffness
}

/// Ground-effect downforce çarpanı: şasi taban `clearance`'ı (gövde tabanının yere
/// dikey boşluğu, m) azaldıkça `1.0`'dan `multiplier`'a YUMUŞAK rampa yapar.
/// `clearance ≥ height` → 1.0 (etki yok); `clearance = 0` → `multiplier`; arası lineer.
/// Negatif clearance (gövde yerin içinde) `multiplier`'a kırpılır.
fn ground_effect_factor(clearance: f32, height: f32, multiplier: f32) -> f32 {
    let t = (1.0 - clearance.max(0.0) / height.max(1e-3)).clamp(0.0, 1.0);
    1.0 + (multiplier - 1.0) * t
}

pub fn update_vehicle(
    vehicle_entity: BodyHandle,
    vehicle: &mut VehicleController,
    vehicle_rb: &mut RigidBody,
    vehicle_transform: &Transform,
    vehicle_vel: &mut Velocity,
    all_colliders: &[(BodyHandle, Transform, Collider)],
    dt: f32,
) {
    if vehicle_rb.is_static() {
        return;
    }

    // Yerel eksenler
    let up = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(0.0, 1.0, 0.0));
    let forward = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(0.0, 0.0, -1.0));
    let right = vehicle_transform
        .rotation
        .mul_vec3(Vec3::new(1.0, 0.0, 0.0));

    let v_com = vehicle_vel.linear;
    let forward_speed = v_com.dot(forward);
    vehicle.current_speed_kmh = forward_speed * 3.6;

    // --------------------------------------------------------
    // 1. GÜÇ AKTARMA ORGANı
    // --------------------------------------------------------
    let gear_ratio = vehicle
        .tuning
        .gear_ratios
        .get(vehicle.current_gear)
        .copied()
        .unwrap_or(0.0);
    let total_ratio = gear_ratio * vehicle.tuning.final_drive_ratio;

    // RPM ← arka tekerlek angular_velocity ortalamasından
    let mut avg_rear_ω = 0.0f32;
    let mut rear_count = 0.0f32;
    for w in &vehicle.wheels {
        if w.axle_type == Axle::Rear {
            avg_rear_ω += w.angular_velocity;
            rear_count += 1.0;
        }
    }
    if rear_count > 0.0 {
        avg_rear_ω /= rear_count;
    }

    let wheel_rpm = avg_rear_ω.abs() * 9.549; // rad/s → rpm
    vehicle.engine_rpm =
        (wheel_rpm * total_ratio.abs()).clamp(vehicle.tuning.idle_rpm, vehicle.tuning.max_rpm);

    let engine_torque = vehicle.engine_torque();
    // Geri viteste tork yönü ters
    let torque_sign = if total_ratio < 0.0 { -1.0 } else { 1.0 };
    let drive_torque_total = engine_torque * total_ratio.abs() * torque_sign;

    // --------------------------------------------------------
    // 1.5 Otomatik vites
    // --------------------------------------------------------
    vehicle.auto_shift_tick(dt);

    // --------------------------------------------------------
    // 2. AERODİNAMİK (fiziksel — ½ρCdAv²)
    // --------------------------------------------------------
    const AIR_DENSITY: f32 = 1.225; // kg/m³
    let spd = v_com.length();
    let spd_sq = spd * spd;
    let a = &vehicle.tuning.aero;
    let q = 0.5 * AIR_DENSITY * spd_sq; // dinamik basınç

    // Zemin etkisi: araç gövdesi yere yaklaştıkça downforce artar. Referans = ŞASİ TABAN
    // clearance'ı: aracın KENDİ collider'ının dünya-AABB tabanı (min.y) ile tekerlek temas
    // noktası (ground_hit.point.y) arasındaki dikey boşluk. (Eskiden `hit.distance-0.5` =
    // süspansiyon-bağlantı-to-zemin ≈0.85m kullanılıyordu; bu hep `ground_effect_height`
    // üstünde kaldığından ge_factor hep 1.0 = etki ÖLÜYDÜ.) Aracın collider'ı `all_colliders`
    // içinde (raycast'ten self-exclude edilse de erişilebilir). Normal sürüş yüksekliğinde
    // clearance > ge_height → ge_factor=1.0 (davranış AYNEN korunur); yalnız araç gerçekten
    // alçaldığında (yük/downforce) yumuşak rampayla artar → doğal, disruption-suz.
    let ground_contact_y = vehicle
        .wheels
        .iter()
        .filter_map(|w| w.ground_hit.as_ref().map(|hit| hit.point.y))
        .fold(f32::MAX, f32::min);
    let chassis_floor_y = all_colliders
        .iter()
        .find(|(h, _, _)| *h == vehicle_entity)
        .map(|(_, tr, col)| col.compute_aabb(tr.position, tr.rotation).min.y);
    let ge_factor = match chassis_floor_y {
        Some(floor_y) if ground_contact_y.is_finite() => ground_effect_factor(
            floor_y - ground_contact_y,
            a.ground_effect_height,
            a.ground_effect_multiplier,
        ),
        _ => 1.0,
    };

    let drag_dir = if spd > 0.1 { -v_com / spd } else { Vec3::ZERO };
    let drag_force = drag_dir * (a.drag_coefficient * a.frontal_area * q);
    let lift_force = up * (a.lift_coefficient * a.frontal_area * q * ge_factor);

    // Aero kuvvetini basınç merkezinden uygula (tork üretir)
    let cop_world =
        vehicle_transform.position + vehicle_transform.rotation.mul_vec3(a.center_of_pressure);
    let com = vehicle_transform.position
        + vehicle_transform
            .rotation
            .mul_vec3(vehicle_rb.center_of_mass);
    apply_force_at_point(
        vehicle_rb,
        vehicle_vel,
        com,
        vehicle_transform.rotation,
        drag_force + lift_force,
        cop_world,
        dt,
    );

    // --------------------------------------------------------
    // 3. ACKERMANN DİREKSİYON
    // --------------------------------------------------------
    let steer_angle = vehicle.steering_input * vehicle.max_steering_angle;
    let turn_radius = if steer_angle.abs() > 0.01 {
        vehicle.tuning.wheelbase / steer_angle.tan()
    } else {
        f32::MAX
    };

    // --------------------------------------------------------
    // 4. TEKERLEK DÖNGÜSÜ — 1. geçiş: Raycast + Süspansiyon setup
    // --------------------------------------------------------
    let rear_count_f = rear_count.max(1.0);

    for wheel in &mut vehicle.wheels {
        let attach_world = vehicle_transform.position
            + vehicle_transform
                .rotation
                .mul_vec3(wheel.attachment_local_pos);
        let ray_dir = vehicle_transform
            .rotation
            .mul_vec3(wheel.direction_local)
            .normalize();

        // Ray origin'i attach_world'den biraz geriye al (yukarıya) ki araç yere tam oturduğunda
        // raycast origin'i yerin içinde kalıp çarpışmayı kaçırmasın!
        let ray_origin_offset = 0.5;
        let ray_start = attach_world - ray_dir * ray_origin_offset;
        let ray_max = wheel.suspension_rest_length
            + wheel.radius
            + wheel.suspension_max_travel
            + ray_origin_offset;
        let ray = Ray::new(ray_start, ray_dir);

        // Raycast
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_dist = ray_max;

        for (other_ent, other_trans, other_col) in all_colliders {
            if *other_ent == vehicle_entity || other_col.is_trigger {
                continue;
            }
            let aabb = other_col.compute_aabb(other_trans.position, other_trans.rotation);
            if Raycast::ray_aabb(&ray, &aabb).is_none() {
                continue;
            }
            if let Some((dist, normal)) = Raycast::ray_shape(&ray, &other_col.shape, other_trans) {
                if dist < closest_dist {
                    closest_dist = dist;
                    closest_hit = Some(RaycastHit {
                        entity: *other_ent,
                        point: ray.point_at(dist),
                        normal,
                        distance: dist,
                    });
                }
            }
        }

        if let Some(hit) = closest_hit {
            wheel.is_grounded = true;
            wheel.ground_hit = Some(hit);

            // Gerçek mesafe için eklediğimiz offseti çıkarıyoruz
            let actual_dist = closest_dist - ray_origin_offset;

            // Süspansiyon sıkışması: yay uzunluğu = çarpma mesafesi - tekerlek yarıçapı
            let raw_len = (actual_dist - wheel.radius).clamp(
                wheel.suspension_rest_length - wheel.suspension_max_travel,
                wheel.suspension_rest_length + wheel.suspension_max_travel,
            );
            wheel.suspension_length = raw_len;
        } else {
            wheel.is_grounded = false;
            wheel.ground_hit = None;
            wheel.suspension_length = wheel.suspension_rest_length;
            wheel.suspension_force = 0.0;
        }

        // Ackermann açısı (ön tekerlek)
        if wheel.axle_type == Axle::Front {
            wheel.steering_angle = ackermann_steering_angle(
                steer_angle,
                turn_radius,
                vehicle.tuning.wheelbase,
                vehicle.tuning.track_width,
                wheel.is_left,
            );
        }

        // Tork dağıtımı (RWD)
        wheel.drive_torque = if wheel.axle_type == Axle::Rear {
            drive_torque_total / rear_count_f
        } else {
            0.0
        };

        // Fren dağıtımı (%60 ön / %40 arka)
        let bias = if wheel.axle_type == Axle::Front {
            0.6
        } else {
            0.4
        };
        wheel.brake_torque = vehicle.brake_input * vehicle.tuning.max_brake_torque * bias;
    }

    // --------------------------------------------------------
    // 5. Anti-roll bar farkları
    // --------------------------------------------------------
    let (mut fl, mut fr, mut rl, mut rr) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for w in &vehicle.wheels {
        let travel = w.suspension_rest_length - w.suspension_length;
        match (&w.axle_type, w.is_left) {
            (Axle::Front, true) => fl = travel,
            (Axle::Front, false) => fr = travel,
            (Axle::Rear, true) => rl = travel,
            (Axle::Rear, false) => rr = travel,
        }
    }
    let front_diff = fl - fr;
    let rear_diff = rl - rr;

    // --------------------------------------------------------
    // 6. TEKERLEK DÖNGÜSÜ — 2. geçiş: Kuvvetler + Tekerlek integrasyon
    // --------------------------------------------------------

    // Motor/volan ataleti — sürülen tekerlekte gear² ile yansır (aşağıda). Döngü
    // içinde `vehicle`'a erişilemez (wheels mutable borrow), o yüzden burada yakala.
    let flywheel_inertia = vehicle.flywheel_inertia;

    for wheel in &mut vehicle.wheels {
        let attach_world = vehicle_transform.position
            + vehicle_transform
                .rotation
                .mul_vec3(wheel.attachment_local_pos);
        let ray_dir = vehicle_transform
            .rotation
            .mul_vec3(wheel.direction_local)
            .normalize();

        // --- YAY KUVVET ENTEGRASYONu (her zaman, grounded veya değil) ---
        // Tekerlek ataletini (I = 0.5 m r²) hesapla.
        // Guard: wheel_mass=0 veya radius=0 ise inertia=0 → net_torque/wheel_inertia
        // = inf/NaN ve tüm simülasyon sessizce bozulur. Epsilon-clamp ile tek
        // noktada koruyarak aşağıdaki tüm bölmeleri (690/693/710/713) güvene al.
        let wheel_inertia = (0.5 * wheel.wheel_mass * wheel.radius.powi(2)).max(1e-6);

        // Sürülen (Rear) tekerlek motora şanzımanla kenetli → motor+volan ataleti
        // gear² ile YANSIR (I_eff = I_wheel + I_flywheel·ratio²). Bunu katmadan tekerlek
        // yalnız kendi minik ataletiyle (~0.8) anında redline'a fırlıyordu = kalkışta
        // patinaj + tüm vitesleri geçme. Yansıyan atalet (düşük viteste ~30 kg·m²) motoru
        // gerçekçi biçimde kademeli döndürür; yüksek viteste ratio küçülür → daha çevik.
        let effective_inertia = if wheel.axle_type == Axle::Rear {
            wheel_inertia + flywheel_inertia * total_ratio * total_ratio
        } else {
            wheel_inertia
        };

        if wheel.is_grounded {
            if let Some(hit) = wheel.ground_hit.as_ref() {
                // 6.1 Gelişmiş Süspansiyon: baskı/geri dönüş ayrı damper
                let point_rel = attach_world - vehicle_transform.position;
                let point_vel = vehicle_vel.linear + vehicle_vel.angular.cross(point_rel);
                let susp_vel = point_vel.dot(ray_dir); // pozitif = yay sıkışıyor
                let compression = wheel.suspension_rest_length - wheel.suspension_length;

                let spring_force = wheel.suspension_stiffness * compression;

                // Baskı: damping_compression, geri dönüş: damping_rebound (genelde 2-3x baskı)
                let damper_coeff = if susp_vel > 0.0 {
                    wheel.suspension_damping // baskı katsayısı
                } else {
                    wheel.suspension_damping * 2.5 // rebound (daha sert)
                };
                let mut damper_force = damper_coeff * susp_vel;
                // Rebound damper'ı (2.5×) statik yay yükünü İPTAL etmesin. Açık (explicit)
                // entegratörde sert rebound damping + aşağıdaki `.max(0.0)` kırpması +
                // baskı/rebound anahtarlaması, DURAĞAN araçta küçük bir dikey limit-cycle
                // doğuruyordu: yay pozitif sıkışmada olsa bile (compression>0) rebound
                // fazında damper yayı tam götürüp `suspension_force`'u 0'a düşürüyordu.
                // Normal yük 0 → Pacejka lastik kuvveti (∝ Fz) 0 → sürülen tekerlek grip'siz
                // → tam gazda yerinde patinaj (rpm redline'a fırlar, araç KALKMAZ). Damper'ı
                // yay kuvvetinin yarısıyla tabanlayarak tekerleğin zemine basılı kalmasını
                // (Fz>0) garanti et; baskı fazını (pozitif) etkilemez, yalnız rebound'u sınırlar.
                damper_force = damper_force.max(-spring_force * 0.5);

                // Bump stop: max seyahat sonunda sert non-linear yay
                let bump_stop_travel = wheel.suspension_max_travel * 0.1;
                let bump_excess = compression - (wheel.suspension_max_travel - bump_stop_travel);
                let bump_stop_force = if bump_excess > 0.0 {
                    bump_excess * wheel.suspension_stiffness * 8.0
                } else {
                    0.0
                };

                // Anti-roll bar — adds up-force to the more-compressed corner to
                // resist roll (see `anti_roll_force`; the sign was previously
                // inverted, amplifying roll).
                let axle_diff = match wheel.axle_type {
                    Axle::Front => front_diff,
                    Axle::Rear => rear_diff,
                };
                let arb_force =
                    anti_roll_force(wheel.is_left, axle_diff, vehicle.tuning.anti_roll_stiffness);

                wheel.suspension_force =
                    (spring_force + damper_force + bump_stop_force + arb_force).max(0.0);
                let susp_impulse = (-ray_dir) * wheel.suspension_force;
                apply_force_at_point(
                    vehicle_rb,
                    vehicle_vel,
                    com,
                    vehicle_transform.rotation,
                    susp_impulse,
                    attach_world,
                    dt,
                );

                // 6.2 Pacejka Kuvvetleri
                let steering_rot = Quat::from_axis_angle(up, wheel.steering_angle);
                let wheel_forward = steering_rot.mul_vec3(forward).normalize();
                let wheel_right = steering_rot.mul_vec3(right).normalize();

                let v_long = point_vel.dot(wheel_forward);
                let v_lat = point_vel.dot(wheel_right);

                // Denom: düşük hızda sıfır bölünmeyi önle
                let ref_vel = v_long.abs().max(0.5);

                // Longitudinal slip ratio
                let wheel_linear_vel = wheel.angular_velocity * wheel.radius;
                let slip_ratio = (wheel_linear_vel - v_long) / ref_vel;

                // Lateral slip angle [rad]
                let slip_angle = -(v_lat / ref_vel).atan();

                let normal_load = wheel.suspension_force;

                // Kombine Pacejka MF — sürtünme çemberi dahilinde
                let (final_long, final_lat) = pacejka_combined(
                    &wheel.pacejka_long,
                    &wheel.pacejka_lat,
                    slip_ratio,
                    slip_angle,
                    normal_load,
                );

                // Lastik kuvvetini temas noktasından uygula
                let tire_force = wheel_forward * final_long + wheel_right * final_lat;
                let contact_pt = hit.point;
                apply_force_at_point(
                    vehicle_rb,
                    vehicle_vel,
                    com,
                    vehicle_transform.rotation,
                    tire_force,
                    contact_pt,
                    dt,
                );

                // 6.3 Tekerlek angular_velocity entegrasyonu (Semi-implicit Euler)
                // Reaksiyon torku lastikten gelen geri tepme
                let reaction_torque = final_long * wheel.radius;

                // Fren torku: tekerlek dönüşünün tersine
                let brake_dir = if wheel.angular_velocity.abs() > 0.01 {
                    -wheel.angular_velocity.signum()
                } else {
                    0.0
                };
                let effective_brake = wheel.brake_torque * brake_dir;

                // Net tork
                let net_torque = wheel.drive_torque + effective_brake - reaction_torque;

                // DOĞRUSAL-İMPLİSİT spin güncellemesi. Lastik tepki-torku ω'ya STIFF bağlı:
                // reaction = final_long·r, final_long ≈ K·slip, slip = (ω·r − v_long)/ref_vel →
                // ∂reaction/∂ω = K·r²/ref_vel (K = b·c·d·Fz, Pacejka sıfır-slip sertliği). Açık
                // Euler bunu ω'da açık işlerken düşük-ataletli serbest-yuvarlanan tekerlekte
                // λ·dt ≫ 2 olup KAOTİK salınıyordu (ön-tekerlek jitter + dönüşte scrub-sürükleme).
                // Sertliği paydaya ekleyip implisit yapmak koşulsuz kararlı kılar; sürüş/fren
                // dengesini değiştirmez (yalnız kararlılık).
                let slip_stiffness =
                    wheel.pacejka_long.b * wheel.pacejka_long.c * wheel.pacejka_long.d * normal_load;
                let implicit_inertia =
                    effective_inertia + slip_stiffness * wheel.radius * wheel.radius * dt / ref_vel;
                wheel.angular_velocity += (net_torque / implicit_inertia) * dt;

                // --- TRAKSİYON KONTROL (patinaj sınırı) ---
                // Sürülen tekerleğin drive_torque'u lastik tutuşunun ürettiği reaksiyon
                // torkunu (≤ μ·Fz·r) kat kat aşabilir → ω dengeye oturmayıp KAÇAR
                // (ω·r ≫ v_long, gözlemde 109 m/s). Böyle bir slip_ratio sürtünme
                // çemberini BOYUNA doldurur ve Lorentzian çapraz-ağırlık (gy) yüzünden
                // lastiğin YANAL tutuşunu ~0'a çeker: sürülen aks yanal kuvvet üretemez,
                // en küçük yaw bozunumu büyür → araç DÜZ tam gazda kendiliğinden spin
                // atar (kalkışta da yerinde patinaj). ω'yı hedef slip'e kırpmak boyuna
                // kuvveti Pacejka tepesi civarında (iyi hızlanma) tutar ama yanal tutuşu
                // korur (kararlılık). Yalnız TAHRİKLİ tekerlek etkilenir → ön direksiyon
                // yetkisi ve fren/ABS yolu (drive_torque≈0) değişmez; hız arttıkça
                // (ref_vel↑) izin verilen ω da büyür → araç normal hızlanır.
                if wheel.drive_torque.abs() > 1.0 && wheel.radius > 1e-4 {
                    const TC_TARGET_SLIP: f32 = 0.2; // ~Pacejka tepe slip'i
                    let spin_margin = TC_TARGET_SLIP * ref_vel;
                    let hi = (v_long + spin_margin) / wheel.radius;
                    let lo = (v_long - spin_margin) / wheel.radius;
                    wheel.angular_velocity = wheel.angular_velocity.clamp(lo, hi);
                }

                // Fren kilitleme: abs >= tekerlek hızı değilse sıfırla
                let max_brake_decel = wheel.brake_torque / effective_inertia * dt;
                if vehicle.brake_input > 0.01 && wheel.angular_velocity.abs() < max_brake_decel {
                    wheel.angular_velocity = 0.0;
                }
            }
        } else {
            // Havada: sadece motor + fren, yay kuvveti yok
            wheel.suspension_force = 0.0;

            let brake_dir = if wheel.angular_velocity.abs() > 0.01 {
                -wheel.angular_velocity.signum()
            } else {
                0.0
            };

            let effective_brake = wheel.brake_torque * brake_dir;
            let net_torque = wheel.drive_torque + effective_brake;
            wheel.angular_velocity += (net_torque / effective_inertia) * dt;

            // Fren kilitleme: abs >= tekerlek hızı değilse sıfırla
            let max_brake_decel = wheel.brake_torque / effective_inertia * dt;
            if vehicle.brake_input > 0.01 && wheel.angular_velocity.abs() < max_brake_decel {
                wheel.angular_velocity = 0.0;
            }
        }

        // Viskoz spin sönümü SADECE havadaki tekerleğe (hava/rulman sürtünmesi serbest
        // dönen tekerleği yavaşça durdurur). YERDEKİ tekerlekte uygulamak, lastiğin
        // yuvarlanma-kısıtını korumak için sürekli negatif slip üretip şasiye büyük,
        // hıza-orantılı FANTOM geri-sürükleme (~600 N/tekerlek) biniyordu → düz/coast
        // sürüşte ve dönüşte hız kaybı. Yerde: lastik/yuvarlanma zaten yönetir.
        if !wheel.is_grounded {
            let damping_coeff = 2.0; // rad/s² / (rad/s)
            wheel.angular_velocity *= (1.0 - damping_coeff * dt).max(0.0);
        }

        // Çok yavaşsa ve girdi yoksa dur
        if wheel.angular_velocity.abs() < 0.05
            && wheel.drive_torque.abs() < 1.0
            && wheel.brake_torque < 1.0
        {
            wheel.angular_velocity = 0.0;
        }

        // Görsel rotasyon
        wheel.rotation_angle += wheel.angular_velocity * dt;
        wheel.rotation_angle %= std::f32::consts::TAU;
    }
}

// ============================================================
// YARDIMCI FONKSİYONLAR
// ============================================================

/// Merkezi kuvvet (tork olmadan)
#[allow(dead_code)]
fn apply_force_central(rb: &RigidBody, vel: &mut Velocity, force: Vec3, dt: f32) {
    if rb.is_static() {
        return;
    }
    vel.linear += force * rb.inv_mass() * dt;
}

/// Belirli bir noktadan kuvvet uygulama — tork üretir
fn apply_force_at_point(
    rb: &RigidBody,
    vel: &mut Velocity,
    center_of_mass: Vec3,
    rotation: Quat,
    force: Vec3,
    point: Vec3,
    dt: f32,
) {
    if rb.is_static() {
        return;
    }
    vel.linear += (force * rb.inv_mass()) * dt;
    let torque = (point - center_of_mass).cross(force);
    vel.angular += (rb.inv_world_inertia_tensor(rotation) * torque) * dt;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal headless harness: build a 4-wheel VehicleController on flat ground and
    /// drive `update_vehicle` with a hand-rolled semi-implicit integrator (the ECS
    /// rigid integrator is not a dep here). Returns the final state after `steps`.
    fn sim_vehicle(
        throttle: f32,
        steer: f32,
        steps: usize,
        seed: Option<(RigidBody, Transform, Velocity, VehicleController)>,
    ) -> (RigidBody, Transform, Velocity, VehicleController) {
        let veh_id = BodyHandle::from_id(2);
        let ground_id = BodyHandle::from_id(1);
        let ground = Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)); // top at y=0
        let ground_t = Transform::new(Vec3::new(0.0, -1.0, 0.0));

        let (mut rb, mut t, mut vel, mut vc) = seed.unwrap_or_else(|| {
            let mut rb = RigidBody::new(1200.0, true);
            rb.calculate_box_inertia(1.4, 0.7, 2.4);
            rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
            let t = Transform::new(Vec3::new(0.0, 1.0, 0.0)); // start above ground
            let mut vc = VehicleController::new();
            for (x, z, front) in [
                (0.7_f32, 1.0_f32, true),
                (-0.7, 1.0, true),
                (0.7, -1.0, false),
                (-0.7, -1.0, false),
            ] {
                vc.add_wheel(Wheel {
                    attachment_local_pos: Vec3::new(x, 0.2, z),
                    radius: 0.3,
                    axle_type: if front { Axle::Front } else { Axle::Rear },
                    is_left: x > 0.0,
                    suspension_rest_length: 0.15,
                    suspension_max_travel: 0.15,
                    suspension_stiffness: 40000.0,
                    suspension_damping: 3000.0,
                    wheel_mass: 25.0,
                    ..Default::default()
                });
            }
            (rb, t, Velocity::default(), vc)
        });

        let dt = 1.0 / 60.0;
        let gravity = Vec3::new(0.0, -9.81, 0.0);
        let colliders = [
            (ground_id, ground_t, ground.clone()),
            (veh_id, t, Collider::box_collider(Vec3::new(0.7, 0.35, 1.4))),
        ];
        for _ in 0..steps {
            vc.throttle_input = throttle.abs();
            vc.set_reverse(throttle < 0.0);
            vc.steering_input = steer;
            vel.linear += gravity * dt; // gravity (rigid integrator would do this)
            update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, dt);
            // Semi-implicit integrate pose from the mutated velocity.
            t.position += vel.linear * dt;
            if vel.angular.length() > 1e-6 {
                t.rotation = (Quat::from_scaled_axis(vel.angular * dt) * t.rotation).normalize();
            }
            assert!(
                t.position.is_finite() && vel.linear.is_finite() && vel.angular.is_finite(),
                "vehicle sim produced NaN/Inf"
            );
        }
        (rb, t, vel, vc)
    }

    /// E2E: the vehicle must SETTLE on the ground (all wheels grounded, sane ride height),
    /// ACCELERATE under throttle, and — the regression that motivated wiring/fixing the
    /// model — NOT lose all its speed coasting through a turn (the explicit-Euler spin
    /// instability + phantom spin-damper drag used to bleed ~70% of speed and oscillate).
    #[test]
    fn e2e_vehicle_settles_accelerates_and_holds_speed_in_turn() {
        // 1. Settle from a drop, no throttle (~2s).
        let (rb, t, vel, vc) = sim_vehicle(0.0, 0.0, 120, None);
        let grounded = vc.wheels.iter().filter(|w| w.is_grounded).count();
        assert_eq!(grounded, 4, "all 4 wheels must ground after settling, got {grounded}");
        assert!(
            t.position.y > 0.0 && t.position.y < 0.7,
            "chassis rests at a sane height, got {}",
            t.position.y
        );
        assert!(vel.linear.length() < 1.0, "settled car should be ~stationary");

        // 2. Full throttle straight (~3s) → must build real speed.
        let (rb, t, vel, vc) = sim_vehicle(1.0, 0.0, 180, Some((rb, t, vel, vc)));
        let cruise = vel.linear.length();
        assert!(cruise > 3.0, "throttle must accelerate the car, got {cruise} m/s");
        assert!(vc.engine_rpm >= vc.tuning.idle_rpm, "engine rpm must be sane");
        assert!(
            vc.wheels.iter().filter(|w| w.is_grounded).count() == 4,
            "must stay grounded while driving"
        );

        // 3. Coast (no throttle) + full steer (~2.5s) → speed must NOT collapse.
        //    Regression guard: before the implicit-spin + damper-gate fixes this lost ~70%.
        let speed_in = vel.linear.length();
        let (_rb, _t, vel2, _vc) = sim_vehicle(0.0, 1.0, 150, Some((rb, t, vel, vc)));
        let speed_out = vel2.linear.length();
        assert!(
            speed_out > speed_in * 0.55,
            "coasting turn must not scrub away most of the speed: {speed_in:.1} -> {speed_out:.1} m/s"
        );
    }

    /// Aerodinamik hava direnci = ½·ρ·Cd·A·v², hız yönüne KARŞI. Aracı yüksekte (hiçbir
    /// tekerlek yere değmez → tek yatay kuvvet aero), gaz KAPALI, bilinen hızda tek adım
    /// sürüp yatay hız kaybının analitik ½ρCdAv²/m·dt ile eşleştiğini doğrular. Böylece
    /// drag hem UYGULANIYOR hem DOĞRU formül/işaret/büyüklükte.
    #[test]
    fn aero_drag_matches_half_rho_cd_a_v_squared() {
        const MASS: f32 = 1200.0;
        const V0: f32 = 50.0; // m/s ileri (-Z)
        const RHO: f32 = 1.225;
        let dt = 1.0 / 60.0;

        let veh_id = BodyHandle::from_id(2);
        let mut rb = RigidBody::new(MASS, true);
        rb.calculate_box_inertia(1.4, 0.7, 2.4);
        rb.center_of_mass = Vec3::new(0.0, 0.2, 0.0);
        let t = Transform::new(Vec3::new(0.0, 100.0, 0.0)); // havada
        let mut vc = VehicleController::new();
        for (x, z, front) in [
            (0.7_f32, 1.0_f32, true),
            (-0.7, 1.0, true),
            (0.7, -1.0, false),
            (-0.7, -1.0, false),
        ] {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.2, z),
                radius: 0.3,
                axle_type: if front { Axle::Front } else { Axle::Rear },
                is_left: x > 0.0,
                suspension_rest_length: 0.15,
                suspension_max_travel: 0.15,
                ..Default::default()
            });
        }
        // Zemin ÇOK aşağıda → süspansiyon ışını ona ulaşmaz → hiçbir tekerlek grounded olmaz.
        let colliders = [(
            BodyHandle::from_id(1),
            Transform::new(Vec3::new(0.0, -1000.0, 0.0)),
            Collider::box_collider(Vec3::new(200.0, 1.0, 200.0)),
        )];

        let mut vel = Velocity::new(Vec3::new(0.0, 0.0, -V0));
        vc.throttle_input = 0.0;
        vc.brake_input = 0.0;
        vc.steering_input = 0.0;

        update_vehicle(veh_id, &mut vc, &mut rb, &t, &mut vel, &colliders, dt);

        // İzolasyon garantisi: airborne → tek yatay kuvvet aero drag.
        assert!(
            vc.wheels.iter().all(|w| !w.is_grounded),
            "araç havada olmalı (hiçbir tekerlek grounded değil)"
        );

        // Beklenen: F = ½ρ·Cd·A·v² (+Z, yani hareketin tersi); Δv = F·dt/m.
        let aero = &vc.tuning.aero;
        let f_drag = 0.5 * RHO * V0 * V0 * aero.drag_coefficient * aero.frontal_area;
        let expected_dv = f_drag / MASS * dt;
        let actual_dv = vel.linear.z - (-V0); // -50'den +Z'ye ne kadar arttı (yavaşlama)

        assert!(
            actual_dv > 0.0,
            "drag hareketin TERSİNE olmalı (+Z), bulundu Δvz={actual_dv}"
        );
        assert!(
            (actual_dv - expected_dv).abs() < expected_dv * 0.02,
            "drag Δv {actual_dv:.6} ≈ ½ρCdAv²/m·dt = {expected_dv:.6} olmalı (±%2)"
        );
        assert!(
            vel.linear.length() < V0,
            "drag toplam hızı düşürmeli: {} < {V0}",
            vel.linear.length()
        );
    }

    /// Ground-effect: downforce yalnız araç GERÇEKTEN alçaldığında artmalı; normal
    /// sürüş yüksekliğinde etki 1.0 (davranış korunur). Yumuşak, monoton rampa.
    #[test]
    fn ground_effect_ramps_downforce_only_when_low() {
        let (height, mult) = (0.15_f32, 1.8_f32);
        // Normal ride height (clearance ≥ height) → NO effect (davranış aynen korunur).
        assert_eq!(ground_effect_factor(0.36, height, mult), 1.0);
        assert_eq!(ground_effect_factor(height, height, mult), 1.0);
        // Gövde yere değerken → tam multiplier.
        assert!((ground_effect_factor(0.0, height, mult) - mult).abs() < 1e-5);
        // Yarı yükseklik → yumuşak rampanın tam ortası.
        let mid = ground_effect_factor(height * 0.5, height, mult);
        assert!((mid - (1.0 + (mult - 1.0) * 0.5)).abs() < 1e-5, "mid={mid}");
        // Monoton: alçaldıkça downforce artar.
        assert!(
            ground_effect_factor(0.05, height, mult) > ground_effect_factor(0.10, height, mult)
        );
        // Negatif clearance (gövde yerin içinde) → multiplier'a kırpılır (patlamaz).
        assert!((ground_effect_factor(-0.1, height, mult) - mult).abs() < 1e-5);
    }

    /// Pacejka combined-slip must stay inside the friction circle: the resultant of the
    /// longitudinal + lateral tire force cannot exceed mu_peak * normal_load.
    #[test]
    fn pacejka_combined_respects_friction_circle() {
        let long = PacejkaParams::default();
        let lat = PacejkaParams::default();
        let fz = 3000.0_f32;
        // Large slip in both axes → both axes saturate; resultant must be clamped.
        let (fx, fy) = pacejka_combined(&long, &lat, 0.6, 0.6, fz);
        let mag = (fx * fx + fy * fy).sqrt();
        let limit = long.d.max(lat.d) * 1.2 * fz;
        assert!(
            mag <= limit + 1.0,
            "combined force {mag} must not exceed friction-circle limit {limit}"
        );
        // Zero slip → zero force.
        let (fx0, fy0) = pacejka_combined(&long, &lat, 0.0, 0.0, fz);
        assert!(fx0.abs() < 1e-3 && fy0.abs() < 1e-3, "no slip → no force");
    }

    /// Ackermann geometry: the inner wheel (nearer the turn centre) must steer
    /// more sharply than the outer wheel, for turns in both directions. The old
    /// code had the inner/outer half-track sign flipped (reverse-Ackermann), so
    /// the inner wheel came out *shallower* — this asserts the corrected mapping.
    #[test]
    fn ackermann_inner_wheel_steers_more_than_outer() {
        let wheelbase = 2.8_f32;
        let track = 1.6_f32;

        // Left turn: positive steer → positive turn radius → left wheel is inner.
        let steer = 0.3_f32;
        let turn_radius = wheelbase / steer.tan();
        let left = ackermann_steering_angle(steer, turn_radius, wheelbase, track, true);
        let right = ackermann_steering_angle(steer, turn_radius, wheelbase, track, false);
        assert!(
            left > right,
            "left(inner) {left} must steer more than right(outer) {right} on a left turn"
        );

        // Right turn: mirror — right wheel is inner and steers more (in magnitude).
        let steer = -0.3_f32;
        let turn_radius = wheelbase / steer.tan();
        let left = ackermann_steering_angle(steer, turn_radius, wheelbase, track, true);
        let right = ackermann_steering_angle(steer, turn_radius, wheelbase, track, false);
        assert!(
            right.abs() > left.abs(),
            "right(inner) {right} must steer more than left(outer) {left} on a right turn"
        );
    }

    #[test]
    fn test_suspension_spring_and_damper_math() {
        // "Kuvvet Testi: Bir yaya 10cm sıkışma uygulandığında, sönümleme katsayısı X iken..."
        let stiffness = 25000.0; // N/m (Süspansiyon yay sertliği)
        let compression = 0.1; // 0.1 m (10 cm sıkışma)
        let spring_force = stiffness * compression;

        // Yay tam 0.1 metre sıkıştığında, Hooke Kanunu'na göre (F = k*x) 2500N kuvvet üretmeli.
        assert_eq!(spring_force, 2500.0, "Hooke's Law spring force failed");

        // Sönümleme (Damper) Testi
        let damping_compression = 3000.0; // N*s/m (Sönümleme katsayısı)
        let susp_vel_compressing = 1.0; // 1 m/s hızla sıkışıyor (amortisör direnci)

        // Baskı sırasında damper kuvveti hıza zıt (dirençli) ve pozitif olmalı (F = c*v)
        let damper_force = damping_compression * susp_vel_compressing;
        assert_eq!(damper_force, 3000.0, "Damper force calculation failed");

        // Toplam Süspansiyon Kuvveti (Yay + Amortisör)
        let total_suspension_force = spring_force + damper_force;
        assert_eq!(
            total_suspension_force, 5500.0,
            "Total suspension force calculation failed"
        );
    }

    /// Anti-roll bar must RESIST body roll, not amplify it. With the left corner
    /// more compressed than the right (`diff > 0` → body rolling left), the bar
    /// must add up-force to the lower (left) corner and remove it from the higher
    /// (right) corner. The old code flipped both signs (pro-roll), so `left < 0`
    /// and `right > 0` — this locks the corrected convention.
    #[test]
    fn anti_roll_bar_resists_roll_not_amplifies() {
        let k = 3000.0;
        let diff = 0.05; // left suspension 5 cm more compressed than right

        let left = anti_roll_force(true, diff, k);
        let right = anti_roll_force(false, diff, k);

        assert!(
            left > 0.0,
            "ARB must add up-force to the more-compressed (lower) left corner, got {left}"
        );
        assert!(
            right < 0.0,
            "ARB must remove up-force from the less-compressed (higher) right corner, got {right}"
        );
        // A torsion bar transfers load — equal and opposite, zero net vertical force.
        assert!(
            (left + right).abs() < 1e-6,
            "ARB forces must be equal and opposite (load transfer), got {left} and {right}"
        );
        // A level car (no compression difference) produces no ARB force.
        assert_eq!(anti_roll_force(true, 0.0, k), 0.0);
        assert_eq!(anti_roll_force(false, 0.0, k), 0.0);
    }

    #[test]
    fn test_pacejka_combined_slip() {
        let long = PacejkaParams::default();
        let lat = PacejkaLat::default();
        let normal_load = 5000.0; // 500 kg tekerlek yükü (Fz)

        // 1. Durum: Sıfır Slip (Kayma Yok)
        let (fx1, fy1) = pacejka_combined(&long, &lat, 0.0, 0.0, normal_load);
        assert!(
            fx1.abs() < 1e-4,
            "Expected zero longitudinal force at zero slip"
        );
        assert!(fy1.abs() < 1e-4, "Expected zero lateral force at zero slip");

        // 2. Durum: Sadece İleri Kayma (Burnout/Frenleme)
        let (fx2, fy2) = pacejka_combined(&long, &lat, 0.15, 0.0, normal_load);
        let expected_fx2 = long.calculate_force(0.15, normal_load);
        assert!(
            (fx2 - expected_fx2).abs() < 1e-4,
            "Expected combined force to match pure force when no lateral slip is present"
        );
        assert!(
            fy2.abs() < 1e-4,
            "Expected zero lateral force when purely accelerating straight"
        );

        // 3. Durum: Kombine Kayma (Virajda Gazlama - Friction Circle Test)
        // Her iki yönde kayma olduğunda (Drift durumu), eksenler birbirinin tutuşunu düşürmeli (Weighting)
        let (fx3, fy3) = pacejka_combined(&long, &lat, 0.15, 0.15, normal_load);

        // fx3, fx2'den (sadece düz gitmekten) çok daha düşük olmalıdır çünkü yanal kuvvet (fy3) de yol tutuşundan pay alıyor
        assert!(
            fx3 < fx2,
            "Combined slip should reduce longitudinal grip (Friction Circle violated)"
        );
        assert!(
            fy3 > 1000.0,
            "Expected significant lateral force during cornering"
        );
    }
}
