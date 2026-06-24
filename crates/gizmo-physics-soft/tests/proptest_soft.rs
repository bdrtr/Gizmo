//! Property-based tests for the soft-body / cloth / rope simulators.
//!
//! Faz 1.2 — yumuşak-cisim çözücülerinin kontratları:
//!   * SABİT DÜĞÜM  — pinlenen (inv_mass=0 / is_fixed) düğüm ASLA hareket etmez.
//!   * SAĞLAMLIK    — rastgele yerçekimi altında yüzlerce alt-adım NaN/Inf üretmez.
//!   * ELASTİKLİK   — sıkıştırılmış FEM tetrahedronu ÇÖKMEZ; hacmini geri kazanır
//!     (Faz 0'da FEM J-cutoff 0.1→1e-4 düzeltmesinin regresyonu).

use gizmo_math::Vec3;
use gizmo_physics_soft::{Cloth, Rope, SoftBodyMesh};
use proptest::prelude::*;

/// Bir tetrahedronun (işaretsiz) hacmi.
fn tet_volume(p: [Vec3; 4]) -> f32 {
    (p[1] - p[0]).dot((p[2] - p[0]).cross(p[3] - p[0])).abs() / 6.0
}

fn arb_gravity() -> impl Strategy<Value = Vec3> {
    (-20.0f32..20.0, -20.0f32..20.0, -20.0f32..20.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// CLOTH: pinlenen düğüm rastgele yerçekiminde bile tam yerinde kalır; hiçbir
    /// düğüm NaN/Inf olmaz.
    #[test]
    fn cloth_pinned_node_is_fixed_and_finite(g in arb_gravity()) {
        let mut cloth = Cloth::new(5, 5, 0.4, 1.0);
        cloth.pin_node(0); // sol-alt köşe
        let pinned = cloth.nodes[0].position;

        for _ in 0..120 {
            cloth.step(1.0 / 60.0, g, 4);
        }

        prop_assert_eq!(cloth.nodes[0].position, pinned, "pinli cloth düğümü hareket etti");
        for (i, n) in cloth.nodes.iter().enumerate() {
            prop_assert!(n.position.is_finite(), "cloth düğüm {i} NaN/Inf");
        }
    }

    /// ROPE: sabit uç (fix_start) rastgele yerçekiminde yerinde kalır; tüm
    /// düğümler sonlu.
    #[test]
    fn rope_fixed_endpoint_stays_and_finite(g in arb_gravity()) {
        let mut rope = Rope::new(
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            10,
            0.3,
            1.0,
            true,  // fix_start
            false,
        );
        let anchor = rope.nodes[0].position;

        for _ in 0..120 {
            rope.step(1.0 / 60.0, g);
        }

        prop_assert_eq!(rope.nodes[0].position, anchor, "sabit ip ucu hareket etti");
        for (i, n) in rope.nodes.iter().enumerate() {
            prop_assert!(n.position.is_finite(), "ip düğüm {i} NaN/Inf");
        }
    }

    /// FEM SAĞLAMLIK: tek bir tetrahedron rastgele yerçekiminde 120 kare boyunca
    /// NaN/Inf üretmez ve sonsuza ışınlanmaz.
    #[test]
    fn softbody_tet_finite_under_gravity(g in arb_gravity()) {
        let mut sb = SoftBodyMesh::new(1.0e5, 0.3).expect("valid material params");
        let rest = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        for p in rest {
            sb.add_node(p, 1.0);
        }
        sb.add_element(0, 1, 2, 3).expect("valid node indices");

        for _ in 0..120 {
            sb.step(1.0 / 60.0, g, &[]);
            for (i, n) in sb.nodes.iter().enumerate() {
                prop_assert!(n.position.is_finite(), "softbody düğüm {i} NaN/Inf");
                prop_assert!(n.velocity.is_finite(), "softbody hız {i} NaN/Inf");
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// FEM ELASTİKLİK (Faz 0 J-cutoff regresyonu): yerçekimsiz, merkeze doğru
    /// `scale` ile SIKIŞTIRILMIŞ bir tetrahedron — geçerli ama sıkışmış bir eleman
    /// ÇÖKMEMELİ, hacmini geri itmeli. Eski J-cutoff (0.1) sıkışmış elemanları
    /// çökertiyordu; düzeltilmiş çözücüde hacim sıkıştırılmış halinden BÜYÜR.
    #[test]
    fn softbody_compressed_tet_recovers_volume(scale in 0.3f32..0.7) {
        let rest = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let centroid = (rest[0] + rest[1] + rest[2] + rest[3]) / 4.0;
        let compressed: [Vec3; 4] =
            std::array::from_fn(|i| centroid + (rest[i] - centroid) * scale);

        let mut sb = SoftBodyMesh::new(1.0e5, 0.3).expect("valid material params");
        for p in compressed {
            sb.add_node(p, 1.0);
        }
        // rest_data, mevcut (sıkıştırılmış) pozisyonlardan DEĞİL — önce dinlenme
        // konfigürasyonunu kurmamız gerekir. add_element rest'i mevcut konumdan
        // hesapladığı için, düğümleri önce rest'e koyup element ekleyip sonra
        // sıkıştırırız.
        sb.nodes[0].position = rest[0];
        sb.nodes[1].position = rest[1];
        sb.nodes[2].position = rest[2];
        sb.nodes[3].position = rest[3];
        sb.add_element(0, 1, 2, 3).expect("valid node indices");
        // Şimdi sıkıştır.
        for i in 0..4 {
            sb.nodes[i].position = compressed[i];
        }

        let v_compressed = tet_volume(compressed);
        for _ in 0..240 {
            sb.step(1.0 / 60.0, Vec3::ZERO, &[]);
        }
        let v_final = tet_volume([
            sb.nodes[0].position,
            sb.nodes[1].position,
            sb.nodes[2].position,
            sb.nodes[3].position,
        ]);

        prop_assert!(v_final.is_finite(), "hacim NaN/Inf");
        prop_assert!(
            v_final > v_compressed * 1.05,
            "sıkışmış tet hacmini geri kazanmadı (çöktü?): {v_compressed} → {v_final}"
        );
    }
}
