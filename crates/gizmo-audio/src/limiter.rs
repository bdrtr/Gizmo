//! A master limiter — twenty explosions at once stay under the ceiling instead of clipping.
//!
//! Every sound the manager plays is summed by one mixer, and the sum has no upper bound: eight
//! sounds at half volume peak at 4.0, and the device clips anything past 1.0 into a square-edged
//! crackle. This stage sits on that sum, after the mixer and before the device, and turns the whole
//! mix down just enough to keep its peak at the ceiling.
//!
//! Brick-wall, no look-ahead: the gain follows the loudest sample of each frame **instantly** on the
//! way up, so no sample ever passes the ceiling, and lets go **slowly** on the way down (the
//! release), so the gain does not chase the waveform and buzz. The channels of a frame share one
//! gain, so turning the mix down never moves a sound from left to right.
//!
//! **Bit-exact below the ceiling, and off**: while the mix stays under the ceiling and the release
//! has let go, the samples come through untouched — no multiply by `1.0`, no rounding.

use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A limiter's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Limiter {
    /// The ceiling, as a linear amplitude in `(0, 1]`. No sample of the mix leaves above it.
    pub ceiling: f32,
    /// How long, in seconds, the gain takes to recover by about two-thirds (one time constant)
    /// once the mix drops back under the ceiling.
    pub release: f32,
}

impl Default for Limiter {
    /// A ceiling of about −1 dBFS (0.89) and a 100 ms release: room for the converter, and slow
    /// enough not to pump on a bass note.
    fn default() -> Self {
        Self { ceiling: 0.89, release: 0.1 }
    }
}

impl Limiter {
    /// Any shape: the ceiling is clamped into `0.01..=1`, the release into `0.001..=5` seconds.
    #[must_use]
    pub fn new(ceiling: f32, release: f32) -> Self {
        let c = if ceiling.is_nan() { 1.0 } else { ceiling.clamp(0.01, 1.0) };
        let r = if release.is_nan() { 0.1 } else { release.clamp(0.001, 5.0) };
        Self { ceiling: c, release: r }
    }
}

/// The shape every [`LimiterStage`] reads, shared with the audio thread. A ceiling of zero is off.
#[derive(Debug, Default)]
pub(crate) struct LimiterControl {
    ceiling: AtomicU32,
    release: AtomicU32,
    /// The most the gain has turned the mix down since it was last read, as `f32` bits (`1.0` =
    /// never). Written by the audio thread, read and reset by [`take_min_gain`](Self::take_min_gain).
    min_gain: AtomicU32,
}

impl LimiterControl {
    pub(crate) fn new() -> Arc<Self> {
        let c = Self::default();
        c.min_gain.store(1.0f32.to_bits(), Ordering::Relaxed);
        Arc::new(c)
    }

    /// Sets the shape, or `None` for off. Heard from the next frame.
    pub(crate) fn set(&self, limiter: Option<Limiter>) {
        let l = limiter.unwrap_or(Limiter { ceiling: 0.0, release: 0.1 });
        self.release.store(l.release.to_bits(), Ordering::Relaxed);
        self.ceiling.store(l.ceiling.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> Option<Limiter> {
        let ceiling = f32::from_bits(self.ceiling.load(Ordering::Relaxed));
        let release = f32::from_bits(self.release.load(Ordering::Relaxed));
        (ceiling > 0.0).then_some(Limiter { ceiling, release })
    }

    /// The lowest gain applied since the last call (`1.0` if the limiter never acted), and resets it.
    pub(crate) fn take_min_gain(&self) -> f32 {
        f32::from_bits(self.min_gain.swap(1.0f32.to_bits(), Ordering::Relaxed))
    }

    fn note_gain(&self, gain: f32) {
        // Only the audio thread writes a value below 1, so a plain compare-then-store is enough:
        // the reader's reset can at worst lose one frame's report.
        if gain < f32::from_bits(self.min_gain.load(Ordering::Relaxed)) {
            self.min_gain.store(gain.to_bits(), Ordering::Relaxed);
        }
    }
}

/// The mixer every sound connects to, and its sum through the limiter — what goes into the
/// device's own mixer.
///
/// An empty rodio mixer reads as a *finished* source, and a mixer drops a finished source for
/// good: without the silent infinite input added here, the first moment with nothing playing
/// would unplug the bus and every sound after it would go nowhere. Adding `0.0` to the sum
/// changes no bit.
pub(crate) fn master_bus(
    channels: ChannelCount,
    rate: SampleRate,
    control: Arc<LimiterControl>,
) -> (rodio::mixer::Mixer, LimiterStage<rodio::mixer::MixerSource>) {
    let (bus, out) = rodio::mixer::mixer(channels, rate);
    bus.add(rodio::source::Zero::new(channels, rate));
    (bus, LimiterStage::new(out, control))
}

/// The most channels a frame is buffered for. A source with more passes through unlimited.
const MAX_CHANNELS: usize = 16;

/// The limiter as a source stage around the mixed output.
pub(crate) struct LimiterStage<S> {
    inner: S,
    control: Arc<LimiterControl>,
    channels: usize,
    /// One frame, pulled whole so its channels share a gain.
    frame: [Sample; MAX_CHANNELS],
    /// How many samples of `frame` are filled, and how many of those have been handed out.
    filled: usize,
    next: usize,
    /// The peak envelope: jumps to a louder frame at once, decays toward quieter ones.
    envelope: f32,
    /// `release` and the sample rate it was turned into a per-frame decay for.
    decay_for: (u32, u32),
    decay: f32,
}

impl<S: Source> LimiterStage<S> {
    pub(crate) fn new(inner: S, control: Arc<LimiterControl>) -> Self {
        let channels = usize::from(inner.channels().get());
        Self {
            inner,
            control,
            channels,
            frame: [0.0; MAX_CHANNELS],
            filled: 0,
            next: 0,
            envelope: 0.0,
            decay_for: (u32::MAX, 0),
            decay: 0.0,
        }
    }

    /// Pulls the next frame and scales it. `false` when the source has nothing more.
    fn fill(&mut self) -> bool {
        let mut n = 0;
        while n < self.channels {
            match self.inner.next() {
                Some(s) => {
                    self.frame[n] = s;
                    n += 1;
                }
                None => break,
            }
        }
        self.filled = n;
        self.next = 0;
        if n == 0 {
            return false;
        }

        let ceiling = f32::from_bits(self.control.ceiling.load(Ordering::Relaxed));
        if ceiling <= 0.0 {
            self.envelope = 0.0;
            return true;
        }
        let peak = self.frame[..n].iter().fold(0.0f32, |m, s| m.max(s.abs()));

        let release_bits = self.control.release.load(Ordering::Relaxed);
        let rate = self.inner.sample_rate().get();
        if self.decay_for != (release_bits, rate) {
            let release = f32::from_bits(release_bits).max(1e-4);
            self.decay = (-1.0 / (release * rate as f32)).exp();
            self.decay_for = (release_bits, rate);
        }
        self.envelope = peak.max(self.envelope * self.decay);

        if self.envelope > ceiling {
            let gain = ceiling / self.envelope;
            for s in &mut self.frame[..n] {
                *s *= gain;
            }
            self.control.note_gain(gain);
        }
        true
    }
}

impl<S: Source> Iterator for LimiterStage<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.channels > MAX_CHANNELS {
            return self.inner.next();
        }
        if self.next == self.filled && !self.fill() {
            return None;
        }
        let s = self.frame[self.next];
        self.next += 1;
        Some(s)
    }
}

impl<S: Source> Source for LimiterStage<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;
    use std::num::NonZero;

    const RATE: u32 = 48_000;

    fn sines(voices: usize, amp: f32, frames: usize) -> Vec<f32> {
        // Stereo, every voice in phase: the worst case for the sum.
        (0..frames)
            .flat_map(|i| {
                let t = i as f32 / RATE as f32;
                let s = voices as f32 * amp * (std::f32::consts::TAU * 220.0 * t).sin();
                [s, s * 0.5]
            })
            .collect()
    }

    fn run(samples: Vec<f32>, limiter: Option<Limiter>) -> (Vec<f32>, f32) {
        let control = LimiterControl::new();
        control.set(limiter);
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(RATE).unwrap(), samples);
        let out: Vec<f32> = LimiterStage::new(src, control.clone()).collect();
        (out, control.take_min_gain())
    }

    #[test]
    fn eight_loud_voices_never_pass_the_ceiling_and_keep_their_balance() {
        let input = sines(8, 0.5, RATE as usize / 4);
        let raw_peak = input.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(raw_peak > 3.9, "the unlimited sum peaks near 4.0, got {raw_peak}");

        let (out, min_gain) = run(input.clone(), Some(Limiter::default()));
        assert_eq!(out.len(), input.len(), "the limiter must not drop or add samples");
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 0.89 + 1e-6, "limited peak {peak} passed the ceiling");
        assert!(peak > 0.85, "the limiter turned it down too far: {peak}");
        assert!(min_gain < 0.25, "the reported gain {min_gain} must show the reduction");
        // One gain per frame: right is still exactly half of left wherever left is non-zero.
        for f in out.chunks(2) {
            assert!((f[1] - f[0] * 0.5).abs() <= 1e-6, "a frame's channels got different gains: {f:?}");
        }
    }

    #[test]
    fn under_the_ceiling_and_off_are_bit_exact() {
        let quiet = sines(1, 0.5, RATE as usize / 10);
        let (out, g) = run(quiet.clone(), Some(Limiter::default()));
        assert_eq!(out, quiet, "a mix under the ceiling must come through untouched");
        assert_eq!(g, 1.0);

        let loud = sines(8, 0.5, RATE as usize / 10);
        let (out, g) = run(loud.clone(), None);
        assert_eq!(out, loud, "off must be bit-exact even when the mix clips");
        assert_eq!(g, 1.0);
    }

    #[test]
    fn the_bus_outlives_silence_and_passes_its_sum_through_unchanged() {
        let ch = NonZero::new(2).unwrap();
        let rate = NonZero::new(RATE).unwrap();
        let (bus, mut master) = master_bus(ch, rate, LimiterControl::new());
        // Nothing playing for a while: the bus keeps answering (with silence) rather than ending.
        for _ in 0..4800 {
            assert_eq!(master.next(), Some(0.0), "an idle bus must not read as finished");
        }
        // Two sounds after the silence: what comes out is their plain sum, bit for bit.
        let a = sines(1, 0.3, 2000);
        let b: Vec<f32> = sines(1, 0.2, 2000).iter().map(|s| -s * 0.7).collect();
        bus.add(SamplesBuffer::new(ch, rate, a.clone()));
        bus.add(SamplesBuffer::new(ch, rate, b.clone()));
        let out: Vec<f32> = master.by_ref().take(a.len()).collect();
        let expected: Vec<f32> = a.iter().zip(&b).map(|(x, y)| 0.0 + x + y).collect();
        assert_eq!(out, expected);
        assert_eq!(master.next(), Some(0.0), "and it is still alive after they finish");
    }

    #[test]
    fn the_gain_recovers_after_a_burst() {
        // 50 ms at 4×, then a quiet tone: the quiet part must come back to full level within a
        // few release constants, and not instantly (that would buzz).
        let mut input = sines(8, 0.5, RATE as usize / 20);
        let quiet = sines(1, 0.2, RATE as usize);
        input.extend_from_slice(&quiet);
        let burst = RATE as usize / 20 * 2;
        let (out, _) = run(input.clone(), Some(Limiter::new(0.89, 0.05)));

        let ratio = |range: std::ops::Range<usize>| {
            let (a, b) = range.fold((0.0f32, 0.0f32), |(a, b), i| (a + out[i].abs(), b + input[i].abs()));
            a / b
        };
        // Just after the burst: still held down.
        assert!(ratio(burst..burst + 960) < 0.6, "the release let go at once");
        // Half a second later (10 time constants): back to exactly the input.
        let late = burst + RATE as usize;
        assert_eq!(out[late..late + 960], input[late..late + 960], "the gain never came back");
    }
}
