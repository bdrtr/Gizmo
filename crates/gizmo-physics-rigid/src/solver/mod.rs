use crate::components::{RigidBody, Velocity};
use gizmo_math::Vec3;
use gizmo_physics_core::components::Transform;
/// AAA Constraint Solver — Sequential Impulses (SI) with:
/// - Warm-starting (önceki frame'in impulslarını uygula)
/// - Accumulated-impulse clamping (negatif normal impulse engeli)
/// - Coulomb friction cone (statik + dinamik)
/// - Speculative contacts (penetrasyon öncesi temas)
/// - 2-boyutlu sürtünme (iki tangent yönü)
/// - Restitution threshold (micro-bounce önleme)
/// - Solver iteration sayısı konfigüre edilebilir
use gizmo_physics_core::ContactManifold;

mod block;
mod standalone;
mod tgs;

// ─────────────────────────────────────────────────────────────────────────────
// Support-order (topological) contact ordering.
// ─────────────────────────────────────────────────────────────────────────────
//
// Sorts an island's contacts into SUPPORT order — closest to a static/kinematic
// anchor first, propagating outward. Support depth = graph distance (in contacts)
// from the nearest anchor, via a multi-source BFS over the island's contact graph.
// For a ground-anchored vertical stack this is exactly bottom-up: the ground contact,
// then box1↔box2, then box2↔box3, … It generalises to trees/piles (no fragile "is
// this a 1-D chain?" test).
//
// The value it DOES deliver: a DETERMINISTIC TOTAL ORDER keyed on
// (max_depth, min_depth, canonical entity pair) that is INDEPENDENT of broadphase
// pair-emission order → the island solve becomes pair-order-invariant, which is the
// property that unblocks incremental broadphase (docs/ENGINE.md §7).
//
// What it was HOPED to deliver but does NOT: fixing the tall-stack instability. The
// theory was that a vertical N-stack's Delassus matrix is a 1-D Laplacian with O(N²)
// condition number, so support-order GS would converge it in ~O(N) sweeps instead of
// O(N²). Empirically REFUTED (2026-07-14): ordering barely moves the blow-up frame,
// and the blow-up is chaotic (not monotonic) in iteration count — the instability is a
// metastable normal-channel resonance, not under-convergence. See the root-cause note
// in crates/gizmo-physics-rigid/tests/soak_and_golden.rs. Hence default OFF.
/// Returns the island's maximum support depth (graph distance in contacts from the
/// nearest anchor) — used to scale the block solver's iteration count so a tall stack
/// gets enough sweeps for support to propagate up the column. When `reorder` is true,
/// also permutes `manifolds` into support order in place.
fn support_order_manifolds(
    manifolds: &mut [ContactManifold],
    rigid_bodies: &[RigidBody],
    entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
    reorder: bool,
) -> u32 {
    let n = manifolds.len();
    if n < 2 {
        return n as u32; // 0 or 1 contact: trivially ordered, depth ≤ 1.
    }

    // ── 1) Intern distinct bodies → compact local indices; record anchors + edges. ──
    // `medges[i]` = local endpoints of manifold i (None if an endpoint isn't mapped —
    // those keep last, deterministically by entity pair, matching the solver's own
    // `continue` on an unmapped manifold).
    let mut local: rustc_hash::FxHashMap<usize, u32> = rustc_hash::FxHashMap::default();
    let mut global: Vec<usize> = Vec::new(); // local idx → global body idx
    let mut is_anchor: Vec<bool> = Vec::new(); // local idx → non-dynamic (static/kinematic)?
    let mut medges: Vec<Option<(u32, u32)>> = Vec::with_capacity(n);

    for m in manifolds.iter() {
        let (ga, gb) = match (
            entity_index_map.get(&m.entity_a.id()),
            entity_index_map.get(&m.entity_b.id()),
        ) {
            (Some(&a), Some(&b)) => (a, b),
            _ => {
                medges.push(None);
                continue;
            }
        };
        let mut ends = [0u32; 2];
        for (slot, &gidx) in ends.iter_mut().zip([ga, gb].iter()) {
            *slot = match local.get(&gidx) {
                Some(&li) => li,
                None => {
                    let li = global.len() as u32;
                    local.insert(gidx, li);
                    global.push(gidx);
                    is_anchor.push(!rigid_bodies[gidx].is_dynamic());
                    li
                }
            };
        }
        medges.push(Some((ends[0], ends[1])));
    }

    let v = global.len();
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); v];
    for &(a, b) in medges.iter().flatten() {
        adj[a as usize].push(b);
        adj[b as usize].push(a);
    }

    // ── 2) Multi-source BFS from anchors → support depth (min contacts to an anchor). ──
    // BFS yields the min graph distance regardless of visitation order, so `depth` is a
    // deterministic function of the island's contact graph.
    const INF: u32 = u32::MAX;
    let mut depth: Vec<u32> = vec![INF; v];
    let mut queue: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    for li in 0..v {
        if is_anchor[li] {
            depth[li] = 0;
            queue.push_back(li as u32);
        }
    }
    // Anchor-free island (e.g. two boxes colliding mid-air): root the BFS at the body
    // with the lowest GLOBAL index — deterministic, so the order stays pair-invariant.
    if queue.is_empty() {
        let mut root = 0u32;
        let mut best = usize::MAX;
        for (li, &g) in global.iter().enumerate() {
            if g < best {
                best = g;
                root = li as u32;
            }
        }
        depth[root as usize] = 0;
        queue.push_back(root);
    }
    let mut max_depth = 0u32;
    while let Some(u) = queue.pop_front() {
        let du = depth[u as usize];
        max_depth = max_depth.max(du);
        for &w in &adj[u as usize] {
            if depth[w as usize] == INF {
                depth[w as usize] = du + 1;
                queue.push_back(w);
            }
        }
    }

    if !reorder {
        return max_depth; // caller only wanted the depth (e.g. for adaptive iterations).
    }

    // ── 3) Sort key per manifold → deterministic total order. ──
    // (max_depth, min_depth, min_entity_id, max_entity_id): anchor-closest contact
    // first; the canonical entity pair is unique per manifold so the key is a strict
    // total order → independent of the input (emission) order.
    let key_of = |i: usize| -> (u32, u32, u32, u32) {
        let ea = manifolds[i].entity_a.id();
        let eb = manifolds[i].entity_b.id();
        let (ida, idb) = if ea <= eb { (ea, eb) } else { (eb, ea) };
        match medges[i] {
            Some((la, lb)) => {
                let (da, db) = (depth[la as usize], depth[lb as usize]);
                let (lo, hi) = if da <= db { (da, db) } else { (db, da) };
                (hi, lo, ida, idb)
            }
            None => (INF, INF, ida, idb),
        }
    };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by_key(|&i| key_of(i));

    // ── 4) Apply the permutation to `manifolds` in place (no clone of the Vec-bearing
    // ContactManifold). `pos[orig] = destination slot`; cycle-swap into place. ──
    let mut pos: Vec<usize> = vec![0; n];
    for (slot, &orig) in order.iter().enumerate() {
        pos[orig] = slot;
    }
    for i in 0..n {
        while pos[i] != i {
            let target = pos[i];
            manifolds.swap(i, target);
            pos.swap(i, target);
        }
    }

    max_depth
}

// ─────────────────────────────────────────────────────────────────────────────
// Konfigürasyon
// ─────────────────────────────────────────────────────────────────────────────

/// Tuning for the contact constraint solver.
///
/// This is configuration, not state: it is `Copy` and carries nothing from one substep to
/// the next. The accumulated normal/tangent impulses live on the contact points inside the
/// manifolds, which is what makes it safe to hand the same solver value to every island
/// being solved in parallel.
///
/// Two solve paths are selected by these settings — the default TGS-soft path
/// ([`ConstraintSolver::use_tgs_soft`]) and the older split-impulse sequential-impulse
/// path, which also takes over for any island containing a CCD-enabled body. Several
/// fields are finished-but-gated experiments; each says so in its own documentation, so
/// read the field before flipping it because it sounded useful.
///
/// The values are simulation *inputs*: a replay or a rollback only reproduces the original
/// trajectory if the solver is configured identically. They are not captured in the
/// world's serialized state or in a rollback snapshot, so keeping them in sync is the
/// caller's job.
///
/// `#[non_exhaustive]`: start from [`Default`] or [`ConstraintSolver::new`] and adjust
/// individual fields.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ConstraintSolver {
    /// PGS iterasyon sayısı (daha fazla = daha stabil, daha yavaş)
    pub iterations: usize,
    /// Baumgarte stabilizasyon faktörü (0.1..0.3 arası ideal)
    /// Split Impulse kapalıyken fallback olarak kullanılır.
    pub baumgarte: f32,
    /// Penetrasyon toleransı — bu kadar penetrasyon normal kabul edilir
    pub slop: f32,
    /// Warm-start faktörü (0.8 = önceki frame impulsunun %80'ini uygula)
    pub warm_start_factor: f32,
    /// Bu hızın altındaki çarpışmalarda restitution sıfır yapılır (dinlenme teması)
    pub restitution_velocity_threshold: f32,
    /// Maksimum pozisyon düzeltme miktarı (metre/step) - Patlamaları önler
    pub max_linear_correction: f32,
    /// Split Impulse (Pseudo-Velocity) — pozisyon düzeltmesini ayrı bir
    /// pseudo-velocity kanalında yapar, velocity'yi kirletmez.
    /// Stacking stabilitesi ve resting contact jitter'ını önler.
    /// (TGS Soft açıkken kullanılmaz; eski yol fallback olarak kalır.)
    pub split_impulse_enabled: bool,
    /// Split Impulse penetrasyon düzeltme oranı (0.1..0.4 arası ideal)
    pub split_impulse_erp: f32,

    // ── TGS Soft (modern çözücü) ──────────────────────────────────────────
    /// TGS Soft yolunu kullan (soft constraint + relax pass). Açıkken split-impulse
    /// devre dışı kalır; pozisyon düzeltmesi soft bias'ın velocity katkısından
    /// (biased−relaxed)·dt olarak `pos_corrections`'a yazılır. Yüksek-enerji çarpan
    /// uzun yığınları (n≥16) SI'nin çözemediği yerde kararlı tutar.
    pub use_tgs_soft: bool,
    /// Temas yumuşaklığı frekansı (Hz). Yüksek = daha sert/az gömülme; düşük = daha
    /// yumuşak/kararlı. Box2D v3 varsayılanı 30 Hz. Substep oranına da clamp'lenir.
    pub contact_hertz: f32,
    /// Temas sönümleme oranı (ζ). >1 aşırı-sönümlü (sekmesiz, kararlı yığın). ~10.
    pub contact_damping_ratio: f32,
    /// Relax (bias=0) iterasyon sayısı — soft bias'ın enjekte ettiği hızı temizler.
    pub relax_iterations: usize,
    /// Maksimum soft-bias hızı (m/s) — derin gömülmede pozisyon düzeltme hızını
    /// sınırlar (patlama yok). Box2D v3 ≈ 3·lengthUnits.
    pub max_bias_velocity: f32,

    /// Temasları çözmeden önce ankraj-tabanlı (bottom-up) DETERMİNİSTİK support
    /// sırasına diz → ada çözümü broadphase pair-emission sırasından BAĞIMSIZ olur
    /// (pair-order-invariant). Bu, incremental-broadphase'in önünü açan uyku-determinizmi
    /// özelliğidir (docs/ENGINE.md §7).
    ///
    /// **VARSAYILAN AÇIK** (`Default` `true` veriyor). Bu yorum uzun süre "VARSAYILAN KAPALI"
    /// diyordu ve yanlıştı — 2026-08-06'da düzeltildi. Yığın kararlılığını inceleyen biri için
    /// önemli bir fark: sıralama devrede, yani ada çözümü broadphase pair-emission sırasından
    /// zaten bağımsız ve "sıralamayı açmayı denesek mi" sorusu kapalı.
    ///
    /// Başlangıçta yüksek-yığın instabilitesini çözeceği umuluyordu (Stage 1 "linchpin"), ancak
    /// ampirik ölçüm bunu ÇÜRÜTTÜ: sıralama patlama frame'ini pek oynatmıyor (bkz.
    /// soak_and_golden.rs kök-neden notu — instabilite bir under-convergence değil, metastable
    /// rezonans). Asıl sağladığı şey belirlenimci, pair-emission'dan bağımsız bir toplam sıra;
    /// incremental-broadphase işinin önünü açan uyku-determinizmi özelliği bu.
    ///
    /// Not: kapatmak `island_depth` hesabını KALDIRMAZ — `block_solver` açıkken BFS yine
    /// koşuyor (yalnız permütasyon uygulanmıyor), çünkü adaptif sweep sayısı derinliğe bağlı.
    pub support_ordering: bool,

    /// Dönen ankrajlar (Box2D-v3 tekniği): her sweep'te temas ayrımını (separation) ve
    /// jacobian kollarını (r_a/r_b) birikmiş dp DÖNMESİYLE yeniden hesapla — donmuş-ankraj
    /// linearizasyon kaymasını giderir. VARSAYILAN KAPALI: resting-stack instabilitesini
    /// ÇÖZMEZ (ampirik: N16 patlamasını ~%30 geciktirir, N32'yi çözmez — enjeksiyon
    /// HEM TGS HEM SI yolunda; yani solver-üstü/paylaşılan bir kök, bkz. soak_and_golden.rs
    /// kök-neden notu). Doğru bir teknik; per-sweep 2 Quat maliyeti nedeniyle varsayılan
    /// açılmadı. Hazır ve gated.
    pub rotating_anchors: bool,

    /// Warm-start eşleme toleransı (m): bir önceki substep'in temas impulsu, YENİ temas
    /// noktasına yalnız bu mesafeden yakınsa taşınır (pipeline.rs narrowphase warm-start).
    /// Dinlenen yığın instabilitesinin ENJEKSİYON KANALI burası: kaymış bir temas noktasına
    /// eski (skaler) impulsu YENİ kaldıraç koluyla yeniden uygulamak artık-tork bırakıp
    /// yanal salınımı pompalıyor (buckling). Toleransı düşürmek (ör. 1e-3) yalnız GERÇEKTEN
    /// dural noktalara warm-start verir → kaymış noktalar soğuk başlar → pompa kesilir.
    /// Varsayılan 0.02 (tarihsel davranış).
    pub warm_start_match_tolerance: f32,

    /// Manifold BLOCK solver: bir temas manifoldunun (aynı gövde-çifti, ≤4 coplanar nokta)
    /// normal kısıtlarını ARDIŞIK Gauss-Seidel yerine BİRLİKTE (doğrudan aktif-küme LCP) çöz.
    /// Noktalar-arası (tilt) kuplajı tam çözer → yığın-kolonunun yanal restoring stiffness'ini
    /// buckling-kritiğin ÜSTÜNE çıkarır (dinlenen-yığın instabilitesinin yapısal fixi). Sürtünme
    /// yine ardışık. Varsayılan kapalı (A/B + doğrulama); çalışırsa default açılacak.
    pub block_solver: bool,

    /// Let the block solver pick its own sweep count from the island's support depth instead of
    /// always using [`iterations`](Self::iterations).
    ///
    /// With this on (the default) a block-solved island of support depth ≥ 5 is swept
    /// `min(96, max(iterations, max(28, 3·depth/2)))` times. The floor of 28 is *above* the
    /// default `iterations` of 20, so on the default configuration the adaptive count is the
    /// only one a deep island ever sees, and lowering `iterations` cannot lower it.
    ///
    /// Turning it off makes `iterations` mean exactly what it says for every island. That is
    /// what a measurement harness needs — a sweep count it can actually set, including below
    /// the floor (see `tests/solver_quality.rs`) — and it is also the escape hatch for a caller
    /// who has measured their own scene and wants the budget back.
    ///
    /// Do not turn it off casually for a scene with tall stacks. The sweep count is not just a
    /// convergence level: the TGS path derives its inner position-integration step from it
    /// (`h = dt / n_iterations`, `solver/tgs.rs`), so fewer sweeps is also a coarser temporal
    /// discretisation inside the substep, and a stacked column needs the sweeps for support to
    /// reach its top.
    pub adaptive_iterations: bool,

    /// Block-solver Tikhonov regularizasyonu (manifoldun ortalama normal efektif kütlesinin
    /// oranı). 4-coplanar temas bloğunun rank-eksikliğini giderir; fiziksel tilt-restoring
    /// modlarını sert bırakacak kadar küçük olmalı.
    ///
    /// **0.1'den 0.05'e indirildi (2026-08-06).** 0.1, arayüzlerin çoğunun DEJENERE tek noktalı
    /// olduğu — yani rank-eksikliği hiç oluşmadığı, dolayısıyla bu terimin hiç uygulanmadığı —
    /// dönemde seçilmişti. `clip_box_box` derinlik toleransı 4-noktalı bloğu norm hâline
    /// getirince 0.1 fazla yumuşatma oldu: bir regülarizasyon terimi aynı zamanda bir
    /// yumuşatmadır ve yumuşak arayüz yavaş yakınsar. Ölçüldü — sıkıştırılmış serbest zincir
    /// (n=24, varsayılan sweep) 0.1'de 379 karede dinleniyor ve 5.4e-4 momentum sızdırıyor,
    /// 0.05'te 0. karede dinleniyor ve 4e-6 sızdırıyor: 135× daha iyi korunum. 0.05'ten
    /// 0.002'ye kadar her değer aynı; en BÜYÜĞÜ seçildi, çünkü rank-eksikliğine karşı en çok
    /// sayısal payı bırakan o. 12 katlı yığınlar (6 hücre) her değerde ayakta.
    /// (`tests/solver_quality.rs::does_block_regularization_drive_the_convergence_cost`)
    pub block_regularization: f32,

    /// Whole-CHAIN direct solve: yüksek (support-depth≥5), yeterince küçük chain adalarının
    /// TÜM normal impulslarını her sweep'te BİRLİKTE (yoğun aktif-küme LCP) çöz → inter-manifold
    /// support kuplajını TAM çözer, orta-yükseklik kule kararlılığını artırır (N24/N40 gibi).
    /// VARSAYILAN KAPALI: O(n³) maliyeti pahalı ve aşırı kuleleri (N32+) yine ROBUST çözmüyor
    /// (kalan instabilite friction/geometri-kanalında; normal+friction ortak çözücü gerek).
    /// Doğru+test edildi; yüksek kuleye ihtiyaç olan ve maliyeti kaldıran sahneler için gated.
    pub direct_chain_solve: bool,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self {
            iterations: 20,
            baumgarte: 0.15,
            slop: 0.005,
            // Full warm-start (Box2D v3 / Rapier standard). The previous 0.85 discarded 15%
            // of the accumulated impulse each substep, forcing partial re-convergence whose
            // soft-constraint bias injected a marginal amount of energy — harmless at small N
            // but compounding in tall resting stacks (blow-up at N≥24). Full warm-start closes
            // that injection and makes stacks robustly stable to N≈40 (verified: soak grid
            // N=16..40 bounded over 3000 frames). See soak_and_golden::grid_candidate_fixes.
            warm_start_factor: 1.0,
            restitution_velocity_threshold: 1.0,
            max_linear_correction: 0.02,
            split_impulse_enabled: true,
            split_impulse_erp: 0.1,
            use_tgs_soft: true,
            contact_hertz: 30.0,
            contact_damping_ratio: 10.0,
            relax_iterations: 4,
            max_bias_velocity: 4.0,
            support_ordering: true,
            rotating_anchors: false,
            warm_start_match_tolerance: 0.02,
            block_solver: true,
            adaptive_iterations: true,
            block_regularization: 0.05,
            direct_chain_solve: false,
        }
    }
}

impl ConstraintSolver {
    /// Solver with a custom base sweep count; every other field keeps its default, so the
    /// TGS-soft path and the manifold block solver stay enabled.
    ///
    /// `iterations` counts sweeps per substep, not per rendered frame, and it is a base
    /// count rather than an exact one: on the TGS-soft path with the block solver enabled,
    /// a deep (stacked) island instead gets a sweep count derived from its depth, which
    /// can be more than the value passed here or — past an internal ceiling on adaptive
    /// iterations — fewer. Clear [`adaptive_iterations`](Self::adaptive_iterations) to make
    /// it exact. Passing 0 is accepted — it is not a division-by-zero hazard — but it leaves
    /// ordinary islands' contacts unsolved.
    pub fn new(iterations: usize) -> Self {
        Self {
            iterations,
            ..Default::default()
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // ANA SOLVER: Manifold listesi üzerinde PGS (Projected Gauss-Seidel)
    // ─────────────────────────────────────────────────────────────────────────

    /// `pos_corrections` (uzunluğu `velocities` ile aynı), split-impulse pozisyon
    /// düzeltmesini gövde başına (lineer Δkonum, açısal Δ-scaled-axis) olarak DIŞARI
    /// yazar. Eskiden bu düzeltme doğrudan `velocities`'e ekleniyordu; bu, pozisyon
    /// düzeltme hızının kalıcı hıza sızmasına (resting jitter / cisimlerin uyumaması)
    /// yol açıyordu. Çağıran bu deltaları pozisyona uygulamalıdır.
    ///
    /// Returns what the adaptive policy decided for this island — see [`SolveStats`]. An island
    /// with no manifolds is not solved and reports zeroes.
    // İndeks-tabanlı döngüler kasıtlı: `mid`/`cid` aynı anda paralel dizileri
    // (manifolds + rigid_bodies/transforms/velocities/pseudo_vel, hepsi entity
    // indeksiyle hizalı) okuyup `manifolds[mid].contacts[cid]` impulslarını geri
    // yazıyor. iter_mut'a çevirmek split-borrow gymnastics gerektirir ve bu
    // determinizm-kritik PGS yolunda fayda sağlamaz.
    #[allow(clippy::needless_range_loop)]
    pub fn solve_contacts(
        &self,
        manifolds: &mut [ContactManifold],
        rigid_bodies: &[RigidBody],
        transforms: &[Transform],
        velocities: &mut [Velocity],
        pos_corrections: &mut [(Vec3, Vec3)],
        entity_index_map: &rustc_hash::FxHashMap<u32, usize>,
        // Distinct GLOBAL body indices in this island. The shared `pos_corrections`
        // buffer is thread-local-reused across islands, so we clear only THIS island's
        // entries (the caller reads back only these) instead of the whole world array.
        island_bodies: &[usize],
        dt: f32,
    ) -> SolveStats {
        // Pozisyon düzeltme buffer'ını sıfırla — yalnız bu adanın girdileri (buffer çağıran
        // tarafından adalar arası yeniden kullanılıyor; full-world sıfırlama O(n_islands×n_bodies)'ti).
        for &i in island_bodies {
            pos_corrections[i] = (Vec3::ZERO, Vec3::ZERO);
        }
        if manifolds.is_empty() {
            return SolveStats::default();
        }

        // ── Support order + island depth ──
        // Support-order the island's contacts (bottom-up from anchors) when enabled — a
        // deterministic, pair-emission-invariant total order that lets the block solver's
        // support front propagate up the column. The BFS also yields the island's support
        // depth, which drives ADAPTIVE ITERATIONS: a tall stack needs iterations ≥ its
        // height for support to reach the top, so with the block solver we scale the sweep
        // count with depth (short piles keep the base count → no perf cost).
        let island_depth = if self.support_ordering || self.block_solver {
            support_order_manifolds(manifolds, rigid_bodies, entity_index_map, self.support_ordering)
        } else {
            0
        };
        // Adaptive iterations: a stacked column of support-depth D is linearly unstable
        // (buckling) until support propagates to its top, which needs sweeps that scale
        // with D. Shallow islands (D<5, no buckling) keep the base count; bucklable stacks
        // get max(FLOOR, 1.5·D) sweeps (empirically: D=5 needs ≥24, D=32 needs ~48), capped.
        let n_iterations = if self.adaptive_iterations && self.block_solver && island_depth >= 5 {
            let target = (island_depth as usize * 3 / 2).max(Self::BLOCK_ITERS_FLOOR);
            self.iterations.max(target).min(Self::BLOCK_ITERS_CAP)
        } else {
            self.iterations
        };

        // TGS Soft yolu (modern çözücü): soft constraint + relax pass.
        // İSTİSNA: CCD-etkin gövde içeren island'lar eski (split-impulse) yolu kullanır.
        // CCD speculative temasları ince ayarlıdır; TGS'in dp/relax akışı yüksek-hızlı
        // açılı çarpmalarda speculative clamp'le çatışıp tünellemeye yol açabiliyor.
        // CCD cisimleri (mermiler) nadir ve genelde izole; yığın kararlılığı TGS'te kalır.
        let has_ccd = manifolds.iter().any(|m| {
            [m.entity_a, m.entity_b].iter().any(|e| {
                entity_index_map
                    .get(&e.id())
                    .is_some_and(|&i| rigid_bodies[i].ccd_enabled)
            })
        });

        // Tall, bucklable stacks (support-depth ≥ 5) are the historically unstable case
        // (resting-stack instability). Only these deep islands reach this branch, so a
        // trace here — chosen solver path, adaptive sweep count, block/direct flags — is a
        // low-frequency, high-signal window into the stack solve without touching the
        // per-contact inner loop.
        if island_depth >= 5 {
            tracing::trace!(
                island_depth,
                n_iterations,
                n_manifolds = manifolds.len(),
                solver = if self.use_tgs_soft && !has_ccd { "tgs-soft" } else { "split-impulse" },
                block_solver = self.block_solver,
                direct_chain = self.direct_chain_solve,
                "solving tall (bucklable) stack island"
            );
        }

        if self.use_tgs_soft && !has_ccd {
            self.solve_contacts_tgs(
                manifolds,
                rigid_bodies,
                transforms,
                velocities,
                pos_corrections,
                entity_index_map,
                island_bodies,
                island_depth,
                n_iterations,
                dt,
            );
            return SolveStats {
                island_depth,
                iterations: n_iterations,
            };
        }

        // ── Split Impulse: pseudo-velocity buffers ────────────────────────
        // Pozisyon düzeltmesi asıl velocity'den ayrılır, böylece resting
        // contact'larda jitter engellenir ve stacking stabilitesi artar.
        let mut pseudo_vel: Vec<(Vec3, Vec3)> = vec![(Vec3::ZERO, Vec3::ZERO); velocities.len()];

        // ── Warm-starting ────────────────────────────────────────────────────
        for mid in 0..manifolds.len() {
            let entity_a_id = manifolds[mid].entity_a.id();
            let entity_b_id = manifolds[mid].entity_b.id();

            let idx_a = match entity_index_map.get(&entity_a_id) {
                Some(&i) => i,
                None => continue,
            };
            let idx_b = match entity_index_map.get(&entity_b_id) {
                Some(&i) => i,
                None => continue,
            };

            let inv_m_a = rigid_bodies[idx_a].inv_mass();
            let inv_m_b = rigid_bodies[idx_b].inv_mass();
            let inv_i_a = rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
            let inv_i_b = rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
            let dyn_a = rigid_bodies[idx_a].is_dynamic();
            let dyn_b = rigid_bodies[idx_b].is_dynamic();

            let com_a = transforms[idx_a].position
                + transforms[idx_a]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_a].center_of_mass);
            let com_b = transforms[idx_b].position
                + transforms[idx_b]
                    .rotation
                    .mul_vec3(rigid_bodies[idx_b].center_of_mass);

            for contact in &manifolds[mid].contacts {
                let r_a = contact.point - com_a;
                let r_b = contact.point - com_b;

                let wn = contact.normal * (contact.normal_impulse * self.warm_start_factor);
                let wt = contact.tangent_impulse * self.warm_start_factor;

                if dyn_a {
                    velocities[idx_a].linear -= wn * inv_m_a;
                    velocities[idx_a].linear -= wt * inv_m_a;
                    velocities[idx_a].angular -= inv_i_a * (r_a.cross(wn) + r_a.cross(wt));
                }
                if dyn_b {
                    velocities[idx_b].linear += wn * inv_m_b;
                    velocities[idx_b].linear += wt * inv_m_b;
                    velocities[idx_b].angular += inv_i_b * (r_b.cross(wn) + r_b.cross(wt));
                }
            }
        }

        // ── İteratif PGS ─────────────────────────────────────────────────────
        let inv_dt = if dt > 0.0 { 1.0 / dt } else { 0.0 };

        let n_manifolds = manifolds.len();
        for iter in 0..self.iterations {
            // Symmetric Gauss-Seidel: alternate the sweep direction every
            // iteration. Plain forward-only PGS applies the manifold's contact
            // points in a fixed order; each point's impulse is off-centre, so the
            // transient bias never fully cancels and a *perfectly symmetric* impact
            // (e.g. an axis-aligned box stack) picks up spurious angular velocity
            // that tips and collapses tall stacks. Reversing on odd iterations
            // cancels the directional bias and keeps such stacks upright. The
            // order is a deterministic function of `iter`, so determinism holds.
            let reverse = iter % 2 == 1;
            for mi in 0..n_manifolds {
                let mid = if reverse { n_manifolds - 1 - mi } else { mi };
                let entity_a_id = manifolds[mid].entity_a.id();
                let entity_b_id = manifolds[mid].entity_b.id();

                let idx_a = match entity_index_map.get(&entity_a_id) {
                    Some(&i) => i,
                    None => continue,
                };
                let idx_b = match entity_index_map.get(&entity_b_id) {
                    Some(&i) => i,
                    None => continue,
                };

                let friction = manifolds[mid].friction;
                let restitution = manifolds[mid].restitution;

                let n_contacts = manifolds[mid].contacts.len();
                for ci in 0..n_contacts {
                    let cid = if reverse { n_contacts - 1 - ci } else { ci };
                    let contact_pt = manifolds[mid].contacts[cid].point;
                    let normal = manifolds[mid].contacts[cid].normal;
                    let penetration = manifolds[mid].contacts[cid].penetration;
                    let acc_n = manifolds[mid].contacts[cid].normal_impulse;
                    let acc_t = manifolds[mid].contacts[cid].tangent_impulse;

                    let com_a = transforms[idx_a].position
                        + transforms[idx_a]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position
                        + transforms[idx_b]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_b].center_of_mass);
                    let r_a = contact_pt - com_a;
                    let r_b = contact_pt - com_b;

                    let inv_m_a = rigid_bodies[idx_a].inv_mass();
                    let inv_m_b = rigid_bodies[idx_b].inv_mass();
                    let inv_i_a =
                        rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b =
                        rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b {
                        continue;
                    }

                    // Temas noktasındaki göreli hız
                    let va = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                    let vb = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                    let rel_vel = vb - va;
                    let vel_norm = rel_vel.dot(normal);

                    // ── Normal İmpuls ────────────────────────────────────────
                    let r_a_x_n = r_a.cross(normal);
                    let r_b_x_n = r_b.cross(normal);
                    let k_n = inv_m_a
                        + inv_m_b
                        + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
                        + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);

                    if k_n < 1e-8 {
                        continue;
                    }

                    // Pozisyon düzeltme stratejisi:
                    // Split Impulse: bias=0 (pozisyon düzeltmesi ayrı pseudo-velocity kanalında)
                    // Fallback: Baumgarte bias velocity'ye karıştırılır
                    let bias = if penetration < 0.0 {
                        // Speculative contact: nesne henüz teması yapmadı
                        penetration * inv_dt
                    } else if self.split_impulse_enabled {
                        // Split Impulse: pozisyon düzeltme tamamen pseudo-velocity pass'te
                        // Velocity kanalı temiz kalır → resting jitter yok
                        0.0
                    } else {
                        // Fallback Baumgarte
                        let correction = (penetration - self.slop)
                            .max(0.0)
                            .min(self.max_linear_correction);
                        self.baumgarte * inv_dt * correction
                    };

                    // Restitution: sadece yüksek hızlı GERÇEK çarpışmalarda. Speculative
                    // temas (penetration < 0) bir boşluk-kapatma LİMİTİdir; ona restitution
                    // uygulamak bias'ı bozar (cisim substep'ler arası tutarsız yavaşlar ve
                    // son substep'te yüzeyi aşıp girer). Sekme, cisim gerçekten değdiğinde
                    // (penetration ≥ 0) doğal olarak uygulanır.
                    let e = if penetration < 0.0 {
                        0.0
                    } else if -vel_norm > self.restitution_velocity_threshold {
                        restitution
                    } else {
                        0.0
                    };

                    let delta_n = (-(1.0 + e) * vel_norm + bias) / k_n;
                    let new_acc_n = (acc_n + delta_n).max(0.0); // Clamp: çekme yok
                    let actual_n = new_acc_n - acc_n;
                    manifolds[mid].contacts[cid].normal_impulse = new_acc_n;

                    let imp_n = normal * actual_n;
                    if dyn_a {
                        velocities[idx_a].linear -= imp_n * inv_m_a;
                        velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_n));
                    }
                    if dyn_b {
                        velocities[idx_b].linear += imp_n * inv_m_b;
                        velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_n));
                    }

                    // ── Sürtünme İmpulsu (2-tangent Coulomb cone) ───────────
                    // Normalden türetilen SABİT iki ortonormal tangent (t1,t2); birikim
                    // her eksende skaler ve birlikte dairesel koniye clamp'lenir.
                    // (Eski tek-tangent yöntemi tangenti her iterasyonda döndürüp birikmiş
                    // impulsun dik bileşenini kaybediyordu → kayıplı/yön kayan sürtünme.)
                    let (t1, t2) = {
                        let a = if normal.x.abs() > 0.9 {
                            Vec3::new(0.0, 1.0, 0.0).cross(normal)
                        } else {
                            Vec3::new(1.0, 0.0, 0.0).cross(normal)
                        }
                        .normalize();
                        (a, normal.cross(a))
                    };

                    let va2 = velocities[idx_a].linear + velocities[idx_a].angular.cross(r_a);
                    let vb2 = velocities[idx_b].linear + velocities[idx_b].angular.cross(r_b);
                    let rel2 = vb2 - va2;

                    // Eksen başına efektif kütle: k = inv_m + (r×t)·I⁻¹·(r×t).
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

                    // Birikmiş tangent impulsu sabit baza ayrıştır, her eksende çöz.
                    let acc_t1 = acc_t.dot(t1);
                    let acc_t2 = acc_t.dot(t2);
                    let mut new1 = if k_t1 > 1e-8 {
                        acc_t1 - rel2.dot(t1) / k_t1
                    } else {
                        acc_t1
                    };
                    let mut new2 = if k_t2 > 1e-8 {
                        acc_t2 - rel2.dot(t2) / k_t2
                    } else {
                        acc_t2
                    };

                    // Dairesel Coulomb koni: |(new1,new2)| ≤ μ_s·N ise statik; aşarsa μ_d·N'e ölçekle.
                    let max_static = manifolds[mid].static_friction * new_acc_n.abs();
                    let max_dynamic = friction * new_acc_n.abs();
                    let mag = (new1 * new1 + new2 * new2).sqrt();
                    if mag > max_static && mag > 1e-12 {
                        let s = max_dynamic / mag;
                        new1 *= s;
                        new2 *= s;
                    }

                    let imp_t = t1 * (new1 - acc_t1) + t2 * (new2 - acc_t2);
                    manifolds[mid].contacts[cid].tangent_impulse = t1 * new1 + t2 * new2;
                    if dyn_a {
                        velocities[idx_a].linear -= imp_t * inv_m_a;
                        velocities[idx_a].angular -= inv_i_a.mul_vec3(r_a.cross(imp_t));
                    }
                    if dyn_b {
                        velocities[idx_b].linear += imp_t * inv_m_b;
                        velocities[idx_b].angular += inv_i_b.mul_vec3(r_b.cross(imp_t));
                    }
                }
            }
        }

        // ── Split Impulse: Pozisyon Düzeltme Pass ────────────────────────────
        // Asıl velocity'den bağımsız olarak pseudo-velocity hesaplar.
        // Bu pass penetrasyon düzeltmesini velocity kanalından ayırır.
        // Birikimli pseudo-impulse takibi ile over-correction engellenir.
        if self.split_impulse_enabled {
            // Per-contact birikimli pseudo-impulse (PGS clamping için).
            //
            // Tek düz tampon + offset tablosu; `solve_contacts_tgs`'in `vn0`'ıyla aynı düzeltme
            // (C4-followup). Eskiden manifold başına bir iç `Vec` ayrılıyordu, yani ada başına
            // substep başına `1 + N_manifold` ayırma. `ArrayVec` olamaz: `m.contacts.len()`
            // capped değil ve `ArrayVec` taşmada panikler.
            let mut acc_off: Vec<usize> = Vec::with_capacity(manifolds.len() + 1);
            acc_off.push(0);
            let mut acc_total = 0usize;
            for m in manifolds.iter() {
                acc_total += m.contacts.len();
                acc_off.push(acc_total);
            }
            let mut acc_pseudo = vec![0.0f32; acc_total];

            let pos_iterations = (self.iterations / 2).max(4);
            for _ in 0..pos_iterations {
                for mid in 0..manifolds.len() {
                    let entity_a_id = manifolds[mid].entity_a.id();
                    let entity_b_id = manifolds[mid].entity_b.id();

                    let idx_a = match entity_index_map.get(&entity_a_id) {
                        Some(&i) => i,
                        None => continue,
                    };
                    let idx_b = match entity_index_map.get(&entity_b_id) {
                        Some(&i) => i,
                        None => continue,
                    };

                    let inv_m_a = rigid_bodies[idx_a].inv_mass();
                    let inv_m_b = rigid_bodies[idx_b].inv_mass();
                    let inv_i_a =
                        rigid_bodies[idx_a].inv_world_inertia_tensor(transforms[idx_a].rotation);
                    let inv_i_b =
                        rigid_bodies[idx_b].inv_world_inertia_tensor(transforms[idx_b].rotation);
                    let dyn_a = rigid_bodies[idx_a].is_dynamic();
                    let dyn_b = rigid_bodies[idx_b].is_dynamic();

                    if !dyn_a && !dyn_b {
                        continue;
                    }

                    let com_a = transforms[idx_a].position
                        + transforms[idx_a]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_a].center_of_mass);
                    let com_b = transforms[idx_b].position
                        + transforms[idx_b]
                            .rotation
                            .mul_vec3(rigid_bodies[idx_b].center_of_mass);

                    for cid in 0..manifolds[mid].contacts.len() {
                        let contact_pt = manifolds[mid].contacts[cid].point;
                        let normal = manifolds[mid].contacts[cid].normal;
                        let penetration = manifolds[mid].contacts[cid].penetration;

                        let correction = (penetration - self.slop)
                            .max(0.0)
                            .min(self.max_linear_correction);
                        if correction < 1e-6 {
                            continue;
                        }

                        let r_a = contact_pt - com_a;
                        let r_b = contact_pt - com_b;

                        let r_a_x_n = r_a.cross(normal);
                        let r_b_x_n = r_b.cross(normal);
                        let k_n = inv_m_a
                            + inv_m_b
                            + (inv_i_a.mul_vec3(r_a_x_n)).dot(r_a_x_n)
                            + (inv_i_b.mul_vec3(r_b_x_n)).dot(r_b_x_n);
                        if k_n < 1e-8 {
                            continue;
                        }

                        // Pseudo-velocity relative to contact normal
                        let pv_a = pseudo_vel[idx_a].0 + pseudo_vel[idx_a].1.cross(r_a);
                        let pv_b = pseudo_vel[idx_b].0 + pseudo_vel[idx_b].1.cross(r_b);
                        let pv_rel = pv_b.dot(normal) - pv_a.dot(normal);

                        let bias = self.split_impulse_erp * inv_dt * correction;
                        // Velocity solver ile aynı konvansiyon: delta = (-pv_rel + bias) / k
                        // pv_rel > 0 → nesneler zaten ayrılıyor → düzeltme azalır
                        // pv_rel ≈ bias → yakınsadı → delta ≈ 0
                        let delta_p = (-pv_rel + bias) / k_n;

                        // Birikimli clamp: toplam pseudo-impulse ≥ 0 (çekme yok)
                        let acc_i = acc_off[mid] + cid;
                        let old_acc = acc_pseudo[acc_i];
                        let new_acc = (old_acc + delta_p).max(0.0);
                        let actual_delta = new_acc - old_acc;
                        acc_pseudo[acc_i] = new_acc;

                        let imp_p = normal * actual_delta;
                        if dyn_a {
                            pseudo_vel[idx_a].0 -= imp_p * inv_m_a;
                            pseudo_vel[idx_a].1 -= inv_i_a.mul_vec3(r_a.cross(imp_p));
                        }
                        if dyn_b {
                            pseudo_vel[idx_b].0 += imp_p * inv_m_b;
                            pseudo_vel[idx_b].1 += inv_i_b.mul_vec3(r_b.cross(imp_p));
                        }
                    }
                }
            }

            // Pseudo-velocity'yi HIZA EKLEME (eski hata buydu). Bunun yerine pozisyon
            // düzeltmesi olarak dışarı yaz: Δkonum = pseudo_vel * dt. Çağıran bunu
            // doğrudan transform'a uygular; hız kanalı temiz kalır.
            for i in 0..velocities.len() {
                pos_corrections[i] = (pseudo_vel[i].0 * dt, pseudo_vel[i].1 * dt);
            }
        }

        // The split-impulse path sweeps `self.iterations` (see the loop above) and has never
        // consumed the adaptive count, so reporting `n_iterations` here would be a lie about
        // work that was not done. An island reaches this path only by containing a CCD-enabled
        // body or by `use_tgs_soft` being off.
        SolveStats {
            island_depth,
            iterations: self.iterations,
        }
    }
}

/// What the solver decided for one island, reported back so a caller can see the sweep count
/// the adaptive policy actually chose.
///
/// The engine folds this into [`PhysicsMetrics`](crate::island::PhysicsMetrics); it is returned
/// separately because the metrics are per step and this is per island, and because the policy is
/// exactly the thing a caller tuning solver cost needs visibility into.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SolveStats {
    /// The island's support depth — BFS eccentricity of its contact graph from its anchors, or
    /// from its lowest-indexed body when it has none. Zero for an island with no manifolds.
    pub island_depth: u32,
    /// Biased sweeps run for this island. On the TGS path with the block solver on and depth
    /// ≥ 5 this is the depth-adaptive count rather than
    /// [`ConstraintSolver::iterations`](ConstraintSolver::iterations); on the split-impulse
    /// path it is always `iterations`, which is that path's real behaviour and not a rounding
    /// of it.
    pub iterations: usize,
}

#[cfg(test)]
mod support_order_tests {
    use super::*;
    use gizmo_physics_core::BodyHandle;

    // Build a manifold between two bodies (contacts irrelevant to ordering — it keys on
    // the entity pair + graph depth, never reads contact points).
    fn manifold(a: u32, b: u32) -> ContactManifold {
        ContactManifold::new(BodyHandle::from_id(a), BodyHandle::from_id(b))
    }

    fn id_pairs(ms: &[ContactManifold]) -> Vec<(u32, u32)> {
        ms.iter().map(|m| (m.entity_a.id(), m.entity_b.id())).collect()
    }

    /// The ordering is a deterministic TOTAL order independent of the input (broadphase
    /// emission) order, and for a ground-anchored chain it is bottom-up from the anchor.
    #[test]
    fn support_order_is_pair_emission_invariant_and_bottom_up() {
        // Body 0 = static ground; bodies 1..=5 = a dynamic vertical chain.
        let mut rigid_bodies = vec![RigidBody::new_static()];
        for _ in 0..5 {
            rigid_bodies.push(RigidBody::new(1.0, true));
        }
        let entity_index_map: rustc_hash::FxHashMap<u32, usize> =
            (0..rigid_bodies.len() as u32).map(|i| (i, i as usize)).collect();

        // Chain contacts: ground↔1, 1↔2, 2↔3, 3↔4, 4↔5.
        let chain = [(0u32, 1u32), (1, 2), (2, 3), (3, 4), (4, 5)];

        // Several different emission orders of the SAME contact set.
        let orders: [Vec<(u32, u32)>; 3] = [
            chain.to_vec(),
            vec![(3, 4), (0, 1), (4, 5), (1, 2), (2, 3)], // shuffled
            chain.iter().rev().copied().collect(),        // reversed
        ];

        let mut results = Vec::new();
        for order in &orders {
            let mut ms: Vec<ContactManifold> = order.iter().map(|&(a, b)| manifold(a, b)).collect();
            support_order_manifolds(&mut ms, &rigid_bodies, &entity_index_map, true);
            results.push(id_pairs(&ms));
        }

        // (1) Pair-order-invariance: every emission order yields the identical solve order.
        assert_eq!(results[0], results[1], "shuffled emission changed the solve order");
        assert_eq!(results[0], results[2], "reversed emission changed the solve order");

        // (2) Bottom-up from the anchor: the ground contact (0,1) is solved first, then
        //     the chain propagates outward.
        assert_eq!(
            results[0],
            vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
            "expected anchor-first bottom-up support order"
        );
    }

    /// Anchor-free island (no static body): still a deterministic total order,
    /// independent of emission order (rooted at the lowest body index).
    #[test]
    fn support_order_anchor_free_is_deterministic() {
        // Four dynamic bodies (ids 1..=4), no static anchor, chained 1-2-3-4.
        let mut rigid_bodies = vec![RigidBody::new_static()]; // id 0 unused here
        for _ in 0..4 {
            rigid_bodies.push(RigidBody::new(1.0, true));
        }
        let entity_index_map: rustc_hash::FxHashMap<u32, usize> =
            (0..rigid_bodies.len() as u32).map(|i| (i, i as usize)).collect();

        let a: Vec<(u32, u32)> = vec![(1, 2), (2, 3), (3, 4)];
        let b: Vec<(u32, u32)> = vec![(3, 4), (1, 2), (2, 3)];

        let mut ma: Vec<ContactManifold> = a.iter().map(|&(x, y)| manifold(x, y)).collect();
        let mut mb: Vec<ContactManifold> = b.iter().map(|&(x, y)| manifold(x, y)).collect();
        support_order_manifolds(&mut ma, &rigid_bodies, &entity_index_map, true);
        support_order_manifolds(&mut mb, &rigid_bodies, &entity_index_map, true);
        assert_eq!(id_pairs(&ma), id_pairs(&mb), "anchor-free order must be emission-invariant");
    }
}
