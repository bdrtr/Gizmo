use super::data::*;
use gizmo_physics_core::components::Transform;
use crate::components::{RigidBody, Velocity};
use gizmo_math::{Quat, Vec3};

/// Birikmiş-λ yuvaları ([`JointScratch`]).
///
/// Yuvalar DERLEME ZAMANI SABİTİ, ilerleyen bir imleç DEĞİL: satırların çoğu koşullu
/// atlanıyor (`fixed.rs`'te `err_len >= 1e-4`, `hinge.rs`'te `err_mag > 1e-6` ve limit
/// dalları, `slider.rs`'te `err.abs() > 1e-4`, `ball_socket.rs`'teki koni/twist/swing
/// kapıları, `d6.rs`'teki `continue` kolları), bir imleç atlanan her satırda sonraki
/// satırların kimliğini kaydırır ve λ'lar yanlış satıra yazılırdı.
///
/// Bir DOF'un ALT ve ÜST limiti aynı yuvayı paylaşır: iki bağ ama tek serbestlik derecesi,
/// ve bir geçiş boyunca `transforms` DEĞİŞMEDİĞİ için (çözücü `&[Transform]` alır,
/// entegrasyon geçişten sonra) hangi dalın seçildiği 10 iterasyon boyunca sabittir — ters
/// işaretli bayat bir λ miras alınamaz.
pub(crate) mod row {
    /// 0,1,2 — nokta kısıtının X/Y/Z'si, slider'ın iki dik ekseni, D6 lineer DOF'ları.
    pub const LIN: usize = 0;
    /// 3,4,5 — Fixed 3-eksen açısal kilidi, D6 açısal DOF'ları, hinge eksen hizalaması,
    /// slider açısal kilidi, ball-socket koni (3) ve twist (4).
    pub const ANG: usize = 3;
    /// Hinge/slider limiti, distance min|max — hepsi tek DOF'un iki yönlü sınırı.
    pub const LIMIT: usize = 6;
    /// 7,8 — ball-socket asimetrik swing limitleri (perp1, perp2).
    pub const SWING: usize = 7;
    /// 9 — motor / servo satırı. Kopma toplamına KATILMAZ: motor dış yük değil eyleyicidir.
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
    /// not a cap on the impulse a joint may apply. On a rigid row (`compliance == 0`), with
    /// the defaults and the world's 1/240 s substep, it starts to bind at roughly 7 cm of
    /// error (`max_correction_speed · dt / position_bias`); below that it is inert.
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
    /// rather than snapping.
    pub max_angular_speed: f32,
    /// Baumgarte factor β for the rigid rows — dimensionless, default 0.3.
    ///
    /// A row with positional error `C` is given the target velocity `β·C/dt` (then clamped
    /// by the two ceilings above), so once the row converges roughly the fraction β of the
    /// remaining error is worked off over that substep. `0` turns every rigid row into a
    /// pure velocity constraint: the error stops growing but is never removed. `1` asks for
    /// all of it inside a single substep.
    ///
    /// Rows with `compliance > 0` derive their bias from the compliance instead and ignore
    /// this. The hinge and slider POSITION SERVOS, however, reuse it as their proportional
    /// gain when converting a target angle/offset into a target velocity — retuning β
    /// retunes how hard those servos chase their target.
    pub position_bias: f32,
    /// `compliance > 0` olan satırların sönüm oranı ζ (yumuşak kısıt yay-damper'ının).
    /// 1.0 = kritik sönüm: yay hedefe salınmadan oturur. Rijit satırlar (compliance = 0)
    /// bu alanı hiç görmez — onlar Baumgarte yolunda kalır.
    pub compliance_damping_ratio: f32,
}

impl Default for JointSolver {
    fn default() -> Self {
        Self {
            iterations: 10,
            max_correction_speed: 5.0,
            max_angular_speed: 5.0,
            position_bias: 0.3,
            compliance_damping_ratio: 1.0,
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
    /// `dt` is the substep length in SECONDS and must be > 0: it divides the Baumgarte bias
    /// (`β·C/dt`) and converts accumulated impulses back into the forces and torques that
    /// are compared against the break thresholds. `transforms` is read-only — positions are
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
    ///    order.
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
    /// inherits them from the previous one. The latched state on the joints themselves does
    /// carry over — `is_broken` and the reference poses survive every call. The map is only
    /// ever looked up in, never iterated, so its hash order does not reach the result; the
    /// order of `joints` does. Single-threaded and same-platform bit-reproducible only.
    pub fn solve_joints(
        &self,
        joints: &mut [Joint],
        entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        dt: f32,
    ) {
        // Birikmiş λ'lar bir çözücü GEÇİŞİNE (= bir substep) aittir; aşağıdaki iterasyonlar
        // bu birikimi yakınsatır. Geçiş başında sıfırla — döngünün İÇİNDE sıfırlamak tüm
        // değişikliği no-op'a indirir.
        //
        // λ adımlar arasında TAŞINMADIĞI için `WorldSnapshot`'a girmesi gerekmez: rollback
        // restore'undan sonraki ilk `solve_joints` onu zaten sıfırdan kurar. Substep'ler
        // arası warm-start eklendiği gün bu tersine döner ve snapshot'a girmesi ZORUNLU olur
        // (bkz. `PhysicsWorld::WorldSnapshot`'taki contact_cache gerekçesi).
        for joint in joints.iter_mut() {
            joint.scratch = JointScratch::default();
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
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }

        for _ in 0..self.iterations {
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
        // VERMEZ, çünkü onlar dış yük değil eyleyicidir (bkz. docs/FIXPLAN.md B4 commit 4).
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

    /// Yumuşak (compliant) bir satırın Box2D v3 soft-constraint katsayıları:
    /// `(bias_rate, mass_scale, impulse_scale)`.
    ///
    /// Temas çözücüsü bu formülasyonu zaten kullanıyor (`solver/tgs.rs:117-124` ve `:597`);
    /// joint çözücüsü Baumgarte β + hız kırpmasında kalmış ve `compliance`'ı efektif kütleye
    /// `α/dt²` (CFM) ekleyerek uyguluyordu. O yol bu motorda ÇALIŞMIYOR, iki ayrı sebepten:
    ///
    /// 1. **CFM tek başına yumuşaklık üretmez.** `k`'yi büyütmek yalnızca her iterasyonun
    ///    adımını küçültür; iterasyon sayısı arttıkça seri yine RİJİT çözüme yakınsar. Yani
    ///    `compliance` fiziksel bir ters-sertlik değil, `iterations`'a bağlı bir gevşetme
    ///    katsayısıydı.
    /// 2. **Eksik geri besleme terimi (`-α̃·λ`) bu çözücüye eklenemiyor.** Denge noktası
    ///    `λ_toplam = bias/α̃` olurdu, ama `bias` burada `max_correction_speed` ile HIZ-KIRPILI:
    ///    kırpma ısırdığı an λ tavanlanıyor, taşınacak yükün çok altında kalıyor ve kısıt
    ///    sessizce boşalıyor (ölçüldü: 2 m'lik halat 600 adımda 27.4 m — serbest düşüş).
    ///
    /// Soft formülasyonda `c` bir ÇARPAN: denge `λ_toplam = c·bias_rate·C = dt·ω²·C`. Gereken
    /// bias küçük kalıyor, kırpma hiç ısırmıyor. CFM'de `λ = bias/α̃` bir BÖLME'ydi ve aynı
    /// yükü taşımak kırpmanın ~14 katı bias istiyordu. Aynı fiziğin iki yazılışı; kırpmalı bir
    /// çözücüde taban tabana zıt koşullanma.
    ///
    /// `ω = √(k/α)` — satırın efektif kütlesinden türetilir (`m_eff = 1/k`), yani
    /// `ω = √(K/m_eff)` klasik yay frekansı. Dengede `λ = dt·ω²·C/k = dt·C/α`, yani
    /// **`F = C/α`: Hooke yasası.** Sertlik sabit, ağır yük daha çok uzatır — `compliance`
    /// bir ters-sertlik olarak ilan edildiğine göre olması gereken de bu.
    ///
    /// # `impulse_scale` terimi `k`'ye BÖLÜNMEZ
    ///
    /// Bu ayrım kozmetik değil, KARARLILIK meselesi. `-impulse_scale·λ` de `/k` ile
    /// bölünürse λ yinelemesi `λ_{n+1} = λ_n·(1 - impulse_scale/k) + …` olur ve
    /// `impulse_scale > 2k`, yani `m_eff > 2/impulse_scale` olduğunda IRAKSAR. Ölçüldü:
    /// bölünen biçimde α = 0.03'lük halat 1 kg'ı 0.2937 m uzatıyor (Hooke: 0.2943 ✓) ama
    /// 4 kg'da kısıt tamamen boşalıp cisim 331 m'ye düşüyor — 2000 adımlık serbest düşüş.
    ///
    /// Bölünmeyen biçimde yineleme `λ_{n+1} = λ_n·(1 - impulse_scale) + …` ve
    /// `impulse_scale = 1/(1+c) ∈ (0, 1]` olduğundan **koşulsuz kararlı**.
    ///
    /// (`solver/tgs.rs:597` temas çözücüsünde terim `/ k_n` ile bölünüyor. Oradaki
    /// `impulse_scale` çok daha küçük — contact_hertz=30, ζ=10 ile ≈0.058 — bu yüzden sınır
    /// `m_eff ≈ 34`'e çıkıyor ve mevcut soak sahnelerinde ısırmıyor. Ölçülmesi gereken ayrı
    /// bir konu; bkz. docs/FIXPLAN.md.)
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

    /// Birikmiş-λ kırpma: `lambda_min/max` ARTIMA değil, geçiş boyunca birikmiş TOPLAMA
    /// uygulanır. Tek yönlü bir satır (limit / halat / koni) böylece NEGATİF artım da
    /// uygulayabilir — toplam doğru tarafta kaldığı sürece kendi önceki aşırı-düzeltmesini
    /// GERİ ALIR. Eskiden her artım ayrı kırpıldığından geri verme mümkün değildi.
    ///
    /// Uygulanan (geri döndürülen) değer artımdır, birikim değil; hızlar artımla güncellenir.
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
            *accum = total;
            delta
        } else {
            let applied = clamped - *accum;
            *accum = clamped;
            applied
        }
    }

    /// Apply a 1-DOF angular velocity constraint along `direction` (hard).
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

    /// Soft (compliant) form of [`Self::apply_angular_constraint`]. `compliance` ≥ 0 is the
    /// inverse stiffness (CFM): the effective mass is regularised by `compliance/dt²`, so a
    /// larger value yields a springier constraint that gives under load (0 = fully rigid,
    /// identical to the hard path). Lets a specific limit/weld be soft without changing the
    /// global Baumgarte factor.
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

        // İki rejim. `compliance == 0` → RİJİT: Baumgarte bias + hız kırpması, bit-aynı
        // korunuyor (motorun bugüne kadar doğru çalışan yolu bu). `compliance > 0` → YUMUŞAK:
        // temas çözücüsüyle aynı soft-constraint formülasyonu, bkz. `soft_coefficients`.
        let (position_bias, mass_scale, impulse_scale) = if compliance > 0.0 {
            let (bias_rate, m, i) = self.soft_coefficients(compliance, k, dt);
            (
                (bias_rate * error).clamp(-self.max_angular_speed, self.max_angular_speed),
                m,
                i,
            )
        } else {
            (
                (self.position_bias * error / dt)
                    .clamp(-self.max_angular_speed, self.max_angular_speed),
                1.0,
                0.0,
            )
        };
        // Doğru terim odur — yumuşak bir satırın denge noktası Jv + α̃·λ_toplam = bias'tır —
        // ama `position_bias` bu çözücüde `max_correction_speed`/`max_angular_speed` ile
        // HIZ-KIRPILI. Kırpma ısırdığı anda denge λ_toplam = bias_max/α̃ değerine tavanlanır;
        // bu, taşınması gereken yükün çok altında kalır ve kısıt sessizce boşalır. Ölçüldü:
        // compliance=0.03, 1 kg yük, dt=1/240 → 2 m'lik halat 600 adımda 27.4 m'ye uzuyor
        // (yani serbest düşüş), oysa doğru statik uzama α·m·g/β = 0.98 m. Kırpmayı 5000'e
        // çekince ölçüm 1.007 m — terim doğru, kırpmayla ETKİLEŞİMİ yanlış.
        // Bu yüzden compliance'ın iterasyon-sayısına bağımlılığı burada KAPANMIYOR; kırpma
        // rejimiyle birlikte ele alınacak (bkz. docs/FIXPLAN.md, B4 sonrası).
        // `impulse_scale` terimi BÖLÜNMEZ — gerekçe `soft_coefficients`'ta (kararlılık).
        let delta = mass_scale * (-vel_err + position_bias) / k - impulse_scale * *scratch.row(slot);
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

    /// Apply a 1-DOF linear velocity constraint along `direction` at the anchor points (hard).
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

    /// Soft (compliant) form of [`Self::apply_linear_constraint`]. See
    /// [`Self::apply_angular_constraint_soft`] — `compliance/dt²` regularises the effective
    /// mass (0 ⇒ rigid).
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

        // İki rejim — gerekçe `apply_angular_constraint_soft`'ta.
        let (position_bias, mass_scale, impulse_scale) = if compliance > 0.0 {
            let (bias_rate, m, i) = self.soft_coefficients(compliance, k, dt);
            (
                (bias_rate * error).clamp(-self.max_correction_speed, self.max_correction_speed),
                m,
                i,
            )
        } else {
            (
                (self.position_bias * error / dt)
                    .clamp(-self.max_correction_speed, self.max_correction_speed),
                1.0,
                0.0,
            )
        };
        // `impulse_scale` terimi BÖLÜNMEZ — gerekçe `soft_coefficients`'ta (kararlılık).
        let delta = mass_scale * (-rel_vel + position_bias) / k - impulse_scale * *scratch.row(slot);
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

    /// 1-DOF doğrusal hız kısıtı, DOĞRU efektif kütleyle tek uygulamada ankor
    /// noktalarındaki bağıl hızı tam olarak sıfırlar (λ = -Jv/k, yeni Jv = Jv + kλ = 0).
    /// Yanlış `k` ile (eski `((I⁻¹r)×n)×r·n`) over/undershoot olur ve bağıl hız ≠ 0 kalır;
    /// bu test bu yüzden doğru çapraz-çarpım sırasını ayırt eder.
    #[test]
    fn linear_constraint_zeroes_relative_velocity_with_correct_effective_mass() {
        let solver = JointSolver::default();

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
        assert!(
            rel_n.abs() < 1e-5,
            "tek uygulamada bağıl hız sıfırlanmalı; kalan = {rel_n} (yanlış efektif kütle?)"
        );
    }

    /// İki eşit kütleli cisim, ankorları kütle merkezinde (r = 0 → k = 1/m + 1/m = 2),
    /// tek yönlü (yalnız ÇEKEN) bir satır. Aradaki tek fark clamp'in nereye uygulandığı.
    fn one_sided_pair() -> ([RigidBody; 2], [Transform; 2], [Velocity; 2]) {
        let body = || RigidBody::new(1.0, false);
        (
            [body(), body()],
            [Transform::new(Vec3::ZERO), Transform::new(Vec3::ZERO)],
            [Velocity::default(), Velocity::default()],
        )
    }

    /// Tek yönlü bir satır kendi ÖNCEKİ impulse'ını GERİ VEREBİLMELİ.
    ///
    /// Eskiden clamp her iterasyonun kendi artımına uygulanıyordu: yalnız-çeken bir satırda
    /// (`lambda_max = 0`) pozitif bir artım her seferinde 0'a kırpıldığından satır, kendi
    /// aşırı-düzeltmesini geri alamıyordu — tek yönlü bir cırcır. Clamp artık geçiş boyunca
    /// birikmiş TOPLAMA uygulanıyor, dolayısıyla toplam doğru tarafta kaldığı sürece
    /// negatif/pozitif her artım uygulanabilir.
    ///
    /// Ayırt edici: eski kodda ikinci çağrı hiçbir şey uygulamaz ve bağıl hız −1.0'da kalır.
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
        assert!(rel(&vels).abs() < 1e-6, "ilk çağrı bağıl hızı sıfırlamalı: {}", rel(&vels));

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
        assert!(
            rel(&vels).abs() < 1e-6,
            "geri verdikten sonra bağıl hız yine sıfır olmalı, kalan = {}",
            rel(&vels)
        );
    }

    /// …ama uyguladığından FAZLASINI geri veremez: biriken toplam sınırı geçemez, yani
    /// yalnız-çeken bir satır hiçbir koşulda İTMEZ.
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

    /// Birikim bir GEÇİŞE ait: `solve_joints` her çağrıda sıfırdan başlamalı. Sıfırlama
    /// olmasa ikinci geçiş, birincinin doymuş λ'sını miras alır ve aynı girdiye farklı
    /// cevap verir — adımlar arası sessiz bir durum sızıntısı.
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
        let map: rustc_hash::FxHashMap<u32, usize> =
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

        assert_eq!(
            joints[0].scratch, lambda_after_one,
            "aynı girdiyle ikinci geçiş aynı λ'yı üretmeli; birikim geçişler arasında taşınmış"
        );
        assert_eq!(v2, v1, "…ve dolayısıyla aynı hızları");
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
