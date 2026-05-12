//! Fixed-Point (Sabit Noktalı) Matematik Kütüphanesi
//!
//! Kayan noktalı (Floating-Point / f32, f64) sayılar farklı CPU mimarilerinde (x86, ARM)
//! veya farklı derleyici optimizasyonlarında farklı yuvarlama hataları verebilir.
//! Bu kütüphane, Multiplayer, eSpor, Lock-step RTS oyunlarında %100 Bit-Exact
//! Cross-Platform Determinism sağlamak için Q16.16 ve Q32.32 sabit noktalı yapıları içerir.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

/// Q16.16 Sabit Noktalı Sayı (32-bit)
///
/// 16 bit tam sayı kısmı, 16 bit ondalık kısmı temsil eder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Fp32(pub i32);

impl Fp32 {
    pub const SHIFT: usize = 16;
    pub const ONE_RAW: i32 = 1 << Self::SHIFT;

    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(Self::ONE_RAW);
    pub const MINUS_ONE: Self = Self(-Self::ONE_RAW);
    pub const MAX: Self = Self(i32::MAX);
    pub const MIN: Self = Self(i32::MIN);

    // Pi yaklaşımları
    pub const PI: Self = Self(205887); // 3.1415926 * 65536
    pub const TWO_PI: Self = Self(411774); // 2 * PI  (205887 * 2)
    pub const HALF_PI: Self = Self(102943); // PI / 2  (205887 / 2)

    #[inline]
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    #[inline]
    pub fn from_i32(val: i32) -> Self {
        Self(val << Self::SHIFT)
    }

    #[inline]
    pub fn from_f32(val: f32) -> Self {
        Self((val * (Self::ONE_RAW as f32)) as i32)
    }

    #[inline]
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Self::ONE_RAW as f32
    }

    #[inline]
    pub fn to_i32(self) -> i32 {
        self.0 >> Self::SHIFT
    }

    #[inline]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// Bit-exact Karekök (Sqrt) - Newton-Raphson veya Integer Sqrt algoritması
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

    /// Taylor Serisi veya Bhaskara Approximation ile Bit-exact Sinüs
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

    pub fn cos(self) -> Self {
        (self + Self::HALF_PI).sin()
    }
}

impl Add for Fp32 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Fp32 {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl Mul for Fp32 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let val = (self.0 as i64 * rhs.0 as i64) >> Fp32::SHIFT;
        Self(val as i32)
    }
}

impl Div for Fp32 {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self {
        debug_assert!(rhs.0 != 0, "Fp32: division by zero");
        let val = ((self.0 as i64) << Fp32::SHIFT) / (rhs.0 as i64);
        Self(val as i32)
    }
}

impl Neg for Fp32 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl std::ops::Rem for Fp32 {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self {
        Self(self.0 % rhs.0)
    }
}

impl AddAssign for Fp32 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}
impl SubAssign for Fp32 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FpVec3 {
    pub x: Fp32,
    pub y: Fp32,
    pub z: Fp32,
}

impl FpVec3 {
    pub const ZERO: Self = Self {
        x: Fp32::ZERO,
        y: Fp32::ZERO,
        z: Fp32::ZERO,
    };

    #[inline]
    pub fn new(x: Fp32, y: Fp32, z: Fp32) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, rhs: Self) -> Fp32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self {
            x: self.y * rhs.z - self.z * rhs.y,
            y: self.z * rhs.x - self.x * rhs.z,
            z: self.x * rhs.y - self.y * rhs.x,
        }
    }

    pub fn length_squared(self) -> Fp32 {
        self.dot(self)
    }

    pub fn length(self) -> Fp32 {
        self.length_squared().sqrt()
    }

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

    /// Fixed-point tolerans: Q16.16'da 1 LSB = 1/65536 ≈ 0.0000153
    /// Trigonometrik yaklaşımlar için daha geniş tolerans kullanıyoruz.
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
