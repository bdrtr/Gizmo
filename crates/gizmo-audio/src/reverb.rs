//! Reverb — a cave, a hall, a tiled corridor — switchable while sounds play.
//!
//! Freeverb (Jezar, public domain): per channel, eight parallel feedback comb filters with a
//! one-pole low-pass in the loop (the damping — walls swallow high frequencies first), then four
//! series all-passes that smear the echoes into a wash. Built the way the underwater `Muffle`
//! is, as a source stage every sound the manager plays is wrapped in, reading shared parameters per
//! sample: turning reverb on is a store, not a walk over the sinks.
//!
//! **Off is bit-exact**: with `wet` at zero the samples come through untouched and the filters are
//! not run. **A reverb has a tail**: when the sound it carries ends, the stage keeps playing what
//! is still ringing until it falls below audibility (or three seconds pass), so a gunshot in a
//! cave does not stop dead with its last sample.

use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A reverb's shape. `wet` 0 is off.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Reverb {
    /// How big the space sounds, `0..=1` — the comb feedback, and so how long the tail is.
    pub room: f32,
    /// How much the walls soak up the highs, `0..=1`.
    pub damping: f32,
    /// How much of the reverberated sound is added, `0..=1`. The dry sound is always kept.
    pub wet: f32,
}

impl Reverb {
    /// A small room: short, a little dull.
    #[must_use]
    pub fn room() -> Self {
        Self { room: 0.5, damping: 0.5, wet: 0.25 }
    }

    /// A hall: long and bright.
    #[must_use]
    pub fn hall() -> Self {
        Self { room: 0.85, damping: 0.25, wet: 0.35 }
    }

    /// A cave: long, dark and loud.
    #[must_use]
    pub fn cave() -> Self {
        Self { room: 0.92, damping: 0.6, wet: 0.5 }
    }

    /// Any shape, clamped into range.
    #[must_use]
    pub fn new(room: f32, damping: f32, wet: f32) -> Self {
        let c = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        Self { room: c(room), damping: c(damping), wet: c(wet) }
    }
}

/// The parameters every [`ReverbStage`] in a manager reads, shared with the audio thread.
#[derive(Debug, Default)]
pub(crate) struct ReverbControl {
    room: AtomicU32,
    damping: AtomicU32,
    wet: AtomicU32,
}

impl ReverbControl {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Sets the shape, or `None` for off. Heard on the next buffer.
    pub(crate) fn set(&self, reverb: Option<Reverb>) {
        let r = reverb.unwrap_or(Reverb { room: 0.0, damping: 0.0, wet: 0.0 });
        self.room.store(r.room.to_bits(), Ordering::Relaxed);
        self.damping.store(r.damping.to_bits(), Ordering::Relaxed);
        self.wet.store(r.wet.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn get(&self) -> Reverb {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        Reverb { room: f(&self.room), damping: f(&self.damping), wet: f(&self.wet) }
    }
}

/// Freeverb's tunings, in samples at 44.1 kHz.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
/// Added to every delay of the second channel, so stereo does not collapse into one image.
const STEREO_SPREAD: usize = 23;
const INPUT_GAIN: f32 = 0.015;
/// The tail is cut once a whole frame stays below this…
const SILENCE: f32 = 1e-4;
/// …or after this long past the end of the input, whichever comes first.
const MAX_TAIL_SECONDS: f32 = 3.0;

#[derive(Debug, Clone)]
struct Comb {
    buf: Vec<f32>,
    at: usize,
    store: f32,
}

impl Comb {
    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.at];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.at] = input + self.store * feedback;
        self.at = (self.at + 1) % self.buf.len();
        out
    }
}

#[derive(Debug, Clone)]
struct AllPass {
    buf: Vec<f32>,
    at: usize,
}

impl AllPass {
    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buf[self.at];
        let out = delayed - input;
        self.buf[self.at] = input + delayed * 0.5;
        self.at = (self.at + 1) % self.buf.len();
        out
    }
}

#[derive(Debug, Clone)]
struct Channel {
    combs: Vec<Comb>,
    allpasses: Vec<AllPass>,
}

impl Channel {
    fn new(rate: u32, spread: usize) -> Self {
        let scale = |n: usize| (((n + spread) as u64 * u64::from(rate)) / 44_100).max(1) as usize;
        Self {
            combs: COMBS.iter().map(|&n| Comb { buf: vec![0.0; scale(n)], at: 0, store: 0.0 }).collect(),
            allpasses: ALLPASSES.iter().map(|&n| AllPass { buf: vec![0.0; scale(n)], at: 0 }).collect(),
        }
    }

    fn process(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let x = input * INPUT_GAIN;
        let mut out: f32 = self.combs.iter_mut().map(|c| c.process(x, feedback, damp)).sum();
        for a in &mut self.allpasses {
            out = a.process(out);
        }
        out
    }
}

/// A source stage adding the manager's reverb to what it carries — see the module docs.
pub(crate) struct ReverbStage<I> {
    input: I,
    control: Arc<ReverbControl>,
    channels: Vec<Channel>,
    tuned_for: Option<(u32, u16)>,
    cursor: usize,
    /// Past the input's end: samples of tail played, and the loudest sample of the frame so far.
    tail: Option<usize>,
    frame_peak: f32,
    quiet_frames: usize,
}

impl<I: Source> ReverbStage<I> {
    pub(crate) fn new(input: I, control: Arc<ReverbControl>) -> Self {
        Self { input, control, channels: Vec::new(), tuned_for: None, cursor: 0, tail: None, frame_peak: 0.0, quiet_frames: 0 }
    }

    fn reset(&mut self) {
        self.channels.clear();
        self.tuned_for = None;
        self.cursor = 0;
    }

    fn ensure_tuned(&mut self) {
        let (rate, count) = (self.input.sample_rate().get(), self.input.channels().get());
        if self.tuned_for != Some((rate, count)) {
            self.channels = (0..count).map(|c| Channel::new(rate, usize::from(c) * STEREO_SPREAD)).collect();
            self.tuned_for = Some((rate, count));
            self.cursor = 0;
        }
    }
}

impl<I: Source> Iterator for ReverbStage<I> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let r = self.control.get();
        let input = if self.tail.is_some() { None } else { self.input.next() };
        let dry = match input {
            Some(s) => s,
            None => {
                // The input is over. Without reverb, or with nothing ringing, so is this.
                if r.wet <= 0.0 || self.channels.is_empty() {
                    return None;
                }
                let played = self.tail.get_or_insert(0);
                *played += 1;
                let rate = self.tuned_for.map_or(44_100, |t| t.0) as f32;
                let count = self.channels.len();
                if *played as f32 > MAX_TAIL_SECONDS * rate * count as f32 || self.quiet_frames > (rate as usize / 20) {
                    return None;
                }
                0.0
            }
        };
        if r.wet <= 0.0 {
            // Off: untouched, and the filters' memory dropped so turning it on starts clean.
            if !self.channels.is_empty() {
                self.reset();
            }
            return Some(dry);
        }
        self.ensure_tuned();
        let feedback = r.room * 0.28 + 0.7;
        let damp = r.damping * 0.4;
        let ch = self.cursor;
        let wet = self.channels[ch].process(dry, feedback, damp);
        self.cursor = (self.cursor + 1) % self.channels.len();
        let out = dry + wet * r.wet * 3.0;
        // Silence detection for the tail, a whole frame at a time.
        if self.tail.is_some() {
            self.frame_peak = self.frame_peak.max(out.abs());
            if self.cursor == 0 {
                if self.frame_peak < SILENCE {
                    self.quiet_frames += 1;
                } else {
                    self.quiet_frames = 0;
                }
                self.frame_peak = 0.0;
            }
        }
        Some(out)
    }
}

impl<I: Source> Source for ReverbStage<I> {
    fn current_span_len(&self) -> Option<usize> {
        // The tail runs past the input's spans: say "unknown" rather than promise a length.
        if self.tail.is_some() { None } else { self.input.current_span_len() }
    }

    fn channels(&self) -> ChannelCount {
        self.input.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.input.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.input.try_seek(pos)?;
        self.reset();
        self.tail = None;
        self.quiet_frames = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZero;

    struct Signal {
        samples: std::vec::IntoIter<Sample>,
        rate: u32,
        channels: u16,
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
            NonZero::new(self.channels).unwrap()
        }
        fn sample_rate(&self) -> SampleRate {
            NonZero::new(self.rate).unwrap()
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    const RATE: u32 = 44_100;

    fn stage(samples: Vec<f32>, channels: u16, reverb: Option<Reverb>) -> ReverbStage<Signal> {
        let control = ReverbControl::new();
        control.set(reverb);
        ReverbStage::new(Signal { samples: samples.into_iter(), rate: RATE, channels }, control)
    }

    fn energy(s: &[f32]) -> f32 {
        s.iter().map(|x| x * x).sum()
    }

    /// Off is the input, bit for bit, and ends when the input does.
    #[test]
    fn off_is_bit_exact() {
        let input: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.37).sin() * 0.5).collect();
        let out: Vec<f32> = stage(input.clone(), 1, None).collect();
        assert_eq!(out, input);
    }

    /// One click in a cave rings on after it — a tail with energy well past the click, decaying —
    /// and the stage keeps playing it after the input has ended, then stops by itself.
    #[test]
    fn a_click_rings_on_and_the_tail_ends() {
        let mut input = vec![0.0; 100];
        input[0] = 1.0;
        let out: Vec<f32> = stage(input, 1, Some(Reverb::cave())).collect();
        assert!(out.len() > RATE as usize / 2, "the tail was cut with the input: {} samples", out.len());
        assert!(out.len() <= (RATE as f32 * (MAX_TAIL_SECONDS + 0.1)) as usize, "the tail did not end: {}", out.len());
        let early = energy(&out[2000..12_000]);
        let late = energy(&out[22_000..32_000]);
        assert!(early > 1e-6, "no reverberation after the click: {early}");
        assert!(late < early, "the tail does not decay: {early} then {late}");
    }

    /// A bigger room rings longer.
    #[test]
    fn a_bigger_room_rings_longer() {
        let mut input = vec![0.0; 100];
        input[0] = 1.0;
        let small: Vec<f32> = stage(input.clone(), 1, Some(Reverb::new(0.3, 0.5, 0.5))).collect();
        let big: Vec<f32> = stage(input, 1, Some(Reverb::new(0.95, 0.5, 0.5))).collect();
        assert!(big.len() > small.len(), "room {} vs {}", small.len(), big.len());
    }

    /// Channels reverberate apart: a silent channel beside a loud one stays silent.
    #[test]
    fn a_silent_channel_stays_silent() {
        let mut input = vec![0.0; 2000];
        for i in (0..2000).step_by(2) {
            input[i] = ((i as f32) * 0.1).sin();
        }
        let out: Vec<f32> = stage(input, 2, Some(Reverb::hall())).collect();
        assert!(out.iter().skip(1).step_by(2).all(|s| *s == 0.0), "the right channel picked up the left's reverb");
    }

    /// Turning it off mid-sound drops the wet at once — the next sample is the dry one.
    #[test]
    fn switching_it_off_is_heard_at_once() {
        let input: Vec<f32> = (0..4000).map(|i| (i as f32 * 0.05).sin()).collect();
        let control = ReverbControl::new();
        control.set(Some(Reverb::cave()));
        let mut s = ReverbStage::new(Signal { samples: input.clone().into_iter(), rate: RATE, channels: 1 }, Arc::clone(&control));
        for _ in 0..2000 {
            s.next();
        }
        control.set(None);
        assert_eq!(s.next(), Some(input[2000]));
    }
}
