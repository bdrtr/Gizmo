//! An equalizer on the whole mix — [`Equalizer`](crate::equalizer::Equalizer), set with
//! [`AudioManager::set_equalizer`](crate::AudioManager::set_equalizer).
//!
//! Up to [`MAX_EQ_BANDS`](crate::equalizer::MAX_EQ_BANDS) bands — low shelf, bell, high shelf — each a biquad from Robert
//! Bristow-Johnson's *Audio EQ Cookbook*, run on the summed output before the compressor and the
//! limiter. Each channel keeps its own filter history (see `filter.rs` for why that matters), and
//! the history and the coefficients are `f64`: a shelf at 50 Hz on a 48 kHz stream has poles within
//! a few thousandths of the unit circle, where `f32` coefficients round the filter into another.
//!
//! The bands reach the audio thread without a lock, through a sequence count: the main thread
//! makes it odd, writes, makes it even; the audio thread takes a band set only when the count was
//! even and unchanged across its read, and otherwise keeps the one it has until the next frame.

use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How many bands an [`Equalizer`] can hold. A band past this many is left out.
pub const MAX_EQ_BANDS: usize = 4;

/// One band of an [`Equalizer`]. Gains are in dB and kept within ±24; frequencies in Hz, kept
/// between 10 Hz and just under half the output's sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum EqBand {
    /// Everything below `freq_hz` raised (or, with a negative gain, lowered) by `gain_db`, the
    /// change half done at `freq_hz`.
    LowShelf {
        /// The shelf's corner, Hz.
        freq_hz: f32,
        /// How far below it is moved, dB.
        gain_db: f32,
    },
    /// A bell centred on `freq_hz`: raised or lowered by `gain_db` there, `q` setting how narrow
    /// (`1` spans about an octave and a third at half its gain; kept within `0.1..=10`).
    Peak {
        /// The bell's centre, Hz.
        freq_hz: f32,
        /// How far the centre is moved, dB.
        gain_db: f32,
        /// How narrow the bell is.
        q: f32,
    },
    /// Everything above `freq_hz` raised or lowered by `gain_db`.
    HighShelf {
        /// The shelf's corner, Hz.
        freq_hz: f32,
        /// How far above it is moved, dB.
        gain_db: f32,
    },
}

/// An equalizer: bands applied to the mix one after another.
///
/// ```
/// use gizmo_audio::equalizer::Equalizer;
///
/// // Warmer and less harsh: bass up, a dip where cheap speakers honk, the top rolled off.
/// let eq = Equalizer::new().low_shelf(120.0, 3.0).peak(900.0, -2.0, 1.2).high_shelf(6_000.0, -4.0);
/// assert_eq!(eq.bands().len(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Equalizer {
    bands: Vec<EqBand>,
}

impl Equalizer {
    /// An equalizer with no bands, which changes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a low shelf — see [`EqBand::LowShelf`].
    #[must_use]
    pub fn low_shelf(self, freq_hz: f32, gain_db: f32) -> Self {
        self.with(EqBand::LowShelf { freq_hz, gain_db })
    }

    /// Adds a bell — see [`EqBand::Peak`].
    #[must_use]
    pub fn peak(self, freq_hz: f32, gain_db: f32, q: f32) -> Self {
        self.with(EqBand::Peak { freq_hz, gain_db, q })
    }

    /// Adds a high shelf — see [`EqBand::HighShelf`].
    #[must_use]
    pub fn high_shelf(self, freq_hz: f32, gain_db: f32) -> Self {
        self.with(EqBand::HighShelf { freq_hz, gain_db })
    }

    /// Adds `band`, unless the equalizer already has [`MAX_EQ_BANDS`].
    #[must_use]
    pub fn with(mut self, band: EqBand) -> Self {
        if self.bands.len() < MAX_EQ_BANDS {
            self.bands.push(band);
        } else {
            log::warn!("[Audio] an equalizer holds {MAX_EQ_BANDS} bands; {band:?} is left out");
        }
        self
    }

    /// The bands, in the order they are applied.
    #[must_use]
    pub fn bands(&self) -> &[EqBand] {
        &self.bands
    }
}

/// The bands every [`EqualizerStage`] reads, shared with the audio thread.
#[derive(Debug, Default)]
pub(crate) struct EqualizerControl {
    /// Odd while the main thread is writing; bumped by two for every new band set.
    sequence: AtomicU32,
    count: AtomicU32,
    /// Per band: kind (1 low shelf, 2 peak, 3 high shelf), frequency, gain, q — the last three as
    /// `f32` bits.
    bands: [[AtomicU32; 4]; MAX_EQ_BANDS],
}

impl EqualizerControl {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Sets the bands, or `None` (or no bands) for off. Heard from the next frame.
    pub(crate) fn set(&self, eq: Option<&Equalizer>) {
        let bands = eq.map_or(&[][..], Equalizer::bands);
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.count.store(bands.len().min(MAX_EQ_BANDS) as u32, Ordering::Relaxed);
        for (slot, band) in self.bands.iter().zip(bands) {
            let (kind, freq, gain, q) = match *band {
                EqBand::LowShelf { freq_hz, gain_db } => (1, freq_hz, gain_db, 0.0),
                EqBand::Peak { freq_hz, gain_db, q } => (2, freq_hz, gain_db, q),
                EqBand::HighShelf { freq_hz, gain_db } => (3, freq_hz, gain_db, 0.0),
            };
            slot[0].store(kind, Ordering::Relaxed);
            slot[1].store(freq.to_bits(), Ordering::Relaxed);
            slot[2].store(gain.to_bits(), Ordering::Relaxed);
            slot[3].store(q.to_bits(), Ordering::Relaxed);
        }
        self.sequence.fetch_add(1, Ordering::Release);
    }

    /// The bands set, `None` when off.
    pub(crate) fn get(&self) -> Option<Equalizer> {
        let bands = self.read().map(|(_, bands)| bands).unwrap_or_default();
        (!bands.is_empty()).then_some(Equalizer { bands })
    }

    /// A consistent band set and its sequence, or `None` while a write is in progress.
    fn read(&self) -> Option<(u32, Vec<EqBand>)> {
        let before = self.sequence.load(Ordering::Acquire);
        if before % 2 == 1 {
            return None;
        }
        let count = (self.count.load(Ordering::Relaxed) as usize).min(MAX_EQ_BANDS);
        let bands = self.bands[..count]
            .iter()
            .filter_map(|slot| {
                let freq_hz = f32::from_bits(slot[1].load(Ordering::Relaxed));
                let gain_db = f32::from_bits(slot[2].load(Ordering::Relaxed));
                let q = f32::from_bits(slot[3].load(Ordering::Relaxed));
                match slot[0].load(Ordering::Relaxed) {
                    1 => Some(EqBand::LowShelf { freq_hz, gain_db }),
                    2 => Some(EqBand::Peak { freq_hz, gain_db, q }),
                    3 => Some(EqBand::HighShelf { freq_hz, gain_db }),
                    _ => None,
                }
            })
            .collect();
        (self.sequence.load(Ordering::Acquire) == before).then_some((before, bands))
    }
}

/// Direct-form-I biquad coefficients, divided through by `a0`.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// The cookbook's coefficients for `band` at `rate` Hz, or `None` for a band that changes
    /// nothing (a gain of 0 dB).
    fn for_band(band: &EqBand, rate: f64) -> Option<Self> {
        let clamp_freq = |f: f32| f64::from(if f.is_nan() { 1_000.0 } else { f }).clamp(10.0, rate * 0.49);
        let clamp_gain = |g: f32| f64::from(if g.is_nan() { 0.0 } else { g }).clamp(-24.0, 24.0);
        let (freq, gain) = match *band {
            EqBand::LowShelf { freq_hz, gain_db } | EqBand::HighShelf { freq_hz, gain_db } | EqBand::Peak { freq_hz, gain_db, .. } => {
                (clamp_freq(freq_hz), clamp_gain(gain_db))
            }
        };
        if gain == 0.0 {
            return None;
        }
        let a = 10f64.powf(gain / 40.0);
        let w0 = std::f64::consts::TAU * freq / rate;
        let (sin, cos) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match *band {
            EqBand::Peak { q, .. } => {
                let q = f64::from(if q.is_nan() { 1.0 } else { q }).clamp(0.1, 10.0);
                let alpha = sin / (2.0 * q);
                (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a)
            }
            EqBand::LowShelf { .. } => {
                // Shelf slope S = 1: the steepest that stays monotonic.
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * (sin / 2.0 * 2f64.sqrt());
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha),
                    (a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha,
                )
            }
            EqBand::HighShelf { .. } => {
                let two_sqrt_a_alpha = 2.0 * a.sqrt() * (sin / 2.0 * 2f64.sqrt());
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + two_sqrt_a_alpha),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - two_sqrt_a_alpha),
                    (a + 1.0) - (a - 1.0) * cos + two_sqrt_a_alpha,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - two_sqrt_a_alpha,
                )
            }
        };
        Some(Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 })
    }
}

/// The most channels the equalizer keeps a history for. A source with more passes through.
const MAX_CHANNELS: usize = 16;

/// The equalizer as a source stage around the mixed output.
pub(crate) struct EqualizerStage<S> {
    inner: S,
    control: Arc<EqualizerControl>,
    channels: usize,
    /// The band set in use, from which sequence and sample rate.
    filters: Vec<Biquad>,
    built_for: Option<(u32, u32)>,
    /// Per band, per channel: x[n-1], x[n-2], y[n-1], y[n-2].
    history: Vec<[[f64; 4]; MAX_CHANNELS]>,
    /// Which channel the next sample is.
    channel: usize,
}

impl<S: Source> EqualizerStage<S> {
    pub(crate) fn new(inner: S, control: Arc<EqualizerControl>) -> Self {
        let channels = usize::from(inner.channels().get());
        Self { inner, control, channels, filters: Vec::new(), built_for: None, history: Vec::new(), channel: 0 }
    }

    /// Takes a new band set if one has been written, at a frame boundary.
    fn refresh(&mut self) {
        let rate = self.inner.sample_rate().get();
        let sequence = self.control.sequence.load(Ordering::Acquire);
        if self.built_for == Some((sequence, rate)) {
            return;
        }
        let Some((sequence, bands)) = self.control.read() else { return };
        let filters: Vec<Biquad> = bands.iter().filter_map(|b| Biquad::for_band(b, f64::from(rate))).collect();
        // A new band set starts from silence: the old history belongs to other filters.
        self.history = vec![[[0.0; 4]; MAX_CHANNELS]; filters.len()];
        self.filters = filters;
        self.built_for = Some((sequence, rate));
    }
}

impl<S: Source> Iterator for EqualizerStage<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.channel == 0 {
            self.refresh();
        }
        let x = self.inner.next()?;
        let channel = self.channel;
        self.channel = (self.channel + 1) % self.channels.max(1);
        // Off — no bands, or none that changes anything — is the input, bit for bit.
        if self.filters.is_empty() || channel >= MAX_CHANNELS {
            return Some(x);
        }
        let mut v = f64::from(x);
        for (f, history) in self.filters.iter().zip(&mut self.history) {
            let h = &mut history[channel];
            let y = f.b0 * v + f.b1 * h[0] + f.b2 * h[1] - f.a1 * h[2] - f.a2 * h[3];
            *h = [v, h[0], y, h[2]];
            v = y;
        }
        Some(v as f32)
    }
}

impl<S: Source> Source for EqualizerStage<S> {
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

    /// One second of a stereo sine at `hz`, left at `amp` and right silent.
    fn tone(hz: f32, amp: f32) -> Vec<f32> {
        (0..RATE as usize)
            .flat_map(|i| [amp * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin(), 0.0])
            .collect()
    }

    fn run(input: &[f32], eq: Option<&Equalizer>) -> Vec<f32> {
        let control = EqualizerControl::new();
        control.set(eq);
        let src = SamplesBuffer::new(NonZero::new(2).unwrap(), NonZero::new(RATE).unwrap(), input.to_vec());
        EqualizerStage::new(src, control).collect()
    }

    /// The gain, in dB, a sine at `hz` comes through `eq` with — measured on the left channel's
    /// RMS over the second half, past the filters' settling.
    fn gain_db(eq: &Equalizer, hz: f32) -> f32 {
        let input = tone(hz, 0.25);
        let out = run(&input, Some(eq));
        let rms = |s: &[f32]| (s[s.len() / 2..].iter().step_by(2).map(|v| v * v).sum::<f32>() / (s.len() / 4) as f32).sqrt();
        20.0 * (rms(&out) / rms(&input)).log10()
    }

    #[test]
    fn off_and_flat_are_bit_exact() {
        let input = tone(440.0, 0.5);
        assert_eq!(run(&input, None), input, "off must change nothing");
        let flat = Equalizer::new().low_shelf(100.0, 0.0).peak(1_000.0, 0.0, 1.0);
        assert_eq!(run(&input, Some(&flat)), input, "bands of 0 dB must change nothing");
    }

    #[test]
    fn a_low_shelf_moves_the_bass_and_leaves_the_treble() {
        let eq = Equalizer::new().low_shelf(200.0, 12.0);
        let bass = gain_db(&eq, 40.0);
        let treble = gain_db(&eq, 5_000.0);
        assert!((bass - 12.0).abs() < 0.5, "40 Hz under a +12 dB shelf at 200 Hz came through at {bass} dB");
        assert!(treble.abs() < 0.3, "5 kHz moved {treble} dB under a bass shelf");
    }

    #[test]
    fn a_bell_moves_its_centre_and_leaves_the_far_ends() {
        let eq = Equalizer::new().peak(1_000.0, 6.0, 1.0);
        let centre = gain_db(&eq, 1_000.0);
        assert!((centre - 6.0).abs() < 0.2, "the bell's centre came through at {centre} dB");
        for far in [60.0, 15_000.0] {
            let g = gain_db(&eq, far);
            assert!(g.abs() < 0.5, "{far} Hz moved {g} dB under a bell at 1 kHz");
        }
    }

    #[test]
    fn a_high_shelf_cuts_the_treble_and_leaves_the_bass() {
        let eq = Equalizer::new().high_shelf(3_000.0, -12.0);
        let treble = gain_db(&eq, 15_000.0);
        let bass = gain_db(&eq, 100.0);
        assert!((treble + 12.0).abs() < 0.6, "15 kHz under a −12 dB shelf at 3 kHz came through at {treble} dB");
        assert!(bass.abs() < 0.3, "100 Hz moved {bass} dB under a treble shelf");
    }

    #[test]
    fn bands_add_up_and_a_silent_channel_stays_silent() {
        let eq = Equalizer::new().low_shelf(200.0, 6.0).high_shelf(3_000.0, 6.0);
        // Each shelf alone moves 40 Hz and 15 kHz by its own 6 dB; together, each end by 6 dB.
        assert!((gain_db(&eq, 40.0) - 6.0).abs() < 0.5);
        assert!((gain_db(&eq, 15_000.0) - 6.0).abs() < 0.6);
        let out = run(&tone(440.0, 0.5), Some(&eq));
        assert!(out.iter().skip(1).step_by(2).all(|&s| s == 0.0), "the silent right channel picked up the left's filter history");
    }

    #[test]
    fn a_fifth_band_is_left_out() {
        let eq = (0..6).fold(Equalizer::new(), |eq, i| eq.peak(100.0 * (i + 1) as f32, 3.0, 1.0));
        assert_eq!(eq.bands().len(), MAX_EQ_BANDS);
    }

    #[test]
    fn a_change_is_heard_and_reads_back() {
        let control = EqualizerControl::new();
        assert_eq!(control.get(), None);
        let eq = Equalizer::new().peak(500.0, -3.0, 2.0);
        control.set(Some(&eq));
        assert_eq!(control.get(), Some(eq));
        control.set(None);
        assert_eq!(control.get(), None);
    }
}
