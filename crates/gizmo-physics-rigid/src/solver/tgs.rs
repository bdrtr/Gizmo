use super::ConstraintSolver;
use crate::components::{RigidBody, Velocity};
use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::ContactManifold;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(target_arch = "wasm32")]
use crate::parallel_compat::*;

/// Manifolds in an island from which its per-contact preparation runs on several threads
/// (`prepare_contacts`, `approach_speeds`, `record_sliding`): a 4000-sphere pile's one island
/// holds 5800, and these passes were a fifth of its solve, on one thread, on the step's critical
/// path. Below this the work is less than handing it out.
pub(super) const PARALLEL_MANIFOLDS: usize = 1024;
/// Manifolds a thread takes at a time in those passes.
pub(super) const MANIFOLDS_PER_CHUNK: usize = 256;

/// Per-contact quantities that are INVARIANT across all 24 TGS sweeps (the biased
/// iterations + relax passes) — hoisted out of the sweep and computed exactly once.
/// The sweep then does only the velocity/dp-dependent work. `acc_n`/`acc_t` are the
/// mutable impulse accumulators: seeded from the manifold, mutated in place each
/// sweep, and written back to the manifold after solving (warm-start continuity).
/// Every field is produced with the SAME expression the sweep used, so the result
/// is bit-identical to the pre-hoist solver. Contacts that the old sweep skipped
/// (`k_n < 1e-8`, or a manifold with two non-dynamic bodies) are simply not built.
pub(super) struct Prepared {
    pub(super) idx_a: usize,
    pub(super) idx_b: usize,
    pub(super) dyn_a: bool,
    pub(super) dyn_b: bool,
    inv_m_a: f32,
    inv_m_b: f32,
    inv_i_a: Mat3,
    inv_i_b: Mat3,
    r_a: Vec3,
    r_b: Vec3,
    normal: Vec3,
    k_n: f32,
    t1: Vec3,
    t2: Vec3,
    k_t1: f32,
    k_t2: f32,
    friction: f32,
    static_friction: f32,
    /// Coulomb-form bound on the rolling row, as a fraction of the normal impulse. Zero
    /// disables the row entirely, which is the default and the behaviour that existed
    /// before it.
    rolling: f32,
    restitution: f32,
    pen0: f32,
    vn0: f32,
    mid: usize,
    cid: usize,
    acc_n: f32,
    acc_t: Vec3,
    /// The contact's state at the end of the last step (`ContactPoint::sliding`): which friction
    /// budget this step's sweeps use.
    sliding: bool,
    /// Accumulated rolling (torque) impulse, kept for the same reason `acc_t` is: the
    /// Coulomb bound is on the *total* impulse over the sweep, not on each increment.
    acc_r: Vec3,
    /// The row's angular terms, worked out once here instead of in every sweep: `r×n`, `r×t₁`,
    /// `r×t₂` for each body and the same through its inverse inertia (`I⁻¹(r×n)`, …). A sweep then
    /// reads a velocity along a row as dot products and hands an impulse out as scaled sums — no
    /// 3×3 product and no cross product (Jolt keeps the same `I⁻¹(r×axis)` per row). The block
    /// solver's step reads only these.
    jac: Angular,
}

/// See [`Prepared::jac`].
#[derive(Clone, Copy)]
struct Angular {
    rn_a: Vec3,
    rn_b: Vec3,
    in_a: Vec3,
    in_b: Vec3,
    rt1_a: Vec3,
    rt1_b: Vec3,
    it1_a: Vec3,
    it1_b: Vec3,
    rt2_a: Vec3,
    rt2_b: Vec3,
    it2_a: Vec3,
    it2_b: Vec3,
    /// `rolling_lever` of the row.
    lever: f32,
}

/// Rolling-resistance row: oppose the pair's relative spin with a torque impulse, bounded
/// like friction by the normal impulse.
///
/// **Why sliding friction cannot do this.** In pure rolling the contact point is
/// instantaneously at rest, so a tangential Coulomb force does no work there however large
/// the coefficient — measured in `benchmarks/vs-rapier`, a thousand-sphere pile is still
/// rolling after fifty seconds and has spread to a 39.6 m radius. What stops a real ball is
/// the deformation of the surfaces under it, and that is this row.
///
/// It resists the whole relative angular velocity rather than only its component about the
/// normal: a ball rolling on a plane spins about a *tangential* axis, so a normal-only row
/// would leave exactly the case this exists for untouched.
fn apply_rolling(p: &mut Prepared, velocities: &mut [Velocity]) {
    p.acc_r = super::rolling_row(
        velocities,
        (p.idx_a, p.idx_b),
        (p.dyn_a, p.dyn_b),
        (p.inv_i_a, p.inv_i_b),
        (p.rolling, super::rolling_lever(p.r_a, p.r_b, p.normal, (p.dyn_a, p.dyn_b))),
        p.acc_n,
        p.acc_r,
    );
}

/// One contact manifold's contiguous run in `prepared`, plus its precomputed N×N normal
/// Delassus matrix `a` (constant across sweeps under frozen anchors). Used by the block
/// solver to resolve the manifold's coplanar normal impulses jointly.
pub(super) struct BlockGroup {
    pub(super) start: usize,
    pub(super) n: usize,
    a: [[f32; 4]; 4],
}

impl ConstraintSolver {
    /// Adaptive iterations (block solver): minimum sweeps for a deep island (D≥5).
    ///
    /// **28 → 16 (2026-08-06), measured.** The old value was calibrated against an instability
    /// that has since been fixed at its root — partial island sleep and the degenerate
    /// single-point manifold — and it was compensating for both. It also exceeded the default
    /// `iterations` of 20, so a caller could not lower the sweep count of a deep island at all,
    /// which docs/ENGINE.md recorded as a defect.
    ///
    /// What the ensembles say now (`tests/solver_quality.rs`): 1-wide towers of N = 16, 24 and 32
    /// stand at EVERY sweep count from 1 to 96 on three ground sizes — 72 cells, zero collapses,
    /// peak lean 0.0000 throughout — where before the fixes N=24 collapsed on all three grounds
    /// at 1, 4, 8 and 16 sweeps. Realistic piles (4×6×4, 8×6×8, 4×12×4) likewise stand at 4
    /// sweeps upward, with resting penetration reaching zero by 16 and peak lean differing by
    /// half a millimetre across the whole range.
    ///
    /// 16 rather than lower because the data still shows margin shrinking at the very bottom —
    /// an N=32 tower peaks at 0.486 m/s at one sweep against a 0.5 blow-up threshold, and 0.215
    /// at sixteen. The floor now sits below the default `iterations`, so it constrains nobody
    /// who has not already lowered that.
    pub(super) const BLOCK_ITERS_FLOOR: usize = 16;
    /// Cap on adaptive iterations — an extreme tower is bounded; short piles never reach it.
    pub(super) const BLOCK_ITERS_CAP: usize = 96;

    /// Max island contacts for the whole-chain DIRECT solve (dense O(n³) LCP). Above this,
    /// fall back to the iterative block path. Covers towers up to ~N64 (≈4 contacts/interface).
    const DIRECT_SOLVE_CAP: usize = 256;

    // ─────────────────────────────────────────────────────────────────────────
    // TGS SOFT ÇÖZÜCÜ (Temporal Gauss-Seidel, soft constraints)
    // ─────────────────────────────────────────────────────────────────────────
    //
    // Box2D v3 "Soft Step" uyarlaması. Engine zaten 240 Hz sabit substep'le döner
    // (her substep: collision + solve), yani temporal alt-adım ZATEN var. Eksik olan,
    // her substep içindeki temas çözümünün KALİTESİYDİ: ileri-SI + split-impulse,
    // yüksek-enerjili çarpan UZUN yığınlarda (n≥16) çarpma dürtüsünü tüm temaslar
    // boyunca yayamıyor → metastable yığın çöküyordu.
    //
    // Bu çözücü her substep'te:
    //   1) warm-start (önceki impulslar),
    //   2) BIASED soft solve (N iter) + iterasyonlar arası POZİSYON-DELTA entegrasyonu
    //      (gerçek TGS): normal kısıt kritik-sönümlü yay-damper gibi çözülür
    //      (biasRate/massScale/impulseScale, contact_hertz+damping'ten); her iterasyondan
    //      sonra pozisyon-delta'sı (dp) güncel hızla ilerletilir ve bir sonraki iterasyonun
    //      bias'ı GÜNCEL penetrasyonu görür → düzeltme yığın boyunca yayılır (asıl kazanç),
    //   3) RELAX (M iter, bias=0) + restitution: soft bias hızını temizler (dinlenme TEMİZ,
    //      uyuyabilir) ve taze (vn0 hızlı, pen0≥0) çarpmada sekmeyi sönümlü/hedef-hız olarak
    //      geri verir,
    //   4) pozisyon düzeltmesi = dp − (relaxed hız)·dt olarak DIŞARI yazılır (pipeline bunu
    //      transform'a uygular; hız kanalına relaxed+restitution yazılır).
    //
    // Böylece pozisyon penetrasyonu TGS-entegre dp ile düzelir ama kalıcı hıza sızmaz.
    // NOT: CCD-etkin gövde içeren island'lar bu yola GİRMEZ — speculative temaslar için
    // eski split-impulse yolu kullanılır (solve_contacts başında has_ccd dalı); TGS'in
    // dp/relax akışı yüksek-hızlı açılı çarpmalarda speculative clamp'le çatışabiliyor.
    #[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
    pub(super) fn solve_contacts_tgs(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        // What the relax sweeps changed for a body held only by speculative contacts, applied after
        // the position integration (see where it is filled).
        bounces: &mut [(Vec3, Vec3)],
        // Bare `FxHashMap` and not the opaque `EntityIndexMap`: `pub(super)` keeps this off
        // the public surface, so it is not a `rustc-hash` leak (docs/ENGINE.md §4). The
        // public caller (`solve_contacts`) unwraps with the `pub(crate)` `raw()`.
        entity_index_map: &crate::world::EntityIndexMap,
        // Distinct GLOBAL body indices referenced by THIS island's manifolds. All the
        // per-body scratch (dp) and loops are sized/iterated by this island-local set
        // instead of the full world, so cost is O(island_bodies·iters), not
        // O(n_bodies·iters) per island — the difference between O(Σislands) and
        // O(n_islands·n_bodies) across the frame.
        island_bodies: &[usize],
        // Island support depth (max graph distance from an anchor) — gates the whole-chain
        // direct solve (only tall, bucklable chains use it).
        island_depth: u32,
        // Effective biased-iteration count for THIS island (adaptive: scaled with support
        // depth when the block solver is on, else `self.iterations`).
        n_iterations: usize,
        dt: f32,
        // The island's joints, swept before each contact sweep (`super::BetweenSweeps`).
        mut between: super::BetweenSweeps<'_>,
    ) {
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        let coeffs = self.soft_coefficients(dt, inv_dt);
        let (vn0_off, vn0) = Self::approach_speeds(manifolds, rigid_bodies, transforms, velocities, entity_index_map);
        self.warm_start_tgs(manifolds, rigid_bodies, transforms, velocities, entity_index_map);
        let mut real_bodies = Self::real_contact_bodies(manifolds, entity_index_map);
        // A joint's bodies take the position integration too: its correction is made in the biased
        // sweeps and kept in the position, not in the velocity (`super::IslandJoints::bodies`).
        if let Some(joints) = &between {
            real_bodies.extend(joints.bodies.iter().copied());
        }

        let h = if n_iterations > 0 {
            dt / n_iterations as f32
        } else {
            dt
        };

        let mut prepared = Self::prepare_contacts(manifolds, rigid_bodies, transforms, entity_index_map, &vn0_off, &vn0);
        let block_groups = if self.block_solver { self.block_groups(&prepared) } else { Vec::new() };

        // ── Whole-chain DIRECT solve setup ──
        // For a tall, bucklable chain (support-depth ≥ 5) small enough to solve densely,
        // assemble the FULL island Delassus matrix and solve all normal impulses jointly
        // each sweep → the inter-manifold support coupling is resolved EXACTLY (not left to
        // iterate), eliminating the tall-tower buckling the iterative path can't robustly fix.
        // A[i][j] = coupling of contacts i,j through any shared body: for each shared body X,
        // s_i(X)·s_j(X)·[invM_X (n_i·n_j) + (invI_X (r_{X,j}×n_j))·(r_{X,i}×n_i)], with sign
        // −1 on the contact's body A and +1 on body B. Symmetric; regularised on the diagonal
        // (coplanar rank-deficiency, as in the per-manifold block).
        let nprep = prepared.len();
        let use_direct = self.direct_chain_solve
            && self.block_solver
            && island_depth >= 5
            && (2..=Self::DIRECT_SOLVE_CAP).contains(&nprep);
        let island_a = if use_direct { self.island_delassus(&prepared) } else { Vec::new() };
        let direct = use_direct.then_some(island_a.as_slice());
        // A large island without joints is swept colour by colour on several threads
        // (`super::parallel`). Decided by the island's size alone, never by the thread count.
        let coloring = (self.block_solver
            && direct.is_none()
            && between.is_none()
            && self.parallel_island_blocks > 0
            && block_groups.len() >= self.parallel_island_blocks)
            .then(|| super::parallel::color_blocks(&prepared, &block_groups, island_bodies, velocities.len()));
        // The bodies whose position delta the biased sweeps integrate.
        let integrate: Vec<usize> = island_bodies
            .iter()
            .copied()
            .filter(|i| real_bodies.contains(i) && rigid_bodies[*i].is_dynamic())
            .collect();

        // Per-body position-delta scratch (dp), indexed by GLOBAL body index but sized once
        // per thread and RESET only for `island_bodies` — no per-island full-world allocation.
        // Islands are disjoint (a body is in exactly one island), so resetting just this
        // island's entries fully isolates the reused thread-local across islands/substeps.
        thread_local! {
            static DP: std::cell::RefCell<Vec<(Vec3, Vec3)>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        // Taken out and put back rather than borrowed across the solve, for the reason the pipeline's
        // island buffers are (`solve_island_contacts`): a coloured solve waits in a `rayon::scope`.
        let mut dp = DP.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        {
            if dp.len() < velocities.len() {
                dp.resize(velocities.len(), (Vec3::ZERO, Vec3::ZERO));
            }
            for &i in island_bodies {
                dp[i] = (Vec3::ZERO, Vec3::ZERO);
            }

            // ── 3) BIASED soft solve + iterasyon-arası pozisyon entegrasyonu ──
            // Her iterasyondan sonra pozisyon-delta'sı güncel hızla ilerletilir; bir sonraki
            // iterasyonun bias'ı GÜNCEL penetrasyonu görür → düzeltme yığın boyunca yayılır
            // (SI'nin yapamadığı; uzun/çarpan yığınları ayakta tutan temel mekanizma).
            let _sweep = crate::profile::Scope::new(&crate::profile::PHASES.sweep);
            if let Some(coloring) = &coloring {
                self.sweep_colored(super::parallel::ColoredSweeps {
                    prepared: &mut prepared,
                    groups: &block_groups,
                    coloring,
                    velocities,
                    dp: dp.as_mut_slice(),
                    integrate: &integrate,
                    n_iterations,
                    h,
                    bias_rate: coeffs.0,
                    inv_dt,
                });
            }
            for iter in 0..if coloring.is_some() { 0 } else { n_iterations } {
                // The joints first and the contacts after, in every sweep (as Box2D v3 orders them):
                // the last word goes to the contacts, which have no correction for a small overlap
                // (`slop`) — a joint's error is corrected by its own bias on the next substep.
                if let Some(between) = between.as_mut() {
                    (between.sweep)(velocities, dp.as_slice(), false);
                }
                self.sweep(&mut prepared, &block_groups, direct, velocities, dp.as_slice(), iter % 2 == 1, Some(coeffs), inv_dt);
                for &i in &integrate {
                    dp[i].0 += velocities[i].linear * h;
                    dp[i].1 += velocities[i].angular * h;
                }
            }

            // ── 4) RELAX (bias=0) — soft bias hızını temizle + restitution ──
            drop(_sweep);
            // A dynamic body held only by speculative contacts takes no TGS position delta: it moves
            // by its velocity, over the whole substep. That must be the velocity the biased sweeps
            // left — the one that brings it up to what it is about to meet — and what the relax
            // sweeps change, the bounce above all, is handed back for after it has got there
            // (`bounces`), as the split-impulse path does. A bounce taken into the velocity sent it
            // back from where it began the substep: a ball struck in a row gave all its speed away
            // without moving, and the one it struck went through the next in the same substep.
            let speculative_only: Vec<(usize, Velocity)> = island_bodies
                .iter()
                .filter(|&&i| rigid_bodies[i].is_dynamic() && !real_bodies.contains(&i))
                .map(|&i| (i, velocities[i]))
                .collect();
            let _relax = crate::profile::Scope::new(&crate::profile::PHASES.relax);
            for iter in 0..if coloring.is_some() { 0 } else { self.relax_iterations } {
                if let Some(between) = between.as_mut() {
                    (between.sweep)(velocities, dp.as_slice(), true);
                }
                self.sweep(&mut prepared, &block_groups, direct, velocities, dp.as_slice(), iter % 2 == 1, None, inv_dt);
            }

            // ── 5) Pozisyon düzeltmesi = dp − (relaxed hız)·dt ──
            // dp, biased çözümün TGS-entegre GERÇEK pozisyon değişimidir (penetrasyon
            // düzeltmesi yığın boyunca yayılmış). position_integration relaxed·dt ekleyecek;
            // farkı burada ekleyince toplam = dp olur → penetrasyon düzelir, hız temiz kalır
            // (resting jitter yok, uyuyabilir). Restitution relax içinde uygulandığından
            // sekme hızı `velocities`'tedir; çıkarılması sekme yer-değişimini bu kareye değil
            // SONRAKİ kareye taşır (kararlı). Yalnız aktif dinamik gövdelere uygulanır.
            for (i, swept) in speculative_only {
                bounces[i] = (velocities[i].linear - swept.linear, velocities[i].angular - swept.angular);
                velocities[i] = swept;
            }
            for &i in island_bodies {
                if real_bodies.contains(&i) && rigid_bodies[i].is_dynamic() {
                    let dlin = dp[i].0 - velocities[i].linear * dt;
                    let dang = dp[i].1 - velocities[i].angular * dt;
                    pos_corrections[i] = (dlin, dang);
                }
            }
        }
        DP.with(|cell| *cell.borrow_mut() = dp);

        // Impulse birikimcilerini manifold'a geri yaz (warm-start sürekliliği: bir sonraki
        // substep bu değerleri okur). Eski kod bunu her sweep'te yazıyordu; `prepared` bunları
        // bellekte tuttuğu için tek bir yazma yeterli, sonuç aynı.
        for p in &prepared {
            manifolds[p.mid].contacts[p.cid].normal_impulse = p.acc_n;
            manifolds[p.mid].contacts[p.cid].tangent_impulse = p.acc_t;
        }
    }

    /// The soft contact's coefficients for a substep of `dt`: `(bias_rate, mass_scale,
    /// impulse_scale)`.
    fn soft_coefficients(&self, dt: f32, inv_dt: f32) -> (f32, f32, f32) {
        // ── Soft constraint katsayıları (Box2D v3 formülü) ──
        // hertz'i substep oranının çeyreğine clamp'le (ω·dt makul kalsın).
        let hertz = self.contact_hertz.min(0.25 * inv_dt);
        let zeta = self.contact_damping_ratio;
        let omega = 2.0 * std::f32::consts::PI * hertz;
        let denom = 2.0 * zeta + dt * omega;
        let c = dt * omega * denom;
        let bias_rate = if denom > 1e-9 { omega / denom } else { 0.0 };
        let mass_scale = if c > 0.0 { c / (1.0 + c) } else { 1.0 };
        let impulse_scale = if c > 0.0 { 1.0 / (1.0 + c) } else { 0.0 };
        (bias_rate, mass_scale, impulse_scale)
    }

    /// Every contact's approach speed before the warm start, for restitution, at
    /// `vn0[off[manifold] + contact]`.
    #[allow(clippy::needless_range_loop)]
    fn approach_speeds(
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &[Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
    ) -> (Vec<usize>, Vec<f32>) {
        let com_of = |idx: usize| -> Vec3 {
            transforms[idx].position
                + transforms[idx]
                    .rotation
                    .mul_vec3(rigid_bodies[idx].center_of_mass)
        };

        // ── 0) Restitution için başlangıç yaklaşma hızı (warm-start ÖNCESİ) ──
        // Tek düz tampon + offset tablosu. Eskiden `map(|m| vec![0.0f32; m.contacts.len()])`
        // ile manifold başına bir iç `Vec` ayrılıyordu, yani ada başına substep başına
        // `1 + N_manifold` ayırma — narrowphase'in tamamıyla aynı payda, ama kare zamanının
        // büyük çoğunluğunu tutan fazda. Saf scratch: yalnız aşağıda bir kez yazılıyor,
        // `Prepared` kurulurken bir kez okunuyor.
        //
        // `ArrayVec` OLAMAZ: `m.contacts.len()` capped değil (bkz. `ContactManifold::contacts`
        // — box–plane 8 köşe verebilir, compound sınırsız) ve `ArrayVec` taşmada büyümek
        // yerine panikler; step döngüsünden çağrılan bir çözücüde bu çökme demek.
        let mut vn0_off: Vec<usize> = Vec::with_capacity(manifolds.len() + 1);
        vn0_off.push(0);
        let mut vn0_total = 0usize;
        for m in manifolds.iter() {
            vn0_total += m.contacts.len();
            vn0_off.push(vn0_total);
        }
        let mut vn0 = vec![0.0f32; vn0_total];
        let speeds = |mid: usize, out: &mut [f32]| {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => return,
            };
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            for cid in 0..manifolds[mid].contacts.len() {
                let ct = manifolds[mid].contacts[cid];
                let r_a = ct.point - com_a;
                let r_b = ct.point - com_b;
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                out[cid] = (vb - va).dot(ct.normal);
            }
        };
        // In parallel for a big island, each manifold into its own stretch of `vn0`.
        if manifolds.len() >= PARALLEL_MANIFOLDS {
            let mut stretches: Vec<(usize, &mut [f32])> = Vec::with_capacity(manifolds.len());
            let mut rest: &mut [f32] = &mut vn0;
            for mid in 0..manifolds.len() {
                let (this, tail) = std::mem::take(&mut rest).split_at_mut(vn0_off[mid + 1] - vn0_off[mid]);
                stretches.push((mid, this));
                rest = tail;
            }
            stretches.par_iter_mut().with_min_len(MANIFOLDS_PER_CHUNK).for_each(|(mid, out)| speeds(*mid, out));
        } else {
            for mid in 0..manifolds.len() {
                speeds(mid, &mut vn0[vn0_off[mid]..vn0_off[mid + 1]]);
            }
        }
        (vn0_off, vn0)
    }

    /// The TGS path's warm start: last step's impulses, scaled by `warm_start_factor`.
    fn warm_start_tgs(
        &self,
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        entity_index_map: &crate::world::EntityIndexMap,
    ) {
        for manifold in manifolds {
            let Some(super::ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b }) =
                super::ContactPair::of(manifold, rigid_bodies, transforms, entity_index_map)
            else {
                continue;
            };
            for contact in &manifold.contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;
                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;
                if dyn_a {
                    velocities[idx_a].linear -= (wn + wt) * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear += (wn + wt) * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }
    }

    /// The bodies with a real (penetrating, `pen0 ≥ 0`) contact: only they take the TGS
    /// position delta.
    fn real_contact_bodies(manifolds: &[ContactManifold], entity_index_map: &crate::world::EntityIndexMap) -> rustc_hash::FxHashSet<usize> {
        // ── 2) GERÇEK (penetran, pen0≥0) teması olan gövdeler ──
        // Yalnız bunlar TGS pozisyon düzeltmesi (dp) alır. Sadece SPECULATIVE (gap, pen0<0)
        // teması olan gövdeler (ör. CCD mermisi) eski yolu kullanır: biased clamp velocity +
        // position_integration taşır; relax/dp UYGULANMAZ (yoksa hız sıfırlanıp donar ya da
        // dp taşıp tüneller). Eski `active` (= manifold'da geçen tüm gövdeler) artık gereksiz:
        // `island_bodies` zaten tam olarak o küme. `has_real` da full-world bit-vektörü yerine
        // ada-boyutu küçük bir kümedir.
        let mut real_bodies: rustc_hash::FxHashSet<usize> = rustc_hash::FxHashSet::default();
        for m in manifolds.iter() {
            if m.contacts.iter().any(|c| c.penetration >= 0.0) {
                if let (Some(&a), Some(&b)) = (
                    entity_index_map.get(&m.entity_a.id()),
                    entity_index_map.get(&m.entity_b.id()),
                ) {
                    real_bodies.insert(a);
                    real_bodies.insert(b);
                }
            }
        }
        real_bodies
    }

    /// Everything about a contact that does not change between sweeps, computed once
    /// (`Prepared`), with its impulse accumulators seeded from the manifold.
    #[allow(clippy::needless_range_loop)]
    fn prepare_contacts(
        manifolds: &[ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        entity_index_map: &crate::world::EntityIndexMap,
        vn0_off: &[usize],
        vn0: &[f32],
    ) -> Vec<Prepared> {
        // ── Per-contact SABİT precompute (hoist) ──
        // sweep'ler arası DEĞİŞMEYEN her şeyi (idx / inv_m / inv_i / com→r / normal / k_n /
        // t1,t2 / k_t) bir KEZ hesapla; 24 sweep aynı `prepared` dizisini yeniden kullanır.
        // Değerler eski sweep'le BİREBİR aynı ifadelerle üretildiğinden sonuç bit-identical
        // (davranış korunur — tgs_hash_check oracle'ı ile doğrulandı). Eski `continue`
        // davranışı korunur: iki-statik manifold ve k_n<1e-8 temaslar hiç eklenmez, dolayısıyla
        // her sweep'te atlanmış olurlar. Impulse birikimcileri (acc_n/acc_t) manifold'dan
        // tohumlanır, sweep'lerde yerinde güncellenir, sonda geri yazılır.
        let _prep = crate::profile::Scope::new(&crate::profile::PHASES.prepare);
        let fill = |mids: std::ops::Range<usize>, prepared: &mut Vec<Prepared>| {
        for mid in mids {
            let Some(super::ContactPair { idx_a, idx_b, inv_m_a, inv_m_b, inv_i_a, inv_i_b, dyn_a, dyn_b, com_a, com_b }) =
                super::ContactPair::of(&manifolds[mid], rigid_bodies, transforms, entity_index_map)
            else {
                continue;
            };
            if !dyn_a && !dyn_b {
                continue;
            }
            let friction = manifolds[mid].friction;
            let static_friction = manifolds[mid].static_friction;
            let restitution = manifolds[mid].restitution;
            let rolling = manifolds[mid].rolling_friction;
            let n_contacts = manifolds[mid].contacts.len();
            for cid in 0..n_contacts {
                let ct = manifolds[mid].contacts[cid];
                let normal = ct.normal;
                let r_a = ct.point - com_a;
                let r_b = ct.point - com_b;
                let r_a_x_n = r_a.cross(normal);
                let r_b_x_n = r_b.cross(normal);
                let k_n = inv_m_a
                    + inv_m_b
                    + inv_i_a.mul_vec3(r_a_x_n).dot(r_a_x_n)
                    + inv_i_b.mul_vec3(r_b_x_n).dot(r_b_x_n);
                if k_n < 1e-8 {
                    continue;
                }
                let (t1, t2) = {
                    let a = if normal.x.abs() > 0.9 {
                        Vec3::new(0.0, 1.0, 0.0).cross(normal)
                    } else {
                        Vec3::new(1.0, 0.0, 0.0).cross(normal)
                    }
                    .normalize();
                    (a, normal.cross(a))
                };
                let eff_mass = |taxis: Vec3| -> f32 {
                    let rxt_a = r_a.cross(taxis);
                    let rxt_b = r_b.cross(taxis);
                    inv_m_a
                        + inv_m_b
                        + inv_i_a.mul_vec3(rxt_a).dot(rxt_a)
                        + inv_i_b.mul_vec3(rxt_b).dot(rxt_b)
                };
                let k_t1 = eff_mass(t1);
                let k_t2 = eff_mass(t2);
                prepared.push(Prepared {
                    idx_a,
                    idx_b,
                    dyn_a,
                    dyn_b,
                    inv_m_a,
                    inv_m_b,
                    inv_i_a,
                    inv_i_b,
                    r_a,
                    r_b,
                    normal,
                    k_n,
                    t1,
                    t2,
                    k_t1,
                    k_t2,
                    friction,
                    static_friction,
                    rolling,
                    restitution,
                    pen0: ct.penetration,
                    vn0: vn0[vn0_off[mid] + cid],
                    mid,
                    cid,
                    acc_n: ct.normal_impulse,
                    acc_t: ct.tangent_impulse,
                    sliding: ct.sliding,
                    // Not warm-started from the contact: the manifold carries no rolling
                    // impulse field, so the row starts cold each substep. Rolling resistance
                    // is a slow, dissipative effect rather than a stiff constraint, so the
                    // cost of a cold start is a fraction of one substep's decay.
                    acc_r: Vec3::ZERO,
                    jac: {
                        let (rt1_a, rt1_b, rt2_a, rt2_b) = (r_a.cross(t1), r_b.cross(t1), r_a.cross(t2), r_b.cross(t2));
                        Angular {
                            rn_a: r_a_x_n,
                            rn_b: r_b_x_n,
                            in_a: inv_i_a.mul_vec3(r_a_x_n),
                            in_b: inv_i_b.mul_vec3(r_b_x_n),
                            rt1_a,
                            rt1_b,
                            it1_a: inv_i_a.mul_vec3(rt1_a),
                            it1_b: inv_i_b.mul_vec3(rt1_b),
                            rt2_a,
                            rt2_b,
                            it2_a: inv_i_a.mul_vec3(rt2_a),
                            it2_b: inv_i_b.mul_vec3(rt2_b),
                            lever: super::rolling_lever(r_a, r_b, normal, (dyn_a, dyn_b)),
                        }
                    },
                });
            }
        }
        };
        // A big island's contacts are prepared in parallel, in chunks of manifolds whose results
        // are joined in order: the same rows, the same arithmetic, on as many threads as there are.
        let mut prepared: Vec<Prepared> = Vec::with_capacity(manifolds.len());
        if manifolds.len() >= PARALLEL_MANIFOLDS {
            let parts: Vec<Vec<Prepared>> = (0..manifolds.len().div_ceil(MANIFOLDS_PER_CHUNK))
                .into_par_iter()
                .map(|c| {
                    let mids = c * MANIFOLDS_PER_CHUNK..((c + 1) * MANIFOLDS_PER_CHUNK).min(manifolds.len());
                    let mut part = Vec::with_capacity(mids.len() * 2);
                    fill(mids, &mut part);
                    part
                })
                .collect();
            for part in parts {
                prepared.extend(part);
            }
        } else {
            fill(0..manifolds.len(), &mut prepared);
        }
        prepared
    }

    /// The block solver's groups: each manifold's contacts in blocks of at most four, with their
    /// regularised normal Delassus matrix.
    #[allow(clippy::needless_range_loop)]
    fn block_groups(&self, prepared: &[Prepared]) -> Vec<BlockGroup> {
        // ── Block-solver groups (Stage: manifold block normal solve) ──
        // Each manifold's contacts are CONTIGUOUS in `prepared` (built mid-outer, cid-inner),
        // all sharing the same body pair. Precompute, once, the N×N normal Delassus matrix A
        // per group (constant across sweeps under frozen anchors): A[r][c] = change in contact
        // r's normal velocity per unit normal impulse at contact c. The biased/relax sweeps
        // then solve each group's normals JOINTLY (block LCP) so the inter-point tilt coupling
        // is exact — the restoring stiffness that keeps tall stacks from buckling.
        let mut groups: Vec<BlockGroup> = Vec::new();
        let mut i = 0usize;
        while i < prepared.len() {
            let mid = prepared[i].mid;
            let m_start = i;
            while i < prepared.len() && prepared[i].mid == mid {
                i += 1;
            }
            // Chunk this manifold's contacts into blocks of ≤4 so EVERY contact is
            // solved. Previously a single group with n = count.min(4) was pushed and the
            // walk jumped to the next manifold, so any manifold with >4 contacts
            // (compound / multi-collider bodies, or an unreduced box-plane's 8 corner
            // contacts) had its contacts beyond the 4th orphaned into NO group → they
            // received zero normal AND zero friction impulse → that side sank / tilted /
            // never settled. For the common ≤4-contact manifold this is byte-identical to
            // before (one group, same `start`/`n`/`a`), so single-box stacks are unchanged.
            let mut start = m_start;
            while start < i {
                let n = (i - start).min(4);
                let mut a = [[0.0f32; 4]; 4];
                for r in 0..n {
                    let pr = &prepared[start + r];
                    let ua_r = pr.r_a.cross(pr.normal);
                    let ub_r = pr.r_b.cross(pr.normal);
                    for c in 0..n {
                        let pc = &prepared[start + c];
                        let ua_c = pc.r_a.cross(pc.normal);
                        let ub_c = pc.r_b.cross(pc.normal);
                        let ndot = pr.normal.dot(pc.normal);
                        a[r][c] = (pr.inv_m_a + pr.inv_m_b) * ndot
                            + pr.inv_i_a.mul_vec3(ua_c).dot(ua_r)
                            + pr.inv_i_b.mul_vec3(ub_c).dot(ub_r);
                    }
                }
                // Tikhonov regularisation: a 4-coplanar-contact manifold over-determines
                // its 3 DOF (1 force + 2 tilt torques), so `a` is rank-deficient (singular).
                // Add a small diagonal (BLOCK_REG × mean-diagonal) so the redundant direction
                // yields a bounded, evenly-distributed impulse instead of a huge garbage one
                // from a near-zero pivot. Small enough to leave the physical (well-conditioned)
                // tilt-restoring modes stiff.
                if n >= 2 {
                    let mut mean_diag = 0.0f32;
                    for r in 0..n {
                        mean_diag += a[r][r];
                    }
                    mean_diag /= n as f32;
                    let reg = self.block_regularization * mean_diag;
                    for r in 0..n {
                        a[r][r] += reg;
                    }
                }
                groups.push(BlockGroup { start, n, a });
                start += n;
            }
        }
        groups
    }

    /// The whole island's normal Delassus matrix, `nprep × nprep` row-major and regularised on
    /// the diagonal, for the whole-chain direct solve.
    #[allow(clippy::needless_range_loop)]
    fn island_delassus(&self, prepared: &[Prepared]) -> Vec<f32> {
        let nprep = prepared.len();
        let mut a = vec![0.0f32; nprep * nprep];
        for i in 0..nprep {
            for j in i..nprep {
                let (pi, pj) = (&prepared[i], &prepared[j]);
                let mut sum = 0.0f32;
                let ends_i = [
                    (pi.idx_a, pi.r_a, pi.inv_m_a, pi.inv_i_a, -1.0f32),
                    (pi.idx_b, pi.r_b, pi.inv_m_b, pi.inv_i_b, 1.0f32),
                ];
                let ends_j = [
                    (pj.idx_a, pj.r_a, -1.0f32),
                    (pj.idx_b, pj.r_b, 1.0f32),
                ];
                for &(xi, r_i, inv_m, inv_i, s_i) in &ends_i {
                    for &(xj, r_j, s_j) in &ends_j {
                        if xi == xj {
                            let ndot = pi.normal.dot(pj.normal);
                            let ui = r_i.cross(pi.normal);
                            let uj = r_j.cross(pj.normal);
                            sum += s_i * s_j * (inv_m * ndot + inv_i.mul_vec3(uj).dot(ui));
                        }
                    }
                }
                a[i * nprep + j] = sum;
                a[j * nprep + i] = sum;
            }
        }
        // Diagonal regularisation (proportional) for the coplanar rank-deficiency.
        for i in 0..nprep {
            a[i * nprep + i] *= 1.0 + self.block_regularization;
        }
        a
    }

    /// One sweep of the island's contacts by the solver the island uses — the whole-chain direct
    /// solve (`direct`, the island's Delassus matrix), the manifold blocks, or contact by contact.
    /// Biased with `coeffs` (the soft contact's bias rate, mass and impulse scales); without, the
    /// relax pass, which applies restitution.
    #[allow(clippy::too_many_arguments)]
    fn sweep(
        &self,
        prepared: &mut [Prepared],
        block_groups: &[BlockGroup],
        direct: Option<&[f32]>,
        velocities: &mut [Velocity],
        dp: &[(Vec3, Vec3)],
        reverse: bool,
        coeffs: Option<(f32, f32, f32)>,
        inv_dt: f32,
    ) {
        let (relax, (bias_rate, mass_scale, impulse_scale)) = match coeffs {
            Some(coeffs) => (false, coeffs),
            None => (true, (0.0, 1.0, 0.0)),
        };
        if let Some(island_a) = direct {
            self.tgs_sweep_island(prepared, island_a, velocities, dp, relax, !relax, bias_rate, mass_scale, impulse_scale, inv_dt);
        } else if self.block_solver {
            self.tgs_sweep_block(prepared, block_groups, velocities, dp, relax, reverse, !relax, bias_rate, inv_dt);
        } else {
            self.tgs_sweep_prepared(prepared, velocities, dp, relax, reverse, !relax, bias_rate, mass_scale, impulse_scale, inv_dt);
        }
    }

    /// One TGS iteration sweep — the precomputed (`prepared`) version. For each contact it
    /// solves the normal row (with an optional soft bias) plus 2-tangent Coulomb friction;
    /// `use_bias=false` gives the rigid relax pass. The constants
    /// (`r_a/r_b/normal/k_n/t1/t2/k_t/inv_i…`) are ready in `prepared`, so only the part that
    /// varies with velocity/dp is computed here. Reverse order (`reverse`) is the exact reverse
    /// of the forward sequence — identical to the old code's (manifold-reversed +
    /// contact-reversed) order.
    #[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
    fn tgs_sweep_prepared(
        &self,
        prepared: &mut [Prepared],
        velocities: &mut [Velocity],
        // Gerçek TGS: gövde başına biriken pozisyon-delta'sı (lin, açısal-scaled-axis).
        dp: &[(Vec3, Vec3)],
        // Relax aşamasında restitution'u hedef-hız olarak uygula (sönümlü, kararlı).
        apply_restitution: bool,
        reverse: bool,
        use_bias: bool,
        bias_rate: f32,
        mass_scale: f32,
        impulse_scale: f32,
        inv_dt: f32,
    ) {
        let n = prepared.len();
        for k in 0..n {
            let ki = if reverse { n - 1 - k } else { k };
            let p = &mut prepared[ki];

            // Ankraj kolları (r_a/r_b): donmuş (p.r_a) ya da Stage 4b'de birikmiş dp
            // DÖNMESİYLE döndürülmüş. `disp_*` = temas noktasının bu substep'teki yer
            // değişimi (lineer + ankraj hareketi). Donmuş dalda `dp.1.cross(p.r_a)` ilk-
            // mertebe yaklaşımı korunur → default davranış BİT-AYNI.
            let (ra, rb, disp_a, disp_b) = if self.rotating_anchors {
                let ra = gizmo_math::Quat::from_scaled_axis(dp[p.idx_a].1).mul_vec3(p.r_a);
                let rb = gizmo_math::Quat::from_scaled_axis(dp[p.idx_b].1).mul_vec3(p.r_b);
                (ra, rb, dp[p.idx_a].0 + (ra - p.r_a), dp[p.idx_b].0 + (rb - p.r_b))
            } else {
                (
                    p.r_a,
                    p.r_b,
                    dp[p.idx_a].0 + dp[p.idx_a].1.cross(p.r_a),
                    dp[p.idx_b].0 + dp[p.idx_b].1.cross(p.r_b),
                )
            };

            // GÜNCEL penetrasyon (gerçek TGS): biriken pozisyon-delta'larıyla düzelt.
            let penetration = p.pen0 - (disp_b - disp_a).dot(p.normal);

            // ── Normal kısıt ──
            let va = velocities[p.idx_a].linear + velocities[p.idx_a].angular.cross(ra);
            let vb = velocities[p.idx_b].linear + velocities[p.idx_b].angular.cross(rb);
            let vel_norm = (vb - va).dot(p.normal);

            // Restitution hedef-hızı: yalnız RELAX aşamasında, GERÇEK temasta
            // (pen0≥0, speculative değil), eşiği aşan çarpmada — sönümlü/clamp'li.
            let target_vn = if apply_restitution
                && p.restitution > 0.0
                && (p.pen0 >= 0.0 || p.acc_n > 0.0)
                && p.vn0 < -self.restitution_velocity_threshold
            {
                -p.restitution * p.vn0
            } else {
                0.0
            };

            // Soft bias: penetrasyonda yay-damper gibi ayır; speculative boşlukta
            // (penetration<0) kapanmayı boşluk/dt ile sınırla; aksi halde bias yok.
            // A bounce is the whole target: a speculative contact that stopped a body short of the
            // surface would otherwise take the gap still open out of the bounce (Box2D applies
            // restitution in a pass of its own, without the separation).
            let (bias, m_scale, i_scale) = if target_vn > 0.0 {
                (0.0, 1.0, 0.0)
            } else if penetration < 0.0 {
                (penetration * inv_dt, 1.0, 0.0)
            } else if use_bias && penetration > self.slop {
                let b = (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity);
                (b, mass_scale, impulse_scale)
            } else {
                (0.0, 1.0, 0.0)
            };
            // `i_scale·acc_n` is NOT divided by `k_n`. It is a relaxation of the ACCUMULATED
            // impulse, not a velocity error to be converted into one, so it carries no mass
            // factor — this is Box2D v3's `-normalMass·massScale·(vn + bias) − impulseScale·λ`,
            // and the same conclusion `ConstraintSolver::soft_coefficients` reached for joints.
            //
            // Dividing it makes the recursion `λ_{n+1} = λ_n·(1 − i_scale/k_n) + …`, whose
            // factor leaves the unit disc once `m_eff > 2/i_scale` (≈34.6 at the shipped
            // softness). Contacts do not blow up the way the joint rows did, because
            // `max(0.0)` truncates every negative half-cycle — but the response stops being
            // mass-invariant, which a soft constraint parameterised by hertz and damping must
            // be. Measured on a box resting in 0.2 m of penetration, final height by mass:
            //   divided:     1 kg 0.4733 · 100 kg 0.4092 · 300 kg 0.4084 · 1000 kg 0.4872
            //   not divided: 0.471068 at every mass from 1 kg to 5000 kg
            // The divided column is both mass-dependent and non-monotonic; see
            // `tests/contact_soft_stability.rs`.
            let delta_n = m_scale * (target_vn - vel_norm + bias) / p.k_n - i_scale * p.acc_n;
            let new_acc_n = (p.acc_n + delta_n).max(0.0);
            let actual_n = new_acc_n - p.acc_n;
            p.acc_n = new_acc_n;

            let imp_n = p.normal * actual_n;
            if p.dyn_a {
                velocities[p.idx_a].linear -= imp_n * p.inv_m_a;
                velocities[p.idx_a].angular -= p.inv_i_a.mul_vec3(ra.cross(imp_n));
            }
            if p.dyn_b {
                velocities[p.idx_b].linear += imp_n * p.inv_m_b;
                velocities[p.idx_b].angular += p.inv_i_b.mul_vec3(rb.cross(imp_n));
            }

            // ── Sürtünme (2-tangent Coulomb cone) ── t1/t2/k_t1/k_t2 precompute'lu.
            let va2 = velocities[p.idx_a].linear + velocities[p.idx_a].angular.cross(ra);
            let vb2 = velocities[p.idx_b].linear + velocities[p.idx_b].angular.cross(rb);
            let rel2 = vb2 - va2;
            let acc_t1 = p.acc_t.dot(p.t1);
            let acc_t2 = p.acc_t.dot(p.t2);
            let mut new1 = if p.k_t1 > 1e-8 {
                acc_t1 - rel2.dot(p.t1) / p.k_t1
            } else {
                acc_t1
            };
            let mut new2 = if p.k_t2 > 1e-8 {
                acc_t2 - rel2.dot(p.t2) / p.k_t2
            } else {
                acc_t2
            };
            // Coulomb budget: μ_s for a contact at rest at the last step's end, μ_d for one sliding (`friction_limit`).
            let limit =
                self.friction_limit(p.sliding, p.static_friction, p.friction, new_acc_n);
            let mag = (new1 * new1 + new2 * new2).sqrt();
            if mag > limit && mag > 1e-12 {
                let s = limit / mag;
                new1 *= s;
                new2 *= s;
            }
            let imp_t = p.t1 * (new1 - acc_t1) + p.t2 * (new2 - acc_t2);
            p.acc_t = p.t1 * new1 + p.t2 * new2;
            if p.dyn_a {
                velocities[p.idx_a].linear -= imp_t * p.inv_m_a;
                velocities[p.idx_a].angular -= p.inv_i_a.mul_vec3(ra.cross(imp_t));
            }
            if p.dyn_b {
                velocities[p.idx_b].linear += imp_t * p.inv_m_b;
                velocities[p.idx_b].angular += p.inv_i_b.mul_vec3(rb.cross(imp_t));
            }
            apply_rolling(p, velocities);
        }
    }

    /// Block-solver sweep: for each manifold group, solve its coplanar NORMAL impulses
    /// JOINTLY (active-set LCP over the precomputed Delassus block) so the inter-point
    /// tilt coupling is exact — the restoring stiffness that keeps tall stacks from
    /// buckling — then solve friction sequentially per contact (unchanged). Frozen
    /// anchors (block_solver takes precedence over rotating_anchors). The normal solve
    /// is rigid (drives vn to bias/restitution target); the soft/relax distinction is
    /// carried by `use_bias` + the per-contact bias target, and `dp` still integrates
    /// position between biased iterations, so the overall soft-step behaviour is kept.
    ///
    /// Each group is solved by [`block_group_step`](Self::block_group_step) on a copy of its two
    /// bodies' velocities, the same step the large islands' coloured sweep
    /// ([`super::parallel`]) runs on several threads.
    #[allow(clippy::too_many_arguments)]
    fn tgs_sweep_block(
        &self,
        prepared: &mut [Prepared],
        groups: &[BlockGroup],
        velocities: &mut [Velocity],
        dp: &[(Vec3, Vec3)],
        apply_restitution: bool,
        reverse: bool,
        use_bias: bool,
        bias_rate: f32,
        inv_dt: f32,
    ) {
        let ng = groups.len();
        for gi in 0..ng {
            let g = &groups[if reverse { ng - 1 - gi } else { gi }];
            if g.n == 0 {
                continue;
            }
            let run = &mut prepared[g.start..g.start + g.n];
            let (ia, ib) = (run[0].idx_a, run[0].idx_b);
            let mut pair = [velocities[ia], velocities[ib]];
            self.block_group_step(g, run, &mut pair, (dp[ia], dp[ib]), apply_restitution, use_bias, bias_rate, inv_dt);
            velocities[ia] = pair[0];
            velocities[ib] = pair[1];
        }
    }

    /// One block group's step: its normal impulses jointly, then friction contact by contact.
    ///
    /// `run` is the group's contacts (`prepared[g.start..g.start + g.n]`), all of one manifold and
    /// so of one body pair; `pair` is that pair's velocities, `[a, b]`, and `dp_ab` their position
    /// deltas. Only the dynamic ends of `pair` are written. Taking the pair by value rather than
    /// the island's velocity buffer is what lets groups that share no dynamic body run at once.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn block_group_step(
        &self,
        g: &BlockGroup,
        run: &mut [Prepared],
        pair: &mut [Velocity; 2],
        (dp_a, dp_b): ((Vec3, Vec3), (Vec3, Vec3)),
        apply_restitution: bool,
        use_bias: bool,
        bias_rate: f32,
        inv_dt: f32,
    ) {
        let n = g.n;
        let mut rhs = [0.0f32; 4];
        let mut acc = [0.0f32; 4];
        let (lin, ang) = (pair[1].linear - pair[0].linear, (pair[0].angular, pair[1].angular));
        let (dlin, dang) = (dp_b.0 - dp_a.0, (dp_a.1, dp_b.1));
        for c in 0..n {
            let p = &run[c];
            let j = &p.jac;
            // (b's point − a's point)·n, from the rows' r×n: ω×r·n = ω·(r×n).
            let penetration = p.pen0 - (dlin.dot(p.normal) + dang.1.dot(j.rn_b) - dang.0.dot(j.rn_a));
            let vn = lin.dot(p.normal) + ang.1.dot(j.rn_b) - ang.0.dot(j.rn_a);
            let tvn = if apply_restitution
                && p.restitution > 0.0
                && (p.pen0 >= 0.0 || p.acc_n > 0.0)
                && p.vn0 < -self.restitution_velocity_threshold
            {
                -p.restitution * p.vn0
            } else {
                0.0
            };
            let bias = if tvn > 0.0 {
                0.0
            } else if penetration < 0.0 {
                penetration * inv_dt
            } else if use_bias && penetration > self.slop {
                (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity)
            } else {
                0.0
            };
            rhs[c] = tvn + bias - vn;
            acc[c] = p.acc_n;
        }

        let lambda = super::block::solve_normal_block(n, &g.a, &rhs, &acc);

        for c in 0..n {
            let p = &mut run[c];
            let delta = lambda[c] - p.acc_n;
            p.acc_n = lambda[c];
            if p.dyn_a {
                pair[0].linear -= p.normal * (delta * p.inv_m_a);
                pair[0].angular -= p.jac.in_a * delta;
            }
            if p.dyn_b {
                pair[1].linear += p.normal * (delta * p.inv_m_b);
                pair[1].angular += p.jac.in_b * delta;
            }
        }

        for p in run.iter_mut() {
            let j = &p.jac;
            let lin = pair[1].linear - pair[0].linear;
            let (wa, wb) = (pair[0].angular, pair[1].angular);
            let acc_t1 = p.acc_t.dot(p.t1);
            let acc_t2 = p.acc_t.dot(p.t2);
            let mut new1 = if p.k_t1 > 1e-8 {
                acc_t1 - (lin.dot(p.t1) + wb.dot(j.rt1_b) - wa.dot(j.rt1_a)) / p.k_t1
            } else {
                acc_t1
            };
            let mut new2 = if p.k_t2 > 1e-8 {
                acc_t2 - (lin.dot(p.t2) + wb.dot(j.rt2_b) - wa.dot(j.rt2_a)) / p.k_t2
            } else {
                acc_t2
            };
            let limit = self.friction_limit(p.sliding, p.static_friction, p.friction, p.acc_n);
            let mag = (new1 * new1 + new2 * new2).sqrt();
            if mag > limit && mag > 1e-12 {
                let s = limit / mag;
                new1 *= s;
                new2 *= s;
            }
            let (d1, d2) = (new1 - acc_t1, new2 - acc_t2);
            let imp_t = p.t1 * d1 + p.t2 * d2;
            p.acc_t = p.t1 * new1 + p.t2 * new2;
            if p.dyn_a {
                pair[0].linear -= imp_t * p.inv_m_a;
                pair[0].angular -= j.it1_a * d1 + j.it2_a * d2;
            }
            if p.dyn_b {
                pair[1].linear += imp_t * p.inv_m_b;
                pair[1].angular += j.it1_b * d1 + j.it2_b * d2;
            }
            p.acc_r = super::rolling_row(
                pair,
                (0, 1),
                (p.dyn_a, p.dyn_b),
                (p.inv_i_a, p.inv_i_b),
                (p.rolling, j.lever),
                p.acc_n,
                p.acc_r,
            );
        }
    }

    #[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
    fn tgs_sweep_island(
        &self,
        prepared: &mut [Prepared],
        island_a: &[f32],
        velocities: &mut [Velocity],
        dp: &[(Vec3, Vec3)],
        apply_restitution: bool,
        use_bias: bool,
        bias_rate: f32,
        mass_scale: f32,
        impulse_scale: f32,
        inv_dt: f32,
    ) {
        let n = prepared.len();
        if n == 0 {
            return;
        }
        let _ = (mass_scale, impulse_scale); // rigid solve (regularised A tames it)

        let mut rhs = vec![0.0f32; n];
        let mut acc = vec![0.0f32; n];
        let mut lambda = vec![0.0f32; n];
        for c in 0..n {
            let p = &prepared[c];
            let dp_a = dp[p.idx_a].0 + dp[p.idx_a].1.cross(p.r_a);
            let dp_b = dp[p.idx_b].0 + dp[p.idx_b].1.cross(p.r_b);
            let penetration = p.pen0 - (dp_b - dp_a).dot(p.normal);
            let va = velocities[p.idx_a].linear + velocities[p.idx_a].angular.cross(p.r_a);
            let vb = velocities[p.idx_b].linear + velocities[p.idx_b].angular.cross(p.r_b);
            let vn = (vb - va).dot(p.normal);
            let tvn = if apply_restitution
                && p.restitution > 0.0
                && (p.pen0 >= 0.0 || p.acc_n > 0.0)
                && p.vn0 < -self.restitution_velocity_threshold
            {
                -p.restitution * p.vn0
            } else {
                0.0
            };
            // A bounce is the whole target (see `tgs_sweep_prepared`).
            let bias = if tvn > 0.0 {
                0.0
            } else if penetration < 0.0 {
                penetration * inv_dt
            } else if use_bias && penetration > self.slop {
                (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity)
            } else {
                0.0
            };
            rhs[c] = tvn + bias - vn;
            acc[c] = p.acc_n;
        }

        super::block::solve_island_normals(n, island_a, &rhs, &acc, &mut lambda);

        for c in 0..n {
            let p = &mut prepared[c];
            let delta = lambda[c] - p.acc_n;
            p.acc_n = lambda[c];
            let imp_n = p.normal * delta;
            if p.dyn_a {
                velocities[p.idx_a].linear -= imp_n * p.inv_m_a;
                velocities[p.idx_a].angular -= p.inv_i_a.mul_vec3(p.r_a.cross(imp_n));
            }
            if p.dyn_b {
                velocities[p.idx_b].linear += imp_n * p.inv_m_b;
                velocities[p.idx_b].angular += p.inv_i_b.mul_vec3(p.r_b.cross(imp_n));
            }
        }

        // ── Sequential FRICTION per contact (identical to tgs_sweep_block). ──
        for c in 0..n {
            let p = &mut prepared[c];
            let va2 = velocities[p.idx_a].linear + velocities[p.idx_a].angular.cross(p.r_a);
            let vb2 = velocities[p.idx_b].linear + velocities[p.idx_b].angular.cross(p.r_b);
            let rel2 = vb2 - va2;
            let acc_t1 = p.acc_t.dot(p.t1);
            let acc_t2 = p.acc_t.dot(p.t2);
            let mut new1 = if p.k_t1 > 1e-8 {
                acc_t1 - rel2.dot(p.t1) / p.k_t1
            } else {
                acc_t1
            };
            let mut new2 = if p.k_t2 > 1e-8 {
                acc_t2 - rel2.dot(p.t2) / p.k_t2
            } else {
                acc_t2
            };
            // Coulomb budget: μ_s for a contact at rest at the last step's end, μ_d for one sliding (`friction_limit`).
            let limit = self.friction_limit(p.sliding, p.static_friction, p.friction, p.acc_n);
            let mag = (new1 * new1 + new2 * new2).sqrt();
            if mag > limit && mag > 1e-12 {
                let s = limit / mag;
                new1 *= s;
                new2 *= s;
            }
            let imp_t = p.t1 * (new1 - acc_t1) + p.t2 * (new2 - acc_t2);
            p.acc_t = p.t1 * new1 + p.t2 * new2;
            if p.dyn_a {
                velocities[p.idx_a].linear -= imp_t * p.inv_m_a;
                velocities[p.idx_a].angular -= p.inv_i_a.mul_vec3(p.r_a.cross(imp_t));
            }
            if p.dyn_b {
                velocities[p.idx_b].linear += imp_t * p.inv_m_b;
                velocities[p.idx_b].angular += p.inv_i_b.mul_vec3(p.r_b.cross(imp_t));
            }
            apply_rolling(p, velocities);
        }
    }
}

#[cfg(test)]
mod tests {
    // The TGS soft-constraint impulse ordering. `i_scale * acc_n` is applied AFTER the
    // effective-mass division — `m_scale*(...)/k_n - i_scale*acc_n` — because it relaxes the
    // accumulated impulse rather than converting a velocity error into one.
    //
    // A commit once claimed the opposite and moved the term inside the division, pinned by an
    // arithmetic test that only showed the two orderings DIFFER, never that the inside form is
    // the right one. It is not: it is Box2D v3's formula turned inside out, and it costs
    // mass-invariance. The test below asserts the property that decides between them.
    #[test]
    fn the_impulse_scale_penalty_carries_no_mass_factor() {
        // Representative soft-constraint values (bias_rate>0, use_tgs_soft path).
        let m_scale = 0.75_f32;
        let i_scale = 0.25_f32;
        let target_vn = 0.0_f32;
        let vel_norm = -2.0_f32;
        let bias = 1.5_f32;
        let acc_n = 0.6_f32;

        // The driving term scales with the effective mass; the penalty term must not. Doubling
        // `k_n` must therefore halve the first and leave the second untouched.
        let delta = |k_n: f32| m_scale * (target_vn - vel_norm + bias) / k_n - i_scale * acc_n;
        let drive = m_scale * (target_vn - vel_norm + bias);
        assert!((delta(4.0) - (drive / 4.0 - i_scale * acc_n)).abs() < 1e-9);
        assert!(
            ((delta(4.0) + i_scale * acc_n) - 2.0 * (delta(8.0) + i_scale * acc_n)).abs() < 1e-6,
            "only the driving half may scale with 1/k_n"
        );

        // The ordering the old code used, for contrast — it puts the penalty inside.
        let inside = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / 4.0;
        assert!(
            (delta(4.0) - inside).abs() > 1e-6,
            "the two orderings must differ here, or this test proves nothing"
        );
    }

    /// Why the inside form is not merely different but unstable: it turns the λ update into
    /// `λ_{n+1} = λ_n·(1 − i_scale/k_n) + …`, and that factor leaves the unit disc once
    /// `k_n < i_scale/2`, i.e. `m_eff > 2/i_scale`. At the shipped contact softness
    /// (`hertz = 30`, `ζ = 10`, substep 1/240) `i_scale ≈ 0.0577`, so the bound is `m_eff ≈ 34.6`.
    ///
    /// Contacts survive it only because `max(0.0)` truncates the negative half of every cycle —
    /// the same recursion in a joint's ±∞-clamped equality row dropped a body 331 m. This test
    /// runs the bare recursion, with and without the clamp, so the mechanism is visible rather
    /// than asserted in prose.
    #[test]
    fn the_inside_ordering_diverges_above_its_mass_bound() {
        let i_scale = 0.0577_f32;
        let k_n = 0.001_f32; // m_eff = 1000, far past the 34.6 bound
        let drive = 1.0_f32;

        let mut unclamped = 0.0_f32;
        let mut clamped = 0.0_f32;
        for _ in 0..40 {
            unclamped += (drive - i_scale * unclamped) / k_n;
            clamped = (clamped + (drive - i_scale * clamped) / k_n).max(0.0);
        }
        assert!(
            !unclamped.is_finite() || unclamped.abs() > 1e6,
            "the inside ordering must run away here: got {unclamped}"
        );
        assert!(
            clamped.is_finite() && clamped < 1e5,
            "the non-negativity clamp is what contains it for contacts: got {clamped}"
        );

        // The shipped ordering is a contraction for ANY effective mass, since
        // `i_scale ∈ (0, 1]`. Its rate is `1 − i_scale`, so it needs more than the 40 steps
        // the divergent case needs to make its point — at 40 it is still 9% short.
        let mut outside = 0.0_f32;
        for _ in 0..400 {
            outside = outside + drive / k_n - i_scale * outside;
        }
        assert!(
            outside.is_finite() && (outside - drive / (k_n * i_scale)).abs() < 1.0,
            "the shipped ordering must converge to drive/(k_n·i_scale) = {}, got {outside}",
            drive / (k_n * i_scale)
        );
    }
}
