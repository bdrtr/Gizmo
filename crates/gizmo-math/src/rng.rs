//! A small seeded random-number generator, owned by the engine.
//!
//! Before this existed every game that wanted a spread of positions carried its own: four-line
//! xorshifts in `spotlight`, `parallel_query`, `change_detection`, `kristal_avcisi` and the
//! particle emitter, `rand::rng()` in `showcase`, `StdRng::seed_from_u64` in `rpg_demo` and
//! `hill_climb`. The xorshifts all had the same trap written next to them — zero is a fixed point,
//! so the seed had to be forced odd — and the `rand` ones put a dependency's types in game code.
//!
//! # What it is
//!
//! **SplitMix64**: 64 bits of state, one add and three multiply-xorshift rounds per output. It
//! passes BigCrush, every seed is valid (zero included), and it is a pure function of its state,
//! so the same seed gives the same sequence on every platform — the integer path is exact, and
//! [`Rng::f32`] is built from integer bits with one exact multiply, so it is too.
//!
//! # What it is not
//!
//! Not cryptographic: the state can be recovered from a few outputs. Use the OS for secrets.
//!
//! It is also not a global. An engine-wide "random" call would make determinism depend on who
//! called it first; an `Rng` is a value you own — put one in a resource, keep one per system,
//! or [`fork`](Rng::fork) one per entity.

use glam::{Vec2, Vec3};

/// A seeded SplitMix64 generator — see the [module docs](self).
///
/// ```
/// use gizmo_math::Rng;
///
/// let mut rng = Rng::new(42);
/// let x = rng.range(-5.0..5.0);          // f32 in [-5, 5)
/// let lane = rng.range(0..3);            // i32 in {0, 1, 2}
/// let card = rng.pick(&["a", "b", "c"]); // Some(&"a" | &"b" | &"c")
/// assert!((-5.0..5.0).contains(&x) && (0..3).contains(&lane) && card.is_some());
///
/// // The same seed replays the same sequence.
/// assert_eq!(Rng::new(7).u64(), Rng::new(7).u64());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rng {
    state: u64,
}

/// The golden-ratio increment SplitMix64 steps by.
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64's output function: a bijective scramble of one 64-bit word.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    /// A generator seeded with `seed`. Every value is a valid seed, zero included.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// A generator for one of many independent streams under one seed — a frame and an entity,
    /// a level and a chunk.
    ///
    /// The point is combining two numbers without the collisions a hand-rolled combination has:
    /// `seed + stream` makes (frame 1, entity 2) the same stream as (frame 2, entity 1), and
    /// `seed ^ stream` does the same for (3, 5) and (5, 3). Here the stream is scrambled before
    /// it meets the seed, so swapping the two gives a different generator.
    #[must_use]
    pub fn with_stream(seed: u64, stream: u64) -> Self {
        Self { state: mix(seed ^ mix(stream.wrapping_add(GAMMA))) }
    }

    /// A child generator, independent of this one from here on, and this one advanced by one
    /// step. For handing each spawned thing its own stream without them sharing one.
    #[must_use]
    pub fn fork(&mut self) -> Self {
        Self { state: mix(self.u64() ^ GAMMA) }
    }

    /// The next 64 random bits.
    pub fn u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(GAMMA);
        mix(self.state)
    }

    /// The next 32 random bits — the high half of [`u64`](Self::u64), which is the better half.
    pub fn u32(&mut self) -> u32 {
        (self.u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)`: 24 random bits, exactly representable, so `1.0` is never returned.
    pub fn f32(&mut self) -> f32 {
        (self.u64() >> 40) as f32 * (1.0 / (1u32 << 24) as f32)
    }

    /// Uniform in `[0, 1)` with 53 random bits.
    pub fn f64(&mut self) -> f64 {
        (self.u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[-1, 1)` — the jitter most effects want.
    pub fn signed(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    /// A value from `range`: `a..b` or `a..=b`, of `f32`, `i32`, `u32`, `i64`, `u64` or `usize`.
    ///
    /// **No `f64` ranges, on purpose.** With an `f64` impl beside the `f32` one, a bare literal
    /// range — `rng.range(-5.0..5.0)`, the way every caller writes it — has two candidates, falls
    /// back to `f64`, and does not compile where an `f32` is wanted; the first four call sites
    /// converted to this type all failed that way. With `f32` the only float impl, the literal
    /// infers. Scale [`f64`](Self::f64) for a double.
    ///
    /// # Panics
    ///
    /// On an empty range — `5..5`, `3.0..1.0` — because there is no value to return and a
    /// silent `a` hides the bug that produced the range.
    pub fn range<R: SampleRange>(&mut self, range: R) -> R::Output {
        range.sample(self)
    }

    /// A fair coin.
    pub fn bool(&mut self) -> bool {
        self.u64() >> 63 == 1
    }

    /// `true` with probability `p` (clamped to `[0, 1]`: `chance(0.0)` is never, `chance(1.0)`
    /// always).
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// One element of `items`, uniformly, or `None` for an empty slice.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(self.below(items.len() as u64) as usize)
        }
    }

    /// Shuffles `items` in place, every order equally likely (Fisher–Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// A point uniform in the axis-aligned box from `min` to `max`.
    pub fn in_box(&mut self, min: Vec3, max: Vec3) -> Vec3 {
        Vec3::new(self.f32(), self.f32(), self.f32()) * (max - min) + min
    }

    /// A direction uniform on the unit circle.
    pub fn unit_vec2(&mut self) -> Vec2 {
        let a = self.f32() * std::f32::consts::TAU;
        Vec2::new(a.cos(), a.sin())
    }

    /// A direction uniform on the unit sphere — uniform in area, not in angle, so it does not
    /// bunch up at the poles the way two random angles do.
    pub fn unit_vec3(&mut self) -> Vec3 {
        let z = self.signed();
        let a = self.f32() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    }

    /// A point uniform inside the unit ball.
    pub fn in_unit_sphere(&mut self) -> Vec3 {
        // The cube root makes the density uniform in volume rather than in radius.
        self.unit_vec3() * self.f32().cbrt()
    }

    /// Uniform in `0..n` for `n > 0`, by multiply-high (Lemire) with rejection, so there is no
    /// modulo bias at any `n`.
    fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let threshold = n.wrapping_neg() % n;
        loop {
            let m = u128::from(self.u64()) * u128::from(n);
            if (m as u64) >= threshold {
                return (m >> 64) as u64;
            }
        }
    }
}

/// A range [`Rng::range`] can sample — `a..b` and `a..=b` over the engine's number types.
///
/// Sealed: the set is the one `Rng` knows how to sample without bias, and a downstream impl
/// could not reach the private integer sampler anyway.
pub trait SampleRange: sealed::Sealed {
    /// What a sample is.
    type Output;
    /// Draws one value.
    fn sample(self, rng: &mut Rng) -> Self::Output;
}

mod sealed {
    pub trait Sealed {}
}

macro_rules! float_range {
    ($t:ty, $unit:ident) => {
        impl sealed::Sealed for std::ops::Range<$t> {}
        impl SampleRange for std::ops::Range<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                assert!(self.start < self.end, "Rng::range on an empty range {:?}", self);
                let v = self.start + rng.$unit() * (self.end - self.start);
                // Rounding can land exactly on `end` for a wide range; keep the half-open promise.
                if v < self.end { v } else { self.start }
            }
        }
        impl sealed::Sealed for std::ops::RangeInclusive<$t> {}
        impl SampleRange for std::ops::RangeInclusive<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                let (a, b) = self.into_inner();
                assert!(a <= b, "Rng::range on an empty range {a:?}..={b:?}");
                (a + rng.$unit() * (b - a)).min(b)
            }
        }
    };
}
// `f32` only — see `Rng::range` for why a second float impl breaks every literal call site.
float_range!(f32, f32);

macro_rules! int_range {
    ($($t:ty),*) => {$(
        impl sealed::Sealed for std::ops::Range<$t> {}
        impl SampleRange for std::ops::Range<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                assert!(self.start < self.end, "Rng::range on an empty range {:?}", self);
                // The span as an unsigned 64-bit count, wrapping so signed ranges work.
                let span = (self.end as i128 - self.start as i128) as u64;
                (self.start as i128 + rng.below(span) as i128) as $t
            }
        }
        impl sealed::Sealed for std::ops::RangeInclusive<$t> {}
        impl SampleRange for std::ops::RangeInclusive<$t> {
            type Output = $t;
            fn sample(self, rng: &mut Rng) -> $t {
                let (a, b) = self.into_inner();
                assert!(a <= b, "Rng::range on an empty range {a:?}..={b:?}");
                let span = b as i128 - a as i128 + 1;
                if span > u64::MAX as i128 {
                    // The whole of u64/i64: every bit pattern is in range.
                    return rng.u64() as $t;
                }
                (a as i128 + rng.below(span as u64) as i128) as $t
            }
        }
    )*};
}
int_range!(i32, u32, i64, u64, usize);

#[cfg(test)]
mod tests {
    use super::*;

    /// The first outputs of SplitMix64 from seed 0 — the published reference values, so this is
    /// the algorithm and not merely *a* generator.
    #[test]
    fn seed_zero_matches_the_reference_splitmix64_sequence() {
        let mut rng = Rng::new(0);
        assert_eq!(rng.u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(rng.u64(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(rng.u64(), 0x06C4_5D18_8009_454F);
    }

    #[test]
    fn the_same_seed_replays_and_a_different_one_does_not() {
        let a: Vec<u64> = { let mut r = Rng::new(99); (0..16).map(|_| r.u64()).collect() };
        let b: Vec<u64> = { let mut r = Rng::new(99); (0..16).map(|_| r.u64()).collect() };
        let c: Vec<u64> = { let mut r = Rng::new(100); (0..16).map(|_| r.u64()).collect() };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// Neighbouring streams share no outputs, and swapping seed and stream — the collision a
    /// hand-written `seed + stream` or `seed ^ stream` has — gives a different generator.
    #[test]
    fn streams_are_disjoint_and_seed_and_stream_do_not_commute() {
        let take = |mut r: Rng| (0..64).map(|_| r.u64()).collect::<std::collections::HashSet<_>>();
        let a = take(Rng::with_stream(5, 0));
        let b = take(Rng::with_stream(5, 1));
        assert!(a.is_disjoint(&b), "streams 0 and 1 share outputs");
        assert_ne!(Rng::with_stream(1, 2), Rng::with_stream(2, 1));
        assert_ne!(Rng::with_stream(3, 5), Rng::with_stream(5, 3));
    }

    #[test]
    fn a_fork_is_independent_of_its_parent() {
        let mut parent = Rng::new(1);
        let mut child = parent.fork();
        let p: Vec<u64> = (0..32).map(|_| parent.u64()).collect();
        let c: Vec<u64> = (0..32).map(|_| child.u64()).collect();
        assert!(p.iter().all(|x| !c.contains(x)));
    }

    #[test]
    fn unit_floats_stay_half_open_and_fill_the_interval() {
        let mut rng = Rng::new(3);
        let (mut lo, mut hi) = (1.0f32, 0.0f32);
        for _ in 0..100_000 {
            let v = rng.f32();
            assert!((0.0..1.0).contains(&v), "{v}");
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo < 1e-3 && hi > 1.0 - 1e-3, "[{lo}, {hi}] does not cover [0, 1)");
        let d = rng.f64();
        assert!((0.0..1.0).contains(&d));
    }

    /// Ranges hit both ends (inclusive) or stop short of the end (exclusive), and every value in
    /// a small integer range turns up about equally often.
    #[test]
    fn integer_ranges_are_inclusive_where_asked_and_uniform() {
        let mut rng = Rng::new(11);
        let mut counts = [0u32; 6];
        for _ in 0..60_000 {
            counts[rng.range(0..6usize)] += 1;
        }
        for (i, &c) in counts.iter().enumerate() {
            assert!((9_000..11_000).contains(&c), "face {i} came up {c} times in 60 000");
        }
        let mut seen = [false; 5];
        for _ in 0..1_000 {
            let v = rng.range(-2..=2);
            seen[(v + 2) as usize] = true;
        }
        assert!(seen.iter().all(|&s| s), "an inclusive range missed a value: {seen:?}");
        for _ in 0..1_000 {
            assert!((-3..7).contains(&rng.range(-3i64..7)));
            assert!((10..=12).contains(&rng.range(10u32..=12)));
        }
        let _ = rng.range(0..=u64::MAX);
        let _ = rng.range(i64::MIN..=i64::MAX);
    }

    /// A bare literal range infers `f32` — the property the missing `f64` impl buys. This is a
    /// compile-time test: add an `f64` range impl and it stops building.
    #[test]
    fn a_literal_float_range_is_an_f32() {
        let v: f32 = Rng::new(1).range(-5.0..5.0);
        let w: f32 = Rng::new(2).range(0.0..=1.0);
        assert!((-5.0..5.0).contains(&v) && (0.0..=1.0).contains(&w));
    }

    #[test]
    fn float_ranges_stay_inside() {
        let mut rng = Rng::new(5);
        for _ in 0..10_000 {
            let v = rng.range(-5.0f32..5.0);
            assert!((-5.0..5.0).contains(&v));
            let w = rng.range(2.0..=3.0);
            assert!((2.0..=3.0).contains(&w));
        }
    }

    #[test]
    #[should_panic(expected = "empty range")]
    fn an_empty_range_panics_rather_than_answering() {
        let _ = Rng::new(0).range(5..5);
    }

    #[test]
    fn chance_bool_pick_and_shuffle() {
        let mut rng = Rng::new(8);
        assert!((0..1000).all(|_| !rng.chance(0.0)));
        assert!((0..1000).all(|_| rng.chance(1.0)));
        let heads = (0..10_000).filter(|_| rng.bool()).count();
        assert!((4_700..5_300).contains(&heads), "{heads} heads in 10 000");
        let quarter = (0..10_000).filter(|_| rng.chance(0.25)).count();
        assert!((2_300..2_700).contains(&quarter), "{quarter} of 10 000 at p = 0.25");

        assert_eq!(rng.pick::<u8>(&[]), None);
        assert_eq!(rng.pick(&[42]), Some(&42));
        let mut picked = [0u32; 4];
        for _ in 0..4_000 {
            picked[*rng.pick(&[0usize, 1, 2, 3]).unwrap()] += 1;
        }
        assert!(picked.iter().all(|&n| (850..1_150).contains(&n)), "pick is not uniform: {picked:?}");

        let mut deck: Vec<u32> = (0..52).collect();
        rng.shuffle(&mut deck);
        let mut sorted = deck.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..52).collect::<Vec<_>>(), "a shuffle lost or duplicated a card");
        assert_ne!(deck, sorted, "a 52-card shuffle came back in order");
        // Every position takes every value eventually: position 0 over many shuffles.
        let mut first = std::collections::HashSet::new();
        for _ in 0..2_000 {
            let mut d: Vec<u32> = (0..8).collect();
            rng.shuffle(&mut d);
            first.insert(d[0]);
        }
        assert_eq!(first.len(), 8, "the first slot never received some values: {first:?}");
    }

    /// Directions are unit length and not bunched: the mean of many is near zero, and the share
    /// with |z| < 0.5 is a half — uniform-in-area puts exactly half the sphere's area there,
    /// while two random angles would put ~a third.
    #[test]
    fn directions_are_unit_and_uniform_on_the_sphere() {
        let mut rng = Rng::new(21);
        let mut sum = Vec3::ZERO;
        let mut belt = 0;
        for _ in 0..20_000 {
            let v = rng.unit_vec3();
            assert!((v.length() - 1.0).abs() < 1e-4, "{v:?}");
            sum += v;
            belt += usize::from(v.z.abs() < 0.5);
        }
        assert!((sum / 20_000.0).length() < 0.03, "mean direction {:?}", sum / 20_000.0);
        assert!((9_500..10_500).contains(&belt), "{belt} of 20 000 in the |z| < 0.5 belt");
        for _ in 0..1_000 {
            assert!((rng.unit_vec2().length() - 1.0).abs() < 1e-4);
            assert!(rng.in_unit_sphere().length() <= 1.0 + 1e-5);
            let p = rng.in_box(Vec3::new(-1.0, 2.0, 3.0), Vec3::new(1.0, 4.0, 3.5));
            assert!(p.x >= -1.0 && p.x < 1.0 && p.y >= 2.0 && p.y < 4.0 && p.z >= 3.0 && p.z < 3.5, "{p:?}");
        }
    }
}
