/// Island-Based Parallel Solver
///
/// Fizik dünyasını bağlı bileşenlere (island) ayırır.
/// Birbirinden bağımsız island'lar Rayon ile paralel çözülür.
/// Hareketsiz island'lar sleeping'e alınarak tamamen atlanır.
use crate::collision::ContactManifold;
use gizmo_core::entity::Entity;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Island veri yapısı
// ─────────────────────────────────────────────────────────────────────────────

/// Tek bir fizik adası — birbirine temas eden dinamik cisimler
pub struct Island {
    /// Bu island'a ait manifold indisleri (orijinal Vec'teki)
    pub manifold_indices: Vec<usize>,
    /// Uyku durumu: tüm cisimler yeterince yavaşsa true
    pub sleeping: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// IslandManager
// ─────────────────────────────────────────────────────────────────────────────

pub struct IslandManager;

impl IslandManager {
    /// Manifoldları bağlı bileşenlere (island) ayır.
    /// İki manifold aynı island'a aittir ↔ ortak bir dinamik cisme sahipler.
    ///
    /// Algoritma: Union-Find (path compression + rank)
    pub fn build_islands(
        manifolds: &[ContactManifold],
        entity_is_dynamic: &impl Fn(Entity) -> bool,
    ) -> Vec<Island> {
        if manifolds.is_empty() { return Vec::new(); }

        let n = manifolds.len();

        // ── Union-Find veri yapısı ────────────────────────────────────────────
        let mut parent: Vec<usize> = (0..n).collect();
        let mut rank:   Vec<u8>    = vec![0; n];

        fn find(parent: &mut Vec<usize>, i: usize) -> usize {
            if parent[i] != i {
                parent[i] = find(parent, parent[i]); // Path compression
            }
            parent[i]
        }

        fn union(parent: &mut Vec<usize>, rank: &mut Vec<u8>, a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra == rb { return; }
            // Rank-based union
            match rank[ra].cmp(&rank[rb]) {
                std::cmp::Ordering::Less    => parent[ra] = rb,
                std::cmp::Ordering::Greater => parent[rb] = ra,
                std::cmp::Ordering::Equal   => { parent[rb] = ra; rank[ra] += 1; }
            }
        }

        // Her dinamik entity için hangi manifoldlara ait olduğunu bul
        let mut entity_to_manifolds: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, m) in manifolds.iter().enumerate() {
            if entity_is_dynamic(m.entity_a) {
                entity_to_manifolds.entry(m.entity_a.id()).or_default().push(i);
            }
            if entity_is_dynamic(m.entity_b) {
                entity_to_manifolds.entry(m.entity_b.id()).or_default().push(i);
            }
        }

        // Aynı dinamik cisme ait manifoldları birleştir
        for manifold_list in entity_to_manifolds.values() {
            for w in manifold_list.windows(2) {
                union(&mut parent, &mut rank, w[0], w[1]);
            }
        }

        // Kökü aynı manifoldları grupla
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(i);
        }

        groups.into_values()
            .map(|indices| Island { manifold_indices: indices, sleeping: false })
            .collect()
    }

    /// Manifoldları island gruplarına göre böl — her island kendi manifold Vec'ine sahip
    pub fn split_manifolds(
        manifolds: Vec<ContactManifold>,
        islands: &[Island],
    ) -> Vec<Vec<ContactManifold>> {
        let mut manifold_opts: Vec<Option<ContactManifold>> =
            manifolds.into_iter().map(Some).collect();

        islands.iter().map(|island| {
            island.manifold_indices.iter()
                .filter_map(|&i| manifold_opts[i].take())
                .collect()
        }).collect()
    }

    /// Island'ın uyuma uygun olup olmadığını kontrol et.
    /// Tüm temas noktalarındaki impuls toplamı eşiğin altındaysa → uyku
    pub fn should_sleep(manifolds: &[ContactManifold], impulse_threshold: f32) -> bool {
        manifolds.iter().all(|m| {
            m.contacts.iter().all(|c| {
                c.normal_impulse.abs() < impulse_threshold
                    && c.tangent_impulse.length() < impulse_threshold
            })
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PhysicsMetrics — Profiling
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct PhysicsMetrics {
    pub broadphase_ms:   f32,
    pub narrowphase_ms:  f32,
    pub solver_ms:       f32,
    pub integration_ms:  f32,
    pub island_count:    usize,
    pub sleeping_count:  usize,
    pub contact_count:   usize,
    pub body_count:      usize,
}

impl PhysicsMetrics {
    pub fn print_hud(&self) {
        println!(
            "[Physics] Islands:{} Sleep:{} Contacts:{} Bodies:{} | \
             Broad:{:.2}ms Narrow:{:.2}ms Solver:{:.2}ms Integrate:{:.2}ms",
            self.island_count, self.sleeping_count, self.contact_count, self.body_count,
            self.broadphase_ms, self.narrowphase_ms, self.solver_ms, self.integration_ms,
        );
    }

    pub fn total_ms(&self) -> f32 {
        self.broadphase_ms + self.narrowphase_ms + self.solver_ms + self.integration_ms
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::entity::Entity;
    use crate::collision::{ContactManifold, ContactPoint};
    use gizmo_math::Vec3;

    fn make_manifold(ea: u32, eb: u32) -> ContactManifold {
        let mut m = ContactManifold::new(Entity::new(ea, 0), Entity::new(eb, 0));
        m.contacts.push(ContactPoint {
            point: Vec3::ZERO, normal: Vec3::Y, penetration: 0.01,
            local_point_a: Vec3::ZERO, local_point_b: Vec3::ZERO,
            normal_impulse: 0.0, tangent_impulse: Vec3::ZERO,
        });
        m
    }

    #[test]
    fn test_single_island() {
        // A-B ve B-C → tek island
        let manifolds = vec![make_manifold(1, 2), make_manifold(2, 3)];
        let is_dyn = |e: Entity| e.id() != 0;
        let islands = IslandManager::build_islands(&manifolds, &is_dyn);
        assert_eq!(islands.len(), 1, "A-B and B-C should form one island");
        assert_eq!(islands[0].manifold_indices.len(), 2);
    }

    #[test]
    fn test_two_islands() {
        // A-B ve C-D → iki ayrı island
        let manifolds = vec![make_manifold(1, 2), make_manifold(3, 4)];
        let is_dyn = |e: Entity| e.id() != 0;
        let islands = IslandManager::build_islands(&manifolds, &is_dyn);
        assert_eq!(islands.len(), 2, "A-B and C-D should form two islands");
    }

    #[test]
    fn test_empty_manifolds() {
        let is_dyn = |_: Entity| true;
        let islands = IslandManager::build_islands(&[], &is_dyn);
        assert!(islands.is_empty());
    }

    #[test]
    fn test_sleeping_detection() {
        let mut m = make_manifold(1, 2);
        m.contacts[0].normal_impulse = 0.001; // Çok düşük
        assert!(IslandManager::should_sleep(&[m], 0.01));

        let mut m2 = make_manifold(1, 2);
        m2.contacts[0].normal_impulse = 100.0; // Yüksek aktivite
        assert!(!IslandManager::should_sleep(&[m2], 0.01));
    }
}
