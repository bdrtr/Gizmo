//! `gizmo-audio` is the audio subsystem of the Gizmo engine.
//!
//! It is a thin, [`rodio`]-backed layer that exposes a small public surface:
//!
//! - [`AudioSource`] — an ECS component describing a 2D or 3D playable sound.
//! - [`AudioManager`] — a resource that loads sounds into memory and plays,
//!   updates and stops both global (stereo) and 3D spatial sinks.
//! - [`AudioError`] — the error type returned when loading or playing sounds fails.
//!
//! Sounds are decoded from in-memory byte buffers (loaded once via
//! [`AudioManager::load_sound`]) to avoid per-play disk I/O. Spatial playback
//! tracks emitter and listener (ear) positions and attenuates volume by
//! distance. No `rodio` types appear in the public API, keeping the dependency
//! contract internal.

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source, SpatialSink};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;

// ======================== ERRORS ========================

/// Errors that can occur while loading or playing a sound with the
/// [`AudioManager`].
#[derive(Debug)]
#[non_exhaustive]
pub enum AudioError {
    /// An I/O error occurred while reading the sound file.
    Io(std::io::Error),
    /// The requested sound file could not be found at the given path.
    NotFound(String),
    /// No usable audio output device/backend could be opened.
    Backend(String),
    /// A playback was requested for a sound name that has not been loaded
    /// into memory via [`AudioManager::load_sound`].
    NotLoaded(String),
    /// The in-memory sound bytes could not be decoded into a playable stream.
    Decode(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Io(err) => write!(f, "IO Error: {}", err),
            AudioError::NotFound(path) => write!(f, "File not found: {}", path),
            AudioError::Backend(msg) => write!(f, "Audio backend error: {}", msg),
            AudioError::NotLoaded(name) => {
                write!(f, "Sound '{}' is not loaded into memory", name)
            }
            AudioError::Decode(msg) => write!(f, "Failed to decode sound: {}", msg),
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for AudioError {
    fn from(err: std::io::Error) -> Self {
        AudioError::Io(err)
    }
}

// ======================== ECS COMPONENT ========================

/// ECS component for a sound source that can be played in 2D or 3D.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct AudioSource {
    /// Name of the loaded sound to play (see [`AudioManager::load_sound`]).
    pub sound_name: String,
    /// Whether the sound should be played as a 3D spatial source.
    pub is_3d: bool,
    /// Playback volume multiplier (1.0 = original volume).
    pub volume: f32,
    /// Playback pitch/speed multiplier (1.0 = original pitch).
    pub pitch: f32,
    /// Whether the sound should loop indefinitely.
    pub loop_sound: bool,
    /// Distance at which the sound is fully attenuated (silent).
    pub max_distance: f32,
    /// Internal id of the active sink playing this source, if any.
    pub _internal_sink_id: Option<u64>,
    /// Latches once this source has been auto-started, so a finished **one-shot** is not
    /// restarted every frame. (When a one-shot ends the spatial system clears
    /// `_internal_sink_id`; without this sentinel the auto-start guard would fire again
    /// next frame → infinite repeat.) Transient runtime state — not persisted.
    #[serde(skip)]
    pub has_played: bool,
}

impl Default for AudioSource {
    fn default() -> Self {
        Self::new("default")
    }
}

impl AudioSource {
    /// Creates a new [`AudioSource`] for the sound with the given name.
    pub fn new(name: &str) -> Self {
        Self {
            sound_name: name.to_string(),
            is_3d: true,
            volume: 1.0,
            pitch: 1.0,
            loop_sound: false,
            max_distance: 100.0, // Varsayılan değer
            _internal_sink_id: None,
            has_played: false,
        }
    }

    /// Sets whether the sound loops, returning the modified source.
    pub fn with_loop(mut self, l: bool) -> Self {
        self.loop_sound = l;
        self
    }

    /// Sets the attenuation distance, returning the modified source.
    pub fn with_max_distance(mut self, dist: f32) -> Self {
        self.max_distance = dist;
        self
    }
}

// ======================== AUDIO MANAGER ========================

/// Resource that owns the audio output device and manages loaded sounds and
/// active playback sinks (both global and 3D spatial).
pub struct AudioManager {
    // OutputStream is kept alive so audio actually plays
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,

    // RAM'e (Memory) yüklenmiş ses dosyaları (Disk I/O darboğazını önler)
    sound_buffers: HashMap<String, Arc<[u8]>>,

    // Aktif SpatialSink'leri veya normal Sink'leri takip edip parametrelerini güncellemek için
    active_spatial_sinks: HashMap<u64, SpatialSink>,
    active_sinks: HashMap<u64, Sink>,
    next_sink_id: u64,

    // Su-altı "boğma" modu: aktifken tüm sesler kısık + hafif düşük pitch (dampening).
    underwater: bool,
}

// SAFETY: wasm32'de (atomics/paylaşımlı-bellek OLMADAN) yürütme tek thread'dir —
// bir değer başka bir thread'e fiilen taşınamayacağı için bu impl'ler
// gözlemlenemez; cpal'ın WebAudio tipleri yalnızca ham JS handle'ları taşıdığı
// için !Send'dir. wgpu'nun `fragile-send-sync-non-atomic-wasm` deseninin
// birebir karşılığı. `not(target_feature = "atomics")` koşulu bilinçli: wasm
// threads etkinleştirilirse impl kaybolur ve World-resource kullanımı derleme
// hatasıyla yeniden değerlendirmeye zorlar (sessiz unsoundness yerine).
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Send for AudioManager {}
#[cfg(all(target_arch = "wasm32", not(target_feature = "atomics")))]
unsafe impl Sync for AudioManager {}

impl std::fmt::Debug for AudioManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioManager")
            .field("loaded_sounds", &self.sound_buffers.len())
            .field("active_spatial_sinks", &self.active_spatial_sinks.len())
            .field("active_sinks", &self.active_sinks.len())
            .field("next_sink_id", &self.next_sink_id)
            .finish_non_exhaustive()
    }
}

/// Clamp a playback-speed/pitch factor to a value that is safe for rodio's `Speed`
/// filter. A factor of `0.0` (or negative, or NaN) makes rodio compute a source
/// sample-rate of `(orig_rate * factor) as u32 == 0`, which trips a `from >= 1`
/// assert inside `SampleRateConverter::new` and PANICS on the cpal audio callback
/// thread, killing playback. `pitch = 0` is reachable from a scene-authored /
/// serde-deserialized `AudioSource.pitch` and from the near-field 3D-audio path.
pub(crate) fn sanitize_playback_speed(pitch: f32) -> f32 {
    if pitch.is_finite() {
        pitch.max(0.01)
    } else {
        1.0
    }
}

impl AudioManager {
    /// Creates a new audio manager bound to the default output device.
    ///
    /// # Web (WASM) note
    ///
    /// On `wasm32` the backend is the browser's `AudioContext` (via cpal's
    /// WebAudio backend). Browsers suspend an `AudioContext` created before a
    /// user gesture (autoplay policy): construct the `AudioManager` from an
    /// input handler (first click/keypress) rather than at startup, or the
    /// sinks will play silently.
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::Backend`] if no audio output device is available
    /// or the default device cannot be opened.
    pub fn new() -> Result<Self, AudioError> {
        match OutputStream::try_default() {
            Ok((stream, stream_handle)) => {
                log::info!("Gizmo Audio: Ses cihazı başlatıldı! 3D Uzamsal (Spatial) Motor Aktif.");
                Ok(Self {
                    _stream: stream,
                    stream_handle,
                    sound_buffers: HashMap::new(),
                    active_spatial_sinks: HashMap::new(),
                    active_sinks: HashMap::new(),
                    next_sink_id: 1,
                    underwater: false,
                })
            }
            Err(e) => {
                log::error!("Gizmo Audio Başarısız (Cihaz bulunamadı): {}", e);
                Err(AudioError::Backend(e.to_string()))
            }
        }
    }

    /// Sesi diske gidip okuyarak byte array olarak RAM'e kaydeder
    pub fn load_sound(&mut self, name: &str, path: &str) -> Result<(), AudioError> {
        let mut file =
            File::open(Path::new(path)).map_err(|_| AudioError::NotFound(path.to_string()))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).map_err(AudioError::Io)?;
        self.sound_buffers.insert(name.to_string(), buffer.into());
        Ok(())
    }

    /// Registers an already-decoded-from-disk (or embedded / fetched) sound
    /// buffer under `name`. The bytes must be a complete audio file in a
    /// format rodio can decode (WAV/OGG/FLAC/MP3), exactly as
    /// [`load_sound`](Self::load_sound) would have read from disk.
    ///
    /// This is the loading path for targets without a filesystem (WASM, where
    /// assets arrive via `fetch`/`include_bytes!`) and for games that embed
    /// audio in the binary.
    pub fn load_sound_bytes(&mut self, name: &str, bytes: impl Into<Arc<[u8]>>) {
        self.sound_buffers.insert(name.to_string(), bytes.into());
    }

    /// Update çağrıldığında biten sesleri temizler
    pub fn update(&mut self) {
        self.clean_dead_sinks();
    }

    // ── Su-altı ses boğma (underwater muffle) ────────────────────────────────
    /// Su altındayken hacim çarpanı (kısılır).
    const UW_VOLUME_MUL: f32 = 0.4;
    /// Su altındayken oynatma hızı = pitch (hafif düşürülür → "boğuk/uzak" his).
    const UW_SPEED: f32 = 0.85;

    /// Su-altı "boğma" modunu aç/kapa. Aktifken tüm sesler kısılır + hafif düşük pitch'e iner
    /// (rodio `Sink` canlı alçak-geçiren filtre desteklemediğinden gerçek low-pass yerine bu
    /// dampening kullanılır — "muffled" hissi verir). İDEMPOTENT: yalnız durum DEĞİŞİNCE uygular,
    /// bu yüzden her frame güvenle çağrılabilir. NOT: hacim çarpanla geri alındığından, su
    /// altındayken oyun tarafı `set_volume` çağırırsa yüzeye çıkışta hafif sapma olabilir
    /// (sürekli ambient sesler için sorun değil).
    pub fn set_underwater(&mut self, on: bool) {
        if on == self.underwater {
            return;
        }
        self.underwater = on;
        let (vol_mul, speed) = if on {
            (Self::UW_VOLUME_MUL, Self::UW_SPEED)
        } else {
            (1.0 / Self::UW_VOLUME_MUL, 1.0)
        };
        for sink in self.active_sinks.values() {
            sink.set_volume(sink.volume() * vol_mul);
            sink.set_speed(speed);
        }
        for sink in self.active_spatial_sinks.values() {
            sink.set_volume(sink.volume() * vol_mul);
            sink.set_speed(speed);
        }
    }

    /// Su-altı boğma modu şu an aktif mi.
    #[inline]
    pub fn is_underwater(&self) -> bool {
        self.underwater
    }

    /// Yeni oluşturulan bir normal `Sink`'e, o an su altındaysak boğmayı uygular.
    fn apply_underwater_to(sink: &Sink, underwater: bool) {
        if underwater {
            sink.set_volume(sink.volume() * Self::UW_VOLUME_MUL);
            sink.set_speed(Self::UW_SPEED);
        }
    }

    /// Normal (Global/Stereo) bir ses oynatır (tek seferlik)
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded,
    /// [`AudioError::Decode`] if the bytes cannot be decoded, or
    /// [`AudioError::Backend`] if a playback sink cannot be created.
    pub fn play(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_internal(name, false)
    }

    /// Normal (Global/Stereo) bir sesi döngüsel oynatır
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded,
    /// [`AudioError::Decode`] if the bytes cannot be decoded, or
    /// [`AudioError::Backend`] if a playback sink cannot be created.
    pub fn play_looped(&mut self, name: &str) -> Result<u64, AudioError> {
        self.play_internal(name, true)
    }

    fn play_internal(&mut self, name: &str, looped: bool) -> Result<u64, AudioError> {
        let bytes = self.sound_buffers.get(name).ok_or_else(|| {
            log::error!("AudioManager: '{}' adlı ses bellekte yok!", name);
            AudioError::NotLoaded(name.to_string())
        })?;
        let cursor = Cursor::new(Arc::clone(bytes));
        let decoder = Decoder::new(cursor).map_err(|e| AudioError::Decode(e.to_string()))?;
        let sink = Sink::try_new(&self.stream_handle).map_err(|e| AudioError::Backend(e.to_string()))?;
        if looped {
            sink.append(decoder.repeat_infinite());
        } else {
            sink.append(decoder);
        }
        let id = self.next_sink_id;
        self.next_sink_id = self.next_sink_id.wrapping_add(1);

        // Su altındayken başlayan ses de boğuk gelsin.
        Self::apply_underwater_to(&sink, self.underwater);
        self.active_sinks.insert(id, sink);
        Ok(id)
    }

    /// 3D Uzamsal (Spatial) bir ses oynatır (tek seferlik)
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded,
    /// [`AudioError::Decode`] if the bytes cannot be decoded, or
    /// [`AudioError::Backend`] if a spatial sink cannot be created.
    pub fn play_3d(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
    ) -> Result<u64, AudioError> {
        self.play_3d_internal(name, emitter_pos, left_ear, right_ear, false)
    }

    /// 3D Uzamsal bir sesi döngüsel oynatır
    ///
    /// # Errors
    ///
    /// Returns [`AudioError::NotLoaded`] if `name` was never loaded,
    /// [`AudioError::Decode`] if the bytes cannot be decoded, or
    /// [`AudioError::Backend`] if a spatial sink cannot be created.
    pub fn play_3d_looped(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
    ) -> Result<u64, AudioError> {
        self.play_3d_internal(name, emitter_pos, left_ear, right_ear, true)
    }

    fn play_3d_internal(
        &mut self,
        name: &str,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
        looped: bool,
    ) -> Result<u64, AudioError> {
        let bytes = self.sound_buffers.get(name).ok_or_else(|| {
            log::error!("AudioManager: '{}' adlı 3D ses bellekte yok!", name);
            AudioError::NotLoaded(name.to_string())
        })?;
        let cursor = Cursor::new(Arc::clone(bytes));
        let decoder = Decoder::new(cursor).map_err(|e| AudioError::Decode(e.to_string()))?;
        let sink = SpatialSink::try_new(&self.stream_handle, emitter_pos, left_ear, right_ear)
            .map_err(|e| AudioError::Backend(e.to_string()))?;
        if looped {
            sink.append(decoder.repeat_infinite());
        } else {
            sink.append(decoder);
        }

        let id = self.next_sink_id;
        self.next_sink_id = self.next_sink_id.wrapping_add(1);

        if self.underwater {
            sink.set_volume(sink.volume() * Self::UW_VOLUME_MUL);
            sink.set_speed(Self::UW_SPEED);
        }
        self.active_spatial_sinks.insert(id, sink);
        Ok(id)
    }

    // ========== ECS SINK GÜNCELLEMELERİ ==========

    /// Updates an active spatial sink's emitter/ear positions and recomputes
    /// its volume based on distance attenuation and `base_volume`.
    pub fn update_spatial_sink(
        &mut self,
        id: u64,
        emitter_pos: [f32; 3],
        left_ear: [f32; 3],
        right_ear: [f32; 3],
        max_distance: f32,
        base_volume: f32,
    ) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_emitter_position(emitter_pos);
            sink.set_left_ear_position(left_ear);
            sink.set_right_ear_position(right_ear);

            let listener_pos = [
                (left_ear[0] + right_ear[0]) / 2.0,
                (left_ear[1] + right_ear[1]) / 2.0,
                (left_ear[2] + right_ear[2]) / 2.0,
            ];
            let dx = emitter_pos[0] - listener_pos[0];
            let dy = emitter_pos[1] - listener_pos[1];
            let dz = emitter_pos[2] - listener_pos[2];
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            let mut volume = if max_distance > 0.0 {
                (1.0 - distance / max_distance).max(0.0)
            } else {
                1.0
            };
            volume *= base_volume;

            sink.set_volume(volume);
        }
    }

    /// Sets the volume of the active sink with the given id.
    pub fn set_volume(&mut self, id: u64, volume: f32) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_volume(volume);
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.set_volume(volume);
        }
    }

    /// Sets the pitch/playback speed of the active sink with the given id.
    pub fn set_pitch(&mut self, id: u64, pitch: f32) {
        let pitch = sanitize_playback_speed(pitch);
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_speed(pitch);
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.set_speed(pitch);
        }
    }

    /// Stops the active sink with the given id.
    pub fn stop(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.stop();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.stop();
        }
    }

    /// Pauses the active sink with the given id.
    pub fn pause(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.pause();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.pause();
        }
    }

    /// Resumes the (paused) active sink with the given id.
    pub fn resume(&mut self, id: u64) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.play();
        } else if let Some(sink) = self.active_sinks.get(&id) {
            sink.play();
        }
    }

    /// Çalan bitmiş sesleri (Sinks) Garbage Collector gibi temizler
    pub fn clean_dead_sinks(&mut self) {
        self.active_spatial_sinks.retain(|_, sink| !sink.empty());
        self.active_sinks.retain(|_, sink| !sink.empty());
    }

    /// Returns whether the sink with the given id is currently playing.
    pub fn is_playing(&self, id: u64) -> bool {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            !sink.empty() && !sink.is_paused()
        } else if let Some(sink) = self.active_sinks.get(&id) {
            !sink.empty() && !sink.is_paused()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_playback_speed;

    #[test]
    fn playback_speed_never_reaches_zero() {
        // 0 / negative / NaN would make rodio's SampleRateConverter assert (from >= 1)
        // and panic the audio thread. All must clamp to a strictly-positive factor
        // such that `orig_rate * factor >= 1` for any realistic rate (>= ~100 Hz).
        assert!(sanitize_playback_speed(0.0) >= 0.01);
        assert!(sanitize_playback_speed(-2.0) >= 0.01);
        assert_eq!(sanitize_playback_speed(f32::NAN), 1.0);
        assert_eq!(sanitize_playback_speed(f32::INFINITY), 1.0);
        // A normal pitch passes through untouched.
        assert_eq!(sanitize_playback_speed(1.5), 1.5);
        assert_eq!(sanitize_playback_speed(0.5), 0.5);
    }
}

gizmo_core::impl_component!(AudioSource);
