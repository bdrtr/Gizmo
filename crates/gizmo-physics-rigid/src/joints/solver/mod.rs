use super::data::*;
use gizmo_physics_core::components::Transform;
use crate::components::{RigidBody, Velocity};
use gizmo_math::{Quat, Vec3};

/// Accumulated-λ slots ([`JointScratch`]).
///
/// The slots are COMPILE-TIME CONSTANTS, not an advancing cursor: most rows are conditionally
/// skipped (`err_len >= 1e-4` in `fixed.rs`, `err_mag > 1e-6` and the limit branches in
/// `hinge.rs`, `err.abs() > 1e-4` in `slider.rs`, the cone/twist/swing gates in
/// `ball_socket.rs`, the `continue` arms in `d6.rs`), and a cursor would shift the identity of
/// every following row each time one was skipped — writing λs into the wrong row.
///
/// A DOF's LOWER and UPPER limit share one slot: two constraints but a single degree of
/// freedom, and since `transforms` does NOT change during a pass (the solver takes
/// `&[Transform]`, integration happens after the pass), which branch is taken stays fixed for
/// all 10 iterations — a stale λ of the opposite sign cannot be inherited.
pub(crate) mod row {
    /// 0,1,2 — the point constraint's X/Y/Z, a slider's two perpendicular axes, D6's linear DOFs.
    pub const LIN: usize = 0;
    /// 3,4,5 — Fixed's 3-axis angular lock, D6's angular DOFs, a hinge's axis alignment, a
    /// slider's angular lock, ball-socket cone (3) and twist (4).
    pub const ANG: usize = 3;
    /// Hinge/slider limit, distance min|max — all of them the two-sided bound of a single DOF.
    pub const LIMIT: usize = 6;
    /// 7,8 — ball-socket asymmetric swing limits (perp1, perp2).
    pub const SWING: usize = 7;
    /// 9 — the motor / servo row. It does NOT count towards the breaking total: a motor is an
    /// actuator, not an external load.
    pub const MOTOR: usize = 9;
}

/// Sequential-impulse (Gauss–Seidel) solver for a world's joint array.
///
/// Configuration only: every field is a tuning scalar, which is why the type is `Copy` and
/// why one instance can solve any number of joints — the accumulated impulses of a pass are
/// stored on each [`Joint`] and reset at the start of every [`Self::solve_joints`] call.
///
/// The rows of one joint are applied one after another and each sees the velocities the
/// previous row already wrote (that is what makes it Gauss–Seidel rather than a block
/// solve), so a chain of joints converges over [`Self::iterations`] sweeps instead of being
/// satisfied exactly, and the answer depends on the order of the joint slice.
///
/// Every field is simulation input: changing one changes the solved velocities, so a replay
/// or a rollback must run with the same values that were recorded with. Bit-equality is
/// same-platform only.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct JointSolver {
    /// Gauss–Seidel sweeps over the whole joint array per [`Self::solve_joints`] call
    /// (default 10).
    ///
    /// Velocity-level rows only. The force-based contributions — the Spring joint, the
    /// slider suspension and hinge torsional springs, the D6 drives — are applied once per
    /// call outside this loop, so raising the count does not multiply a spring force, nor
    /// the force a joint reports for breaking (see [`Self::solve_joints`] § Breaking;
    /// `tests/joint_break.rs` guards it).
    ///
    /// A quality knob, not a physical one: more sweeps buy a closer approach to the rigid
    /// answer at linear cost. `0` is legal and skips constraint solving entirely, while the
    /// force-based pass and the break check still run.
    pub iterations: usize,
    /// Ceiling on the positional-repair velocity of a LINEAR row, in metres per second
    /// (default 5).
    ///
    /// It clamps the bias term only — the part of the target velocity that comes from
    /// position error — and never the row's response to actual relative velocity, so it is
    /// not a cap on the impulse a joint may apply. It binds at `max_correction_speed /
    /// bias_rate` of error, whatever produced that rate: **≈2.9 cm** on the default soft
    /// rigid path ([`Self::rigid_hertz`] = 200 at the world's 1/240 s substep, bias rate
    /// 173.7 s⁻¹), and 6.9 cm on the legacy Baumgarte path (`β/dt` = 72 s⁻¹). Below that it
    /// is inert.
    ///
    /// Must be ≥ 0: it is used as `bias.clamp(-self, self)`, which panics outright when the
    /// value is negative. `0` leaves the linear rows as pure velocity constraints that hold
    /// the error where it is but never work it off. Motor and drive rows do not pass through
    /// this clamp at all.
    pub max_correction_speed: f32,
    /// The angular counterpart of [`Self::max_correction_speed`], in radians per second
    /// (default 5), applied to every angular row: the Fixed 3-axis lock, hinge axis
    /// alignment, the ball-socket cone/twist/swing limits and the D6 angular DOFs.
    ///
    /// Same non-negativity requirement, same bias-only reach. It bounds the angular error a
    /// row can work off in one substep by roughly `max_angular_speed · dt`, so a joint that
    /// starts far from its target orientation rotates into place over several substeps
    /// rather than snapping. Binding threshold, as above: **≈1.65°** on the default soft
    /// rigid path, 4.0° on the legacy Baumgarte path.
    pub max_angular_speed: f32,
    /// Baumgarte factor β of the LEGACY rigid path — dimensionless, default 0.3.
    ///
    /// Two live roles, and neither is a deprecation stub:
    ///
    /// 1. **The legacy rigid path**, selected by setting [`Self::rigid_hertz`] to `0`. A row
    ///    with positional error `C` is given the target velocity `β·C/dt` (then clamped by
    ///    the two ceilings above), so once the row converges roughly the fraction β of the
    ///    remaining error is worked off over that substep. `0` turns every rigid row into a
    ///    pure velocity constraint: the error stops growing but is never removed. `1` asks
    ///    for all of it inside a single substep. This is the supported rollback lever for
    ///    the soft rigid rows — `golden_state.rs` locks a scene against it.
    /// 2. **The hinge and slider POSITION SERVOS** reuse it as their proportional gain when
    ///    converting a target angle/offset into a target velocity, on BOTH paths — retuning
    ///    β retunes how hard those servos chase their target.
    ///
    /// Rows with `compliance > 0`, and rigid rows on the default `rigid_hertz > 0` path,
    /// derive their bias rate from a frequency instead and ignore role 1.
    pub position_bias: f32,
    /// Damping ratio ζ of EVERY soft row's spring–damper, default 1.0 (critical damping: the
    /// row settles on its target without overshoot).
    ///
    /// Named for compliance because that is where it started, but since rigid rows became
    /// soft rows parameterised by [`Self::rigid_hertz`] it governs them too — the field name
    /// is now narrower than its reach, and renaming it would be a breaking change on a
    /// `#[non_exhaustive]` struct mid-hardening for nothing but a nicer identifier.
    ///
    /// It does NOT enter a row's static stiffness (that is `ω²` alone); it divides both the
    /// feedback term and the repair rate. Copying the contact solver's ζ = 10 onto joints at
    /// its `contact_hertz` = 30 would give `ω²` = 3.6e4 — a 1 kg rope sagging 2.8e-4 m and a
    /// 16-link chain sagging 1.8 m. That naive mirror is why the two subsystems differ here.
    pub compliance_damping_ratio: f32,
    /// Soft-constraint frequency of the RIGID rows (`compliance == 0`), in hertz — default
    /// 200, clamped to `1/dt`. `0` (or negative) selects the legacy Baumgarte path.
    ///
    /// A rigid row is not solved as a hard equality with a Baumgarte push; it is solved as a
    /// very stiff spring–damper at this frequency, the same Box2D-v3 soft formulation the
    /// contact solver uses (see `soft_coefficients` below and `solver/tgs.rs`). The
    /// observable contract is a closed form:
    ///
    /// ```text
    /// static constraint error = a / ω²,   ω = 2π · min(rigid_hertz, 1/dt)
    /// ```
    ///
    /// where `a` is the acceleration the row has to hold off (`g` for a hanging load). At the
    /// default 200 Hz that is 6.2 µm per `g` — **mass-, iteration- and (below the clamp)
    /// dt-independent**, none of which a Baumgarte row or a compliance can express.
    ///
    /// **"Below the clamp" is load-bearing for EMBEDDERS.** [`PhysicsWorld`](crate::PhysicsWorld)
    /// always substeps at 1/240 s, so 200 Hz is under its `1/dt` = 240 and the law holds as
    /// written. Calling [`Self::solve_joints`] directly at a coarser step does not get the same
    /// joint: at `dt = 1/60` the row runs at 60 Hz and the static error is `(200/60)² ≈ 11×`
    /// larger. The old Baumgarte row had no such cliff (it converged to zero error at every
    /// `dt`), so this is new. Step at 1/240 or lower `rigid_hertz` deliberately.
    ///
    /// **Why it exists.** The hard path has no `−impulse_scale·λ` feedback term. That is what
    /// destroyed joint warm start at its natural factor of 1.0 (a 16-link chain with a 200 kg
    /// tip settled at 4.83 m against a converged 16.00, with 44 m/s of residual motion) while
    /// the identical chain at `compliance = 1e-6` stayed stable. See `docs/ENGINE.md` §7
    /// commit 5.
    ///
    /// Be careful with the mechanism, because the obvious story is wrong and was believed here
    /// for a while: it is NOT that a row accumulates its own Baumgarte residual pass after
    /// pass. Write the update as `λ' = (1 − mass_scale − impulse_scale)·λ + mass_scale·(b − v₀)/k`,
    /// where `v₀` is the relative velocity this row is not responsible for. On the hard path
    /// `mass_scale = 1, impulse_scale = 0`, so the coefficient on `λ` is already ZERO — a hard
    /// row re-derives its λ from scratch every iteration and has no memory of its own. The
    /// memory that blows up lives in `v₀`, i.e. in the Gauss–Seidel coupling between rows of an
    /// ill-conditioned chain. What the feedback term buys is a converged system of
    /// `J·v = b − (impulse_scale/mass_scale)·k·λ` — CFM with `α̃ = k·impulse_scale/mass_scale`,
    /// a compliance proportional to the row's own `k`, which is what makes it a constant-
    /// FREQUENCY softening and is why a frequency is the honest way to parameterise it.
    ///
    /// **It deliberately diverges from [`ConstraintSolver`](crate::ConstraintSolver)'s
    /// contact settings** — 200 Hz not 30, ζ = 1 not 10, and the ceiling is `1/dt` not
    /// `0.25/dt`. A contact carries roughly its own body's weight; a joint can carry 400×
    /// that, so the same numbers would sag visibly. The reasoning is at `rigid_coefficients`.
    ///
    /// **Cost, stated plainly:** a rigid row is now a spring, so it is strictly softer than
    /// the Baumgarte row was, which converged to zero error given enough iterations. On the
    /// 200 kg chain this raises the converged floor by ≈39 mm and no iteration count removes
    /// it. `tests/joint_rigid_stiffness.rs` is what bounds the loss.
    pub rigid_hertz: f32,
    /// Fraction of the previous substep's λ injected before iteration 0, default **0.0 (off)**.
    ///
    /// **Whether to ship this on is NOT yet decided.** It is committed at `0.0` so the
    /// measurement phase can sweep it without a throwaway patch; the default is inert and
    /// every committed scene runs cold. Do not raise it in library code.
    ///
    /// A warm start hands the solver the answer it converged to last substep, so ten warm
    /// sweeps land far closer to the rigid answer than ten cold ones on an ill-conditioned
    /// chain. It previously destroyed such a chain at factor 1.0; [`Self::rigid_hertz`] is
    /// the fix that diagnosis pointed at, and this knob is how that is measured.
    ///
    /// Injection is a SEPARATE sweep before iteration 0, never in place: this solver clamps
    /// the accumulated TOTAL, so `λ_prev + (−Jv_pre − k·λ_prev + bias)/k = (−Jv_pre + bias)/k`
    /// — in-place injection cancels exactly and is an algebraic no-op (measured; see
    /// `docs/ENGINE.md`). With a non-zero factor the λ of a pass becomes carried simulation
    /// state, which `WorldSnapshot` already covers (it clones the joints).
    ///
    /// Two things the injection sweep does NOT cover, both harmless and both worth knowing
    /// before reading a measurement: the MOTOR rows are hand-rolled and call `accumulate`
    /// directly rather than going through the shared helpers, so they simply run a normal
    /// solve during the injection sweep — i.e. a warm-started motor gets `iterations + 1`
    /// refining sweeps of the same budgeted total, not a multiplied force
    /// (`tests/joint_motor.rs` is what holds that). And the force-based pass — Spring, slider
    /// suspension, hinge torsional, D6 drives — sits outside the loop entirely and is
    /// untouched.
    /// **Default 0.5 since 2026-08-17, and the number is measured, not taste.** A 16-link chain,
    /// 10 iterations, settled (2000 substeps of 1/240 s), constraint error at the tip:
    ///
    /// | tip mass | warm 0 | warm 0.5 | warm 0 + 11 iter | warm 0 + 20 iter |
    /// |---|---|---|---|---|
    /// | 20 kg | 0.01249 m | **0.00881** | 0.01162 | 0.00783 |
    /// | 200 kg | 0.10359 m | **0.06254** | 0.08328 | 0.05888 |
    ///
    /// The injection sweep costs about one iteration, and buys roughly four times what an extra
    /// plain iteration buys; at 200:1 it delivers what ten extra iterations would. What it costs
    /// is residual motion — at 200 kg the chain's `max|v|` goes 0.0116 → 0.0399 m/s, and at
    /// **1.0** it goes to 0.19 m/s while the error stops improving (0.087, worse than 0.5). That
    /// is why the default is a half and not a whole: past ~0.5 this is buying jitter.
    ///
    /// Ordinary mass ratios pay nothing for it — at 1 and 20 kg the residual velocity is
    /// unchanged at 1e-4 m/s and the error still drops ~30 %.
    pub warm_start_factor: f32,
}

impl Default for JointSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            max_correction_speed: 5.0,
            max_angular_speed: 5.0,
            position_bias: 0.3,
            compliance_damping_ratio: 1.0,
            rigid_hertz: 200.0,
            warm_start_factor: 0.5,
        }
    }
}

/// Both ends of this joint are immobile, so the joint has nothing to solve this pass.
///
/// A DYNAMIC body is inert when it is asleep; anything else (static, kinematic) when it is not
/// moving. The threshold is deliberately the same `1e-8` the joint-graph wake pass in
/// `pipeline.rs` uses for a kinematic mover, so the two agree about what "moving" means.
///
/// NOT a plain `is_sleeping` on both ends: [`RigidBody::new_static`] presets `is_sleeping`, and
/// essentially every scene in this repo calls `anchor.wake_up()` on its static anchor, so
/// `is_sleeping(a) && is_sleeping(b)` would never fire on the one joint that matters — the
/// anchor↔first-link joint of a sleeping chain.
///
/// Stable for the duration of a `solve_joints` call: the joint solver never writes a
/// non-dynamic body's velocity and never touches `is_sleeping`, so every phase re-evaluating
/// this predicate gets the same answer.
#[inline]
fn joint_is_inert(
    rigid_bodies: &[RigidBody],
    velocities: &[Velocity],
    idx_a: usize,
    idx_b: usize,
) -> bool {
    let inert = |i: usize| -> bool {
        let rb = &rigid_bodies[i];
        if rb.is_dynamic() {
            rb.is_sleeping
        } else {
            velocities[i].linear.length_squared() <= 1e-8
                && velocities[i].angular.length_squared() <= 1e-8
        }
    };
    inert(idx_a) && inert(idx_b)
}

impl JointSolver {
    /// A solver running `iterations` sweeps per call, with every other knob left at its
    /// [`Default`] value (5 m/s and 5 rad/s bias ceilings, β = 0.3, ζ = 1).
    ///
    /// `iterations` is not validated — see the field for what `0` means. Since the struct is
    /// `#[non_exhaustive]`, this and `Default` are the only ways to build one from outside
    /// the crate; the remaining knobs are then set by assigning to the public fields.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    /// Solve every joint for one substep, writing the result into `velocities`.
    ///
    /// `dt` is the substep length in SECONDS and must be > 0: it sets every row's soft
    /// coefficients (and, on the legacy path, divides the Baumgarte bias `β·C/dt`) and
    /// converts accumulated impulses back into the forces and torques that are compared
    /// against the break thresholds. `transforms` is read-only — positions are
    /// integrated later in the step — so the positional error is frozen for the duration of
    /// the call and the correction only becomes visible after integration.
    ///
    /// `rigid_bodies`, `transforms` and `velocities` are parallel arrays over the same index
    /// space, and `entity_index_map` maps a [`BodyHandle`](gizmo_physics_core::BodyHandle)
    /// id to that index. A joint is skipped in silence when it is already broken, when
    /// either endpoint is absent from the map, or when both endpoints resolve to the same
    /// index; a mapped index that is out of range for the slices panics.
    ///
    /// It is skipped too when BOTH ends are inert — a sleeping dynamic body, or a
    /// static/kinematic body that is not moving. The solver used to write velocities into
    /// sleeping bodies that position integration then discarded, so this costs nothing that
    /// was reaching the simulation, but two consequences are user-visible. An embedder calling
    /// this without a wake pass of its own now gets *nothing* solved for a mechanism whose
    /// bodies are all asleep — [`PhysicsWorld`](crate::PhysicsWorld) runs a joint-graph wake
    /// pass immediately before this call, so any component containing a mover is already awake
    /// when the gate is evaluated, but a bare `solve_joints` user has to arrange that. And a
    /// joint whose bodies fall asleep stops reporting load, so it can no longer break — which
    /// matches what already happens when a contact island sleeps.
    ///
    /// `dt` must be finite and strictly positive. A zero, negative, infinite or NaN `dt`
    /// clears the accumulated impulses and returns without touching a velocity or a break
    /// flag — no time passed, so no impulse was delivered and nothing can have broken. (It
    /// used to run the whole solve: the break check's `impulse/dt` was `+inf`, which exceeds
    /// every finite `break_force`, so one zero-length call snapped every breakable joint in
    /// the world at once.)
    ///
    /// Three phases, in order:
    ///
    /// 1. [`Self::iterations`] Gauss–Seidel sweeps of the velocity-level rows, in joint
    ///    order — preceded by one λ-injection sweep when [`Self::warm_start_factor`] is
    ///    non-zero, which it is by default (0.5).
    /// 2. One pass of the force-based contributions — the Spring joint, the slider
    ///    suspension and hinge torsional springs, the D6 drives. These depend on position
    ///    rather than velocity, so running them inside the loop above would apply them
    ///    `iterations` times over.
    /// 3. One break check per joint.
    ///
    /// Only the velocities of DYNAMIC bodies are written; static and kinematic ones act as
    /// boundary conditions and are read but never modified. Sleep state is not consulted: a
    /// sleeping dynamic body's velocity is written like any other, and taking `&[RigidBody]`
    /// means this cannot wake it — the caller has to, otherwise position integration skips
    /// the body and the correction is discarded.
    ///
    /// # Breaking
    ///
    /// Break is judged once per call on the pass's NET impulse: `‖Σ λᵢnᵢ‖ / dt` against
    /// `break_force` in newtons, and the angular equivalent against `break_torque` in N·m.
    /// Because it is the vector sum and not a sum of row magnitudes, the three orthogonal
    /// linear rows of a Fixed joint carrying one diagonal load report that load rather than
    /// up to √3 times it (`tests/joint_break.rs` guards the diagonal case). Constraint
    /// rows and the force-based springs feed the sum; motors and D6 drives deliberately do
    /// not, an actuator being an input rather than an external load. A joint that breaks is
    /// marked and skipped by every later call — nothing here un-breaks it.
    ///
    /// # Determinism
    ///
    /// The accumulated impulses are cleared at the start of every call, so a pass never
    /// inherits them from the previous one — unless [`Self::warm_start_factor`] is non-zero,
    /// which it is by default, and that is exactly what makes λ carried simulation state (`WorldSnapshot` clones the
    /// joints, so a rollback already carries it). The latched state on the joints themselves
    /// does carry over regardless — `is_broken` and the reference poses survive every call. The map is only
    /// ever looked up in, never iterated, so its hash order does not reach the result; the
    /// order of `joints` does. Single-threaded and same-platform bit-reproducible only.
    pub fn solve_joints(
        &self,
        joints: &mut [Joint],
        entity_index_map: &crate::world::EntityIndexMap,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        dt: f32,
    ) {
        // Birikmiş λ'lar bir çözücü GEÇİŞİNE (= bir substep) aittir; aşağıdaki iterasyonlar
        // bu birikimi yakınsatır. Geçiş başında sıfırla — döngünün İÇİNDE sıfırlamak tüm
        // değişikliği no-op'a indirir.
        //
        // `warm_start_factor == 0` (VARSAYILAN) iken λ adımlar arasında TAŞINMAZ: rollback
        // restore'undan sonraki ilk `solve_joints` onu zaten sıfırdan kurar. Faktör sıfırdan
        // büyükse geçen geçişin λ'sı `prev_rows`'ta saklanıyor ve gerçek taşınan durum
        // oluyor — `WorldSnapshot` `joints`'i klonladığı için bu kendiliğinden kapsanıyor.
        //
        // Sıfır uzunluklu adım `prev_rows`'a DOKUNMAZ, ama bunu geçmişin hayatta kaldığı
        // şeklinde okuma: sonraki gerçek geçişin `begin_pass`'i `prev_rows`'u, az önce
        // sıfırlanmış olan `rows`'tan yeniden yazar, yani warm start bir geçiş SOĞUK başlar.
        // Bilinçli bir kabul; ayrıntı `JointScratch::clear_pass`'ta. `rows`/`impulse_*` her
        // iki yolda da sıfırlanıyor, yani faktör 0'da ayrım gözlemlenemez.
        let steppable = dt > 0.0 && dt.is_finite();
        for joint in joints.iter_mut() {
            if steppable {
                joint.scratch.begin_pass();
            } else {
                joint.scratch.clear_pass();
            }
        }

        // A non-positive (or NaN) step is not a step, and every quantity below is a RATE:
        // the Baumgarte term is `error/dt`, a motor budget is `max_force·dt`, and the break
        // check divides the pass's impulse by dt. At `dt == 0` those are `x/0` and `0/0`, not
        // "small". The break check was the one that did real damage: `‖Σλᵢnᵢ‖ / 0` is `+inf`
        // for a joint carrying ANY load, and `inf > break_force` holds for every FINITE
        // threshold — so a single call with `dt == 0` snapped every breakable joint in the
        // world at once, irreversibly (nothing ever clears `is_broken`). Only a joint left at
        // the constructor default `f32::INFINITY` survived, because `inf > inf` is false.
        //
        // `PhysicsWorld::step` cannot reach this: it substeps at `FIXED_DT` and a paused world
        // returns before the accumulator (`world/step.rs`). But `JointSolver` is public, the
        // crate advertises the physics as embeddable, and `solve_joints` uses the dt it is
        // handed — which is how an embedder's paused/zero-length frame gets here.
        //
        // Doing nothing is the only answer that cannot be wrong: no time passed, so no impulse
        // was delivered and nothing can have broken. The scratch is cleared FIRST so the
        // documented "impulses are cleared at the start of every call" stays true and a reader
        // of `joint.scratch` sees this pass's honest zero instead of the previous pass's total.
        //
        // NOT fixed here, and worth knowing: a very small POSITIVE dt has the same shape of
        // problem. `position_bias` saturates at `max_correction_speed`, so λ stops shrinking
        // with dt while `λ/dt` keeps growing — a joint's reported force diverges as dt → 0⁺.
        // That is a calibration question about the speed clamp, not a degenerate-input one.
        //
        // (`dt <= 0.0` is false for NaN, hence the second half; an infinite dt is rejected on
        // the same grounds — a motor budget of `max_force·∞` is not a step either.)
        if !steppable {
            return;
        }

        // Warm start AÇIKSA iterasyonların önüne fazladan bir süpürme konur ve o süpürmede
        // her satır λ hesaplamak yerine `factor · λ_prev` enjekte eder. Ayrı süpürme olması
        // ŞART: satırın kendi yerinde enjeksiyon cebirsel bir no-op (bkz.
        // [`Self::warm_start_factor`]). Faktör 0'da `passes == iterations`, `warm_factor`
        // hiç yazılmıyor ve yol bit-aynı.
        let warm = self.warm_start_factor > 0.0;
        let passes = self.iterations + usize::from(warm);
        for pass in 0..passes {
            let injecting = warm && pass == 0;
            for joint in joints.iter_mut() {
                if joint.is_broken {
                    continue;
                }

                let idx_a = entity_index_map.get(&joint.entity_a.id()).copied();
                let idx_b = entity_index_map.get(&joint.entity_b.id()).copied();
                let (Some(idx_a), Some(idx_b)) = (idx_a, idx_b) else {
                    continue;
                };
                if idx_a == idx_b {
                    continue;
                }
                // Both ends immobile → nothing to solve. `PhysicsWorld` runs its joint-graph
                // wake pass immediately BEFORE this call and wakes every dynamic body in a
                // component containing a mover, so this gate is a strictly weaker test
                // evaluated after it and cannot swallow a component that was just woken.
                // Nothing in the type system holds that ordering —
                // `waking_travels_the_whole_chain_in_one_step` is what would break.
                if joint_is_inert(rigid_bodies, velocities, idx_a, idx_b) {
                    continue;
                }

                if injecting {
                    joint.scratch.set_warm_injection(self.warm_start_factor);
                }

                // Dispatch on the JointType enum (a Copy value derived from joint.data via
                // the compile-forced From impl), not the &str — so a new JointData variant
                // that forgot a solver case is a compile error, not a silent no-op.
                match JointType::from(&joint.data) {
                    JointType::Fixed => self.solve_fixed_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Hinge => self.solve_hinge_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::BallSocket => self.solve_ball_socket_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Slider => self.solve_slider_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::Distance => self.solve_distance_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    JointType::D6 => self.solve_d6_joint(
                        joint,
                        rigid_bodies,
                        transforms,
                        velocities,
                        idx_a,
                        idx_b,
                        dt,
                    ),
                    // Spring is force-based (depends on position, not velocity); running it
                    // inside the iteration loop would apply the force ~iterations times.
                    // It is applied once per step outside the loop (see below).
                    JointType::Spring => {}
                }

                if injecting {
                    joint.scratch.set_warm_injection(0.0);
                }
            }
        }

        // ── Kuvvet-tabanlı eklemler: step başına BİR kez ──────────────────
        // Yay kuvveti pozisyona bağlı olduğundan velocity-solver iterasyonları
        // boyunca sabittir; döngü dışında tek sefer uygulanmalıdır.
        for joint in joints.iter_mut() {
            if joint.is_broken {
                continue;
            }
            let (Some(idx_a), Some(idx_b)) = (
                entity_index_map.get(&joint.entity_a.id()).copied(),
                entity_index_map.get(&joint.entity_b.id()).copied(),
            ) else {
                continue;
            };
            if idx_a == idx_b {
                continue;
            }
            // Same gate as the velocity phase, and it does real work here: a sleeping slider's
            // suspension spring would otherwise keep writing velocities that position
            // integration discards, AND keep reporting load — `solve_spring_joint`,
            // `solve_slider_spring` and `solve_hinge_spring` all add to `impulse_*` before
            // testing `dyn_a`/`dyn_b`, so a spring between two immobile bodies can trip
            // `break_force`.
            if joint_is_inert(rigid_bodies, velocities, idx_a, idx_b) {
                continue;
            }
            // Force-based contributions: Spring is always force-based; Slider/Hinge carry
            // optional suspension/torsional springs (the solve_*_spring fns no-op if off).
            match JointType::from(&joint.data) {
                JointType::Spring => {
                    self.solve_spring_joint(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::Slider => {
                    self.solve_slider_spring(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::Hinge => {
                    self.solve_hinge_spring(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                JointType::D6 => {
                    self.solve_d6_drives(joint, rigid_bodies, transforms, velocities, idx_a, idx_b, dt)
                }
                _ => {}
            }
        }

        // ── Kopma kontrolü: geçiş başına BİR kez, NET tepki üzerinden ─────────
        //
        // Eskiden her joint türü kendi içinde, İTERASYON DÖNGÜSÜNÜN İÇİNDE kontrol
        // ediyordu (8 ayrı yer) ve ölçtüğü şey `Σ|λᵢ|` — satır büyüklüklerinin L1
        // toplamı — idi. Üç ayrı biçimde yanlıştı:
        //   * eş-doğrusal OLMAYAN satırların büyüklüklerini topluyordu: Fixed'in üç dik
        //     lineer satırında bu net tepkiyi √3'e kadar abartır, ball-socket'te
        //     (koni + twist + swing, dik bile değiller) daha da fazla;
        //   * `iterations` ile ölçekleniyordu — `world.joint_solver.iterations` public
        //     bir alan, yani onu değiştirmek sahnedeki HER eşiği sessizce yeniden
        //     ölçekliyordu;
        //   * `fixed.rs`'teki `err_len >= 1e-4` kapısı, kusursuz sabitlenmiş bir kaynağın
        //     lineer kontrolünü tamamen atlıyordu.
        //
        // Artık ölçülen şey geçişin NET impulse vektörü `‖Σ λᵢ·nᵢ‖ / dt` — yani eklemin
        // gerçekten taşıdığı kuvvet/tork. Kuvvet-tabanlı yaylar da (Spring, slider
        // süspansiyonu, hinge torsiyon yayı) bu toplama katkı verir; motorlar/sürücüler
        // VERMEZ, çünkü onlar dış yük değil eyleyicidir (bkz. docs/ENGINE.md §7).
        for joint in joints.iter_mut() {
            if joint.is_broken {
                continue;
            }
            let force = joint.scratch.impulse_lin.length() / dt;
            let torque = joint.scratch.impulse_ang.length() / dt;
            if joint.check_break(force, torque) {
                tracing::debug!(
                    entity_a = ?joint.entity_a,
                    entity_b = ?joint.entity_b,
                    joint_type = joint.joint_type(),
                    applied_force = force,
                    break_force = joint.break_force,
                    applied_torque = torque,
                    break_torque = joint.break_torque,
                    "Joint broke (net reaction exceeded break threshold)"
                );
            }
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    /// The SWING half of the swing–twist decomposition of `q` about the unit axis `a`
    /// (both expressed in the same frame): the factor whose rotation axis is PERPENDICULAR
    /// to `a`, i.e. how far `a` itself has been tipped, with the roll about `a` divided out.
    ///
    /// Construction (the standard one): write `q = (v, w)` and project the vector part onto
    /// the axis, `p = (v·a)a`. Then `twist = (p, w)/‖(p, w)‖` is a rotation about `a`, and
    /// `swing = q · twist⁻¹` satisfies `q = swing · twist`. The `a`-component of `swing`'s
    /// vector part cancels exactly — for `a = ẑ` it is `zc − ws` with `c = w/‖(p,w)‖` and
    /// `s = z/‖(p,w)‖`, which is zero — so `swing` really is a pure tip and carries no roll.
    /// The same algebra gives `swing.w = ‖(p, w)‖ ≥ 0` whenever `q.w ≥ 0`, so a canonicalised
    /// input yields a canonicalised swing and `2·acos(swing.w)` is the swing angle in `[0, π]`.
    ///
    /// Two inputs have no decomposition and both return `q` unchanged, i.e. "it is all swing":
    ///
    /// * `a == ZERO` — no axis to decompose about. This is the fallback the ball-socket cone
    ///   relies on when `twist_axis` was never set: measuring the whole deviation is the only
    ///   thing a cone with no axis can mean, and it is exactly what the cone did before.
    /// * `‖(p, w)‖ ≈ 0` — a swing of π with no roll component, where the twist is genuinely
    ///   undefined rather than merely small.
    #[inline]
    pub(crate) fn swing_about(q: Quat, a: Vec3) -> Quat {
        let p = a * Vec3::new(q.x, q.y, q.z).dot(a);
        let twist = Quat::from_xyzw(p.x, p.y, p.z, q.w);
        if twist.length_squared() < 1e-12 {
            return q;
        }
        q * twist.normalize().conjugate()
    }

    /// Two unit vectors perpendicular to `v`.
    fn perpendiculars(v: Vec3) -> (Vec3, Vec3) {
        let p1 = if v.x.abs() < 0.9 {
            v.cross(Vec3::X).normalize()
        } else {
            v.cross(Vec3::Y).normalize()
        };
        (p1, v.cross(p1))
    }

    /// World-space lever arm from a body's **centre of mass** to a point.
    ///
    /// Every joint row needs this, and every one of them used to compute
    /// `anchor - transforms[idx].position` — the arm about the transform ORIGIN. The two
    /// agree only when `center_of_mass` is zero. They differ for every compound collider
    /// (`RigidBody::update_inertia_from_collider` derives a shifted COM automatically),
    /// every fracture chunk (`fracture.rs` sets one explicitly) and every vehicle chassis,
    /// so those bodies got the wrong torque and the wrong effective mass from every joint
    /// attached to them.
    ///
    /// This is the same expression `Integrator::apply_impulse_at_point` uses, which is the
    /// convention the contact solver has always followed — the joint path was the outlier.
    #[inline]
    pub(crate) fn lever_arm(
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        idx: usize,
        point: Vec3,
    ) -> Vec3 {
        let t = &transforms[idx];
        let global_com = t.position + t.rotation * rigid_bodies[idx].center_of_mass;
        point - global_com
    }

    /// Box2D-v3 soft-constraint coefficients of a compliant row:
    /// `(bias_rate, mass_scale, impulse_scale)`.
    ///
    /// The contact solver already uses this formulation (`solver/tgs.rs:117-124` and `:597`);
    /// the joint solver was still on Baumgarte β + velocity clamping, applying `compliance` by
    /// adding `α/dt²` (CFM) to the effective mass. That path does NOT work in this engine, for
    /// two separate reasons:
    ///
    /// 1. **CFM on its own does not produce softness.** Growing `k` only shrinks each
    ///    iteration's step; as the iteration count rises the series still converges to the RIGID
    ///    solution. So `compliance` was not a physical inverse stiffness — it was a relaxation
    ///    factor whose meaning depended on `iterations`.
    /// 2. **The missing feedback term (`-α̃·λ`) cannot be added to this solver.** The
    ///    equilibrium would be `λ_total = bias/α̃`, but `bias` here is SPEED-CLAMPED by
    ///    `max_correction_speed`: the moment the clamp bites, λ is capped far below the load it
    ///    has to carry and the constraint quietly goes slack (measured: a 2 m rope reached
    ///    27.4 m in 600 steps — free fall).
    ///
    /// In the soft formulation `c` is a MULTIPLIER: equilibrium is `λ_total = c·bias_rate·C =
    /// dt·ω²·C`. The required bias stays small and the clamp never bites. Under CFM, `λ =
    /// bias/α̃` was a DIVISION, and carrying the same load asked for ~14× the clamp's bias. Two
    /// ways of writing the same physics; diametrically opposite conditioning in a solver that
    /// clamps.
    ///
    /// `ω = √(k/α)` — derived from the row's effective mass (`m_eff = 1/k`), i.e. the classic
    /// spring frequency `ω = √(K/m_eff)`. At equilibrium `λ = dt·ω²·C/k = dt·C/α`, that is
    /// **`F = C/α`: Hooke's law.** Stiffness is constant and a heavier load stretches it
    /// further — which is what `compliance` being declared an inverse stiffness has to mean.
    ///
    /// # The `impulse_scale` term is NOT divided by `k`
    ///
    /// The distinction is not cosmetic, it is STABILITY. If `-impulse_scale·λ` were also
    /// divided by `k`, the λ iteration becomes `λ_{n+1} = λ_n·(1 - impulse_scale/k) + …`, which
    /// DIVERGES once `impulse_scale > 2k`, i.e. `m_eff > 2/impulse_scale`. Measured: in the
    /// divided form a rope with α = 0.03 stretches 0.2937 m under 1 kg (Hooke: 0.2943 ✓), but at
    /// 4 kg the constraint goes fully slack and the body falls 331 m — 2000 steps of free fall.
    ///
    /// Undivided, the iteration is `λ_{n+1} = λ_n·(1 - impulse_scale) + …` and, since
    /// `impulse_scale = 1/(1+c) ∈ (0, 1]`, it is **unconditionally stable**.
    ///
    /// (In the contact solver at `solver/tgs.rs:597` the term IS divided by `k_n`. Its
    /// `impulse_scale` there is far smaller — ≈0.058 at contact_hertz=30, ζ=10 — so the bound
    /// rises to `m_eff ≈ 34` and does not bite in any current soak scene. That is a separate
    /// thing to measure; see docs/ENGINE.md.)
    ///
    /// # The clamping-regime question is CLOSED
    ///
    /// This document used to leave an open question: "compliance's iteration dependence will be
    /// dealt with together with the clamping regime". The answer: the soft formulation is in the
    /// **multiplier** regime and the clamp does not bite the equilibrium. The identity is —
    ///
    /// ```text
    /// ω² = mass_scale · bias_rate / (impulse_scale · dt)
    /// static error C* = a / ω²,   equilibrium bias b* = impulse_scale · a · dt / mass_scale
    /// ```
    ///
    /// — and because `mass_scale + impulse_scale ≡ 1`, the equilibrium λ is exactly `a·dt·m_eff`,
    /// i.e. the real load, with no dependence on the clamp at all. Even in the heaviest scene in
    /// the suite the required bias is 21× below the ceiling (details in
    /// [`Self::rigid_coefficients`]). Rigid rows are on this path too now; the only difference
    /// there is where ω comes from.
    #[inline]
    fn soft_coefficients(&self, compliance: f32, k: f32, dt: f32) -> (f32, f32, f32) {
        let omega = (k / compliance).sqrt();
        let denom = 2.0 * self.compliance_damping_ratio + dt * omega;
        if denom <= 1e-9 {
            return (0.0, 1.0, 0.0);
        }
        let c = dt * omega * denom;
        (omega / denom, c / (1.0 + c), 1.0 / (1.0 + c))
    }

    /// Soft-constraint coefficients of a RIGID row (`compliance == 0`):
    /// `(bias_rate, mass_scale, impulse_scale)`.
    ///
    /// Same Box2D-v3 arithmetic as [`Self::soft_coefficients`], but ω comes from a FREQUENCY
    /// ([`Self::rigid_hertz`]) rather than from `√(k/α)` — which is undefined at
    /// `compliance == 0`. It does not depend on `k`, so every rigid row in the world shares
    /// one triple; it is recomputed per row anyway because hoisting it would mean threading a
    /// triple through 25 call sites to save ~8 flops next to the inverse-inertia matrix
    /// products already in the same function.
    ///
    /// # Why this row is SOFT now
    ///
    /// Until now the rigid row ran with `mass_scale = 1, impulse_scale = 0`: Baumgarte β·C/dt
    /// plus velocity clamping, and NO feedback term. An under-converged pass integrates the
    /// Baumgarte residual into λ with nothing there to bleed it back out. That is what broke
    /// warm-starting at its natural f=1.0 (a 16-link chain with a 200 kg end: 4.83 m of residual
    /// motion at 44 m/s) while the same chain was STABLE with `compliance = 1e-6`. See
    /// docs/ENGINE.md B4.
    ///
    /// # Clamping regime: MULTIPLIER, not division — but that is a MARGIN, not a guarantee
    ///
    /// The equilibrium bias is `b* = impulse_scale·a·dt/mass_scale` — `1.099e-4·a` at the
    /// defaults. On a 1 kg rope that is 1.1e-3 m/s (4500× below the ceiling); in the heaviest
    /// scene in the suite, the anchor row of the chain with a 200 kg end (`a ≈ 2109 m/s²`), it
    /// is 0.232 m/s — **21× below** the 5 m/s ceiling. The CFM disaster happened because
    /// `λ = bias/α̃` was a DIVISION and the required bias was ~14× the clamp; here `c` is a
    /// multiplier and the required bias is 4500× under it.
    ///
    /// What happens if it DOES bite still has to be written down, because it is WORSE than the
    /// old path. With `b_max < b*` a steady state is impossible: the residual velocity stays at
    /// `b* − b_max` and the error grows LINEARLY in time — the constraint goes slack. On the old
    /// Baumgarte path the clamp merely slowed the repair down (`v_final = clamped bias`, bounded
    /// error). The threshold is `a > max_correction_speed·c/dt ≈ 45 000 m/s²`; the heaviest
    /// measured scene has 21× of margin, but the margin narrows as `rigid_hertz` is lowered
    /// (smaller `c`), and with a `hertz` pinned to `1/dt` — `c ≤ 52` — the threshold rises to
    /// ≈ 62 000 m/s². If a scene ever enters that region, the fix is to raise
    /// `max_correction_speed`, not to lower `rigid_hertz`.
    ///
    /// # Why `hertz` and ζ DIFFER from the contact solver's
    ///
    /// `solve_contacts_tgs` uses 30 Hz / ζ=10. The same numbers here give `ω² = 3.6e4`: a 1 kg
    /// rope sags 2.8e-4 m (RED against the current 1e-4 bound) and the chain sags 1.8 m. The
    /// reason is structural: a contact carries roughly its own body's weight, a joint can carry
    /// 400× that. 200 Hz / ζ=1 reproduces both axes of the measured `compliance = 1e-6` control
    /// (`impulse_scale` 0.0257 ↔ measured band 0.021–0.037, `bias_rate` 173.7 ↔ 179) and is the
    /// STIFFEST setting that stays inside that verified band.
    #[inline]
    fn rigid_coefficients(&self, dt: f32) -> (f32, f32, f32) {
        // Kırpma temas çözücüsünün `0.25/dt`'siyle TERS gerekçeye sahip. Orada amaç ω·dt'yi
        // küçük tutmak; burada amaç `impulse_scale`'in çökmesini engellemek — hertz büyüdükçe
        // satır geri besleme terimi olmayan sert Baumgarte'a, yani düzeltmek için var olduğu
        // hataya geri dejenere olur. `1/dt` ω·dt ≤ 2π'yi pinler, dolayısıyla
        // `impulse_scale ≥ 0.0189`.
        let hertz = self.rigid_hertz.min(1.0 / dt);
        let omega = 2.0 * std::f32::consts::PI * hertz;
        let denom = 2.0 * self.compliance_damping_ratio + dt * omega;
        if denom <= 1e-9 {
            return (0.0, 1.0, 0.0);
        }
        let c = dt * omega * denom;
        (omega / denom, c / (1.0 + c), 1.0 / (1.0 + c))
    }

    /// Accumulated-λ clamping: `lambda_min/max` are applied to the TOTAL accumulated over the
    /// pass, not to the increment. That lets a one-sided row (limit / rope / cone) apply a
    /// NEGATIVE increment too — it can TAKE BACK its own earlier over-correction, as long as the
    /// total stays on the right side. Clamping each increment separately, as it used to, made
    /// giving anything back impossible.
    ///
    /// The value applied (and returned) is the increment, not the accumulation; velocities are
    /// updated with the increment.
    #[inline]
    fn accumulate(accum: &mut f32, delta: f32, lambda_min: f32, lambda_max: f32) -> f32 {
        let total = *accum + delta;
        let clamped = total.clamp(lambda_min, lambda_max);
        if clamped == total {
            // Sınır ISIRMADI → artımı OLDUĞU GİBİ uygula. `clamped - *accum` yazsaydık
            // birikim büyüdükçe f32 yuvarlama farkı doğardı. Bu dal sayesinde ±∞ clamp'li
            // ve compliance = 0 olan EŞİTLİK satırları — Fixed, D6 Locked, slider'ın dik
            // eksenleri ve açısal kilidi, hinge eksen hizalaması — bugünküyle BİT-AYNI
            // kalır. Davranış değişimi yalnızca gerçekten bir sınıra dayanan satırlarda.
            // (Bu iddia KIRPMA DALI hakkında; hâlâ doğru. Rijit satırların `delta`sı ayrıca
            // `rigid_hertz` ile değişti — o başka bir yerde, çağıranda.)
            *accum = total;
            delta
        } else {
            let applied = clamped - *accum;
            *accum = clamped;
            applied
        }
    }

    /// Apply a 1-DOF angular velocity constraint along `direction` at zero compliance.
    /// `error` is the positional error in radians (positive = bodies need to rotate apart).
    #[allow(clippy::too_many_arguments)]
    fn apply_angular_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        self.apply_angular_constraint_soft(
            rigid_bodies, transforms, velocities, idx_a, idx_b, direction, error, dt, lambda_min,
            lambda_max, 0.0, scratch, slot,
        )
    }

    /// Compliant form of [`Self::apply_angular_constraint`]. `compliance` ≥ 0 is the inverse
    /// stiffness: a row at `α > 0` obeys Hooke's law, `F = C/α`, with its frequency derived
    /// from `√(k/α)`. Lets a specific limit/weld be soft without touching the global tuning.
    ///
    /// `compliance == 0` no longer means "hard": it means the row's frequency comes from
    /// [`Self::rigid_hertz`] instead (200 Hz by default, a static error of `a/ω²`), and only
    /// `rigid_hertz <= 0` restores the Baumgarte path.
    ///
    /// **One exception, and it is a correctness one:** a row with `error == 0.0` has no
    /// position term, so nothing would ever take back the `impulse_scale · v` a soft row
    /// leaves behind and the angle would drift linearly without bound. Such a row stays a
    /// hard velocity constraint on every path. `fixed.rs`'s 3-axis angular lock is the only
    /// structurally velocity-only caller in the crate; see the comment on the branch.
    #[allow(clippy::too_many_arguments)]
    fn apply_angular_constraint_soft(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        compliance: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        if direction.length_squared() < 1e-10 {
            return 0.0;
        }

        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let w_a = velocities[idx_a].angular;
        let w_b = velocities[idx_b].angular;
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        let k = direction.dot(inv_i_a.mul_vec3(direction)) + direction.dot(inv_i_b.mul_vec3(direction));
        if k < 1e-10 {
            return 0.0;
        }
        let vel_err = (w_b - w_a).dot(direction);

        // Üç kol. `compliance > 0` → YUMUŞAK, ω = √(k/α) (Hooke). `compliance == 0`,
        // `rigid_hertz > 0` ve satırın bir KONUM terimi VAR → yine YUMUŞAK ama ω bir
        // FREKANSTAN geliyor; bkz. `rigid_coefficients`. Aksi hâlde → sert hız kısıtı.
        //
        // `error == 0.0` neden yumuşak OLAMAZ — bu bir mikro-optimizasyon değil, DOĞRULUK:
        // yumuşak bir satır geriye `impulse_scale · v` bırakır ve o artığı geri alan şey
        // satırın konum terimidir. Konum terimi olmayan bir satırda geri alacak hiçbir şey
        // yoktur, artık hız sabit kalır ve AÇI ZAMANDA DOĞRUSAL, SINIRSIZ sürüklenir:
        // `ω_artık = α·dt/c` (varsayılanlarda 1.10e-4 rad/s, her 1 rad/s²'lik dış açısal
        // ivme için). Ölçüldü: 10 rad/s² altındaki bir kaynak 40 s'de 0.127 rad (7.3°).
        //
        // Yapısal olarak konum terimsiz TEK çağrı yeri `fixed.rs`'in 3 eksenli açısal kilidi
        // (`error` orada HARFİ HARFİNE `0.0`); diğer açısal çağrı yerlerinin hepsi gerçek bir
        // hata taşıyor ve sıkı eşitsizliklerle kapılı (`err_mag > 1e-6`, `swing_angle >
        // limit`, …), yani bu dala yalnızca o kilit — ve teğet geçen bir D6 `Locked` satırı —
        // düşer. Konum terimi olmayan satırda Baumgarte artığı da yoktur, dolayısıyla
        // `rigid_hertz`'in düzeltmek için var olduğu warm-start teşhisi buraya UYGULANMAZ:
        // `λ_{n+1} = (bias − v₀)/k` sert biçimde de hafızasızdır, enjeksiyondan bağımsızdır.
        let (bias, mass_scale, impulse_scale) = if compliance > 0.0 {
            let (bias_rate, m, i) = self.soft_coefficients(compliance, k, dt);
            (bias_rate * error, m, i)
        } else if self.rigid_hertz > 0.0 && error != 0.0 {
            let (bias_rate, m, i) = self.rigid_coefficients(dt);
            (bias_rate * error, m, i)
        } else {
            // ESKİ Baumgarte — ve `error == 0.0` olan hız-kilidi satırı, ki ikisi tam olarak
            // ÇAKIŞIR (`β·0/dt == 0`), bu yüzden ayrı bir kol yazılmadı.
            //
            // İfade HARFİ HARFİNE korunuyor, ortak bir `bias_rate * error` üzerinden yeniden
            // yazılmıyor: f32'de `(β·error)/dt` ile `(β/dt)·error` farklı yuvarlanır ve bu
            // kolun bir işi de değişiklik öncesiyle bit-aynı olmak.
            (self.position_bias * error / dt, 1.0, 0.0)
        };
        let position_bias = bias.clamp(-self.max_angular_speed, self.max_angular_speed);
        // Kırpma hikâyesinin tamamı `soft_coefficients`'ta: CFM'de `λ = bias/α̃` bir BÖLME
        // olduğu için kırpma ısırdığı an kısıt boşalıyordu (2 m'lik halat 600 adımda 27.4 m);
        // yumuşak formülasyonda `c` bir ÇARPAN ve denge bias'ı tavanın en az 21 katı altında.
        // `impulse_scale` terimi BÖLÜNMEZ — gerekçe yine `soft_coefficients`'ta (kararlılık).
        //
        // Warm start açıkken (`warm_start_factor > 0`) ilk süpürme λ hesaplamaz: geçen
        // substep'in λ'sını ölçekleyip ENJEKTE eder. Enjeksiyonun kendi süpürmesinde olması
        // ŞART — satırın kendi yerinde yapılan enjeksiyon cebirsel bir no-op'tur, çünkü clamp
        // burada TOPLAM üzerinden çalışıyor ve λ_prev sadeleşiyor (bkz. `warm_start_factor`).
        let delta = if let Some(factor) = scratch.warm_injection() {
            factor * scratch.prev_row_value(slot)
        } else {
            mass_scale * (-vel_err + position_bias) / k - impulse_scale * *scratch.row(slot)
        };
        let lambda = Self::accumulate(scratch.row(slot), delta, lambda_min, lambda_max);
        // Geçişin NET açısal impulse'ı — `break_torque` bundan hesaplanır. Artımların
        // VEKTÖR toplamı: eş-doğrusal olmayan satırların büyüklüklerini toplamak (eski
        // `.abs()` yığını) taşınan torku Fixed'de √3'e kadar abartıyordu.
        scratch.impulse_ang += direction * lambda;

        let delta_a = inv_i_a.mul_vec3(direction) * lambda;
        let delta_b = inv_i_b.mul_vec3(direction) * lambda;

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].angular -= delta_a;
            }
            if dyn_b {
                r[0].angular += delta_b;
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].angular += delta_b;
            }
            if dyn_a {
                r[0].angular -= delta_a;
            }
        }
        lambda
    }

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points, at
    /// zero compliance.
    #[allow(clippy::too_many_arguments)]
    fn apply_linear_constraint(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        self.apply_linear_constraint_soft(
            rigid_bodies, transforms, velocities, idx_a, idx_b, direction, r_a, r_b, error, dt,
            lambda_min, lambda_max, 0.0, scratch, slot,
        )
    }

    /// Compliant form of [`Self::apply_linear_constraint`]. See
    /// [`Self::apply_angular_constraint_soft`] — `α > 0` gives Hooke's law, `α == 0` gives a
    /// row at [`Self::rigid_hertz`].
    #[allow(clippy::too_many_arguments)]
    fn apply_linear_constraint_soft(
        &self,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        idx_a: usize,
        idx_b: usize,
        direction: Vec3,
        r_a: Vec3,
        r_b: Vec3,
        error: f32,
        dt: f32,
        lambda_min: f32,
        lambda_max: f32,
        compliance: f32,
        scratch: &mut JointScratch,
        slot: usize,
    ) -> f32 {
        let inv_m_a = rigid_bodies[idx_a].inv_mass();
        let inv_m_b = rigid_bodies[idx_b].inv_mass();
        let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
        let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
        let v_a = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
        let v_b = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
        let dyn_a = rigid_bodies[idx_a].is_dynamic();
        let dyn_b = rigid_bodies[idx_b].is_dynamic();

        // Efektif kütlenin açısal terimi: Jacobian açısal kısmı (r×n) olmak üzere
        // k_ang = (r×n)·I⁻¹·(r×n). (Eskiden ((I⁻¹ r)×n)×r·n hesaplanıyordu — farklı bir
        // nicelik; merkez-dışı ankor + anizotropik atalette yanlış impulse büyüklüğü.)
        let rxn_a = r_a.cross(direction);
        let rxn_b = r_b.cross(direction);
        let k = inv_m_a
            + inv_m_b
            + inv_i_a.mul_vec3(rxn_a).dot(rxn_a)
            + inv_i_b.mul_vec3(rxn_b).dot(rxn_b);
        if k < 1e-10 {
            return 0.0;
        }
        let rel_vel = (v_b - v_a).dot(direction);

        // Üç kol — gerekçe `apply_angular_constraint_soft`'ta.
        let (bias, mass_scale, impulse_scale) = if compliance > 0.0 {
            let (bias_rate, m, i) = self.soft_coefficients(compliance, k, dt);
            (bias_rate * error, m, i)
        } else if self.rigid_hertz > 0.0 {
            let (bias_rate, m, i) = self.rigid_coefficients(dt);
            (bias_rate * error, m, i)
        } else {
            // ESKİ Baumgarte, harfi harfine (bit-aynılık için — açısal eşe bak).
            (self.position_bias * error / dt, 1.0, 0.0)
        };
        let position_bias = bias.clamp(-self.max_correction_speed, self.max_correction_speed);
        // `impulse_scale` terimi BÖLÜNMEZ — gerekçe `soft_coefficients`'ta (kararlılık).
        // Warm-start enjeksiyon süpürmesi — gerekçe `apply_angular_constraint_soft`'ta.
        let delta = if let Some(factor) = scratch.warm_injection() {
            factor * scratch.prev_row_value(slot)
        } else {
            mass_scale * (-rel_vel + position_bias) / k - impulse_scale * *scratch.row(slot)
        };
        let lambda = Self::accumulate(scratch.row(slot), delta, lambda_min, lambda_max);
        // Geçişin NET doğrusal impulse'ı — `break_force` bundan hesaplanır (bkz. açısal eş).
        scratch.impulse_lin += direction * lambda;

        let impulse = direction * lambda;

        if idx_a < idx_b {
            let (l, r) = velocities.split_at_mut(idx_b);
            if dyn_a {
                l[idx_a].linear -= impulse * inv_m_a;
                l[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
            }
            if dyn_b {
                r[0].linear += impulse * inv_m_b;
                r[0].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
            }
        } else {
            let (l, r) = velocities.split_at_mut(idx_a);
            if dyn_b {
                l[idx_b].linear += impulse * inv_m_b;
                l[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(impulse));
            }
            if dyn_a {
                r[0].linear -= impulse * inv_m_a;
                r[0].angular -= inv_i_a.mul_vec3(r_a.cross(impulse));
            }
        }
        lambda
    }

    // ── joint solvers ─────────────────────────────────────────────────────────

}

// god-file Tier 3 round-2 bölmesi: per-joint çözücüler joint_types alt-modülünde
mod joint_types;

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_core::BodyHandle;

    #[test]
    fn test_joint_creation() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::fixed(e1, e2, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(joint.joint_type(), "Fixed");
        assert!(!joint.is_broken);
    }

    #[test]
    fn test_hinge_joint() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::hinge(e1, e2, Vec3::ZERO, Vec3::ZERO, Vec3::Y);
        assert_eq!(joint.joint_type(), "Hinge");
        if let JointData::Hinge(data) = joint.data {
            assert_eq!(data.axis, Vec3::Y);
        } else {
            panic!("expected hinge data");
        }
    }

    #[test]
    fn test_spring_joint() {
        let e1 = BodyHandle::from_id(1);
        let e2 = BodyHandle::from_id(2);
        let joint = Joint::spring(e1, e2, Vec3::ZERO, Vec3::ZERO, 1.0, 100.0, 10.0);
        if let JointData::Spring(data) = joint.data {
            assert_eq!(data.stiffness, 100.0);
            assert_eq!(data.damping, 10.0);
        } else {
            panic!("expected spring data");
        }
    }

    /// With the CORRECT effective mass, one application of a 1-DOF linear velocity constraint
    /// removes exactly `mass_scale` of the relative velocity and leaves `impulse_scale·v` behind
    /// (λ = mass_scale·(-Jv)/k, new Jv = Jv + kλ = impulse_scale·Jv).
    ///
    /// With the wrong `k` (the old `((I⁻¹r)×n)×r·n`) the amount removed is scaled by
    /// `k_correct/k_wrong` and the remainder MISSES that value; this is why the test still
    /// distinguishes the correct cross-product order — and does so more sharply than before,
    /// because it now measures against a computed constant rather than against zero.
    ///
    /// The remainder is NOT zero, because the rigid row is soft now at `rigid_hertz`: there are
    /// 10 applications in a pass and `impulse_scale ≈ 0.019` decays exponentially. That is the
    /// price, and what the row buys with it is the feedback term.
    #[test]
    fn linear_constraint_zeroes_relative_velocity_with_correct_effective_mass() {
        let solver = JointSolver::default();
        let (_, mass_scale, _) = solver.rigid_coefficients(1.0 / 60.0);

        let body = || {
            let mut rb = RigidBody::new(1.0, false);
            rb.local_inertia = Vec3::new(2.0, 5.0, 8.0); // anizotropik atalet
            rb
        };
        let bodies = [body(), body()];
        let transforms = [Transform::new(Vec3::ZERO), Transform::new(Vec3::ZERO)];
        let mut vels = [
            Velocity::default(),
            Velocity::new(Vec3::new(0.0, 1.0, 0.0)), // B ankora göre Y'de bağıl hız
        ];

        // Merkez-dışı ankorlar (bug bu durumda ortaya çıkar).
        let r_a = Vec3::new(0.3, 0.0, 0.0);
        let r_b = Vec3::new(-0.2, 0.1, 0.0);
        let direction = Vec3::Y;

        solver.apply_linear_constraint(
            &bodies,
            &transforms,
            &mut vels,
            0,
            1,
            direction,
            r_a,
            r_b,
            0.0, // pozisyon hatası yok → saf hız kısıtı
            1.0 / 60.0,
            f32::NEG_INFINITY,
            f32::INFINITY,
            &mut JointScratch::default(),
            row::LIN,
        );

        let v_a = vels[0].linear + vels[0].angular.cross(r_a);
        let v_b = vels[1].linear + vels[1].angular.cross(r_b);
        let rel_n = (v_b - v_a).dot(direction);
        let expected = (1.0 - mass_scale) * 1.0; // impulse_scale · v_pre
        assert!(
            (rel_n - expected).abs() < 1e-5,
            "tek uygulama bağıl hızın mass_scale katını silmeli: beklenen kalan {expected}, \
             ölçülen {rel_n} (yanlış efektif kütle?)"
        );
    }

    /// Two bodies of equal mass with their anchors at the centre of mass (r = 0 → k = 1/m +
    /// 1/m = 2), and a one-sided (PULL-only) row. The only thing that varies is where the clamp
    /// is applied.
    fn one_sided_pair() -> ([RigidBody; 2], [Transform; 2], [Velocity; 2]) {
        let body = || RigidBody::new(1.0, false);
        (
            [body(), body()],
            [Transform::new(Vec3::ZERO), Transform::new(Vec3::ZERO)],
            [Velocity::default(), Velocity::default()],
        )
    }

    /// A one-sided row must be able to GIVE BACK the impulse it applied.
    ///
    /// The clamp used to be applied to each iteration's own increment: on a pull-only row
    /// (`lambda_max = 0`) a positive increment was clamped to 0 every time, so the row could
    /// never undo its own over-correction — a one-way ratchet. The clamp now applies to the
    /// TOTAL accumulated over the pass, so any increment, negative or positive, can be applied
    /// as long as the total stays on the right side.
    ///
    /// What makes it discriminating: under the old code the second call applies nothing and the
    /// relative velocity stays at −1.0.
    #[test]
    fn a_one_sided_row_can_return_the_impulse_it_applied() {
        let solver = JointSolver::default();
        let (bodies, transforms, mut vels) = one_sided_pair();
        let mut scratch = JointScratch::default();

        // 1) Cisimler ayrılıyor (bağıl hız +1) → yalnız-çeken satır onları yakalar.
        vels[1].linear = Vec3::Y;
        let first = solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, Vec3::Y, Vec3::ZERO, Vec3::ZERO,
            0.0, // pozisyon hatası yok → saf hız kısıtı
            1.0 / 60.0,
            f32::NEG_INFINITY,
            0.0, // yalnız çek
            &mut scratch,
            row::LIMIT,
        );
        assert!(first < 0.0, "çeken satır negatif λ uygulamalı, uyguladığı = {first}");
        let rel = |v: &[Velocity; 2]| (v[1].linear - v[0].linear).dot(Vec3::Y);
        // Yumuşak satır bir uygulamada `impulse_scale·v` bırakır (bkz. yukarıdaki test).
        let (_, mass_scale, _) = solver.rigid_coefficients(1.0 / 60.0);
        let leak = 1.0 - mass_scale;
        assert!(
            (rel(&vels) - leak).abs() < 1e-6,
            "ilk çağrı bağıl hızın mass_scale katını silmeli, kalan = {}",
            rel(&vels)
        );

        // 2) Başka bir satır (burada elle) cisimleri BİRBİRİNE yaklaştırıyor. Satırın artık
        //    daha az çekmesi gerekiyor: doğru davranış, uyguladığının bir kısmını geri vermek.
        vels[0].linear = Vec3::ZERO;
        vels[1].linear = -Vec3::Y;
        assert!((rel(&vels) - (-1.0)).abs() < 1e-6);

        let second = solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, Vec3::Y, Vec3::ZERO, Vec3::ZERO, 0.0,
            1.0 / 60.0,
            f32::NEG_INFINITY,
            0.0,
            &mut scratch,
            row::LIMIT,
        );
        assert!(
            second > 0.0,
            "satır kendi impulse'ını geri vermeli (pozitif artım); uyguladığı = {second} \
             — iterasyon-başına clamp'te bu 0'a kırpılırdı"
        );
        // Birikmiş toplam tam olarak 0'a kırpıldı — satır uyguladığının HEPSİNİ geri verdi —
        // dolayısıyla kalan, birinci çağrının kalanının aynadaki görüntüsü: `-impulse_scale·v`.
        assert_eq!(*scratch.row(row::LIMIT), 0.0, "toplam üst sınıra dönmeli");
        assert!(
            (rel(&vels) - (-leak)).abs() < 1e-6,
            "geri verdikten sonra kalan bağıl hız {} olmalı, ölçülen = {}",
            -leak,
            rel(&vels)
        );
    }

    /// …but it cannot give back MORE than it applied: the accumulated total may not cross the
    /// bound, so a pull-only row never PUSHES, under any circumstances.
    #[test]
    fn a_one_sided_row_never_pushes_past_its_bound() {
        let solver = JointSolver::default();
        let (bodies, transforms, mut vels) = one_sided_pair();
        let mut scratch = JointScratch::default();
        let dt = 1.0 / 60.0;
        let args = (Vec3::Y, Vec3::ZERO, Vec3::ZERO, 0.0f32);

        vels[1].linear = Vec3::Y;
        solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, args.0, args.1, args.2, args.3, dt,
            f32::NEG_INFINITY, 0.0, &mut scratch, row::LIMIT,
        );
        let applied_total = *scratch.row(row::LIMIT);
        assert!(applied_total < 0.0);

        // Cisimler artık HIZLA yaklaşıyor: satır bunu düzeltmeye çalışsa iterek yapardı.
        vels[0].linear = Vec3::ZERO;
        vels[1].linear = Vec3::Y * -3.0;
        solver.apply_linear_constraint(
            &bodies, &transforms, &mut vels, 0, 1, args.0, args.1, args.2, args.3, dt,
            f32::NEG_INFINITY, 0.0, &mut scratch, row::LIMIT,
        );

        let total = *scratch.row(row::LIMIT);
        assert_eq!(total, 0.0, "biriken toplam üst sınırda durmalı, durduğu = {total}");
        assert!(
            (vels[1].linear - vels[0].linear).dot(Vec3::Y) < 0.0,
            "yalnız-çeken satır cisimleri AYIRMAMALI; bağıl hız = {}",
            (vels[1].linear - vels[0].linear).dot(Vec3::Y)
        );
    }

    /// The accumulation belongs to ONE PASS: `solve_joints` must start from zero on every
    /// call. Without the reset, a second pass inherits the first one's saturated λ and answers
    /// the same input differently — a silent state leak between steps.
    #[test]
    fn accumulated_lambda_does_not_leak_between_passes() {
        use crate::joints::data::Joint;
        use gizmo_physics_core::BodyHandle;

        let solver = JointSolver::default();
        let mut bodies = [RigidBody::new(1.0, false), RigidBody::new(1.0, false)];
        for rb in &mut bodies {
            rb.local_inertia = Vec3::splat(1.0);
        }
        let transforms = [
            Transform::new(Vec3::ZERO),
            Transform::new(Vec3::new(0.0, -3.0, 0.0)), // halat gergin (max 2.0)
        ];
        let map: crate::world::EntityIndexMap =
            [(1u32, 0usize), (2u32, 1usize)].into_iter().collect();
        let fresh = || {
            vec![Joint::rope(
                BodyHandle::from_id(1),
                BodyHandle::from_id(2),
                Vec3::ZERO,
                Vec3::ZERO,
                2.0,
            )]
        };
        let start = [Velocity::default(), Velocity::new(Vec3::new(0.0, -4.0, 0.0))];

        // Tek geçiş.
        let mut joints = fresh();
        let mut v1 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v1, 1.0 / 60.0);
        let lambda_after_one = joints[0].scratch;

        // Aynı eklem üzerinde İKİ geçiş, ikincisi aynı başlangıç hızlarıyla.
        let mut v2 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v2, 1.0 / 60.0);
        v2 = start;
        solver.solve_joints(&mut joints, &map, &bodies, &transforms, &mut v2, 1.0 / 60.0);

        for slot in 0..JointScratch::LEN {
            assert_eq!(
                joints[0].scratch.row_value(slot),
                lambda_after_one.row_value(slot),
                "yuva {slot}: aynı girdiyle ikinci geçiş aynı λ'yı üretmeli; birikim geçişler \
                 arasında taşınmış"
            );
        }
        assert_eq!(v2, v1, "…ve dolayısıyla aynı hızları");

        // `prev_rows` KARŞILAŞTIRILMIYOR, ve bu bilinçli: geçen geçişin λ'sını taşımak onun
        // TEK işi (warm start'ın girdisi). Onu da eşit istemek "birikim sızmasın" iddiasını
        // "hiçbir şey taşınmasın"a çevirirdi — `warm_start_factor` varsayılan 0'da bu ayrım
        // gözlemlenemez, ve `rows` üzerindeki döngü tam olarak gözlemlenebilir olanı pinler.
        assert_eq!(
            lambda_after_one.prev_row_value(row::LIMIT),
            0.0,
            "ilk geçişin öncesinde taşınacak λ yok"
        );
        assert_ne!(
            joints[0].scratch.prev_row_value(row::LIMIT),
            0.0,
            "üçüncü geçiş ikincinin λ'sını taşımalı — warm start'ın okuduğu şey bu"
        );
    }

    /// The decomposition's defining properties, on a rotation that mixes both parts.
    ///
    /// Not a regression test — `swing_about` is new, so nothing here can fail on the old
    /// code. It exists because the ball-socket cone now trusts three claims about it, and a
    /// wrong decomposition would be invisible in a behavioural test that happens to use a
    /// pure swing or a pure twist. The claims: the swing carries NO roll about the axis, it
    /// leaves a pure swing untouched, and it leaves nothing at all of a pure twist.
    #[test]
    fn swing_about_removes_the_roll_and_nothing_else() {
        let axis = Vec3::new(0.3, -0.5, 0.81).normalize();
        // A genuine swing tips the axis, so its own axis must be PERPENDICULAR to it.
        let swing_axis = axis.cross(Vec3::X).normalize();
        let swing_in = Quat::from_axis_angle(swing_axis, 0.7);
        let twist_in = Quat::from_axis_angle(axis, 1.1);

        // Mixed: `q = swing·twist` must decompose back to exactly `swing`, and that swing must
        // carry no component along the axis.
        let swing = JointSolver::swing_about(swing_in * twist_in, axis);
        let roll = Vec3::new(swing.x, swing.y, swing.z).dot(axis);
        assert!(
            roll.abs() < 1e-5,
            "the swing must carry no roll about the axis, got {roll}"
        );
        assert!(
            (2.0 * swing.w.abs().clamp(0.0, 1.0).acos() - 0.7).abs() < 1e-4,
            "…and it must be the swing that went in (0.7 rad), got {}",
            2.0 * swing.w.abs().clamp(0.0, 1.0).acos()
        );

        // A pure twist is ALL twist: nothing is left for the cone to clamp. This is the whole
        // point of the change — the cone used to see 1.1 rad here.
        let none = JointSolver::swing_about(twist_in, axis);
        let none_angle = 2.0 * none.w.abs().clamp(0.0, 1.0).acos();
        assert!(
            none_angle < 1e-3,
            "a pure twist must decompose to zero swing, got {none_angle} rad"
        );

        // …and a pure swing survives whole, so the cone did not become permissive in general.
        let all = JointSolver::swing_about(swing_in, axis);
        let all_angle = 2.0 * all.w.abs().clamp(0.0, 1.0).acos();
        assert!(
            (all_angle - 0.7).abs() < 1e-4,
            "a pure swing must decompose to itself, got {all_angle} rad (expected 0.7)"
        );

        // No axis ⇒ no decomposition: the whole deviation is reported as swing, which is the
        // fallback `cone_limit_angle` relies on when `twist_axis` was never set.
        let whole = JointSolver::swing_about(twist_in, Vec3::ZERO);
        assert!(
            (whole.w - twist_in.w).abs() < 1e-6,
            "a zero axis must return the input unchanged"
        );
    }

    #[test]
    fn test_perpendiculars_orthogonality() {
        let v = Vec3::new(1.0, 0.0, 0.0);
        let (p1, p2) = JointSolver::perpendiculars(v);
        assert!(p1.dot(v).abs() < 1e-5);
        assert!(p2.dot(v).abs() < 1e-5);
        assert!(p1.dot(p2).abs() < 1e-5);
    }
}
