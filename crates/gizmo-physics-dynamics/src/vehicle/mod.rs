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
        // Sıcak yol: her lastik her frame çağrılır → yalnız GERÇEKTEN doygunken (limit aşıldı)
        // trace. Kaymanın hangi eksenden geldiğini ve ne kadar kırpıldığını verir.
        tracing::trace!(
            mag,
            limit,
            scale,
            slip_ratio,
            slip_angle,
            normal_load,
            "[Tire] friction circle saturated — clamping combined force to μ·Fz"
        );
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
    /// Ölçülmüş tork eğrisi: `(rpm, N·m)` çiftleri, **devire göre artan** sırada.
    ///
    /// Boş bırakılırsa (varsayılan) motor [`Self::max_engine_torque`] ile ölçeklenen kendi
    /// parametrik çan eğrisini kullanır — yani mevcut davranış bit düzeyinde korunur.
    /// Doluysa çan eğrisi tamamen devre dışı kalır ve tork bu noktalardan doğrusal
    /// interpolasyonla okunur.
    ///
    /// Neden var: çan eğrisi her aracın tepe torkunu **aynı devirde** yapmasına yol açıyor
    /// (`ratio = 0.4`). Gerçek bir aracın eğrisi elde varsa bu, arabaları birbirinden ayıran
    /// şeyi atmak demek — iki farklı motor aynı tepe torka sahipse birebir aynı sürülür.
    /// Örnek: NFSU2'nun 240SX kaydı tepe torkunu 4675 devirde yapıyor, çan eğrisi ise
    /// 3280'de; 1.400 devirlik bir fark ve tamamen farklı bir düşüş şekli.
    ///
    /// Aralık dışı devirler **uçlara sabitlenir**, sıfıra düşmez: motorun kırmızı çizgiyi
    /// aşınca arabayı frenlemesi gerçek değil, ve çan eğrisinin `0.05` tabanı da tam bunun
    /// için vardı.
    pub torque_curve: Vec<(f32, f32)>,
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
            // Boş = çan eğrisi. Varsayılanın davranışı değişmiyor.
            torque_curve: Vec::new(),
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

    /// Motor tork eğrisi — ölçülmüş noktalar varsa onlardan, yoksa parametrik çan eğrisinden.
    ///
    /// İki yol da gazla doğrusal ölçekleniyor ve ikisi de asla negatif dönmüyor: kırmızı çizgiyi
    /// aşan bir motorun arabayı *frenlemesi* fizik değil, hata olur.
    pub fn engine_torque(&self) -> f32 {
        let t = &self.tuning;
        let nm = if t.torque_curve.is_empty() {
            let ratio = (self.engine_rpm - t.idle_rpm).max(0.0) / (t.max_rpm - t.idle_rpm).max(1.0);
            let curve = (1.0 - (ratio - 0.4).powi(2) * 2.5).clamp(0.05, 1.0);
            t.max_engine_torque * curve
        } else {
            Self::torque_from_curve(&t.torque_curve, self.engine_rpm)
        };
        nm * self.throttle_input.abs()
    }

    /// `(rpm, N·m)` noktalarından devire karşılık gelen torku doğrusal interpolasyonla okur.
    ///
    /// Uçların dışı **sabitlenir** (ekstrapolasyon yok): ilk noktanın altında ilk değer, son
    /// noktanın üstünde son değer. Ekstrapolasyon yapsaydı düşen bir eğri yeterince yüksek
    /// devirde negatife geçer ve motor arabayı frenlemeye başlardı — çan eğrisinin `0.05`
    /// tabanının koruduğu şey buydu, ve aynı koruma burada da olmalı.
    ///
    /// Noktaların artan sırada olduğu varsayılır. Değilse sonuç anlamsız olur ama panik veya
    /// negatif değer üretmez; sıralama çağıranın işi, çünkü onu burada her karede yapmak
    /// ölçülmüş bir eğriyi kullanmanın maliyetini boşuna ikiye katlar.
    fn torque_from_curve(points: &[(f32, f32)], rpm: f32) -> f32 {
        let first = points[0];
        if rpm <= first.0 {
            return first.1.max(0.0);
        }
        let last = points[points.len() - 1];
        if rpm >= last.0 {
            return last.1.max(0.0);
        }
        for pair in points.windows(2) {
            let (lo, hi) = (pair[0], pair[1]);
            if rpm <= hi.0 {
                let span = hi.0 - lo.0;
                // Aynı deviri iki kez yazan bir eğri sıfıra bölmez; ikincisi kazanır.
                if span <= 0.0 {
                    return hi.1.max(0.0);
                }
                let k = (rpm - lo.0) / span;
                return (lo.1 + (hi.1 - lo.1) * k).max(0.0);
            }
        }
        last.1.max(0.0)
    }

    pub fn set_reverse(&mut self, on: bool) {
        // set_reverse çağırıcı tarafından her frame çağrılabilir (girdi eşlemesi); yalnız
        // değer GERÇEKTEN değiştiğinde logla — aksi halde per-frame gürültü olur.
        let changed = self.reverse_input != on;
        self.current_gear = if on {
            0
        } else if self.current_gear == 0 {
            2
        } else {
            self.current_gear
        };
        self.reverse_input = on;
        if changed {
            tracing::debug!(reverse = on, gear = self.current_gear, "[Vehicle] reverse toggled");
        }
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
            tracing::debug!(
                gear = self.current_gear,
                rpm = self.engine_rpm,
                upshift_rpm = self.tuning.upshift_rpm,
                "[Vehicle] auto-shift UP"
            );
        } else if self.engine_rpm < self.tuning.downshift_rpm && self.current_gear > 2 {
            self.current_gear -= 1;
            self.shift_cooldown = 0.3;
            tracing::debug!(
                gear = self.current_gear,
                rpm = self.engine_rpm,
                downshift_rpm = self.tuning.downshift_rpm,
                "[Vehicle] auto-shift DOWN"
            );
        }
    }
}

// ============================================================
// The simulation + its tests live in `dynamics`; re-export so `vehicle::update_vehicle`,
// `vehicle::weather_grip_factor`, etc. (and the crate-root `pub use vehicle::*`) stay unchanged.
// ============================================================
mod dynamics;
pub use dynamics::*;

#[cfg(test)]
mod tests {
    //! Pure unit tests for the vehicle *data types* (torque curve, reverse/auto-shift
    //! state machines, Pacejka single-axis math). The per-frame `update_vehicle`
    //! simulation lives — with its own tests — in `dynamics`.
    use super::*;

    // ── Engine torque curve ─────────────────────────────────

    /// The engine torque bell-curve must peak at `ratio = 0.4` (the rpm where the
    /// parabolic `curve` term reaches its clamped maximum of 1.0), and there it must
    /// equal `max_engine_torque · throttle`. Torque at idle and at redline must both
    /// be strictly below the peak. Locks the shape of the curve, not just "nonzero".
    #[test]
    fn engine_torque_peaks_at_ratio_0_4() {
        let mut vc = VehicleController::new();
        vc.throttle_input = 1.0;
        let t = &vc.tuning;
        // ratio = (rpm - idle)/(max - idle) = 0.4  →  rpm = idle + 0.4·(max - idle)
        let peak_rpm = t.idle_rpm + 0.4 * (t.max_rpm - t.idle_rpm);
        let (idle, max, max_torque) = (t.idle_rpm, t.max_rpm, t.max_engine_torque);

        vc.engine_rpm = peak_rpm;
        let peak = vc.engine_torque();
        assert!(
            (peak - max_torque).abs() < 1e-3,
            "peak torque must be max_engine_torque at ratio 0.4, got {peak} vs {max_torque}"
        );

        vc.engine_rpm = idle;
        let at_idle = vc.engine_torque();
        vc.engine_rpm = max;
        let at_redline = vc.engine_torque();
        assert!(at_idle < peak, "idle torque {at_idle} must be below peak {peak}");
        assert!(at_redline < peak, "redline torque {at_redline} must be below peak {peak}");
        assert!(at_idle > 0.0 && at_redline > 0.0, "curve stays positive across the band");
    }

    /// Torque scales linearly with throttle and is exactly zero at closed throttle.
    #[test]
    fn engine_torque_scales_with_throttle_and_is_zero_when_closed() {
        let mut vc = VehicleController::new();
        vc.engine_rpm = 3000.0;

        vc.throttle_input = 0.0;
        assert_eq!(vc.engine_torque(), 0.0, "closed throttle → no torque");

        vc.throttle_input = 0.5;
        let half = vc.engine_torque();
        vc.throttle_input = 1.0;
        let full = vc.engine_torque();
        assert!(half > 0.0);
        assert!(
            (full - 2.0 * half).abs() < 1e-3,
            "torque must be linear in throttle: full {full} ≈ 2·half {half}"
        );
    }

    /// The parabolic curve term goes negative for extreme rpm, but the `.clamp(0.05, 1.0)`
    /// floor keeps torque positive at `0.05·max·throttle` — never negative (which would
    /// mean the engine *brakes* the car past redline). Guards the lower clamp.
    #[test]
    fn engine_torque_clamps_to_positive_floor_past_redline() {
        let mut vc = VehicleController::new();
        vc.throttle_input = 1.0;
        vc.engine_rpm = 20_000.0; // absurd over-rev → raw curve term goes deeply negative
        let torque = vc.engine_torque();
        let expected_floor = vc.tuning.max_engine_torque * 0.05;
        assert!(torque > 0.0, "torque must never go negative, got {torque}");
        assert!(
            (torque - expected_floor).abs() < 1e-3,
            "over-rev torque must sit on the 0.05 floor = {expected_floor}, got {torque}"
        );
    }

    // ── Measured torque curve ───────────────────────────────

    /// A car whose curve is given reads it back point for point, and interpolates between.
    ///
    /// The numbers are NFSU2's 240SX record — nine points from idle to the limiter — because the
    /// whole reason this field exists is that the bell curve cannot represent them: this engine
    /// peaks at `ratio = 0.4`, which is 3280 rpm on that car, and the car itself peaks at 4675.
    #[test]
    fn a_measured_curve_is_read_instead_of_the_bell() {
        let mut vc = VehicleController::new();
        vc.throttle_input = 1.0;
        vc.tuning.torque_curve = vec![
            (800.0, 140.0),
            (1575.0, 150.0),
            (2350.0, 160.0),
            (3125.0, 180.0),
            (3900.0, 200.0),
            (4675.0, 216.0),
            (5450.0, 203.0),
            (6225.0, 170.0),
            (7000.0, 150.0),
        ];

        // Every point comes back exactly.
        for &(rpm, nm) in &vc.tuning.torque_curve.clone() {
            vc.engine_rpm = rpm;
            let got = vc.engine_torque();
            assert!((got - nm).abs() < 1e-3, "at {rpm} rpm expected {nm}, got {got}");
        }

        // Half way between two points is half way between their values.
        vc.engine_rpm = (4675.0 + 5450.0) / 2.0;
        let mid = vc.engine_torque();
        assert!((mid - (216.0 + 203.0) / 2.0).abs() < 1e-3, "{mid}");

        // And the peak is where the *car* puts it, not where the bell would.
        let bell_peak_rpm = vc.tuning.idle_rpm + 0.4 * (vc.tuning.max_rpm - vc.tuning.idle_rpm);
        vc.engine_rpm = bell_peak_rpm;
        let at_bell_peak = vc.engine_torque();
        vc.engine_rpm = 4675.0;
        let at_real_peak = vc.engine_torque();
        assert!(
            at_real_peak > at_bell_peak,
            "the measured peak {at_real_peak} must beat the bell's rpm {at_bell_peak}"
        );
    }

    /// Outside the curve the ends are held, never extrapolated — because a falling curve
    /// extrapolated far enough goes negative, and a negative engine torque is an engine braking
    /// the car. That is exactly what the bell curve's `0.05` floor was protecting against, so the
    /// measured path has to protect against it too.
    #[test]
    fn a_measured_curve_holds_its_ends_rather_than_extrapolating() {
        let mut vc = VehicleController::new();
        vc.throttle_input = 1.0;
        vc.tuning.torque_curve = vec![(1000.0, 100.0), (5000.0, 300.0), (7000.0, 120.0)];

        vc.engine_rpm = 0.0;
        assert!((vc.engine_torque() - 100.0).abs() < 1e-3, "below the first point holds it");
        vc.engine_rpm = 500.0;
        assert!((vc.engine_torque() - 100.0).abs() < 1e-3);

        vc.engine_rpm = 20_000.0;
        let over = vc.engine_torque();
        assert!((over - 120.0).abs() < 1e-3, "far past the last point holds it, got {over}");
        assert!(over > 0.0, "and never goes negative");
    }

    /// The default is empty, so every existing vehicle keeps the bell curve **bit for bit**.
    /// This engine's determinism contract makes that the whole point of the field being a `Vec`
    /// with an empty default rather than a curve someone has to opt out of.
    #[test]
    fn an_empty_curve_leaves_the_bell_untouched() {
        assert!(VehicleTuning::default().torque_curve.is_empty());

        let mut bell = VehicleController::new();
        bell.throttle_input = 0.7;
        let mut measured = VehicleController::new();
        measured.throttle_input = 0.7;
        measured.tuning.torque_curve = vec![(800.0, 999.0), (7000.0, 999.0)];

        for rpm in [800.0, 2000.0, 3280.0, 5000.0, 7000.0, 12_000.0] {
            bell.engine_rpm = rpm;
            measured.engine_rpm = rpm;
            let b = bell.engine_torque();
            // The bell path is unchanged: reproduce it here rather than trusting the reading.
            let t = &bell.tuning;
            let ratio = (rpm - t.idle_rpm).max(0.0) / (t.max_rpm - t.idle_rpm).max(1.0);
            let want = t.max_engine_torque
                * (1.0 - (ratio - 0.4).powi(2) * 2.5).clamp(0.05, 1.0)
                * 0.7;
            assert!((b - want).abs() < 1e-3, "at {rpm} rpm bell gave {b}, expected {want}");
            // …and a filled curve takes the other branch, so the two disagree.
            assert!((measured.engine_torque() - 999.0 * 0.7).abs() < 1e-3);
        }
    }

    /// A measured curve still scales linearly with throttle and is zero when it is closed, the
    /// same contract the bell curve keeps.
    #[test]
    fn a_measured_curve_still_obeys_the_throttle() {
        let mut vc = VehicleController::new();
        vc.tuning.torque_curve = vec![(1000.0, 200.0), (6000.0, 400.0)];
        vc.engine_rpm = 3500.0;

        vc.throttle_input = 0.0;
        assert_eq!(vc.engine_torque(), 0.0);
        vc.throttle_input = 0.5;
        let half = vc.engine_torque();
        vc.throttle_input = 1.0;
        let full = vc.engine_torque();
        assert!((full - 2.0 * half).abs() < 1e-3, "full {full} ≈ 2·half {half}");
        assert!((full - 300.0).abs() < 1e-3, "3500 rpm is the midpoint: {full}");
    }

    /// Degenerate curves do not panic and do not produce a negative or infinite torque: one
    /// point, a repeated rpm (which would divide by zero), and a negative value in the table.
    #[test]
    fn a_degenerate_curve_is_survived_rather_than_trusted() {
        let mut vc = VehicleController::new();
        vc.throttle_input = 1.0;

        vc.tuning.torque_curve = vec![(3000.0, 250.0)];
        for rpm in [0.0, 3000.0, 9000.0] {
            vc.engine_rpm = rpm;
            assert!((vc.engine_torque() - 250.0).abs() < 1e-3, "one point is a flat curve");
        }

        vc.tuning.torque_curve = vec![(1000.0, 100.0), (4000.0, 200.0), (4000.0, 300.0), (6000.0, 50.0)];
        vc.engine_rpm = 4000.0;
        let at_dup = vc.engine_torque();
        assert!(at_dup.is_finite() && at_dup > 0.0, "a repeated rpm must not divide by zero");

        vc.tuning.torque_curve = vec![(1000.0, -50.0), (6000.0, -10.0)];
        for rpm in [500.0, 3000.0, 9000.0] {
            vc.engine_rpm = rpm;
            assert_eq!(vc.engine_torque(), 0.0, "a negative table never brakes the car");
        }
    }

    // ── set_reverse state machine ───────────────────────────

    /// Engaging reverse selects gear index 0 (the reverse ratio) and sets the flag;
    /// disengaging from reverse returns to gear 2 (first forward gear). Coming out of
    /// reverse does NOT restore whatever forward gear was selected before — it always
    /// lands on gear 2. Locks the exact transitions of the little gearbox state machine.
    #[test]
    fn set_reverse_toggles_gear_and_flag() {
        let mut vc = VehicleController::new();
        assert_eq!(vc.current_gear, 2, "default is first forward gear");

        vc.set_reverse(true);
        assert_eq!(vc.current_gear, 0, "reverse selects gear index 0");
        assert!(vc.reverse_input);

        vc.set_reverse(false);
        assert_eq!(vc.current_gear, 2, "leaving reverse returns to gear 2");
        assert!(!vc.reverse_input);

        // Engaging reverse from a high forward gear, then leaving, still lands on 2
        // (no restore of the prior forward gear).
        vc.current_gear = 5;
        vc.set_reverse(true);
        assert_eq!(vc.current_gear, 0);
        vc.set_reverse(false);
        assert_eq!(vc.current_gear, 2, "no restore of prior forward gear");
    }

    /// Requesting "not reverse" while already in a forward gear must leave that gear
    /// untouched (the else-if only rescues gear 0). Idempotent for repeated reverse.
    #[test]
    fn set_reverse_false_in_forward_gear_is_noop_and_reverse_is_idempotent() {
        let mut vc = VehicleController::new();
        vc.current_gear = 4;
        vc.set_reverse(false);
        assert_eq!(vc.current_gear, 4, "disengaging reverse from a forward gear keeps it");
        assert!(!vc.reverse_input);

        vc.set_reverse(true);
        vc.set_reverse(true);
        assert_eq!(vc.current_gear, 0, "repeated reverse stays at gear 0");
        assert!(vc.reverse_input);
    }

    // ── auto_shift_tick state machine ───────────────────────

    /// Above `upshift_rpm` the box shifts up one gear and arms a cooldown; while the
    /// cooldown is active a subsequent tick (even still over-rev) must NOT shift again;
    /// once enough dt has elapsed to clear the cooldown, the next over-rev tick upshifts.
    #[test]
    fn auto_shift_upshifts_then_respects_cooldown() {
        let mut vc = VehicleController::new(); // gear 2, cooldown 0, auto_shift on
        vc.engine_rpm = vc.tuning.upshift_rpm + 300.0;

        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 3, "over-rev must upshift");
        assert!(vc.shift_cooldown > 0.0, "upshift arms a cooldown");

        // Still over-rev, but cooldown active → no shift.
        vc.auto_shift_tick(0.1);
        assert_eq!(vc.current_gear, 3, "cooldown must block a back-to-back upshift");

        // A large dt clears the remaining cooldown; the same tick then upshifts.
        vc.auto_shift_tick(1.0);
        assert_eq!(vc.current_gear, 4, "after cooldown elapses, over-rev upshifts again");
    }

    /// Below `downshift_rpm` the box shifts down — but never below gear 2 (the first
    /// forward gear; 0 = reverse, 1 = neutral must not be auto-selected).
    #[test]
    fn auto_shift_downshifts_but_floors_at_gear_2() {
        let mut vc = VehicleController::new();
        vc.current_gear = 4;
        vc.engine_rpm = vc.tuning.downshift_rpm - 500.0;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 3, "under-rev must downshift");

        // At gear 2, under-rev must NOT drop into neutral/reverse.
        let mut vc = VehicleController::new(); // gear 2
        vc.engine_rpm = vc.tuning.downshift_rpm - 500.0;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 2, "must not auto-shift below first forward gear");
    }

    /// Guards: auto-shift is skipped in reverse and when disabled, is clamped at the
    /// top gear, is a no-op inside the rpm dead-band, and (the underflow guard) must
    /// not panic when `gear_ratios` is empty.
    #[test]
    fn auto_shift_guards_reverse_disabled_topgear_deadband_and_empty() {
        // Reverse → skipped even at high rpm.
        let mut vc = VehicleController::new();
        vc.set_reverse(true); // gear 0, reverse_input true
        vc.engine_rpm = vc.tuning.upshift_rpm + 1000.0;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 0, "no auto-shift while in reverse");

        // Disabled → skipped.
        let mut vc = VehicleController::new();
        vc.auto_shift = false;
        vc.engine_rpm = vc.tuning.upshift_rpm + 1000.0;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 2, "no auto-shift when auto_shift is off");

        // Top gear → clamped (cannot exceed the last ratio index).
        let mut vc = VehicleController::new();
        vc.current_gear = vc.tuning.gear_ratios.len() - 1;
        vc.engine_rpm = vc.tuning.upshift_rpm + 1000.0;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(
            vc.current_gear,
            vc.tuning.gear_ratios.len() - 1,
            "cannot upshift past the top gear"
        );

        // Dead-band (between downshift and upshift) → unchanged.
        let mut vc = VehicleController::new();
        vc.current_gear = 3;
        vc.engine_rpm = (vc.tuning.downshift_rpm + vc.tuning.upshift_rpm) * 0.5;
        vc.auto_shift_tick(1.0 / 60.0);
        assert_eq!(vc.current_gear, 3, "no shift inside the rpm dead-band");

        // Empty gear_ratios → early return, no `len()-1` underflow panic.
        let mut vc = VehicleController::new();
        vc.tuning.gear_ratios.clear();
        vc.engine_rpm = vc.tuning.upshift_rpm + 1000.0;
        vc.auto_shift_tick(1.0 / 60.0); // must not panic
        assert_eq!(vc.current_gear, 2, "empty gearbox: no shift, no panic");
    }

    // ── Pacejka single-axis math ────────────────────────────

    /// `calculate_force` is an ODD function of slip (F(-σ) = -F(σ)), is exactly zero at
    /// zero slip, scales LINEARLY with normal load, and its magnitude is bounded by
    /// `d·Fz` (since it is `d·Fz·sin(...)` and |sin| ≤ 1). These are the invariants the
    /// combined-slip model and friction circle rely on.
    #[test]
    fn pacejka_single_axis_is_odd_linear_in_load_and_bounded() {
        let p = PacejkaParams::default();
        let fz = 4000.0_f32;

        // Zero slip → zero force.
        assert!(p.calculate_force(0.0, fz).abs() < 1e-6, "no slip → no force");

        // Odd symmetry.
        let f_pos = p.calculate_force(0.12, fz);
        let f_neg = p.calculate_force(-0.12, fz);
        assert!(f_pos > 0.0, "positive slip → positive force, got {f_pos}");
        assert!((f_pos + f_neg).abs() < 1e-4, "force must be odd: {f_pos} vs {f_neg}");

        // Linear in normal load (the sin(...) term is load-independent).
        let f_1k = p.calculate_force(0.1, 1000.0);
        let f_3k = p.calculate_force(0.1, 3000.0);
        assert!(
            (f_3k - 3.0 * f_1k).abs() < f_1k.abs() * 1e-4,
            "force ∝ normal load: F(3000) {f_3k} ≈ 3·F(1000) {f_1k}"
        );

        // Magnitude bounded by d·Fz for any slip (|sin| ≤ 1).
        let bound = p.d * fz;
        for slip in [0.01_f32, 0.1, 0.3, 1.0, 5.0, -2.0] {
            let f = p.calculate_force(slip, fz);
            assert!(
                f.abs() <= bound + 1e-3,
                "|force| {} must not exceed d·Fz = {bound} at slip {slip}",
                f.abs()
            );
        }
    }

    /// The Lorentzian cross-slip weighting: returns exactly 1.0 at zero other-slip (so
    /// pure-axis forces are untouched), is even in the other-slip, decreases as the
    /// other axis slips more, and is floored at 0.35 (kinetic-friction residual grip) —
    /// never below, never above 1.0. Guards the `.max(0.35)` floor and the =1 base case.
    #[test]
    fn lorentzian_weighting_floor_symmetry_and_monotonicity() {
        let p = PacejkaParams::default();

        assert_eq!(p.weighting_lorentzian(0.0), 1.0, "no cross-slip → full grip preserved");

        // Even in the argument.
        assert!(
            (p.weighting_lorentzian(0.05) - p.weighting_lorentzian(-0.05)).abs() < 1e-6,
            "weighting must be even"
        );

        // Monotone decreasing (until the floor) as the other axis slips more.
        assert!(
            p.weighting_lorentzian(0.02) > p.weighting_lorentzian(0.1),
            "more cross-slip → less grip"
        );

        // Floored at 0.35, capped at 1.0, everywhere.
        for s in [0.0_f32, 0.1, 1.0, 10.0, 1000.0, -500.0] {
            let w = p.weighting_lorentzian(s);
            assert!((0.35 - 1e-9..=1.0 + 1e-9).contains(&w), "weighting {w} out of [0.35, 1] at {s}");
        }
        assert!(
            (p.weighting_lorentzian(1000.0) - 0.35).abs() < 1e-6,
            "huge cross-slip pins the weighting to the 0.35 floor"
        );
    }
}
