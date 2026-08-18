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
    /// Builds a ray, normalizing `direction` and repairing a degenerate one.
    ///
    /// Because [`Ray`] is `#[non_exhaustive]`, this (and [`Ray::from_ndc`]) is the only
    /// way an external crate can make one — deliberately, because this is where the
    /// unit-length invariant that every `intersect_*` method assumes is established.
    ///
    /// A zero or near-zero `direction` carries no heading, and a bare `normalize()`
    /// would turn it into NaN. This used to be guarded by a `debug_assert!`, which
    /// compiles away in release — so release builds silently produced NaN rays whose
    /// intersection tests then returned nonsense. The guard is now a runtime one: a
    /// degenerate direction is replaced by `+X`.
    ///
    /// That substitution is **silent and unreported**. Validate the direction yourself if
    /// that matters. Note also that the `+X` fallback is not shared:
    /// [`Ray::from_ndc`] falls back to `+Z`, as does
    /// `gizmo-physics-core`'s separate `Ray` type. None of these are a sentinel you
    /// should test against.
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

    /// Unprojects a screen point into a world-space picking ray.
    ///
    /// `ndc` is Normalized Device Coordinates, x and y in `[-1.0, 1.0]`.
    /// `view_proj_inv` is the inverse of `Projection * View`.
    ///
    /// Depth convention is **WGPU/Vulkan/D3D**: NDC z runs `0.0` (near) → `1.0` (far).
    /// The origin therefore lands *on the near plane*, not at the camera position, so
    /// the `t` later returned by `intersect_*` is measured from the near plane. A
    /// GL-style `[-1, 1]` depth range would need a different near sample.
    ///
    /// **Degenerate input is repaired, never reported.** Two cases used to be covered
    /// only by `debug_assert!` — i.e. not at all in release builds:
    /// - a singular `view_proj_inv` drives the homogeneous `w` to zero or non-finite,
    ///   and the perspective divide emitted Inf/NaN. Now returns a ray at the world
    ///   origin pointing `+Z`.
    /// - near and far unproject to the same point (e.g. a degenerate orthographic
    ///   matrix), leaving a zero direction that a bare `normalize()` turned into NaN.
    ///   Now the direction falls back to `+Z` with the computed origin kept.
    ///
    /// Neither repair is reported, so the caller cannot distinguish "bad matrix" from
    /// "the user really did click along +Z".
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

    /// Checks the documented invariant: origin and direction are all-finite (no
    /// NaN/Inf) and the direction is unit-length.
    ///
    /// Needed because the guards in [`Ray::new`] / [`Ray::from_ndc`] are bypassable
    /// *inside this crate*: the fields are `pub`, so a `Ray { origin, direction }`
    /// literal can install a zero, unnormalized or NaN direction. (Note
    /// [`Ray::intersect_obb`] also builds a `Ray` by literal, but its direction is the
    /// incoming one rotated, so the invariant survives — see its own doc.) Every
    /// `intersect_*` method
    /// assumes the invariant and produces garbage without it, so validate any ray whose
    /// provenance you do not control before trusting a hit distance.
    ///
    /// The tolerance is applied to `length_squared() - 1.0` (`< 1e-4`), so it admits a
    /// length roughly within `5e-5` of 1. Loose on purpose: a direction that has been
    /// through a chain of `f32` rotations must still pass, not only one straight out of
    /// `normalize()`.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.origin.is_finite()
            && self.direction.is_finite()
            && (self.direction.length_squared() - 1.0).abs() < 1e-4
    }

    /// The point reached after travelling `t` along the ray: `origin + direction * t`.
    ///
    /// Because the direction is unit-length, `t` is a true distance in world units —
    /// the same units the `intersect_*` methods return, so `ray.at(hit_t)` is the
    /// contact point. Negative `t` walks backwards behind the origin; nothing clamps it.
    #[inline]
    pub fn at(self, t: f32) -> Vec3A {
        self.origin + self.direction * t
    }

    /// Slab-algorithm intersection against an axis-aligned box given as raw corners.
    ///
    /// Returns the distance in world units (see [`Ray::at`]) to the **entry** face when
    /// the origin is outside the box. When the origin is *inside*, entry is behind you,
    /// so the **exit** distance is returned instead — a raycast started inside a
    /// collider reports where it leaves, never `0.0`. A box lying entirely behind the
    /// origin returns `None`.
    ///
    /// Edge case worth knowing: the "is entry ahead of me?" test is strict (`tmin >
    /// 0.0`), so an origin sitting *exactly* on the entry face reports the exit
    /// distance too, not `0.0`.
    ///
    /// Boundary contact counts as a hit throughout (inclusive), matching
    /// [`Aabb::intersects`](crate::Aabb::intersects).
    ///
    /// **Axis-parallel rays are handled scalar-wise, on purpose.** For an axis where
    /// `|direction| < 1e-8` the slab is skipped entirely: miss if the origin lies
    /// outside `[min, max]` on that axis, otherwise that axis simply imposes no
    /// constraint. The previous vectorized version computed `(min - origin) * (1/0)`
    /// and produced `0 * ∞ = NaN`; `Vec3A::min/max` propagate the *other* operand on
    /// NaN, so a ray grazing the **min** face returned a phantom miss while the same
    /// ray on the **max** face hit (the scalar reduction there swallowed the NaN) —
    /// asymmetric and platform-dependent. The scalar slab makes min and max symmetric
    /// and deterministic. `test_ray_parallel_grazes_min_and_max_face` is the regression.
    ///
    /// `min` and `max` need not be ordered: each slab swaps its two plane distances, so an
    /// axis handed over back-to-front produces exactly the same interval as the correct
    /// ordering — an inverted box is silently treated as its well-ordered equivalent, not
    /// rejected. The axis-parallel branch is the exception: it has no swap, so a ray
    /// parallel to an inverted axis always misses.
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

    /// Convenience wrapper over [`Ray::intersect_bounds`] taking an [`Aabb`](crate::Aabb)
    /// directly; all of that method's semantics (inside → exit distance, inclusive
    /// boundaries, axis-parallel handling) apply unchanged.
    ///
    /// **Do not feed this an `Aabb::empty()` sentinel** (`min = +∞`, `max = -∞`); it is
    /// not special-cased and the infinities do not cancel cleanly. A general-direction
    /// ray comes back `Some(f32::INFINITY)` (every slab degenerates to
    /// `[-∞, +∞]`), while a ray with any exactly-zero direction component takes the
    /// axis-parallel branch and comes back `None`. Guard with
    /// [`Aabb::is_empty`](crate::Aabb::is_empty) before calling if empty boxes can
    /// reach you — e.g. freshly-initialised BVH nodes.
    #[inline]
    pub fn intersect_aabb(self, aabb: crate::aabb::Aabb) -> Option<f32> {
        self.intersect_bounds(aabb.min, aabb.max)
    }

    /// Exact ray–triangle intersection (Möller–Trumbore).
    ///
    /// Returns the distance in world units to the hit point, or `None`. Winding order
    /// of `v0, v1, v2` does not matter: the parallel test is `|det| < 1e-8`, i.e.
    /// **two-sided** — a triangle hit from behind still registers. (Nothing here culls
    /// backfaces, despite what the inline comment suggests; add your own normal test if
    /// you need single-sided behaviour.)
    ///
    /// Hits nearer than `1e-8` are discarded, which also covers `t <= 0` (behind the
    /// origin). That epsilon is the shadow-/bounce-ray self-intersection guard: without
    /// it a ray spawned exactly on a surface re-hits its own triangle at `t ≈ 0`.
    ///
    /// The barycentric acceptance is inclusive on the edges (`0 <= u <= 1`,
    /// `v >= 0`, `u + v <= 1`), so a ray through a shared edge hits **both**
    /// adjacent triangles — dedupe by nearest `t` if that matters.
    ///
    /// Degenerate (zero-area) triangles fall into the `|det| < 1e-8` branch and report
    /// `None` rather than dividing by zero.
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

    /// Intersection with an Oriented Bounding Box.
    ///
    /// `center` and `rotation` place the box in world space (`rotation` maps box-local
    /// axes to world); `half_extents` are the box's half-sizes **along its own axes**, in
    /// the same world units as `center`.
    ///
    /// Works by pulling the ray into box-local space and reusing
    /// [`Ray::intersect_bounds`] against `[-half_extents, +half_extents]`, so all of
    /// that method's semantics carry over — including "origin inside ⇒ exit distance".
    /// The returned `t` needs no rescaling: a quaternion rotation preserves length, so
    /// local distance equals world distance.
    ///
    /// The local ray is built with a struct literal rather than [`Ray::new`], skipping
    /// the normalize + degenerate guard. That is safe *only* because the rotation is
    /// length-preserving and the incoming direction already satisfies the unit-length
    /// invariant — which is exactly why that invariant is enforced at construction.
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

    /// Regression: a ray parallel to an axis must hit when its origin is exactly on the MIN
    /// face. The old state produced `0 * ∞ = NaN` and, because `Vec3A::min/max` (SIMD) propagated
    /// the wrong operand, returned a bogus miss (whereas the max face worked because the scalar
    /// reduction swallowed the NaN → asymmetry). The min and max faces are now symmetric.
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

    // ── `intersect_triangle` ─────────────────────────────────────────────────────────────────
    //
    // Documented in detail and, until 2026-08-18, mentioned nowhere in the workspace and covered
    // by nothing. Every claim below is one its doc comment makes.

    /// A unit triangle in the z = 0 plane, spanning (0,0)–(1,0)–(0,1).
    fn tri() -> (Vec3, Vec3, Vec3) {
        (
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        )
    }

    #[test]
    fn a_ray_through_the_middle_hits_at_the_right_distance() {
        let (a, b, c) = tri();
        let ray = Ray::new(Vec3::new(0.25, 0.25, -3.0), Vec3::Z);
        let t = ray.intersect_triangle(a, b, c).expect("straight through the face");
        assert!((t - 3.0).abs() < 1e-5, "t = {t}");
    }

    /// "Both faces hit, despite what the inline comment suggests" — the docs say so explicitly,
    /// so a later `a < 1e-8` (culling backfaces) would be a behaviour change and not a tidy-up.
    #[test]
    fn a_backface_hit_is_still_a_hit() {
        let (a, b, c) = tri();
        let front = Ray::new(Vec3::new(0.25, 0.25, -3.0), Vec3::Z);
        let back = Ray::new(Vec3::new(0.25, 0.25, 3.0), Vec3::NEG_Z);
        assert!(front.intersect_triangle(a, b, c).is_some());
        assert!(
            back.intersect_triangle(a, b, c).is_some(),
            "this test is single-sided now, which the docs say it is not"
        );
    }

    #[test]
    fn a_ray_beside_the_triangle_misses() {
        let (a, b, c) = tri();
        // Past the hypotenuse: u + v > 1.
        let ray = Ray::new(Vec3::new(0.9, 0.9, -3.0), Vec3::Z);
        assert!(ray.intersect_triangle(a, b, c).is_none());
        // And outside the u range entirely.
        let ray = Ray::new(Vec3::new(-0.5, 0.25, -3.0), Vec3::Z);
        assert!(ray.intersect_triangle(a, b, c).is_none());
    }

    /// Edge-inclusive on purpose: a ray through a shared edge hits **both** adjacent triangles,
    /// and the docs tell the caller to dedupe by nearest `t`. A strict comparison would silently
    /// open a crack along every shared edge in a mesh, which is the classic way a raycast starts
    /// falling through the floor.
    #[test]
    fn the_barycentric_acceptance_is_inclusive_on_the_edges() {
        let (a, b, c) = tri();
        for point in [
            Vec3::new(0.5, 0.5, 0.0), // exactly on the hypotenuse: u + v == 1
            Vec3::new(0.0, 0.5, 0.0), // on the u == 0 edge
            Vec3::new(0.5, 0.0, 0.0), // on the v == 0 edge
        ] {
            let ray = Ray::new(point + Vec3::new(0.0, 0.0, -3.0), Vec3::Z);
            assert!(
                ray.intersect_triangle(a, b, c).is_some(),
                "a ray through {point:?} — exactly on an edge — must hit"
            );
        }
    }

    #[test]
    fn a_triangle_behind_the_origin_is_not_a_hit() {
        let (a, b, c) = tri();
        let ray = Ray::new(Vec3::new(0.25, 0.25, 3.0), Vec3::Z); // pointing away
        assert!(ray.intersect_triangle(a, b, c).is_none());
    }

    /// The `t > 1e-8` guard, which the docs describe as the self-intersection epsilon: a ray
    /// spawned exactly ON a surface must not re-hit it at `t ≈ 0`. Without it every shadow or
    /// bounce ray in a renderer starts by hitting the thing it left.
    #[test]
    fn a_ray_starting_on_the_surface_does_not_hit_it_again() {
        let (a, b, c) = tri();
        let ray = Ray::new(Vec3::new(0.25, 0.25, 0.0), Vec3::Z);
        assert!(
            ray.intersect_triangle(a, b, c).is_none(),
            "a ray leaving the surface re-hit its own triangle at t ≈ 0"
        );
    }

    #[test]
    fn a_degenerate_triangle_reports_nothing_rather_than_dividing_by_zero() {
        let p = Vec3::new(1.0, 2.0, 3.0);
        let ray = Ray::new(Vec3::ZERO, Vec3::X);
        let t = ray.intersect_triangle(p, p, p); // zero area
        assert!(t.is_none(), "got {t:?}");
        assert!(t.is_none_or(|v| v.is_finite()), "and certainly not a NaN");
    }
}
