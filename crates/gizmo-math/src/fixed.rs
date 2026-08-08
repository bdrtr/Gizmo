//! Fixed-Point Maths Library
//!
//! Floating-point (f32, f64) numbers can give different rounding errors on different CPU
//! architectures (x86, ARM) or under different compiler optimisations.
//! This library contains the Q16.16 and Q32.32 fixed-point structures in order to provide
//! 100% Bit-Exact Cross-Platform Determinism in Multiplayer, eSports and Lock-step RTS games.
//!
//! ## Status: EXPERIMENTAL — the simulation does not use this module
//!
//! Nothing in the engine depends on [`Fp32`] or [`FpVec3`]. Gizmo's physics state
//! (Transform/Velocity/solver) runs entirely on `glam`/`f32`, and the determinism guarantee
//! is deliberately **same-platform only** — replay and rollback bit-equality, verified via
//! `state_hash`. Cross-platform bit-exactness is an explicit non-goal precisely because it
//! would require a fixed-point or softfloat migration; this module is the prototype kept
//! around for that possible post-1.0 feature. Treat it as a sketch, not as production
//! maths. See `docs/ENGINE.md` §5.
//!
//! Two consequences of that status, so nobody is caught out:
//!
//! - Only the **Q16.16** type exists. The `Q32.32` promised in the Turkish header above was
//!   never written.
//! - Precision is well below what "bit-exact" suggests to most readers. Bit-exact here means
//!   *reproducible* — every operation is pure integer arithmetic, so the same inputs give
//!   the same bits on every CPU and at every optimisation level. It does not mean *accurate*:
//!   [`Fp32::sqrt`] resolves only to 1/256, and [`Fp32::sin`]/[`Fp32::cos`] are a Bhaskara I
//!   approximation good to a couple of percent.
//!
//! ## Saturating contract
//!
//! Nothing here wraps: overflow saturates to [`Fp32::MAX`]/[`Fp32::MIN`], division by zero
//! returns a saturated value instead of trapping, `x % 0` returns [`Fp32::ZERO`], and `sqrt`
//! of a negative returns zero. Wrapping was the original behaviour and produced sign-flipped
//! garbage (`fp32_from_i32_saturates_out_of_range` and `fp32_abs_neg_saturate_at_min` at the
//! bottom of this file pin the two worst cases); saturation keeps a bad value bounded and,
//! crucially, still deterministic — a clamp is reproducible, a wrap is merely surprising.
//!
//! One panic path survives: `Rem` guards a zero divisor and then defers to `i32::%`, so
//! `Fp32::MIN % Fp32::from_raw(-1)` panics with "attempt to calculate the remainder with
//! overflow". That happens in release builds too — Rust checks the `MIN % -1` pair
//! unconditionally rather than emit the undefined machine instruction.
//!
//! The trade-off is that arithmetic is **not associative near the limits**: `(MAX + ONE)`
//! clamps to `MAX`, so `(MAX + ONE) - ONE` yields `MAX - ONE` rather than `MAX`. A clamped
//! result is a modelling error to fix upstream (rescale your world units), not a behaviour
//! to rely on.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Q16.16 Fixed-Point Number (32-bit)
///
/// 16 bits represent the integer part, 16 bits the fractional part.
///
/// A signed 32-bit fixed-point scalar: the stored integer is the real value scaled by
/// 2^16. Range `[-32768.0, 32767.99998]`, resolution `2^-16 ≈ 1.526e-5` — *uniform* across
/// the whole range, unlike `f32`. That uniformity is the reason the type is reproducible,
/// and also the reason it runs out of headroom at world coordinates a float would shrug off.
///
/// **Experimental — the engine's simulation does not use this type.** See the module docs.
///
/// Unlike `f32` it has no NaN and no signed zero, so the derived `Eq`/`Ord`/`Hash` are total
/// and numerically meaningful (the raw→value mapping is strictly increasing). Values can go
/// straight into a `HashMap` key or a checksum. `Serialize` writes the raw integer, so a
/// serde round-trip is exact — which is the whole point of the type for a lockstep or replay
/// wire format.
///
/// Rounding is **not uniform across operations**, which is worth internalising before
/// trusting a long chain of them: `Mul` re-normalises with an arithmetic `>>` and therefore
/// rounds toward -∞ (a negative product can land one raw unit below the truncated answer);
/// `Div` uses integer division and rounds toward zero; [`Self::to_i32`] truncates toward
/// zero; [`Self::sqrt`] floors onto a 1/256 grid. All are deterministic, none are
/// round-to-nearest, and their biases do not cancel.
///
/// The saturating contract in action:
///
/// ```
/// # use gizmo_math::Fp32;
/// assert_eq!(Fp32::MIN.abs(), Fp32::MAX);        // saturates instead of overflowing
/// assert_eq!(Fp32::ONE / Fp32::ZERO, Fp32::MAX); // division by zero is total
/// ```
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct Fp32(
    /// The raw Q16.16 integer: the represented value multiplied by 65536 ([`Fp32::ONE_RAW`])
    /// and truncated. Public so callers can hash, serialize or bit-compare a value without a
    /// lossy detour through `f32`; construct from it with [`Fp32::from_raw`] rather than
    /// scaling by hand. Every method on this type is a thin wrapper over integer arithmetic
    /// on this one field, so reading it directly (as [`FpVec3::normalize`] does for its
    /// `len.0 > 0` guard) costs nothing and hides nothing.
    pub i32,
);

impl Fp32 {
    /// Number of fractional bits — the "16" after the point in Q16.16. Doubles as the shift
    /// amount used to re-normalise a product (`Mul`) or a dividend (`Div`), so it defines
    /// the format rather than parameterising it: changing it would silently invalidate every
    /// constant below and the `<< 8` in [`Self::sqrt`]. Not a tuning knob.
    pub const SHIFT: usize = 16;
    /// Raw representation of `1.0`, i.e. `1 << SHIFT` = 65536. It is the scale factor
    /// between a real number and its raw form (`raw = value * ONE_RAW`) and the reciprocal
    /// of the type's resolution — the smallest representable step is `1 / 65536 ≈ 1.526e-5`.
    pub const ONE_RAW: i32 = 1 << Self::SHIFT;

    /// Additive identity, and the value every guarded failure path in this module falls back
    /// to: `sqrt` of a negative, `x % 0`, `0 / 0`, `from_f32(NaN)` and
    /// [`FpVec3::normalize`] of a degenerate vector all land here instead of panicking or
    /// producing a NaN analogue. A zero result is therefore ambiguous — it may be an answer
    /// or a swallowed error.
    pub const ZERO: Self = Self(0);
    /// Multiplicative identity (raw 65536). `a * ONE == a` holds *exactly*: the `>> SHIFT`
    /// in `Mul` undoes the scale with no rounding, which is not true of products in general.
    pub const ONE: Self = Self(Self::ONE_RAW);
    /// `-1.0`, exactly the negation of [`Self::ONE`] (negating a non-extreme value is
    /// lossless). Provided as a constant because `Neg` is not `const`, so `-Fp32::ONE`
    /// cannot appear in a `const` item.
    pub const MINUS_ONE: Self = Self(-Self::ONE_RAW);
    /// Largest representable value: `i32::MAX` raw ≈ `32767.99998`. This is the saturation
    /// ceiling for every arithmetic operation and the result of `positive / ZERO`, so `MAX`
    /// showing up in a computation usually means an overflow was clamped rather than that
    /// the maths genuinely produced it.
    pub const MAX: Self = Self(i32::MAX);
    /// Smallest representable value: `i32::MIN` raw, exactly `-32768.0`. Note the
    /// two's-complement asymmetry it inherits — `MIN` has no positive counterpart, so both
    /// `-MIN` and `MIN.abs()` saturate to [`Self::MAX`], one raw unit short of the true
    /// magnitude.
    pub const MIN: Self = Self(i32::MIN);

    // Pi yaklaşımları
    /// π in radians: `round(π * 65536) = 205887`, i.e. `3.1415863`, about `6.4e-6` rad below
    /// the true value. Radians are the angular unit throughout this module. This is the
    /// reference constant — [`Self::TWO_PI`] and [`Self::HALF_PI`] are derived from it and
    /// inherit its error, and [`Self::sin`] uses it both for range reduction and inside the
    /// approximation itself.
    pub const PI: Self = Self(205887); // 3.1415926 * 65536
    /// 2π, exactly `2 * PI` in raw units. Used as the period in [`Self::sin`]'s range
    /// reduction. Because it is ~`1.3e-5` rad short of true 2π, sine accuracy degrades with
    /// the number of periods an argument must be folded through — keep accumulated angles
    /// wrapped rather than letting them grow without bound.
    pub const TWO_PI: Self = Self(411774); // 2 * PI  (205887 * 2)
    /// π/2, computed as `PI / 2` under integer division and therefore **truncated**: the
    /// exact half would be 102943.5. The visible consequence is that `HALF_PI + HALF_PI` is
    /// 205886, one raw unit *below* [`Self::PI`] — the three constants are not perfectly
    /// consistent with one another. [`Self::cos`] is defined as `sin(x + HALF_PI)` and so
    /// inherits that truncation.
    pub const HALF_PI: Self = Self(102943); // PI / 2  (205887 / 2)

    /// Reinterprets an already-scaled integer as Q16.16 (value = `raw / 65536`), performing
    /// no conversion of its own — the inverse of reading the `.0` field. `const`, unlike the
    /// arithmetic operators, so this is the way to declare fixed-point constants.
    #[inline]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Converts a whole number to Q16.16. Exact for `val` in `[-32768, 32767]`; anything
    /// outside that saturates to [`Self::MIN`]/[`Self::MAX`] rather than wrapping.
    ///
    /// The saturation is a *fixed bug*, not defensive padding: the original `val << SHIFT`
    /// wrapped, so `from_i32(40000)` came back as `-25536` — an out-of-range input silently
    /// changed sign, while `from_f32(40000.0)` on the same magnitude saturated. Pinned by
    /// `fp32_from_i32_saturates_out_of_range`.
    #[inline]
    pub fn from_i32(val: i32) -> Self {
        // Saturating — consistent with `from_f32` and every arithmetic op. Plain
        // `val << SHIFT` silently WRAPPED for |val| >= 32768 (Q16.16 tops out at
        // ±32767.99…), turning an out-of-range integer into a wrong-signed value
        // (e.g. `from_i32(40000)` → -25536) while `from_f32(40000.0)` saturated to
        // ~32768. `val * ONE_RAW == val << SHIFT` in range; saturates past it.
        Self(val.saturating_mul(Self::ONE_RAW))
    }

    /// Converts a float to Q16.16 by scaling and truncating toward zero. Meant for the
    /// *boundary*: turn designer/asset floats into fixed point once, then stay in fixed
    /// point. Round-tripping through `f32` mid-computation re-introduces exactly the
    /// platform-dependent rounding this module exists to avoid.
    ///
    /// Total, like everything here — Rust's float→int cast saturates, so `±inf` and
    /// out-of-range values clamp to [`Self::MIN`]/[`Self::MAX`], and `NaN` maps to
    /// [`Self::ZERO`] (silently: there is no error channel).
    ///
    /// Precision caveat: the `val * 65536.0` product is computed in `f32`, whose 24-bit
    /// mantissa holds it exactly only up to 2^24, so above `|val| ≈ 256` the low fractional
    /// bits of the result are no longer meaningful.
    #[inline]
    pub fn from_f32(val: f32) -> Self {
        Self((val * (Self::ONE_RAW as f32)) as i32)
    }

    /// Converts back to `f32` for rendering, logging and comparison against reference maths.
    /// One-way by intent: feeding the result back into the deterministic pipeline defeats
    /// the purpose of the type.
    ///
    /// Exact only while `|self| < 256` — above that the raw integer exceeds `f32`'s 24-bit
    /// mantissa and is rounded to the nearest representable float, so `to_f32` is not a
    /// perfect inverse of [`Self::from_f32`] at large magnitudes.
    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Self::ONE_RAW as f32
    }

    /// Drops the fractional part, truncating **toward zero** (`1.5 → 1`, `-1.5 → -1`), which
    /// makes it the exact inverse of [`Self::from_i32`] for in-range values.
    ///
    /// Deliberately an integer divide rather than an arithmetic `>>`: the shift rounds toward
    /// -∞ and would give `to_i32(-1.5) == -2` against `to_i32(1.5) == 1`, an asymmetry around
    /// zero that is a classic source of off-by-one grid and index bugs. Note this makes it
    /// inconsistent with the `Mul` operator, which *does* use `>>` and does round toward -∞.
    #[inline]
    pub fn to_i32(self) -> i32 {
        // Sıfıra doğru kes (truncation) — `from_i32` ile tutarlı ve geleneksel
        // tamsayı dönüşümüyle aynı. (Aritmetik `>>` negatiflerde -∞'a yuvarlardı:
        // to_i32(-1.5) = -2 ama to_i32(1.5) = 1 → asimetrik/tutarsızdı.)
        self.0 / Self::ONE_RAW
    }

    /// Magnitude, saturating. `i32::abs` overflows at `i32::MIN` (debug panic; in release it
    /// returns a still-*negative* value), and [`Self::MIN`] is a public constant any caller
    /// can reach, so this clamps: `MIN.abs() == MAX`, one raw unit short of the true
    /// magnitude. The two properties downstream code may rely on are that the result is
    /// never negative and the call never panics. Pinned by `fp32_abs_neg_saturate_at_min`.
    #[inline]
    pub fn abs(self) -> Self {
        // `i32::abs` is UB at i32::MIN (debug panic / wraps to a still-negative value in
        // release). `Fp32::MIN.abs()` is publicly reachable, so saturate to keep the
        // module's no-panic, always-non-negative contract (abs(MIN) == MAX).
        Self(self.0.saturating_abs())
    }

    /// Bit-exact Square Root (Sqrt) - Newton-Raphson or Integer Sqrt algorithm
    ///
    /// Restoring (digit-by-digit) integer square root. Every step is a shift, compare, add
    /// or subtract on `u32` — no floats anywhere — so the result is identical on every
    /// platform. (The loop count is *not* fixed: the `bit > x` pre-scan runs 0–15 times
    /// depending on the magnitude of the input. That affects timing, not the answer.)
    ///
    /// **Only 8 fractional bits of precision.** The integer root of a Q16.16 raw value `v`
    /// is `floor(sqrt(v)) = floor(sqrt(value) * 256)`, and the `<< 8` that re-normalises it
    /// shifts in zeros: the low byte of the result is always zero, the resolution is
    /// `1/256 ≈ 0.0039` instead of the type's usual `1.526e-5`, and the error is always
    /// downward (it never over-estimates). This is the dominant error term in
    /// [`FpVec3::length`] and [`FpVec3::normalize`], and the reason the tests here accept a
    /// 0.05 tolerance.
    ///
    /// Negative inputs return [`Self::ZERO`] rather than panicking or yielding a NaN
    /// analogue; callers who need to distinguish "root of zero" from "input was negative"
    /// must check the sign themselves.
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Self::ZERO;
        }
        let mut bit = 1u32 << 30; // Max pwr of 4
        let mut x = self.0 as u32;

        while bit > x {
            bit >>= 2;
        }

        let mut res = 0u32;
        while bit != 0 {
            if x >= res + bit {
                x -= res + bit;
                res = (res >> 1) + bit;
            } else {
                res >>= 1;
            }
            bit >>= 2;
        }
        // Q16.16 formatına uyarlamak için (res << 8) yapıyoruz.
        // Çünkü x aslında (val * 2^16), karekökü (val^0.5 * 2^8).
        Self((res << 8) as i32)
    }

    /// Bit-exact Sine via Taylor Series or Bhaskara Approximation
    ///
    /// Takes an angle in **radians** of any magnitude. It is first folded into `[0, 2π)`
    /// with a raw integer `%` — plus one `+= TWO_PI`, because Rust's remainder keeps the sign
    /// of the dividend and would otherwise leave negative angles negative — then mirrored
    /// into `[0, π]` with a recorded sign flip. Both steps are exact in raw units, so the
    /// reduction itself is not the error source; [`Self::TWO_PI`] being ~`1.3e-5` rad short
    /// of true 2π is, and that error grows with the number of periods folded away.
    ///
    /// The kernel is the Bhaskara I approximation `sin(x) ≈ 16x(π−x) / (5π² − 4x(π−x))`,
    /// not a Taylor series despite the Turkish line above: no factorials, one divide, and
    /// every intermediate stays comfortably inside the Q16.16 range (the largest is
    /// `5π² ≈ 49.3`, against a ceiling of 32768). Accuracy is roughly 2% relative.
    ///
    /// Note what the tests actually pin, which is less: they check landmark values against
    /// literals, plus the odd-symmetry, Pythagorean and periodicity identities, at 0.01–0.02
    /// absolute. Nothing measures this approximation against a true sine, so the 2% figure is
    /// the published property of Bhaskara I, not a tested bound. Adequate for a lockstep
    /// gameplay angle, nowhere near adequate for an orientation integrator.
    pub fn sin(self) -> Self {
        // Bhaskara I sine approximation formula
        // sin(x) ≈ (16 * x * (PI - x)) / (5 * PI^2 - 4 * x * (PI - x))
        // Not: Açının 0 ile PI arasında olması gerekir. Radian wrap yapılmalı.
        let mut x = self % Self::TWO_PI;
        if x < Self::ZERO {
            x += Self::TWO_PI;
        }
        let sign = if x > Self::PI {
            x -= Self::PI;
            -1
        } else {
            1
        };

        let pi_minus_x = Self::PI - x;
        let num = Self::from_i32(16) * x * pi_minus_x;
        let den = Self::from_i32(5) * Self::PI * Self::PI - Self::from_i32(4) * x * pi_minus_x;

        let res = num / den;
        if sign < 0 {
            -res
        } else {
            res
        }
    }

    /// Cosine of an angle in **radians**, via the identity `cos(x) = sin(x + π/2)`.
    ///
    /// Inherits every caveat of [`Self::sin`], plus two of its own: the shift uses
    /// [`Self::HALF_PI`], truncated to 102943 raw against a true π/2 of 102943.71, so the
    /// whole curve carries a ~`1.1e-5` rad phase bias; and the `+` saturates, so for
    /// arguments within π/2 of [`Self::MAX`] the addition clamps and the answer is
    /// meaningless.
    pub fn cos(self) -> Self {
        (self + Self::HALF_PI).sin()
    }
}

impl Add for Fp32 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        // Sessiz taşma yerine doygunluk (saturating): determinizmi korur ama
        // büyük koordinatlarda i32 sarmasından kaynaklanan çöp değer üretmez.
        Self(self.0.saturating_add(rhs.0))
    }
}

impl Sub for Fp32 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl Mul for Fp32 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let val = (self.0 as i64 * rhs.0 as i64) >> Fp32::SHIFT;
        // Q16.16 aralığını aşarsa `as i32` sessizce sarıp çöp üretiyordu; doygunlukla sınırla.
        Self(val.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }
}

impl Div for Fp32 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        // Sıfıra bölme: i64/0 release'de DAİMA panic eder. Determinizmi korumak ve
        // kullanıcının kolayca tetikleyebileceği panik yolunu kapatmak için sıfıra
        // bölmede panik yerine doygun (saturating) sonsuz/sıfır döndürürüz:
        //   pozitif / 0 -> MAX, negatif / 0 -> MIN, 0 / 0 -> 0.
        if rhs.0 == 0 {
            return match self.0.cmp(&0) {
                std::cmp::Ordering::Greater => Self(i32::MAX),
                std::cmp::Ordering::Less => Self(i32::MIN),
                std::cmp::Ordering::Equal => Self(0),
            };
        }
        let val = ((self.0 as i64) << Fp32::SHIFT) / (rhs.0 as i64);
        // Q16.16 aralığını aşarsa `as i32` sessizce sarıp çöp üretiyordu; doygunlukla sınırla.
        Self(val.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }
}

impl Neg for Fp32 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        // Saturating negation: `-i32::MIN` overflows (debug panic / wraps). Keeps the
        // module's no-panic saturating contract consistent with Add/Sub/Mul/Div.
        Self(self.0.saturating_neg())
    }
}

impl std::ops::Rem for Fp32 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self {
        // Sıfıra göre mod (% 0) release'de panic eder; determinizm için panik yerine
        // sıfır döndür (x % 0 -> 0).
        if rhs.0 == 0 {
            return Self(0);
        }
        Self(self.0 % rhs.0)
    }
}

impl AddAssign for Fp32 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        // `a += b` ile `a + b` aynı (saturating) sonucu vermeli; ham `+=` debug'da
        // panic / release'de wrapping üretip Add'den sapıyordu.
        *self = *self + rhs;
    }
}
impl SubAssign for Fp32 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        // `a -= b` ile `a - b` aynı (saturating) sonucu vermeli.
        *self = *self - rhs;
    }
}
impl MulAssign for Fp32 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}
impl DivAssign for Fp32 {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}

// --- Vektör yapıları (Sabit noktalı 3D Fizik için) ---

/// A 3-component vector of [`Fp32`] — the fixed-point counterpart of `Vec3`, part of the
/// same experimental cross-platform-determinism prototype. **The engine does not use it**:
/// the physics pipeline is `glam::Vec3`/`f32` end to end (see the module docs).
///
/// Carries no coordinate frame of its own. If used for positions the crate convention
/// applies — right-handed, Y-up, -Z forward (see the `gizmo_math` crate docs) — and the
/// natural unit is metres. `Eq`/`Hash` are derived and exact, which is legitimate here
/// because the components have no NaN and no signed zero: two vectors compare equal iff
/// they are bit-identical, making the type usable as a map key or checksum input where
/// `Vec3` is not.
///
/// The usable magnitude is far smaller than the component range suggests. [`Self::dot`] sums
/// squares in Q16.16, which saturates around 32768, so [`Self::length_squared`] and
/// everything built on it are only meaningful while `|v| < ~181`. The operator set is
/// deliberately minimal — add, subtract, scale by a scalar, divide by a scalar — with no
/// `Neg`, no compound assignment and no component indexing.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FpVec3 {
    /// Rightward component under the crate's right-handed convention. Clamps at
    /// [`Fp32::MIN`]/[`Fp32::MAX`] *independently* of its siblings, so an overflowing vector
    /// does not merely lose magnitude — a single saturated component silently rotates the
    /// direction.
    pub x: Fp32,
    /// Up component: the crate is Y-up, so gravity would act along `-y`. No method in this
    /// file treats any axis specially — [`FpVec3::dot`] and [`FpVec3::cross`] are the plain
    /// formulas — so this is convention for callers to honour, not behaviour.
    pub y: Fp32,
    /// Depth component. The crate convention is -Z forward, so a forward-facing direction
    /// has negative `z`. That the basis really is right-handed is pinned by
    /// `fpvec3_cross_basis_vectors`: `x.cross(y) == +z`.
    pub z: Fp32,
}

impl FpVec3 {
    /// The origin and additive identity — and the sentinel [`Self::normalize`] returns when
    /// there is no direction to report. Since `normalize` has no other error channel, a
    /// `ZERO` result from it is indistinguishable from normalizing a genuinely zero vector;
    /// test the input yourself when that distinction matters.
    pub const ZERO: Self = Self {
        x: Fp32::ZERO,
        y: Fp32::ZERO,
        z: Fp32::ZERO,
    };

    /// Assembles a vector from three already-fixed-point scalars. There is intentionally no
    /// `f32` constructor: conversion is [`Fp32::from_f32`]'s job and belongs once, at the
    /// edge of the deterministic region, not sprinkled through vector construction.
    #[inline]
    pub fn new(x: Fp32, y: Fp32, z: Fp32) -> Self {
        Self { x, y, z }
    }

    /// Dot product `x·x + y·y + z·z`, evaluated left to right with every product and sum
    /// saturating in Q16.16. The unit is the product of the operands' units (m² for two
    /// positions).
    ///
    /// Saturation is the practical ceiling on this whole type: the sum tops out near 32768,
    /// so the result is trustworthy only while each `|component| < ~104` (equivalently
    /// `|v| < ~181` for [`Self::length_squared`]). Products also round toward -∞, so a pair
    /// that is perpendicular in exact arithmetic will generally *not* dot to exactly
    /// [`Fp32::ZERO`] — do not use `== ZERO` as a perpendicularity test outside the axis-
    /// aligned cases the tests cover.
    pub fn dot(self, rhs: Self) -> Fp32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Right-handed cross product: `x × y == z`, `y × z == x`, `z × x == y`, pinned by
    /// `fpvec3_cross_basis_vectors`, so the handedness matches the rest of `gizmo-math`.
    /// Anti-commutativity `a × b == -(b × a)` holds exactly (barring saturation), because
    /// each component is the same two products subtracted in the opposite order.
    ///
    /// Six [`Fp32`] multiplies, each saturating and each rounding toward -∞: the result is
    /// therefore only approximately perpendicular to its inputs, is not normalized, and — as
    /// its magnitude is `|a||b|sin θ` — saturates at much smaller inputs than the components
    /// alone would.
    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    /// Squared magnitude (`self.dot(self)`), in the square of the components' unit. Prefer
    /// it to [`Self::length`] for comparisons and thresholds: it skips the square root, and
    /// the square root is where nearly all of this type's precision is spent.
    ///
    /// Two silent failure modes at the ends of the range. It **saturates** once `|v| > ~181`.
    /// And it **underflows to zero** when every component is below `1/256` — each square is
    /// computed as `(raw * raw) >> 16` and truncates to 0 raw below that point — which is
    /// exactly the cutoff at which [`Self::normalize`] gives up.
    pub fn length_squared(self) -> Fp32 {
        self.dot(self)
    }

    /// Magnitude, `sqrt(length_squared())`, in the components' own unit.
    ///
    /// Resolution is `1/256 ≈ 0.0039`, not the type's `1.526e-5`, because [`Fp32::sqrt`]
    /// keeps only 8 fractional bits — and it always floors, so this systematically
    /// under-reports. Combined with `length_squared`'s saturation the usable domain is
    /// `|v| < ~181`; past that the answer pins near 181 no matter how long the vector is.
    pub fn length(self) -> Fp32 {
        self.length_squared().sqrt()
    }

    /// Unit-length version of the vector, or [`Self::ZERO`] when there is no stable direction
    /// to return — never a NaN analogue.
    ///
    /// "No stable direction" means `length()` evaluated to zero, which, because
    /// [`Self::length_squared`] truncates, covers not only the true zero vector but any
    /// vector whose components are all below `1/256`. Note also that the result is only
    /// approximately unit length, and for short vectors it is badly off: `length()` floors
    /// onto the 1/256 grid, so dividing by it overshoots — `(0.0078, 0, 0)` normalizes to
    /// a vector of length ~2.0. Never assume `n.length() == Fp32::ONE`.
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len.0 > 0 {
            self / len
        } else {
            Self::ZERO
        }
    }
}

impl Add for FpVec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for FpVec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<Fp32> for FpVec3 {
    type Output = Self;
    fn mul(self, rhs: Fp32) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Div<Fp32> for FpVec3 {
    type Output = Self;
    fn div(self, rhs: Fp32) -> Self {
        Self::new(self.x / rhs, self.y / rhs, self.z / rhs)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-point tolerance: in Q16.16, 1 LSB = 1/65536 ≈ 0.0000153
    /// We use a wider tolerance for the trigonometric approximations.
    const FP_EPS: f32 = 0.01; // ~%1 tolerans (Bhaskara max error ~%1.9)

    fn fp_approx(a: Fp32, expected: f32) -> bool {
        (a.to_f32() - expected).abs() < FP_EPS
    }

    // =======================================================================
    // Fp32 — Dönüşümler (Conversions)
    // =======================================================================

    #[test]
    fn fp32_from_i32_roundtrip() {
        for val in [-100, -1, 0, 1, 42, 1000] {
            let fp = Fp32::from_i32(val);
            assert_eq!(fp.to_i32(), val, "from_i32({val}).to_i32() roundtrip failed");
        }
    }

    #[test]
    fn fp32_from_i32_saturates_out_of_range() {
        // Q16.16 tops out at ±32767.99…; out-of-range ints must SATURATE (like the
        // other ops), not wrap. Old `val << SHIFT` wrapped `40000` to a negative
        // value — the classic bug this guards against.
        assert!(Fp32::from_i32(40000).to_f32() > 32000.0, "must not wrap to negative");
        assert!(Fp32::from_i32(100000).to_f32() > 32000.0);
        assert!(Fp32::from_i32(-100000).to_f32() < -32000.0);
        // In-range values still map exactly (no spurious saturation).
        assert_eq!(Fp32::from_i32(32767).to_i32(), 32767);
        // Consistent sign with the float constructor for the same magnitude.
        assert!(Fp32::from_i32(40000).to_f32().signum() == Fp32::from_f32(40000.0).to_f32().signum());
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 burada rastgele bir yuvarlak test girdisi, PI değil
    fn fp32_from_f32_roundtrip() {
        for val in [0.0f32, 1.0, -1.0, 0.5, -0.25, 3.14, 100.75] {
            let fp = Fp32::from_f32(val);
            let back = fp.to_f32();
            assert!(
                (back - val).abs() < 0.001,
                "from_f32({val}).to_f32() = {back}, expected ~{val}"
            );
        }
    }

    #[test]
    fn fp32_from_raw() {
        let fp = Fp32::from_raw(65536);
        assert_eq!(fp, Fp32::ONE);
        assert_eq!(fp.to_f32(), 1.0);
    }

    // =======================================================================
    // Fp32 — Sabitler (Constants Consistency)
    // =======================================================================

    #[test]
    fn fp32_constants_consistency() {
        // TWO_PI = 2 * PI
        assert_eq!(Fp32::TWO_PI.0, Fp32::PI.0 * 2, "TWO_PI != 2 * PI");

        // HALF_PI = PI / 2 (integer division)
        assert_eq!(Fp32::HALF_PI.0, Fp32::PI.0 / 2, "HALF_PI != PI / 2");

        // ONE = from_i32(1)
        assert_eq!(Fp32::ONE, Fp32::from_i32(1));

        // MINUS_ONE = -ONE
        assert_eq!(Fp32::MINUS_ONE, -Fp32::ONE);

        // ZERO
        assert_eq!(Fp32::ZERO.0, 0);
    }

    #[test]
    fn fp32_pi_accuracy() {
        let pi_f32 = Fp32::PI.to_f32();
        assert!(
            (pi_f32 - std::f32::consts::PI).abs() < 0.0001,
            "PI as f32 = {pi_f32}, expected ~3.14159"
        );
    }

    // =======================================================================
    // Fp32 — Aritmetik Operatörler
    // =======================================================================

    #[test]
    fn fp32_add() {
        let a = Fp32::from_f32(1.5);
        let b = Fp32::from_f32(2.25);
        assert!(fp_approx(a + b, 3.75));
    }

    #[test]
    fn fp32_sub() {
        let a = Fp32::from_f32(5.0);
        let b = Fp32::from_f32(3.5);
        assert!(fp_approx(a - b, 1.5));
    }

    #[test]
    fn fp32_mul() {
        let a = Fp32::from_f32(3.0);
        let b = Fp32::from_f32(4.0);
        assert!(fp_approx(a * b, 12.0));

        // Çarpım ile ondalık
        let c = Fp32::from_f32(2.5);
        let d = Fp32::from_f32(1.5);
        assert!(fp_approx(c * d, 3.75));
    }

    #[test]
    fn fp32_div() {
        let a = Fp32::from_f32(10.0);
        let b = Fp32::from_f32(4.0);
        assert!(fp_approx(a / b, 2.5));
    }

    #[test]
    #[allow(clippy::approx_constant)] // 3.14 burada rastgele bir yuvarlak test girdisi, PI değil
    fn fp32_neg() {
        let a = Fp32::from_f32(3.14);
        assert!(fp_approx(-a, -3.14));
        assert!(fp_approx(-(-a), 3.14));
    }

    #[test]
    fn fp32_rem() {
        let a = Fp32::from_f32(7.0);
        let b = Fp32::from_f32(3.0);
        assert!(fp_approx(a % b, 1.0));
    }

    #[test]
    fn fp32_abs() {
        assert!(fp_approx(Fp32::from_f32(-5.0).abs(), 5.0));
        assert!(fp_approx(Fp32::from_f32(5.0).abs(), 5.0));
        assert!(fp_approx(Fp32::ZERO.abs(), 0.0));
    }

    #[test]
    fn fp32_abs_neg_saturate_at_min() {
        // Regression: `i32::MIN.abs()` / `-i32::MIN` overflow (debug panic, or a still-negative
        // value in release). Fp32::MIN is a public const, so both must saturate to keep the
        // module's no-panic, always-non-negative contract.
        assert_eq!(Fp32::MIN.abs(), Fp32::MAX);
        assert!(Fp32::MIN.abs().0 >= 0, "abs() must never be negative");
        assert_eq!(-Fp32::MIN, Fp32::MAX);
    }

    // =======================================================================
    // Fp32 — Compound Assign Operatörleri
    // =======================================================================

    #[test]
    fn fp32_add_assign() {
        let mut a = Fp32::from_f32(1.0);
        a += Fp32::from_f32(2.0);
        assert!(fp_approx(a, 3.0));
    }

    #[test]
    fn fp32_sub_assign() {
        let mut a = Fp32::from_f32(5.0);
        a -= Fp32::from_f32(3.0);
        assert!(fp_approx(a, 2.0));
    }

    #[test]
    fn fp32_mul_assign() {
        let mut a = Fp32::from_f32(3.0);
        a *= Fp32::from_f32(4.0);
        assert!(fp_approx(a, 12.0));
    }

    #[test]
    fn fp32_div_assign() {
        let mut a = Fp32::from_f32(10.0);
        a /= Fp32::from_f32(2.0);
        assert!(fp_approx(a, 5.0));
    }

    // =======================================================================
    // Fp32 — Sqrt
    // =======================================================================

    #[test]
    fn fp32_sqrt_perfect_squares() {
        for val in [1.0f32, 4.0, 9.0, 16.0, 25.0, 100.0] {
            let result = Fp32::from_f32(val).sqrt().to_f32();
            let expected = val.sqrt();
            assert!(
                (result - expected).abs() < 0.05,
                "sqrt({val}) = {result}, expected {expected}"
            );
        }
    }

    #[test]
    fn fp32_sqrt_fractional() {
        let result = Fp32::from_f32(2.0).sqrt().to_f32();
        assert!(
            (result - std::f32::consts::SQRT_2).abs() < 0.05,
            "sqrt(2) = {result}, expected ~1.414"
        );
    }

    #[test]
    fn fp32_sqrt_zero() {
        assert_eq!(Fp32::ZERO.sqrt(), Fp32::ZERO);
    }

    #[test]
    fn fp32_sqrt_negative_returns_zero() {
        assert_eq!(Fp32::from_f32(-4.0).sqrt(), Fp32::ZERO);
        assert_eq!(Fp32::MINUS_ONE.sqrt(), Fp32::ZERO);
    }

    // =======================================================================
    // Fp32 — Trigonometri (sin / cos)
    // =======================================================================

    #[test]
    fn fp32_sin_known_values() {
        // sin(0) = 0
        assert!(fp_approx(Fp32::ZERO.sin(), 0.0));

        // sin(PI) = 0
        assert!(
            Fp32::PI.sin().to_f32().abs() < 0.02,
            "sin(PI) = {}, expected ~0",
            Fp32::PI.sin().to_f32()
        );

        // sin(PI/2) = 1
        assert!(
            fp_approx(Fp32::HALF_PI.sin(), 1.0),
            "sin(PI/2) = {}, expected ~1.0",
            Fp32::HALF_PI.sin().to_f32()
        );

        // sin(3*PI/2) = -1
        let three_half_pi = Fp32::PI + Fp32::HALF_PI;
        assert!(
            fp_approx(three_half_pi.sin(), -1.0),
            "sin(3PI/2) = {}, expected ~-1.0",
            three_half_pi.sin().to_f32()
        );
    }

    #[test]
    fn fp32_cos_known_values() {
        // cos(0) = 1
        assert!(
            fp_approx(Fp32::ZERO.cos(), 1.0),
            "cos(0) = {}, expected ~1.0",
            Fp32::ZERO.cos().to_f32()
        );

        // cos(PI) = -1
        assert!(
            fp_approx(Fp32::PI.cos(), -1.0),
            "cos(PI) = {}, expected ~-1.0",
            Fp32::PI.cos().to_f32()
        );

        // cos(PI/2) ≈ 0
        assert!(
            Fp32::HALF_PI.cos().to_f32().abs() < 0.02,
            "cos(PI/2) = {}, expected ~0",
            Fp32::HALF_PI.cos().to_f32()
        );
    }

    #[test]
    fn fp32_sin_negative_angle() {
        // sin(-x) = -sin(x)
        let x = Fp32::from_f32(1.0);
        let sin_x = x.sin().to_f32();
        let sin_neg_x = (-x).sin().to_f32();
        assert!(
            (sin_x + sin_neg_x).abs() < 0.02,
            "sin(x) + sin(-x) = {}, expected ~0",
            sin_x + sin_neg_x
        );
    }

    #[test]
    fn fp32_sin_cos_pythagorean_identity() {
        // sin²(x) + cos²(x) = 1 for various angles
        for angle_f32 in [0.5f32, 1.0, 1.5, 2.0, 2.5, 3.0, 4.5, 5.5] {
            let x = Fp32::from_f32(angle_f32);
            let s = x.sin().to_f32();
            let c = x.cos().to_f32();
            let identity = s * s + c * c;
            assert!(
                (identity - 1.0).abs() < 0.05,
                "sin²({angle_f32}) + cos²({angle_f32}) = {identity}, expected ~1.0"
            );
        }
    }

    #[test]
    fn fp32_sin_wrap_around() {
        // sin(x) = sin(x + 2*PI) — periodicity
        let x = Fp32::from_f32(1.23);
        let s1 = x.sin().to_f32();
        let s2 = (x + Fp32::TWO_PI).sin().to_f32();
        assert!(
            (s1 - s2).abs() < 0.02,
            "sin(x) = {s1}, sin(x + 2PI) = {s2}, expected equal"
        );
    }

    // =======================================================================
    // Fp32 — Ordering / Comparison
    // =======================================================================

    #[test]
    fn fp32_ordering() {
        assert!(Fp32::ZERO < Fp32::ONE);
        assert!(Fp32::MINUS_ONE < Fp32::ZERO);
        assert!(Fp32::ONE > Fp32::MINUS_ONE);
        assert_eq!(Fp32::ONE, Fp32::ONE);
    }

    // =======================================================================
    // FpVec3 — Temel Operasyonlar
    // =======================================================================

    #[test]
    fn fpvec3_add_sub() {
        let a = FpVec3::new(Fp32::from_f32(1.0), Fp32::from_f32(2.0), Fp32::from_f32(3.0));
        let b = FpVec3::new(Fp32::from_f32(4.0), Fp32::from_f32(5.0), Fp32::from_f32(6.0));

        let sum = a + b;
        assert!(fp_approx(sum.x, 5.0));
        assert!(fp_approx(sum.y, 7.0));
        assert!(fp_approx(sum.z, 9.0));

        let diff = b - a;
        assert!(fp_approx(diff.x, 3.0));
        assert!(fp_approx(diff.y, 3.0));
        assert!(fp_approx(diff.z, 3.0));
    }

    #[test]
    fn fpvec3_scalar_mul_div() {
        let v = FpVec3::new(Fp32::from_f32(2.0), Fp32::from_f32(4.0), Fp32::from_f32(6.0));
        let scaled = v * Fp32::from_f32(0.5);
        assert!(fp_approx(scaled.x, 1.0));
        assert!(fp_approx(scaled.y, 2.0));
        assert!(fp_approx(scaled.z, 3.0));

        let divided = v / Fp32::from_f32(2.0);
        assert!(fp_approx(divided.x, 1.0));
        assert!(fp_approx(divided.y, 2.0));
        assert!(fp_approx(divided.z, 3.0));
    }

    // =======================================================================
    // FpVec3 — Dot / Cross / Length
    // =======================================================================

    #[test]
    fn fpvec3_dot() {
        let a = FpVec3::new(Fp32::from_f32(1.0), Fp32::from_f32(2.0), Fp32::from_f32(3.0));
        let b = FpVec3::new(Fp32::from_f32(4.0), Fp32::from_f32(5.0), Fp32::from_f32(6.0));
        // 1*4 + 2*5 + 3*6 = 32
        assert!(fp_approx(a.dot(b), 32.0));
    }

    #[test]
    fn fpvec3_dot_perpendicular_is_zero() {
        let x = FpVec3::new(Fp32::ONE, Fp32::ZERO, Fp32::ZERO);
        let y = FpVec3::new(Fp32::ZERO, Fp32::ONE, Fp32::ZERO);
        assert_eq!(x.dot(y), Fp32::ZERO);
    }

    #[test]
    fn fpvec3_cross_basis_vectors() {
        let x = FpVec3::new(Fp32::ONE, Fp32::ZERO, Fp32::ZERO);
        let y = FpVec3::new(Fp32::ZERO, Fp32::ONE, Fp32::ZERO);
        let z = FpVec3::new(Fp32::ZERO, Fp32::ZERO, Fp32::ONE);

        // X × Y = Z
        let xy = x.cross(y);
        assert_eq!(xy.x, Fp32::ZERO);
        assert_eq!(xy.y, Fp32::ZERO);
        assert_eq!(xy.z, Fp32::ONE);

        // Y × Z = X
        let yz = y.cross(z);
        assert_eq!(yz.x, Fp32::ONE);
        assert_eq!(yz.y, Fp32::ZERO);
        assert_eq!(yz.z, Fp32::ZERO);

        // Z × X = Y
        let zx = z.cross(x);
        assert_eq!(zx.x, Fp32::ZERO);
        assert_eq!(zx.y, Fp32::ONE);
        assert_eq!(zx.z, Fp32::ZERO);
    }

    #[test]
    fn fpvec3_cross_anti_commutative() {
        // A × B = -(B × A)
        let a = FpVec3::new(Fp32::from_f32(1.0), Fp32::from_f32(2.0), Fp32::from_f32(3.0));
        let b = FpVec3::new(Fp32::from_f32(4.0), Fp32::from_f32(5.0), Fp32::from_f32(6.0));
        let ab = a.cross(b);
        let ba = b.cross(a);
        assert_eq!(ab.x, Fp32::from_raw(-ba.x.0));
        assert_eq!(ab.y, Fp32::from_raw(-ba.y.0));
        assert_eq!(ab.z, Fp32::from_raw(-ba.z.0));
    }

    #[test]
    fn fpvec3_length_squared() {
        // (3, 4, 0) → length² = 25
        let v = FpVec3::new(Fp32::from_f32(3.0), Fp32::from_f32(4.0), Fp32::ZERO);
        assert!(fp_approx(v.length_squared(), 25.0));
    }

    #[test]
    fn fpvec3_length() {
        // (3, 4, 0) → length = 5
        let v = FpVec3::new(Fp32::from_f32(3.0), Fp32::from_f32(4.0), Fp32::ZERO);
        let len = v.length().to_f32();
        assert!(
            (len - 5.0).abs() < 0.1,
            "length of (3,4,0) = {len}, expected ~5.0"
        );
    }

    #[test]
    fn fpvec3_normalize() {
        let v = FpVec3::new(Fp32::from_f32(3.0), Fp32::from_f32(4.0), Fp32::ZERO);
        let n = v.normalize();
        let len = n.length().to_f32();
        assert!(
            (len - 1.0).abs() < 0.1,
            "normalized length = {len}, expected ~1.0"
        );
    }

    #[test]
    fn fpvec3_normalize_zero_vector() {
        let v = FpVec3::ZERO;
        let n = v.normalize();
        assert_eq!(n, FpVec3::ZERO, "normalizing zero vector should return zero");
    }

    #[test]
    fn fpvec3_zero_constant() {
        assert_eq!(FpVec3::ZERO.x, Fp32::ZERO);
        assert_eq!(FpVec3::ZERO.y, Fp32::ZERO);
        assert_eq!(FpVec3::ZERO.z, Fp32::ZERO);
    }
}
