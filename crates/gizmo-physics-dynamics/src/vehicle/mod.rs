use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_core::components::PhysicsMaterial;
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::world::Weather;
use gizmo_physics_core::raycast::{Ray, RaycastHit};
use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};

// ============================================================
// PACEJKA MF — Kombine Slip Modeli (MF 5.2 benzeri)
// F_x = Fx_pure(σx) * Gx(σy)   — uzunlamasına weighting
// F_y = Fy_pure(σy) * Gy(σx)   — yanal weighting
// ============================================================

/// Magic-Formula tyre coefficients for **one** axis of a tyre.
///
/// Evaluates `F(σ, Fz) = d·Fz · sin(c · atan(b·σ − e·(b·σ − atan(b·σ))))`, where `σ` is the
/// slip on that axis: a dimensionless slip *ratio* for the longitudinal axis, or a slip
/// *angle* in radians for the lateral one. The same struct is used for both — nothing in it
/// records which axis it was tuned for, so a longitudinal set silently produces nonsense if
/// handed to the lateral slot.
///
/// The curve is odd in slip (`F(−σ) = −F(σ)`), exactly zero at zero slip, and strictly linear
/// in the normal load. Its magnitude is bounded by `d · Fz` for any slip whatsoever, which is
/// the invariant [`pacejka_combined`]'s friction circle is built on top of.
///
/// Coefficients are not validated. Negative or NaN values are evaluated as given.
#[derive(Clone, Debug)]
pub struct PacejkaParams {
    /// Stiffness factor B — scales slip on the way into the formula, so it sets the slope of
    /// the curve at zero slip (the tyre's cornering / longitudinal stiffness, together with
    /// `c` and `d`) and therefore how *early* the peak arrives. Larger `b` → peak at smaller
    /// slip. Dimensionless against a slip ratio, per-radian against a slip angle.
    ///
    /// The product `b·c·d·Fz` is also used directly as the zero-slip slip stiffness in the
    /// wheel-spin update, so raising this stiffens that solve as well.
    pub b: f32, // Stiffness factor
    /// Shape factor C — sets how far around the peak the `sin` is swept, i.e. how much force
    /// is retained past the peak (the "falloff" of a sliding tyre). It does not change the
    /// `d·Fz` bound: `|sin| ≤ 1` holds for every `c`.
    pub c: f32, // Shape factor
    /// Peak factor D — multiplied by the normal load before anything else, so it behaves as a
    /// dimensionless peak friction coefficient: the force this axis can produce never exceeds
    /// `d · Fz` newtons. [`pacejka_combined`] sizes its friction circle from
    /// `max(long.d, lat.d) · 1.2`.
    pub d: f32, // Peak factor (normal load ile ölçeklenir)
    /// Curvature factor E — shapes the curve immediately around the peak. At exactly `e = 1.0`
    /// the inner expression collapses to plain `atan(b·slip)`; below 1 the peak is sharper.
    /// Dimensionless.
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
    /// The pure Pacejka value for one axis ([-∞,+∞] slip → force)
    pub fn calculate_force(&self, slip: f32, normal_load: f32) -> f32 {
        let bx = self.b * slip;
        let d = self.d * normal_load;
        let inner = self.c * (bx - self.e * (bx - bx.atan())).atan();
        d * inner.sin()
    }

    /// The combined-slip weighting function (Lorentzian falloff).
    /// σ_other = the normalised slip in the perpendicular direction.
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

/// A `PacejkaLat` alias, kept for backwards compatibility.
pub type PacejkaLat = PacejkaParams;

/// Combined Pacejka: compute the longitudinal and lateral forces together,
/// keeping them inside the friction circle.
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

/// Which axle a [`Wheel`] sits on.
///
/// This is the only grouping the vehicle model has: the drive-torque split
/// ([`Drivetrain::drives`]), the brake bias, the steering overwrite and the anti-roll bar
/// pairing all key off it — see the two variants for what each does. There is no third axle,
/// so a six-wheeler has to be modelled as two of these groups.
#[derive(Clone, Debug, PartialEq)]
pub enum Axle {
    /// The steered axle. `update_vehicle` overwrites [`Wheel::steering_angle`] on these wheels
    /// every step from the Ackermann geometry, so any value written by hand is discarded, and
    /// they receive 60 % of the commanded brake torque.
    Front,
    /// The unsteered axle. `update_vehicle` never writes [`Wheel::steering_angle`] here, so a
    /// hand-set angle persists across steps (a crude rear-steer); these wheels take 40 % of
    /// the commanded brake torque.
    Rear,
}

/// The drivetrain layout: which axle the engine's torque goes to, and which wheels the engine
/// RPM is derived from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drivetrain {
    /// Front-wheel drive
    Fwd,
    /// Rear-wheel drive (the default)
    Rwd,
    /// All four wheels
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

/// One suspension corner: the strut, the tyre on the end of it, and the state
/// `update_vehicle` carries for it between steps.
///
/// Wheels are **not** rigid bodies. Each is a single raycast along its own strut axis from its
/// attachment point; the spring/damper reaction and the Pacejka tyre force are applied to the
/// chassis body, and the only wheel state that feeds back into those forces is
/// [`angular_velocity`] — spin about its own axle. ([`rotation_angle`] is integrated too, but
/// nothing in the model reads it.) There is no wheel collider, so a wheel cannot hit a wall
/// side-on.
///
/// The fields fall into three groups. Setup (geometry, spring rates, tyre coefficients) is
/// yours to set. [`drive_torque`] and [`brake_torque`] are recomputed from
/// [`VehicleController`]'s inputs on every step and cannot be driven directly; so is
/// [`steering_angle`], but only on [`Axle::Front`] wheels — on [`Axle::Rear`] wheels it is
/// left alone and a hand-set value persists. The rest are outputs — read them, don't write
/// them.
///
/// [`angular_velocity`]: Wheel::angular_velocity
/// [`rotation_angle`]: Wheel::rotation_angle
/// [`drive_torque`]: Wheel::drive_torque
/// [`brake_torque`]: Wheel::brake_torque
/// [`steering_angle`]: Wheel::steering_angle
#[derive(Clone, Debug)]
pub struct Wheel {
    /// Where the strut is anchored to the chassis, in metres, in chassis-local space, relative
    /// to the `Transform` origin. The suspension ray starts here (offset 0.5 m back along the
    /// ray so the origin cannot end up buried in the ground) and the spring force is applied
    /// here.
    pub attachment_local_pos: Vec3,
    /// Direction the strut pushes the wheel, in chassis-local space; `(0, -1, 0)` — straight
    /// down — by default. Re-normalised every step, so its length is irrelevant, but a zero
    /// or non-finite vector normalises to NaN and poisons the whole step.
    pub direction_local: Vec3,
    /// Which axle this corner belongs to; see [`Axle`] for what keys off it.
    ///
    /// Free to change between steps, and it takes effect on the next one: a wheel moved to
    /// [`Axle::Front`] starts having its [`steering_angle`](Wheel::steering_angle) overwritten
    /// from the Ackermann geometry, and one moved off a driven axle stops receiving drive
    /// torque. Nothing requires both axles to be populated — with a [`Drivetrain`] whose axle
    /// carries no wheels, no wheel is driven at all and
    /// [`engine_rpm`](VehicleController::engine_rpm) sits pinned at
    /// [`idle_rpm`](VehicleTuning::idle_rpm), because engine speed is derived from the mean
    /// spin of the driven wheels.
    pub axle_type: Axle,
    /// Which side of its axle this wheel is on. Chooses the sign of the half-track term in the
    /// Ackermann geometry (the inner wheel of a turn steers more) and the sign of the
    /// anti-roll bar force.
    ///
    /// `update_vehicle` buckets wheels by `(axle_type, is_left)` into exactly four slots to
    /// compute the anti-roll compression difference. The four slots are re-zeroed at the top of
    /// every step, so two wheels sharing a bucket silently overwrite one another, and an axle
    /// with no `is_left = false` wheel gets its difference measured against zero rather than
    /// against a real corner — so each axle wants exactly one of each.
    pub is_left: bool,

    /// Loaded tyre radius in metres. It is threaded through most of the step: the reach of the
    /// suspension ray; the subtraction that turns the ray's hit distance into a strut length;
    /// the wheel's rotational inertia (`0.5 · wheel_mass · radius²`, a solid disc) and the `r²`
    /// term of the implicit spin update; the conversion between spin and road speed
    /// (`angular_velocity · radius`); the tyre reaction torque (`long_force · radius`); and the
    /// traction-control spin clamp, where it is a divisor.
    ///
    /// A radius of 0 does not divide by zero — the inertia is floored at `1e-6` kg·m² — but the
    /// traction-control spin clamp is skipped entirely below `1e-4` m.
    pub radius: f32,
    /// Strut length in metres with no load on the spring: the distance from
    /// [`attachment_local_pos`](Wheel::attachment_local_pos) to the wheel centre at which the
    /// spring produces zero force. This is the spring's zero point, not a travel limit —
    /// compression is `suspension_rest_length − suspension_length` and may go negative
    /// (the strut extending) when the wheel drops into a dip.
    pub suspension_rest_length: f32,
    /// Travel in metres allowed either side of the rest length; `suspension_length` is clamped
    /// to `rest ± max_travel`. The last 10 % of the compression stroke engages a bump stop
    /// eight times stiffer than the main spring, so the effective bottom-out is slightly
    /// before the clamp. Floored at 1 mm where it is used as a divisor.
    pub suspension_max_travel: f32,
    /// Spring rate in newtons per metre of compression. With gravity supplying the load, the
    /// static ride height is roughly `corner_weight / suspension_stiffness` below the rest
    /// length, so a rate far too low simply parks the car on its bump stops.
    pub suspension_stiffness: f32,
    /// Damper rate in N·s/m for the **compression** stroke. The rebound stroke uses 2.5× this
    /// value, and the resulting damper force is floored at `−0.5 ×` the spring force so a
    /// stiff rebound can never cancel the static spring load: if it could, the normal load
    /// would reach zero, and with it every Pacejka tyre force (which is proportional to `Fz`).
    pub suspension_damping: f32,

    /// Magic-Formula coefficients for the longitudinal (drive/brake) axis, evaluated against
    /// the slip ratio `(angular_velocity · radius − v_forward) / max(|v_forward|, 0.5)`.
    /// `b · c · d · Fz` from this set is additionally used as the slip stiffness that makes
    /// the wheel-spin update implicit, so it affects spin stability as well as grip.
    pub pacejka_long: PacejkaParams,
    /// Magic-Formula coefficients for the lateral (cornering) axis, evaluated against the slip
    /// angle in radians. Independent of [`pacejka_long`](Wheel::pacejka_long) up to the shared
    /// friction circle, which uses the larger of the two `d` values.
    pub pacejka_lat: PacejkaLat,
    /// Mass of the wheel + tyre in kilograms, used **only** for its own rotational inertia
    /// (`0.5 · m · r²`). `update_vehicle` never adds it to the chassis mass and it produces no
    /// unsprung-mass effects; the whole car's mass lives on the chassis `RigidBody`.
    pub wheel_mass: f32,

    // Inputs (set by VehicleController each frame)
    /// Steer angle in radians about the chassis up axis; positive steers the wheel to the left
    /// under this engine's `−Z` forward / `+X` right convention.
    ///
    /// Recomputed every step for [`Axle::Front`] wheels from the Ackermann geometry, so
    /// writing it there has no effect — steer via [`VehicleController::steering_input`].
    /// [`Axle::Rear`] wheels are never written, so a value set here does persist.
    pub steering_angle: f32,
    /// Drive torque delivered to this wheel in newton-metres, signed (negative in reverse).
    ///
    /// Recomputed every step: a driven wheel gets the engine torque times the total ratio,
    /// divided evenly among the driven wheels; an undriven wheel gets exactly 0. Writing it
    /// yourself is overwritten before it is used.
    pub drive_torque: f32,
    /// Brake torque available at this wheel in newton-metres. The sign is chosen at use to
    /// oppose the current spin, so this is meant to be a magnitude — but nothing enforces that.
    ///
    /// Recomputed every step as `brake_input · max_brake_torque · bias`, with `bias` 0.6 on the
    /// front axle and 0.4 on the rear, and [`brake_input`](VehicleController::brake_input) is
    /// not clamped: a negative pedal value carries straight through to a negative torque here,
    /// and both the `brake_torque < 1.0` coast check and the wheel-lock threshold use the value
    /// as-is, without an `abs()`. Writing it yourself is overwritten.
    pub brake_torque: f32,

    // State
    /// Output: whether this step's suspension ray found a non-trigger collider within reach.
    ///
    /// While false the tyre produces no force whatsoever — an airborne wheel only spins up
    /// under drive/brake torque and bleeds off through a 2 s⁻¹ exponential drag. That spin
    /// drag is deliberately *not* applied on the ground, where the tyre model owns the wheel.
    pub is_grounded: bool,
    /// Output: the contact the suspension ray found this step, or `None` when airborne.
    ///
    /// `point` is the world-space contact where the tyre force is applied. Note `distance` is
    /// measured from a ray origin offset 0.5 m back along the ray from the attachment point,
    /// and the tyre sits on the end of the strut, so it overstates the strut length by
    /// `0.5 + radius` before the travel clamp — 0.85 m with the default 0.35 m wheel. Use
    /// [`suspension_length`](Wheel::suspension_length) for the strut, not this.
    pub ground_hit: Option<RaycastHit>,
    /// Output: current strut length in metres, clamped to
    /// `suspension_rest_length ± suspension_max_travel`. Snapped back to the rest length (and
    /// the force zeroed) the moment the wheel leaves the ground, so it never carries a stale
    /// compression into the next landing.
    pub suspension_length: f32,
    /// Output: accumulated spin angle in radians, wrapped into `(−τ, τ)` by float remainder —
    /// it keeps the sign of the spin, so a reversing wheel winds negative.
    ///
    /// Purely cosmetic: `update_vehicle` integrates and wraps it, and no force in the model
    /// reads it back.
    pub rotation_angle: f32,
    /// Wheel spin in rad/s about its own axle, positive when the wheel is rolling the car
    /// forwards (`angular_velocity · radius` is compared directly against forward road speed).
    ///
    /// The only wheel state whose integration feeds back into the forces, and it is integrated
    /// *linearly-implicitly* —
    /// the tyre's slip stiffness is folded into the denominator — because an explicit step is
    /// unstable for a light, free-rolling wheel. Driven wheels additionally see the flywheel
    /// inertia reflected through the gearbox. Snapped to exactly 0 once it falls below
    /// 0.05 rad/s while both drive and brake torque are under 1 N·m, so a coasting car's
    /// wheels actually come to rest instead of creeping forever.
    pub angular_velocity: f32,
    /// Output: the strut force in newtons, applied to the chassis along `−direction_local`
    /// rotated into world space — straight up only while the chassis is level and the strut
    /// points straight down. Clamped at `≥ 0`, so a strut can push the chassis away from the
    /// ground but never pull it back. Sum of spring, damper, bump stop and anti-roll bar.
    ///
    /// This doubles as the tyre's normal load `Fz` in the Pacejka model, so a corner at 0 N
    /// produces no grip at all in either axis. Zeroed while airborne.
    pub suspension_force: f32,
    /// The dynamic friction of the ground being touched (for the grip multiplier). Captured
    /// from the PhysicsMaterial of the collider hit by the first-pass raycast; falls back to
    /// ASPHALT while airborne.
    /// grip_mult = surface_friction / ASPHALT.dynamic_friction → ice/sand/asphalt give real
    /// grip.
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

/// A `pacejka` getter, kept for backwards compatibility.
impl Wheel {
    /// Compatibility accessor from before the tyre model was split into separate longitudinal
    /// and lateral coefficient sets.
    ///
    /// Returns the **longitudinal** set only. There is no matching accessor for
    /// [`Wheel::pacejka_lat`], so code that reads this and assumes it describes the whole tyre
    /// is reading half of it. Prefer the fields.
    #[inline]
    pub fn pacejka(&self) -> &PacejkaParams {
        &self.pacejka_long
    }
}

/// Aerodinamik paket
#[derive(Clone, Debug)]
pub struct AeroPackage {
    /// Dimensionless drag coefficient Cd. Drag is `½ · ρ · Cd · A · v²` with ρ fixed at
    /// 1.225 kg/m³ (sea-level air; there is no altitude model), directed against the chassis'
    /// **world** velocity — not against its forward axis, so it also resists sideways slides.
    /// Below 0.1 m/s the direction is zeroed rather than normalising a near-zero vector, which
    /// is why a stationary car feels no drag at all. Never scaled by ground effect.
    pub drag_coefficient: f32,     // Cd
    /// Dimensionless lift coefficient Cl, applied along the chassis' **local up** axis. A
    /// negative value is therefore downforce, and the default is negative. Because the axis is
    /// local rather than world, an inverted car gets its downforce pointing at the sky.
    ///
    /// Unlike drag, this term is multiplied by the ground-effect factor.
    pub lift_coefficient: f32,     // Cl (negatif = downforce)
    /// Reference area in square metres, shared by both the drag and the lift term. There is no
    /// separate wing area — scaling this scales grip and top speed together.
    pub frontal_area: f32,         // m²
    /// Point the combined drag + lift force is applied at, in metres in chassis-local space,
    /// measured from the `Transform` origin. Its offset from the centre of mass is what turns
    /// aero into a pitch/yaw moment rather than a pure translation; land it exactly on the
    /// centre of mass and aero produces no moment at all.
    pub center_of_pressure: Vec3,  // CoM'dan offset (yerel)
    /// Nominal ride height in metres at which ground effect would begin.
    ///
    /// **Has no effect for any value ≥ 1 mm.** `update_vehicle` does not measure real
    /// ride height; it synthesises a clearance of `ground_effect_height · (1 − compression)`
    /// from the mean suspension compression fraction and then divides by
    /// `ground_effect_height` again, so for any value ≥ 1 mm it cancels exactly and the ramp
    /// is driven purely by suspension travel. It is still read, so it must stay finite and
    /// non-negative.
    pub ground_effect_height: f32, // Bu yüksekliğin altında zemin etkisi devreye girer
    /// Factor the lift term is multiplied by when the suspension is fully compressed; `1.0`
    /// disables ground effect. The multiplier ramps linearly from 1.0 at full droop to this
    /// value at full compression, averaged over the **grounded** wheels only — with no wheel
    /// on the ground it is 1.0, so an airborne car loses its ground effect but keeps its
    /// static downforce.
    ///
    /// This is a feedback loop: more downforce compresses the suspension, which raises the
    /// multiplier. It is bounded only by the suspension's travel clamp. Values below 1.0 are
    /// not rejected and invert the effect.
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

/// Static setup for a vehicle: engine, gearbox, the chassis geometry the steering and
/// anti-roll models need, and the aero package.
///
/// Nothing here is written by the simulation — `update_vehicle` only reads it — so a tune can
/// be built once and cloned onto every car of a type. The geometry fields ([`wheelbase`],
/// [`track_width`]) are *steering-model* numbers and are never cross-checked against where the
/// wheels are actually attached; the two can disagree and the model will not notice.
///
/// [`wheelbase`]: VehicleTuning::wheelbase
/// [`track_width`]: VehicleTuning::track_width
#[derive(Clone, Debug)]
pub struct VehicleTuning {
    /// Lower bound of the engine speed band, in rpm. The engine speed is clamped into
    /// `[idle_rpm, max_rpm]` every step, so it never reads below this even with the car
    /// stationary and the clutch effectively locked.
    ///
    /// Must not exceed [`max_rpm`](VehicleTuning::max_rpm): the clamp panics on an inverted
    /// range.
    pub idle_rpm: f32,
    /// Upper bound (redline) of the engine speed band, in rpm, and the top of the range the
    /// built-in bell torque curve is parameterised over.
    ///
    /// This is a clamp, not a limiter — there is no fuel cut and no over-rev damage; the
    /// engine speed simply saturates here while the wheels keep accelerating.
    pub max_rpm: f32,
    /// [0]=reverse, [1]=neutral, [2..]=forward gears
    pub gear_ratios: Vec<f32>,
    /// Final-drive (differential) ratio, multiplied onto the selected gear ratio.
    ///
    /// The product runs the transmission in both directions — wheel torque is
    /// `engine_torque · |ratio|`, engine speed is `wheel_rpm · |ratio|` — and its **square**
    /// scales the flywheel inertia reflected onto each driven wheel. So it trades acceleration
    /// against top speed and changes throttle response at the same time. A value of 0 leaves
    /// the car with no drive at all.
    pub final_drive_ratio: f32,
    /// Automatic gearbox: the upshift RPM threshold
    pub upshift_rpm: f32,
    /// Automatic gearbox: the downshift RPM threshold
    pub downshift_rpm: f32,
    /// Distance between the front and rear axles in metres. Sets the turn radius
    /// (`wheelbase / tan(steer_angle)`) and, with the track width, the per-wheel Ackermann
    /// angles. It does NOT make the car turn more lazily the way a real longer wheelbase
    /// would: the chassis yaw comes from tyre forces at [`Wheel::attachment_local_pos`], and
    /// raising this only pulls both front angles toward the nominal steer angle (measured at
    /// track 1.6, steer 0.4: L=1.5 gives 0.500/0.332, L=8.0 gives 0.416/0.385).
    ///
    /// Steering geometry only. The wheels' real positions come from
    /// [`Wheel::attachment_local_pos`] and are never reconciled with this.
    pub wheelbase: f32,
    /// **Full** left-to-right wheel spacing in metres; the Ackermann geometry uses half of it
    /// to offset the inner and outer wheels from the turn centre. At 0 both front wheels get
    /// the same angle (no Ackermann correction).
    ///
    /// Steering geometry only — it does **not** describe the anti-roll bar's lever arm, and is
    /// never reconciled with [`Wheel::attachment_local_pos`].
    pub track_width: f32,
    /// Anti-roll (sway) bar rate in newtons per metre, applied on both axles.
    ///
    /// Multiplied by the axle's left-minus-right suspension *compression* difference in metres;
    /// the result is added to the more-compressed corner's suspension force and subtracted from
    /// the other. 0 disables the bar; raising it reduces body roll.
    ///
    /// The subtraction is not guaranteed to match the addition, so this is not a strictly
    /// zero-sum load transfer. Each corner's total is clamped at `≥ 0`, which swallows whatever
    /// the bar tried to remove past that point, and the bar term is only computed for a
    /// *grounded* corner — with one wheel of an axle in the air the other keeps its share and
    /// nothing cancels it.
    pub anti_roll_stiffness: f32,
    /// Peak crankshaft torque in newton-metres for the built-in bell curve, reached 40 % of the
    /// way from [`idle_rpm`](VehicleTuning::idle_rpm) to [`max_rpm`](VehicleTuning::max_rpm)
    /// and never falling below 5 % of it anywhere in the band.
    ///
    /// **Ignored entirely** when [`torque_curve`](VehicleTuning::torque_curve) is non-empty.
    pub max_engine_torque: f32,
    /// Brake torque in newton-metres commanded at full brake input, **per wheel and before the
    /// front/rear bias** — a front wheel receives 0.6× this and a rear wheel 0.4×. It is not a
    /// total for the car, so it does not need dividing by the wheel count.
    pub max_brake_torque: f32,
    /// Drag, downforce and ground-effect parameters. Aero is applied to the chassis every step
    /// regardless of whether any wheel is grounded.
    pub aero: AeroPackage,
    /// A measured torque curve: `(rpm, N·m)` pairs, in **ascending rpm** order.
    ///
    /// Left empty (the default), the engine uses its own parametric bell curve scaled by
    /// [`Self::max_engine_torque`] — so existing behaviour is preserved bit for bit. When it is
    /// populated the bell curve is bypassed entirely and torque is read from these points by
    /// linear interpolation.
    ///
    /// Why it exists: the bell curve makes every vehicle peak at the **same rpm**
    /// (`ratio = 0.4`). If a real vehicle's curve is available, that throws away the thing that
    /// tells cars apart — two different engines with the same peak torque drive identically.
    /// For example, NFSU2's 240SX record peaks at 4675 rpm where the bell curve peaks at 3280;
    /// a difference of 1,400 rpm and an entirely different fall-off.
    ///
    /// Out-of-range rpms are **clamped to the ends** rather than falling to zero: an engine
    /// braking the car once past the redline is not real, and the bell curve's `0.05` floor was
    /// there for exactly that.
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

/// A driveable vehicle: its suspension corners, its tune, the driver's inputs, and the
/// drivetrain state carried between steps.
///
/// A `Component`. `update_vehicle` reads the input fields, applies every resulting force to
/// the single chassis body it is handed, and writes the output fields back. There are no child
/// entities — the wheels exist only as entries in [`wheels`](VehicleController::wheels).
///
/// The three pedal/wheel inputs are yours to set each frame; `engine_rpm`,
/// `current_speed_kmh` and (with auto-shift on) `current_gear` are outputs recomputed from
/// the simulation and writing them achieves nothing.
#[derive(Clone, Debug)]
pub struct VehicleController {
    /// Every suspension corner. A wheel's *role* comes from its [`Wheel::axle_type`] and
    /// [`Wheel::is_left`] rather than from its index, but the order is still load-bearing:
    /// `update_vehicle` accumulates each corner's suspension and tyre force into the chassis
    /// velocity as it walks the vector, and the next corner reads that already-updated velocity
    /// back when it computes its own contact-point velocity. Reordering the vector therefore
    /// changes the result.
    ///
    /// An empty list is legal and yields a chassis with aero and no traction whatsoever.
    /// Drive torque is divided evenly among the *driven* wheels, so adding a driven wheel
    /// weakens each of the others rather than adding power.
    pub wheels: Vec<Wheel>,
    /// This vehicle's own copy of the setup — owned by value, so editing it affects only this
    /// vehicle and takes effect on the very next step.
    ///
    /// Swapping a whole tune in mid-run resets none of the state around it:
    /// [`current_gear`](VehicleController::current_gear) keeps its value and is re-indexed into
    /// the new [`gear_ratios`](VehicleTuning::gear_ratios), reading as 0.0 — neutral — if it is
    /// now out of range.
    pub tuning: VehicleTuning,

    /// Accelerator, 0..1. Only its magnitude is used, so a negative value is **not** reverse —
    /// use [`set_reverse`](VehicleController::set_reverse) for that. A magnitude above
    /// `f32::EPSILON` counts as driver input and wakes a sleeping chassis. Not clamped: values above 1 scale the
    /// engine torque past its peak.
    pub throttle_input: f32, // 0..1
    /// Brake pedal, 0..1. Scales [`VehicleTuning::max_brake_torque`] through the front/rear
    /// bias. Above 0.01 it also enables the wheel-lock snap, which zeroes a wheel's spin when
    /// one step of brake torque would otherwise reverse it — that is what stops the wheels
    /// juddering backwards under heavy braking. Counts as driver input for the wake check.
    pub brake_input: f32,    // 0..1
    /// Steering, −1..1, multiplied by [`max_steering_angle`] to give the nominal steer angle.
    /// Positive steers left under this engine's `−Z` forward / `+X` right convention.
    ///
    /// Counts as driver input for the wake check. Not clamped, and there is no steering rate
    /// limit — snapping this from 0 to 1 in one frame snaps the front wheels with it.
    ///
    /// [`max_steering_angle`]: VehicleController::max_steering_angle
    pub steering_input: f32, // -1..1
    /// Whether reverse is currently engaged. Set it through
    /// [`set_reverse`](VehicleController::set_reverse) rather than directly — that is what
    /// keeps [`current_gear`](VehicleController::current_gear) consistent with the flag.
    /// While true, automatic shifting is suppressed entirely.
    pub reverse_input: bool,
    /// Whether [`auto_shift_tick`](VehicleController::auto_shift_tick) may change gear.
    /// `update_vehicle` calls that tick unconditionally, so clearing this holds the gear;
    /// with it false, `current_gear` is left entirely to the caller. Not the only way in —
    /// `auto_shift_tick` also returns early while [`Self::reverse_input`] is set, and when
    /// `gear_ratios` is empty.
    pub auto_shift: bool, // Otomatik vites etkin mi?

    /// Index into [`VehicleTuning::gear_ratios`]: 0 is reverse, 1 is neutral, 2 and up are the
    /// forward gears.
    ///
    /// An out-of-range index does not panic — the ratio reads as 0.0, which behaves as
    /// neutral. Automatic *downshifting* is floored at 2, so it never drops the car into
    /// neutral or reverse; the upshift branch has no such floor and is guarded only against
    /// running past the top gear, so a hand-set gear of 0 or 1 with
    /// [`reverse_input`](VehicleController::reverse_input) clear can be auto-upshifted into 1
    /// or 2.
    pub current_gear: usize, // gear_ratios index
    /// Steer angle in radians at full lock (default ≈ 0.52 rad, 30°). Bounds the *nominal*
    /// angle only: the Ackermann split then gives the inner wheel of a turn a slightly larger
    /// angle than this, and the outer wheel a smaller one.
    pub max_steering_angle: f32,
    /// Seconds remaining before automatic shifting is allowed again; counted down by
    /// `auto_shift_tick` and armed by it at 0.4 s after an upshift and 0.3 s after a
    /// downshift. It exists to stop the box hunting between two gears.
    ///
    /// The decrement sits AFTER that tick's early returns, so a cooldown armed just before
    /// [`Self::auto_shift`] is cleared or [`Self::reverse_input`] is set stops counting down
    /// and stays armed until automatic shifting is re-enabled.
    ///
    /// Manual gear changes do not arm it, and it does not block them.
    pub shift_cooldown: f32, // Vites değişimi sonrası bekleme süresi (s)

    /// Output: engine speed in rpm.
    ///
    /// Recomputed from scratch every step out of the mean spin of the **driven** wheels times
    /// the total ratio, then clamped to `[idle_rpm, max_rpm]` — it is not integrated, and the
    /// clutch is permanently locked, so there is no free-revving engine here and no neutral
    /// blip. With no driven wheels it sits at idle.
    pub engine_rpm: f32,
    /// Output: **signed** forward speed in km/h — the chassis velocity projected onto its own
    /// `−Z` axis. It goes negative in reverse and while sliding backwards, and reads near zero
    /// in a pure sideways slide, so it is a speedometer reading rather than a speed over
    /// ground (`Velocity::linear.length()` is that).
    pub current_speed_kmh: f32,
    /// Combined engine and flywheel rotational inertia in kg·m².
    ///
    /// Reflected onto each **driven** wheel through the gearbox as
    /// `flywheel_inertia · total_ratio²`, which in a low gear dominates the wheel's own
    /// `0.5·m·r²` by more than an order of magnitude. That is what stops a driven wheel from
    /// snapping straight to the redline off the line; in a tall gear the ratio shrinks and the
    /// engine picks up more freely. Undriven wheels never see it.
    pub flywheel_inertia: f32,   // kg·m²
    /// The drivetrain layout (FWD/RWD/AWD) — torque distribution and RPM derivation follow it.
    pub drivetrain: Drivetrain,
}

#[cfg(feature = "ecs")]
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
    /// A controller with **no wheels**, default tuning, first forward gear (index 2) selected
    /// and the engine at idle. Identical to `Default::default()`.
    ///
    /// Stepping it in this state is harmless but pointless: with no wheels there is no
    /// suspension and no traction, only aero. Add corners with
    /// [`add_wheel`](VehicleController::add_wheel) first.
    pub fn new() -> Self {
        Self::default()
    }
    /// Appends a suspension corner.
    ///
    /// Performs no validation. Nothing checks that each axle ends up with exactly one
    /// `is_left = true` and one `is_left = false` wheel — which the anti-roll bar assumes — nor
    /// that the attachment points agree with [`VehicleTuning::wheelbase`] and
    /// [`VehicleTuning::track_width`], nor that the chosen drivetrain has any wheels to drive.
    pub fn add_wheel(&mut self, wheel: Wheel) {
        self.wheels.push(wheel);
    }

    /// The engine torque curve — from the measured points if there are any, otherwise from the
    /// parametric bell curve.
    ///
    /// Both paths scale linearly with throttle and neither ever returns a negative value: an
    /// engine past the redline *braking* the car would not be physics, it would be a bug.
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

    /// Reads the torque for an rpm out of the `(rpm, N·m)` points by linear interpolation.
    ///
    /// Outside the ends it **clamps** (no extrapolation): below the first point the first value,
    /// above the last point the last one. Extrapolating would take a falling curve negative at a
    /// high enough rpm and the engine would start braking the car — which is what the bell
    /// curve's `0.05` floor protected against, and the same protection has to hold here.
    ///
    /// The points are assumed to be in ascending order. If they are not, the result is
    /// meaningless but it neither panics nor goes negative; sorting is the caller's job, because
    /// doing it here every frame would needlessly double the cost of using a measured curve.
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

    /// Engages or disengages reverse, keeping [`current_gear`](VehicleController::current_gear)
    /// consistent with [`reverse_input`](VehicleController::reverse_input).
    ///
    /// Engaging always selects gear 0, the reverse ratio. Disengaging selects gear 2, the first
    /// forward gear — but **only** when the box is currently in gear 0; called with `false`
    /// while already in a forward gear it leaves that gear alone. The forward gear that was
    /// selected before reverse is not remembered, so coming out of reverse always lands in 2.
    ///
    /// Safe to call every frame from a held input: it is idempotent, and it only emits a log
    /// line when the flag actually changes.
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

    /// The automatic gearbox — upshift/downshift against the RPM thresholds
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

// The wheel's view of the scene — one ray, one nearest surface — and its two answers
// (linear scan / broadphase scene query).
mod ground_query;
pub use ground_query::{ColliderListQuery, WheelGroundHit, WheelGroundQuery};

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
