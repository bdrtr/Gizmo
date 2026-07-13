use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_core::components::PhysicsMaterial;
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::Weather;
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

/// Tahrik düzeni: motor torku hangi aksa gider ve motor RPM'i hangi tekerleklerden türetilir.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drivetrain {
    /// Önden çekiş
    Fwd,
    /// Arkadan itiş (varsayılan)
    Rwd,
    /// Dört tekerlek
    Awd,
}

impl Drivetrain {
    /// Bu aks tahrik ediliyor mu?
    #[inline]
    pub fn drives(self, axle: &Axle) -> bool {
        match self {
            Drivetrain::Fwd => *axle == Axle::Front,
            Drivetrain::Rwd => *axle == Axle::Rear,
            Drivetrain::Awd => true,
        }
    }
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
    /// Temas edilen zeminin dinamik sürtünmesi (grip çarpanı için). 1. geçiş raycast'inde
    /// çarpılan collider'ın PhysicsMaterial'ından yakalanır; havadayken ASPHALT'a döner.
    /// grip_mult = surface_friction / ASPHALT.dynamic_friction → buz/kum/asfalt gerçek tutuş verir.
    pub surface_friction: f32,
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
            surface_friction: PhysicsMaterial::ASPHALT.dynamic_friction,
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
    pub flywheel_inertia: f32,   // kg·m²
    /// Tahrik düzeni (FWD/RWD/AWD) — tork dağıtımı ve RPM türetimi buna göre.
    pub drivetrain: Drivetrain,
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
            flywheel_inertia: 0.25, // kg·m²
            drivetrain: Drivetrain::Rwd,
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
// The simulation + its tests live in `dynamics`; re-export so `vehicle::update_vehicle`,
// `vehicle::weather_grip_factor`, etc. (and the crate-root `pub use vehicle::*`) stay unchanged.
// ============================================================
mod dynamics;
pub use dynamics::*;
