//! # Gizmo Math
//! 
//! Gizmo Engine'nin temel matematik altyapısını ve render/fizik veri tiplerini barındırır.
//!
//! ## Konvansiyonlar (Conventions)
//! - **Koordinat Sistemi**: Sağ-Elli (Right-Handed, RH). 
//! - **Yukarı Ekseni**: Y-Up (0.0, 1.0, 0.0).
//! - **İleri Ekseni**: -Z (Kamera her zaman eksi Z eksenine doğru bakar).
//! - **Matris Düzeni**: Column-Major (glam mimarisi ile uyumlu).
//!
//! Normal matrisi hesaplamaları için yapılandırılmış `Mat3`, ve 3B uzay sınırları 
//! hesaplamaları için boyut optimize edilmiş `Aabb`, `Frustum`, `Ray` yapıları barındırır.

pub mod aabb;
pub mod frustum;
pub mod ray;

// Geriye dönük uyumluluk veya ekstra yardımcı metodlar için pub modüller kalsın
// ama custom tipleri glam ile değiştiriyoruz.
pub use glam::{EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

pub use aabb::Aabb;
pub use frustum::{Frustum, Intersection, Plane};
pub use ray::Ray;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_intersects_aabb_inside_frustum() {
        // Frustum: Camera at (0, 0, 5), looking at -Z (RH geometry)
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Center AABB at origin
        let aabb = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));

        // Ensure AABB is cleanly within the camera frustum limits
        assert_eq!(frustum.test_aabb(&aabb), Intersection::Inside);

        // Ray shooting exactly down the -Z axis from the camera position targeting the object
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, -1.0));

        // Math simulation verification: It should collide with the box
        let t = ray.intersect_aabb(&aabb);
        assert!(t.is_some());
        
        let intersection_distance = t.unwrap();
        // Distance from camera Z=5 to AABB Front-Face Z=1 requires a travel distance of precisely 4 units
        assert!((intersection_distance - 4.0).abs() < 1e-5);
    }

    #[test]
    fn aabb_transform_then_frustum_cull() {
        // Frustum: Camera at (0, 0, 5), looking at -Z (RH bounds)
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 1.0, 0.1, 100.0);
        let vp = proj * view;
        let frustum = Frustum::from_matrix(&vp);

        // Default Unit AABB representing a local unscaled Model
        let local_aabb = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));

        // Scene Step 1: Object is pushed into the active view frustum
        let inside_mat = Mat4::from_translation(Vec3::new(0.0, 0.0, -10.0));
        let world_aabb_inside = local_aabb.transform(&inside_mat);
        assert_eq!(frustum.test_aabb(&world_aabb_inside), Intersection::Inside);

        // Scene Step 2: Object is rotated and pushed way outside to the right of the visible frustum limits
        let outside_mat = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
        let world_aabb_outside = local_aabb.transform(&outside_mat);
        assert_eq!(frustum.test_aabb(&world_aabb_outside), Intersection::Outside);
    }
}
