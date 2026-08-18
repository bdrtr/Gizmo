//! A low-pass filter that can be retuned **while the sound is playing** — what the underwater
//! muffle stands on since 2026-08-18.
//!
//! Before it, "muffled" was a turn-down plus a 0.85× playback speed, because rodio's `Player`
//! offers no live filter: once a source is `append`ed the player owns it, so `BltFilter::to_low_pass`
//! is unreachable from the outside. Slowing playback is a *pitch shift*, though — a looped engine
//! sound drops a tone and a music track detunes, neither of which is what water does. Water eats
//! the high frequencies.
//!
//! The way to a live filter is therefore not to reach into the player but to hand it a source that
//! reads its own parameter: [`Muffle`] holds an `Arc<AtomicU32>` cutoff that the audio thread loads
//! per sample, so `set_underwater` is one atomic store and no sink is touched at all.
//!
//! **The biquad state is per channel**, which is the one thing that cannot be borrowed from rodio's
//! `BltFilter`: that filter keeps a single `x_n1/x_n2/y_n1/y_n2` set and applies it to an
//! interleaved stream, so on stereo the left channel's history filters the right channel's samples.
//! `a_silent_channel_stays_silent` is the test that pins the difference — it fails on a shared-state
//! implementation and passes here.

use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The cutoff every [`Muffle`] in a manager reads, shared with the audio thread.
///
/// `0` means **bypass**: the samples come through untouched, bit for bit, and the biquad is not
/// evaluated at all. Any other value is a cutoff in Hz.
#[derive(Debug, Default)]
pub(crate) struct MuffleControl {
    cutoff_hz: AtomicU32,
}

impl MuffleControl {
    /// A control that is not filtering anything.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self { cutoff_hz: AtomicU32::new(0) })
    }

    /// Sets the cutoff in Hz, or `0` to bypass. Heard on the next buffer the device pulls — the
    /// point of the whole design, since no sink has to be found and rewritten.
    pub(crate) fn set_cutoff_hz(&self, hz: u32) {
        self.cutoff_hz.store(hz, Ordering::Relaxed);
    }

    /// The current cutoff, `0` for bypass.
    pub(crate) fn cutoff_hz(&self) -> u32 {
        self.cutoff_hz.load(Ordering::Relaxed)
    }
}

/// Direct-Form I biquad coefficients, already normalised by `a0`.
#[derive(Debug, Clone, Copy, Default)]
struct Coefficients {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coefficients {
    /// RBJ cookbook low-pass at `cutoff` Hz for a stream running at `rate` Hz.
    ///
    /// `q` is `1/sqrt(2)` — a Butterworth response, i.e. maximally flat in the pass band with no
    /// resonant peak at the corner. (rodio's `to_low_pass` uses `0.5`, which is over-damped and
    /// rolls off *into* the pass band; for a muffle the corner is the whole effect, so the flat
    /// one is the one to have.)
    fn low_pass(cutoff: f32, rate: f32) -> Self {
        // Below ~20 Hz there is nothing to keep, and a cutoff at or above Nyquist makes `tan`
        // blow up and the filter go unstable — a NaN that reaches the device is a dead stream.
        let nyquist = rate * 0.5;
        let cutoff = cutoff.clamp(20.0, nyquist * 0.9);
        let q = std::f32::consts::FRAC_1_SQRT_2;

        let w0 = 2.0 * std::f32::consts::PI * cutoff / rate;
        let (sin_w0, cos_w0) = w0.sin_cos();
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) * 0.5) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
}

/// One channel's filter memory. Direct Form I: two input samples back, two output samples back.
#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

/// A [`Source`] that low-passes what it carries, at a cutoff it re-reads while playing.
///
/// Wrapped around the decoder at `play` time, so every sound the engine starts can be muffled
/// later without being found again.
#[derive(Debug)]
pub(crate) struct Muffle<I> {
    input: I,
    control: Arc<MuffleControl>,

    /// What `coefficients` were computed for — recomputing on every sample would be a `sin_cos`
    /// per sample per channel, and the cutoff moves twice a session.
    tuned_for: Option<(u32, u32)>,
    coefficients: Coefficients,
    /// One entry per channel; interleaved streams advance `channel` per sample.
    channels: Vec<ChannelState>,
    channel: usize,
}

impl<I> Muffle<I> {
    /// Wraps `input`, reading its cutoff from `control`.
    pub(crate) fn new(input: I, control: Arc<MuffleControl>) -> Self {
        Self {
            input,
            control,
            tuned_for: None,
            coefficients: Coefficients::default(),
            channels: Vec::new(),
            channel: 0,
        }
    }
}

impl<I> Iterator for Muffle<I>
where
    I: Source,
{
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        let sample = self.input.next()?;

        let cutoff = self.control.cutoff_hz();
        if cutoff == 0 {
            // Bypassed: untouched samples, and the memory is dropped so that re-engaging starts
            // from silence rather than from a history that belongs to a different moment.
            if self.tuned_for.is_some() {
                self.tuned_for = None;
                self.channels.clear();
                self.channel = 0;
            }
            return Some(sample);
        }

        let rate = self.input.sample_rate().get();
        let channel_count = usize::from(self.input.channels().get());
        if self.tuned_for != Some((cutoff, rate)) {
            self.coefficients = Coefficients::low_pass(cutoff as f32, rate as f32);
            self.tuned_for = Some((cutoff, rate));
        }
        // A span change can change the channel count mid-stream; the cursor and the per-channel
        // memory have to follow it or the channels swap filters.
        if self.channels.len() != channel_count {
            self.channels = vec![ChannelState::default(); channel_count];
            self.channel = 0;
        }

        let c = self.coefficients;
        let state = &mut self.channels[self.channel];
        let y = c.b0 * sample + c.b1 * state.x1 + c.b2 * state.x2 - c.a1 * state.y1
            - c.a2 * state.y2;
        state.x2 = state.x1;
        state.x1 = sample;
        state.y2 = state.y1;
        state.y1 = y;

        self.channel = (self.channel + 1) % channel_count;
        Some(y)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.input.size_hint()
    }
}

impl<I> Source for Muffle<I>
where
    I: Source,
{
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.input.current_span_len()
    }

    #[inline]
    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    #[inline]
    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.input.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZero;

    /// A finite interleaved test signal with a declared rate and channel count.
    struct Signal {
        samples: std::vec::IntoIter<Sample>,
        rate: u32,
        channels: u16,
    }

    impl Signal {
        fn new(samples: Vec<Sample>, rate: u32, channels: u16) -> Self {
            Self { samples: samples.into_iter(), rate, channels }
        }
    }

    impl Iterator for Signal {
        type Item = Sample;
        fn next(&mut self) -> Option<Sample> {
            self.samples.next()
        }
    }

    impl Source for Signal {
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> ChannelCount {
            NonZero::new(self.channels).expect("test signals have channels")
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(self.rate).expect("test signals have a rate")
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    const RATE: u32 = 48_000;

    /// `n` samples of a mono sine at `freq`, amplitude 1.
    fn sine(freq: f32, n: usize) -> Vec<Sample> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / RATE as f32).sin())
            .collect()
    }

    /// RMS of the second half, so the filter's start-up transient is not measured as signal.
    fn settled_rms(samples: &[Sample]) -> f32 {
        let tail = &samples[samples.len() / 2..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    fn run(input: Vec<Sample>, channels: u16, cutoff: u32) -> Vec<Sample> {
        let control = MuffleControl::new();
        control.set_cutoff_hz(cutoff);
        Muffle::new(Signal::new(input, RATE, channels), control).collect()
    }

    /// THE MEASUREMENT: what the filter does to a tone below and above its corner.
    ///
    /// A muffle that attenuates everything equally is a volume knob; one that attenuates nothing is
    /// a bypass. The numbers here are the response of a Butterworth (Q = 1/√2) biquad, which is
    /// −3 dB at the corner and −12 dB per octave above it.
    #[test]
    fn a_tone_below_the_corner_passes_and_one_above_it_does_not() {
        const CUTOFF: u32 = 800;
        let reference = settled_rms(&sine(100.0, 4_800));

        let low = settled_rms(&run(sine(100.0, 4_800), 1, CUTOFF));
        let corner = settled_rms(&run(sine(800.0, 4_800), 1, CUTOFF));
        let high = settled_rms(&run(sine(6_400.0, 4_800), 1, CUTOFF));

        let db = |x: f32| 20.0 * (x / reference).log10();
        // 100 Hz is three octaves below the corner: essentially untouched.
        assert!(db(low) > -1.0, "100 Hz must pass: {:.2} dB", db(low));
        // At the corner, a Butterworth low-pass is -3 dB by definition. This is the assertion that
        // would fail if the Q were wrong, or the coefficients were high-pass, or the rate were
        // taken from the wrong place.
        assert!(
            (db(corner) - -3.0).abs() < 1.0,
            "the corner must be -3 dB, got {:.2} dB",
            db(corner)
        );
        // Three octaves above: -12 dB/octave gives about -36 dB.
        assert!(db(high) < -30.0, "6.4 kHz must be gone: {:.2} dB", db(high));
    }

    /// The defect this filter exists to not have: rodio's `BltFilter` keeps ONE biquad memory and
    /// runs an interleaved stream through it, so on stereo each channel is filtered with the other
    /// channel's history. Feed a signal to the left and silence to the right — the right must stay
    /// exactly silent. On a shared-state filter it does not.
    #[test]
    fn a_silent_channel_stays_silent() {
        let mut stereo = Vec::new();
        for s in sine(3_000.0, 2_400) {
            stereo.push(s); // left: a tone the filter has to work on
            stereo.push(0.0); // right: silence
        }
        let out = run(stereo, 2, 500);

        let right_energy: f32 = out.iter().skip(1).step_by(2).map(|s| s.abs()).sum();
        assert_eq!(
            right_energy, 0.0,
            "silence filtered is silence; a non-zero right channel is the left one leaking \
             through shared filter state"
        );
        let left_energy: f32 = out.iter().step_by(2).map(|s| s.abs()).sum();
        assert!(left_energy > 0.0, "and the left channel must still be carrying something");
    }

    /// Bypass has to be free and exact: `0` is the state a game is in for its whole run except
    /// while it is underwater, and a filter that "almost" passes audio through colours everything.
    #[test]
    fn bypass_is_sample_for_sample_identical() {
        let input = sine(1_000.0, 500);
        let out = run(input.clone(), 1, 0);
        assert_eq!(out, input, "cutoff 0 must not touch a single sample");
    }

    /// The whole point of the shared control: the cutoff moves while the source is being consumed,
    /// and the samples after it are filtered without anything reaching into the player.
    #[test]
    fn the_cutoff_can_move_while_the_sound_is_playing() {
        let control = MuffleControl::new();
        let mut muffle = Muffle::new(Signal::new(sine(6_000.0, 4_800), RATE, 1), control.clone());

        let dry: Vec<Sample> = muffle.by_ref().take(2_400).collect();
        control.set_cutoff_hz(400);
        let wet: Vec<Sample> = muffle.collect();

        assert!(
            settled_rms(&wet) < settled_rms(&dry) * 0.1,
            "the tail must be filtered: dry {:.4} wet {:.4}",
            settled_rms(&dry),
            settled_rms(&wet)
        );
    }

    /// A cutoff at or above Nyquist is a division the bilinear transform cannot make; unclamped it
    /// produces NaN, and one NaN in a filter's own feedback path makes every sample after it NaN
    /// — silence on the device, from a number a game is allowed to ask for.
    #[test]
    fn an_absurd_cutoff_does_not_poison_the_stream() {
        for cutoff in [1, 24_000, 48_000, 1_000_000] {
            let out = run(sine(440.0, 480), 1, cutoff);
            assert!(
                out.iter().all(|s| s.is_finite()),
                "cutoff {cutoff} Hz produced a non-finite sample"
            );
        }
    }

    /// Re-engaging must not replay a history from before the bypass — the filter's memory is a
    /// moment in time, and reusing a stale one is a click.
    #[test]
    fn leaving_and_re_entering_the_filter_starts_from_a_clean_memory() {
        let control = MuffleControl::new();
        control.set_cutoff_hz(500);
        let mut muffle = Muffle::new(Signal::new(sine(50.0, 1_000), RATE, 1), control.clone());

        let _filtered: Vec<Sample> = muffle.by_ref().take(500).collect();
        control.set_cutoff_hz(0);
        let _bypassed: Vec<Sample> = muffle.by_ref().take(100).collect();
        assert!(muffle.tuned_for.is_none(), "bypass must drop the tuning");
        assert!(muffle.channels.is_empty(), "and the per-channel memory with it");
    }
}
