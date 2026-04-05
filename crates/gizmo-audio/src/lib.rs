use rodio::{OutputStream, OutputStreamHandle, Sink, Decoder};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub struct AudioManager {
    // OutputStream is kept alive so audio actually plays
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    // Loaded sound buffers? Rodio handles Decoders directly from file or cursor.
    // For simplicity, we just keep paths here, or load them to memory.
    // Memory loading is best for game engines.
    sounds: HashMap<String, String>, 
}

impl AudioManager {
    pub fn new() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, stream_handle)) => {
                println!("Gizmo Audio: Ses cihazı başlatıldı!");
                Some(Self {
                    _stream: stream,
                    stream_handle,
                    sounds: HashMap::new(),
                })
            }
            Err(e) => {
                println!("Gizmo Audio Başarısız: {}", e);
                None
            }
        }
    }

    pub fn load_sound(&mut self, name: &str, path: &str) {
        // Just store the path for now. In a real engine, we'd read bytes to Vec<u8> and use Cursor to avoid disk reads on every play.
        self.sounds.insert(name.to_string(), path.to_string());
    }

    pub fn play(&self, name: &str) {
        if let Some(path) = self.sounds.get(name) {
            if let Ok(file) = File::open(Path::new(path)) {
                let reader = BufReader::new(file);
                if let Ok(decoder) = Decoder::new(reader) {
                    if let Ok(sink) = Sink::try_new(&self.stream_handle) {
                        sink.append(decoder);
                        sink.detach(); // Fire and forget (plays asynchronously in background)
                    }
                }
            }
        } else {
            println!("AudioManager: {} adlı ses bulunamadı!", name);
        }
    }
}
