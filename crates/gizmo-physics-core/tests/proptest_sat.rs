//! Property-based tests for the box–box SAT narrow-phase manifold (`NarrowPhase::box_box`).
//!
//! Faz 1 — `proptest_collision.rs` çarpışma/normal SİMETRİSİNİ kapsıyordu; bu
//! testler MANIFOLD doğruluğunu hedefler:
//!   * ANALİTİK   — eksen-hizalı, tek eksende sığ örtüşmede penetrasyon derinliği
//!     gerçek MTV'ye (örtüşme miktarı) eşit, normal o eksen boyunca (±X).
//!   * SAĞLAMLIK  — rastgele dönmüş örtüşen kutularda her temas normali BİRİM +
//!     sonlu, penetrasyon pozitif + sonlu (SAT/clip yolu NaN/dejenere üretmez).
//!   * AYRIK      — bounding-sphere'leri ayrık kutular ASLA temas döndürmez.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::NarrowPhase;
use proptest::prelude::*;

fn arb_quat() -> impl Strategy<Value = Quat> {
    (-1.0f32..1.0, -1.0f32..1.0, -1.0f32..1.0, 0.0f32..std::f32::consts::TAU).prop_map(
        |(x, y, z, angle)| {
            let axis = Vec3::new(x, y, z);
            let axis = if axis.length_squared() < 1e-6 { Vec3::Y } else { axis.normalize() };
            Quat::from_axis_angle(axis, angle)
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// ANALİTİK: iki eksen-hizalı birim küp, X boyunca `d` mesafede; X-örtüşmesi
    /// (2−d) Y/Z-örtüşmesinden (2.0) küçük → SAT min eksen olarak X seçer. Normal
    /// ±X olmalı ve EN DERİN temasın penetrasyonu MTV'ye (2−d) eşit olmalı.
    #[test]
    fn box_box_axis_aligned_penetration_equals_mtv(d in 1.05f32..1.95) {
        let h = Vec3::splat(1.0);
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO, Quat::IDENTITY, h,
            Vec3::new(d, 0.0, 0.0), Quat::IDENTITY, h,
        );
        prop_assert!(!contacts.is_empty(), "örtüşen kutular temas üretmedi (d={d})");

        let overlap = 2.0 - d; // gerçek MTV
        let mut max_pen = 0.0f32;
        for c in &contacts {
            prop_assert!(c.normal.is_finite() && (c.normal.length() - 1.0).abs() < 1e-3,
                "normal birim değil: {:?}", c.normal);
            prop_assert!(c.normal.x.abs() > 0.99, "min eksen X olmalı, normal={:?}", c.normal);
            prop_assert!(c.penetration > 0.0 && c.penetration.is_finite(),
                "penetrasyon geçersiz: {}", c.penetration);
            prop_assert!(c.penetration <= overlap + 0.02,
                "penetrasyon MTV'yi aştı: {} > {overlap}", c.penetration);
            max_pen = max_pen.max(c.penetration);
        }
        prop_assert!((max_pen - overlap).abs() < 0.02,
            "en derin temas MTV'ye eşit değil: {max_pen} vs {overlap}");
    }

    /// SAĞLAMLIK: rastgele dönmüş, merkezleri yakın (örtüşen) iki kutu. Temas
    /// varsa: her normal birim+sonlu, her penetrasyon pozitif+sonlu.
    #[test]
    fn box_box_random_overlap_normals_valid(
        rot_a in arb_quat(),
        rot_b in arb_quat(),
        ox in -0.6f32..0.6, oy in -0.6f32..0.6, oz in -0.6f32..0.6,
        hax in 0.5f32..1.5, hay in 0.5f32..1.5, haz in 0.5f32..1.5,
    ) {
        let ha = Vec3::new(hax, hay, haz);
        let hb = Vec3::new(1.0, 1.0, 1.0);
        // Merkezler arası küçük offset → kutular kesin örtüşür (min yarı-kenar > offset).
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO, rot_a, ha,
            Vec3::new(ox, oy, oz), rot_b, hb,
        );
        prop_assert!(!contacts.is_empty(), "çakışık merkezli kutular temas üretmeli");
        for c in &contacts {
            prop_assert!(c.normal.is_finite(), "normal NaN/Inf");
            prop_assert!((c.normal.length() - 1.0).abs() < 1e-3, "normal birim değil: {:?}", c.normal);
            prop_assert!(c.penetration.is_finite() && c.penetration > 0.0,
                "penetrasyon geçersiz: {}", c.penetration);
            prop_assert!(c.point.is_finite(), "temas noktası NaN/Inf");
        }
    }

    /// AYRIK: bounding-sphere yarıçapları toplamından daha uzakta duran iki kutu
    /// (herhangi rotasyonda) ASLA temas döndürmemeli.
    #[test]
    fn box_box_separated_returns_empty(
        rot_a in arb_quat(),
        rot_b in arb_quat(),
        hax in 0.3f32..1.5, hay in 0.3f32..1.5, haz in 0.3f32..1.5,
        hbx in 0.3f32..1.5, hby in 0.3f32..1.5, hbz in 0.3f32..1.5,
        dir_x in -1.0f32..1.0, dir_y in -1.0f32..1.0, dir_z in -1.0f32..1.0,
    ) {
        let ha = Vec3::new(hax, hay, haz);
        let hb = Vec3::new(hbx, hby, hbz);
        let dir_raw = Vec3::new(dir_x, dir_y, dir_z);
        prop_assume!(dir_raw.length() > 0.3);
        let dir = dir_raw.normalize();
        // Bounding-sphere yarıçapları = köşe mesafeleri; aralarına +1.0 güvenlik payı.
        let separation = ha.length() + hb.length() + 1.0;
        let contacts = NarrowPhase::box_box(
            Vec3::ZERO, rot_a, ha,
            dir * separation, rot_b, hb,
        );
        prop_assert!(contacts.is_empty(),
            "ayrık kutular {} temas döndürdü (sep={separation})", contacts.len());
    }
}
