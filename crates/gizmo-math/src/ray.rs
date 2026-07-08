use glam::{Quat, Vec3, Vec3A};

/// A 3D ray with an origin and a normalized direction.
///
/// `#[non_exhaustive]` forbids struct-literal construction from other crates, so
/// external callers must go through [`Ray::new`] / [`Ray::from_ndc`], which
/// enforce the normalized-direction invariant. Fields stay public for reading.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Ray {
    /// World-space starting point of the ray.
    pub origin: Vec3A,
    /// Unit-length direction the ray travels along.
    pub direction: Vec3A, // Normalize edilmiş olmalı
}

impl Ray {
    #[inline]
    pub fn new(origin: impl Into<Vec3A>, direction: impl Into<Vec3A>) -> Self {
        // normalize() sıfır/yakın-sıfır yönde NaN üretir; release'de debug_assert
        // kaybolup geçersiz (NaN) Ray sessizce oluşurdu. normalize_or_zero ile
        // dejenere yönü tespit edip güvenli bir varsayılana (+X) düşürüyoruz.
        let dir = direction.into().normalize_or_zero();
        let direction = if dir == Vec3A::ZERO { Vec3A::X } else { dir };
        Self {
            origin: origin.into(),
            direction,
        }
    }

    /// NDC (Normalized Device Coordinates) uzayından 3B Dünya (World) uzayına bir Ray oluşturur.
    /// `ndc`: [-1.0, 1.0] aralığında ekran koordinatları.
    /// `view_proj_inv`: (Projection * View) matrisinin tersi.
    #[inline]
    pub fn from_ndc(ndc: glam::Vec2, view_proj_inv: glam::Mat4) -> Self {
        // WGPU standardında NDC depth 0.0 (near) ile 1.0 (far) arasındadır.
        let near_ndc = glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let far_ndc = glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);

        // debug_assert! release'de derlenip kaybolur; tekil (singular) VP-inverse
        // veya sıfır w bileşeni sessizce Inf/NaN üretirdi. Runtime guard ile
        // dejenere w tespit edip güvenli bir varsayılan Ray'e (+Z) düşüyoruz.
        let near_world = view_proj_inv * near_ndc;
        let far_world = view_proj_inv * far_ndc;

        let fallback = || Self::new(Vec3::ZERO, Vec3::Z);

        if near_world.w.abs() < 1e-10 || !near_world.w.is_finite() {
            return fallback();
        }
        if far_world.w.abs() < 1e-10 || !far_world.w.is_finite() {
            return fallback();
        }

        let near_world = near_world / near_world.w;
        let far_world = far_world / far_world.w;

        let origin = near_world.truncate();
        // near == far (ör. dejenere ortografik projeksiyon) durumunda yön sıfır
        // vektör olur; bare normalize() NaN üretir → safe_normalize_or ile +Z'ye düş.
        let direction = crate::safe_normalize_or(far_world.truncate() - origin, Vec3::Z);

        Self::new(origin, direction)
    }

    /// Ray'in dokümante edilen değişmezini (invariant) sağlayıp sağlamadığını
    /// doğrular: yön birim uzunlukta ve tüm bileşenler sonlu (NaN/Inf değil).
    ///
    /// Alanlar `pub` olduğundan çağıranlar `Ray { .. }` struct literaliyle
    /// [`Ray::new`]/[`Ray::from_ndc`] guard'larını atlayarak geçersiz (sıfır/
    /// normalize-edilmemiş/NaN yönlü) bir Ray kurabilir. Böyle bir Ray'e
    /// güvenmeden önce bu kontrolle doğrulanabilir.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.origin.is_finite()
            && self.direction.is_finite()
            && (self.direction.length_squared() - 1.0).abs() < 1e-4
    }

    /// Işının uzayda `t` uzaklığındaki ulaştığı (çarpıştığı) kesin noktayı hesaplar.
    #[inline]
    pub fn at(self, t: f32) -> Vec3A {
        self.origin + self.direction * t
    }

    /// Bir eksen kısıtlı boundary kutusuyla kesişim testi yapar (Slab Algorithm).
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    ///
    /// Eksene paralel bir ışın (bir bileşende `direction == 0`) o eksende ayrıca
    /// ele alınır: kaynak koordinatı [min, max] aralığının DIŞINDAYSA ıska,
    /// içindeyse (sınır dahil) o eksen kısıt getirmez. Eski vektörleştirilmiş hal
    /// `(min - origin) * (1/0)` ile `0 * ∞ = NaN` üretiyordu; `Vec3A::min/max`
    /// (SIMD) NaN'da yanlış operandı yaydığından ışın min-yüzüne tam değerken
    /// sahte ıska dönüyor, max-yüzünde ise skaler indirgeme NaN'ı yuttuğu için
    /// isabet dönüyordu — asimetrik ve platforma bağlı. Skaler slab bunu
    /// deterministik ve simetrik kılar.
    #[inline]
    pub fn intersect_bounds(self, min: Vec3A, max: Vec3A) -> Option<f32> {
        let o = [self.origin.x, self.origin.y, self.origin.z];
        let d = [self.direction.x, self.direction.y, self.direction.z];
        let mn = [min.x, min.y, min.z];
        let mx = [max.x, max.y, max.z];

        let mut tmin = f32::NEG_INFINITY;
        let mut tmax = f32::INFINITY;

        for i in 0..3 {
            if d[i].abs() < 1e-8 {
                // Bu eksene paralel: yalnızca kaynak dilimin tamamen dışındaysa ıska.
                // Sınır üstünde (o == mn ya da o == mx) nokta "içeride" (kapsayıcı),
                // dolayısıyla bu eksen kısıt getirmez — 0 * ∞ = NaN'dan kaçınılır.
                if o[i] < mn[i] || o[i] > mx[i] {
                    return None;
                }
            } else {
                let inv = 1.0 / d[i];
                let mut t0 = (mn[i] - o[i]) * inv;
                let mut t1 = (mx[i] - o[i]) * inv;
                if t0 > t1 {
                    core::mem::swap(&mut t0, &mut t1);
                }
                tmin = tmin.max(t0);
                tmax = tmax.min(t1);
                if tmin > tmax {
                    return None;
                }
            }
        }

        if tmax > 0.0 {
            Some(if tmin > 0.0 { tmin } else { tmax })
        } else {
            None
        }
    }

    /// Bir Aabb nesnesiyle (Axis-Aligned Bounding Box) doğrudan kesişim testi yapar.
    #[inline]
    pub fn intersect_aabb(self, aabb: crate::aabb::Aabb) -> Option<f32> {
        self.intersect_bounds(aabb.min, aabb.max)
    }

    /// Möller–Trumbore algoritması kullanarak bir üçgenle hassas kesişim (Mesh Raycasting) testi yapar.
    /// Kesişiyorsa t_near mesafesini döner, aksi halde None döner.
    #[inline]
    pub fn intersect_triangle(
        self,
        v0: impl Into<Vec3A>,
        v1: impl Into<Vec3A>,
        v2: impl Into<Vec3A>,
    ) -> Option<f32> {
        let v0 = v0.into();
        let v1 = v1.into();
        let v2 = v2.into();
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;

        let h = self.direction.cross(edge2);
        let a = edge1.dot(h);

        // Culling backfaces and parallel rays
        if a.abs() < 1e-8 {
            return None;
        }

        let f = 1.0 / a;
        let s = self.origin - v0;
        let u = f * s.dot(h);

        if !(0.0..=1.0).contains(&u) {
            return None;
        }

        let q = s.cross(edge1);
        let v = f * self.direction.dot(q);

        if v < 0.0 || u + v > 1.0 {
            return None;
        }

        let t = f * edge2.dot(q);

        if t > 1e-8 {
            Some(t)
        } else {
            None
        }
    }

    /// Bir OBB (Oriented Bounding Box) kutusuyla kesişim testi yapar.
    /// Kesişiyorsa t_near mesafesini döner, kesişmiyorsa None döner.
    #[inline]
    pub fn intersect_obb(
        self,
        center: impl Into<Vec3A>,
        half_extents: impl Into<Vec3A>,
        rotation: Quat,
    ) -> Option<f32> {
        let c = center.into();
        let he = half_extents.into();
        let inv_rot = rotation.inverse();

        // Işını OBB'nin yerel uzayına çeviriyoruz.
        // Quat * Vec3A dönüşümü bulunmadığı için Vec3 üzerinden yapıp Vec3A'ya cast etmeliyiz.
        let local_origin = Vec3A::from(inv_rot * Vec3::from(self.origin - c));
        let local_direction = Vec3A::from(inv_rot * Vec3::from(self.direction));

        // Quat dönüşümü uzunluğu koruduğu için direction zaten normalize edilmiştir.
        // Performans için gereksiz normalize() ve debug_assert! çağrılarından kaçınarak doğrudan struct oluşturuyoruz.
        let local_ray = Ray {
            origin: local_origin,
            direction: local_direction,
        };

        // Yerel koordinatlarda AABB testi
        local_ray.intersect_bounds(-he, he)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ray_intersect_aabb_hit() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 4.0).abs() < 1e-5); // Hits the front face at z = -1, origin is -5, distance is 4
    }

    #[test]
    fn test_ray_intersect_aabb_miss() {
        let ray = Ray::new(Vec3::new(0.0, 5.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_none());
    }

    #[test]
    fn test_ray_intersect_aabb_inside() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 1.0).abs() < 1e-5); // Inside the box, hits the back face at z = 1, distance is 1
    }

    #[test]
    fn test_ray_intersect_aabb_behind() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t = ray.intersect_aabb(aabb);
        assert!(t.is_none()); // The box is strictly behind the ray origin
    }

    #[test]
    fn test_ray_intersect_obb() {
        let origin = Vec3::new(0.0, 0.0, -5.0);
        let direction = Vec3::new(0.0, 0.0, 1.0);
        let ray = Ray::new(origin, direction);

        let obb_center = Vec3::new(0.0, 0.0, 0.0);
        let obb_extents = Vec3::new(1.0, 1.0, 1.0);

        // 45 degrees rotated around Y
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);

        let t = ray.intersect_obb(obb_center, obb_extents, rot);
        assert!(t.is_some());

        // Since OBB is rotated by 45 degrees, ray hits the tilted face earlier.
        // Unrotated distance is 4.0. With 45 degree tilt, the half-diagonal length is sqrt(2), so front face is at -sqrt(2).
        // 5.0 - 1.414 = approx 3.585
        assert!((t.unwrap() - (5.0 - std::f32::consts::SQRT_2)).abs() < 1e-4);
    }

    #[test]
    fn test_ray_parallel_hit() {
        // Parallel ray moving exactly along the Y-axis (Direction X and Z are exactly ZERO)
        let ray = Ray::new(Vec3::new(0.0, -5.0, 0.0), Vec3::Y);
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // This hit should register and perfectly calculate intersections without Div-By-Zero NaNs
        let t = ray.intersect_aabb(aabb);
        assert!(t.is_some());
        assert!((t.unwrap() - 4.0).abs() < 1e-5);
    }

    #[test]
    fn test_ray_parallel_miss() {
        // Parallel ray moving exactly along the Y-axis but offset completely outside the AABB X-range
        let ray_miss = Ray::new(Vec3::new(5.0, -5.0, 0.0), Vec3::Y);
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        let t_miss = ray_miss.intersect_aabb(aabb);
        assert!(t_miss.is_none());
    }

    /// Regresyon: eksene paralel bir ışın, kaynağı tam olarak MIN yüzeyi üstündeyken
    /// isabet etmeli. Eski hal `0 * ∞ = NaN` üretip `Vec3A::min/max` (SIMD) yanlış
    /// operandı yaydığından sahte ıska dönüyordu (max yüzü ise skaler indirgeme
    /// NaN'ı yuttuğu için çalışıyordu → asimetri). Min ve max yüzü artık simetrik.
    #[test]
    fn test_ray_parallel_grazes_min_and_max_face() {
        let aabb = crate::aabb::Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));

        // MIN yüzü: X ekseninde paralel (dir X=0), kaynak X = min.x = -1.
        let ray_min = Ray::new(Vec3::new(-1.0, -5.0, 0.0), Vec3::Y);
        let t_min = ray_min.intersect_aabb(aabb);
        assert!(t_min.is_some(), "min yüzeyine değen paralel ışın isabet etmeli");
        assert!((t_min.unwrap() - 4.0).abs() < 1e-5);

        // MAX yüzü: simetrik olarak kaynak X = max.x = 1 de isabet etmeli.
        let ray_max = Ray::new(Vec3::new(1.0, -5.0, 0.0), Vec3::Y);
        let t_max = ray_max.intersect_aabb(aabb);
        assert!(t_max.is_some(), "max yüzeyine değen paralel ışın isabet etmeli");
        assert!((t_max.unwrap() - 4.0).abs() < 1e-5);

        // Sınırın hemen dışı (X = -1.001) paralel ışında ıska olmalı.
        let ray_out = Ray::new(Vec3::new(-1.001, -5.0, 0.0), Vec3::Y);
        assert!(ray_out.intersect_aabb(aabb).is_none(), "sınır dışı paralel ışın ıska olmalı");
    }
    #[test]
    fn test_ray_from_ndc() {
        let view = glam::Mat4::look_at_rh(
            Vec3::new(0.0, 0.0, 10.0), // Camera at Z=10
            Vec3::ZERO,                 // Looking at origin
            Vec3::Y,                    // Up is Y
        );
        let proj = glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        let view_proj_inv = (proj * view).inverse();

        // Center pixel (NDC = 0,0)
        let ray_center = Ray::from_ndc(glam::Vec2::new(0.0, 0.0), view_proj_inv);
        
        // Origins usually lie on the near plane.
        // The camera is at Z=10, looking at Z=0. Direction should be -Z.
        assert!((ray_center.direction.z - (-1.0)).abs() < 1e-5);
        assert!(ray_center.direction.x.abs() < 1e-5);
        assert!(ray_center.direction.y.abs() < 1e-5);
    }

    #[test]
    fn test_ray_from_ndc_singular_matrix_is_finite() {
        // Tekil (singular) VP-inverse: w bileşeni sıfır → release'de eskiden
        // Inf/NaN üretirdi. Runtime guard artık güvenli varsayılan Ray döndürmeli.
        let ray = Ray::from_ndc(glam::Vec2::new(0.0, 0.0), glam::Mat4::ZERO);
        assert!(ray.origin.is_finite());
        assert!(ray.direction.is_finite());
        // Yön birim uzunlukta olmalı (NaN/sıfır değil).
        assert!((ray.direction.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_ray_from_ndc_degenerate_direction_is_finite() {
        // near ve far dünya noktaları çakışırsa (far - near) == 0 → bare normalize()
        // NaN üretir. Böyle bir matris kurgulayalım: her iki NDC de aynı noktaya gitsin.
        // w'yi geçerli tutup xyz'yi sabitleyen bir matris (son satır [0,0,0,1],
        // ilk üç satır sıfır) origin=(0,0,0), far=(0,0,0) verir → yön sıfır.
        let m = glam::Mat4::from_cols(
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
            glam::Vec4::ZERO,
            glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
        );
        let ray = Ray::from_ndc(glam::Vec2::new(0.3, -0.4), m);
        assert!(ray.direction.is_finite());
        assert!((ray.direction.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_ray_is_valid_detects_bypassed_invariant() {
        // Constructor guard'larını atlayan struct literali → geçersiz Ray.
        let bad = Ray {
            origin: Vec3A::ZERO,
            direction: Vec3A::ZERO,
        };
        assert!(!bad.is_valid());

        let nan = Ray {
            origin: Vec3A::ZERO,
            direction: Vec3A::splat(f32::NAN),
        };
        assert!(!nan.is_valid());

        // new()/from_ndc() ile kurulan Ray her zaman geçerli olmalı.
        assert!(Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0)).is_valid());
    }
}
