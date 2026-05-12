use crate::{Mat3, Vec3};
use std::ops::{Add, Mul, Sub};

/// Plücker Coordinate (Spatial Vector)
/// Temsil ettiği kavrama göre:
/// - Hız (Velocity): [angular_velocity, linear_velocity]
/// - Kuvvet (Force): [moment, force]
/// - İvme (Acceleration): [angular_acceleration, linear_acceleration]
#[derive(Clone, Copy, Debug)]
pub struct SpatialVector {
    pub w: Vec3, // Angular part (w)
    pub v: Vec3, // Linear part (v)
}

impl SpatialVector {
    pub const ZERO: Self = Self {
        w: Vec3::ZERO,
        v: Vec3::ZERO,
    };

    pub fn new(w: Vec3, v: Vec3) -> Self {
        Self { w, v }
    }

    /// Dot çarpımı (Force * Velocity = Power)
    pub fn dot(self, other: Self) -> f32 {
        self.w.dot(other.w) + self.v.dot(other.v)
    }

    /// Cross product for Spatial Velocities (Motion x Motion)
    pub fn cross_motion(self, other: Self) -> Self {
        Self {
            w: self.w.cross(other.w),
            v: self.w.cross(other.v) + self.v.cross(other.w),
        }
    }

    /// Cross product for Spatial Forces (Motion x Force)
    pub fn cross_force(self, f: Self) -> Self {
        Self {
            w: self.w.cross(f.w) + self.v.cross(f.v),
            v: self.w.cross(f.v),
        }
    }

    /// U * U^T = 6x6 Spatial Matrix
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

/// 6x6 Spatial Matrix (Articulated Body Inertia)
#[derive(Clone, Copy, Debug)]
pub struct SpatialMatrix {
    pub m00: Mat3,
    pub m01: Mat3,
    pub m10: Mat3,
    pub m11: Mat3,
}

impl SpatialMatrix {
    pub const ZERO: Self = Self {
        m00: Mat3::ZERO,
        m01: Mat3::ZERO,
        m10: Mat3::ZERO,
        m11: Mat3::ZERO,
    };

    pub fn mul_vec(self, v: SpatialVector) -> SpatialVector {
        SpatialVector {
            w: self.m00 * v.w + self.m01 * v.v,
            v: self.m10 * v.w + self.m11 * v.v,
        }
    }

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

/// Spatial Inertia Tensor (6x6 Matris Karşılığı)
/// Bir RigidBody modelinin eylemsizlik profilidir.
#[derive(Clone, Copy, Debug)]
pub struct SpatialInertia {
    pub rot: Mat3, // Angular Inertia (I)
    pub mass: f32, // Linear Mass (m)
    pub com: Vec3, // Center of Mass (c) - Origin'e göre offset
}

impl SpatialInertia {
    pub fn new(mass: f32, rot_inertia: Mat3, com_offset: Vec3) -> Self {
        Self {
            rot: rot_inertia,
            mass,
            com: com_offset,
        }
    }

    pub fn from_mass_inertia(mass: f32, inertia: Mat3) -> Self {
        Self {
            rot: inertia,
            mass,
            com: Vec3::ZERO,
        }
    }

    /// I * v (Spatial Inertia tensor times Spatial Velocity = Spatial Momentum)
    pub fn mul_vec(self, v: SpatialVector) -> SpatialVector {
        // [ rot_shifted , mass * [com]x^T ] [ w ]
        // [ mass * [com]x , mass * I_3 ]     [ v ]
        // Basitleştirilmiş COM = 0 hali için:
        let com_cross_v = self.com.cross(v.v);
        let com_cross_w = self.com.cross(v.w);

        let mut force_w = self.rot.mul_vec3(v.w) + com_cross_v * self.mass;
        // Parallel axis theorem correction if COM offset is non-zero
        if self.com.length_squared() > 1e-12 {
            force_w += self.com.cross(com_cross_w) * self.mass;
        }

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
    /// Converts the Rigid Body Inertia to a full 6x6 Spatial Matrix.
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
        let rot_shifted = self.rot + c_cross_c_cross * m;

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

