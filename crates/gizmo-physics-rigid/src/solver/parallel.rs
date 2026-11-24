//! A large island's contact sweeps on several threads.
//!
//! The solver's sweeps are Gauss-Seidel: each manifold's block reads the velocities the blocks before
//! it wrote. Islands are solved in parallel with one another, but an island is one thread's — and a
//! pile is one island. Measured 2026-11-24 against Godot 4.7's Jolt (`gizmo-godot-bench`): a
//! thousand boxes in a pile, 15.6 of the frame's 25.8 ms in the sweeps, and the same 13.6 ms on one
//! thread as on sixteen.
//!
//! What Box2D v3 and Jolt do, and this does: **colour** the island's blocks so that no two blocks of
//! a colour share a dynamic body, and sweep colour by colour. The blocks of one colour touch
//! disjoint velocities, so they can be solved at once, and in any order — the result does not
//! depend on how many threads there are or which took which block. A static or kinematic body is
//! read by many blocks and written by none, so it does not constrain the colouring.
//!
//! The order changes, though: a sweep is now "colour 0's blocks, then colour 1's, …" instead of the
//! support order bottom-up, so an island solved this way converges differently from the same island
//! solved serially. Which islands take this path is decided by their size alone
//! ([`ConstraintSolver::parallel_island_blocks`](super::ConstraintSolver::parallel_island_blocks)),
//! never by the thread count, so a given world steps the same on every machine's thread pool.
//!
//! **No barrier.** The sweep is a list of stages (a colour of a sweep, or the position integration
//! between sweeps), each cut into chunks that can run in any order. Workers take tickets from one
//! counter; a ticket names a chunk, and its holder waits only until every chunk of the stages
//! before it is done. A worker that starts late — its thread busy with another island — just takes
//! a later ticket, and the calling thread takes tickets too, so the solve finishes even if no
//! helper ever starts.

use super::tgs::{BlockGroup, Prepared};
use super::wide::{WideBlock, LANES};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
#[cfg(target_arch = "wasm32")]
use crate::parallel_compat::*;
use super::ConstraintSolver;
use crate::components::Velocity;
use gizmo_math::Vec3;
use std::sync::atomic::{AtomicUsize, Ordering};

/// More colours than this and the rest of the blocks go to one serial overflow stage. A pile's
/// boxes touch a handful of neighbours each; 64 is far past what a contact graph needs.
const MAX_COLORS: u32 = 64;
/// Blocks per chunk at least. Smaller chunks spread a colour over more workers; a ticket is two
/// atomic operations on lines every worker touches, so below this a chunk costs more to hand out
/// than to solve. Measured 2026-11-25 on gizmo-godot-bench at 60 Hz (three runs each): 32 → 8
/// took the 1000-box pile 4.7 → 4.3 ms a frame and 4000 spheres 12.2 → 11.8; 4 was no better,
/// 64 and 128 worse (5.2, 6.4 ms on the pile).
const MIN_BLOCKS_PER_CHUNK: usize = 8;
/// Whether the target has the SIMD a wide block is built for (`glam::Vec4`): SSE on x86-64, NEON on
/// AArch64, `simd128` on wasm. Without it the four lanes are four scalar sums and packing is a loss.
const WIDE_SIMD: bool = cfg!(any(target_arch = "x86_64", target_arch = "aarch64", target_feature = "simd128"));

/// Items per chunk at least, an item being a wide block (four blocks) or a block solved alone: the
/// same two-wide-block floor as `MIN_BLOCKS_PER_CHUNK`'s eight blocks.
const MIN_ITEMS_PER_CHUNK: usize = MIN_BLOCKS_PER_CHUNK / LANES;
/// Chunks per worker in a colour: a few, so that a worker that started late or ran into slower
/// blocks does not hold the stage up.
const CHUNKS_PER_WORKER: usize = 2;
/// Bodies per chunk of the position integration between sweeps.
const BODIES_PER_CHUNK: usize = 128;
/// Helper threads at most, besides the caller. Past this, measured on the 1000-box pile, the spin
/// waits between stages cost more than the extra hands save.
#[cfg(not(target_arch = "wasm32"))]
const MAX_WORKERS: usize = 8;

/// An island's blocks by colour: `order[starts[c]..starts[c + 1]]` are colour `c`'s, in the
/// island's support order. The last colour is the overflow when there is one
/// (`overflow` is true): its blocks may share bodies, and it is swept by one thread.
pub(super) struct Coloring {
    order: Vec<u32>,
    starts: Vec<usize>,
    overflow: bool,
}

impl Coloring {
    fn colors(&self) -> usize {
        self.starts.len() - 1
    }

    fn color(&self, c: usize) -> &[u32] {
        &self.order[self.starts[c]..self.starts[c + 1]]
    }
}

/// Greedy colouring in the blocks' own (support) order: each block takes the lowest colour neither
/// of its dynamic bodies has yet. Deterministic: it depends on the blocks and their order alone.
pub(super) fn color_blocks(prepared: &[Prepared], groups: &[BlockGroup], island_bodies: &[usize], bodies: usize) -> Coloring {
    thread_local! {
        static USED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    USED.with(|cell| {
        let mut used = cell.borrow_mut();
        if used.len() < bodies {
            used.resize(bodies, 0);
        }
        for &i in island_bodies {
            used[i] = 0;
        }
        let overflow_color = MAX_COLORS as usize;
        let mut color_of = Vec::with_capacity(groups.len());
        let mut counts = vec![0usize; overflow_color + 1];
        for g in groups {
            let p = &prepared[g.start];
            let a = if p.dyn_a { used[p.idx_a] } else { 0 };
            let b = if p.dyn_b { used[p.idx_b] } else { 0 };
            let free = (!(a | b)).trailing_zeros();
            let c = if free < MAX_COLORS {
                let bit = 1u64 << free;
                if p.dyn_a {
                    used[p.idx_a] |= bit;
                }
                if p.dyn_b {
                    used[p.idx_b] |= bit;
                }
                free as usize
            } else {
                overflow_color
            };
            color_of.push(c);
            counts[c] += 1;
        }
        // Colours in use, in order; the overflow last.
        let overflow = counts[overflow_color] > 0;
        let last_color = counts[..overflow_color].iter().rposition(|&n| n > 0).map_or(0, |c| c + 1);
        let slots: Vec<usize> = (0..last_color).chain(overflow.then_some(overflow_color)).collect();
        let mut starts = Vec::with_capacity(slots.len() + 1);
        let mut slot_of = vec![usize::MAX; overflow_color + 1];
        let mut at = 0;
        for (s, &c) in slots.iter().enumerate() {
            starts.push(at);
            slot_of[c] = s;
            at += counts[c];
        }
        starts.push(at);
        let mut fill = starts.clone();
        let mut order = vec![0u32; groups.len()];
        for (gi, &c) in color_of.iter().enumerate() {
            let s = slot_of[c];
            order[fill[s]] = gi as u32;
            fill[s] += 1;
        }
        Coloring { order, starts, overflow }
    })
}

/// A raw pointer that may cross threads. Every use below writes only what the colouring or the
/// chunking gives that thread alone.
struct Shared<T>(*mut T);
// By hand: a derive would ask `T: Copy`, and the pointer is copyable whatever it points at.
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Shared<T> {}
impl<T> Shared<T> {
    /// The pointer, through a method so that a closure captures the whole `Shared` (which is
    /// `Sync`) rather than its field (which is not).
    fn at(self, i: usize) -> *mut T {
        // SAFETY: callers index within the slice the pointer was taken from.
        unsafe { self.0.add(i) }
    }
}
// SAFETY: see the type — the stages hand disjoint elements to the threads that touch them.
unsafe impl<T> Send for Shared<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Shared<T> {}

#[derive(Clone, Copy)]
enum Stage {
    /// One colour's blocks; `biased` is the biased pass (else the relax pass, with restitution),
    /// `reverse` the overflow's order.
    Color { color: usize, biased: bool, reverse: bool },
    /// `dp += v · h` for the bodies the sweeps integrate.
    Integrate,
    /// One colour's blocks hand their last step's impulses back to their bodies (the warm start),
    /// before the first sweep.
    Warm { color: usize },
}

/// What the island's coloured solve needs besides `self`.
pub(super) struct ColoredSweeps<'a> {
    pub(super) prepared: &'a mut [Prepared],
    pub(super) groups: &'a [BlockGroup],
    pub(super) coloring: &'a Coloring,
    pub(super) velocities: &'a mut [Velocity],
    pub(super) dp: &'a mut [(Vec3, Vec3)],
    /// The bodies whose `dp` the biased sweeps integrate (real contact bodies, dynamic).
    pub(super) integrate: &'a [usize],
    pub(super) n_iterations: usize,
    pub(super) h: f32,
    pub(super) bias_rate: f32,
    pub(super) inv_dt: f32,
    /// The warm start's factor, applied colour by colour before the sweeps; 0 for none.
    pub(super) warm: f32,
}

impl ConstraintSolver {
    /// The biased sweeps with their position integration, then the relax sweeps — what
    /// `solve_contacts_tgs` does for an island without joints, colour by colour.
    pub(super) fn sweep_colored(&self, s: ColoredSweeps<'_>) {
        let colors = s.coloring.colors();
        let overflow = s.coloring.overflow.then(|| colors - 1);
        let workers = workers();

        // Each colour's work: its blocks packed four to a wide block (`super::wide`), then those it
        // solves one at a time — a block with rolling resistance, and every block of the overflow
        // colour, whose blocks may share a body. Packed colour by colour on several threads. Without
        // SIMD (wasm without `simd128`), or with `wide_blocks` off, every block is solved alone.
        let wide_ok = self.wide_blocks && WIDE_SIMD;
        let (packed, single): (Vec<Vec<u32>>, Vec<Vec<u32>>) = (0..colors)
            .map(|c| {
                let blocks = s.coloring.color(c);
                if Some(c) == overflow || !wide_ok {
                    return (Vec::new(), blocks.to_vec());
                }
                blocks.iter().partition(|&&gi| {
                    let g = &s.groups[gi as usize];
                    !s.prepared[g.start..g.start + g.n].iter().any(|p| p.rolls())
                })
            })
            .unzip();
        let (prepared_ro, groups_ro) = (&*s.prepared, s.groups);
        let mut wide: Vec<Vec<WideBlock>> = packed
            .par_iter()
            .map(|blocks| blocks.chunks(LANES).map(|four| WideBlock::build(prepared_ro, groups_ro, four)).collect())
            .collect();
        let item_count: Vec<usize> = (0..colors).map(|c| wide[c].len() + single[c].len()).collect();
        let items_of = |c: usize| item_count[c];
        // Each colour's chunk size, in items (a wide block or a single block): its items spread over
        // the workers a few chunks each. The size decides only who solves what, not the result.
        let chunk_len: Vec<usize> = (0..colors)
            .map(|c| {
                let n = items_of(c);
                if Some(c) == overflow {
                    n.max(1)
                } else {
                    n.div_ceil(workers * CHUNKS_PER_WORKER).max(MIN_ITEMS_PER_CHUNK)
                }
            })
            .collect();
        let chunks_of = |c: usize| items_of(c).div_ceil(chunk_len[c]);
        // Odd sweeps go through the colours backwards, as the serial sweep goes through the blocks.
        let mut stages: Vec<(Stage, usize)> = Vec::new();
        let sweep = |stages: &mut Vec<(Stage, usize)>, iter: usize, biased: bool| {
            let reverse = iter % 2 == 1;
            for k in 0..colors {
                let color = if reverse { colors - 1 - k } else { k };
                stages.push((Stage::Color { color, biased, reverse }, chunks_of(color)));
            }
        };
        let integrate_chunks = s.integrate.len().div_ceil(BODIES_PER_CHUNK);
        // The warm start, colour by colour: the blocks of a colour share no dynamic body, so they
        // hand their impulses back at once. It was a pass over the island's manifolds on one thread
        // before the sweeps.
        if s.warm > 0.0 {
            for color in 0..colors {
                stages.push((Stage::Warm { color }, chunks_of(color)));
            }
        }
        for iter in 0..s.n_iterations {
            sweep(&mut stages, iter, true);
            if integrate_chunks > 0 {
                stages.push((Stage::Integrate, integrate_chunks));
            }
        }
        for iter in 0..self.relax_iterations {
            sweep(&mut stages, iter, false);
        }

        let wide_at: Vec<Shared<WideBlock>> = wide.iter_mut().map(|w| Shared(w.as_mut_ptr())).collect();
        let prepared = Shared(s.prepared.as_mut_ptr());
        let velocities = Shared(s.velocities.as_mut_ptr());
        let dp = Shared(s.dp.as_mut_ptr());
        let (groups, integrate, h) = (s.groups, s.integrate, s.h);
        let (bias_rate, inv_dt, warm) = (s.bias_rate, s.inv_dt, s.warm);
        let wide_len: Vec<usize> = (0..colors).map(|c| packed[c].len().div_ceil(LANES)).collect();
        let work = |stage: Stage, chunk: usize| match stage {
            Stage::Color { color, biased, reverse } => {
                let len = chunk_len[color];
                let range = chunk * len..((chunk + 1) * len).min(items_of(color));
                let step = |gi: u32| {
                    let g = &groups[gi as usize];
                    // SAFETY: a block's run of `prepared` belongs to that block alone, and no other
                    // block of this colour has either of its dynamic bodies (`color_blocks`); the
                    // overflow colour runs as one chunk. Its static or kinematic end is only read,
                    // here and by every other block — nothing writes it during the sweeps.
                    unsafe {
                        let run = std::slice::from_raw_parts_mut(prepared.at(g.start), g.n);
                        let (ia, ib) = (run[0].idx_a, run[0].idx_b);
                        let (dyn_a, dyn_b) = (run[0].dyn_a, run[0].dyn_b);
                        let mut pair = [velocities.at(ia).read(), velocities.at(ib).read()];
                        let dp_ab = (dp.at(ia).read(), dp.at(ib).read());
                        self.block_group_step(g, run, &mut pair, dp_ab, !biased, biased, bias_rate, inv_dt);
                        if dyn_a {
                            velocities.at(ia).write(pair[0]);
                        }
                        if dyn_b {
                            velocities.at(ib).write(pair[1]);
                        }
                    }
                };
                let item = |k: usize| {
                    if k < wide_len[color] {
                        // SAFETY: a wide block is in one chunk of its colour; its lanes' bodies are
                        // its blocks', which no other block of the colour has.
                        unsafe {
                            (*wide_at[color].at(k)).step(self, groups, velocities.at(0), dp.at(0), !biased, biased, bias_rate, inv_dt);
                        }
                    } else {
                        step(single[color][k - wide_len[color]]);
                    }
                };
                if reverse && Some(color) == overflow {
                    range.rev().for_each(item);
                } else {
                    range.for_each(item);
                }
            }
            Stage::Warm { color } => {
                let len = chunk_len[color];
                for k in chunk * len..((chunk + 1) * len).min(items_of(color)) {
                    if k < wide_len[color] {
                        // SAFETY: as for `Stage::Color`.
                        unsafe { (*wide_at[color].at(k)).warm(velocities.at(0), warm) };
                        continue;
                    }
                    let g = &groups[single[color][k - wide_len[color]] as usize];
                    // SAFETY: as for `Stage::Color` — a colour's blocks share no dynamic body, and the
                    // overflow colour is one chunk.
                    unsafe {
                        let run = std::slice::from_raw_parts(prepared.at(g.start), g.n);
                        let (ia, ib) = (run[0].idx_a, run[0].idx_b);
                        let mut pair = [velocities.at(ia).read(), velocities.at(ib).read()];
                        Self::block_warm_start(run, &mut pair, warm);
                        if run[0].dyn_a {
                            velocities.at(ia).write(pair[0]);
                        }
                        if run[0].dyn_b {
                            velocities.at(ib).write(pair[1]);
                        }
                    }
                }
            }
            Stage::Integrate => {
                let from = chunk * BODIES_PER_CHUNK;
                for &i in &integrate[from..(from + BODIES_PER_CHUNK).min(integrate.len())] {
                    // SAFETY: each body is in one chunk of `integrate`, and no block runs during
                    // this stage.
                    unsafe {
                        let v = velocities.at(i).read();
                        let d = &mut *dp.at(i);
                        d.0 += v.linear * h;
                        d.1 += v.angular * h;
                    }
                }
            }
        };
        run_stages(&stages, workers, &work);
        // The wide blocks' impulses back into their rows.
        for w in wide.iter().flatten() {
            w.write_back(s.prepared);
        }
    }
}

/// The threads a coloured solve uses, the caller included. One on wasm, which has no rayon: the
/// coloured order is the same there, on one thread.
fn workers() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    let n = rayon::current_num_threads().clamp(1, MAX_WORKERS + 1);
    #[cfg(target_arch = "wasm32")]
    let n = 1;
    n
}

/// Runs `stages` in order, each stage's chunks on whichever workers take them (see the module).
fn run_stages(stages: &[(Stage, usize)], workers: usize, work: &(dyn Fn(Stage, usize) + Sync)) {
    // `ends[s]`: the tickets of stages `0..=s`.
    let mut ends = Vec::with_capacity(stages.len());
    let mut total = 0;
    for &(_, chunks) in stages {
        total += chunks;
        ends.push(total);
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let worker = || {
        let mut stage = 0;
        loop {
            let ticket = next.fetch_add(1, Ordering::Relaxed);
            if ticket >= total {
                return;
            }
            while ends[stage] <= ticket {
                stage += 1;
            }
            let begin = if stage == 0 { 0 } else { ends[stage - 1] };
            // Every chunk before this stage done: its writes are visible (Acquire pairs with the
            // Release of each `done` increment).
            let mut spins = 0u32;
            while done.load(Ordering::Acquire) < begin {
                spins += 1;
                if spins < 256 {
                    std::hint::spin_loop();
                } else {
                    std::thread::yield_now();
                }
            }
            work(stages[stage].0, ticket - begin);
            done.fetch_add(1, Ordering::Release);
        }
    };
    #[cfg(target_arch = "wasm32")]
    {
        let _ = workers;
        worker();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if workers <= 1 {
        worker();
    } else {
        rayon::scope(|scope| {
            for _ in 1..workers {
                scope.spawn(|_| worker());
            }
            worker();
        });
    }
}
