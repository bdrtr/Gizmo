//! A player connected to its bus once its sound is in it, so the bus converts the sound at its own
//! rate from the first sample.
//!
//! rodio's `Player::connect_new` connects a player's queue to a mixer at once, and the mixer
//! builds its rate converter for the queue right then — from what the queue holds then, its empty
//! start, which reads as 48 kHz. The sound appended after came out converted at that rate for the
//! queue's first span: a 400-sample sound at 8 kHz on an 8 kHz output played as 66 samples, and on
//! a 44.1 kHz device the first ~512 samples of every sound would have played 9 % slow (measured
//! 2026-11-26, rodio 0.22.2). Here the player is built apart (`Player::new`), its sound appended,
//! its queue moved onto the sound by taking its first sample (`Primed`), and only then connected:
//! the converter is built for the sound's own rate.
//!
//! `SpatialPlayer` can only be built connected, so a spatial sound is a `Player` with rodio's own
//! `Spatial` stage in front of it, its positions read every 10 ms as `SpatialPlayer` does.

use rodio::source::{SeekError, Spatial};
use rodio::{ChannelCount, Player, Sample, SampleRate, Source};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A source with its first sample already taken, handed out first.
pub(crate) struct Primed<S> {
    first: Option<Sample>,
    inner: S,
}

impl<S: Source> Primed<S> {
    fn new(mut inner: S) -> Self {
        let first = inner.next();
        Self { first, inner }
    }
}

impl<S: Source> Iterator for Primed<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        self.first.take().or_else(|| self.inner.next())
    }
}

impl<S: Source> Source for Primed<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len().map(|n| n + usize::from(self.first.is_some()))
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
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.first = None;
        self.inner.try_seek(pos)
    }
}

/// A player on `bus`, `fill`ed (its volume, speed and sound) before it is connected.
pub(crate) fn connect(bus: &rodio::mixer::Mixer, fill: impl FnOnce(&Player)) -> Player {
    let (player, queue) = Player::new();
    fill(&player);
    bus.add(Primed::new(queue));
    player
}

/// A spatial sound: a [`Player`] with rodio's `Spatial` stage in front of its source, which reads
/// the positions set here every 10 ms. Everything else is the player's.
pub(crate) struct SpatialSink {
    player: Player,
    positions: Arc<Mutex<[[f32; 3]; 3]>>,
}

impl SpatialSink {
    /// A spatial sink at `(emitter, left ear, right ear)` on `bus`, its volume and speed set by
    /// `price` and `source` appended before it is connected.
    pub(crate) fn connect<S>(
        bus: &rodio::mixer::Mixer,
        (emitter, left, right): ([f32; 3], [f32; 3], [f32; 3]),
        price: impl FnOnce(&Player),
        source: S,
    ) -> Self
    where
        S: Source + Send + 'static,
    {
        let positions = Arc::new(Mutex::new([emitter, left, right]));
        let read = Arc::clone(&positions);
        let player = connect(bus, |player| {
            price(player);
            player.append(Spatial::new(source, emitter, left, right).periodic_access(Duration::from_millis(10), move |s| {
                let [e, l, r] = *read.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                s.set_positions(e, l, r);
            }));
        });
        Self { player, positions }
    }

    fn set(&self, k: usize, pos: [f32; 3]) {
        self.positions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)[k] = pos;
    }

    pub(crate) fn set_emitter_position(&self, pos: [f32; 3]) {
        self.set(0, pos);
    }

    pub(crate) fn set_left_ear_position(&self, pos: [f32; 3]) {
        self.set(1, pos);
    }

    pub(crate) fn set_right_ear_position(&self, pos: [f32; 3]) {
        self.set(2, pos);
    }
}

impl std::ops::Deref for SpatialSink {
    type Target = Player;

    fn deref(&self) -> &Player {
        &self.player
    }
}
