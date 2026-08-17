/// Island-Based Parallel Solver
///
/// Splits the physics world into connected components (islands).
/// Islands that are independent of each other are solved in parallel with Rayon by PhysicsWorld.
/// Islands that are not moving go to sleep and are skipped entirely.
use gizmo_physics_core::ContactManifold;
use gizmo_physics_core::BodyHandle;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// Island veri yapısı
// ─────────────────────────────────────────────────────────────────────────────

/// A single physics island — dynamic bodies that are in contact with one another
#[derive(Debug, Clone, PartialEq)]
pub struct Island {
    /// Bu island'a ait manifold indisleri (orijinal Vec'teki)
    pub manifold_indices: Vec<usize>,
    /// Sleep state: true once every body in the island is slow enough
    pub sleeping: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// IslandManager
// ─────────────────────────────────────────────────────────────────────────────

/// Namespace for the island partitioning routines — a zero-sized, stateless type whose
/// methods are all associated functions.
///
/// It deliberately keeps nothing between calls: the partition is recomputed from the
/// current contact set every substep, so islands never go stale when bodies are added,
/// removed or re-indexed. Nothing here touches body state; the caller decides what to do
/// with the returned grouping.
#[derive(Debug, Clone, Copy, Default)]
pub struct IslandManager;

impl IslandManager {
    /// Partition manifolds into connected components (islands).
    /// Two manifolds belong to the same island ↔ they share a dynamic body.
    ///
    /// Algorithm: union-find (path compression + rank)
    pub fn build_islands(
        manifolds: &[ContactManifold],
        entity_is_dynamic: &impl Fn(BodyHandle) -> bool,
    ) -> Vec<Island> {
        if manifolds.is_empty() {
            return Vec::new();
        }

        let n = manifolds.len();

        // ── Union-Find veri yapısı ────────────────────────────────────────────
        let mut parent: Vec<usize> = (0..n).collect();
        let mut rank: Vec<u8> = vec![0; n];

        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]]; // Path splitting / compression
                i = parent[i];
            }
            i
        }

        fn union(parent: &mut [usize], rank: &mut [u8], a: usize, b: usize) {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra == rb {
                return;
            }
            // Rank-based union
            match rank[ra].cmp(&rank[rb]) {
                std::cmp::Ordering::Less => parent[ra] = rb,
                std::cmp::Ordering::Greater => parent[rb] = ra,
                std::cmp::Ordering::Equal => {
                    parent[rb] = ra;
                    rank[ra] += 1;
                }
            }
        }

        // Her dinamik entity için hangi manifoldlara ait olduğunu bul
        let mut entity_to_manifolds: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, m) in manifolds.iter().enumerate() {
            if entity_is_dynamic(m.entity_a) {
                entity_to_manifolds
                    .entry(m.entity_a.id())
                    .or_default()
                    .push(i);
            }
            if entity_is_dynamic(m.entity_b) {
                entity_to_manifolds
                    .entry(m.entity_b.id())
                    .or_default()
                    .push(i);
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

        // DETERMINIZM: `groups` bir HashMap; `into_values()` sırası süreçten sürece
        // değişir (hash randomization). Island'lar arası çözüm sırası bundan etkilenmesin
        // diye her island'ın indislerini sırala + island'ları en küçük indise göre sırala.
        // (Island'lar AYRIK olduğundan fizik sonucu sıradan bağımsız; bu, süreçler-arası
        // tutarlı sıra/warm-start eşlemesi ve tekrarlanabilir hata ayıklama içindir.)
        let mut islands: Vec<Island> = groups
            .into_values()
            .map(|mut indices| {
                indices.sort_unstable();
                Island {
                    manifold_indices: indices,
                    sleeping: false,
                }
            })
            .collect();
        islands
            .sort_unstable_by_key(|isl| isl.manifold_indices.first().copied().unwrap_or(usize::MAX));
        // Hot path (per substep) → trace. Shows how the contact set partitioned into
        // independently-solvable islands (drives the parallel solve + sleep decisions).
        tracing::trace!(
            manifold_count = n,
            island_count = islands.len(),
            "contact islands built"
        );
        islands
    }

    /// Split manifolds by island group — each island gets its own manifold Vec
    pub fn split_manifolds(
        manifolds: Vec<ContactManifold>,
        islands: &[Island],
    ) -> Vec<Vec<ContactManifold>> {
        let mut manifold_opts: Vec<Option<ContactManifold>> =
            manifolds.into_iter().map(Some).collect();

        islands
            .iter()
            .map(|island| {
                let mut indices = island.manifold_indices.clone();
                indices.sort_unstable(); // Deterministik çözüm sırası için sort
                indices
                    .into_iter()
                    .filter_map(|i| manifold_opts[i].take())
                    .collect()
            })
            .collect()
    }

    /// Decide whether the island may go to sleep.
    /// If the total impulse across all its contact points is under the threshold → sleep
    pub fn should_sleep(manifolds: &[ContactManifold], impulse_threshold: f32) -> bool {
        if manifolds.is_empty() {
            return false;
        }

        manifolds.iter().all(|m| {
            m.lifetime > 3 && // En az birkaç frame aktif olmalı (warm-up)
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

/// Profiling counters for the last completed `PhysicsWorld::step`.
///
/// Purely observational: nothing here is read back by the simulation, so the measurement
/// cannot perturb the result and these values are excluded from the world's serialized
/// state. They are therefore *not* part of the replay/rollback state either — two runs
/// that hash identically will still report different timings.
///
/// **Accumulation window.** One render frame may run many fixed substeps. The timers and
/// the contact/island counts are zeroed at the start of a step and summed over every
/// substep of that step; the body counts are sampled once, after the last substep. A step
/// whose accumulator had not filled therefore reports zeroed timers with fresh body
/// counts, while a step that returns before simulating anything — paused, or servicing a
/// rewind — leaves the whole struct holding the previous frame's numbers.
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PhysicsMetrics {
    /// Wall-clock milliseconds spent updating broadphase proxies and gathering candidate
    /// pairs.
    pub broadphase_ms: f32,
    /// Wall-clock milliseconds spent turning candidate pairs into contact manifolds,
    /// including the collision/trigger event bookkeeping that runs with it.
    pub narrowphase_ms: f32,
    /// Wall-clock milliseconds spent in the constraint solve — contacts *and* joints are
    /// one stage here, so a heavy ragdoll shows up in this number and not elsewhere.
    pub solver_ms: f32,
    /// Of [`solver_ms`](Self::solver_ms): support-ordering the island and finding its depth.
    ///
    /// These four break the solver quarter down at function granularity. They are a strict
    /// subset of `solver_ms` and need not sum to it — what is left over is the work between
    /// them (island assembly, the position-correction write-back).
    pub solver_order_ms: f32,
    /// Of [`solver_ms`](Self::solver_ms): building the per-contact constraint rows.
    pub solver_prepare_ms: f32,
    /// Of [`solver_ms`](Self::solver_ms): the biased sweeps, the main iteration.
    pub solver_sweep_ms: f32,
    /// Of [`solver_ms`](Self::solver_ms): the relax pass that applies restitution.
    pub solver_relax_ms: f32,
    /// Of [`narrowphase_ms`](Self::narrowphase_ms): the collision maths itself — shape
    /// dispatch, SAT, GJK.
    ///
    /// Worth having beside its twin below: `docs/ENGINE.md` §7 records that box-box SAT is
    /// ~3 % of a frame, and the pair of numbers is what keeps that claim checkable rather
    /// than remembered.
    pub narrowphase_dispatch_ms: f32,
    /// Of [`narrowphase_ms`](Self::narrowphase_ms): everything around the maths — material
    /// combination, warm-starting, the contact cache, collision events.
    pub narrowphase_manifold_ms: f32,
    /// Wall-clock milliseconds spent in *both* integration stages: velocity integration
    /// before the solver and position integration plus CCD resolution after it.
    pub integration_ms: f32,
    /// Islands built, summed over the step's substeps — with several substeps this is a
    /// multiple of the islands actually present, not the current partition size.
    pub island_count: usize,
    /// Dynamic bodies asleep after the last substep. Static and kinematic bodies are never
    /// counted even when they carry the sleeping flag — which static bodies built through
    /// `RigidBody::new_static` do from the start.
    pub sleeping_count: usize,
    /// Contact *points* — not manifolds — fed to the solver.
    pub contact_count: usize,
    /// Bodies registered in the world after the last substep, all body types included.
    pub body_count: usize,
    /// Biased contact sweeps the solver actually ran, summed over the step's islands *and*
    /// substeps. Divide by [`island_count`](Self::island_count) for a per-island average.
    ///
    /// This is the *effective* sweep count the depth-adaptive policy handed out, which is not
    /// [`ConstraintSolver::iterations`](crate::solver::ConstraintSolver::iterations): a deep
    /// island gets `min(96, max(iterations, max(28, 3·depth/2)))` instead, so the configured
    /// value is a floor the caller cannot lower unless
    /// [`adaptive_iterations`](crate::solver::ConstraintSolver::adaptive_iterations) is off.
    ///
    /// Read it when you want to know where solver time is going — the sweep count, not the
    /// per-contact cost, is what makes a large contact island expensive — and when you want to
    /// know whether a change to the sweep policy touched a given scene at all. A test that
    /// passes without this number moving has not exercised the policy.
    pub solver_sweeps: usize,
    /// Largest island support depth seen this step, over islands and substeps.
    ///
    /// Support depth is the BFS eccentricity of the island's contact graph rooted at its
    /// static/kinematic anchors — or, for an island with no anchor at all, at its lowest-indexed
    /// body. It is the sole input to the adaptive sweep count above.
    ///
    /// Do not read it as a stack height. For a column it is the height, but for a
    /// mutually-touching cluster it is the cluster's diameter, and for a wide block it is the
    /// height while saying nothing about the mass riding on it.
    pub max_island_depth: u32,
}

impl PhysicsMetrics {
    /// Emit the whole set as a single `tracing` event at debug level.
    ///
    /// Despite the name nothing is drawn and nothing is printed to stdout — the line only
    /// materialises if a subscriber is installed and enables debug for this crate.
    pub fn print_hud(&self) {
        tracing::debug!(
            island_count = self.island_count,
            sleeping_count = self.sleeping_count,
            contact_count = self.contact_count,
            body_count = self.body_count,
            solver_sweeps = self.solver_sweeps,
            max_island_depth = self.max_island_depth,
            broadphase_ms = self.broadphase_ms,
            narrowphase_ms = self.narrowphase_ms,
            solver_ms = self.solver_ms,
            integration_ms = self.integration_ms,
            "[Physics Metrics] Islands:{} Sleep:{} Contacts:{} Bodies:{} Sweeps:{} Depth:{} | Broad:{:.2}ms Narrow:{:.2}ms Solver:{:.2}ms Integrate:{:.2}ms",
            self.island_count, self.sleeping_count, self.contact_count, self.body_count,
            self.solver_sweeps, self.max_island_depth,
            self.broadphase_ms, self.narrowphase_ms, self.solver_ms, self.integration_ms,
        );
    }

    /// Sum of the four phase timers, in milliseconds.
    ///
    /// A lower bound on the real cost of the step rather than a measurement of it: only
    /// the four timed phases are included, so per-frame work outside them — the
    /// rewind-history snapshot, for one — is invisible here.
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
    use gizmo_physics_core::{ContactManifold, ContactPoint};
    use gizmo_physics_core::BodyHandle;
    use gizmo_math::Vec3;

    fn make_manifold(ea: u32, eb: u32) -> ContactManifold {
        let mut m = ContactManifold::new(BodyHandle::from_id(ea), BodyHandle::from_id(eb));
        m.contacts.push(ContactPoint {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            penetration: 0.01,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        });
        m
    }

    #[test]
    fn test_single_island() {
        // A-B ve B-C → tek island
        let manifolds = vec![make_manifold(1, 2), make_manifold(2, 3)];
        let is_dyn = |e: BodyHandle| e.id() != 0;
        let islands = IslandManager::build_islands(&manifolds, &is_dyn);
        assert_eq!(islands.len(), 1, "A-B and B-C should form one island");
        assert_eq!(islands[0].manifold_indices.len(), 2);
    }

    #[test]
    fn test_two_islands() {
        // A-B ve C-D → iki ayrı island
        let manifolds = vec![make_manifold(1, 2), make_manifold(3, 4)];
        let is_dyn = |e: BodyHandle| e.id() != 0;
        let islands = IslandManager::build_islands(&manifolds, &is_dyn);
        assert_eq!(islands.len(), 2, "A-B and C-D should form two islands");
    }

    #[test]
    fn test_empty_manifolds() {
        let is_dyn = |_: BodyHandle| true;
        let islands = IslandManager::build_islands(&[], &is_dyn);
        assert!(islands.is_empty());
    }

    #[test]
    fn test_sleeping_detection() {
        let mut m = make_manifold(1, 2);
        m.lifetime = 4;
        m.contacts[0].normal_impulse = 0.001; // Çok düşük
        assert!(IslandManager::should_sleep(&[m], 0.01));

        let mut m2 = make_manifold(1, 2);
        m2.lifetime = 4;
        m2.contacts[0].normal_impulse = 100.0; // Yüksek aktivite
        assert!(!IslandManager::should_sleep(&[m2], 0.01));
    }
}
