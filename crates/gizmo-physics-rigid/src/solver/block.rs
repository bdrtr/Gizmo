//! Direct block solve of a contact manifold's coupled NORMAL constraints.
//!
//! A box-box (or any) contact manifold has up to 4 coplanar contact points that all
//! connect the SAME body pair (a, b). Sequential Gauss-Seidel solves them one at a
//! time, so the inter-point coupling — the impulse *redistribution* that resists a
//! tilt of the top box — is only partially resolved per sweep. For a tall stacked
//! column that under-resolution drops the effective lateral/rotational restoring
//! stiffness just below the buckling-critical value, so the column is linearly
//! unstable (leans, then topples) — the resting-stack instability
//! (soak_resting_stacks_never_gain_energy).
//!
//! Solving the manifold's normal impulses JOINTLY makes that redistribution exact,
//! restoring the tilt-resisting torque. The joint problem is a small (N≤4) LCP:
//!   find λ ≥ 0 s.t. for every ACTIVE contact  (A·Δ)_i = target_i − vn_i,  λ = acc + Δ,
//! with contacts that would need λ<0 (separating) clamped to λ=0. We solve it with a
//! bounded active-set method: solve the dense SPD system on the active set, clamp the
//! most-negative λ to zero, repeat. N≤4 ⇒ this converges in ≤N drops and is cheap.

// Dense linear-algebra kernels here index flat/row-major matrices by (row·n + col), so
// index loops are the clearest form; the thread-local scratch tuple is intentional.
#![allow(
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::doc_overindented_list_items
)]

/// Solve the dense system `a·x = b` (size `n`, n≤4) by Gauss-Elimination with partial
/// pivoting, in place. Returns `None` if the matrix is (near-)singular. `a` is the
/// active-set Delassus block (symmetric positive-definite in exact arithmetic, but we
/// pivot for numerical safety). Writes the solution into `b` and returns it.
fn solve_dense(a: &mut [[f32; 4]; 4], b: &mut [f32; 4], n: usize) -> Option<[f32; 4]> {
    for col in 0..n {
        // Partial pivot: largest |a[row][col]| at or below the diagonal.
        let mut piv = col;
        let mut best = a[col][col].abs();
        for row in (col + 1)..n {
            let v = a[row][col].abs();
            if v > best {
                best = v;
                piv = row;
            }
        }
        if best < 1e-12 {
            return None; // singular / degenerate (e.g. duplicate contact points)
        }
        if piv != col {
            a.swap(piv, col);
            b.swap(piv, col);
        }
        let inv = 1.0 / a[col][col];
        for row in (col + 1)..n {
            let f = a[row][col] * inv;
            if f != 0.0 {
                for k in col..n {
                    a[row][k] -= f * a[col][k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    // Back-substitution.
    let mut x = [0.0f32; 4];
    for row in (0..n).rev() {
        let mut s = b[row];
        for k in (row + 1)..n {
            s -= a[row][k] * x[k];
        }
        x[row] = s / a[row][row];
    }
    Some(x)
}

/// Active-set block LCP for a manifold's normal impulses.
///
/// * `n`   — number of contacts (1..=4).
/// * `a`   — N×N (Tikhonov-regularised) Delassus matrix: `a[i][j]` = change in contact
///           i's normal velocity per unit normal impulse at contact j. A 4-coplanar-
///           contact manifold over-determines its 3 DOF (1 force + 2 tilt torques), so
///           the raw matrix is rank-deficient; the caller adds a small diagonal so the
///           redundant direction degrades to an even pressure distribution instead of
///           producing a huge garbage impulse.
/// * `rhs` — per-contact driving term `m_scale·(target_i − vn_i) − i_scale·acc_i` (the
///           coupled generalisation of the soft per-contact update). The block solves
///           `A·Δ = rhs` on the active set.
/// * `acc` — current accumulated normal impulses (warm-started, ≥0).
///
/// Returns the new accumulated impulses `λ` (each ≥ 0). The caller applies the delta
/// `λ_i − acc_i` as a normal impulse at contact i. Falls back to the (regularised)
/// diagonal solution if the dense solve still degenerates.
pub(super) fn solve_normal_block(
    n: usize,
    a: &[[f32; 4]; 4],
    rhs_in: &[f32; 4],
    acc: &[f32; 4],
) -> [f32; 4] {
    debug_assert!((1..=4).contains(&n));
    let mut active = [true; 4];

    // At most n drops, plus a hard guard.
    for _ in 0..=n {
        // Assemble the reduced system over the currently-active contacts. Inactive
        // contacts are fixed at λ=0 ⇒ their Δ_j = −acc_j, moved to the RHS.
        let idx: [usize; 4] = {
            let mut t = [0usize; 4];
            let mut m = 0;
            for i in 0..n {
                if active[i] {
                    t[m] = i;
                    m += 1;
                }
            }
            t
        };
        let m = (0..n).filter(|&i| active[i]).count();

        if m == 0 {
            // Everything clamped: λ = 0 for all (each contact separates).
            return [0.0; 4];
        }

        let mut am = [[0.0f32; 4]; 4];
        let mut rhs = [0.0f32; 4];
        for r in 0..m {
            let i = idx[r];
            let mut b = rhs_in[i];
            for j in 0..n {
                if !active[j] {
                    // fixed Δ_j = −acc_j contributes a[i][j]·Δ_j to the constraint
                    b -= a[i][j] * (-acc[j]);
                }
            }
            rhs[r] = b;
            for c in 0..m {
                am[r][c] = a[i][idx[c]];
            }
        }

        let delta = match solve_dense(&mut am, &mut rhs, m) {
            Some(x) => x,
            None => {
                // Degenerate even after regularisation — fall back to the per-contact
                // (diagonal) solve, clamped ≥0. Never worse than the old sequential path.
                let mut lambda = [0.0f32; 4];
                for i in 0..n {
                    let d = if a[i][i] > 1e-12 { rhs_in[i] / a[i][i] } else { 0.0 };
                    lambda[i] = (acc[i] + d).max(0.0);
                }
                return lambda;
            }
        };

        // λ = acc + Δ on the active set, 0 on the clamped set.
        let mut lambda = [0.0f32; 4];
        for r in 0..m {
            let i = idx[r];
            lambda[i] = acc[i] + delta[r];
        }

        // Clamp the single most-negative active λ to 0 and re-solve; if none, done.
        let mut worst = None;
        let mut worst_val = -1e-7f32;
        for r in 0..m {
            let i = idx[r];
            if lambda[i] < worst_val {
                worst_val = lambda[i];
                worst = Some(i);
            }
        }
        match worst {
            Some(i) => active[i] = false,
            None => return lambda,
        }
    }

    // Guard fallback (should be unreachable): clamp whatever we have.
    let mut lambda = [0.0f32; 4];
    for i in 0..n {
        if active[i] {
            let d = if a[i][i] > 1e-12 { rhs_in[i] / a[i][i] } else { 0.0 };
            lambda[i] = (acc[i] + d).max(0.0);
        }
    }
    lambda
}

// ─────────────────────────────────────────────────────────────────────────────
//  Whole-CHAIN direct solve — dynamic-size active-set LCP for a stack island.
// ─────────────────────────────────────────────────────────────────────────────
//
// The per-manifold block solver fixes each patch's intra-manifold torque, but a tall
// column also needs the INTER-manifold support to propagate up the chain — which the
// per-sweep iterative solve does only approximately, leaving a weak buckling creep that
// no iteration count robustly removes (chaotic across builds). Solving the WHOLE chain's
// normal impulses jointly, exactly, each sweep resolves that coupling in one shot →
// iteration-independent → the tall-tower buckling is eliminated by construction.
//
// It is a dense (n×n) mixed LCP over all `n` normal contacts of the island. Callers gate
// on a contact-count cap (O(n³) per active-set step), and only for chain-like islands
// (high support-depth) where buckling actually occurs — wide piles stay on the cheap
// iterative path.

/// Gaussian elimination with partial pivoting on a row-major flat `m×m` matrix `a`,
/// solving `a·x = b` in place (solution left in `b`). Returns false if singular.
fn dense_solve_flat(a: &mut [f32], b: &mut [f32], m: usize) -> bool {
    for col in 0..m {
        let mut piv = col;
        let mut best = a[col * m + col].abs();
        for row in (col + 1)..m {
            let v = a[row * m + col].abs();
            if v > best {
                best = v;
                piv = row;
            }
        }
        if best < 1e-12 {
            return false;
        }
        if piv != col {
            for k in 0..m {
                a.swap(piv * m + k, col * m + k);
            }
            b.swap(piv, col);
        }
        let inv = 1.0 / a[col * m + col];
        for row in (col + 1)..m {
            let f = a[row * m + col] * inv;
            if f != 0.0 {
                for k in col..m {
                    a[row * m + k] -= f * a[col * m + k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    for row in (0..m).rev() {
        let mut s = b[row];
        for k in (row + 1)..m {
            s -= a[row * m + k] * b[k];
        }
        b[row] = s / a[row * m + row];
    }
    true
}

/// Active-set LCP over a whole island's `n` normal contacts. `a` is the dense row-major
/// n×n (regularised) Delassus matrix; `rhs[i]` the soft driving term; `acc` the current
/// (warm-started) impulses. Writes the new impulses `λ ≥ 0` into `lambda`. Solves
/// `A·Δ = rhs` on the active set, clamps the most-negative λ to 0, re-solves — exact for
/// the final active set. Uses thread-local scratch to avoid per-sweep allocation.
pub(super) fn solve_island_normals(
    n: usize,
    a: &[f32],
    rhs: &[f32],
    acc: &[f32],
    lambda: &mut [f32],
) {
    thread_local! {
        static SCRATCH: std::cell::RefCell<(Vec<bool>, Vec<usize>, Vec<f32>, Vec<f32>)> =
            const { std::cell::RefCell::new((Vec::new(), Vec::new(), Vec::new(), Vec::new())) };
    }
    SCRATCH.with(|cell| {
        let (active, idx, am, b) = &mut *cell.borrow_mut();
        active.clear();
        active.resize(n, true);

        let diag_fallback = |lambda: &mut [f32], active: &[bool]| {
            for i in 0..n {
                lambda[i] = if active[i] {
                    let d = if a[i * n + i] > 1e-12 { rhs[i] / a[i * n + i] } else { 0.0 };
                    (acc[i] + d).max(0.0)
                } else {
                    0.0
                };
            }
        };

        for _ in 0..=n {
            idx.clear();
            for i in 0..n {
                if active[i] {
                    idx.push(i);
                }
            }
            let m = idx.len();
            if m == 0 {
                lambda[..n].fill(0.0);
                return;
            }

            am.clear();
            am.resize(m * m, 0.0);
            b.clear();
            b.resize(m, 0.0);
            for r in 0..m {
                let i = idx[r];
                let mut bi = rhs[i];
                for j in 0..n {
                    if !active[j] {
                        bi -= a[i * n + j] * (-acc[j]); // fixed Δ_j = −acc_j
                    }
                }
                b[r] = bi;
                for c in 0..m {
                    am[r * m + c] = a[i * n + idx[c]];
                }
            }

            if !dense_solve_flat(am, b, m) {
                diag_fallback(lambda, active);
                return;
            }

            for i in 0..n {
                if !active[i] {
                    lambda[i] = 0.0;
                }
            }
            for r in 0..m {
                lambda[idx[r]] = acc[idx[r]] + b[r];
            }

            let mut worst = None;
            let mut wv = -1e-7f32;
            for &i in idx.iter() {
                if lambda[i] < wv {
                    wv = lambda[i];
                    worst = Some(i);
                }
            }
            match worst {
                Some(i) => active[i] = false,
                None => return,
            }
        }
        diag_fallback(lambda, active);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_solve_identity() {
        let mut a = [[0.0f32; 4]; 4];
        for i in 0..3 {
            a[i][i] = 1.0;
        }
        let mut b = [2.0, -3.0, 5.0, 0.0];
        let x = solve_dense(&mut a, &mut b, 3).unwrap();
        assert!((x[0] - 2.0).abs() < 1e-6 && (x[1] + 3.0).abs() < 1e-6 && (x[2] - 5.0).abs() < 1e-6);
    }

    #[test]
    fn dense_solve_coupled_2x2() {
        // [[2,1],[1,2]] x = [3,3] → x = [1,1].
        let mut a = [[0.0f32; 4]; 4];
        a[0] = [2.0, 1.0, 0.0, 0.0];
        a[1] = [1.0, 2.0, 0.0, 0.0];
        let mut b = [3.0, 3.0, 0.0, 0.0];
        let x = solve_dense(&mut a, &mut b, 2).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-5 && (x[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dense_solve_singular_none() {
        let mut a = [[0.0f32; 4]; 4];
        a[0] = [1.0, 1.0, 0.0, 0.0];
        a[1] = [1.0, 1.0, 0.0, 0.0]; // rank-deficient
        let mut b = [1.0, 2.0, 0.0, 0.0];
        assert!(solve_dense(&mut a, &mut b, 2).is_none());
    }

    // The `rhs` here is the rigid case: rhs_i = target_i − vn_i (m_scale=1, i_scale=0).
    #[test]
    fn block_all_active_matches_direct_solve() {
        // 2 coupled contacts, both penetrating (approaching). A Δ = rhs = [1,1] →
        // Δ = [0.4,0.4]; both λ>0 → active.
        let mut a = [[0.0f32; 4]; 4];
        a[0] = [2.0, 0.5, 0.0, 0.0];
        a[1] = [0.5, 2.0, 0.0, 0.0];
        let rhs = [1.0, 1.0, 0.0, 0.0];
        let lambda = solve_normal_block(2, &a, &rhs, &[0.0; 4]);
        assert!((lambda[0] - 0.4).abs() < 1e-4, "λ0={}", lambda[0]);
        assert!((lambda[1] - 0.4).abs() < 1e-4, "λ1={}", lambda[1]);
    }

    #[test]
    fn block_clamps_separating_contact() {
        // Contact 0 approaching (rhs>0), contact 1 strongly SEPARATING (rhs<0):
        // the joint solve must clamp contact 1 to λ=0 (unilateral), not pull it back.
        let mut a = [[0.0f32; 4]; 4];
        a[0] = [1.0, 0.2, 0.0, 0.0];
        a[1] = [0.2, 1.0, 0.0, 0.0];
        let rhs = [1.0, -5.0, 0.0, 0.0]; // c1 separating
        let lambda = solve_normal_block(2, &a, &rhs, &[0.0; 4]);
        assert!(lambda[1].abs() < 1e-6, "separating contact must get λ≈0, got {}", lambda[1]);
        assert!((lambda[0] - 1.0).abs() < 1e-4, "λ0={}", lambda[0]); // c0 alone: 1/1
    }

    #[test]
    fn block_restoring_torque_redistributes() {
        // Buckling-relevant: deeper corner (larger rhs) must carry more impulse.
        let mut a = [[0.0f32; 4]; 4];
        a[0] = [1.5, 0.5, 0.0, 0.0];
        a[1] = [0.5, 1.5, 0.0, 0.0];
        let rhs = [2.0, 1.5, 0.0, 0.0]; // corner 0 deeper
        let lambda = solve_normal_block(2, &a, &rhs, &[0.0; 4]);
        // Direct solve: Δ = [1.125, 0.625] → both λ>0, deeper corner carries more.
        assert!(lambda[0] > lambda[1], "deeper corner must carry more impulse: {lambda:?}");
        assert!(lambda[0] > 0.0 && lambda[1] > 0.0, "both active: {lambda:?}");
        // Verify A·λ = rhs.
        let r0 = a[0][0] * lambda[0] + a[0][1] * lambda[1] - rhs[0];
        let r1 = a[1][0] * lambda[0] + a[1][1] * lambda[1] - rhs[1];
        assert!(r0.abs() < 1e-4 && r1.abs() < 1e-4, "residual r0={r0} r1={r1}");
    }

    #[test]
    fn block_single_contact_is_scalar_solve() {
        let mut a = [[0.0f32; 4]; 4];
        a[0][0] = 2.0;
        let rhs = [3.0, 0.0, 0.0, 0.0];
        let lambda = solve_normal_block(1, &a, &rhs, &[0.0; 4]);
        assert!((lambda[0] - 1.5).abs() < 1e-5); // 3/2
    }

    #[test]
    fn island_all_active_exact_linear_solve() {
        // 3-contact coupled chain, all active (rhs>0), no warm-start. λ = A⁻¹·rhs.
        // A = [[2,-0.5,0],[-0.5,2,-0.5],[0,-0.5,2]] (tridiagonal), rhs=[1,1,1].
        let a = [2.0, -0.5, 0.0, -0.5, 2.0, -0.5, 0.0, -0.5, 2.0];
        let rhs = [1.0, 1.0, 1.0];
        let acc = [0.0; 3];
        let mut lambda = [0.0f32; 3];
        solve_island_normals(3, &a, &rhs, &acc, &mut lambda);
        // Verify A·λ = rhs on all (all active since rhs>0 and coupling mild).
        for i in 0..3 {
            let mut s = 0.0;
            for j in 0..3 {
                s += a[i * 3 + j] * lambda[j];
            }
            assert!((s - rhs[i]).abs() < 1e-4, "row {i}: A·λ={s} != {}", rhs[i]);
            assert!(lambda[i] > 0.0, "λ{i}={} should be active", lambda[i]);
        }
    }

    #[test]
    fn island_clamps_separating_contact() {
        // Middle contact separating (rhs<0) → must clamp to 0; neighbours stay active.
        let a = [1.0, 0.1, 0.0, 0.1, 1.0, 0.1, 0.0, 0.1, 1.0];
        let rhs = [1.0, -3.0, 1.0];
        let acc = [0.0; 3];
        let mut lambda = [0.0f32; 3];
        solve_island_normals(3, &a, &rhs, &acc, &mut lambda);
        assert!(lambda[1].abs() < 1e-6, "separating middle must clamp: {lambda:?}");
        assert!(lambda[0] > 0.0 && lambda[2] > 0.0, "ends active: {lambda:?}");
    }

    #[test]
    fn island_matches_block_for_single_manifold() {
        // A 3-contact single-manifold island (dense coupled) must give the same λ as the
        // fixed-size block solver on the same system.
        let a4 = {
            let mut a = [[0.0f32; 4]; 4];
            a[0] = [2.0, 0.5, 0.3, 0.0];
            a[1] = [0.5, 2.0, 0.4, 0.0];
            a[2] = [0.3, 0.4, 2.0, 0.0];
            a
        };
        let rhs = [1.0, 0.8, 1.2, 0.0];
        let acc = [0.0; 4];
        let block = solve_normal_block(3, &a4, &rhs, &acc);
        let a_flat = [
            2.0, 0.5, 0.3, 0.5, 2.0, 0.4, 0.3, 0.4, 2.0,
        ];
        let mut isl = [0.0f32; 3];
        solve_island_normals(3, &a_flat, &[1.0, 0.8, 1.2], &[0.0; 3], &mut isl);
        for i in 0..3 {
            assert!((block[i] - isl[i]).abs() < 1e-4, "mismatch at {i}: {} vs {}", block[i], isl[i]);
        }
    }

    #[test]
    fn block_rank_deficient_regularised_is_bounded() {
        // 4 coplanar contacts over-determine 3 DOF → RAW A is singular (rank 3). With a
        // regularised diagonal the solve must stay BOUNDED (no huge garbage impulse).
        // Build A = ones (the singular part) + reg·I.
        let reg = 0.05f32;
        let mut a = [[1.0f32; 4]; 4];
        for i in 0..4 {
            a[i][i] += reg;
        }
        let rhs = [1.0, 1.0, 1.0, 1.0];
        let lambda = solve_normal_block(4, &a, &rhs, &[0.0; 4]);
        for (i, &l) in lambda.iter().enumerate() {
            assert!(l.is_finite() && l.abs() < 100.0, "λ{i}={l} not bounded");
        }
    }
}
