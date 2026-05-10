use crate::components::{Collider, RigidBody, Transform, Velocity};
use crate::raycast::{Ray, Raycast, RaycastHit};
use gizmo_core::entity::Entity;
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
        1.0 / (1.0 + k * k).sqrt() // Lorentzian scaling
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

#[derive(Clone)]
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
    pub fn get_engine_torque(&self) -> f32 {
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
        let max_gear = self.tuning.gear_ratios.len() - 1;
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

pub fn update_vehicle(
    vehicle_entity: Entity,
    vehicle: &mut VehicleController,
    vehicle_rb: &mut RigidBody,
    vehicle_transform: &Transform,
    vehicle_vel: &mut Velocity,
    all_colliders: &[(Entity, Transform, Collider)],
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

    let engine_torque = vehicle.get_engine_torque();
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

    // Zemin etkisi: alçak araçlarda downforce artar
    let height_above_ground = vehicle
        .wheels
        .iter()
        .filter(|w| w.is_grounded)
        .filter_map(|w| w.ground_hit.as_ref().map(|hit| hit.distance - 0.5)) // 0.5 is ray_origin_offset
        .fold(f32::MAX, f32::min);
    let ge_factor = if height_above_ground < a.ground_effect_height {
        a.ground_effect_multiplier
    } else {
        1.0
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
            let sign = if wheel.is_left { 1.0 } else { -1.0 };
            wheel.steering_angle = if turn_radius.abs() < 1e4 {
                (vehicle.tuning.wheelbase / (turn_radius + sign * vehicle.tuning.track_width * 0.5))
                    .atan()
            } else {
                steer_angle
            };
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
        // Tekerlek ataletini (I = 0.5 m r²) hesapla
        let wheel_inertia = 0.5 * wheel.wheel_mass * wheel.radius.powi(2);

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
                let damper_force = damper_coeff * susp_vel;

                // Bump stop: max seyahat sonunda sert non-linear yay
                let bump_stop_travel = wheel.suspension_max_travel * 0.1;
                let bump_excess = compression - (wheel.suspension_max_travel - bump_stop_travel);
                let bump_stop_force = if bump_excess > 0.0 {
                    bump_excess * wheel.suspension_stiffness * 8.0
                } else {
                    0.0
                };

                // Anti-roll bar
                let arb_force = match wheel.axle_type {
                    Axle::Front => {
                        if wheel.is_left {
                            -front_diff
                        } else {
                            front_diff
                        }
                    }
                    Axle::Rear => {
                        if wheel.is_left {
                            -rear_diff
                        } else {
                            rear_diff
                        }
                    }
                } * vehicle.tuning.anti_roll_stiffness;

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

                // Semi-implicit: önce hızı güncelle, sonra pozisyonu
                wheel.angular_velocity += (net_torque / wheel_inertia) * dt;

                // Fren kilitleme: abs >= tekerlek hızı değilse sıfırla
                let max_brake_decel = wheel.brake_torque / wheel_inertia * dt;
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
            wheel.angular_velocity += (net_torque / wheel_inertia) * dt;

            // Fren kilitleme: abs >= tekerlek hızı değilse sıfırla
            let max_brake_decel = wheel.brake_torque / wheel_inertia * dt;
            if vehicle.brake_input > 0.01 && wheel.angular_velocity.abs() < max_brake_decel {
                wheel.angular_velocity = 0.0;
            }
        }

        // dt-doğru sönümleme: (1 - coeff * dt) ≈ exp(-coeff * dt)
        let damping_coeff = 2.0; // rad/s² / (rad/s)
        wheel.angular_velocity *= (1.0 - damping_coeff * dt).max(0.0);

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
