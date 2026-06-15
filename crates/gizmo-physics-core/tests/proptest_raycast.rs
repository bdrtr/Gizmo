//! Property-based tests for the raycast routines (`Raycast`).
//!
//! Faz 1 — Faz 0'da `ray_aabb` (içeriden-başlama normali) ve `ray_capsule`
//! (arka-kapsül sahte isabet) düzeltildi ama property kapsamı yoktu. Bu testler
//! analitik kontratları rastgele yapılandırmalarda doğrular:
//!   * KÜRE   — merkeze nişan alan dış ışın isabet eder; mesafe = |o−c|−r;
//!     isabet noktası yüzeyde; normal birim + ışına bakar. Ters yön → ıska.
//!   * OBB    — RIGID-transform değişmezliği: ışın + kutu aynı katı dönüşümle
//!     döndürülünce çarpışma mesafesi (t) DEĞİŞMEZ ve normal aynı dönüşümle taşınır.
//!     Identity rotasyonda ray_box, ray_aabb ile birebir uyuşur.
//!   * Genel  — her isabette normal birim+sonlu, t>0, isabet noktası ışın üstünde.

use gizmo_math::{Aabb, Quat, Vec3};
use gizmo_physics_core::{Ray, Raycast};
use proptest::prelude::*;

fn arb_pos(extent: f32) -> impl Strategy<Value = Vec3> {
    (-extent..extent, -extent..extent, -extent..extent).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn arb_quat() -> impl Strategy<Value = Quat> {
    (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0, 0.0f32..std::f32::consts::TAU).prop_map(
        |(x, y, z, angle)| {
            let axis = Vec3::new(x, y, z);
            let axis = if axis.length_squared() < 1e-6 { Vec3::Y } else { axis.normalize() };
            Quat::from_axis_angle(axis, angle)
        },
    )
}

fn arb_half() -> impl Strategy<Value = Vec3> {
    (0.3f32..2.0, 0.3f32..2.0, 0.3f32..2.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// KÜRE: merkeze nişan alan dış ışın isabet eder; t = |o−c|−r; isabet
    /// noktası yüzeyde; normal birim ve ışına bakar (n·d < 0).
    #[test]
    fn ray_sphere_aimed_hits_with_correct_distance(
        center in arb_pos(6.0),
        origin in arb_pos(6.0),
        radius in 0.3f32..2.0,
    ) {
        let to_center = center - origin;
        let dist = to_center.length();
        prop_assume!(dist > radius + 0.5); // ışın dışarıda başlasın

        let dir = to_center / dist;
        let ray = Ray::new(origin, dir);
        let (t, n) = Raycast::ray_sphere(&ray, center, radius)
            .expect("merkeze nişan alan dış ışın küreyi ıskaladı");

        prop_assert!(t > 0.0, "t önde olmalı: {t}");
        prop_assert!((t - (dist - radius)).abs() < 1e-2 * dist.max(1.0),
            "küre mesafesi yanlış: t={t} beklenen={}", dist - radius);

        let p = ray.point_at(t);
        prop_assert!(((p - center).length() - radius).abs() < 1e-2 * radius.max(1.0),
            "isabet noktası yüzeyde değil");
        prop_assert!(n.is_finite() && (n.length() - 1.0).abs() < 1e-3, "normal birim değil: {n:?}");
        prop_assert!(n.dot(dir) < 0.0, "normal ışına bakmıyor: n·d={}", n.dot(dir));
    }

    /// KÜRE: dış noktadan TERS yöne giden ışın küreyi ıskalar.
    #[test]
    fn ray_sphere_pointing_away_misses(
        center in arb_pos(4.0),
        offset in arb_pos(4.0),
        radius in 0.3f32..1.5,
    ) {
        let d = offset.length();
        prop_assume!(d > radius + 0.5); // origin küre dışında
        let origin = center + offset;
        let dir = offset / d; // merkezden UZAĞA
        let ray = Ray::new(origin, dir);
        prop_assert!(Raycast::ray_sphere(&ray, center, radius).is_none(),
            "ters yöne giden ışın isabet etti");
    }

    /// OBB RIGID-DEĞİŞMEZLİK: ışın + kutu aynı katı dönüşümle (Q, p) taşınınca
    /// çarpışma mesafesi t değişmez ve normal Q ile döner.
    #[test]
    fn ray_box_is_rigid_transform_invariant(
        center in arb_pos(3.0),
        rot in arb_quat(),
        half in arb_half(),
        dir_seed in arb_pos(1.0),
        xform_q in arb_quat(),
        xform_p in arb_pos(5.0),
    ) {
        // Merkeze nişan al → kutuyu garanti vurur. Origin'i dışarı koy.
        let off_dir = if dir_seed.length_squared() < 1e-6 { Vec3::X } else { dir_seed.normalize() };
        let origin = center + off_dir * 8.0;
        let dir = (center - origin).normalize();
        let ray = Ray::new(origin, dir);

        let base = Raycast::ray_box(&ray, center, rot, half);
        prop_assert!(base.is_some(), "merkeze nişan alan ışın kutuyu ıskaladı");
        let (t0, n0) = base.unwrap();

        // Tüm sahneyi katı dönüşümle taşı.
        let o2 = xform_q * origin + xform_p;
        let d2 = xform_q * dir;
        let c2 = xform_q * center + xform_p;
        let r2 = xform_q * rot;
        let ray2 = Ray::new(o2, d2);
        let (t1, n1) = Raycast::ray_box(&ray2, c2, r2, half)
            .expect("katı-dönüşümlü ışın kutuyu ıskaladı (değişmezlik kırıldı)");

        prop_assert!((t0 - t1).abs() < 1e-2 * t0.max(1.0),
            "rigid-transform t'yi değiştirdi: {t0} → {t1}");
        let n0_mapped = xform_q * n0;
        prop_assert!((n0_mapped - n1).length() < 2e-2,
            "rigid-transform normali tutarsız taşıdı: {n0_mapped:?} vs {n1:?}");
    }

    /// OBB ↔ AABB tutarlılığı: identity rotasyonda ray_box, ray_aabb ile aynı t.
    #[test]
    fn ray_box_identity_matches_aabb(
        center in arb_pos(4.0),
        half in arb_half(),
        dir_seed in arb_pos(1.0),
    ) {
        let off_dir = if dir_seed.length_squared() < 1e-6 { Vec3::X } else { dir_seed.normalize() };
        let origin = center + off_dir * 8.0;
        let dir = (center - origin).normalize();
        let ray = Ray::new(origin, dir);

        let (t_box, _) = Raycast::ray_box(&ray, center, Quat::IDENTITY, half)
            .expect("ray_box ıskaladı");
        let aabb = Aabb::from_center_half_extents(center, half);
        let t_aabb = Raycast::ray_aabb(&ray, &aabb).expect("ray_aabb ıskaladı");

        prop_assert!((t_box - t_aabb).abs() < 1e-3,
            "ray_box ve ray_aabb identity'de uyuşmuyor: {t_box} vs {t_aabb}");
    }
}
