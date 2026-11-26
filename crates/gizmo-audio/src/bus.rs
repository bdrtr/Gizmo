//! Buses that sum their sounds, carry an effect of their own and send to each other.
//!
//! A bus used to be a gain on each sound routed to it: every sound connected to the one master
//! mixer, priced by its bus's number (`Mixer::volume_for`). That prices a music slider, but nothing
//! can be done to a bus's *sound*: there was no sum to put a reverb on, and no way to feed a reverb
//! bus from the others — a send. Now every bus is a node of its own: a rodio mixer its sounds are
//! connected to, its sum through its own reverb (`AudioManager::set_bus_reverb`), and into the master.
//!
//! **A send** (`Mixer::set_send`) copies what a bus puts out, at a level, into another: the
//! from-bus's output, after its fader and its effect, goes on to the master as before and is also
//! written into a queue the to-bus's mixer reads from (`SendReturn`), under that bus's gain. Both
//! ends are pulled on the audio thread one sample each per sample of the master, and rodio starts a
//! source only on a frame boundary, so the queue holds about a frame and the copy arrives a frame
//! late, on its own channel. The return reads whole frames so it cannot slip a channel.
//!
//! A send that would make a loop is refused (`SendError`): its signal would come back to where it
//! started a frame later and ring for ever.

use crate::reverb::{ReverbControl, ReverbStage};
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One send: the samples on their way from one bus to another, its level, the to-bus's gain on
/// them, and whether it still exists.
#[derive(Debug)]
pub(crate) struct SendLink {
    queue: Mutex<VecDeque<Sample>>,
    level: AtomicU32,
    gain: AtomicU32,
    alive: AtomicBool,
    channels: usize,
}

impl SendLink {
    /// The most frames a queue holds before its oldest are dropped: what is in it when one end is
    /// not being pulled (a bus whose node is new), so it cannot grow.
    const MAX_FRAMES: usize = 64;

    pub(crate) fn new(channels: ChannelCount) -> Arc<Self> {
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            level: AtomicU32::new(0f32.to_bits()),
            gain: AtomicU32::new(1f32.to_bits()),
            alive: AtomicBool::new(true),
            channels: usize::from(channels.get()),
        })
    }

    pub(crate) fn set_level(&self, level: f32) {
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Ends the send: its return stops, and rodio drops it.
    pub(crate) fn close(&self) {
        self.alive.store(false, Ordering::Relaxed);
    }

    fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    fn push(&self, sample: Sample) {
        let mut q = self.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        q.push_back(sample * self.level());
        if q.len() > Self::MAX_FRAMES * self.channels {
            let over = q.len() - Self::MAX_FRAMES * self.channels;
            // Whole frames, so the channels stay where they are.
            q.drain(..over.div_ceil(self.channels) * self.channels);
        }
    }
}

/// What a bus's output reads on the audio thread: its reverb and the sends it feeds.
#[derive(Debug)]
pub(crate) struct BusControl {
    pub(crate) reverb: Arc<ReverbControl>,
    sends: Mutex<Vec<Arc<SendLink>>>,
    /// Bumped when `sends` changes, so the output takes the lock only then.
    version: AtomicU64,
}

impl BusControl {
    fn new() -> Arc<Self> {
        Arc::new(Self { reverb: ReverbControl::new(), sends: Mutex::new(Vec::new()), version: AtomicU64::new(0) })
    }

    pub(crate) fn set_sends(&self, sends: Vec<Arc<SendLink>>) {
        *self.sends.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = sends;
        self.version.fetch_add(1, Ordering::Release);
    }
}

/// A bus's sum on its way to the master: through the bus's reverb, and copied into its sends.
pub(crate) struct BusOut {
    inner: ReverbStage<rodio::mixer::MixerSource>,
    control: Arc<BusControl>,
    seen: u64,
    sends: Vec<Arc<SendLink>>,
}

impl Iterator for BusOut {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let s = self.inner.next()?;
        let version = self.control.version.load(Ordering::Acquire);
        if version != self.seen {
            self.sends = self.control.sends.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            self.seen = version;
        }
        for link in &self.sends {
            link.push(s);
        }
        Some(s)
    }
}

impl Source for BusOut {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// A send's arrival in its to-bus: what the from-bus put out, at the send's level and under the
/// to-bus's gain, a frame late. Silent until a whole frame is there.
pub(crate) struct SendReturn {
    link: Arc<SendLink>,
    channels: ChannelCount,
    rate: SampleRate,
    /// Where in a frame this return is, and whether it is reading the queue for this frame.
    at: usize,
    reading: bool,
}

impl SendReturn {
    pub(crate) fn new(link: Arc<SendLink>, channels: ChannelCount, rate: SampleRate) -> Self {
        Self { link, channels, rate, at: 0, reading: false }
    }
}

impl Iterator for SendReturn {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if !self.link.alive.load(Ordering::Relaxed) {
            return None;
        }
        let mut q = self.link.queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.at == 0 {
            self.reading = q.len() >= self.link.channels;
        }
        self.at = (self.at + 1) % self.link.channels;
        if !self.reading {
            return Some(0.0);
        }
        let gain = f32::from_bits(self.link.gain.load(Ordering::Relaxed));
        Some(q.pop_front().unwrap_or(0.0) * gain)
    }
}

impl Source for SendReturn {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> ChannelCount {
        self.channels
    }
    fn sample_rate(&self) -> SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// A bus's node: the mixer its sounds and the sends into it are connected to, and its output's
/// control. Its output is in the master from the start, and never ends.
pub(crate) struct BusNode {
    pub(crate) mixer: rodio::mixer::Mixer,
    pub(crate) control: Arc<BusControl>,
}

impl BusNode {
    pub(crate) fn new(channels: ChannelCount, rate: SampleRate, master: &rodio::mixer::Mixer) -> Self {
        let (mixer, sum) = rodio::mixer::mixer(channels, rate);
        // An empty rodio mixer reads as finished, and the master would drop it.
        mixer.add(rodio::source::Zero::new(channels, rate));
        let control = BusControl::new();
        let inner = ReverbStage::new(sum, Arc::clone(&control.reverb), Arc::new(AtomicBool::new(true)));
        master.add(BusOut { inner, control: Arc::clone(&control), seen: 0, sends: Vec::new() });
        Self { mixer, control }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stereo send keeps its channels: a sound on the left only comes back on the left, whole
    /// frames late, whichever end of the link the master pulls first.
    #[test]
    fn a_send_keeps_its_channels() {
        let (channels, rate) = (std::num::NonZero::new(2).unwrap(), std::num::NonZero::new(8_000).unwrap());
        for from_first in [true, false] {
            let (master, mut out) = rodio::mixer::mixer(channels, rate);
            let (a, b) = if from_first {
                let a = BusNode::new(channels, rate, &master);
                (a, BusNode::new(channels, rate, &master))
            } else {
                let b = BusNode::new(channels, rate, &master);
                (BusNode::new(channels, rate, &master), b)
            };
            let link = SendLink::new(channels);
            link.set_level(1.0);
            b.mixer.add(SendReturn::new(Arc::clone(&link), channels, rate));
            a.control.set_sends(vec![link]);
            let left: Vec<f32> = (0..400).flat_map(|i| [0.25 + i as f32 * 1e-3, 0.0]).collect();
            a.mixer.add(rodio::buffer::SamplesBuffer::new(channels, rate, left));
            let mix: Vec<f32> = (0..1_000).map(|_| out.next().unwrap_or(0.0)).collect();
            assert!(mix.iter().skip(1).step_by(2).all(|&r| r == 0.0), "from first {from_first}: the right channel heard the left");
            let doubled = mix.iter().step_by(2).filter(|&&l| l > 0.4).count();
            assert!(doubled > 300, "from first {from_first}: the send did not come back on the left ({doubled})");
        }
    }
}
