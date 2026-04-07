pub mod ray;
pub mod aabb;
pub mod frustum;

// Geriye dönük uyumluluk veya ekstra yardımcı metodlar için pub modüller kalsın
// ama custom tipleri glam ile değiştiriyoruz.
pub use glam::{
    Vec2,
    Vec3,
    Vec4,
    Mat4,
    Quat
};

pub use ray::Ray;
pub use aabb::Aabb;
pub use frustum::Frustum;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_dot_product() {
        let v1 = Vec3::new(1.0, 0.0, 0.0);
        let v2 = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(v1.dot(v2), 0.0); // Orthogonal vectors

        let v3 = Vec3::new(2.0, 3.0, 4.0);
        let v4 = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(v3.dot(v4), 2.0*1.0 + 3.0*2.0 + 4.0*3.0);
    }

    #[test]
    fn test_vector_cross_product() {
        let v1 = Vec3::new(1.0, 0.0, 0.0);
        let v2 = Vec3::new(0.0, 1.0, 0.0);
        let cross = v1.cross(v2);
        assert_eq!(cross, Vec3::new(0.0, 0.0, 1.0)); // Right-hand rule Z
    }

    #[test]
    fn test_matrix_vector_multiplication() {
        let mat = Mat4::from_translation(Vec3::new(5.0, -2.0, 3.0));
        let vec = Vec4::new(1.0, 1.0, 1.0, 1.0);
        let result = mat * vec;
        assert_eq!(result, Vec4::new(6.0, -1.0, 4.0, 1.0));
    }
}
