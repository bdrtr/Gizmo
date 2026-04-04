use std::ops::Mul;
use crate::{vec3::Vec3, vec4::Vec4};

#[derive(Debug, Clone, Copy, PartialEq)]
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
        mat.cols[2].z = (far + near) / (near - far);
        mat.cols[2].w = -1.0;
        mat.cols[3].z = (2.0 * far * near) / (near - far);
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
