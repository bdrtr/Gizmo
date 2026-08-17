//! The rigid-body component: body type, mass properties, damping, axis locks and the
//! sleep bookkeeping that goes with them.
//!
//! Units in this module are SI throughout — metres, kilograms, seconds, radians, and
//! therefore newtons for force and N·m for torque. Vectors are world-space unless a doc
//! comment explicitly says *body-local*; the two frames are not interchangeable for
//! [`RigidBody::local_inertia`] / [`RigidBody::center_of_mass`], which are body-local,
//! versus the force and torque accumulators, which are world-space.
//!
//! Everything stored here is plain `f32`/`bool` state and is serialised with the
//! component, so it takes part in the engine's *same-platform* replay and rollback
//! bit-equality guarantee. Cross-platform bit-exactness is out of scope.

use gizmo_math::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};
#[cfg(feature = "reflect")]
use bevy_reflect::Reflect;

use super::Velocity;
use gizmo_physics_core::{Collider, ColliderShape};

/// How the simulation is allowed to move a body.
///
/// The variant decides both which integration phases touch the body and its effective mass
/// — see [`RigidBody::inv_mass`] for the latter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub enum BodyType {
    /// Fully simulated: gravity, the force/torque accumulators, damping and contact
    /// impulses all apply, and the body is a sleep candidate.
    Dynamic,   // Fully simulated
    /// Driven by user code, never by the solver. Velocity integration is skipped
    /// entirely (no gravity, no accumulated forces, no damping), but position
    /// integration still advances the transform from whatever [`Velocity`] you write,
    /// so a kinematic body pushes dynamic bodies while nothing can push it back.
    /// Never sleeps — see [`RigidBody::can_sleep`].
    Kinematic, // Moved by user, affects others
    /// Never moved by the engine: position integration returns early, so writing to a
    /// static body's [`Velocity`] has no effect. Moving one means writing its
    /// `Transform` yourself. [`RigidBody::new_static`] additionally parks it as
    /// sleeping with all six axes locked.
    Static,    // Never moves
}

/// Mass properties and motion policy of one simulated body.
///
/// Pair it with a [`Velocity`], a `Transform` and a `Collider`; friction and restitution
/// are *not* here, they come from the collider's material (see [`RigidBody::new`]).
/// Fields are public and unvalidated — nothing recomputes derived state when you assign
/// to them, so after changing [`mass`](Self::mass) or the collider shape you must refresh
/// [`local_inertia`](Self::local_inertia) yourself via
/// [`update_inertia_from_collider`](Self::update_inertia_from_collider).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "reflect", derive(Reflect))]
pub struct RigidBody {
    /// Which integration and mass rules apply; see [`BodyType`].
    ///
    /// Plain data — flipping it mid-simulation is allowed and takes effect on the next
    /// step, but it neither recomputes mass properties nor clears
    /// [`is_sleeping`](Self::is_sleeping).
    pub body_type: BodyType,
    /// Mass in kilograms.
    ///
    /// `0.0` means *infinite mass* (zero inverse mass): a body with it cannot be
    /// accelerated by forces or contacts, which is why the static/kinematic constructors
    /// store zero. It is not the whole story for the solver — see
    /// [`inv_mass`](Self::inv_mass) — but the `calculate_*_inertia` helpers do read this
    /// field whatever the body type is.
    /// Never validated or clamped: a negative mass is stored verbatim and yields a
    /// negative inverse mass the solver is not designed for. Changing it does not
    /// rescale [`local_inertia`](Self::local_inertia).
    pub mass: f32,
    /// Exponential decay rate of linear velocity, per second: the integrator multiplies
    /// linear velocity by `exp(-linear_damping · dt)` each step, so the loss is
    /// frame-rate independent. `0.0` disables it; the default is a token `0.01`.
    ///
    /// This is a crude velocity-*linear* energy sink, not aerodynamic drag — for the
    /// physical v² law use [`drag_coefficient`](Self::drag_coefficient) /
    /// [`with_air_drag`](Self::with_air_drag). A NaN here is rejected by velocity
    /// integration (the body is left untouched and an error is returned) rather than
    /// silently poisoning the velocity.
    pub linear_damping: f32,
    /// Same exponential law as [`linear_damping`](Self::linear_damping) but applied to
    /// angular velocity (rad/s); default `0.05`.
    ///
    /// It scales all three components by the same factor, so it cannot damp spin about
    /// one axis only — use the rotation locks for that.
    pub angular_damping: f32,
    /// Aerodynamic drag coefficient (Cd). When it and `drag_area` are both >0 the integrator
    /// applies physical air resistance: F = ½·ρ·Cd·A·|v|², OPPOSING the velocity. 0 = off
    /// (default). Where `linear_damping` (exponential decay, linear in velocity) is a crude
    /// proxy, this is real v² drag → a body settles at its natural terminal velocity
    /// (v_term = √(2·m·g / (ρ·Cd·A))). Air density comes from `Integrator::air_density`.
    /// `#[serde(default)]`: scenes saved before this field existed load as `0.0` (off).
    #[serde(default)]
    pub drag_coefficient: f32,
    /// The reference (frontal) area exposed to drag, in m². Air resistance is active when this
    /// and `drag_coefficient` are both >0.
    #[serde(default)]
    pub drag_area: f32,
    /// When `false` the gravity term is skipped for this body only; the world's gravity
    /// vector itself is unchanged for everyone else.
    ///
    /// This is not "disable physics": the body still collides and is still accelerated by
    /// contacts, joints and its own accumulators. Irrelevant for non-dynamic bodies,
    /// which never run velocity integration at all.
    pub use_gravity: bool,
    /// Sleep flag. While it is set, **both** velocity and position integration skip the
    /// body, so writing a velocity or an accumulator on a sleeping body has no visible
    /// effect and the write is not consumed either — call [`wake_up`](Self::wake_up)
    /// first (that is why the world's impulse helpers take `&mut RigidBody`).
    ///
    /// Set by [`update_sleep_state`](Self::update_sleep_state) after a run of quiet
    /// steps and cleared by it the moment the body moves again.
    /// [`new_static`](Self::new_static) starts it `true`, since a static body is
    /// permanently "asleep".
    pub is_sleeping: bool,
    /// Opt into continuous collision detection for this body: a fast or thin body that
    /// would otherwise pass through geometry between two discrete steps.
    ///
    /// It has a cost — among other things the broadphase AABB is fattened by the
    /// expected motion when the body is inserted — so it is off by default for dynamic
    /// bodies and on for [`new_kinematic`](Self::new_kinematic) ones. It reduces
    /// tunnelling; it is not a guarantee that tunnelling cannot happen.
    pub ccd_enabled: bool,
    /// Diagonal of the inertia tensor in the **body-local** frame, kg·m².
    ///
    /// Only the diagonal is stored, i.e. the local axes are assumed to be the principal
    /// axes and all products of inertia are zero. The default `Vec3::splat(1.0)` is a
    /// placeholder derived from neither [`mass`](Self::mass) nor any collider, so a body
    /// built by hand spins wrongly until you call
    /// [`update_inertia_from_collider`](Self::update_inertia_from_collider) or one of the
    /// `calculate_*_inertia` helpers.
    ///
    /// A zero component means "no rotation about that local axis" rather than a division
    /// by zero, and an infinite component collapses to the same thing — see
    /// [`inv_local_inertia`](Self::inv_local_inertia).
    pub local_inertia: Vec3,
    /// Freezes angular velocity about the **world** X axis (not a body-local axis: the
    /// locked axis does not follow the body as it turns).
    ///
    /// Two mechanisms enforce it. [`enforce_locks`](Self::enforce_locks) zeroes
    /// `Velocity::angular.x`, and [`inv_world_inertia_tensor`](Self::inv_world_inertia_tensor)
    /// zeroes the X row *and* column of the inverse inertia so solver impulses cannot
    /// spin the body about it either. It removes rotation *rate* only — a body already
    /// tilted stays tilted.
    pub lock_rotation_x: bool,
    /// World **Y** axis counterpart of [`lock_rotation_x`](Self::lock_rotation_x): with
    /// the engine's default −Y gravity this is the yaw axis.
    pub lock_rotation_y: bool,
    /// World **Z** axis counterpart of [`lock_rotation_x`](Self::lock_rotation_x). Set
    /// together with the other two — see [`lock_rotation`](Self::lock_rotation) — this is
    /// what keeps an upright body (character capsule, standing prop) from toppling.
    pub lock_rotation_z: bool,
    /// Pins the body on the **world** X axis by zeroing `Velocity::linear.x` in
    /// [`enforce_locks`](Self::enforce_locks).
    ///
    /// A velocity-level constraint only: it never repositions the transform, so a body
    /// spawned off the intended plane simply stays where it is, and code that writes
    /// `Transform::position` directly is unaffected.
    pub lock_translation_x: bool,
    /// World **Y** axis counterpart of
    /// [`lock_translation_x`](Self::lock_translation_x).
    ///
    /// It does not disable gravity: each step still adds `g·dt` and the lock zeroes it
    /// again at the end of velocity integration, so the body neither falls nor
    /// accumulates downward speed. [`use_gravity`](Self::use_gravity) expresses the same
    /// end state more directly.
    pub lock_translation_y: bool,
    /// World **Z** axis counterpart of
    /// [`lock_translation_x`](Self::lock_translation_x); combined with locked X/Y
    /// rotation it is the usual 2.5-D constraint, confining motion to the world XY plane.
    pub lock_translation_z: bool,
    /// Number of consecutive integration steps in which [`can_sleep`](Self::can_sleep)
    /// held. [`update_sleep_state`](Self::update_sleep_state) increments it and sets
    /// [`is_sleeping`](Self::is_sleeping) once it reaches 60; any step that fails the
    /// test resets it to 0, as does [`wake_up`](Self::wake_up).
    ///
    /// The unit is *physics steps*, not rendered frames or seconds: a world that runs
    /// several fixed sub-steps per frame advances this several times per frame and
    /// therefore falls asleep proportionally sooner in wall-clock terms. It also stops
    /// advancing while the body sleeps, since a sleeping body is never integrated.
    pub sleep_counter: u32, // Frames below sleep threshold
    /// Centre of mass as an offset from the transform's origin, in the **body-local**
    /// frame, metres. `Vec3::ZERO` (the default) means the origin *is* the centre of
    /// mass.
    ///
    /// The world-space centre is `transform.position + transform.rotation *
    /// center_of_mass`, and that is the point contact and joint lever arms are measured
    /// about — measuring them about the transform origin instead was a real bug in this
    /// engine, and the two only agree while this field is zero.
    ///
    /// Position integration is *not* re-centred on it: `Transform::position` still
    /// advances by the linear velocity and the rotation is applied to the transform as
    /// it stands. Note also that
    /// [`update_inertia_from_collider`](Self::update_inertia_from_collider) overwrites
    /// this field for compound colliders.
    pub center_of_mass: Vec3,
    /// Optional per-body fracture trigger, compared against the largest solved **normal
    /// impulse** in a contact manifold this body takes part in — an impulse, not a force,
    /// so its magnitude depends on mass, `dt` and the sub-step count; tune it empirically
    /// rather than from a newton figure.
    ///
    /// `None` (the default) means the pipeline never fractures this body; the standalone
    /// destruction pass instead falls back to its own global threshold for bodies with
    /// `None`. Crossing the threshold only emits a fracture *event* — something else has
    /// to consume it and actually split the body.
    pub fracture_threshold: Option<f32>, // Impulse threshold for fracturing
    /// External force queued for the next velocity integration, in newtons, expressed in
    /// **world** space.
    ///
    /// The integrator adds `F · inv_mass · dt` to the linear velocity and then clears the
    /// accumulator, so one assignment acts for exactly one step: re-set it every step for
    /// a sustained force. That whole path is skipped for a body that is not dynamic or is
    /// asleep — velocity integration returns before both the apply and the clear, so the
    /// queued value survives untouched. A *dynamic* body with `mass == 0.0` is the one case
    /// that is skipped but still cleared, silently dropping the force.
    ///
    /// No engine system currently writes to this field — the `Integrator` force and
    /// impulse helpers change velocity directly instead — so whatever is in here got
    /// there from your code.
    pub force_accumulator: Vec3,
    /// External torque queued for the next velocity integration, N·m, in **world** space
    /// (not body-local).
    ///
    /// It is converted with the world-space inverse inertia `R·I⁻¹·Rᵀ`
    /// ([`inv_world_inertia_tensor`](Self::inv_world_inertia_tensor)), so it produces the
    /// correct axis on a rotated or anisotropic body and respects the rotation locks.
    /// Its lifetime, and what happens to it on a non-dynamic, sleeping or zero-mass body,
    /// are the same as for
    /// [`force_accumulator`](Self::force_accumulator); both are dropped together by
    /// [`clear_forces`](Self::clear_forces).
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

    /// Enables physical air resistance: F = ½·ρ·Cd·A·|v|² (opposing the velocity). `cd` is the
    /// drag coefficient (sphere ~0.47, cube ~1.05, streamlined body ~0.04), `area` the frontal
    /// area in m². Under gravity the body settles at its natural terminal velocity. Chainable.
    pub fn with_air_drag(mut self, cd: f32, area: f32) -> Self {
        self.drag_coefficient = cd.max(0.0);
        self.drag_area = area.max(0.0);
        self
    }

    /// An immovable body for level geometry: zero mass and zero inertia, gravity and
    /// damping off, all six axes locked, and [`is_sleeping`](Self::is_sleeping) preset so
    /// the integrator never touches it.
    ///
    /// The zeroed mass properties are belt-and-braces — [`inv_mass`](Self::inv_mass)
    /// reports zero for a static body whatever `mass` holds. Static bodies still collide
    /// and still take part in queries; only their motion is disabled.
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

    /// A [`BodyType::Kinematic`] body with zero mass and zero inertia.
    ///
    /// Unlike the other constructors it turns [`ccd_enabled`](Self::ccd_enabled) **on**,
    /// because scripted movers (platforms, doors) are the classic tunnelling case. Such a
    /// body also never sleeps, so its `Velocity` is read every step for as long as it
    /// exists — see [`can_sleep`](Self::can_sleep).
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

    /// Arms [`fracture_threshold`](Self::fracture_threshold) with `threshold`, replacing
    /// any previous value. Chainable.
    ///
    /// Stored as given, with no clamping: since the comparison is `impulse > threshold`,
    /// a zero or negative threshold means any contact carrying a positive impulse
    /// fractures the body.
    pub fn with_fracture_threshold(mut self, threshold: f32) -> Self {
        self.fracture_threshold = Some(threshold);
        self
    }

    /// Sets linear + angular damping (a crude energy loss, linear in velocity). For realistic
    /// v² air resistance use [`with_air_drag`](Self::with_air_drag) instead. Chainable.
    pub fn with_damping(mut self, linear: f32, angular: f32) -> Self {
        self.linear_damping = linear.max(0.0);
        self.angular_damping = angular.max(0.0);
        self
    }

    /// Turns gravity on or off for this body (flying or suspended objects). Chainable.
    pub fn with_gravity(mut self, enabled: bool) -> Self {
        self.use_gravity = enabled;
        self
    }

    /// Enables Continuous Collision Detection (CCD) — fast or thin bodies stop tunnelling.
    /// Chainable.
    ///
    /// **It has a price, and the price is measured: a CCD body loses its bounce.** A head-on
    /// collision between equal masses at `restitution = 1` behaves correctly with CCD off (the
    /// striker stops, the struck body takes the velocity: 0.000 / 4.950 m/s) but gives
    /// **2.475 / 2.475** with CCD on — a perfectly elastic collision turned inelastic. The cause
    /// has two steps: (1) if any body in the island has CCD, the `use_tgs_soft && !has_ccd`
    /// condition drops the solver from the modern TGS-soft path **back to the old split-impulse
    /// path**, and (2) that path does not apply restitution on a speculative contact — which is
    /// exactly the kind of contact CCD produces. The speculative constraint eats the approach
    /// velocity, and by the time the real contact forms there is nothing left to bounce back.
    ///
    /// This is not an oversight but **a consequence of the body the design had in mind**: the
    /// rationale next to that exception says "CCD bodies (bullets) are rare and usually
    /// isolated". CCD was designed for a bullet — a single, lonely object that is *not expected
    /// to bounce*. A Newton's cradle violates all three assumptions: all five bodies have CCD,
    /// they all meet in one island, and every one of them is expected to bounce.
    ///
    /// So reaching for this to cure tunnelling on bodies that are *supposed* to bounce (a
    /// billiard ball, a Newton's cradle) silently destroys the scene's actual behaviour — and
    /// usually for nothing: 0.5 m radius balls do not tunnel even at 320 m/s with a 1/240 s
    /// substep (measured in the test below). If tunnelling is suspected, **measure it first**;
    /// turning CCD on as a precaution is not free.
    ///
    /// The measurement: `gizmo-physics-rigid/tests/cradle_passthrough.rs::ccd_makes_a_bounce_a_thud`.
    pub fn with_ccd(mut self) -> Self {
        self.ccd_enabled = true;
        self
    }

    /// Sets the centre of mass, in body-local coordinates. Chainable.
    pub fn with_center_of_mass(mut self, com: Vec3) -> Self {
        self.center_of_mass = com;
        self
    }

    /// Locks all three rotation axes, so the body neither tips nor spins — a character capsule,
    /// or anything meant to stay upright. Chainable.
    pub fn lock_rotation(mut self) -> Self {
        self.lock_rotation_x = true;
        self.lock_rotation_y = true;
        self.lock_rotation_z = true;
        self
    }

    /// Clears the sleep flag and resets [`sleep_counter`](Self::sleep_counter), so the
    /// body is integrated again next step and must serve the full quiet run before it can
    /// sleep once more.
    ///
    /// Call it whenever you change the velocity, the accumulators or the transform of a
    /// body that might be asleep, otherwise the change is skipped by the integrator.
    /// Idempotent on an already-awake body, and it does not look at the body type: on a
    /// static body it only clears a flag, since such a body is never integrated anyway.
    pub fn wake_up(&mut self) {
        self.is_sleeping = false;
        self.sleep_counter = 0;
    }

    /// Whether the body is quiet enough *this step* to count towards falling asleep.
    ///
    /// A pure predicate — it mutates nothing and a `true` does not mean the body is
    /// asleep; [`update_sleep_state`](Self::update_sleep_state) still requires 60
    /// consecutive `true`s. Kinematic bodies always answer `false` (user-driven motion
    /// must never be skipped) and static bodies always `true`, both without looking at
    /// `velocity` at all. A dynamic body qualifies when its linear speed is below
    /// 0.05 m/s *and* its angular speed below 0.05 rad/s — fixed thresholds, not
    /// configurable per body.
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

    /// Advances the sleep state machine by one step, given this step's `velocity`.
    ///
    /// Falling asleep is delayed, waking is immediate: while [`can_sleep`](Self::can_sleep)
    /// holds the counter grows and [`is_sleeping`](Self::is_sleeping) is set on reaching
    /// 60 consecutive qualifying steps, but a single failing step resets the counter and
    /// clears the flag at once.
    ///
    /// The 60 is a step count, not a duration — it is one second only at a 60 Hz step
    /// rate and correspondingly shorter for a world that sub-steps faster.
    ///
    /// **The engine's own pipeline no longer calls this.** It advances the counter with
    /// [`advance_sleep_counter`](Self::advance_sleep_counter) and then decides whether to
    /// actually sleep a body per contact ISLAND rather than per body, because a stack that
    /// sleeps only partway is solved inconsistently — see that method. This one is kept
    /// because it is public API and is the right thing for a caller driving bodies that are
    /// not in contact with anything.
    pub fn update_sleep_state(&mut self, velocity: &Velocity) {
        if self.advance_sleep_counter(velocity) {
            self.is_sleeping = true;
        }
    }

    /// Number of consecutive qualifying steps before a body is allowed to sleep.
    pub const SLEEP_FRAMES_REQUIRED: u32 = 60; // ~1 second at 60fps

    /// Advance the sleep counter by one step and report whether this body is now *eligible* to
    /// sleep — without putting it to sleep.
    ///
    /// Waking still happens here and is still immediate: a single step that fails
    /// [`can_sleep`](Self::can_sleep) resets the counter and clears
    /// [`is_sleeping`](Self::is_sleeping).
    ///
    /// The split exists because eligibility is a property of one body while sleeping is a
    /// property of a contact island. A body that stops being simulated in the middle of a stack
    /// is still solved against — the contact solver reads its mass and exchanges impulses with
    /// it — but is never integrated, so it takes no share of the reaction and the interface
    /// stops conserving momentum. Letting only whole islands sleep keeps that from arising.
    pub fn advance_sleep_counter(&mut self, velocity: &Velocity) -> bool {
        if self.can_sleep(velocity) {
            self.sleep_counter += 1;
            self.sleep_counter >= Self::SLEEP_FRAMES_REQUIRED
        } else {
            self.sleep_counter = 0;
            self.is_sleeping = false;
            false
        }
    }

    /// Whether this body has been slow for long enough to be allowed to sleep, without
    /// advancing anything. See [`advance_sleep_counter`](Self::advance_sleep_counter).
    #[inline]
    pub fn sleep_eligible(&self) -> bool {
        self.sleep_counter >= Self::SLEEP_FRAMES_REQUIRED
    }

    /// `true` only for [`BodyType::Dynamic`].
    ///
    /// This is the condition [`inv_mass`](Self::inv_mass) and
    /// [`inv_local_inertia`](Self::inv_local_inertia) gate on, so it is also the answer to
    /// "does the solver treat this body as having finite mass?".
    #[inline]
    pub fn is_dynamic(&self) -> bool {
        matches!(self.body_type, BodyType::Dynamic)
    }

    /// `true` only for [`BodyType::Kinematic`].
    ///
    /// The three predicates are mutually exclusive, so `!is_static()` does *not* imply
    /// the body is simulated — a kinematic body answers `false` to both `is_static` and
    /// `is_dynamic`.
    #[inline]
    pub fn is_kinematic(&self) -> bool {
        matches!(self.body_type, BodyType::Kinematic)
    }

    /// `true` only for [`BodyType::Static`].
    ///
    /// It says nothing about whether the body is asleep:
    /// [`is_sleeping`](Self::is_sleeping) is an independent flag that
    /// [`new_static`](Self::new_static) merely happens to preset.
    #[inline]
    pub fn is_static(&self) -> bool {
        matches!(self.body_type, BodyType::Static)
    }

    /// Zeroes the velocity components of every locked **world** axis, in place.
    ///
    /// Velocity integration applies it to the stored velocity, and position integration
    /// applies it again to a private copy, so calling it yourself is safe and idempotent.
    /// It constrains rates only: the transform is never clamped, and
    /// `Velocity::pre_linear` / `pre_angular` are left untouched.
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

    /// Inverse mass in kg⁻¹, or `0.0` meaning "infinite mass — cannot be accelerated".
    ///
    /// Returns zero for any non-dynamic body and for `mass == 0.0`, which makes it the
    /// reliable way to ask "can an impulse move this body?" — reading
    /// [`mass`](Self::mass) is not, because a static body may still carry a non-zero one.
    /// Nothing is guarded beyond that: a negative or NaN mass propagates into the result.
    #[inline]
    pub fn inv_mass(&self) -> f32 {
        if self.mass == 0.0 || !self.is_dynamic() {
            0.0
        } else {
            1.0 / self.mass
        }
    }

    /// Component-wise inverse of [`local_inertia`](Self::local_inertia), in the
    /// **body-local** frame (kg⁻¹·m⁻²).
    ///
    /// Zero components map to zero instead of infinity, so a zero principal inertia
    /// reads as "no angular response about that local axis" rather than a division by
    /// zero; an infinite component divides down to zero and means the same thing. Returns
    /// `Vec3::ZERO` for non-dynamic bodies and for `mass == 0.0`, mirroring
    /// [`inv_mass`](Self::inv_mass).
    ///
    /// This is the local-frame quantity and ignores the rotation locks — solver code
    /// wants [`inv_world_inertia_tensor`](Self::inv_world_inertia_tensor), which rotates
    /// it and applies them.
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

    /// Get inverse world-space inertia tensor **for an unrotated body** — simply
    /// `diag(`[`inv_local_inertia`](Self::inv_local_inertia)`)`.
    ///
    /// No rotation is applied and, unlike
    /// [`inv_world_inertia_tensor`](Self::inv_world_inertia_tensor), the rotation locks
    /// are *not* masked out. It is therefore only correct while the body's rotation is
    /// identity, or when the inertia is isotropic (`R·I⁻¹·Rᵀ = I⁻¹` for a sphere).
    /// Anywhere the body may have turned or have locked axes, use the rotation-aware
    /// version instead.
    pub fn inv_world_inertia_tensor_identity(&self) -> Mat3 {
        Mat3::from_diagonal(self.inv_local_inertia())
    }

    /// Get world-space inertia tensor from local inertia and rotation:
    /// `R·diag(local_inertia)·Rᵀ` in kg·m², where `rotation` is the body→world rotation
    /// and is expected to be normalised (an unnormalised quaternion scales the result).
    ///
    /// The forward tensor, unlike its inverse, is unconditional: body type,
    /// `mass == 0.0` and the rotation locks are all ignored, so a static or fully locked
    /// body still reports a non-zero tensor here. Mostly useful for analysis (angular
    /// momentum, kinetic energy); the solver path wants
    /// [`inv_world_inertia_tensor`](Self::inv_world_inertia_tensor).
    pub fn world_inertia_tensor(&self, rotation: Quat) -> Mat3 {
        let rot_mat = Mat3::from_quat(rotation);
        let local_inertia_mat = Mat3::from_diagonal(self.local_inertia);
        rot_mat * local_inertia_mat * rot_mat.transpose()
    }

    /// Get inverse world-space inertia tensor: `R·diag(inv_local_inertia)·Rᵀ`, with
    /// `rotation` the (normalised) body→world rotation. This is the tensor to use for
    /// any world-space torque or angular impulse.
    ///
    /// Returns `Mat3::ZERO` for non-dynamic bodies and for `mass == 0.0` — "no angular
    /// response at all". Each locked rotation axis then has its whole row *and* column
    /// zeroed, so the result stays symmetric and a torque about a locked world axis
    /// yields neither acceleration about that axis nor cross-coupling into the free ones.
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

    /// Drops both accumulators back to `Vec3::ZERO`.
    ///
    /// Velocity integration calls this itself once the accumulators have been applied, so
    /// external code needs it only to *cancel* forces queued earlier in the same step —
    /// after teleporting a body, say. It does not touch the velocity, so anything already
    /// integrated is not undone.
    pub fn clear_forces(&mut self) {
        self.force_accumulator = Vec3::ZERO;
        self.torque_accumulator = Vec3::ZERO;
    }

    /// Overwrites [`local_inertia`](Self::local_inertia) with the solid-box tensor for the
    /// **current** [`mass`](Self::mass): `Ix = m/12·(h²+d²)` and cyclic permutations.
    ///
    /// `w`, `h`, `d` are the *full* extents in metres along the body-local X/Y/Z axes —
    /// not half-extents; passing half-extents under-estimates the inertia fourfold. Set
    /// the mass first: this reads it and never writes it, and it does not consult the
    /// body type, so it will happily fill in a tensor for a static body (harmlessly, as
    /// its inverse is forced to zero). Degenerate input is not rejected: zero extents
    /// give zero components, which then read as "no rotation about that axis".
    pub fn calculate_box_inertia(&mut self, w: f32, h: f32, d: f32) {
        let m = self.mass;
        self.local_inertia = Vec3::new(
            (m / 12.0) * (h * h + d * d),
            (m / 12.0) * (w * w + d * d),
            (m / 12.0) * (w * w + h * h),
        );
    }

    /// Overwrites [`local_inertia`](Self::local_inertia) with the *solid, uniform* sphere
    /// tensor `I = 2/5·m·r²` on all three axes, taking `m` from the current
    /// [`mass`](Self::mass) and `r` in metres.
    ///
    /// The only isotropic case: the tensor is unchanged by rotation, which is why
    /// [`inv_world_inertia_tensor_identity`](Self::inv_world_inertia_tensor_identity) is
    /// exact for a sphere at any orientation. A hollow shell (`2/3·m·r²`) is not offered
    /// — assign [`local_inertia`](Self::local_inertia) directly if you need one.
    pub fn calculate_sphere_inertia(&mut self, r: f32) {
        let i = 0.4 * self.mass * r * r;
        self.local_inertia = Vec3::splat(i);
    }

    /// Principal inertia of a solid cylinder about local +Y, from the analytic formulas:
    /// `I_y = ½·m·r²` about the axis, `I_x = I_z = m·(3r² + h²)/12` across it, with `h` the
    /// full height.
    ///
    /// Not the capsule's number with the caps subtracted: the two shapes differ by a
    /// hemisphere at each end, and those hemispheres are the *farthest* mass from the axis,
    /// so a capsule's transverse inertia runs well above a cylinder's of the same dimensions.
    /// Reusing one for the other would make a wheel resist tipping like something it is not.
    pub fn calculate_cylinder_inertia(&mut self, r: f32, half_h: f32) {
        let m = self.mass;
        let h = half_h * 2.0;
        let i_y = 0.5 * m * r * r;
        let i_xz = m * (3.0 * r * r + h * h) / 12.0;
        self.local_inertia = Vec3::new(i_xz, i_y, i_xz);
    }

    /// Overwrites [`local_inertia`](Self::local_inertia) with the solid-capsule tensor for
    /// a capsule whose axis is the body-local **Y** axis, matching the capsule collider's
    /// own convention.
    ///
    /// `r` is the radius and `half_h` the half-length of the **cylindrical section only**
    /// — the hemispherical caps add a further `r` at each end, so the total height is
    /// `2·(half_h + r)`. The current [`mass`](Self::mass) is split between cylinder and
    /// caps by volume, so the parts sum back to it; Y (the spin axis) gets the smaller
    /// value and X/Z the equal transverse one.
    ///
    /// The caps' parallel-axis transfer deliberately carries **no** extra COM-offset
    /// term: adding one (a since-fixed bug here) inflates the transverse inertia and
    /// makes capsules far too reluctant to tip. Zero radius *and* zero half-height leave
    /// zero inertia rather than dividing by zero.
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

    /// Derives [`local_inertia`](Self::local_inertia) from `collider`'s shape at the
    /// current [`mass`](Self::mass), which it reads but never changes.
    ///
    /// Nothing calls this for you when a body is inserted into `PhysicsWorld` — that path
    /// stores the mass properties exactly as given — so call it after setting the mass or
    /// swapping the shape, otherwise the placeholder unit inertia stands. (The facade
    /// crate's `RigidBodyBundle` does invoke it while spawning.)
    ///
    /// Accuracy per shape:
    /// - **Box, sphere, capsule** — the exact analytic solid tensor.
    /// - **Plane** — infinite on every axis, i.e. zero inverse: a plane cannot be spun.
    /// - **ConvexHull** — approximated by the box tensor of the hull's local AABB, each
    ///   extent floored at 1 mm; an empty vertex list falls back to a 1×1×1 box.
    /// - **TriMesh** — no shape analysis at all, just a 1×1×1 box tensor. Scale it
    ///   yourself if the mesh is not roughly unit-sized.
    /// - **Compound** — mass split over the sub-shapes by volume, then a parallel-axis
    ///   sum. This branch is the one that also **overwrites**
    ///   [`center_of_mass`](Self::center_of_mass), with the volume-weighted average of
    ///   the sub-shape origins. Sub-shape rotations are ignored and each sub-shape's own
    ///   centre of mass is taken to be its local origin, so expect an approximation for
    ///   anything but axis-aligned parts; zero total volume falls back to a 1×1×1 box.
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
            ColliderShape::Cylinder(c) => {
                self.calculate_cylinder_inertia(c.radius, c.half_height);
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
            ColliderShape::TriMesh(_) | ColliderShape::Heightfield(_) => {
                // Both are static surfaces with no solid interior, so there is no tensor to
                // derive. A unit box is the placeholder; a dynamic body wearing terrain is a
                // mistake this cannot repair.
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

#[cfg(feature = "ecs")]
gizmo_core::impl_component!(RigidBody);

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::components::collider::ConvexHullShape;
    use std::sync::Arc;

    /// Fluent builders: configure in one chain instead of reaching for the fields.
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

    /// A ConvexHull's inertia must be derived from its AABB (it used to be a fixed 1×1×1, so
    /// every fracture fragment had the same inertia regardless of its size).
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

    /// A capsule's transverse inertia (i_xz) must match the analytic value. Regression: the
    /// hemisphere's parallel-axis COM-offset term (9/64·r² = 0.140625·r²) used to be counted
    /// twice, so transverse inertia came out too large and the capsule was over-resistant to
    /// tipping. The correct hemisphere-pair contribution is m_sph·(2/5·r² + half_h² +
    /// 3/4·r·half_h) — with NO extra COM-offset term.
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
