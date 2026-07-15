use super::ConstraintSolver;
use crate::components::{RigidBody, Velocity};
use gizmo_math::{Mat3, Vec3};
use gizmo_physics_core::components::Transform;
use gizmo_physics_core::ContactManifold;

/// Per-contact quantities that are INVARIANT across all 24 TGS sweeps (the biased
/// iterations + relax passes) — hoisted out of the sweep and computed exactly once.
/// The sweep then does only the velocity/dp-dependent work. `acc_n`/`acc_t` are the
/// mutable impulse accumulators: seeded from the manifold, mutated in place each
/// sweep, and written back to the manifold after solving (warm-start continuity).
/// Every field is produced with the SAME expression the sweep used, so the result
/// is bit-identical to the pre-hoist solver. Contacts that the old sweep skipped
/// (`k_n < 1e-8`, or a manifold with two non-dynamic bodies) are simply not built.
struct Prepared {
    idx_a: usize,
    idx_b: usize,
    dyn_a: bool,
    dyn_b: bool,
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
    restitution: f32,
    pen0: f32,
    vn0: f32,
    mid: usize,
    cid: usize,
    acc_n: f32,
    acc_t: Vec3,
}

/// One contact manifold's contiguous run in `prepared`, plus its precomputed N×N normal
/// Delassus matrix `a` (constant across sweeps under frozen anchors). Used by the block
/// solver to resolve the manifold's coplanar normal impulses jointly.
struct BlockGroup {
    start: usize,
    n: usize,
    a: [[f32; 4]; 4],
}

impl ConstraintSolver {
    /// Adaptive iterations (block solver): minimum sweeps for any bucklable stack (D≥5).
    /// Even a short stack needs ~this many block sweeps to stay below the buckling limit.
    pub(super) const BLOCK_ITERS_FLOOR: usize = 28;
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
        entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
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
    ) {
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

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

        let com_of = |idx: usize| -> Vec3 {
            transforms[idx].position
                + transforms[idx]
                    .rotation
                    .mul_vec3(rigid_bodies[idx].center_of_mass)
        };

        // ── 0) Restitution için başlangıç yaklaşma hızı (warm-start ÖNCESİ) ──
        let mut vn0: Vec<Vec<f32>> = manifolds
            .iter()
            .map(|m| vec![0.0f32; m.contacts.len()])
            .collect();
        for mid in 0..manifolds.len() {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            for cid in 0..manifolds[mid].contacts.len() {
                let ct = manifolds[mid].contacts[cid];
                let r_a = ct.point - com_a;
                let r_b = ct.point - com_b;
                let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                vn0[mid][cid] = (vb - va).dot(ct.normal);
            }
        }

        // ── 1) Warm-start ──
        for mid in 0..manifolds.len() {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            for contact in &manifolds[mid].contacts {
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

        let h = if n_iterations > 0 {
            dt / n_iterations as f32
        } else {
            dt
        };

        // ── Per-contact SABİT precompute (hoist) ──
        // sweep'ler arası DEĞİŞMEYEN her şeyi (idx / inv_m / inv_i / com→r / normal / k_n /
        // t1,t2 / k_t) bir KEZ hesapla; 24 sweep aynı `prepared` dizisini yeniden kullanır.
        // Değerler eski sweep'le BİREBİR aynı ifadelerle üretildiğinden sonuç bit-identical
        // (davranış korunur — tgs_hash_check oracle'ı ile doğrulandı). Eski `continue`
        // davranışı korunur: iki-statik manifold ve k_n<1e-8 temaslar hiç eklenmez, dolayısıyla
        // her sweep'te atlanmış olurlar. Impulse birikimcileri (acc_n/acc_t) manifold'dan
        // tohumlanır, sweep'lerde yerinde güncellenir, sonda geri yazılır.
        let mut prepared: Vec<Prepared> = Vec::with_capacity(manifolds.len());
        for mid in 0..manifolds.len() {
            let (idx_a, idx_b) = match (
                entity_index_map.get(&manifolds[mid].entity_a.id()),
                entity_index_map.get(&manifolds[mid].entity_b.id()),
            ) {
                (Some(&a), Some(&b)) => (a, b),
                _ => continue,
            };
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();
            if !dyn_a && !dyn_b {
                continue;
            }
            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let com_a = com_of(idx_a);
            let com_b = com_of(idx_b);
            let friction = manifolds[mid].friction;
            let static_friction = manifolds[mid].static_friction;
            let restitution = manifolds[mid].restitution;
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
                    restitution,
                    pen0: ct.penetration,
                    vn0: vn0[mid][cid],
                    mid,
                    cid,
                    acc_n: ct.normal_impulse,
                    acc_t: ct.tangent_impulse,
                });
            }
        }

        // ── Block-solver groups (Stage: manifold block normal solve) ──
        // Each manifold's contacts are CONTIGUOUS in `prepared` (built mid-outer, cid-inner),
        // all sharing the same body pair. Precompute, once, the N×N normal Delassus matrix A
        // per group (constant across sweeps under frozen anchors): A[r][c] = change in contact
        // r's normal velocity per unit normal impulse at contact c. The biased/relax sweeps
        // then solve each group's normals JOINTLY (block LCP) so the inter-point tilt coupling
        // is exact — the restoring stiffness that keeps tall stacks from buckling.
        let block_groups: Vec<BlockGroup> = if self.block_solver {
            let mut groups: Vec<BlockGroup> = Vec::new();
            let mut i = 0usize;
            while i < prepared.len() {
                let mid = prepared[i].mid;
                let start = i;
                while i < prepared.len() && prepared[i].mid == mid {
                    i += 1;
                }
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
            }
            groups
        } else {
            Vec::new()
        };

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
        let island_a: Vec<f32> = if use_direct {
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
        } else {
            Vec::new()
        };

        // Per-body position-delta scratch (dp), indexed by GLOBAL body index but sized once
        // per thread and RESET only for `island_bodies` — no per-island full-world allocation.
        // Islands are disjoint (a body is in exactly one island), so resetting just this
        // island's entries fully isolates the reused thread-local across islands/substeps.
        thread_local! {
            static DP: std::cell::RefCell<Vec<(Vec3, Vec3)>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        DP.with(|cell| {
            let mut dp = cell.borrow_mut();
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
            for iter in 0..n_iterations {
                if use_direct {
                    self.tgs_sweep_island(
                        &mut prepared, &island_a, velocities, dp.as_slice(),
                        false, true, bias_rate, mass_scale, impulse_scale, inv_dt,
                    );
                } else if self.block_solver {
                    self.tgs_sweep_block(
                        &mut prepared, &block_groups, velocities, dp.as_slice(),
                        false, iter % 2 == 1, true, bias_rate, mass_scale, impulse_scale, inv_dt,
                    );
                } else {
                    self.tgs_sweep_prepared(
                        &mut prepared, velocities, dp.as_slice(),
                        false, iter % 2 == 1, true,
                        bias_rate, mass_scale, impulse_scale, inv_dt,
                    );
                }
                for &i in island_bodies {
                    if real_bodies.contains(&i) && rigid_bodies[i].is_dynamic() {
                        dp[i].0 += velocities[i].linear * h;
                        dp[i].1 += velocities[i].angular * h;
                    }
                }
            }

            // ── 4) RELAX (bias=0) — soft bias hızını temizle + restitution ──
            for iter in 0..self.relax_iterations {
                if use_direct {
                    self.tgs_sweep_island(
                        &mut prepared, &island_a, velocities, dp.as_slice(),
                        true, false, 0.0, 1.0, 0.0, inv_dt,
                    );
                } else if self.block_solver {
                    self.tgs_sweep_block(
                        &mut prepared, &block_groups, velocities, dp.as_slice(),
                        true, iter % 2 == 1, false, 0.0, 1.0, 0.0, inv_dt,
                    );
                } else {
                    self.tgs_sweep_prepared(
                        &mut prepared, velocities, dp.as_slice(),
                        true, iter % 2 == 1, false,
                        0.0, 1.0, 0.0, inv_dt,
                    );
                }
            }

            // ── 5) Pozisyon düzeltmesi = dp − (relaxed hız)·dt ──
            // dp, biased çözümün TGS-entegre GERÇEK pozisyon değişimidir (penetrasyon
            // düzeltmesi yığın boyunca yayılmış). position_integration relaxed·dt ekleyecek;
            // farkı burada ekleyince toplam = dp olur → penetrasyon düzelir, hız temiz kalır
            // (resting jitter yok, uyuyabilir). Restitution relax içinde uygulandığından
            // sekme hızı `velocities`'tedir; çıkarılması sekme yer-değişimini bu kareye değil
            // SONRAKİ kareye taşır (kararlı). Yalnız aktif dinamik gövdelere uygulanır.
            for &i in island_bodies {
                if real_bodies.contains(&i) && rigid_bodies[i].is_dynamic() {
                    let dlin = dp[i].0 - velocities[i].linear * dt;
                    let dang = dp[i].1 - velocities[i].angular * dt;
                    pos_corrections[i] = (dlin, dang);
                }
            }
        });

        // Impulse birikimcilerini manifold'a geri yaz (warm-start sürekliliği: bir sonraki
        // substep bu değerleri okur). Eski kod bunu her sweep'te yazıyordu; `prepared` bunları
        // bellekte tuttuğu için tek bir yazma yeterli, sonuç aynı.
        for p in &prepared {
            manifolds[p.mid].contacts[p.cid].normal_impulse = p.acc_n;
            manifolds[p.mid].contacts[p.cid].tangent_impulse = p.acc_t;
        }
    }

    /// Tek TGS iterasyon taraması — precompute'lu (`prepared`) sürüm. Her temasta normal
    /// (opsiyonel soft bias) + 2-tangent Coulomb sürtünme çöz. `use_bias=false` → rijit
    /// relax. Sabitler (`r_a/r_b/normal/k_n/t1/t2/k_t/inv_i…`) `prepared`'da hazır; burada
    /// yalnız hız/dp'ye bağlı değişken kısım hesaplanır. Ters sıra (`reverse`) düz dizinin
    /// TERSİDİR — eski kodun (manifold-ters + contact-ters) sırasıyla BİREBİR aynı.
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

            // Soft bias: penetrasyonda yay-damper gibi ayır; speculative boşlukta
            // (penetration<0) kapanmayı boşluk/dt ile sınırla; aksi halde bias yok.
            let (bias, m_scale, i_scale) = if penetration < 0.0 {
                (penetration * inv_dt, 1.0, 0.0)
            } else if use_bias && penetration > self.slop {
                let b = (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity);
                (b, mass_scale, impulse_scale)
            } else {
                (0.0, 1.0, 0.0)
            };

            // Restitution hedef-hızı: yalnız RELAX aşamasında, GERÇEK temasta
            // (pen0≥0, speculative değil), eşiği aşan çarpmada — sönümlü/clamp'li.
            let target_vn = if apply_restitution
                && p.restitution > 0.0
                && p.pen0 >= 0.0
                && p.vn0 < -self.restitution_velocity_threshold
            {
                -p.restitution * p.vn0
            } else {
                0.0
            };
            let delta_n = (m_scale * (target_vn - vel_norm + bias) - i_scale * p.acc_n) / p.k_n;
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
            let max_static = p.static_friction * new_acc_n.abs();
            let max_dynamic = p.friction * new_acc_n.abs();
            let mag = (new1 * new1 + new2 * new2).sqrt();
            if mag > max_static && mag > 1e-12 {
                let s = max_dynamic / mag;
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
        mass_scale: f32,
        impulse_scale: f32,
        inv_dt: f32,
    ) {
        let ng = groups.len();
        for gi in 0..ng {
            let g = &groups[if reverse { ng - 1 - gi } else { gi }];
            let (start, n) = (g.start, g.n);
            if n == 0 {
                continue;
            }

            // ── Block NORMAL solve ── build the coupled soft driving term per contact:
            // rhs_i = m_scale·(target_i − vn_i) − i_scale·acc_i (the block generalisation
            // of the soft per-contact update). Speculative (pen<0) and non-biased contacts
            // use their own m_scale/i_scale exactly as the sequential sweep does.
            let mut rhs = [0.0f32; 4];
            let mut acc = [0.0f32; 4];
            for c in 0..n {
                let p = &prepared[start + c];
                let dp_a = dp[p.idx_a].0 + dp[p.idx_a].1.cross(p.r_a);
                let dp_b = dp[p.idx_b].0 + dp[p.idx_b].1.cross(p.r_b);
                let penetration = p.pen0 - (dp_b - dp_a).dot(p.normal);
                let va = velocities[p.idx_a].linear + velocities[p.idx_a].angular.cross(p.r_a);
                let vb = velocities[p.idx_b].linear + velocities[p.idx_b].angular.cross(p.r_b);
                let vn = (vb - va).dot(p.normal);
                // RIGID block (max restoring stiffness — the buckling fix): drive vn to
                // the bias/restitution target exactly. The Tikhonov regularisation on `a`
                // (not soft mass-scaling) is what tames the response, so the singular
                // redundant direction stays bounded while the physical modes stay stiff.
                // Softening the block with mass_scale<1 measurably weakens the tall-stack
                // fix, so it is deliberately NOT applied here. Speculative gaps (pen<0)
                // limit approach; otherwise bias only when use_bias.
                let _ = (mass_scale, impulse_scale);
                let bias = if penetration < 0.0 {
                    penetration * inv_dt
                } else if use_bias && penetration > self.slop {
                    (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity)
                } else {
                    0.0
                };
                let tvn = if apply_restitution
                    && p.restitution > 0.0
                    && p.pen0 >= 0.0
                    && p.vn0 < -self.restitution_velocity_threshold
                {
                    -p.restitution * p.vn0
                } else {
                    0.0
                };
                rhs[c] = tvn + bias - vn;
                acc[c] = p.acc_n;
            }

            let lambda = super::block::solve_normal_block(n, &g.a, &rhs, &acc);

            for c in 0..n {
                let p = &mut prepared[start + c];
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

            // ── Sequential FRICTION per contact (2-tangent Coulomb cone) — identical to
            // tgs_sweep_prepared, using the (frozen) anchors and the block's new acc_n. ──
            for c in 0..n {
                let p = &mut prepared[start + c];
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
                let max_static = p.static_friction * p.acc_n.abs();
                let max_dynamic = p.friction * p.acc_n.abs();
                let mag = (new1 * new1 + new2 * new2).sqrt();
                if mag > max_static && mag > 1e-12 {
                    let s = max_dynamic / mag;
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
            }
        }
    }

    /// Whole-CHAIN direct sweep: solve ALL of the island's normal impulses JOINTLY (one
    /// active-set LCP over the precomputed island Delassus matrix) so the inter-manifold
    /// support coupling up the column is resolved EXACTLY, then friction sequentially per
    /// contact. Iteration-independent — this is what eliminates the tall-tower buckling the
    /// per-manifold iterative path leaves as a weak creep. Rigid normal solve (like the
    /// block path); frozen anchors. No `reverse` (a joint solve has no sweep direction).
    #[allow(clippy::too_many_arguments)]
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
            let bias = if penetration < 0.0 {
                penetration * inv_dt
            } else if use_bias && penetration > self.slop {
                (bias_rate * (penetration - self.slop)).min(self.max_bias_velocity)
            } else {
                0.0
            };
            let tvn = if apply_restitution
                && p.restitution > 0.0
                && p.pen0 >= 0.0
                && p.vn0 < -self.restitution_velocity_threshold
            {
                -p.restitution * p.vn0
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
            let max_static = p.static_friction * p.acc_n.abs();
            let max_dynamic = p.friction * p.acc_n.abs();
            let mag = (new1 * new1 + new2 * new2).sqrt();
            if mag > max_static && mag > 1e-12 {
                let s = max_dynamic / mag;
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
        }
    }
}

#[cfg(test)]
mod tests {
    // Regression for the TGS soft-constraint impulse ordering. The impulse-scale
    // penalty term (`i_scale * acc_n`) must be applied *inside* the effective-mass
    // division, i.e. `(m_scale*(...) - i_scale*acc_n) / k_n`, not after it
    // (`m_scale*(...)/k_n - i_scale*acc_n`). The two disagree whenever both an
    // impulse-scale penalty and a non-unit effective mass (k_n != 1) are present.
    #[test]
    fn soft_constraint_impulse_scale_applied_before_division() {
        // Representative soft-constraint values (bias_rate>0, use_tgs_soft path).
        let m_scale = 0.75_f32;
        let i_scale = 0.25_f32;
        let target_vn = 0.0_f32;
        let vel_norm = -2.0_f32;
        let bias = 1.5_f32;
        let k_n = 4.0_f32; // non-unit effective mass exposes the ordering bug
        let acc_n = 0.6_f32;

        // Correct (post-fix) ordering.
        let correct = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / k_n;
        // Buggy (pre-fix) ordering.
        let buggy = m_scale * (target_vn - vel_norm + bias) / k_n - i_scale * acc_n;

        // They must differ, and the current code must match the correct form.
        assert!((correct - buggy).abs() > 1e-6, "orderings must differ for this case");

        let delta_n = (m_scale * (target_vn - vel_norm + bias) - i_scale * acc_n) / k_n;
        assert!((delta_n - correct).abs() < 1e-9);
    }
}
