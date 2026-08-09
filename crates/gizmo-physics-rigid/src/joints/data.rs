use gizmo_physics_core::BodyHandle;
use gizmo_math::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Bir çözücü GEÇİŞİNE ait çözücü scratch'i: satır-başına birikmiş λ ve geçişin NET
/// impulse'ı.
///
/// Eşitsizlik (tek-yönlü) satırlarında — limit, halat, koni — clamp artık her iterasyonun
/// kendi artımına değil, geçiş boyunca birikmiş TOPLAM λ'ya uygulanır. Eskiden negatif bir
/// artım her seferinde 0'a kırpıldığından bir satır kendi ÖNCEKİ AŞIRI-DÜZELTMESİNİ geri
/// alamıyordu: aynı eklemin başka bir satırı Jv'yi geri ittiğinde limit her iterasyonda
/// yeniden pompalıyor, hiç geri vermiyordu — tek yönlü bir cırcır.
///
/// `is_broken` gibi `#[serde(skip)]`: sahne dosyası formatının parçası değil. Her
/// `solve_joints` geçişinin başında sıfırlanır, yani `warm_start_factor = 0` (VARSAYILAN)
/// iken adımlar arasında TAŞINMAZ. Warm start açıldığında `prev_rows` geçen geçişin
/// λ'sını taşır ve gerçek simülasyon durumu olur — `WorldSnapshot` `joints`'i olduğu gibi
/// klonladığı için bu zaten kapsanıyor (bkz. `JointSolver::solve_joints`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JointScratch {
    rows: [f32; JointScratch::LEN],
    /// Bir ÖNCEKİ `solve_joints` geçişinin λ'sı — yalnızca warm start okur.
    prev_rows: [f32; JointScratch::LEN],
    /// Enjeksiyon süpürmesi sırasında warm-start faktörü, aksi hâlde 0. Katsayı yerine
    /// scratch'te durmasının sebebi tamamen pratik: scratch zaten 25 `apply_*` çağrı yerinin
    /// hepsine geçiyor, çözücü modu ise geçmiyordu.
    warm_factor: f32,
    /// Geçişin NET doğrusal impulse'ı, `Σ λᵢ·nᵢ` (dünya uzayı). `break_force` bundan
    /// hesaplanır: taşınan gerçek tepki kuvvetinin büyüklüğü `‖Σ λᵢ·nᵢ‖ / dt`.
    pub(crate) impulse_lin: Vec3,
    /// Geçişin NET açısal impulse'ı, `Σ λᵢ·nᵢ`. `break_torque` bundan hesaplanır.
    pub(crate) impulse_ang: Vec3,
}

impl JointScratch {
    /// En çok satır üreten tür BallSocket: 3 lineer (`solve_fixed_joint`'ten), koni, twist ve
    /// 2 asimetrik swing = 7 satır, en yüksek yuva indeksi 8. 10 = yuva 9 motor satırı için
    /// ayrılmış pay.
    pub const LEN: usize = 10;

    /// Bir satırın birikmiş λ'sına erişim. Yuvalar `joints::solver::row` içinde DERLEME
    /// ZAMANI SABİTİ; koşullu atlanan satırlar yüzünden ilerleyen bir imleç kullanılamaz
    /// (atlanan bir satır sonraki her satırın kimliğini kaydırırdı).
    #[inline]
    pub(crate) fn row(&mut self, slot: usize) -> &mut f32 {
        &mut self.rows[slot]
    }

    /// Read-only view of a row's accumulated λ — the joint block of
    /// [`PhysicsWorld::state_hash`](crate::PhysicsWorld::state_hash) and the crate's tests.
    ///
    /// This is per-PASS scratch, not carried state: it holds whatever the most recent
    /// `solve_joints` left. Hashing it is an early-warning tripwire (a desync shows up in the
    /// solver before it has bled into velocities), not a record of something a rollback would
    /// otherwise lose.
    #[inline]
    pub(crate) fn row_value(&self, slot: usize) -> f32 {
        self.rows[slot]
    }

    /// Previous pass's λ for a row — the warm start's only input.
    #[inline]
    pub(crate) fn prev_row_value(&self, slot: usize) -> f32 {
        self.prev_rows[slot]
    }

    /// `Some(factor)` only while the warm-start injection sweep is running.
    #[inline]
    pub(crate) fn warm_injection(&self) -> Option<f32> {
        (self.warm_factor != 0.0).then_some(self.warm_factor)
    }

    /// Arm/disarm the injection sweep. Set to `0.0` again before the real iterations run.
    #[inline]
    pub(crate) fn set_warm_injection(&mut self, factor: f32) {
        self.warm_factor = factor;
    }

    /// Begin a solver pass that WILL run: this pass's λ and net impulses go to zero, and the
    /// λ the previous pass converged to is retained for the warm start.
    #[inline]
    pub(crate) fn begin_pass(&mut self) {
        self.prev_rows = self.rows;
        self.clear_pass();
    }

    /// Clear a pass that will NOT run (a zero-length step): the same visible zeroing as
    /// [`Self::begin_pass`], leaving `prev_rows` alone.
    ///
    /// Note what that does and does not buy, because it is easy to read as more than it is.
    /// `prev_rows` survives THIS call, but the next real pass's [`Self::begin_pass`] overwrites
    /// it with `rows`, which this call just zeroed — so a zero-length step still costs the
    /// warm start its history and the next real pass starts cold. That is a deliberate
    /// non-goal, not a property: making the history survive would mean latching it at the END
    /// of a real pass instead, and a knob that is off by default (and undecided) does not
    /// justify moving state that `PhysicsWorld::state_hash` reads. What this method IS for is
    /// keeping the zeroing identical on both paths, so that at the default
    /// `JointSolver::warm_start_factor` of 0 the distinction is unobservable.
    #[inline]
    pub(crate) fn clear_pass(&mut self) {
        self.rows = [0.0; Self::LEN];
        self.warm_factor = 0.0;
        self.impulse_lin = Vec3::ZERO;
        self.impulse_ang = Vec3::ZERO;
    }
}

/// A constraint between two rigid bodies: the pair of bodies, where it grips them, when it
/// gives up, plus the kind-specific payload in [`data`](Self::data).
///
/// **The two ends are not interchangeable.** The local frames a [`JointData`] variant
/// carries — the slider `axis`, the ball-socket `twist_axis`, the D6 `frame` — are
/// interpreted in **A's** local space (the hinge `axis` is the exception, see
/// [`HingeJointData::axis`]), and every relative quantity a limit, motor or drive measures
/// is *B relative to A* — swapping the handles flips the sign of every angle, offset and
/// target velocity.
///
/// A `Joint` is simulation state, not merely configuration: [`is_broken`](Self::is_broken)
/// and the reference pose latched inside `data` cannot be recomputed from transforms and
/// velocities, so anything that rewinds the world (rollback, replay) must carry the joint
/// list too, or the resimulation diverges from the uninterrupted run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Joint {
    /// Body **A**, the frame-defining end (see the type-level docs).
    ///
    /// Resolved from the handle's id to a body index on every solver pass; if that id is no
    /// longer in the world the joint is silently skipped for that pass — it is neither
    /// removed nor marked broken, so a joint outliving one of its bodies is a no-op rather
    /// than an error. Must differ from [`entity_b`](Self::entity_b): the constructors
    /// `debug_assert` it, and a joint whose two handles resolve to the same body index is
    /// skipped.
    pub entity_a: BodyHandle,
    /// Body **B**, the driven end: limits, motors, drives and the tracked hinge angle /
    /// slider offset all describe B's motion *relative to A*, positive along the
    /// corresponding A-local axis. Same resolution and distinctness rules as
    /// [`entity_a`](Self::entity_a).
    pub entity_b: BodyHandle,
    /// Attachment point on A, in metres in A's local frame, measured from A's **transform
    /// origin** — not from its centre of mass. The world point is
    /// `transform.position + transform.rotation * local_anchor_a`; `Transform::scale` is not
    /// applied, so rescaling a body does not move its anchor.
    ///
    /// The origin-vs-centre-of-mass distinction is load-bearing for any body whose
    /// `center_of_mass` is offset (compound colliders, fracture chunks, vehicle chassis):
    /// you author the anchor relative to the origin and the solver derives the
    /// centre-of-mass-relative lever arm itself.
    pub local_anchor_a: Vec3,
    /// Attachment point on B, same frame and units as
    /// [`local_anchor_a`](Self::local_anchor_a). `Vec3::ZERO` puts it at B's transform
    /// origin.
    ///
    /// What the *pair* means depends on [`data`](Self::data): Fixed/Hinge/BallSocket drive
    /// the two world anchor points onto each other, Slider/D6 measure the offset between
    /// them along the joint axes, and Spring/Distance measure the length between them — in
    /// every case between the anchors, never between the body origins.
    pub local_anchor_b: Vec3,
    /// Linear break threshold in newtons; `f32::INFINITY` (what every constructor sets)
    /// means unbreakable.
    ///
    /// Checked once per solver pass against the magnitude of that pass's **net** linear
    /// reaction, `‖Σ λᵢ·nᵢ‖ / dt` — the force the joint actually carries, not the sum of its
    /// rows' magnitudes, and not something that scales with the solver's iteration count.
    /// Force-based springs (Spring, slider suspension) count toward it; motors and D6 drives
    /// do not, since they are actuators rather than external load. The comparison is strict,
    /// so a load exactly equal to the threshold does not break the joint, and a non-finite
    /// reaction of NaN never does.
    pub break_force: f32,
    /// Angular break threshold in newton-metres, `f32::INFINITY` = unbreakable. Derived from
    /// the pass's net *angular* impulse; the two thresholds are independent and exceeding
    /// either one alone breaks the joint. The accounting rules are shared — see
    /// [`break_force`](Self::break_force).
    pub break_torque: f32,
    /// Once `true` the joint is skipped by every solver stage: it stops constraining, but
    /// stays in the world's joint list.
    ///
    /// A one-way latch — [`check_break`](Self::check_break) sets it and nothing in the
    /// solver ever clears it, so un-breaking means assigning `false` yourself. It is
    /// `#[serde(skip)]` runtime state: a saved scene never resurrects as already-broken.
    ///
    /// Breaking does **not** restore collisions between the pair: contact filtering keys on
    /// [`collision_enabled`](Self::collision_enabled) alone, so a broken joint that had
    /// collisions disabled leaves its two bodies still passing through each other.
    #[serde(skip)]
    pub is_broken: bool,
    /// Solver scratch — the per-row accumulated λ plus this pass's net impulse.
    ///
    /// Not serialized (like [`is_broken`](Self::is_broken)): a saved scene must not resurrect
    /// mid-solve. It IS cloned, however, and therefore travels in `WorldSnapshot` — the λ rows
    /// now cross the pass boundary as the solver's warm start, so a rollback that dropped them
    /// would resimulate from a colder solver than the continuous run. See [`JointScratch`] for
    /// which parts persist and which are cleared every pass.
    #[serde(skip)]
    pub(crate) scratch: JointScratch,
    /// Whether the two joined bodies may still generate contacts *with each other*.
    ///
    /// `false` — what every constructor sets — suppresses every contact between the pair,
    /// which is what a ragdoll limb or a wheel-to-chassis mount wants, since those pieces
    /// overlap by construction. The suppression is keyed on the two body ids and ignores
    /// [`is_broken`](Self::is_broken), so it outlives the joint's breaking. It affects only
    /// this pair; ordinary collision-layer filtering governs everything else, and enabling
    /// it here does not override a layer mask that already rejects the pair.
    pub collision_enabled: bool,
    /// The joint kind and its parameters — this is what decides which constraint rows get
    /// built each pass.
    ///
    /// Kind-local runtime state lives in here too — the reference pose a
    /// ball-socket/slider/D6 latches on its first solve, and the hinge's tracked angle — so
    /// replacing this value with a freshly built one also discards that latch, and the next
    /// pass re-captures it from the bodies' current pose. It does NOT discard the solver's
    /// accumulated warm-start λ, which belongs to the rows the old payload defined — call
    /// [`reset_warm_start`](Self::reset_warm_start) as well when you reconfigure a joint.
    pub data: JointData,
}

/// The kind-specific payload of a [`Joint`]: the variant selects which solver runs, and the
/// payload carries that kind's axes, limits, motors and latched reference pose.
///
/// Variants are not orthogonal — Hinge and BallSocket run the Fixed joint's point
/// constraint first and then add their own rows, and D6 can express Fixed, Slider and Hinge
/// as configurations. Each variant is mirrored by a [`JointType`], which the `From` impl
/// below keeps in step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JointData {
    /// Weld: the anchors are pinned together *and* relative rotation is driven to zero on
    /// all three axes. Carries no parameters — the orientation it holds is whatever the two
    /// bodies had, because the angular lock zeroes relative angular *velocity* every
    /// sub-step rather than servoing toward a stored pose.
    ///
    /// That makes it the crate's only **velocity-only** row class, and the solver treats it as
    /// one: a soft row leaves a residual velocity that its position term takes back, and this
    /// one has no position term, so it is exempted from `JointSolver::rigid_hertz` and stays a
    /// hard constraint. Softening it made the weld angle drift linearly and without bound
    /// under sustained torque. Giving `Fixed` a latched reference pose — the way
    /// [`SliderJointData`] has one — would remove the exemption, and is a breaking change to
    /// this enum rather than a solver tweak.
    Fixed,
    /// One rotational DOF about a shared local axis, anchors pinned. Optional angle limits,
    /// motor/servo and torsional spring — see [`HingeJointData`].
    Hinge(HingeJointData),
    /// Three rotational DOF, anchors pinned. With every limit disabled there is no angular
    /// constraint at all (and no reference pose is latched); see [`BallSocketJointData`].
    BallSocket(BallSocketJointData),
    /// One translational DOF along an A-local axis, with relative rotation fully locked to
    /// the pose latched on the first solve. See [`SliderJointData`].
    Slider(SliderJointData),
    /// A soft spring-damper between the anchors: a *force* rather than a constraint, and so
    /// the one variant that removes no degree of freedom. See [`SpringJointData`], or
    /// [`Distance`](Self::Distance) for a hard bound.
    Spring(SpringJointData),
    /// A hard inequality on the anchor separation — rope, rigid rod, or a band between two
    /// bounds. See [`DistanceJointData`].
    Distance(DistanceJointData),
    /// Generic 6-DOF joint: per-axis Locked/Free/Limited over the three translations and
    /// three rotations of a configurable A-local frame, plus optional drives. See
    /// [`D6JointData`].
    D6(D6JointData),
}

/// Names a joint kind without carrying its parameters — the `Copy` + `Eq` counterpart of
/// [`JointData`], used for authoring descriptors and for solver dispatch (`match` on this
/// rather than on the string from [`Joint::joint_type`]).
///
/// Unrelated to `multibody::JointType`, a different enum that only exists behind the
/// off-by-default `experimental-multibody` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum JointType {
    /// Names [`JointData::Fixed`] — zero relative DOF.
    Fixed,
    /// Names [`JointData::Hinge`] — one rotational DOF.
    Hinge,
    /// Names [`JointData::BallSocket`] — three rotational DOF.
    BallSocket,
    /// Names [`JointData::Slider`] — one translational DOF.
    Slider,
    /// Names [`JointData::Spring`] — removes no DOF; a force, not a constraint.
    Spring,
    /// Names [`JointData::Distance`] — one inequality on the anchor separation.
    Distance,
    /// Names [`JointData::D6`] — per-axis Locked/Free/Limited over all six DOF.
    D6,
}

/// Compile-forced mapping so `JointType` (the authoring descriptor) and `JointData`
/// (the runtime payload) can never silently drift: adding a `JointData` variant without
/// a matching `JointType` is a compile error here. Used by the solver dispatch.
impl From<&JointData> for JointType {
    fn from(data: &JointData) -> Self {
        match data {
            JointData::Fixed => JointType::Fixed,
            JointData::Hinge(_) => JointType::Hinge,
            JointData::BallSocket(_) => JointType::BallSocket,
            JointData::Slider(_) => JointType::Slider,
            JointData::Spring(_) => JointType::Spring,
            JointData::Distance(_) => JointType::Distance,
            JointData::D6(_) => JointType::D6,
        }
    }
}

/// Revolute-joint parameters: one rotational DOF about [`axis`](Self::axis), with the two
/// anchors held together by the same point constraint a Fixed joint uses.
///
/// The derived `Default` leaves `axis` at zero. The solver then has no axis to align, to
/// measure an angle about or to drive, so every one of those rows is inert and the hinge
/// degrades to the bare point constraint — prefer [`Joint::hinge`], which normalises the
/// axis and seeds a usable limit range.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct HingeJointData {
    /// Rotation axis, expected to be **unit length**, expressed in the local frame of
    /// *both* bodies: the solver aligns `rot_a * axis` with `rot_b * axis`, so the hinge
    /// must lie along the same local direction in each body's own frame (this is the one
    /// axis in this module that is not A-only).
    ///
    /// [`Joint::hinge`] normalises the argument and substitutes `Vec3::Y` for a near-zero
    /// one; assigning this field directly does neither, and a non-unit axis distorts both
    /// the tracked angle and the scale of the limit and motor rows.
    pub axis: Vec3,
    /// Gate for [`lower_limit`](Self::lower_limit)/[`upper_limit`](Self::upper_limit). When
    /// `false` the limit row is not built at all (rather than being built with infinite
    /// bounds) and the hinge turns freely.
    pub use_limits: bool,
    /// Lower bound of the hinge angle in radians, honoured only when
    /// [`use_limits`](Self::use_limits) is set. Compared against
    /// [`current_angle`](Self::current_angle), which lives in `[-π, π]`. Do NOT park this
    /// above `π` to "disable" the lower limit: the `current_angle < lower_limit` test then
    /// holds on every solve, so the row engages permanently and drives the hinge instead of
    /// going inert. Clear [`use_limits`](Self::use_limits) to disable. [`Joint::hinge`]
    /// seeds `-π`.
    pub lower_limit: f32,
    /// Upper bound of the hinge angle in radians; seeded to `π`. Keep it ≥
    /// [`lower_limit`](Self::lower_limit): the two are mutually exclusive branches on one
    /// solver row with the lower one tested first, so an inverted pair lets only the lower
    /// bound act wherever both are violated.
    pub upper_limit: f32,
    /// Gate for the motor row. Note that
    /// [`motor_max_force`](Self::motor_max_force) starts at zero, so enabling this alone
    /// still produces no torque.
    pub use_motor: bool,
    /// Target *relative* angular velocity about the hinge axis, in rad/s, of B with respect
    /// to A (positive = B turning positively about `axis`). Used only when
    /// [`use_motor`](Self::use_motor) is set and
    /// [`motor_is_servo`](Self::motor_is_servo) is not.
    pub motor_target_velocity: f32,
    /// Motor budget — despite the name this is a **torque, in newton-metres**, because the
    /// hinge motor drives an angular row.
    ///
    /// It bounds the motor's accumulated impulse to `±motor_max_force · dt` over the whole
    /// solver pass, not per iteration, so changing the solver's iteration count does not
    /// change the torque the motor can deliver. `0.0` (the constructor default) is a motor
    /// with no authority at all. The motor's impulse is deliberately excluded from the
    /// break-force accounting.
    pub motor_max_force: f32,
    /// When true (and `use_motor`), the motor is a POSITION SERVO: it drives toward
    /// `motor_target_position` (target angle, rad) instead of holding a target velocity,
    /// force-limited by `motor_max_force`. When false it is the classic velocity motor.
    pub motor_is_servo: bool,
    /// Servo target angle in radians, read only when
    /// [`motor_is_servo`](Self::motor_is_servo) is set. Measured on the same scale as
    /// [`current_angle`](Self::current_angle), and driven by proportional control on the
    /// angle error (through the solver's `position_bias`), still capped by
    /// [`motor_max_force`](Self::motor_max_force).
    pub motor_target_position: f32,
    /// Torsional spring / return-to-center: a soft restoring torque toward `rest_angle`
    /// (stiffness + damping) about the hinge axis — self-closing doors, spring flaps, soft
    /// ragdoll joint stiffness. The angular analogue of the Slider suspension spring;
    /// force-based (applied once per step).
    pub use_torsional_spring: bool,
    /// Restoring stiffness in newton-metres per radian of `current_angle - rest_angle`.
    /// Applied once per step as a force (outside the velocity iteration loop), and unlike
    /// the motor it *does* count toward [`Joint::break_torque`].
    pub torsional_stiffness: f32,
    /// Damping in newton-metres per rad/s of relative angular velocity about the axis.
    /// It damps the *relative* rate rather than motion away from `rest_angle`, so it also
    /// resists rotation driven by the motor or by an external load, even at rest.
    pub torsional_damping: f32,
    /// Angle in radians at which the torsional spring exerts no torque, on the same scale as
    /// [`current_angle`](Self::current_angle) — so `0.0` means "A and B agree about the
    /// axis", not "the pose the joint was authored in". The hinge latches no reference pose.
    pub rest_angle: f32,
    /// Solver **output**: the signed rotation of B relative to A about the hinge axis, in
    /// radians in `[-π, π]`.
    ///
    /// Recomputed during each solver pass (and left stale when the projection used to
    /// measure it collapses), then read by the servo and the torsional spring. Writing to it
    /// therefore only holds until the next pass. Zero means the two bodies' orientations
    /// agree about the axis, not that the
    /// joint sits at its creation pose. `#[serde(skip)]`: runtime state, not scene data.
    #[serde(skip)]
    pub current_angle: f32,
}

/// Spherical-joint parameters: the anchors are pinned together and all three rotational DOF
/// are free unless one of the three limit groups here is enabled.
///
/// Cone, twist and swing are all measured against
/// [`initial_relative_rotation`](Self::initial_relative_rotation) — the relative pose latched
/// on the first solve — so that rest pose is the centre of every limit. Every limit is
/// one-sided (it pushes back only once breached) and softened by
/// [`compliance`](Self::compliance).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct BallSocketJointData {
    /// Gate for [`cone_limit_angle`](Self::cone_limit_angle). With this and both other limit
    /// gates off, the joint stays a pure point constraint and no rest pose is latched at all.
    pub use_cone_limit: bool,
    /// Half-angle of the allowed cone, in radians: the largest angle by which
    /// [`twist_axis`](Self::twist_axis) may be tipped away from where it sits in the rest
    /// pose. [`Joint::ball_socket`] seeds `π`, i.e. effectively unlimited.
    ///
    /// The measured quantity is the SWING angle of the swing–twist decomposition of the
    /// deviation from `initial_relative_rotation` about `twist_axis` — roll about the axis
    /// itself is divided out and left to [`use_twist_limit`](Self::use_twist_limit), so the
    /// two limits are independent. Until 2026-08 it was the *full* deviation angle instead,
    /// which meant twist spent the cone's budget: a limb rolled 30° about its own bone, with
    /// its axis not tipped at all, was 30° into a cone it had never entered.
    ///
    /// When `twist_axis` is left at the derived `Default`'s zero there is no axis to
    /// decompose about and the measure falls back to that full deviation angle — the only
    /// thing a cone with no axis can mean.
    pub cone_limit_angle: f32,
    /// Twist (roll about `twist_axis`) limit — the second half of a cone-twist joint.
    /// The cone limits SWING (how far the axis tips); this limits TWIST (spin about it),
    /// so a ragdoll limb no longer spins freely about its own bone. `twist_axis` is in
    /// A's local frame. Two-sided: `[twist_lower, twist_upper]` (radians).
    pub use_twist_limit: bool,
    /// Twist axis in **A's** local frame. Normalised internally, but both the twist row and
    /// the asymmetric swing rows are skipped outright when it is near zero — which is what
    /// the derived `Default` leaves it as ([`Joint::ball_socket`] seeds `Vec3::Y`).
    ///
    /// It is the axis of ALL THREE limit groups, not just the twist one: it is the axis the
    /// cone opens around, the axis whose roll [`twist_lower`](Self::twist_lower)/
    /// [`twist_upper`](Self::twist_upper) bound, and the axis whose two perpendiculars
    /// [`swing_limit_1`](Self::swing_limit_1) and [`swing_limit_2`](Self::swing_limit_2) act
    /// about. Re-aiming it re-aims all of them together.
    pub twist_axis: Vec3,
    /// Lower twist bound in radians about `twist_axis`, measured from the rest pose — so
    /// `0.0` is "no twist relative to the latched pose", and this value is normally negative.
    /// Seeded to `-π`.
    pub twist_lower: f32,
    /// Upper twist bound in radians about `twist_axis`, seeded to `π`. Keep it ≥
    /// [`twist_lower`](Self::twist_lower): the two are exclusive branches of one solver row
    /// with the upper one tested first.
    pub twist_upper: f32,
    /// Asymmetric (per-axis) swing limits: clamp the swing about the two axes perpendicular
    /// to `twist_axis` independently, so a shoulder/hip can have a different range in each
    /// direction — unlike the single circular `cone_limit_angle`. Radians about each perp.
    pub use_swing_limits: bool,
    /// Symmetric swing bound in radians about the first perpendicular of `twist_axis`: the
    /// allowed range is `±swing_limit_1`, there is no separate negative bound. Seeded `π`.
    ///
    /// The quantity compared against it is the component of the swing's ROTATION VECTOR
    /// (axis · angle, radians) along that perpendicular, so for a pure swing about it the
    /// bound is the angle you wrote, over the whole range. Until 2026-08 the code resolved
    /// the quaternion vector part instead — `2·sin(θ/2)`, a chord — and every bound was
    /// therefore too loose by a margin that grew with it: `π/2` first engaged at ≈1.80 rad
    /// (103°), and anything ≥ 2 rad could not engage at all. A swing that is not purely about
    /// one perpendicular still splits between the two rows, which is an approximation of an
    /// elliptical cone, not an exact one.
    pub swing_limit_1: f32,
    /// The same symmetric bound about the second perpendicular of `twist_axis`, with the
    /// same measure and seed as [`swing_limit_1`](Self::swing_limit_1).
    ///
    /// Which world direction each of the two perpendiculars ends up along is derived
    /// internally from `twist_axis` and is not part of the contract; if a limb needs a
    /// specific pairing, check it for your axis rather than assuming an order.
    pub swing_limit_2: f32,
    /// Inverse stiffness applied to the cone/twist/swing LIMITS: 0 = hard stop; larger =
    /// a soft, springy limit that gives under load (natural ragdoll joint feel).
    ///
    /// Literally `1/K` in newton-metres per radian: a limit breached by `θ` past its bound
    /// pushes back with `θ/compliance`, so a heavier limb sinks further into its stop. Note
    /// this was NOT true before the soft-constraint rewrite, when the value behaved as a
    /// solver relaxation factor and its effect halved every time `iterations` doubled.
    pub compliance: f32,
    /// Rest pose latched on the joint's first solve: the rotation of B relative to A
    /// (`rot_a⁻¹ · rot_b`) at that instant, expressed in A's frame. Every cone, twist and
    /// swing bound is measured from it, so a stale value silently redefines where the limits
    /// sit.
    ///
    /// `None` means "not latched yet": the first solver iteration that reaches a limit
    /// stores the current relative pose and builds no angular row in that iteration (the
    /// point constraint holding the anchors together still runs). Set it explicitly to pin a
    /// rest pose independent of where the bodies spawn.
    /// `#[serde(default)]`, so a scene file that omits it re-latches from the loaded pose
    /// instead of failing to parse.
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

/// Prismatic-joint parameters: one translational DOF along [`axis`](Self::axis), with the
/// two off-axis translations pinned and relative rotation fully locked to the pose latched
/// in [`initial_relative_rotation`](Self::initial_relative_rotation).
///
/// The derived `Default` leaves `axis` at zero. The solver treats that as "no axis" and
/// skips every row that needs one — the off-axis pins, the travel limits and the motor — so
/// a defaulted slider is a bare rotation lock, not a slider. Build one with
/// [`Joint::slider`], which substitutes `Vec3::Y` for a near-zero axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SliderJointData {
    /// Sliding direction, in **A's** local frame only — B's orientation never enters, since
    /// B's rotation is locked to A's anyway. [`Joint::slider`] normalises the argument
    /// (falling back to `Vec3::Y` when it is shorter than ~1e-3); assigning this field
    /// afterwards does not, but the solver normalises whatever it finds, so any non-zero
    /// length works. A zero axis means "no axis": the off-axis pins, the travel limits and
    /// the motor are all skipped and only the angular lock runs.
    pub axis: Vec3,
    /// Gate for the travel limits. When `false` (the default) travel along the axis is
    /// unbounded and the limit row is not built; [`Joint::slider`] seeds the bounds to ±10 m
    /// so they only mean something once this is on.
    pub use_limits: bool,
    /// Lower travel bound in metres, honoured only when [`use_limits`](Self::use_limits) is
    /// set. Measured like [`current_position`](Self::current_position) — the along-axis
    /// component of (anchor B − anchor A) — so if the two anchors are not coincident at rest,
    /// the resting value is not 0 and both bounds must be offset to match.
    pub lower_limit: f32,
    /// Upper travel bound in metres. Keep it ≥ [`lower_limit`](Self::lower_limit): the two
    /// are exclusive branches of a single solver row with the lower one tested first.
    pub upper_limit: f32,
    /// Gate for the motor row; as with the hinge,
    /// [`motor_max_force`](Self::motor_max_force) starts at zero, so this alone moves
    /// nothing.
    pub use_motor: bool,
    /// Target relative speed along the axis in m/s, of B's anchor with respect to A's.
    /// Measured at the anchor points, so a body spinning about a distant anchor contributes.
    /// Used only when [`use_motor`](Self::use_motor) is set and
    /// [`motor_is_servo`](Self::motor_is_servo) is not.
    pub motor_target_velocity: f32,
    /// Motor budget in **newtons** (here the row really is linear, unlike the hinge's
    /// same-named field). Bounds the motor's accumulated impulse to `±motor_max_force · dt`
    /// across the whole pass rather than per iteration, so the delivered force does not track
    /// the solver's iteration count. `0.0` = no authority. Motor impulses are excluded from
    /// the break-force accounting.
    pub motor_max_force: f32,
    /// When true (and `use_motor`), the motor is a POSITION SERVO driving toward
    /// `motor_target_position` (target offset along the axis) instead of a target velocity.
    pub motor_is_servo: bool,
    /// Servo target in metres along the axis, read only when
    /// [`motor_is_servo`](Self::motor_is_servo) is set. Compared against
    /// [`current_position`](Self::current_position), driven by proportional control on that
    /// error (through the solver's `position_bias`) and still capped by
    /// [`motor_max_force`](Self::motor_max_force).
    pub motor_target_position: f32,
    /// Suspension spring along the free axis: a soft PD force toward `spring_rest_position`
    /// (stiffness + damping). This is the canonical shock/suspension/elevator-buffer
    /// primitive — a springy prismatic, applied once per step (force-based, like Spring).
    pub use_spring: bool,
    /// Suspension stiffness in newtons per metre of
    /// `current_position - spring_rest_position`. Applied once per step as a force rather
    /// than iterated, and — unlike the motor — it counts toward [`Joint::break_force`].
    pub spring_stiffness: f32,
    /// Suspension damping in newtons per m/s of along-axis relative anchor speed. `0.0`
    /// leaves the spring undamped; because it damps *relative* motion rather than motion
    /// away from the rest position, it also resists the motor and any external drive along
    /// the axis.
    pub spring_damping: f32,
    /// Along-axis offset in metres at which the suspension exerts no force, on the same
    /// scale as [`current_position`](Self::current_position). Independent of the travel
    /// limits: a rest position outside `[lower_limit, upper_limit]` just means the spring
    /// pushes the joint into its hard stop and stays there.
    pub spring_rest_position: f32,
    /// Solver **output** in metres: the along-axis component of (anchor B − anchor A),
    /// rewritten on every solver pass and read by the servo. `#[serde(skip)]` runtime state;
    /// the suspension spring recomputes the same quantity itself rather than reading this.
    /// With no [`axis`](Self::axis) there is no along-axis component and this reads `0.0`
    /// (it used to read NaN — normalising a zero axis — and latch that into public state).
    #[serde(skip)]
    pub current_position: f32,
    /// Relative rotation (`rot_a⁻¹ · rot_b`) latched on the first solve and then held as the
    /// target of the slider's angular lock — a prismatic joint frees translation only, never
    /// rotation.
    ///
    /// `None` = not latched yet; the solver iteration that latches it applies no angular
    /// correction (the translational rows still run).
    /// `#[serde(default)]`, so a scene file omitting it re-latches from the loaded pose.
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

/// Soft spring-damper between the two anchors — a **force**, not a constraint.
///
/// It is applied once per step outside the velocity iteration loop (a position-dependent
/// force applied per iteration would be multiplied by the iteration count), it removes no
/// degree of freedom, and a strong enough load stretches it arbitrarily far. Use
/// [`DistanceJointData`] when the separation must actually be bounded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct SpringJointData {
    /// Anchor-to-anchor distance in metres at which the spring force is zero: a larger
    /// separation pulls the bodies together, a smaller one pushes them apart. The joint is
    /// inert while the anchors are within ~1 µm of each other, where the direction is
    /// undefined.
    pub rest_length: f32,
    /// Spring constant in newtons per metre of deviation from
    /// [`rest_length`](Self::rest_length). A negative value inverts the equilibrium: the
    /// spring then pushes *away* from the rest length instead of returning to it. Nothing
    /// here stabilises a stiff spring implicitly, so the usable stiffness is bounded by the
    /// step size in the usual explicit-integration way.
    pub stiffness: f32,
    /// Damping in newtons per m/s of the anchors' approach/separation speed. It acts only
    /// along the anchor-to-anchor direction; motion perpendicular to that is untouched, so a
    /// spring joint never damps swinging about its own axis.
    pub damping: f32,
    /// Lower gate in metres: at or below this separation the spring's *pulling-together*
    /// impulse is suppressed, while its pushing half still acts. This
    /// gates the spring's own force only — it is not a floor, and other forces can bring the
    /// bodies closer. [`Joint::spring`] sets `0.0`, which in practice never engages.
    pub min_length: f32,
    /// Upper gate in metres, `None` (the constructor default) = no gate. At or beyond it the
    /// spring's *pushing-apart* impulse is suppressed while the pull stays free to bring the
    /// bodies back. Like [`min_length`](Self::min_length) this only gates the spring's own
    /// force and does not constrain the bodies.
    pub max_length: Option<f32>,
}

/// Distance/rope joint: keeps the anchor separation within `[min_length, max_length]`
/// as a HARD (inequality) constraint — unlike `Spring`, which is a soft force toward a
/// rest length. A **rope** is `{min: 0, max: L}`: it only pulls when taut (`len > L`)
/// and is limp when slack (`len < L`), so a released slack body free-falls until the
/// rope catches it — no rigid-rod snap. A **rigid rod** is `{min: L, max: L}`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct DistanceJointData {
    /// Lower bound on the anchor separation, in metres. `0.0` lets the rope go fully slack;
    /// a positive value adds a strut that pushes the anchors apart once they come closer
    /// than this. [`Joint::distance`] clamps a negative argument to `0.0`.
    pub min_length: f32,
    /// Upper bound on the anchor separation, in metres: the joint pulls only once the
    /// separation exceeds it.
    ///
    /// Keep it ≥ [`min_length`](Self::min_length). [`Joint::distance`] enforces that, but
    /// assigning the fields directly does not, and the upper branch is tested first — so
    /// where an inverted pair makes the two regions overlap, only the pull acts.
    pub max_length: f32,
    /// Inverse stiffness: 0 = rigid rope/rod (hard bounds); larger = a stretchy, elastic
    /// rope that gives under load.
    ///
    /// Literally `1/K` in metres per newton: a load of `F` stretches the rope by
    /// `F · compliance`, so hanging 1 kg from a rope with `compliance = 0.03` settles
    /// 0.294 m past its rest length (`0.03 · 1 · 9.81`). Asserted in
    /// `tests/joint_compliance.rs`.
    pub compliance: f32,
}

/// Per-degree-of-freedom mode for the generic 6-DOF ([`D6JointData`]) joint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub enum D6Motion {
    /// Fully constrained (0 relative motion on this axis) — the Fixed-joint behaviour.
    #[default]
    Locked,
    /// Unconstrained — free to translate/rotate on this axis (Slider/Hinge behaviour).
    Free,
    /// Constrained to `[lower, upper]` on this axis (a limited slider/hinge).
    Limited {
        /// Lower bound: metres for a translational axis, radians for a rotational one.
        /// Measured from the joint's reference — zero anchor-to-anchor offset along the axis
        /// for a linear DOF, the latched `initial_relative_rotation` for an angular one — so
        /// these are never absolute world values.
        lower: f32,
        /// Upper bound, same units and reference as `lower`; keep it ≥ `lower`. The two are
        /// exclusive branches of one solver row, so an inverted pair does not error, it just
        /// leaves one side unenforced.
        upper: f32,
    },
}

/// Per-axis DRIVE for a [`D6JointData`]: a spring-damper toward a target that unifies a
/// motor (`damping` pulls the velocity toward `target_velocity`) and a spring (`stiffness`
/// pulls the position toward `target_position`), force-limited by `max_force` (≤0 ⇒
/// unlimited). PhysX-D6-style. `enabled: false` (the default) ⇒ no drive on that axis.
// Exhaustive (a plain config value users build with a struct literal), like PhysicsMaterial.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct D6Drive {
    /// Off by default; a disabled drive is skipped entirely, so its other fields are inert.
    /// A drive is independent of the [`D6Motion`] mode of the same axis — driving a `Locked`
    /// axis just makes the drive fight the lock.
    pub enabled: bool,
    /// Spring term pulling the DOF toward [`target_position`](Self::target_position):
    /// newtons per metre on a translational axis, newton-metres per radian on a rotational
    /// one. `0.0` leaves a pure motor (velocity-only) drive.
    pub stiffness: f32,
    /// Damping term pulling the DOF's rate toward
    /// [`target_velocity`](Self::target_velocity): newtons per m/s (translational) or
    /// newton-metres per rad/s (rotational).
    ///
    /// This is the term that makes a drive behave as a motor — with `stiffness = 0` and a
    /// nonzero target velocity it accelerates the DOF toward that velocity — and `0.0` here
    /// leaves the spring term completely undamped.
    pub damping: f32,
    /// Rest point of the spring term: metres of anchor offset along the axis (translational)
    /// or radians away from the latched reference pose (rotational). Irrelevant while
    /// [`stiffness`](Self::stiffness) is zero.
    pub target_position: f32,
    /// Velocity the damping term drives toward: m/s along the axis or rad/s about it, of B
    /// relative to A. Irrelevant while [`damping`](Self::damping) is zero.
    pub target_velocity: f32,
    /// Clamp on the drive's total output — newtons (translational) or newton-metres
    /// (rotational).
    ///
    /// **Zero or negative means UNLIMITED**, not "disabled"; use
    /// [`enabled`](Self::enabled)`= false` to switch a drive off. `Default` leaves this at
    /// `0.0`, i.e. an unlimited drive.
    pub max_force: f32,
}

/// Generic 6-DOF (D6) joint: per-axis Lock / Free / Limited over 3 translational + 3
/// rotational DOFs, in a configurable local frame. Subsumes Fixed (all locked), Slider
/// (one linear Free/Limited), Hinge (one angular Free/Limited) and hybrids (universal,
/// cylindrical, planar) — the modern default joint (PhysX D6 / Rapier GenericJoint).
/// Pure orchestration of the existing 1-DOF constraint primitives.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[non_exhaustive]
pub struct D6JointData {
    /// Local frame (in A's space) whose X/Y/Z axes define the six DOFs. Identity = A's axes.
    pub frame: Quat,
    /// Translation modes along the frame's X, Y, Z axes.
    pub linear: [D6Motion; 3],
    /// Rotation modes about the frame's X, Y, Z axes.
    pub angular: [D6Motion; 3],
    /// Optional spring-damper drives (motor+spring) per translational axis.
    pub linear_drives: [D6Drive; 3],
    /// Optional spring-damper drives (motor+spring) per rotational axis.
    pub angular_drives: [D6Drive; 3],
    /// Inverse stiffness for every locked/limited DOF (0 = rigid). `1/K`, in the units of
    /// the DOF it applies to — see [`DistanceJointData::compliance`].
    pub compliance: f32,
    /// Reference pose (`rot_a⁻¹ · rot_b`) latched on the first solve; every angular
    /// lock, limit and drive is measured from it, so the authored pose is the zero of all
    /// three angular DOFs.
    ///
    /// `None` = not latched yet: the iteration that latches it builds no angular row, and
    /// while it is still `None` the angular *drives* are skipped too (the linear drives run
    /// regardless). `#[serde(default)]`, so a scene file omitting it re-latches from the
    /// loaded pose.
    #[serde(default)]
    pub initial_relative_rotation: Option<Quat>,
}

impl Joint {
    /// Kind name of [`data`](Self::data) for display and log fields — `"Fixed"`, `"Hinge"`,
    /// `"BallSocket"`, `"Slider"`, `"Spring"`, `"Distance"` or `"D6"`.
    ///
    /// For control flow prefer [`JointType`], which the compiler checks exhaustively; these
    /// strings are meant to be read by humans, not matched on.
    pub fn joint_type(&self) -> &'static str {
        match &self.data {
            JointData::Fixed => "Fixed",
            JointData::Hinge(_) => "Hinge",
            JointData::BallSocket(_) => "BallSocket",
            JointData::Slider(_) => "Slider",
            JointData::Spring(_) => "Spring",
            JointData::Distance(_) => "Distance",
            JointData::D6(_) => "D6",
        }
    }

    /// Welds the two bodies at the given local anchors: the anchor points are pinned
    /// together and relative rotation is locked on all three axes.
    ///
    /// Anchors are in each body's own local frame, relative to its transform origin (see
    /// [`local_anchor_a`](Self::local_anchor_a)). Defaults: unbreakable
    /// (`break_force`/`break_torque` = `f32::INFINITY`) and collisions between the pair
    /// disabled. The two handles must differ — `debug_assert`ed here, and in release a
    /// self-joint is simply skipped by the solver.
    pub fn fixed(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::Fixed,
        }
    }

    /// Revolute joint about `axis`: the anchors are pinned together and rotation is free
    /// about that one axis.
    ///
    /// `axis` is normalised here, and is interpreted in **both** bodies' local frames (see
    /// [`HingeJointData::axis`]); an argument shorter than ~1e-3 is replaced by `Vec3::Y`
    /// rather than yielding a degenerate joint. Limits, motor and torsional spring all start
    /// off, with the limit range seeded to `[-π, π]`; unbreakable and non-colliding like
    /// [`Self::fixed`].
    pub fn hinge(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        let safe_axis = if axis.length_squared() > 1e-6 {
            axis.normalize()
        } else {
            Vec3::Y
        };
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::Hinge(HingeJointData {
                axis: safe_axis,
                use_limits: false,
                lower_limit: -std::f32::consts::PI,
                upper_limit: std::f32::consts::PI,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                motor_is_servo: false,
                motor_target_position: 0.0,
                use_torsional_spring: false,
                torsional_stiffness: 0.0,
                torsional_damping: 0.0,
                rest_angle: 0.0,
                current_angle: 0.0,
            }),
        }
    }

    /// Spherical joint: the anchors are pinned together and all three rotational DOF are
    /// free.
    ///
    /// Every limit starts disabled — `twist_axis` seeded `Vec3::Y`, cone and swing bounds
    /// `π`, twist `±π`, `compliance` 0 (hard limits) — and no rest pose is latched until one
    /// of them is switched on, see
    /// [`BallSocketJointData::initial_relative_rotation`]. Unbreakable and non-colliding like
    /// [`Self::fixed`].
    pub fn ball_socket(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::BallSocket(BallSocketJointData {
                use_cone_limit: false,
                cone_limit_angle: std::f32::consts::PI,
                use_twist_limit: false,
                twist_axis: Vec3::Y,
                twist_lower: -std::f32::consts::PI,
                twist_upper: std::f32::consts::PI,
                use_swing_limits: false,
                swing_limit_1: std::f32::consts::PI,
                swing_limit_2: std::f32::consts::PI,
                compliance: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    /// Prismatic joint along `axis`: B may only translate along that direction relative to
    /// A, and relative rotation is locked to the pose latched on the first solve.
    ///
    /// `axis` is normalised here and taken in **A's** local frame (`Vec3::Y` substituted for
    /// an argument shorter than ~1e-3). Travel limits are seeded to ±10 m but start
    /// disabled, and the motor and suspension spring start off. Unbreakable and
    /// non-colliding like [`Self::fixed`].
    pub fn slider(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        axis: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        let safe_axis = if axis.length_squared() > 1e-6 {
            axis.normalize()
        } else {
            Vec3::Y
        };
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::Slider(SliderJointData {
                axis: safe_axis,
                use_limits: false,
                lower_limit: -10.0,
                upper_limit: 10.0,
                use_motor: false,
                motor_target_velocity: 0.0,
                motor_max_force: 0.0,
                motor_is_servo: false,
                motor_target_position: 0.0,
                use_spring: false,
                spring_stiffness: 0.0,
                spring_damping: 0.0,
                spring_rest_position: 0.0,
                current_position: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    /// Damped spring between the two anchors: `rest_length` in metres, `stiffness` in N/m,
    /// `damping` in N per m/s.
    ///
    /// None of the three is validated — negative stiffness gives a spring that pushes away
    /// from `rest_length` instead of returning to it. This constrains nothing (see
    /// [`SpringJointData`]); `min_length` starts at `0.0` and `max_length` at `None`, so the
    /// spring's own force gates are inactive. Unbreakable and non-colliding like
    /// [`Self::fixed`].
    pub fn spring(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        rest_length: f32,
        stiffness: f32,
        damping: f32,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::Spring(SpringJointData {
                rest_length,
                stiffness,
                damping,
                min_length: 0.0,
                max_length: None,
            }),
        }
    }

    /// Distance joint: constrains the anchor separation to `[min_length, max_length]`
    /// as a hard inequality. `min == max` ⇒ rigid rod; `min == 0` ⇒ rope (see [`Self::rope`]).
    pub fn distance(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        min_length: f32,
        max_length: f32,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::Distance(DistanceJointData {
                min_length: min_length.max(0.0),
                max_length: max_length.max(min_length.max(0.0)),
                compliance: 0.0,
            }),
        }
    }

    /// Rope: inextensible but can go slack. The anchors cannot separate beyond `length`
    /// (pulls when taut), but may come closer (limp when slack) — a released slack body
    /// free-falls until the rope catches, with no rigid-rod snap. Shorthand for
    /// `distance(.., 0.0, length)`.
    pub fn rope(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
        length: f32,
    ) -> Self {
        Self::distance(entity_a, entity_b, local_anchor_a, local_anchor_b, 0.0, length)
    }

    /// Generic 6-DOF joint. Starts fully locked (a weld); set `data.linear[i]` /
    /// `data.angular[i]` to [`D6Motion::Free`]/[`D6Motion::Limited`] to open DOFs — e.g. one
    /// angular axis Free ⇒ hinge, one linear axis Free ⇒ slider. `frame` (in A's space)
    /// orients the six axes.
    pub fn d6(
        entity_a: BodyHandle,
        entity_b: BodyHandle,
        local_anchor_a: Vec3,
        local_anchor_b: Vec3,
    ) -> Self {
        debug_assert_ne!(
            entity_a, entity_b,
            "Joint: entity_a and entity_b must be different"
        );
        Self {
            entity_a,
            entity_b,
            local_anchor_a,
            local_anchor_b,
            break_force: f32::INFINITY,
            break_torque: f32::INFINITY,
            is_broken: false,
            scratch: JointScratch::default(),
            collision_enabled: false,
            data: JointData::D6(D6JointData {
                frame: Quat::IDENTITY,
                linear: [D6Motion::Locked; 3],
                angular: [D6Motion::Locked; 3],
                linear_drives: [D6Drive::default(); 3],
                angular_drives: [D6Drive::default(); 3],
                compliance: 0.0,
                initial_relative_rotation: None,
            }),
        }
    }

    /// Sets both break thresholds: `force` in newtons and `torque` in newton-metres, each
    /// compared once per solver pass against the joint's net reaction — see
    /// [`break_force`](Self::break_force) for exactly what is measured and what is excluded.
    ///
    /// Pass `f32::INFINITY` for "never breaks on this axis"; a merely large number still
    /// breaks under a large enough impulse spike, which for a stiff joint can arrive in a
    /// single sub-step.
    pub fn with_break_force(mut self, force: f32, torque: f32) -> Self {
        self.break_force = force;
        self.break_torque = torque;
        self
    }

    /// Sets [`collision_enabled`](Self::collision_enabled) — see it for why every constructor
    /// leaves it `false`. Pass `true` when the joined pieces are far enough apart that contact
    /// between them is meaningful, the two ends of a long rope being the usual case.
    pub fn with_collision(mut self, enabled: bool) -> Self {
        self.collision_enabled = enabled;
        self
    }

    /// Discard the solver's accumulated warm-start impulses for this joint, so the next pass
    /// starts cold.
    ///
    /// Call it after assigning [`data`](Self::data): replacing the payload discards the latched
    /// reference pose, but the λ accumulated by the OLD configuration's rows stays in the
    /// scratch and would be replayed along the new configuration's directions on the next
    /// pass. Nothing else needs it — a joint whose gate closes has its row pruned
    /// automatically at the end of the pass.
    pub fn reset_warm_start(&mut self) {
        self.scratch = JointScratch::default();
    }

    /// Latches [`is_broken`](Self::is_broken) when `applied_force` (newtons) exceeds
    /// [`break_force`](Self::break_force) or `applied_torque` (newton-metres) exceeds
    /// [`break_torque`](Self::break_torque), and reports whether either did.
    ///
    /// Both comparisons are strict `>`, so exactly meeting a threshold does not break the
    /// joint and NaN arguments never do. The latch is one-way: this never clears
    /// `is_broken`, and it does not check whether the joint was already broken, so calling
    /// it again on a broken joint still returns `true`.
    ///
    /// The solver already calls this once per pass with that pass's net reaction; call it
    /// yourself only when driving a joint outside that solver, and note that a torque
    /// exceeding the threshold breaks the joint even if the force is nowhere near its own.
    pub fn check_break(&mut self, applied_force: f32, applied_torque: f32) -> bool {
        if applied_force > self.break_force {
            self.is_broken = true;
            return true;
        }
        if applied_torque > self.break_torque {
            self.is_broken = true;
            return true;
        }
        false
    }
}
