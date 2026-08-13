//! Sub-phase timers, at function granularity.
//!
//! The four phase timers on [`PhysicsMetrics`](crate::island::PhysicsMetrics) localise a
//! frame to broadphase / narrowphase / solver / integration, which is enough to say *which
//! quarter* and not enough to say *what*. When the solver came out ~2.5× behind Rapier per
//! substep with the cost spread evenly rather than concentrated, that was the end of what
//! those four numbers could answer — and no system profiler is available here (`perf`,
//! `valgrind` and `heaptrack` are all absent on the dev machine).
//!
//! **Why globals rather than a field.** `ConstraintSolver::solve_contacts` takes `&self` on
//! purpose: islands are solved in parallel from one shared configuration value, and that
//! value is `Copy`. There is nowhere on it to put a mutable accumulator without changing
//! that contract, so the counters are module-level atomics — the same shape the phase
//! timers would need if they were ever pushed inside the parallel region.
//!
//! **Determinism.** Nothing here feeds the simulation. The counters are written and read,
//! never branched on, so a run with the timers and a run without produce the same state
//! hash. That is the same argument the existing phase timers already make for themselves.
//!
//! **Cost.** Every scope here wraps per-*island* or per-*substep* work, never per-contact
//! or per-body: at a few hundred `Instant::now()` pairs a frame, against contacts numbered
//! in the thousands, the measurement stays far below what it is measuring.

use std::sync::atomic::{AtomicU64, Ordering};

// `std::time::Instant::now()` panics on wasm; `web_time` bridges to the browser clock.
// Same split, and same reason, as `world::step`.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// One accumulator per sub-phase, in nanoseconds, summed across islands and substeps.
pub(crate) struct Phases {
    /// Support-ordering the island's manifolds and computing its depth.
    pub order: AtomicU64,
    /// Building the solver's per-contact `Prepared` rows from the manifolds.
    pub prepare: AtomicU64,
    /// The biased sweeps — the main constraint iteration.
    pub sweep: AtomicU64,
    /// The relax pass, where restitution is applied.
    pub relax: AtomicU64,
    /// Narrowphase shape dispatch: the actual collision maths.
    pub dispatch: AtomicU64,
    /// Turning narrowphase output into manifolds: materials, warm-start, cache, events.
    pub manifold: AtomicU64,
}

pub(crate) static PHASES: Phases = Phases {
    order: AtomicU64::new(0),
    prepare: AtomicU64::new(0),
    sweep: AtomicU64::new(0),
    relax: AtomicU64::new(0),
    dispatch: AtomicU64::new(0),
    manifold: AtomicU64::new(0),
};

impl Phases {
    /// Zero every counter — called once at the top of a step, like the phase timers.
    pub fn reset(&self) {
        for c in self.all() {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// Milliseconds accumulated in one counter.
    pub fn ms(c: &AtomicU64) -> f32 {
        c.load(Ordering::Relaxed) as f32 / 1.0e6
    }

    fn all(&self) -> [&AtomicU64; 6] {
        [
            &self.order,
            &self.prepare,
            &self.sweep,
            &self.relax,
            &self.dispatch,
            &self.manifold,
        ]
    }
}

/// Times its own lifetime into a counter.
///
/// Deliberately not `#[must_use]`-free: the guard has to be bound to a name, because
/// `let _ = Scope::new(..)` drops it immediately and would record nothing. Bind it as
/// `let _t = ...` and let the scope end it.
pub(crate) struct Scope {
    counter: &'static AtomicU64,
    start: Instant,
}

impl Scope {
    pub fn new(counter: &'static AtomicU64) -> Self {
        Self { counter, start: Instant::now() }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos() as u64;
        self.counter.fetch_add(ns, Ordering::Relaxed);
    }
}
