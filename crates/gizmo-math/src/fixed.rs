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
    pub const TWO_PI: Self = Self(411775); // 2 * PI
    pub const HALF_PI: Self = Self(102944); // PI / 2

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
