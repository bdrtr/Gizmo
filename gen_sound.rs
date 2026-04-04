use std::fs::File;
use std::f32::consts::PI;

fn main() {
    // Generate a simple sine wave blip
    let sample_rate = 44100;
    let duration = 0.2; // seconds
    let num_samples = (sample_rate as f32 * duration) as usize;
    
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    
    let mut writer = hound::WavWriter::create("demo/assets/bounce.wav", spec).unwrap();
    
    for t in 0..num_samples {
        let fraction = t as f32 / sample_rate as f32;
        // Pitch drop effect
        let freq = 800.0 - (fraction * 3000.0);
        let amplitude = 0.5 * (1.0 - fraction / duration); // fade out
        let sample = (fraction * freq * 2.0 * PI).sin() * amplitude;
        writer.write_sample((sample * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
}
