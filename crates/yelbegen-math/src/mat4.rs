use std::ops::Mul;
use crate::{vec3::Vec3, vec4::Vec4};

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mat4 {
    pub cols: [Vec4; 4],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        cols: [
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, 1.0),
        ],
    };

    #[inline]
    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let mut mat = Self::IDENTITY;
        mat.cols[0].x = 2.0 / (right - left);
        mat.cols[1].y = 2.0 / (top - bottom);
        mat.cols[2].z = -2.0 / (far - near);
        mat.cols[3].x = -(right + left) / (right - left);
        mat.cols[3].y = -(top + bottom) / (top - bottom);
        mat.cols[3].z = -(far + near) / (far - near);
        mat
    }

    #[inline]
    pub fn perspective(fov_y_radians: f32, aspect_ratio: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y_radians / 2.0).tan();
        let mut mat = Self::IDENTITY;
        mat.cols[0].x = f / aspect_ratio;
        mat.cols[1].y = f;
        // wgpu/Vulkan konvansiyonu: z aralığı [0, 1] (OpenGL'deki [-1, 1] değil!)
        mat.cols[2].z = far / (near - far);
        mat.cols[2].w = -1.0;
        mat.cols[3].z = (far * near) / (near - far);
        mat.cols[3].w = 0.0;
        mat
    }

    #[inline]
    pub fn translation(offset: Vec3) -> Self {
        let mut mat = Self::IDENTITY;
        mat.cols[3].x = offset.x; // tx
        mat.cols[3].y = offset.y; // ty
        mat.cols[3].z = offset.z; // tz
        mat
    }

    #[inline]
    pub fn rotation_y(angle_radians: f32) -> Self {
        let (s, c) = (angle_radians.sin(), angle_radians.cos());
        let mut mat = Self::IDENTITY;
        mat.cols[0].x = c;
        mat.cols[0].z = -s;
        mat.cols[2].x = s;
        mat.cols[2].z = c;
        mat
    }

    #[inline]
    pub fn scale(s: Vec3) -> Self {
        let mut mat = Self::IDENTITY;
        mat.cols[0].x = s.x;
        mat.cols[1].y = s.y;
        mat.cols[2].z = s.z;
        mat
    }

    #[inline]
    pub fn look_at_rh(eye: Vec3, center: Vec3, up: Vec3) -> Self {
        let f = (center - eye).normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);

        let mut mat = Self::IDENTITY;
        
        mat.cols[0].x = s.x;
        mat.cols[1].x = s.y;
        mat.cols[2].x = s.z;
        mat.cols[3].x = -s.dot(eye);

        mat.cols[0].y = u.x;
        mat.cols[1].y = u.y;
        mat.cols[2].y = u.z;
        mat.cols[3].y = -u.dot(eye);

        mat.cols[0].z = -f.x;
        mat.cols[1].z = -f.y;
        mat.cols[2].z = -f.z;
        mat.cols[3].z = f.dot(eye);

        mat
    }

    #[inline]
    pub fn to_cols_array_2d(&self) -> [[f32; 4]; 4] {
        [
            [self.cols[0].x, self.cols[0].y, self.cols[0].z, self.cols[0].w],
            [self.cols[1].x, self.cols[1].y, self.cols[1].z, self.cols[1].w],
            [self.cols[2].x, self.cols[2].y, self.cols[2].z, self.cols[2].w],
            [self.cols[3].x, self.cols[3].y, self.cols[3].z, self.cols[3].w],
        ]
    }

    pub fn inverse(&self) -> Option<Self> {
        let m = self.cols;

        let coef00 = m[2].z * m[3].w - m[3].z * m[2].w;
        let coef02 = m[1].z * m[3].w - m[3].z * m[1].w;
        let coef03 = m[1].z * m[2].w - m[2].z * m[1].w;
        let coef04 = m[2].y * m[3].w - m[3].y * m[2].w;
        let coef06 = m[1].y * m[3].w - m[3].y * m[1].w;
        let coef07 = m[1].y * m[2].w - m[2].y * m[1].w;
        let coef08 = m[2].y * m[3].z - m[3].y * m[2].z;
        let coef10 = m[1].y * m[3].z - m[3].y * m[1].z;
        let coef11 = m[1].y * m[2].z - m[2].y * m[1].z;
        let coef12 = m[2].x * m[3].w - m[3].x * m[2].w;
        let coef14 = m[1].x * m[3].w - m[3].x * m[1].w;
        let coef15 = m[1].x * m[2].w - m[2].x * m[1].w;
        let coef16 = m[2].x * m[3].z - m[3].x * m[2].z;
        let coef18 = m[1].x * m[3].z - m[3].x * m[1].z;
        let coef19 = m[1].x * m[2].z - m[2].x * m[1].z;
        let coef20 = m[2].x * m[3].y - m[3].x * m[2].y;
        let coef22 = m[1].x * m[3].y - m[3].x * m[1].y;
        let coef23 = m[1].x * m[2].y - m[2].x * m[1].y;

        let fac0 = Vec4::new(coef00, coef00, coef02, coef03);
        let fac1 = Vec4::new(coef04, coef04, coef06, coef07);
        let fac2 = Vec4::new(coef08, coef08, coef10, coef11);
        let fac3 = Vec4::new(coef12, coef12, coef14, coef15);
        let fac4 = Vec4::new(coef16, coef16, coef18, coef19);
        let fac5 = Vec4::new(coef20, coef20, coef22, coef23);

        let vec0 = Vec4::new(m[1].x, m[0].x, m[0].x, m[0].x);
        let vec1 = Vec4::new(m[1].y, m[0].y, m[0].y, m[0].y);
        let vec2 = Vec4::new(m[1].z, m[0].z, m[0].z, m[0].z);
        let vec3 = Vec4::new(m[1].w, m[0].w, m[0].w, m[0].w);

        let inv0 = Vec4::new(
             vec1.x * fac0.x - vec2.x * fac1.x + vec3.x * fac2.x,
            -vec1.y * fac0.y + vec2.y * fac1.y - vec3.y * fac2.y,
             vec1.z * fac0.z - vec2.z * fac1.z + vec3.z * fac2.z,
            -vec1.w * fac0.w + vec2.w * fac1.w - vec3.w * fac2.w,
        );

        let inv1 = Vec4::new(
            -vec0.x * fac0.x + vec2.x * fac3.x - vec3.x * fac4.x,
             vec0.y * fac0.y - vec2.y * fac3.y + vec3.y * fac4.y,
            -vec0.z * fac0.z + vec2.z * fac3.z - vec3.z * fac4.z,
             vec0.w * fac0.w - vec2.w * fac3.w + vec3.w * fac4.w,
        );

        let inv2 = Vec4::new(
             vec0.x * fac1.x - vec1.x * fac3.x + vec3.x * fac5.x,
            -vec0.y * fac1.y + vec1.y * fac3.y - vec3.y * fac5.y,
             vec0.z * fac1.z - vec1.z * fac3.z + vec3.z * fac5.z,
            -vec0.w * fac1.w + vec1.w * fac3.w - vec3.w * fac5.w,
        );

        let inv3 = Vec4::new(
            -vec0.x * fac2.x + vec1.x * fac4.x - vec2.x * fac5.x,
             vec0.y * fac2.y - vec1.y * fac4.y + vec2.y * fac5.y,
            -vec0.z * fac2.z + vec1.z * fac4.z - vec2.z * fac5.z,
             vec0.w * fac2.w - vec1.w * fac4.w + vec2.w * fac5.w,
        );

        let row0 = Vec4::new(inv0.x, inv1.x, inv2.x, inv3.x);
        let dot0 = m[0].x * row0.x + m[0].y * row0.y + m[0].z * row0.z + m[0].w * row0.w;

        if dot0.abs() < 1e-10 {
            return None;
        }

        let rcp_det = 1.0 / dot0;

        Some(Self {
            cols: [
                Vec4::new(inv0.x * rcp_det, inv0.y * rcp_det, inv0.z * rcp_det, inv0.w * rcp_det),
                Vec4::new(inv1.x * rcp_det, inv1.y * rcp_det, inv1.z * rcp_det, inv1.w * rcp_det),
                Vec4::new(inv2.x * rcp_det, inv2.y * rcp_det, inv2.z * rcp_det, inv2.w * rcp_det),
                Vec4::new(inv3.x * rcp_det, inv3.y * rcp_det, inv3.z * rcp_det, inv3.w * rcp_det),
            ],
        })
    }
}

// Mat4 * Mat4 çarpımı (Projection * View * Model için kalbi)
impl Mul for Mat4 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        let mut result = Self::IDENTITY;
        for c in 0..4 {
            for r in 0..4 {
                let mut sum = 0.0;
                for i in 0..4 {
                    let a = match r {
                        0 => self.cols[i].x,
                        1 => self.cols[i].y,
                        2 => self.cols[i].z,
                        3 => self.cols[i].w,
                        _ => 0.0,
                    };
                    let b = match i {
                        0 => rhs.cols[c].x,
                        1 => rhs.cols[c].y,
                        2 => rhs.cols[c].z,
                        3 => rhs.cols[c].w,
                        _ => 0.0,
                    };
                    sum += a * b;
                }
                match r {
                    0 => result.cols[c].x = sum,
                    1 => result.cols[c].y = sum,
                    2 => result.cols[c].z = sum,
                    3 => result.cols[c].w = sum,
                    _ => (),
                }
            }
        }
        result
    }
}

// Mat4 * Vec4 çarpımı (Vertex Transformation & Raycasting için)
impl Mul<Vec4> for Mat4 {
    type Output = Vec4;

    #[inline]
    fn mul(self, rhs: Vec4) -> Self::Output {
        Vec4::new(
            self.cols[0].x * rhs.x + self.cols[1].x * rhs.y + self.cols[2].x * rhs.z + self.cols[3].x * rhs.w,
            self.cols[0].y * rhs.x + self.cols[1].y * rhs.y + self.cols[2].y * rhs.z + self.cols[3].y * rhs.w,
            self.cols[0].z * rhs.x + self.cols[1].z * rhs.y + self.cols[2].z * rhs.z + self.cols[3].z * rhs.w,
            self.cols[0].w * rhs.x + self.cols[1].w * rhs.y + self.cols[2].w * rhs.z + self.cols[3].w * rhs.w,
        )
    }
}
