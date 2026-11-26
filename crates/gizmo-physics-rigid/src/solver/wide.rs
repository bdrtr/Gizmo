//! A colour's blocks solved four at a time.
//!
//! The coloured sweep (`super::parallel`) solves the blocks of a colour in any order, since no two of
//! them share a dynamic body. This packs four of them into one [`WideBlock`] — each a *lane* — with
//! every row quantity laid out across the lanes (`glam::Vec4`, SSE on x86), and solves the four with
//! the same instructions: Box2D v3's wide contact solver, for the 3D block solver. A lane is one
//! block of up to four contacts; a point slot a lane does not use holds zeros and does nothing.
//!
//! Per lane it is the scalar step (`ConstraintSolver::block_group_step`): the normal block through
//! its inverse, `λ = acc + A⁻¹·rhs`, and a lane whose `λ` comes out below −1e-7 (or whose block has no
//! inverse) takes the active-set search on its own; then each point's friction in turn. A block with
//! rolling resistance is not packed — it is solved by the scalar step — nor is the overflow colour,
//! whose blocks may share a body.

use super::tgs::{BlockGroup, Prepared};
use crate::components::Velocity;
use gizmo_math::{Vec3, Vec4};

type F = Vec4;

/// Lanes in a wide block.
pub(super) const LANES: usize = 4;

/// A vector of the four lanes: `x`, `y`, `z` each across them.
#[derive(Clone, Copy, Default)]
struct V3 {
    x: F,
    y: F,
    z: F,
}

impl V3 {
    #[inline]
    fn dot(self, o: V3) -> F {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    fn scale(self, s: F) -> V3 {
        V3 { x: self.x * s, y: self.y * s, z: self.z * s }
    }
    #[inline]
    fn add(self, o: V3) -> V3 {
        V3 { x: self.x + o.x, y: self.y + o.y, z: self.z + o.z }
    }
    #[inline]
    fn sub(self, o: V3) -> V3 {
        V3 { x: self.x - o.x, y: self.y - o.y, z: self.z - o.z }
    }
    #[inline]
    fn set(&mut self, lane: usize, v: Vec3) {
        self.x[lane] = v.x;
        self.y[lane] = v.y;
        self.z[lane] = v.z;
    }
    #[inline]
    fn get(&self, lane: usize) -> Vec3 {
        Vec3::new(self.x[lane], self.y[lane], self.z[lane])
    }
    /// Four vectors, one a lane, built a component at a time.
    #[inline]
    fn of(v: [Vec3; LANES]) -> V3 {
        V3 {
            x: F::new(v[0].x, v[1].x, v[2].x, v[3].x),
            y: F::new(v[0].y, v[1].y, v[2].y, v[3].y),
            z: F::new(v[0].z, v[1].z, v[2].z, v[3].z),
        }
    }
    /// The four lanes as vectors.
    #[inline]
    fn lanes(self) -> [Vec3; LANES] {
        let (x, y, z) = (self.x.to_array(), self.y.to_array(), self.z.to_array());
        [Vec3::new(x[0], y[0], z[0]), Vec3::new(x[1], y[1], z[1]), Vec3::new(x[2], y[2], z[2]), Vec3::new(x[3], y[3], z[3])]
    }
}

/// One point slot of the four lanes.
#[derive(Clone, Copy, Default)]
struct Slot {
    normal: V3,
    t1: V3,
    t2: V3,
    rn_a: V3,
    rn_b: V3,
    in_a: V3,
    in_b: V3,
    rt1_a: V3,
    rt1_b: V3,
    it1_a: V3,
    it1_b: V3,
    rt2_a: V3,
    rt2_b: V3,
    it2_a: V3,
    it2_b: V3,
    pen0: F,
    level: F,
    vn0: F,
    restitution: F,
    k_t1: F,
    k_t2: F,
    friction: F,
    static_friction: F,
    /// 1 where the point slides, 0 where it holds.
    sliding: F,
    acc_n: F,
    acc_t1: F,
    acc_t2: F,
}

/// Four blocks of one colour, solved together. See the module.
#[derive(Clone)]
pub(super) struct WideBlock {
    slots: [Slot; 4],
    inv_m_a: F,
    inv_m_b: F,
    /// `A⁻¹` across the lanes, `inv[r][c]`; a lane without one has `has_inv` false.
    inv: [[F; 4]; 4],
    has_inv: [bool; LANES],
    /// Each lane's block, `None` for an empty lane.
    block: [Option<u32>; LANES],
    n: [usize; LANES],
    /// The most points any lane has: the slots past it are empty in every lane, and not swept.
    used: usize,
    start: [usize; LANES],
    body_a: [usize; LANES],
    body_b: [usize; LANES],
    dyn_a: [bool; LANES],
    dyn_b: [bool; LANES],
}

impl WideBlock {
    /// Packs up to four blocks (`blocks`, indices into `groups`) from `prepared`.
    pub(super) fn build(prepared: &[Prepared], groups: &[BlockGroup], blocks: &[u32]) -> WideBlock {
        debug_assert!(blocks.len() <= LANES);
        let mut w = WideBlock {
            slots: [Slot::default(); 4],
            inv_m_a: F::ZERO,
            inv_m_b: F::ZERO,
            inv: [[F::ZERO; 4]; 4],
            has_inv: [false; LANES],
            block: [None; LANES],
            n: [0; LANES],
            used: 0,
            start: [0; LANES],
            body_a: [0; LANES],
            body_b: [0; LANES],
            dyn_a: [false; LANES],
            dyn_b: [false; LANES],
        };
        for (lane, &gi) in blocks.iter().enumerate() {
            let g = &groups[gi as usize];
            let first = &prepared[g.start];
            w.block[lane] = Some(gi);
            w.n[lane] = g.n;
            w.used = w.used.max(g.n);
            w.start[lane] = g.start;
            w.body_a[lane] = first.idx_a;
            w.body_b[lane] = first.idx_b;
            w.dyn_a[lane] = first.dyn_a;
            w.dyn_b[lane] = first.dyn_b;
            w.inv_m_a[lane] = first.inv_m_a();
            w.inv_m_b[lane] = first.inv_m_b();
            match g.inverse() {
                Some(inv) => {
                    w.has_inv[lane] = true;
                    for r in 0..4 {
                        for c in 0..4 {
                            w.inv[r][c][lane] = inv[r][c];
                        }
                    }
                }
                // One contact: its inverse is a number.
                None if g.n == 1 && g.matrix()[0][0].abs() >= 1e-12 => {
                    w.has_inv[lane] = true;
                    w.inv[0][0][lane] = 1.0 / g.matrix()[0][0];
                }
                None => {}
            }
            for k in 0..g.n {
                let p = &prepared[g.start + k];
                let r = p.row();
                let s = &mut w.slots[k];
                s.normal.set(lane, r.normal);
                s.t1.set(lane, r.t1);
                s.t2.set(lane, r.t2);
                s.rn_a.set(lane, r.rn_a);
                s.rn_b.set(lane, r.rn_b);
                s.in_a.set(lane, r.in_a);
                s.in_b.set(lane, r.in_b);
                s.rt1_a.set(lane, r.rt1_a);
                s.rt1_b.set(lane, r.rt1_b);
                s.it1_a.set(lane, r.it1_a);
                s.it1_b.set(lane, r.it1_b);
                s.rt2_a.set(lane, r.rt2_a);
                s.rt2_b.set(lane, r.rt2_b);
                s.it2_a.set(lane, r.it2_a);
                s.it2_b.set(lane, r.it2_b);
                s.pen0[lane] = r.pen0;
                s.level[lane] = r.level;
                s.vn0[lane] = r.vn0;
                s.restitution[lane] = r.restitution;
                s.k_t1[lane] = r.k_t1;
                s.k_t2[lane] = r.k_t2;
                s.friction[lane] = r.friction;
                s.static_friction[lane] = r.static_friction;
                s.sliding[lane] = if r.sliding { 1.0 } else { 0.0 };
                s.acc_n[lane] = r.acc_n;
                s.acc_t1[lane] = r.acc_t.dot(r.t1);
                s.acc_t2[lane] = r.acc_t.dot(r.t2);
            }
        }
        w
    }

    /// The lanes' velocities: `(linear a, angular a, linear b, angular b)`.
    ///
    /// # Safety
    /// `velocities` points at the island's velocities, and no other thread writes this block's
    /// bodies while it runs (the colouring).
    unsafe fn gather(&self, velocities: *const Velocity) -> (V3, V3, V3, V3) {
        let (mut la, mut wa, mut lb, mut wb) = ([Vec3::ZERO; LANES], [Vec3::ZERO; LANES], [Vec3::ZERO; LANES], [Vec3::ZERO; LANES]);
        for lane in 0..LANES {
            if self.block[lane].is_none() {
                continue;
            }
            // SAFETY: by the function's contract.
            let (va, vb) = unsafe { (&*velocities.add(self.body_a[lane]), &*velocities.add(self.body_b[lane])) };
            (la[lane], wa[lane], lb[lane], wb[lane]) = (va.linear, va.angular, vb.linear, vb.angular);
        }
        (V3::of(la), V3::of(wa), V3::of(lb), V3::of(wb))
    }

    /// Writes the lanes' velocities back to their dynamic bodies.
    ///
    /// # Safety
    /// As [`gather`](Self::gather).
    unsafe fn scatter(&self, velocities: *mut Velocity, (la, wa, lb, wb): (V3, V3, V3, V3)) {
        let (la, wa, lb, wb) = (la.lanes(), wa.lanes(), lb.lanes(), wb.lanes());
        for lane in 0..LANES {
            if self.block[lane].is_none() {
                continue;
            }
            // SAFETY: by the function's contract.
            unsafe {
                if self.dyn_a[lane] {
                    let v = &mut *velocities.add(self.body_a[lane]);
                    v.linear = la[lane];
                    v.angular = wa[lane];
                }
                if self.dyn_b[lane] {
                    let v = &mut *velocities.add(self.body_b[lane]);
                    v.linear = lb[lane];
                    v.angular = wb[lane];
                }
            }
        }
    }

    /// The warm start: each point's last impulses, scaled by `factor`, handed to the bodies.
    ///
    /// # Safety
    /// As [`gather`](Self::gather).
    pub(super) unsafe fn warm(&self, velocities: *mut Velocity, factor: f32) {
        // SAFETY: by the function's contract.
        let (mut la, mut wa, mut lb, mut wb) = unsafe { self.gather(velocities) };
        let f = F::splat(factor);
        for s in &self.slots[..self.used] {
            let (n, t1, t2) = (s.acc_n * f, s.acc_t1 * f, s.acc_t2 * f);
            let linear = s.normal.scale(n).add(s.t1.scale(t1)).add(s.t2.scale(t2));
            la = la.sub(linear.scale(self.inv_m_a));
            wa = wa.sub(s.in_a.scale(n).add(s.it1_a.scale(t1)).add(s.it2_a.scale(t2)));
            lb = lb.add(linear.scale(self.inv_m_b));
            wb = wb.add(s.in_b.scale(n).add(s.it1_b.scale(t1)).add(s.it2_b.scale(t2)));
        }
        // SAFETY: by the function's contract.
        unsafe { self.scatter(velocities, (la, wa, lb, wb)) };
    }

    /// One sweep of the four blocks: the biased one (`use_bias`) or the relax one
    /// (`apply_restitution`), as `block_group_step`.
    ///
    /// # Safety
    /// As [`gather`](Self::gather); `dp` points at the island's position deltas, read only.
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn step(
        &mut self,
        solver: &super::ConstraintSolver,
        groups: &[BlockGroup],
        velocities: *mut Velocity,
        dp: *const (Vec3, Vec3),
        apply_restitution: bool,
        use_bias: bool,
        bias_rate: f32,
        inv_dt: f32,
    ) {
        // SAFETY: by the function's contract.
        let (mut la, mut wa, mut lb, mut wb) = unsafe { self.gather(velocities) };
        let (mut dl, mut da_w, mut db_w) = ([Vec3::ZERO; LANES], [Vec3::ZERO; LANES], [Vec3::ZERO; LANES]);
        for lane in 0..LANES {
            if self.block[lane].is_none() {
                continue;
            }
            // SAFETY: by the function's contract.
            let (da, db) = unsafe { (&*dp.add(self.body_a[lane]), &*dp.add(self.body_b[lane])) };
            (dl[lane], da_w[lane], db_w[lane]) = (db.0 - da.0, da.1, db.1);
        }
        let (dlin, dwa, dwb) = (V3::of(dl), V3::of(da_w), V3::of(db_w));

        let zero = F::ZERO;
        let threshold = F::splat(-solver.restitution_velocity_threshold);
        let slop = F::splat(solver.slop);
        let mut rhs = [zero; 4];
        let mut acc = [zero; 4];
        let rel = lb.sub(la);
        let used = self.used;
        for (k, s) in self.slots[..used].iter().enumerate() {
            let penetration = s.pen0 - (dlin.dot(s.normal) + dwb.dot(s.rn_b) - dwa.dot(s.rn_a));
            let vn = rel.dot(s.normal) + wb.dot(s.rn_b) - wa.dot(s.rn_a);
            let tvn = if apply_restitution {
                let bounces = s.restitution.cmpgt(zero) & (s.pen0.cmpge(zero) | s.acc_n.cmpgt(zero)) & s.vn0.cmplt(threshold);
                F::select(bounces, -s.restitution * s.vn0, zero)
            } else {
                zero
            };
            let soft = if use_bias {
                F::select(penetration.cmpgt(slop), (F::splat(bias_rate) * (penetration - slop)).min(F::splat(solver.max_bias_velocity)), zero)
            } else {
                zero
            };
            // A point shallower than its manifold's deepest may sink to it (`super::resting_level`).
            let gap_or_soft = F::select(penetration.cmplt(zero), penetration * F::splat(inv_dt), soft);
            let sinks = penetration.cmplt(s.level - F::splat(super::LEVEL_NOISE));
            let bias = F::select(tvn.cmpgt(zero), zero, F::select(sinks, (penetration - s.level) * F::splat(inv_dt), gap_or_soft));
            rhs[k] = tvn + bias - vn;
            acc[k] = s.acc_n;
        }

        // λ = acc + A⁻¹·rhs, four lanes at once.
        let mut lambda = acc;
        for (r, l) in lambda[..used].iter_mut().enumerate() {
            let mut d = zero;
            for (c, rhs_c) in rhs[..used].iter().enumerate() {
                d += self.inv[r][c] * *rhs_c;
            }
            *l += d;
        }
        // A lane with a contact pulling, or no inverse: the active-set search, on its own.
        for lane in 0..LANES {
            let Some(gi) = self.block[lane] else { continue };
            let n = self.n[lane];
            let pushing = self.has_inv[lane] && (0..n).all(|k| lambda[k][lane] >= -1e-7);
            if pushing {
                continue;
            }
            let lane_of = |v: &[F; 4]| [v[0][lane], v[1][lane], v[2][lane], v[3][lane]];
            let found = super::block::solve_normal_block_with(n, groups[gi as usize].matrix(), None, &lane_of(&rhs), &lane_of(&acc));
            for (k, l) in lambda.iter_mut().enumerate() {
                l[lane] = found[k];
            }
        }

        for (k, s) in self.slots[..used].iter_mut().enumerate() {
            let delta = lambda[k] - s.acc_n;
            s.acc_n = lambda[k];
            la = la.sub(s.normal.scale(delta * self.inv_m_a));
            wa = wa.sub(s.in_a.scale(delta));
            lb = lb.add(s.normal.scale(delta * self.inv_m_b));
            wb = wb.add(s.in_b.scale(delta));
        }

        let tiny = F::splat(1e-8);
        for s in self.slots[..used].iter_mut() {
            let rel = lb.sub(la);
            let (a1, a2) = (s.acc_t1, s.acc_t2);
            let v1 = rel.dot(s.t1) + wb.dot(s.rt1_b) - wa.dot(s.rt1_a);
            let v2 = rel.dot(s.t2) + wb.dot(s.rt2_b) - wa.dot(s.rt2_a);
            let mut new1 = F::select(s.k_t1.cmpgt(tiny), a1 - v1 / s.k_t1, a1);
            let mut new2 = F::select(s.k_t2.cmpgt(tiny), a2 - v2 / s.k_t2, a2);
            let mu = F::select(s.sliding.cmpgt(zero), s.friction, s.static_friction);
            let limit = mu * s.acc_n.abs();
            let mag = (new1 * new1 + new2 * new2).sqrt();
            let scale = F::select(mag.cmpgt(limit) & mag.cmpgt(F::splat(1e-12)), limit / mag, F::ONE);
            new1 *= scale;
            new2 *= scale;
            let (d1, d2) = (new1 - a1, new2 - a2);
            s.acc_t1 = new1;
            s.acc_t2 = new2;
            let imp_t = s.t1.scale(d1).add(s.t2.scale(d2));
            la = la.sub(imp_t.scale(self.inv_m_a));
            wa = wa.sub(s.it1_a.scale(d1).add(s.it2_a.scale(d2)));
            lb = lb.add(imp_t.scale(self.inv_m_b));
            wb = wb.add(s.it1_b.scale(d1).add(s.it2_b.scale(d2)));
        }

        // SAFETY: by the function's contract.
        unsafe { self.scatter(velocities, (la, wa, lb, wb)) };
    }

    /// The impulses back into the blocks' rows, for the manifolds' warm start next substep.
    pub(super) fn write_back(&self, prepared: &mut [Prepared]) {
        for lane in 0..LANES {
            if self.block[lane].is_none() {
                continue;
            }
            for k in 0..self.n[lane] {
                let s = &self.slots[k];
                let (t1, t2) = (s.t1.get(lane), s.t2.get(lane));
                prepared[self.start[lane] + k].set_impulses(s.acc_n[lane], t1 * s.acc_t1[lane] + t2 * s.acc_t2[lane]);
            }
        }
    }
}
