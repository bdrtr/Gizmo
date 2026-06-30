/// AAA Broadphase: Dynamic AABB Tree (Incremental BVH)
///
/// Özellikler:
/// - SAH (Surface Area Heuristic) ile O(log N) ekleme
/// - Fattened AABB ile gereksiz rebuild yok
/// - AVL rotasyonlu yükseklik dengesi
/// - SIMD AABB overlap testi (x86_64)
/// - Raycast / AABB sorgusu O(log N)
/// - query_pairs: duplicate-free, self-pair yok
///
/// Düzeltmeler (orijinal koda göre):
/// FIX-1  insert: tight_aabb kontrolünde cmpge/cmple yerine skalar karşılaştırma
///         (Vec3A/Vec3 farkından kaynaklanan derleme hatası riski)
/// FIX-2  query_pairs: "iki aşamalı stack" yaklaşımı yanlış duplicate üretiyordu;
///         recursive dual-tree traversal ile replace edildi → garantili duplicate-free
/// FIX-3  balance: rotasyon sonrası f/g parent pointer güncellenmiyordu (silent bug)
/// FIX-4  SpatialHash::clear: &self alıyordu, artık &mut self
/// FIX-5  ray_aabb: tmin başlangıcı 0.0 yerine f32::NEG_INFINITY → negatif yönlerde miss
/// FIX-6  query_pairs SIMD yolu: orijinal kodda SIMD kullanılmıyordu; leaf batch'ler için eklendi
/// FIX-7  find_best_sibling: inherited_cost hesabında yanlış node SA kullanılıyordu
/// FIX-8  remove_leaf → free_node çağrısı parent'ı serbest bırakıyordu ama
///         parent'ın entity/aabb alanları temizlenmiyordu → free_node düzeltildi
use gizmo_core::entity::Entity;
use gizmo_math::{Aabb, Vec3};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// SIMD yardımcıları
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// SIMD: target AABB'i 4 AABB ile aynı anda test et.
/// Dönen bitmask'te bit i, target'ın others[i] ile örtüşüp örtüşmediğini gösterir.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    unsafe {
        let t_min_x = _mm_set1_ps(target.min.x);
        let t_max_x = _mm_set1_ps(target.max.x);
        let t_min_y = _mm_set1_ps(target.min.y);
        let t_max_y = _mm_set1_ps(target.max.y);
        let t_min_z = _mm_set1_ps(target.min.z);
        let t_max_z = _mm_set1_ps(target.max.z);

        let o_min_x = _mm_set_ps(
            others[3].min.x,
            others[2].min.x,
            others[1].min.x,
            others[0].min.x,
        );
        let o_max_x = _mm_set_ps(
            others[3].max.x,
            others[2].max.x,
            others[1].max.x,
            others[0].max.x,
        );
        let o_min_y = _mm_set_ps(
            others[3].min.y,
            others[2].min.y,
            others[1].min.y,
            others[0].min.y,
        );
        let o_max_y = _mm_set_ps(
            others[3].max.y,
            others[2].max.y,
            others[1].max.y,
            others[0].max.y,
        );
        let o_min_z = _mm_set_ps(
            others[3].min.z,
            others[2].min.z,
            others[1].min.z,
            others[0].min.z,
        );
        let o_max_z = _mm_set_ps(
            others[3].max.z,
            others[2].max.z,
            others[1].max.z,
            others[0].max.z,
        );

        // overlap koşulu: a.min <= b.max && b.min <= a.max (her eksen)
        let res = _mm_and_ps(
            _mm_and_ps(
                _mm_and_ps(
                    _mm_cmple_ps(t_min_x, o_max_x),
                    _mm_cmple_ps(o_min_x, t_max_x),
                ),
                _mm_and_ps(
                    _mm_cmple_ps(t_min_y, o_max_y),
                    _mm_cmple_ps(o_min_y, t_max_y),
                ),
            ),
            _mm_and_ps(
                _mm_cmple_ps(t_min_z, o_max_z),
                _mm_cmple_ps(o_min_z, t_max_z),
            ),
        );

        _mm_movemask_ps(res) as u8
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    let mut mask = 0u8;
    for (i, other) in others.iter().enumerate() {
        if aabb_overlaps(target, other) {
            mask |= 1 << i;
        }
    }
    mask
}

/// Scalar AABB overlap testi
#[inline]
fn aabb_overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x
        && b.min.x <= a.max.x
        && a.min.y <= b.max.y
        && b.min.y <= a.max.y
        && a.min.z <= b.max.z
        && b.min.z <= a.max.z
}

/// AABB surface area (SAH için)
#[inline]
fn surface_area(a: &Aabb) -> f32 {
    let d = Vec3::new(a.max.x - a.min.x, a.max.y - a.min.y, a.max.z - a.min.z);
    2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
}

/// AABB birleştirme
#[inline]
fn merge_aabb(a: &Aabb, b: &Aabb) -> Aabb {
    Aabb {
        min: Vec3::new(
            a.min.x.min(b.min.x),
            a.min.y.min(b.min.y),
            a.min.z.min(b.min.z),
        )
        .into(),
        max: Vec3::new(
            a.max.x.max(b.max.x),
            a.max.y.max(b.max.y),
            a.max.z.max(b.max.z),
        )
        .into(),
    }
}

/// AABB'i her yönde margin kadar büyüt
#[inline]
fn fatten(aabb: &Aabb, margin: f32) -> Aabb {
    Aabb {
        min: Vec3::new(
            aabb.min.x - margin,
            aabb.min.y - margin,
            aabb.min.z - margin,
        )
        .into(),
        max: Vec3::new(
            aabb.max.x + margin,
            aabb.max.y + margin,
            aabb.max.z + margin,
        )
        .into(),
    }
}

/// a, b'nin içinde mi? (her eksende)
/// FIX-1: Vec3A cmpge/cmple yerine açık skalar karşılaştırma
#[inline]
fn aabb_contains(outer: &Aabb, inner: &Aabb) -> bool {
    inner.min.x >= outer.min.x
        && inner.min.y >= outer.min.y
        && inner.min.z >= outer.min.z
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
        && inner.max.z <= outer.max.z
}

// ─────────────────────────────────────────────────────────────────────────────
// BVH Node
// ─────────────────────────────────────────────────────────────────────────────

const NULL: usize = usize::MAX;


// god-file Tier 3 round-2 bölmesi: BVH ağacı ve spatial-hash ayrı alt-modüllerde
mod aabb_tree;
mod spatial_hash;

pub use aabb_tree::DynamicAabbTree;
pub use spatial_hash::SpatialHash;

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(id: u32) -> Entity {
        Entity::new(id, 0)
    }

    fn make_aabb(cx: f32, cy: f32, cz: f32, r: f32) -> Aabb {
        Aabb::from_center_half_extents(Vec3::new(cx, cy, cz), Vec3::splat(r))
    }

    #[test]
    fn test_insert_query_pairs_basic() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        let e3 = make_entity(3);

        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0)); // e1 ile örtüşür
        tree.insert(e3, make_aabb(100.0, 0.0, 0.0, 1.0)); // uzakta

        let pairs = tree.query_pairs();

        let has_12 = pairs
            .iter()
            .any(|&(a, b)| (a == e1 && b == e2) || (a == e2 && b == e1));
        let has_13 = pairs
            .iter()
            .any(|&(a, b)| (a == e1 && b == e3) || (a == e3 && b == e1));

        assert!(has_12, "e1-e2 çifti bulunamadı: {:?}", pairs);
        assert!(!has_13, "e1-e3 yanlış çifti var: {:?}", pairs);
    }

    #[test]
    fn test_no_self_pair() {
        let mut tree = DynamicAabbTree::new();
        tree.insert(make_entity(1), make_aabb(0.0, 0.0, 0.0, 1.0));
        assert!(tree.query_pairs().is_empty(), "Self-pair üretilmemeli");
    }

    #[test]
    fn test_no_duplicate_pairs() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..8 {
            tree.insert(make_entity(i), make_aabb(i as f32 * 0.5, 0.0, 0.0, 1.0));
        }
        let pairs = tree.query_pairs();
        let mut seen = std::collections::HashSet::new();
        for &(a, b) in &pairs {
            let key = (a.id().min(b.id()), a.id().max(b.id()));
            assert!(seen.insert(key), "Duplicate çift: ({}, {})", a.id(), b.id());
        }
    }

    #[test]
    fn test_remove() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(0.5, 0.0, 0.0, 1.0));
        tree.remove(e1);
        assert!(
            tree.query_pairs().is_empty(),
            "Silinen entity hâlâ çift üretiyor"
        );
        assert_eq!(tree.entity_count(), 1);
    }

    #[test]
    fn test_fat_aabb_no_rebuild() {
        let mut tree = DynamicAabbTree::new();
        let e = make_entity(1);
        tree.insert(e, make_aabb(0.0, 0.0, 0.0, 1.0));
        let initial_node = tree.entity_map[&e.id()];
        // Fat margin = 0.1, küçük hareket yeniden ekleme gerektirmemeli
        tree.insert(e, make_aabb(0.05, 0.0, 0.0, 1.0));
        let after_node = tree.entity_map[&e.id()];
        assert_eq!(initial_node, after_node, "Küçük harekette node değişmemeli");
    }

    #[test]
    fn test_query_aabb() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(10.0, 0.0, 0.0, 1.0));

        let query = make_aabb(0.5, 0.0, 0.0, 0.5);
        let result = tree.query_aabb(&query);
        assert!(result.contains(&e1), "e1 sorgu sonucunda olmalı");
        assert!(!result.contains(&e2), "e2 sorgu sonucunda olmamalı");
    }

    #[test]
    fn test_query_ray() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(5.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(0.0, 10.0, 0.0, 1.0)); // Ray'in yolunda değil

        let hits = tree.query_ray(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 100.0);
        assert!(hits.iter().any(|(e, _)| *e == e1), "Ray e1'e isabet etmeli");
        assert!(
            !hits.iter().any(|(e, _)| *e == e2),
            "Ray e2'ye isabet etmemeli"
        );

        // t değerleri sıralı mı?
        for i in 1..hits.len() {
            assert!(
                hits[i].1 >= hits[i - 1].1,
                "Ray sonuçları t'ye göre sıralı olmalı"
            );
        }
    }

    #[test]
    fn test_ray_origin_inside_aabb() {
        // FIX-5 testi: origin AABB içindeyken de çalışmalı
        let mut tree = DynamicAabbTree::new();
        let e = make_entity(1);
        tree.insert(e, make_aabb(0.0, 0.0, 0.0, 5.0));
        let hits = tree.query_ray(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 100.0);
        assert!(!hits.is_empty(), "Origin AABB içindeyken ray isabet etmeli");
    }

    #[test]
    fn test_spatial_hash_compat() {
        let mut sh = SpatialHash::new(10.0);
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        sh.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        sh.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0));
        sh.clear(); // FIX-4: &mut self
        assert_eq!(sh.query_pairs().len(), 0);
    }

    #[test]
    fn test_many_entities_no_crash() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..100 {
            tree.insert(make_entity(i), make_aabb((i as f32) * 0.3, 0.0, 0.0, 0.5));
        }
        let pairs = tree.query_pairs();
        // Sadece kilitlenme/panic olmadığını ve mantıklı çıktı üretildiğini doğrula
        assert!(!pairs.is_empty(), "Yakın nesneler çift üretmeli");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_validate_after_operations() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..20 {
            tree.insert(make_entity(i), make_aabb(i as f32, 0.0, 0.0, 0.6));
        }
        tree.validate();
        tree.remove(make_entity(5));
        tree.remove(make_entity(10));
        tree.validate();
    }
}
