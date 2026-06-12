//! Property-based tests for the narrow-phase collision detection.
//!
//! Faz 1.2 — bunlar tekil örnek (`#[test]`) yerine RASTGELE girdi uzayını
//! tarayan değişmez (invariant) testleridir. Amaç: GJK/SAT yollarının
//! matematiksel kontratlarını (simetri, birim-normal, penetrasyon işareti,
//! NaN üretmeme) binlerce kombinasyonda doğrulamak. Bir invariant kırılırsa
//! proptest girdiyi otomatik küçültüp (shrink) minimal karşı-örneği verir.

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{BoxShape, CapsuleShape, ColliderShape, SphereShape};
use gizmo_physics_core::NarrowPhase;
use proptest::prelude::*;

/// Sınırlı bir küp içinde rastgele konum.
fn arb_pos(extent: f32) -> impl Strategy<Value = Vec3> {
    (-extent..extent, -extent..extent, -extent..extent).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

/// Birim quaternion (rastgele eksen + açı). Dejenere ekseni Y'ye sabitler.
fn arb_quat() -> impl Strategy<Value = Quat> {
    (
        -1.0f32..1.0,
        -1.0f32..1.0,
        -1.0f32..1.0,
        0.0f32..std::f32::consts::TAU,
    )
        .prop_map(|(x, y, z, angle)| {
            let axis = Vec3::new(x, y, z);
            let axis = if axis.length_squared() < 1e-6 {
                Vec3::Y
            } else {
                axis.normalize()
            };
            Quat::from_axis_angle(axis, angle)
        })
}

fn arb_half_extents() -> impl Strategy<Value = Vec3> {
    (0.2f32..3.0, 0.2f32..3.0, 0.2f32..3.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn is_finite_vec(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `sphere_sphere` analitik gerçeğe uymalı: net ayrık ise temas yok,
    /// net örtüşük ise temas + doğru penetrasyon + A→B birim normal.
    #[test]
    fn sphere_sphere_matches_analytic(
        pos_a in arb_pos(5.0),
        pos_b in arb_pos(5.0),
        r_a in 0.1f32..3.0,
        r_b in 0.1f32..3.0,
    ) {
        let dist = (pos_b - pos_a).length();
        let rsum = r_a + r_b;
        let contact = NarrowPhase::sphere_sphere(pos_a, r_a, pos_b, r_b);

        // Sınır bandını (±0.01) hariç tutarak boundary jitter'dan kaçın.
        if dist > rsum + 0.01 {
            prop_assert!(contact.is_none(), "ayrık küreler temas üretti (dist={dist}, rsum={rsum})");
        } else if dist < rsum - 0.01 && dist > 0.05 {
            let c = contact.expect("örtüşen küreler temas üretmeli");
            prop_assert!(c.penetration.is_finite() && c.penetration > 0.0);
            prop_assert!((c.penetration - (rsum - dist)).abs() < 1e-3,
                "penetrasyon yanlış: {} beklenen {}", c.penetration, rsum - dist);
            // Normal birim ve A→B yönünde.
            prop_assert!((c.normal.length() - 1.0).abs() < 1e-4, "normal birim değil: {}", c.normal.length());
            let expected_n = (pos_b - pos_a).normalize();
            prop_assert!(c.normal.dot(expected_n) > 0.999, "normal A→B yönünde değil");
        }
    }

    /// Çarpışma VARLIĞI ve temas NORMALİ argüman sırasından bağımsız olmalı:
    /// A,B çarpışıyorsa B,A da çarpışır ve normal ters yöne döner.
    ///
    /// NOT: per-contact penetrasyon büyüklüğü kasıtlı olarak iddia EDİLMEZ.
    /// SAT'ın `min_pen`'i (MTV) simetriktir, ama `test_collision` face-clipping
    /// manifold'undan en derin contact NOKTASINI döndürür; o noktanın derinliği
    /// referans-yüz seçimine (= argüman sırasına) bağlı olduğundan simetrik
    /// değildir. Bu bir bug değil, yüz-clip kontak üretiminin doğasıdır.
    #[test]
    fn box_box_collision_is_symmetric(
        pos_a in arb_pos(3.0),
        rot_a in arb_quat(),
        half_a in arb_half_extents(),
        pos_b in arb_pos(3.0),
        rot_b in arb_quat(),
        half_b in arb_half_extents(),
    ) {
        let shape_a = ColliderShape::Box(BoxShape { half_extents: half_a });
        let shape_b = ColliderShape::Box(BoxShape { half_extents: half_b });

        let ab = NarrowPhase::test_collision(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b);
        let ba = NarrowPhase::test_collision(&shape_b, pos_b, rot_b, &shape_a, pos_a, rot_a);

        prop_assert_eq!(ab.is_some(), ba.is_some(),
            "çarpışma varlığı simetrik değil: ab={} ba={}", ab.is_some(), ba.is_some());

        if let (Some(ca), Some(cb)) = (ab, ba) {
            // Her iki yön de geçerli (sonlu, pozitif) temas vermeli.
            prop_assert!(ca.penetration.is_finite() && ca.penetration > 0.0);
            prop_assert!(cb.penetration.is_finite() && cb.penetration > 0.0);
            // Normaller kabaca zıt yönlü (aynı yöne BAKMAMALI).
            prop_assert!(ca.normal.dot(cb.normal) < 0.5,
                "normaller zıt değil: dot={}", ca.normal.dot(cb.normal));
        }
    }

    /// Hangi şekil çifti / poz / rotasyon olursa olsun, bir temas dönüyorsa
    /// alanları SONLU olmalı, normal birim uzunlukta, penetrasyon negatif değil.
    /// NaN/Inf üreten dejenere yolları yakalar.
    #[test]
    fn contacts_never_nan(
        kind_a in 0u8..3,
        kind_b in 0u8..3,
        pos_a in arb_pos(4.0),
        rot_a in arb_quat(),
        pos_b in arb_pos(4.0),
        rot_b in arb_quat(),
        s in 0.2f32..2.5,
    ) {
        let mk = |k: u8| -> ColliderShape {
            match k {
                0 => ColliderShape::Sphere(SphereShape { radius: s }),
                1 => ColliderShape::Box(BoxShape { half_extents: Vec3::splat(s) }),
                _ => ColliderShape::Capsule(CapsuleShape { radius: s * 0.5, half_height: s }),
            }
        };
        let shape_a = mk(kind_a);
        let shape_b = mk(kind_b);

        for c in NarrowPhase::test_collision_manifold(&shape_a, pos_a, rot_a, &shape_b, pos_b, rot_b) {
            prop_assert!(is_finite_vec(c.normal), "normal NaN/Inf: {:?}", c.normal);
            prop_assert!(is_finite_vec(c.point), "temas noktası NaN/Inf: {:?}", c.point);
            prop_assert!(c.penetration.is_finite(), "penetrasyon NaN/Inf: {}", c.penetration);
            prop_assert!(c.penetration >= -1e-3, "penetrasyon belirgin negatif: {}", c.penetration);
            let nlen = c.normal.length();
            prop_assert!((nlen - 1.0).abs() < 0.05, "normal birim değil: {nlen}");
        }
    }

    /// Aynı konum + rotasyonda iki özdeş kutu derin örtüşür → temas + pozitif
    /// penetrasyon dönmeli (self-intersection sağlık kontrolü).
    #[test]
    fn coincident_boxes_overlap(
        pos in arb_pos(3.0),
        rot in arb_quat(),
        half in arb_half_extents(),
    ) {
        let shape = ColliderShape::Box(BoxShape { half_extents: half });
        let contact = NarrowPhase::test_collision(&shape, pos, rot, &shape, pos, rot);
        let c = contact.expect("çakışık özdeş kutular temas üretmeli");
        prop_assert!(c.penetration > 0.0, "penetrasyon pozitif değil: {}", c.penetration);
        prop_assert!(is_finite_vec(c.normal) && (c.normal.length() - 1.0).abs() < 0.05);
    }
}
