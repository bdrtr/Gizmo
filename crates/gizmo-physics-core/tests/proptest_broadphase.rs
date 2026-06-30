//! Property-based DIFFERENTIAL tests for the broad-phase (`DynamicAabbTree`).
//!
//! Faz 1.2 — bir broad-phase'in tek kritik kontratı: **hiçbir gerçekten
//! örtüşen çifti kaçırmamak** (false-negative yok). Bunu rastgele AABB
//! kümelerinde ağacın `query_pairs()` çıktısını O(N²) kaba-kuvvet (brute-force)
//! referansıyla karşılaştırarak doğruluyoruz.
//!
//! İki invariant:
//!   1. `fat_margin = 0` iken ağaç çiftleri kaba-kuvvetle BİREBİR aynı olmalı
//!      (ne kaçırma ne uydurma). Brute-force, motorun kullandığı AYNI AABB'leri
//!      (`Aabb::from_center_half_extents`) ve AYNI `<=` örtüşme predikatını
//!      kullanır; bu yüzden float farkı/sınır-eşitliği tutarsızlığı olamaz.
//!   2. Varsayılan (şişman) margin'de ağaç çiftleri tight-örtüşen çiftlerin
//!      ÜST-KÜMESİ olmalı — şişman AABB fazladan (yakın ama değmeyen) çift
//!      üretebilir ama gerçek bir örtüşmeyi ASLA kaçırmamalı.
//!
//! Bir çift kaçarsa proptest girdiyi otomatik küçültüp minimal karşı-örneği verir.

use gizmo_physics_core::BodyHandle;
use gizmo_math::{Aabb, Vec3};
use gizmo_physics_core::broadphase::DynamicAabbTree;
use proptest::prelude::*;
use std::collections::HashSet;

/// Tek bir kutu: (merkez_x, merkez_y, merkez_z, yarı-kenar).
type Boxf = (f32, f32, f32, f32);

/// Motorun yaprakta sakladığı AABB ile BİREBİR aynı kurulum.
fn aabb_of(b: &Boxf) -> Aabb {
    Aabb::from_center_half_extents(Vec3::new(b.0, b.1, b.2), Vec3::splat(b.3))
}

/// Motorun `aabb_overlaps` serbest fonksiyonuyla AYNI predikat (kapsayıcı `<=`).
fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x
        && b.min.x <= a.max.x
        && a.min.y <= b.max.y
        && b.min.y <= a.max.y
        && a.min.z <= b.max.z
        && b.min.z <= a.max.z
}

/// O(N²) referans: tight AABB'lerde gerçekten örtüşen tüm (i<j) indeks çiftleri.
fn brute_force_pairs(boxes: &[Boxf]) -> HashSet<(u32, u32)> {
    let aabbs: Vec<Aabb> = boxes.iter().map(aabb_of).collect();
    let mut set = HashSet::new();
    for i in 0..aabbs.len() {
        for j in (i + 1)..aabbs.len() {
            if overlaps(&aabbs[i], &aabbs[j]) {
                set.insert((i as u32, j as u32));
            }
        }
    }
    set
}

/// Ağacın çiftlerini (id_min, id_max) normalize edip set'e topla (duplicate'leri yutar).
fn tree_pairs(tree: &DynamicAabbTree) -> HashSet<(u32, u32)> {
    tree.query_pairs()
        .into_iter()
        .map(|(a, b)| (a.id().min(b.id()), a.id().max(b.id())))
        .collect()
}

/// Hem örtüşme hem ıskalama üretsin diye dar bir uzayda kutular: konumlar küçük,
/// yarı-kenarlar küçük → bazı çiftler kesişir, bazıları kesişmez.
fn arb_boxes() -> impl Strategy<Value = Vec<Boxf>> {
    let one = (-6.0f32..6.0, -6.0f32..6.0, -6.0f32..6.0, 0.3f32..2.5);
    prop::collection::vec(one, 2..=16)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// INVARIANT 1: margin=0 → ağaç çiftleri == kaba-kuvvet (birebir).
    #[test]
    fn query_pairs_exact_match_brute_force(boxes in arb_boxes()) {
        let mut tree = DynamicAabbTree::new().with_fat_margin(0.0);
        for (i, b) in boxes.iter().enumerate() {
            tree.insert(BodyHandle::from_id(i as u32), aabb_of(b));
        }
        let from_tree = tree_pairs(&tree);
        let from_brute = brute_force_pairs(&boxes);
        prop_assert_eq!(
            from_tree, from_brute,
            "BVH query_pairs (margin=0) kaba-kuvvetle eşleşmiyor"
        );
    }

    /// INVARIANT 2: şişman margin → ağaç çiftleri tight-örtüşenlerin üst-kümesi
    /// (hiçbir gerçek örtüşme kaçmaz; soundness).
    #[test]
    fn fat_query_pairs_superset_of_brute_force(boxes in arb_boxes()) {
        let mut tree = DynamicAabbTree::new(); // default fat_margin = 0.1
        for (i, b) in boxes.iter().enumerate() {
            tree.insert(BodyHandle::from_id(i as u32), aabb_of(b));
        }
        let from_tree = tree_pairs(&tree);
        let from_brute = brute_force_pairs(&boxes);
        let missed: Vec<_> = from_brute.difference(&from_tree).collect();
        prop_assert!(
            missed.is_empty(),
            "BVH şişman margin'de gerçek örtüşen çiftleri kaçırdı: {:?}",
            missed
        );
    }

    /// query_pairs ne self-pair ne de duplicate üretmeli (rastgele girdide).
    #[test]
    fn query_pairs_no_self_no_duplicate(boxes in arb_boxes()) {
        let mut tree = DynamicAabbTree::new().with_fat_margin(0.0);
        for (i, b) in boxes.iter().enumerate() {
            tree.insert(BodyHandle::from_id(i as u32), aabb_of(b));
        }
        let raw = tree.query_pairs();
        let mut seen = HashSet::new();
        for (a, b) in raw {
            prop_assert_ne!(a.id(), b.id(), "self-pair üretildi");
            let key = (a.id().min(b.id()), a.id().max(b.id()));
            prop_assert!(seen.insert(key), "duplicate çift: {:?}", key);
        }
    }
}
