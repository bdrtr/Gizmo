use std::ops::{Add, Mul, Sub};
use crate::{Mat3, Vec3};

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
        let outer = |a: Vec3, b: Vec3| -> Mat3 {
            Mat3::from_cols(a * b.x, a * b.y, a * b.z)
        };
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

    pub fn add(self, other: Self) -> Self {
        Self {
            m00: self.m00 + other.m00,
            m01: self.m01 + other.m01,
            m10: self.m10 + other.m10,
            m11: self.m11 + other.m11,
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            m00: self.m00 - other.m00,
            m01: self.m01 - other.m01,
            m10: self.m10 - other.m10,
            m11: self.m11 - other.m11,
        }
    }

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

/// Spatial Inertia Tensor (6x6 Matris Karşılığı)
/// Bir RigidBody modelinin eylemsizlik profilidir.
#[derive(Clone, Copy, Debug)]
pub struct SpatialInertia {
    pub rot: Mat3,     // Angular Inertia (I) 
    pub mass: f32,     // Linear Mass (m)
    pub com: Vec3,     // Center of Mass (c) - Origin'e göre offset
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
        if self.com != Vec3::ZERO {
            force_w += self.com.cross(com_cross_w) * self.mass;
        }
        
        let force_v = v.v * self.mass - com_cross_w * self.mass;
        
        SpatialVector::new(force_w, force_v)
    }

    pub fn add(self, other: Self) -> Self {
        let total_mass = self.mass + other.mass;
        if total_mass == 0.0 {
            return Self::from_mass_inertia(0.0, Mat3::ZERO);
        }
        let total_com = (self.com * self.mass + other.com * other.mass) * (1.0 / total_mass);
        
        // Approximation: rot1 + rot2 (Gerçek rigid body birleşimi için parallel axis eklenmeli)
        Self {
            mass: total_mass,
            com: total_com,
            rot: self.rot + other.rot,
        }
    }

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
