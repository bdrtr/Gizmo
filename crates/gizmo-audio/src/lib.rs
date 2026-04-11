use rodio::{OutputStream, OutputStreamHandle, Sink, SpatialSink, Decoder, Source};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Cursor};
use std::path::Path;
use std::sync::Arc;

// ======================== ECS COMPONENT ========================

/// 3D veya 2D oynatılabilecek ses kaynağı bileşeni (ECS için)
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioSource {
    pub sound_name: String,
    pub is_3d: bool,
    pub volume: f32,
    pub pitch: f32,
    pub loop_sound: bool,
    pub max_distance: f32, // Sesin zayıflayarak tamamen kısılacağı mesafe limiti
    #[serde(skip)]
    pub _internal_sink_id: Option<u64>,
}

impl AudioSource {
    pub fn new(name: &str) -> Self {
        Self {
            sound_name: name.to_string(),
            is_3d: true,
            volume: 1.0,
            pitch: 1.0,
            loop_sound: false,
            max_distance: 100.0, // Varsayılan değer
            _internal_sink_id: None,
        }
    }
    
    pub fn with_loop(mut self, l: bool) -> Self {
        self.loop_sound = l;
        self
    }
    
    pub fn with_max_distance(mut self, dist: f32) -> Self {
        self.max_distance = dist;
        self
    }
}

// ======================== AUDIO MANAGER ========================

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
}

impl AudioManager {
    pub fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, stream_handle)) => {
                println!("Gizmo Audio: Ses cihazı başlatıldı! 3D Uzamsal (Spatial) Motor Aktif.");
                Some(Self {
                    _stream: stream,
                    stream_handle,
                    sound_buffers: HashMap::new(),
                    active_spatial_sinks: HashMap::new(),
                    active_sinks: HashMap::new(),
                    next_sink_id: 1,
                })
            }
            Err(e) => {
                println!("Gizmo Audio Başarısız (Cihaz bulunamadı): {}", e);
                None
            }
        }
    }

    /// Sesi diske gidip okuyarak byte array olarak RAM'e kaydeder
    pub fn load_sound(&mut self, name: &str, path: &str) {
        if let Ok(mut file) = File::open(Path::new(path)) {
            let mut buffer = Vec::new();
            if file.read_to_end(&mut buffer).is_ok() {
                self.sound_buffers.insert(name.to_string(), buffer.into());
                // println!("Gizmo Audio: Belleğe yüklendi -> '{}'", name);
            } else {
                println!("Gizmo Audio Hatası: '{}' okunamadı!", path);
            }
        } else {
            println!("Gizmo Audio Hatası: '{}' bulunamadı!", path);
        }
    }

    /// Normal (Global/Stereo) bir ses oynatır (tek seferlik)
    pub fn play(&mut self, name: &str) -> Option<u64> {
        self.play_internal(name, false)
    }

    /// Normal (Global/Stereo) bir sesi döngüsel oynatır
    pub fn play_looped(&mut self, name: &str) -> Option<u64> {
        self.play_internal(name, true)
    }

    fn play_internal(&mut self, name: &str, looped: bool) -> Option<u64> {
        if let Some(bytes) = self.sound_buffers.get(name) {
            let cursor = Cursor::new(bytes.clone());
            if let Ok(decoder) = Decoder::new(cursor) {
                if let Ok(sink) = Sink::try_new(&self.stream_handle) {
                    if looped {
                        sink.append(decoder.repeat_infinite());
                    } else {
                        sink.append(decoder);
                    }
                    let id = self.next_sink_id;
                    self.next_sink_id += 1;
                    
                    self.active_sinks.insert(id, sink);
                    return Some(id);
                }
            }
        } else {
            println!("AudioManager: '{}' adlı ses bellekte yok!", name);
        }
        None
    }

    /// 3D Uzamsal (Spatial) bir ses oynatır (tek seferlik)
    pub fn play_3d(&mut self, name: &str, emitter_pos: [f32; 3], right_ear: [f32; 3], left_ear: [f32; 3]) -> Option<u64> {
        self.play_3d_internal(name, emitter_pos, right_ear, left_ear, false)
    }

    /// 3D Uzamsal bir sesi döngüsel oynatır
    pub fn play_3d_looped(&mut self, name: &str, emitter_pos: [f32; 3], right_ear: [f32; 3], left_ear: [f32; 3]) -> Option<u64> {
        self.play_3d_internal(name, emitter_pos, right_ear, left_ear, true)
    }

    fn play_3d_internal(&mut self, name: &str, emitter_pos: [f32; 3], right_ear: [f32; 3], left_ear: [f32; 3], looped: bool) -> Option<u64> {
        if let Some(bytes) = self.sound_buffers.get(name) {
            let cursor = Cursor::new(bytes.clone());
            if let Ok(decoder) = Decoder::new(cursor) {
                if let Ok(sink) = SpatialSink::try_new(&self.stream_handle, emitter_pos, left_ear, right_ear) {
                    if looped {
                        sink.append(decoder.repeat_infinite());
                    } else {
                        sink.append(decoder);
                    }
                    
                    let id = self.next_sink_id;
                    self.next_sink_id += 1;
                    
                    self.active_spatial_sinks.insert(id, sink);
                    return Some(id);
                }
            }
        }
        None
    }
    
    // ========== ECS SINK GÜNCELLEMELERİ ==========

    pub fn update_spatial_sink(&mut self, id: u64, emitter_pos: [f32; 3], right_ear: [f32; 3], left_ear: [f32; 3]) {
        if let Some(sink) = self.active_spatial_sinks.get(&id) {
            sink.set_emitter_position(emitter_pos);
            sink.set_left_ear_position(left_ear);
            sink.set_right_ear_position(right_ear);
        }
    }
    
    pub fn set_sink_volume(&mut self, id: u64, volume: f32, is_3d: bool) {
        if is_3d {
            if let Some(sink) = self.active_spatial_sinks.get(&id) {
                sink.set_volume(volume);
            }
        } else {
            if let Some(sink) = self.active_sinks.get(&id) {
                sink.set_volume(volume);
            }
        }
    }
    
    pub fn set_sink_pitch(&mut self, id: u64, pitch: f32, is_3d: bool) {
        if is_3d {
            if let Some(sink) = self.active_spatial_sinks.get(&id) {
                sink.set_speed(pitch); // rodio uses speed as pitch multiplier
            }
        } else {
            if let Some(sink) = self.active_sinks.get(&id) {
                sink.set_speed(pitch); 
            }
        }
    }

    /// Çalan bitmiş sesleri (Sinks) Garbage Collector gibi temizler
    pub fn clean_dead_sinks(&mut self) {
        self.active_spatial_sinks.retain(|_, sink| !sink.empty());
        self.active_sinks.retain(|_, sink| !sink.empty());
    }
    
    pub fn is_playing(&self, id: u64, is_3d: bool) -> bool {
        if is_3d {
            self.active_spatial_sinks.get(&id).map(|s| !s.empty()).unwrap_or(false)
        } else {
            self.active_sinks.get(&id).map(|s| !s.empty()).unwrap_or(false)
        }
    }
}
