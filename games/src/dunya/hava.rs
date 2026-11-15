//! Hava ve sesler.
//!
//! **Hava** bir buçuk, iki buçuk dakikada bir değişiyor: açık, bulutlu, yağmurlu, sisli — sırası
//! dünyanın kendi tohumundan. Değişim birden değil, birkaç saniyede: bulut güneşi ve ayı
//! karartıyor, göğü griye çekiyor; sis ve yağmur görüşü daraltıyor; yağmur kameranın çevresine
//! düşen motorun GPU parçacıkları, ve yağmurda yol kayıyor — fizik dünyasının `Weather::Rain`'i,
//! motorun araç modelinin tutuşu onu okuyor.
//!
//! **Sesler** motorun sentezleyicisinden (`gizmo::audio::synth::Blip`), dosya yok: kristal bir
//! çınlama, odun bir tok vuruş, taş bir tıkırtı, kurmak ve üretmek kısa sesler, biten bir iş iki
//! nota; yağmur ve motorun uğultusu döngü — yağmurun sesi yağmurla, motorunki gazla ve hızla
//! değişiyor. Oyun kodu bir sesi adıyla sıraya koyuyor (`Game::sounds`); pencereli oyun çalıyor,
//! sınama sırayı boşaltıyor.

use super::*;
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::physics::world::Weather as Grip;
use gizmo::renderer::gpu_particles::GpuParticle;

/// Gök.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sky {
    Clear,
    Cloudy,
    Rain,
    Fog,
}

impl Sky {
    pub(super) fn name(self) -> &'static str {
        match self {
            Sky::Clear => "açık",
            Sky::Cloudy => "bulutlu",
            Sky::Rain => "yağmurlu",
            Sky::Fog => "sisli",
        }
    }

    /// Bulut, yağmur, sis: 0…1.
    fn amounts(self) -> [f32; 3] {
        match self {
            Sky::Clear => [0.0, 0.0, 0.0],
            Sky::Cloudy => [0.7, 0.0, 0.15],
            Sky::Rain => [1.0, 1.0, 0.35],
            Sky::Fog => [0.5, 0.0, 1.0],
        }
    }
}

pub(super) struct Weather {
    pub(super) now: Sky,
    next_in: f32,
    cycle: u32,
    /// O an: bulut, yağmur, sis (hedefe doğru yumuşakça).
    pub(super) cloud: f32,
    pub(super) rain: f32,
    pub(super) fog: f32,
}

impl Weather {
    pub(super) fn new() -> Self {
        Self { now: Sky::Clear, next_in: 100.0, cycle: 0, cloud: 0.0, rain: 0.0, fog: 0.0 }
    }

    /// Bir sonraki göğe geç (sınama da çağırıyor).
    pub(super) fn turn(&mut self) {
        self.cycle += 1;
        let h = hash(self.cycle as i32, 7, 911);
        let next = [Sky::Clear, Sky::Cloudy, Sky::Rain, Sky::Fog, Sky::Rain, Sky::Clear][(h % 6) as usize];
        // Aynı gök iki kez üst üste gelmesin.
        self.now = if next == self.now { [Sky::Cloudy, Sky::Rain, Sky::Fog, Sky::Clear][(h / 7 % 4) as usize] } else { next };
        self.next_in = 90.0 + (h % 60) as f32;
    }

    pub(super) fn step(&mut self, dt: f32) {
        self.next_in -= dt;
        if self.next_in <= 0.0 {
            self.turn();
        }
        let [c, r, f] = self.now.amounts();
        let k = (dt / 6.0).min(1.0);
        self.cloud += (c - self.cloud) * k;
        self.rain += (r - self.rain) * k;
        self.fog += (f - self.fog) * k;
    }

    /// Güneşin ve ayın ışığı bu kadarıyla çarpılıyor.
    pub(super) fn light(&self) -> f32 {
        1.0 - 0.6 * self.cloud
    }

    /// Gök renkleri griye doğru; sis yoğunluğu.
    pub(super) fn grade(&self, colour: [f32; 3]) -> [f32; 3] {
        let grey = (colour[0] + colour[1] + colour[2]) / 3.0 * 0.85;
        let t = 0.7 * self.cloud;
        [colour[0] + (grey - colour[0]) * t, colour[1] + (grey - colour[1]) * t, colour[2] + (grey - colour[2]) * t]
    }

    pub(super) fn fog_density(&self, base: f32) -> f32 {
        base * (1.0 + 2.0 * self.rain + 6.0 * self.fog)
    }

    /// Yol kayıyor mu (aracın tutuşu).
    pub(super) fn grip(&self) -> Grip {
        if self.rain > 0.4 {
            Grip::Rain
        } else {
            Grip::Sunny
        }
    }
}

/// Kameranın çevresine bu karenin yağmur damlaları: yağmurun şiddetiyle, kameranın 14–22 m
/// üstünden, baktığı yöne doğru kaydırılmış (`ahead`), biraz rüzgârla. Motorun parçacıkları
/// yerçekimi ve sürtünmeyle ~12 m/s'lik son hıza oturuyor; hıza göre uzayan billboard'lar çizgi
/// gibi düşüyor.
pub(super) fn raindrops(eye: Vec3, ahead: Vec3, rain: f32, clock: f32) -> Vec<GpuParticle> {
    let count = (rain * 140.0) as u32;
    let centre = eye + Vec3::new(ahead.x, 0.0, ahead.z).normalize_or_zero() * 10.0;
    let seed = (clock * 1000.0) as i32;
    (0..count)
        .map(|k| {
            let r = |n: u32| unit(hash(seed, k as i32, n));
            let at = centre + Vec3::new((r(1) - 0.5) * 36.0, 14.0 + r(2) * 8.0, (r(3) - 0.5) * 36.0);
            GpuParticle {
                position: at.to_array(),
                // Yaş: sıfırdan doğup `max_life`'ta ölüyor.
                life: 0.0,
                velocity: [1.5, -12.0 - r(4) * 3.0, 0.4],
                max_life: 1.9,
                color: [0.78, 0.82, 0.92, 0.6],
                size_start: 0.06,
                size_end: 0.06,
                _padding: [0.0; 2],
            }
        })
        .collect()
}

/// Oyunun sesleri, pencereli oyunda.
pub(super) struct Sounds {
    rain: Option<u64>,
    engine: Option<u64>,
}

pub(super) fn install(world: &mut World) -> Option<Sounds> {
    let mut audio = AudioManager::new().ok()?;
    for (name, blip) in [
        ("kristal", Blip::new(Wave::Sine, 880.0, 0.35).with_slide(1760.0).with_volume(0.18)),
        ("odun", Blip::new(Wave::Noise, 200.0, 0.12).with_slide(110.0).with_volume(0.15)),
        ("tas", Blip::new(Wave::Triangle, 520.0, 0.06).with_slide(300.0).with_volume(0.12)),
        ("kur", Blip::new(Wave::Triangle, 180.0, 0.18).with_slide(90.0).with_volume(0.2)),
        ("uret", Blip::new(Wave::Triangle, 440.0, 0.15).with_slide(660.0).with_volume(0.2)),
        ("is", Blip::new(Wave::Sine, 523.0, 0.5).with_slide(1046.0).with_volume(0.2)),
        // Döngüler yumuşak: yağmur alçak bir hışırtı (gürültünün yenilenmesi 900 Hz'te sert bir
        // cızırtıydı), motor üçgen dalgalı bir uğultu (testere dalgası vızıldıyordu); 55 Hz'in
        // bir saniyede tam 55 dönüşü var, döngü tıklamadan kapanıyor.
        ("yagmur", Blip::new(Wave::Noise, 320.0, 2.0).with_volume(0.5).sustained()),
        ("motor", Blip::new(Wave::Triangle, 55.0, 1.0).with_volume(0.5).sustained()),
    ] {
        audio.load_sound_bytes(name, blip.wav());
    }
    let rain = audio.play_looped("yagmur").ok();
    let engine = audio.play_looped("motor").ok();
    for id in [rain, engine].into_iter().flatten() {
        audio.set_volume(id, 0.0);
    }
    world.insert_resource(audio);
    Some(Sounds { rain, engine })
}

/// Sıradaki sesleri çal; döngülerin sesini havaya ve sürüşe göre ayarla.
pub(super) fn play(world: &mut World, sounds: &Sounds, queued: &mut Vec<&'static str>, rain: f32, engine: Option<(f32, f32)>) {
    let Some(mut audio) = world.get_resource_mut::<AudioManager>() else {
        queued.clear();
        return;
    };
    for name in queued.drain(..) {
        let _ = audio.play(name);
    }
    if let Some(id) = sounds.rain {
        audio.set_volume(id, rain * 0.12);
    }
    if let Some(id) = sounds.engine {
        let (throttle, speed) = engine.unwrap_or((0.0, 0.0));
        audio.set_volume(id, if engine.is_some() { 0.04 + 0.1 * throttle.abs() } else { 0.0 });
        audio.set_pitch(id, 0.8 + (speed / 40.0).min(0.9));
    }
}
