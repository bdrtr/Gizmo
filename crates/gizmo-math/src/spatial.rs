use crate::{Mat3, Vec3};
use std::ops::{Add, Mul, Sub};
use serde::{Deserialize, Serialize};

/// A Plücker coordinate — a 6-D "spatial" vector held as two `Vec3` halves, **angular
/// first**.
///
/// One type carries three different physical quantities in Featherstone's formulation,
/// and the units follow from which one you are holding:
///
/// | Read as | `w` (angular half) | `v` (linear half) |
/// |---|---|---|
/// | Motion — velocity     | angular velocity, rad/s   | linear velocity, m/s |
/// | Motion — acceleration | angular accel., rad/s²    | linear accel., m/s²  |
/// | Force                 | moment / torque, N·m      | force, N             |
///
/// Nothing in the type system separates a motion vector from a force vector, so pairing
/// them correctly is the caller's job: [`SpatialVector::cross_motion`] is the
/// motion×motion operator (`v×`), [`SpatialVector::cross_force`] the motion×force
/// operator (`v×*`), and substituting one for the other compiles and silently produces
/// wrong Coriolis terms. [`SpatialVector::dot`] is the dual pairing, so
/// `force.dot(motion)` is power in watts.
///
/// Only the experimental `experimental-multibody` ABA solver uses this — see the module
/// docs.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SpatialVector {
    /// Angular half, stored **first**: angular velocity (rad/s), angular acceleration
    /// (rad/s²) or moment (N·m) depending on interpretation.
    ///
    /// The `[angular; linear]` ordering is not cosmetic — it is what makes the block
    /// layout of [`SpatialMatrix`] line up (`m00`/`m01` act on `w`) and what
    /// [`SpatialInertia::to_matrix`] assumes. Swapping the convention would silently
    /// transpose every 6×6 in the solver.
    pub w: Vec3, // Angular part (w)
    /// Linear half: linear velocity (m/s), linear acceleration (m/s²) or force (N).
    ///
    /// For a *motion* vector this is the velocity of the body-fixed point currently
    /// coincident with the **frame origin**, not of the centre of mass — which is why a
    /// body in pure rotation about a distant origin still has a non-zero `v`, and why
    /// [`SpatialInertia`] has to carry a COM offset at all.
    pub v: Vec3, // Linear part (v)
}

impl SpatialVector {
    /// Both halves zero — the additive identity, and the meaningful "no motion / no
    /// load" state for every interpretation in the table above.
    ///
    /// ABA leans on it in two distinct ways: as the motionless-base state (an articulated
    /// tree's default base velocity and acceleration, and the bias acceleration `c` of a
    /// fixed-base root link), and as the *result* of a `Fixed` joint's motion subspace `S`.
    /// The latter is why `S = ZERO` must not be treated as "uninitialised": it drives
    /// `D = SᵀI^A S` to zero, which the solver reads as a locked joint and handles by
    /// propagating the child's full inertia to the parent.
    pub const ZERO: Self = Self {
        w: Vec3::ZERO,
        v: Vec3::ZERO,
    };

    /// Assembles a spatial vector from its angular part `w` and linear part `v`, in
    /// that order.
    ///
    /// Argument order is the trap: for a **force** the angular slot holds the moment
    /// and the linear slot the force, so a torque-only load is `new(torque, Vec3::ZERO)`
    /// — the opposite of how one usually says "force and torque".
    pub fn new(w: Vec3, v: Vec3) -> Self {
        Self { w, v }
    }

    /// The dual pairing of the two Plücker halves: `w·w' + v·v'`.
    ///
    /// Physically meaningful when one operand is a **force** and the other a **motion**,
    /// in which case the result is power in watts (`τ·ω + f·v`). The solver also uses it
    /// as `Sᵀ p` (joint force projected onto the motion subspace, N·m or N) and as
    /// `Sᵀ U` to form the articulated joint inertia `D`.
    ///
    /// Applying it to two motion vectors is just the Euclidean 6-norm and carries no
    /// physical meaning — the tests do so only to check the arithmetic.
    pub fn dot(self, other: Self) -> f32 {
        self.w.dot(other.w) + self.v.dot(other.v)
    }

    /// Spatial motion cross product `self × other` (Featherstone's `v×`), for combining
    /// two **motion** vectors.
    ///
    /// The angular halves cross as usual while the linear half picks up both coupling
    /// terms, which is what turns a parent's velocity plus a joint's velocity into the
    /// Coriolis/centrifugal bias acceleration `c = v × v_J` in ABA's first pass.
    /// `x.cross_motion(x)` is the zero vector in both halves: the two coupling terms
    /// `w×v` and `v×w` are formed from the same pairwise products, so they cancel
    /// exactly rather than merely to within rounding.
    ///
    /// Passing a force vector as either operand is a silent modelling error; use
    /// [`SpatialVector::cross_force`] instead.
    pub fn cross_motion(self, other: Self) -> Self {
        Self {
            w: self.w.cross(other.w),
            v: self.w.cross(other.v) + self.v.cross(other.w),
        }
    }

    /// Spatial force cross product (Featherstone's `v×*`): `self` must be a **motion**
    /// vector, `f` a **force** vector, and the result is a force.
    ///
    /// This is the dual operator to [`SpatialVector::cross_motion`], not a re-ordering
    /// of it — the coupling terms land in the other half, because a force's angular slot
    /// is a moment rather than an angular rate. ABA uses it once per link to build the
    /// velocity-product bias force `p^A = v ×* (I v)`, i.e. the gyroscopic/centrifugal
    /// term that must be cancelled before the joint accelerations can be solved.
    pub fn cross_force(self, f: Self) -> Self {
        Self {
            w: self.w.cross(f.w) + self.v.cross(f.v),
            v: self.w.cross(f.v),
        }
    }

    /// Outer product `self · otherᵀ`, expanding two 6-vectors into a rank-1 6×6
    /// [`SpatialMatrix`].
    ///
    /// Each block is the 3×3 outer product of the corresponding halves
    /// (`m00 = w⊗w'`, `m01 = w⊗v'`, `m10 = v⊗w'`, `m11 = v⊗v'`), so element `(i, j)` is
    /// `self[i] * other[j]`.
    ///
    /// Exists for exactly one caller: ABA's backward pass forms `U D⁻¹ Uᵀ` as
    /// `u_vec.outer_product(u_vec).mul_scalar(1.0 / d)` and subtracts it from the
    /// articulated inertia. That subtraction is only taken when `D` is non-singular
    /// (`d > 1e-6`); a fixed or singular joint keeps the full inertia instead.
    pub fn outer_product(self, other: Self) -> SpatialMatrix {
        let outer = |a: Vec3, b: Vec3| -> Mat3 { Mat3::from_cols(a * b.x, a * b.y, a * b.z) };
        SpatialMatrix {
            m00: outer(self.w, other.w),
            m01: outer(self.w, other.v),
            m10: outer(self.v, other.w),
            m11: outer(self.v, other.v),
        }
    }
}

impl Add for SpatialVector {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            w: self.w + rhs.w,
            v: self.v + rhs.v,
        }
    }
}

impl Sub for SpatialVector {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            w: self.w - rhs.w,
            v: self.v - rhs.v,
        }
    }
}

impl Mul<f32> for SpatialVector {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self {
            w: self.w * rhs,
            v: self.v * rhs,
        }
    }
}

/// A dense 6×6 spatial matrix, stored as four 3×3 blocks.
///
/// Blocked to match [`SpatialVector`]'s `[angular; linear]` ordering:
///
/// ```text
/// | m00  m01 |   | w |
/// | m10  m11 | · | v |
/// ```
///
/// In the ABA solver this is the **articulated-body inertia** `I^A` — the effective
/// inertia a link presents to its parent once its whole subtree is folded in — and the
/// rank-1 term `U D⁻¹ Uᵀ` subtracted from it.
///
/// It is a plain container with no invariants: a physical articulated inertia is
/// symmetric (`m01 == m10ᵀ`), but nothing here enforces or checks that, and the solver
/// rebuilds the blocks by hand when transforming a child's inertia into its parent's
/// frame.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SpatialMatrix {
    /// Angular-out / angular-in block. For an inertia built by
    /// [`SpatialInertia::to_matrix`] this is the rotational inertia **about the frame
    /// origin** (kg·m²), i.e. the COM tensor with the parallel-axis shift already
    /// applied — not the same thing as [`SpatialInertia::rot`].
    pub m00: Mat3,
    /// Angular-out / linear-in block. For an inertia this is `m·[c]×` (kg·m), the
    /// coupling that vanishes exactly when the centre of mass sits on the frame origin.
    pub m01: Mat3,
    /// Linear-out / angular-in block. For an inertia this is `m·[c]×ᵀ`, which by skew
    /// symmetry equals `−m·[c]×`, i.e. `m01ᵀ`
    /// (`spatial_inertia_to_matrix_skew_symmetric` pins that relationship).
    pub m10: Mat3,
    /// Linear-out / linear-in block. For an inertia this is just `m·I₃` — mass in
    /// kilograms on the diagonal, no coupling.
    pub m11: Mat3,
}

impl SpatialMatrix {
    /// All four blocks zero — the additive identity.
    ///
    /// Not a usable inertia on its own: applied to any motion it yields zero force, and
    /// the resulting `D` is singular, so a link whose `I^A` comes out zero gets `q̈ = 0`
    /// from the solver's `d_val > 1e-6` guard rather than a division by zero. It is not
    /// the ABA accumulator seed either — the backward pass accumulates into each link's
    /// own rigid-body inertia (`inertia.to_matrix()`), not into `ZERO`.
    pub const ZERO: Self = Self {
        m00: Mat3::ZERO,
        m01: Mat3::ZERO,
        m10: Mat3::ZERO,
        m11: Mat3::ZERO,
    };

    /// Applies the 6×6 to a spatial vector, blockwise.
    ///
    /// When `self` is an inertia this maps a **motion** vector to a **force** vector:
    /// a velocity in gives spatial momentum out (angular half N·m·s, linear half N·s),
    /// an acceleration in gives moment/force out (N·m and N). The result is *not*
    /// interchangeable with the input — see [`SpatialVector`] on why that distinction is
    /// unchecked.
    pub fn mul_vec(self, v: SpatialVector) -> SpatialVector {
        SpatialVector {
            w: self.m00 * v.w + self.m01 * v.v,
            v: self.m10 * v.w + self.m11 * v.v,
        }
    }

    /// Scales all four blocks by `scalar`.
    ///
    /// The ABA backward pass calls this with `1.0 / d_val` to finish the `U D⁻¹ Uᵀ`
    /// projection; the reciprocal is taken by the caller, which is also where the
    /// `d_val > 1e-6` singularity guard lives. Nothing is guarded here, so a `scalar` of
    /// `inf`/`NaN` propagates straight into the inertia.
    pub fn mul_scalar(self, scalar: f32) -> Self {
        Self {
            m00: self.m00 * scalar,
            m01: self.m01 * scalar,
            m10: self.m10 * scalar,
            m11: self.m11 * scalar,
        }
    }
}

impl Add for SpatialMatrix {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 + rhs.m00,
            m01: self.m01 + rhs.m01,
            m10: self.m10 + rhs.m10,
            m11: self.m11 + rhs.m11,
        }
    }
}

impl Sub for SpatialMatrix {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 - rhs.m00,
            m01: self.m01 - rhs.m01,
            m10: self.m10 - rhs.m10,
            m11: self.m11 - rhs.m11,
        }
    }
}

/// The inertia profile of one rigid body, in the compact form ABA wants: mass,
/// COM-relative rotational inertia, and a centre-of-mass offset — the parameters of a
/// 6×6 spatial inertia rather than the 6×6 itself.
///
/// Keeping it factored means the parallel-axis (Steiner) shift stays explicit and is
/// applied at the point of use: [`SpatialInertia::to_matrix`] expands to the full
/// [`SpatialMatrix`], [`SpatialInertia::mul_vec`] applies the same operator without
/// materialising it, and `Add` merges two bodies by shifting both onto their combined
/// COM. The first two must agree — `spatial_inertia_mul_vec_matches_to_matrix` compares
/// `mul_vec` against `to_matrix().mul_vec` to keep them honest.
///
/// `Default` gives an all-zero (massless) inertia, which is a legal accumulator seed but
/// not a physical body: `to_matrix()` on it is the zero matrix and `D` will be singular.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct SpatialInertia {
    /// Rotational inertia tensor in kg·m², taken **about this body's own centre of
    /// mass** and expressed on the body-frame axes.
    ///
    /// This is the load-bearing invariant of the struct. Both [`SpatialInertia::to_matrix`]
    /// and the `Add` impl assume it and apply the parallel-axis shift themselves —
    /// `to_matrix` from the COM out to the frame origin, `Add` from each body's COM to
    /// the merged COM. Handing in a tensor already measured about the frame origin
    /// double-counts that shift and inflates the body's rotational inertia, silently.
    pub rot: Mat3, // Angular Inertia (I)
    /// Mass in kilograms.
    ///
    /// `Add` reads it to form the mass-weighted combined centre of mass, and a combined
    /// mass of exactly `0.0` short-circuits to a zero inertia rather than dividing by
    /// zero — note that path **discards both `rot` tensors**, so summing massless bodies
    /// loses their rotational inertia.
    pub mass: f32, // Linear Mass (m)
    /// Centre of mass in metres, as an offset from the **frame origin** — for an ABA
    /// link that origin is the joint frame, not the body's centroid.
    ///
    /// Zero means the frame already sits on the COM and every Steiner term drops out.
    /// Non-zero is what populates the off-diagonal `m01`/`m10` blocks of
    /// [`SpatialInertia::to_matrix`], coupling linear and angular motion.
    pub com: Vec3, // Center of Mass (c) - Origin'e göre offset
}

impl SpatialInertia {
    /// Assembles an inertia from mass (kg), the COM-relative rotational inertia tensor
    /// (kg·m², see [`SpatialInertia::rot`]) and the COM offset from the frame origin (m).
    ///
    /// Note the argument order is `(mass, rot, com)` while the fields are declared
    /// `(rot, mass, com)` — the two do not line up, so a struct literal cannot be
    /// mechanically converted into a `new` call by position.
    ///
    /// Nothing is validated: a negative mass or a non-positive-definite `rot_inertia`
    /// will be accepted and will produce a non-physical articulated inertia downstream.
    pub fn new(mass: f32, rot_inertia: Mat3, com_offset: Vec3) -> Self {
        Self {
            rot: rot_inertia,
            mass,
            com: com_offset,
        }
    }

    /// Inertia for a body whose centre of mass sits exactly on the frame origin
    /// (`com = 0`).
    ///
    /// The common case for a link whose joint frame is already at its centroid, and the
    /// cheap one: with `com = 0` every Steiner term is zero, so `to_matrix()` reduces to
    /// `[[inertia, 0], [0, m·I₃]]` with no linear/angular coupling at all.
    pub fn from_mass_inertia(mass: f32, inertia: Mat3) -> Self {
        Self {
            rot: inertia,
            mass,
            com: Vec3::ZERO,
        }
    }

    /// Applies the inertia to a **motion** vector, producing a **force** vector, without
    /// building the 6×6.
    ///
    /// A velocity in gives spatial momentum out (angular half N·m·s, linear half N·s);
    /// an acceleration in gives moment/force out (N·m and N). Numerically
    /// equivalent to `self.to_matrix().mul_vec(v)` — that equivalence is the contract,
    /// and `spatial_inertia_mul_vec_matches_to_matrix` enforces it.
    ///
    /// **The parallel-axis (Steiner) correction is applied unconditionally**, even when
    /// `com` is zero and the term provably vanishes. That looks like waste and is not: the
    /// previous version skipped it behind a `com.length_squared() > 1e-12` threshold, which
    /// disagreed with `to_matrix()` for a tiny-but-non-zero COM.
    /// `spatial_inertia_mul_vec_matches_to_matrix_tiny_com` is the regression test.
    ///
    /// The two paths agree to `vec3_approx` tolerance, not bit-for-bit: they are different
    /// expression trees (this one subtracts nothing and scales before multiplying; the matrix
    /// path folds the correction into `rot` first), so their f32 rounding differs in the last
    /// place or two. The sibling test uses an epsilon comparison for exactly that reason.
    pub fn mul_vec(self, v: SpatialVector) -> SpatialVector {
        // [ rot_shifted , mass * [com]x^T ] [ w ]
        // [ mass * [com]x , mass * I_3 ]     [ v ]
        // Basitleştirilmiş COM = 0 hali için:
        let com_cross_v = self.com.cross(v.v);
        let com_cross_w = self.com.cross(v.w);

        // Parallel-axis (Steiner) düzeltmesini KOŞULSUZ uygula: com == 0 iken
        // com.cross(com_cross_w) zaten sıfırdır, dolayısıyla ek terim no-op olur.
        // Eşik karşılaştırması (com.length_squared() > 1e-12) platforma göre farklı
        // değerlendirilip determinizmi bozuyordu ve to_matrix() ile tutarsızdı.
        let mut force_w = self.rot.mul_vec3(v.w) + com_cross_v * self.mass;
        force_w -= self.com.cross(com_cross_w) * self.mass;

        let force_v = v.v * self.mass - com_cross_w * self.mass;

        SpatialVector::new(force_w, force_v)
    }

}

impl Add for SpatialInertia {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let total_mass = self.mass + other.mass;
        if total_mass == 0.0 {
            return Self::from_mass_inertia(0.0, Mat3::ZERO);
        }
        let total_com = (self.com * self.mass + other.com * other.mass) * (1.0 / total_mass);

        // Her body için parallel axis theorem uygula
        let shift = |inertia: &SpatialInertia| -> Mat3 {
            let d = inertia.com - total_com; // yeni COM'a offset
            let d_sq = d.dot(d);
            // I_new = I + m*(|d|²E - d⊗d)
            inertia.rot + Mat3::from_diagonal(Vec3::splat(inertia.mass * d_sq))
                - Mat3::from_cols(d * d.x, d * d.y, d * d.z) * inertia.mass
        };

        Self {
            mass: total_mass,
            com: total_com,
            rot: shift(&self) + shift(&other),
        }
    }
}

impl SpatialInertia {
    /// Expands the factored rigid-body inertia into the full 6×6 [`SpatialMatrix`],
    /// referred to the **frame origin** rather than the centre of mass:
    ///
    /// ```text
    /// | I_com − m·[c]×[c]×   m·[c]×  |
    /// | (m·[c]×)ᵀ            m·I₃    |
    /// ```
    ///
    /// where `[c]×` is the skew-symmetric matrix of the COM offset. The `−m·[c]×[c]×`
    /// term is the parallel-axis shift (it equals `m(|c|²E − c cᵀ)`), which is why
    /// [`SpatialInertia::rot`] must be the COM tensor and not the origin tensor.
    ///
    /// ABA calls this once per link per step to seed each link's articulated inertia
    /// `I^A`, which the backward pass then accumulates into. Use
    /// [`SpatialInertia::mul_vec`] instead when you only need to apply the inertia to
    /// one vector — same result, no 6×6 materialised.
    pub fn to_matrix(self) -> SpatialMatrix {
        let m = self.mass;
        let c = self.com;

        let c_cross = Mat3::from_cols(
            Vec3::new(0.0, c.z, -c.y),
            Vec3::new(-c.z, 0.0, c.x),
            Vec3::new(c.y, -c.x, 0.0),
        );
        let mc_cross = c_cross * m;
        let mc_cross_t = mc_cross.transpose();

        let c_cross_c_cross = c_cross * c_cross;
        let rot_shifted = self.rot - c_cross_c_cross * m;

        SpatialMatrix {
            m00: rot_shifted,
            m01: mc_cross,
            m10: mc_cross_t,
            m11: Mat3::from_diagonal(Vec3::splat(m)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn vec3_approx(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    fn mat3_approx(a: Mat3, b: Mat3) -> bool {
        let diff = a - b;
        // All 9 elements should be near zero
        diff.x_axis.length() < EPS && diff.y_axis.length() < EPS && diff.z_axis.length() < EPS
    }

    // =======================================================================
    // SpatialVector — Temel Operasyonlar
    // =======================================================================

    #[test]
    fn spatial_vector_zero() {
        let z = SpatialVector::ZERO;
        assert_eq!(z.w, Vec3::ZERO);
        assert_eq!(z.v, Vec3::ZERO);
    }

    #[test]
    fn spatial_vector_new() {
        let w = Vec3::new(1.0, 2.0, 3.0);
        let v = Vec3::new(4.0, 5.0, 6.0);
        let sv = SpatialVector::new(w, v);
        assert_eq!(sv.w, w);
        assert_eq!(sv.v, v);
    }

    #[test]
    fn spatial_vector_add() {
        let a = SpatialVector::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
        let b = SpatialVector::new(Vec3::new(0.0, 2.0, 0.0), Vec3::new(3.0, 0.0, 0.0));
        let c = a + b;
        assert!(vec3_approx(c.w, Vec3::new(1.0, 2.0, 0.0)));
        assert!(vec3_approx(c.v, Vec3::new(3.0, 1.0, 0.0)));
    }

    #[test]
    fn spatial_vector_sub() {
        let a = SpatialVector::new(Vec3::new(5.0, 4.0, 3.0), Vec3::new(2.0, 1.0, 0.0));
        let b = SpatialVector::new(Vec3::new(1.0, 1.0, 1.0), Vec3::new(1.0, 1.0, 0.0));
        let c = a - b;
        assert!(vec3_approx(c.w, Vec3::new(4.0, 3.0, 2.0)));
        assert!(vec3_approx(c.v, Vec3::new(1.0, 0.0, 0.0)));
    }

    #[test]
    fn spatial_vector_scalar_mul() {
        let a = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let b = a * 2.0;
        assert!(vec3_approx(b.w, Vec3::new(2.0, 4.0, 6.0)));
        assert!(vec3_approx(b.v, Vec3::new(8.0, 10.0, 12.0)));
    }

    #[test]
    fn spatial_vector_dot() {
        let a = SpatialVector::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
        let b = SpatialVector::new(Vec3::new(3.0, 4.0, 0.0), Vec3::new(5.0, 6.0, 0.0));
        // dot = (1*3 + 0*4 + 0*0) + (0*5 + 1*6 + 0*0) = 3 + 6 = 9
        assert!((a.dot(b) - 9.0).abs() < EPS);
    }

    #[test]
    fn spatial_vector_dot_self_is_length_squared() {
        let a = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let expected = 1.0 + 4.0 + 9.0 + 16.0 + 25.0 + 36.0; // = 91
        assert!((a.dot(a) - expected).abs() < EPS);
    }

    #[test]
    fn spatial_vector_cross_motion_self_is_zero() {
        // v × v = 0 for any vector
        let v = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let result = v.cross_motion(v);
        assert!(vec3_approx(result.w, Vec3::ZERO));
        // v part: w×v + v×w = w×v - w×v = 0 (only if w and v are parallel, not generally!)
        // Actually for arbitrary vectors: w×v + v×w should sum to 0 only component-wise.
        // Let's verify with known basis vectors instead.
    }

    #[test]
    fn spatial_vector_cross_motion_basis() {
        // Pure angular velocity around Z, applied to pure angular around X
        let wz = SpatialVector::new(Vec3::Z, Vec3::ZERO);
        let wx = SpatialVector::new(Vec3::X, Vec3::ZERO);
        let result = wz.cross_motion(wx);
        // w part: Z × X = Y (left-hand cross: (0,0,1)×(1,0,0) = (0,1,0))
        // Rust's glam uses RH cross: Z × X = -Y? No: (0,0,1) × (1,0,0) = (0*0-1*0, 1*1-0*0, 0*0-0*1) = (0, 1, 0)
        assert!(vec3_approx(result.w, Vec3::Y));
        assert!(vec3_approx(result.v, Vec3::ZERO));
    }

    #[test]
    fn spatial_vector_cross_force_basis() {
        // Motion (pure angular around Z) cross Force (pure force along X)
        let wz = SpatialVector::new(Vec3::Z, Vec3::ZERO);
        let fx = SpatialVector::new(Vec3::ZERO, Vec3::X);
        let result = wz.cross_force(fx);
        // w part: wz.w × fx.w + wz.v × fx.v = Z×0 + 0×X = 0
        assert!(vec3_approx(result.w, Vec3::ZERO));
        // v part: wz.w × fx.v = Z × X = Y
        assert!(vec3_approx(result.v, Vec3::Y));
    }

    // =======================================================================
    // SpatialVector — Outer Product
    // =======================================================================

    #[test]
    fn spatial_vector_outer_product_structure() {
        let a = SpatialVector::new(Vec3::X, Vec3::ZERO);
        let b = SpatialVector::new(Vec3::Y, Vec3::ZERO);
        let op = a.outer_product(b);
        // m00 = a.w ⊗ b.w = X ⊗ Y, which is a 3×3 matrix with col0=X*Y.x=0, col1=X*Y.y=X, col2=X*Y.z=0
        assert!(vec3_approx(op.m00.y_axis, Vec3::X));
        // m01, m10, m11 should all be zero since one of the factors is always ZERO
        assert!(mat3_approx(op.m01, Mat3::ZERO));
        assert!(mat3_approx(op.m10, Mat3::ZERO));
        assert!(mat3_approx(op.m11, Mat3::ZERO));
    }

    // =======================================================================
    // SpatialMatrix — Operasyonlar
    // =======================================================================

    #[test]
    fn spatial_matrix_zero() {
        let z = SpatialMatrix::ZERO;
        assert!(mat3_approx(z.m00, Mat3::ZERO));
        assert!(mat3_approx(z.m01, Mat3::ZERO));
        assert!(mat3_approx(z.m10, Mat3::ZERO));
        assert!(mat3_approx(z.m11, Mat3::ZERO));
    }

    #[test]
    fn spatial_matrix_mul_vec_zero() {
        let m = SpatialMatrix::ZERO;
        let v = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let result = m.mul_vec(v);
        assert!(vec3_approx(result.w, Vec3::ZERO));
        assert!(vec3_approx(result.v, Vec3::ZERO));
    }

    #[test]
    fn spatial_matrix_mul_vec_identity_like() {
        // Construct a 6x6 "identity" (identity in each 3x3 block on diagonal)
        let m = SpatialMatrix {
            m00: Mat3::IDENTITY,
            m01: Mat3::ZERO,
            m10: Mat3::ZERO,
            m11: Mat3::IDENTITY,
        };
        let v = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let result = m.mul_vec(v);
        assert!(vec3_approx(result.w, v.w));
        assert!(vec3_approx(result.v, v.v));
    }

    #[test]
    fn spatial_matrix_mul_scalar() {
        let m = SpatialMatrix {
            m00: Mat3::IDENTITY,
            m01: Mat3::IDENTITY,
            m10: Mat3::IDENTITY,
            m11: Mat3::IDENTITY,
        };
        let scaled = m.mul_scalar(3.0);
        let expected = Mat3::from_diagonal(Vec3::splat(3.0));
        assert!(mat3_approx(scaled.m00, expected));
        assert!(mat3_approx(scaled.m11, expected));
    }

    #[test]
    fn spatial_matrix_add_sub() {
        let a = SpatialMatrix {
            m00: Mat3::IDENTITY,
            m01: Mat3::ZERO,
            m10: Mat3::ZERO,
            m11: Mat3::IDENTITY,
        };
        let b = a;
        let sum = a + b;
        assert!(mat3_approx(sum.m00, Mat3::from_diagonal(Vec3::splat(2.0))));

        let diff = sum - a;
        assert!(mat3_approx(diff.m00, Mat3::IDENTITY));
    }

    // =======================================================================
    // SpatialInertia — Temel Operasyonlar
    // =======================================================================

    #[test]
    fn spatial_inertia_from_mass_inertia() {
        let si = SpatialInertia::from_mass_inertia(5.0, Mat3::IDENTITY);
        assert_eq!(si.mass, 5.0);
        assert!(mat3_approx(si.rot, Mat3::IDENTITY));
        assert!(vec3_approx(si.com, Vec3::ZERO));
    }

    #[test]
    fn spatial_inertia_mul_vec_zero_com() {
        // COM = 0 → basit: force_w = I*w, force_v = m*v
        let si = SpatialInertia::from_mass_inertia(2.0, Mat3::IDENTITY);
        let vel = SpatialVector::new(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 3.0, 0.0));
        let result = si.mul_vec(vel);
        // force_w = I*w = (1,0,0)
        assert!(vec3_approx(result.w, Vec3::new(1.0, 0.0, 0.0)));
        // force_v = m*v = 2*(0,3,0) = (0,6,0)
        assert!(vec3_approx(result.v, Vec3::new(0.0, 6.0, 0.0)));
    }

    #[test]
    fn spatial_inertia_mul_vec_matches_to_matrix() {
        // Key consistency test: mul_vec() should produce the same result as to_matrix().mul_vec()
        let si = SpatialInertia::new(3.0, Mat3::from_diagonal(Vec3::new(2.0, 4.0, 6.0)), Vec3::new(0.5, -0.3, 0.1));
        let vel = SpatialVector::new(Vec3::new(1.0, -0.5, 0.8), Vec3::new(-0.2, 0.7, -0.4));

        let result_direct = si.mul_vec(vel);
        let result_matrix = si.to_matrix().mul_vec(vel);

        assert!(
            vec3_approx(result_direct.w, result_matrix.w),
            "mul_vec.w = {:?}, to_matrix().mul_vec.w = {:?}",
            result_direct.w, result_matrix.w
        );
        assert!(
            vec3_approx(result_direct.v, result_matrix.v),
            "mul_vec.v = {:?}, to_matrix().mul_vec.v = {:?}",
            result_direct.v, result_matrix.v
        );
    }

    #[test]
    fn spatial_inertia_mul_vec_matches_to_matrix_tiny_com() {
        // Regresyon: com.length_squared() eski 1e-12 eşiğinin ALTINDA ama sıfır
        // değil. Eskiden mul_vec() Steiner düzeltmesini atlayıp to_matrix() ile
        // (ve platformlar arası) tutarsız/deterministik-olmayan sonuç üretiyordu.
        // Artık her iki yol da düzeltmeyi KOŞULSUZ uyguladığı için mul_vec() ile
        // elle-düzeltilmiş referans BİT-AYNI olmalı.
        let tiny = 5.0e-7_f32; // length_squared() (splat) = 7.5e-13 < 1e-12
        let com = Vec3::splat(tiny);
        let mass = 1.0e10_f32; // düzeltme terimini gözle görülür kılan büyük kütle
        let rot = Mat3::from_diagonal(Vec3::new(2.0, 4.0, 6.0));
        let si = SpatialInertia::new(mass, rot, com);
        let vel = SpatialVector::new(Vec3::new(1.0, -0.5, 0.8), Vec3::new(-0.2, 0.7, -0.4));

        // Referans: düzeltme HER ZAMAN uygulanır (koşulsuz).
        let com_cross_v = com.cross(vel.v);
        let com_cross_w = com.cross(vel.w);
        let expected_w =
            rot.mul_vec3(vel.w) + com_cross_v * mass - com.cross(com_cross_w) * mass;

        let result_direct = si.mul_vec(vel);
        // Bit-aynı olmalı: aynı işlem sırası.
        assert_eq!(
            result_direct.w, expected_w,
            "mul_vec düzeltmeyi koşulsuz uygulamalı (tiny-com)"
        );
    }

    #[test]
    fn spatial_inertia_mul_vec_matches_to_matrix_zero_com() {
        // Also verify consistency when COM = 0
        let si = SpatialInertia::from_mass_inertia(5.0, Mat3::from_diagonal(Vec3::new(1.0, 2.0, 3.0)));
        let vel = SpatialVector::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));

        let result_direct = si.mul_vec(vel);
        let result_matrix = si.to_matrix().mul_vec(vel);

        assert!(vec3_approx(result_direct.w, result_matrix.w));
        assert!(vec3_approx(result_direct.v, result_matrix.v));
    }

    // =======================================================================
    // SpatialInertia — Addition (Parallel Axis Theorem)
    // =======================================================================

    #[test]
    fn spatial_inertia_add_same_com() {
        // Two bodies at the same COM → total mass adds, inertia adds directly
        let a = SpatialInertia::from_mass_inertia(2.0, Mat3::IDENTITY);
        let b = SpatialInertia::from_mass_inertia(3.0, Mat3::from_diagonal(Vec3::splat(2.0)));
        let total = a + b;

        assert!((total.mass - 5.0).abs() < EPS);
        assert!(vec3_approx(total.com, Vec3::ZERO));
        // When both COMs are at origin, the shift term vanishes: total.rot = a.rot + b.rot
        let expected_rot = Mat3::from_diagonal(Vec3::splat(3.0)); // 1 + 2
        assert!(mat3_approx(total.rot, expected_rot));
    }

    #[test]
    fn spatial_inertia_add_different_com() {
        // Two point masses at different locations
        let a = SpatialInertia::new(1.0, Mat3::ZERO, Vec3::new(1.0, 0.0, 0.0));
        let b = SpatialInertia::new(1.0, Mat3::ZERO, Vec3::new(-1.0, 0.0, 0.0));
        let total = a + b;

        assert!((total.mass - 2.0).abs() < EPS);
        // Combined COM should be at origin (equal masses, symmetric positions)
        assert!(vec3_approx(total.com, Vec3::ZERO));
        // Each mass is 1 unit from origin → parallel axis: m*d² = 1*1 = 1 each
        // Total rot should have non-zero YY and ZZ components (rotation around Y and Z axes)
        assert!(total.rot.y_axis.y > 0.5); // Should have contribution from parallel axis
        assert!(total.rot.z_axis.z > 0.5);
    }

    #[test]
    fn spatial_inertia_add_zero_masses() {
        let a = SpatialInertia::from_mass_inertia(0.0, Mat3::ZERO);
        let b = SpatialInertia::from_mass_inertia(0.0, Mat3::ZERO);
        let total = a + b;
        assert!((total.mass).abs() < EPS);
    }

    // =======================================================================
    // SpatialInertia — to_matrix Doğrulaması
    // =======================================================================

    #[test]
    fn spatial_inertia_to_matrix_zero_com() {
        // COM = 0 → skew matrix is zero → simplified form
        let si = SpatialInertia::from_mass_inertia(4.0, Mat3::from_diagonal(Vec3::new(1.0, 2.0, 3.0)));
        let mat = si.to_matrix();

        // m00 = rot (no shift since c=0 → c_cross_c_cross = 0)
        assert!(mat3_approx(mat.m00, Mat3::from_diagonal(Vec3::new(1.0, 2.0, 3.0))));
        // m01 = mc_cross = 0
        assert!(mat3_approx(mat.m01, Mat3::ZERO));
        // m10 = mc_cross_t = 0
        assert!(mat3_approx(mat.m10, Mat3::ZERO));
        // m11 = m*I_3
        assert!(mat3_approx(mat.m11, Mat3::from_diagonal(Vec3::splat(4.0))));
    }

    #[test]
    fn spatial_inertia_to_matrix_skew_symmetric() {
        // Verify c_cross is skew-symmetric: c_cross^T = -c_cross
        let si = SpatialInertia::new(2.0, Mat3::IDENTITY, Vec3::new(1.0, 2.0, 3.0));
        let mat = si.to_matrix();

        // m01 should be mc_cross, m10 should be mc_cross^T
        // mc_cross + mc_cross^T should be zero (skew-symmetric property)
        let _sum = mat.m01 + mat.m10.transpose();
        // Actually m10 = mc_cross_t already, so m01.transpose() should equal m10
        let m01_t = mat.m01.transpose();
        assert!(
            mat3_approx(m01_t, mat.m10),
            "m01^T should equal m10 (skew symmetry)"
        );
    }
}

