//! A compressor on the whole mix — [`Compressor`](crate::compressor::Compressor), set with
//! [`AudioManager::set_compressor`](crate::AudioManager::set_compressor).
//!
//! Feed-forward, on the summed output after the equalizer and before the limiter: above the
//! threshold, the mix's level rises one dB for every `ratio` dB the input rises. The level it hears
//! is a peak envelope across the channels — it rises to a louder frame at once and falls at the
//! release's pace, so it holds a tone's peak from one cycle to the next rather than following the
//! wave down to every zero crossing — and every channel of a frame gets the same gain, so the
//! stereo image holds. The reduction deepens toward what that level asks for at the attack's pace
//! and eases as the level falls. Below the threshold with no makeup gain the mix comes through bit
//! for bit, and so does a compressor that is off.
//!
//! The limiter after it is a different tool: it has a ceiling the mix never passes and no ratio.
//! A compressor evens a mix out — the explosion less far above the footsteps — and leaves a peak
//! that is still too high to the limiter.

use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A compressor's shape.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Compressor {
    /// The level, in dBFS, above which the mix is turned down. Kept within `-60..=0`.
    pub threshold_db: f32,
    /// How many dB the input has to rise above the threshold for the output to rise one. `1` is
    /// no compression; kept within `1..=50`.
    pub ratio: f32,
    /// How long, in seconds, a deepening reduction takes to go about two-thirds of the way (one
    /// time constant). Kept within `0.0001..=1`.
    pub attack: f32,
    /// How fast, once the mix is quieter, the level the compressor hears falls: by one time
    /// constant — to about a third of its amplitude — every `release` seconds. Kept within
    /// `0.001..=5`.
    pub release: f32,
    /// A gain, in dB, applied after the reduction — to bring a compressed mix back up. Kept within
    /// `0..=24`.
    pub makeup_db: f32,
}

impl Default for Compressor {
    /// A gentle glue for a game's mix: −18 dBFS, 4:1, 10 ms attack, 150 ms release, no makeup.
    fn default() -> Self {
        Self { threshold_db: -18.0, ratio: 4.0, attack: 0.010, release: 0.150, makeup_db: 0.0 }
    }
}

impl Compressor {
    /// A compressor at `threshold_db` and `ratio`, with the default attack and release.
    #[must_use]
    pub fn new(threshold_db: f32, ratio: f32) -> Self {
        Self { threshold_db, ratio, ..Self::default() }.clamped()
    }

    /// The same compressor with other attack and release times, in seconds.
    #[must_use]
    pub fn with_times(self, attack: f32, release: f32) -> Self {
        Self { attack, release, ..self }.clamped()
    }

    /// The same compressor with a makeup gain, in dB.
    #[must_use]
    pub fn with_makeup(self, makeup_db: f32) -> Self {
        Self { makeup_db, ..self }.clamped()
    }

    fn clamped(self) -> Self {
        let fix = |v: f32, fallback: f32, lo: f32, hi: f32| if v.is_nan() { fallback } else { v.clamp(lo, hi) };
        let d = Self::default();
        Self {
            threshold_db: fix(self.threshold_db, d.threshold_db, -60.0, 0.0),
            ratio: fix(self.ratio, d.ratio, 1.0, 50.0),
            attack: fix(self.attack, d.attack, 0.0001, 1.0),
            release: fix(self.release, d.release, 0.001, 5.0),
            makeup_db: fix(self.makeup_db, d.makeup_db, 0.0, 24.0),
        }
    }
}

/// The shape every [`CompressorStage`] reads, shared with the audio thread. A ratio of zero is
/// off.
#[derive(Debug, Default)]
pub(crate) struct CompressorControl {
    threshold_db: AtomicU32,
    ratio: AtomicU32,
    attack: AtomicU32,
    release: AtomicU32,
    makeup_db: AtomicU32,
    /// The deepest reduction, in dB, since it was last read (`f32` bits). Written by the audio
    /// thread, read and reset by [`take_max_reduction`](Self::take_max_reduction).
    max_reduction_db: AtomicU32,
}

impl CompressorControl {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Sets the shape, or `None` for off. Heard from the next frame.
    pub(crate) fn set(&self, compressor: Option<Compressor>) {
        let c = compressor.map(Compressor::clamped);
        let d = c.unwrap_or_default();
        self.threshold_db.store(d.threshold_db.to_bits(), Ordering::Relaxed);
        self.attack.store(d.attack.to_bits(), Ordering::Relaxed);
        self.release.store(d.release.to_bits(), Ordering::Relaxed);
        self.makeup_db.store(d.makeup_db.to_bits(), Ordering::Relaxed);
        // The ratio last: it is what turns the stage on.
        self.ratio.store(c.map_or(0.0f32, |c| c.ratio).to_bits(), Ordering::Release);
    }

    pub(crate) fn get(&self) -> Option<Compressor> {
        let ratio = f32::from_bits(self.ratio.load(Ordering::Acquire));
        (ratio > 0.0).then(|| Compressor {
            threshold_db: f32::from_bits(self.threshold_db.load(Ordering::Relaxed)),
            ratio,
            attack: f32::from_bits(self.attack.load(Ordering::Relaxed)),
            release: f32::from_bits(self.release.load(Ordering::Relaxed)),
            makeup_db: f32::from_bits(self.makeup_db.load(Ordering::Relaxed)),
        })
    }

    /// The deepest reduction, in dB, since the last call (`0` if it never acted), and resets it.
    pub(crate) fn take_max_reduction(&self) -> f32 {
        f32::from_bits(self.max_reduction_db.swap(0.0f32.to_bits(), Ordering::Relaxed))
    }

    fn note_reduction(&self, db: f32) {
        // Only the audio thread raises it, so compare-then-store loses at most one frame's report
        // to a concurrent reset.
        if db > f32::from_bits(self.max_reduction_db.load(Ordering::Relaxed)) {
            self.max_reduction_db.store(db.to_bits(), Ordering::Relaxed);
        }
    }
}

/// The most channels a frame is buffered for. A source with more passes through uncompressed.
const MAX_CHANNELS: usize = 16;

/// The compressor as a source stage around the mixed output.
pub(crate) struct CompressorStage<S> {
    inner: S,
    control: Arc<CompressorControl>,
    channels: usize,
    frame: [Sample; MAX_CHANNELS],
    filled: usize,
    next: usize,
    /// The level the compressor hears: a peak envelope, linear amplitude.
    envelope: f32,
    /// The gain reduction in force, dB (`0` = none).
    reduction_db: f32,
    /// The attack and release, as bits, and the sample rate they were turned into per-frame
    /// coefficients for.
    coefficients_for: (u32, u32, u32),
    attack_coefficient: f32,
    release_coefficient: f32,
}

impl<S: Source> CompressorStage<S> {
    pub(crate) fn new(inner: S, control: Arc<CompressorControl>) -> Self {
        let channels = usize::from(inner.channels().get());
        Self {
            inner,
            control,
            channels,
            frame: [0.0; MAX_CHANNELS],
            filled: 0,
            next: 0,
            envelope: 0.0,
            reduction_db: 0.0,
            coefficients_for: (u32::MAX, u32::MAX, 0),
            attack_coefficient: 0.0,
            release_coefficient: 0.0,
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

        let ratio = f32::from_bits(self.control.ratio.load(Ordering::Acquire));
        if ratio <= 0.0 {
            self.envelope = 0.0;
            self.reduction_db = 0.0;
            return true;
        }
        let threshold = f32::from_bits(self.control.threshold_db.load(Ordering::Relaxed));
        let makeup = f32::from_bits(self.control.makeup_db.load(Ordering::Relaxed));
        let attack_bits = self.control.attack.load(Ordering::Relaxed);
        let release_bits = self.control.release.load(Ordering::Relaxed);
        let rate = self.inner.sample_rate().get();
        if self.coefficients_for != (attack_bits, release_bits, rate) {
            let per_frame = |seconds: f32| (-1.0 / (seconds.max(1e-5) * rate as f32)).exp();
            self.attack_coefficient = per_frame(f32::from_bits(attack_bits));
            self.release_coefficient = per_frame(f32::from_bits(release_bits));
            self.coefficients_for = (attack_bits, release_bits, rate);
        }

        let peak = self.frame[..n].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        self.envelope = peak.max(self.envelope * self.release_coefficient);
        let level_db = if self.envelope > 1e-9 { 20.0 * self.envelope.log10() } else { -180.0 };
        let target = (level_db - threshold).max(0.0) * (1.0 - 1.0 / ratio);
        // Deepening, at the attack's pace; easing, with the level — whose fall is the release.
        // Following the wave's own level, frame by frame, the reduction sank toward zero at every
        // crossing and settled most of a dB short of what the tone's peak asked for.
        self.reduction_db = if target > self.reduction_db {
            target + (self.reduction_db - target) * self.attack_coefficient
        } else {
            target
        };

        let gain_db = makeup - self.reduction_db;
        // Below the threshold, with no makeup: the samples as they came, not multiplied by a
        // gain that rounds to one.
        if gain_db != 0.0 {
            let gain = 10f32.powf(gain_db / 20.0);
            for s in &mut self.frame[..n] {
                *s *= gain;
            }
        }
        if self.reduction_db > 0.0 {
            self.control.note_reduction(self.reduction_db);
        }
        true
    }
}

impl<S: Source> Iterator for CompressorStage<S> {
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

impl<S: Source> Source for CompressorStage<S> {
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

    /// `seconds` of a 1 kHz stereo sine, its left channel peaking at `peak_db` dBFS and its right
    /// at half that amplitude.
    fn tone(peak_db: f32, seconds: f32) -> Vec<f32> {
        let amp = 10f32.powf(peak_db / 20.0);
        (0..(RATE as f32 * seconds) as usize)
            .flat_map(|i| {
                let s = amp * (std::f32::consts::TAU * 1_000.0 * i as f32 / RATE as f32).sin();
                [s, s * 0.5]
            })
            .collect()
    }

    fn run(input: &[f32], compressor: Option<Compressor>) -> (Vec<f32>, f32) {
        let control = CompressorControl::new();
        control.set(compressor);
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(RATE).unwrap(), input.to_vec());
        let out: Vec<f32> = CompressorStage::new(src, control.clone()).collect();
        (out, control.take_max_reduction())
    }

    /// The left channel's peak over `range` of frames, in dBFS.
    fn peak_db(samples: &[f32], frames: std::ops::Range<usize>) -> f32 {
        let peak = samples[frames.start * 2..frames.end * 2].iter().step_by(2).fold(0.0f32, |m, s| m.max(s.abs()));
        20.0 * peak.log10()
    }

    #[test]
    fn off_and_below_the_threshold_are_bit_exact() {
        let loud = tone(-3.0, 0.2);
        assert_eq!(run(&loud, None).0, loud, "off must change nothing, however loud");
        let quiet = tone(-24.0, 0.2);
        let (out, reduction) = run(&quiet, Some(Compressor::default()));
        assert_eq!(out, quiet, "a mix under the threshold must come through untouched");
        assert_eq!(reduction, 0.0);
    }

    #[test]
    fn above_the_threshold_the_level_rises_one_db_in_ratio() {
        // −6 dBFS into −18 dBFS at 4:1: 12 dB over, 3 dB out — a steady −15 dBFS.
        let input = tone(-6.0, 1.0);
        let (out, reduction) = run(&input, Some(Compressor::new(-18.0, 4.0)));
        let settled = peak_db(&out, RATE as usize / 2..RATE as usize);
        assert!((settled + 15.0).abs() < 0.5, "−6 dBFS at −18 dB, 4:1 settled at {settled} dBFS, not −15");
        assert!((reduction - 9.0).abs() < 0.5, "the reported reduction is {reduction} dB, not 9");
        // One gain per frame: the right channel stays exactly half the left.
        for f in out.chunks(2) {
            assert!((f[1] - f[0] * 0.5).abs() <= 1e-6, "a frame's channels got different gains: {f:?}");
        }
    }

    #[test]
    fn makeup_brings_the_compressed_mix_back_up() {
        let input = tone(-6.0, 1.0);
        let (out, _) = run(&input, Some(Compressor::new(-18.0, 4.0).with_makeup(6.0)));
        let settled = peak_db(&out, RATE as usize / 2..RATE as usize);
        assert!((settled + 9.0).abs() < 0.5, "with 6 dB of makeup it settled at {settled} dBFS, not −9");
    }

    #[test]
    fn the_reduction_comes_on_at_the_attack_and_goes_at_the_release() {
        // Quiet, then a loud second, then quiet again; 5 ms attack, 100 ms release.
        let mut input = tone(-30.0, 0.2);
        input.extend(tone(-6.0, 1.0));
        input.extend(tone(-30.0, 1.5));
        let compressor = Compressor::new(-18.0, 4.0).with_times(0.005, 0.1);
        let (out, _) = run(&input, Some(compressor));
        let gain_db = |frames: std::ops::Range<usize>| peak_db(&out, frames.clone()) - peak_db(&input, frames);
        let loud_at = (RATE as f32 * 0.2) as usize;
        let quiet_at = loud_at + RATE as usize;
        let ms = |m: f32| (RATE as f32 * m / 1_000.0) as usize;
        // A millisecond into the loud part it has barely started; 30 ms in (six attacks) it is on.
        assert!(gain_db(loud_at..loud_at + ms(1.0)) > -3.0, "the reduction came on at once");
        assert!((gain_db(loud_at + ms(30.0)..loud_at + ms(40.0)) + 9.0).abs() < 0.5, "the reduction was not on after six attack times");
        // Just after the loud part it is still turned down; 1.2 s later (twelve releases) it is
        // gone, to the bit.
        assert!(gain_db(quiet_at..quiet_at + ms(10.0)) < -6.0, "the release let go at once");
        let late = quiet_at + ms(1_200.0);
        assert_eq!(out[late * 2..(late + ms(10.0)) * 2], input[late * 2..(late + ms(10.0)) * 2], "the release never let go");
    }

    #[test]
    fn a_ratio_of_one_compresses_nothing_and_settings_read_back() {
        let input = tone(-6.0, 0.2);
        assert_eq!(run(&input, Some(Compressor::new(-18.0, 1.0))).0, input);
        let control = CompressorControl::new();
        assert_eq!(control.get(), None);
        let c = Compressor::new(-12.0, 8.0).with_times(0.002, 0.3).with_makeup(3.0);
        control.set(Some(c));
        assert_eq!(control.get(), Some(c));
        assert_eq!(Compressor::new(f32::NAN, 0.2).ratio, 1.0, "a ratio below one is clamped to one");
    }
}
