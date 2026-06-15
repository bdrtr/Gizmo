//! Property-based tests for the Voronoi fracture core (`voronoi_shatter`).
//!
//! Faz 1.2 — kırılma çekirdeğinin iki kontratı:
//!   * DETERMİNİZM — aynı (extents, parça, seed) → BİREBİR aynı parçalar.
//!     (Faz 0'da quickhull HashMap/HashSet → BTree determinizm düzeltmesi #7'nin
//!     regresyonu; netcode/replay parçalanmanın deterministik olmasına bağlı.)
//!   * GEÇERLİLİK — her parça sonlu köşeler, sonlu kütle-merkezi ve pozitif
//!     (sonlu) hacme sahip; toplam hacim orijinal kutuyu aşırı aşmaz.

use gizmo_math::Vec3;
use gizmo_physics_rigid::voronoi_shatter;
use proptest::prelude::*;

fn arb_extents() -> impl Strategy<Value = Vec3> {
    (0.5f32..5.0, 0.5f32..5.0, 0.5f32..5.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// DETERMİNİZM: aynı girdi iki kez → aynı sayıda parça ve her parça
    /// (hacim, kütle-merkezi, köşeler) bit-bit aynı.
    #[test]
    fn voronoi_shatter_is_deterministic(
        extents in arb_extents(),
        num_pieces in 2u32..12,
        seed in any::<u64>(),
    ) {
        let a = voronoi_shatter(extents, num_pieces, seed);
        let b = voronoi_shatter(extents, num_pieces, seed);

        prop_assert_eq!(a.len(), b.len(), "parça sayısı determinist değil");
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            prop_assert_eq!(ca.volume, cb.volume, "parça {} hacmi determinist değil", i);
            prop_assert_eq!(ca.center_of_mass, cb.center_of_mass,
                "parça {} kütle-merkezi determinist değil", i);
            prop_assert_eq!(&ca.vertices, &cb.vertices,
                "parça {} köşeleri determinist değil", i);
        }
    }

    /// GEÇERLİLİK: her parça sonlu + pozitif hacim; köşeler/COM sonlu; toplam
    /// hacim orijinal kutu hacmini aşırı aşmaz (geometrik sağlamlık).
    #[test]
    fn voronoi_shatter_chunks_are_valid(
        extents in arb_extents(),
        num_pieces in 2u32..12,
        seed in any::<u64>(),
    ) {
        let chunks = voronoi_shatter(extents, num_pieces, seed);
        prop_assert!(!chunks.is_empty(), "hiç parça üretilmedi");

        let box_volume = 8.0 * extents.x * extents.y * extents.z;
        let mut total = 0.0f32;
        for (i, c) in chunks.iter().enumerate() {
            prop_assert!(c.volume.is_finite() && c.volume > 0.0,
                "parça {i} hacmi geçersiz: {}", c.volume);
            prop_assert!(c.center_of_mass.is_finite(), "parça {i} COM NaN/Inf");
            for v in &c.vertices {
                prop_assert!(v.is_finite(), "parça {i} köşesi NaN/Inf");
            }
            total += c.volume;
        }
        // Voronoi parçalanması yaklaşık hacim kullanır; üst sınırı gevşek tut.
        prop_assert!(
            total <= box_volume * 2.0 + 1.0,
            "toplam parça hacmi kutuyu aşırı aştı: {total} > {box_volume}"
        );
    }
}
