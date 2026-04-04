use std::ops::Mul;
use crate::vec3::Vec3;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    #[inline]
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Self {
        let half_angle = angle * 0.5;
        let s = half_angle.sin();
        Self {
            x: axis.x * s,
            y: axis.y * s,
            z: axis.z * s,
            w: half_angle.cos(),
        }
    }

    #[inline]
    pub fn normalize(self) -> Self {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w).sqrt();
        if len > f32::EPSILON {
            Self {
                x: self.x / len,
                y: self.y / len,
                z: self.z / len,
                w: self.w / len,
            }
        } else {
            Self::IDENTITY
        }
    }

    // Gerçekçi simülatör fizikleri için Quat * Vec3 işlemleri çok kritik olacak
    #[inline]
    pub fn mul_vec3(self, v: Vec3) -> Vec3 {
        let q_vec = Vec3::new(self.x, self.y, self.z);
        let uv = q_vec.cross(v);
        let uuv = q_vec.cross(uv);
        v + (uv * self.w + uuv) * 2.0
    }

    /// Quaternion'u 4x4 rotasyon matrisine dönüştürür.
    #[inline]
    pub fn to_mat4(self) -> crate::mat4::Mat4 {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        let (x2, y2, z2) = (x + x, y + y, z + z);
        let (xx, xy, xz) = (x * x2, x * y2, x * z2);
        let (yy, yz, zz) = (y * y2, y * z2, z * z2);
        let (wx, wy, wz) = (w * x2, w * y2, w * z2);

        use crate::vec4::Vec4;
        crate::mat4::Mat4 {
            cols: [
                Vec4::new(1.0 - (yy + zz), xy + wz, xz - wy, 0.0),
                Vec4::new(xy - wz, 1.0 - (xx + zz), yz + wx, 0.0),
                Vec4::new(xz + wy, yz - wx, 1.0 - (xx + yy), 0.0),
                Vec4::new(0.0, 0.0, 0.0, 1.0),
            ],
        }
    }
}

// Quaternion çarpımı, rotasyonları birleştirir (Quat2 * Quat1 önce quat1 rotasyonu)
impl Mul for Quat {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        }
    }
}
