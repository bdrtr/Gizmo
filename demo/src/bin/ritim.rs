//! # Ritim — dört şeritli bir ritim oyunu
//!
//! Notalar dört şeritten aşağı akıyor; çizgiye geldiği an o şeridin tuşuna bas. Müzik kodda
//! sentezleniyor (120 BPM, davul + hi-hat + bas), notalar onun vuruşlarına yerleşiyor. Motorun
//! dokuzuncu oyunu ve **zamanı müziğin kendisinden okuyan** ilki: bir notanın ne zaman çizgide
//! olduğu, kare saatinden değil, hoparlöre giden sesin konumundan hesaplanıyor.
//!
//! | girdi | iş |
//! |---|---|
//! | **D F J K** | dört şerit |
//! | **dokunmatik** | ekranın altındaki dört düğme |
//! | **R** | yeniden başla (müzik başa sarılıyor) |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Çalan bir sesin nerede olduğu okunamıyordu.** Bir ritim oyunu notalarını müziğe göre
//!   zamanlar; kare saatine göre zamanlanan notalar ses gecikmesi, düşen kareler ve bir takılma
//!   kadar müzikten kayar — ve bu kayma birikir. rodio'nun `Player`'ı konumu biliyordu, motor
//!   dışarı açmıyordu. **Kapatıldı:** `AudioManager::position(id)` (sesin saniyesi), `seek(id, s)`
//!   (yeniden başlatma burada müziği başa sarıyor) ve `duration(name)`.
//! - **Hiçbir ses başa sarılamıyordu — ve bunu söyleyen yoktu.** `seek`'in gerçek aygıt testi üç
//!   kat aşağı indi: motorun her sese sardığı alçak geçiren filtre (`Muffle`) atlamayı iletmiyordu;
//!   döngülü seslerin altındaki rodio `repeat_infinite` arabelleği atlayamıyordu; ve çözücüler
//!   uzunluğu bilinmeden kurulduğu için symphonia rastgele erişimi reddediyordu. rodio'nun
//!   `Player::try_seek`'i bunu bazen `Ok` diye bildiriyor. **Kapatıldı:** filtre atlamayı iletip
//!   belleğini sıfırlıyor, döngü motorun kendi kaynağı (`Looping`: sonda aynı baytları yeniden
//!   çözüyor, atlamayı parçanın uzunluğuna göre sarıyor), ve her çözücü uzunluğu ve atlanabilirliği
//!   bilerek kuruluyor.
//!
//! ## Ortam değişkenleri
//!   * `RITIM_OTO=1` — kendi kendine oynar (her notaya tam zamanında basar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::audio::AudioManager;
use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::renderer::components::{ColorTween, ParticleEmitter, ProjectionMode, Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, NodeBundle, PositionType, ProgressBar, Style, UiPlugin, UiRect, Val, VirtualButton};

const RATE: u32 = 44_100;
const BPM: f32 = 120.0;
const BEAT: f32 = 60.0 / BPM;
/// Döngü: 16 vuruş, 8 saniye.
const LOOP_BEATS: usize = 16;
/// Kaç döngü sürüyor bir tur.
const LOOPS: usize = 6;
const LANES: usize = 4;
/// Notanın çizgiye saniyede kaç birim yaklaştığı, ve çizginin yeri.
const SPEED: f32 = 4.0;
const HIT_Y: f32 = -3.0;
const PERFECT: f32 = 0.05;
const GOOD: f32 = 0.12;

const LANE_COLOURS: [Vec4; LANES] = [
    Vec4::new(0.95, 0.3, 0.35, 1.0),
    Vec4::new(0.3, 0.8, 0.45, 1.0),
    Vec4::new(0.35, 0.55, 1.0, 1.0),
    Vec4::new(1.0, 0.8, 0.3, 1.0),
];
const KEYS: [KeyCode; LANES] = [KeyCode::KeyD, KeyCode::KeyF, KeyCode::KeyJ, KeyCode::KeyK];

#[derive(Clone, Copy, PartialEq)]
enum NoteState {
    Waiting,
    Hit,
    Missed,
}

struct Note {
    t: f32,
    lane: usize,
    entity: Option<u32>,
    state: NoteState,
}

struct Game {
    notes: Vec<Note>,
    music: Option<u64>,
    /// Müzik yoksa (ses aygıtı yok) saat buradan.
    fallback_clock: f32,
    /// Şarkının sıfırı müziğin konumunda nerede: başa sarılamayan bir müzikte yeniden başlatma
    /// bir sonraki döngü başına kayıyor.
    offset: f32,
    loop_len: f32,
    score: u32,
    combo: u32,
    best_combo: u32,
    perfect: u32,
    good: u32,
    missed: u32,
    hud: u32,
    judgement: u32,
    lane_marks: [u32; LANES],
    buttons: Option<[u32; LANES]>,
    /// Şarkının ilerlemesi, en üstte ince bir çubuk.
    progress: u32,
    auto: bool,
}

fn lane_x(lane: usize) -> f32 {
    (lane as f32 - (LANES as f32 - 1.0) / 2.0) * 1.2
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Ritim", 1280, 720)
        .add_plugin(UiPlugin)
        .add_plugin(gizmo::systems::lifetime::LifetimePlugin)
        .with_simple_scene(|scene, state| {
            let font = load_font(scene.renderer);
            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 20.0), Vec3::ZERO);
            if let Some(mut q) = scene.world.query_mut::<Mut<Camera>>() {
                for (_, mut cam) in q.iter_mut() {
                    cam.projection = ProjectionMode::OrthographicFit { width: 7.0, height: 9.0 };
                }
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-0.5)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::ONE, 3.0, LightRole::Sun),
            ));
            // Şeritler ve çizgi.
            let mut lane_marks = [0; LANES];
            for (lane, mark) in lane_marks.iter_mut().enumerate() {
                let c = LANE_COLOURS[lane];
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(lane_x(lane), 0.5, -1.0)).with_scale(Vec3::new(0.5, 4.8, 0.05)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_unlit(Vec4::new(c.x * 0.12, c.y * 0.12, c.z * 0.12, 1.0)),
                    MeshRenderer::new(),
                ));
                *mark = scene
                    .world
                    .spawn_bundle((
                        Transform::new(Vec3::new(lane_x(lane), HIT_Y, -0.5)).with_scale(Vec3::new(0.55, 0.08, 0.1)),
                        GlobalTransform::default(),
                        MeshSource("standard_cube".into()),
                        MaterialDesc::new().with_unlit(Vec4::new(c.x * 0.5, c.y * 0.5, c.z * 0.5, 1.0)),
                        MeshRenderer::new(),
                    ))
                    .id();
            }

            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
                .id();
            let judgement = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.3), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();

            let progress = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    inset: UiRect { left: Val::Px(0.0), top: Val::Px(0.0), right: Val::Auto, bottom: Val::Auto },
                    width: Val::Percent(100.0),
                    height: Val::Px(6.0),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::new(1.0, 1.0, 1.0, 0.08)),
                ..Default::default()
            });
            scene.world.add_component(progress, ProgressBar::new(0.0).with_fill(Vec4::new(0.9, 0.85, 0.4, 0.9)));
            let progress = progress.id();

            // Müzik: sentezlenmiş bir döngü; notalar onun vuruşlarına göre.
            let (samples, loop_len) = song();
            let mut music = None;
            if let Ok(mut audio) = AudioManager::new() {
                audio.load_samples("sarki", &samples, RATE);
                music = audio.play_looped("sarki").ok();
                scene.world.insert_resource(audio);
            }

            scene.world.insert_resource(Game {
                notes: chart(),
                music,
                fallback_clock: 0.0,
                offset: 0.0,
                loop_len,
                score: 0,
                combo: 0,
                best_combo: 0,
                perfect: 0,
                good: 0,
                missed: 0,
                hud,
                judgement,
                lane_marks,
                buttons: None,
                progress,
                auto: std::env::var_os("RITIM_OTO").is_some(),
            });
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Şarkının zamanı: çalan müziğin konumu — kare saati değil. Ses yoksa kendi saati.
fn song_time(world: &World, game: &Game) -> f32 {
    game.music
        .and_then(|id| world.get_resource::<AudioManager>()?.position(id))
        .map_or(game.fallback_clock, |p| p - game.offset)
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    game.fallback_clock += dt;
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if game.buttons.is_none() && !input.touches().is_empty() {
        game.buttons = Some(spawn_buttons(world));
    }
    let now = song_time(world, &game);

    // Basılan şeritler: klavye, dokunmatik düğmeler, ya da oto.
    let mut pressed = [false; LANES];
    for (lane, key) in KEYS.iter().enumerate() {
        pressed[lane] |= input.is_key_just_pressed(*key as u32);
    }
    if let Some(buttons) = game.buttons {
        let vb = world.borrow::<VirtualButton>();
        for (lane, id) in buttons.iter().enumerate() {
            pressed[lane] |= vb.get(*id).is_some_and(|b| b.just_pressed());
        }
    }
    if game.auto {
        for n in &game.notes {
            if n.state == NoteState::Waiting && (n.t - now).abs() < PERFECT * 0.5 {
                pressed[n.lane] = true;
            }
        }
    }

    for (lane, p) in pressed.iter().enumerate() {
        if *p {
            judge(world, &mut game, lane, now);
        }
    }
    sweep_misses(world, &mut game, now);
    place_notes(world, &mut game, now);
    draw_hud(world, &game, now);
    world.insert_resource(game);
}

/// Bu şeritte çizgiye en yakın bekleyen nota pencere içindeyse vurulur.
fn judge(world: &mut World, game: &mut Game, lane: usize, now: f32) {
    flash(world, game.lane_marks[lane], LANE_COLOURS[lane]);
    let nearest = game
        .notes
        .iter_mut()
        .filter(|n| n.lane == lane && n.state == NoteState::Waiting)
        .min_by(|a, b| (a.t - now).abs().total_cmp(&(b.t - now).abs()));
    let Some(note) = nearest else { return };
    let off = (note.t - now).abs();
    if off > GOOD {
        return;
    }
    note.state = NoteState::Hit;
    let entity = note.entity.take();
    let (text, points) = if off <= PERFECT { ("Mükemmel", 300) } else { ("İyi", 100) };
    if off <= PERFECT {
        game.perfect += 1;
    } else {
        game.good += 1;
    }
    game.combo += 1;
    game.best_combo = game.best_combo.max(game.combo);
    game.score += points * (1 + game.combo / 10);
    if let Some(id) = entity {
        world.despawn_by_id(id);
    }
    burst(world, Vec3::new(lane_x(lane), HIT_Y, 0.0), LANE_COLOURS[lane]);
    say(world, game.judgement, text, Vec4::new(1.0, 1.0, 0.8, 1.0));
}

fn sweep_misses(world: &mut World, game: &mut Game, now: f32) {
    let mut missed = 0;
    for n in &mut game.notes {
        if n.state == NoteState::Waiting && n.t < now - GOOD {
            n.state = NoteState::Missed;
            missed += 1;
            if let Some(id) = n.entity.take() {
                world.despawn_by_id(id);
            }
        }
    }
    if missed > 0 {
        game.missed += missed;
        game.combo = 0;
        say(world, game.judgement, "Kaçtı", Vec4::new(1.0, 0.4, 0.4, 1.0));
    }
}

/// Görünür penceredeki notaları kur ya da yerine koy: çizgiden `(t − şimdi) × hız` yukarıda.
fn place_notes(world: &mut World, game: &mut Game, now: f32) {
    for n in &mut game.notes {
        if n.state != NoteState::Waiting {
            continue;
        }
        let y = HIT_Y + (n.t - now) * SPEED;
        if y > 5.5 {
            continue;
        }
        let at = Vec3::new(lane_x(n.lane), y, 0.0);
        match n.entity {
            Some(id) => {
                if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
                    t.position = at;
                }
            }
            None => {
                let c = LANE_COLOURS[n.lane];
                n.entity = Some(
                    world
                        .spawn_bundle((
                            Transform::new(at).with_scale(Vec3::new(0.5, 0.14, 0.2)),
                            GlobalTransform::default(),
                            MeshSource("standard_cube".into()),
                            MaterialDesc::new().with_unlit(c).with_emissive(c.truncate() * 0.6),
                            MeshRenderer::new(),
                        ))
                        .id(),
                );
            }
        }
    }
}

fn restart(world: &mut World, game: &mut Game) {
    for n in &mut game.notes {
        if let Some(id) = n.entity.take() {
            world.despawn_by_id(id);
        }
    }
    game.notes = chart();
    game.offset = 0.0;
    if let (Some(id), Some(mut audio)) = (game.music, world.get_resource_mut::<AudioManager>()) {
        if let Err(e) = audio.seek(id, 0.0) {
            // Başa sarılamıyorsa: şarkının sıfırı bir sonraki döngü başı (vuruşlar hizalı kalsın).
            gizmo::gizmo_log!(Warning, "müzik başa sarılamadı ({e}); bir sonraki döngüden başlanıyor");
            let p = audio.position(id).unwrap_or(0.0);
            game.offset = (p / game.loop_len).ceil() * game.loop_len;
        }
    }
    game.fallback_clock = 0.0;
    game.score = 0;
    game.combo = 0;
    game.best_combo = 0;
    game.perfect = 0;
    game.good = 0;
    game.missed = 0;
}

/// Dokunmatik: ekranın altında dört düğme, şeritlerin altında.
fn spawn_buttons(world: &mut World) -> [u32; LANES] {
    std::array::from_fn(|lane| {
        let e = world.spawn_bundle(NodeBundle {
            style: Style {
                position_type: PositionType::Absolute,
                inset: UiRect {
                    left: Val::Percent(lane as f32 * 25.0),
                    top: Val::Auto,
                    right: Val::Auto,
                    bottom: Val::Px(0.0),
                },
                width: Val::Percent(25.0),
                height: Val::Percent(22.0),
                ..Default::default()
            },
            background_color: BackgroundColor(LANE_COLOURS[lane].truncate().extend(0.15)),
            ..Default::default()
        });
        world.add_component(e, VirtualButton::default());
        e.id()
    })
}

fn flash(world: &mut World, mark: u32, colour: Vec4) {
    if let Some(e) = world.get_entity(mark) {
        // Çalışan malzemenin rengi — `ColorTween` de onu sürüyor; açıklama (`MaterialDesc`) her
        // kare ondan geri yazılıyor.
        if let Some(mut m) = world.borrow_mut::<Material>().get_mut(mark) {
            m.albedo = colour;
        }
        world.add_component(e, ColorTween::to(Vec4::new(colour.x * 0.5, colour.y * 0.5, colour.z * 0.5, 1.0), 0.15));
    }
}

fn say(world: &mut World, id: u32, text: &str, colour: Vec4) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        t.content = text.to_string();
        t.color = colour;
    }
    if let Some(e) = world.get_entity(id) {
        world.add_component(e, ColorTween::fade_to(0.0, 0.5).with_delay(0.2));
    }
}

fn burst(world: &mut World, at: Vec3, colour: Vec4) {
    let mut b = ParticleEmitter::one_shot(20);
    b.initial_velocity = Vec3::new(0.0, 2.0, 0.0);
    b.velocity_randomness = 3.0;
    b.lifespan = 0.35;
    b.size_start = 0.1;
    b.size_end = 0.0;
    b.color_start = colour;
    b.color_end = colour.truncate().extend(0.0);
    world.spawn_bundle((Transform::new(at), GlobalTransform::default(), b, gizmo::systems::lifetime::DespawnAfter::secs(0.6)));
}

fn draw_hud(world: &mut World, game: &Game, now: f32) {
    let total = game.notes.len();
    let end = game.notes.last().map_or(1.0, |n| n.t);
    if let Some(mut bar) = world.borrow_mut::<ProgressBar>().get_mut(game.progress) {
        bar.value = (now / end).clamp(0.0, 1.0);
    }
    let done = game.notes.iter().all(|n| n.state != NoteState::Waiting);
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(game.hud) {
        let lap = if game.loop_len > 0.0 { now / game.loop_len } else { 0.0 };
        t.content = format!(
            "Skor {}   Kombo {} (en iyi {})\nMükemmel {}   İyi {}   Kaçan {}   / {}\nŞarkı {:.2} s   tur {:.1}{}",
            game.score,
            game.combo,
            game.best_combo,
            game.perfect,
            game.good,
            game.missed,
            total,
            now,
            lap,
            if done { "\nBitti — R: yeniden" } else { "" }
        );
    }
}

/// Notalar: her vuruşta biri, ara vuruşlarda bazen; şeritler sırayla değil, gezinerek. Aynı
/// tohum her turda aynı parça.
fn chart() -> Vec<Note> {
    let mut rng = Rng::new(120);
    let mut notes = Vec::new();
    let mut lane = 0usize;
    // İlk döngü ısınma: notalar ikinci döngüde başlıyor.
    for half in (LOOP_BEATS * 2)..(LOOP_BEATS * 2 * LOOPS) {
        let on_beat = half % 2 == 0;
        if on_beat || rng.chance(0.3) {
            lane = (lane + rng.range(1..LANES as u32) as usize) % LANES;
            notes.push(Note { t: half as f32 * BEAT / 2.0, lane, entity: None, state: NoteState::Waiting });
        }
    }
    notes
}

/// Döngünün kendisi: davul her vuruşta, hi-hat ara vuruşlarda, bas iki notada gidip geliyor.
fn song() -> (Vec<f32>, f32) {
    let beat_samples = (BEAT * RATE as f32) as usize;
    let len = beat_samples * LOOP_BEATS;
    let mut out = vec![0.0f32; len];
    let mut noise = 0x2545_f491_u32;
    let mut white = || {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        noise as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    for beat in 0..LOOP_BEATS {
        let start = beat * beat_samples;
        // Davul: 120 Hz'den 45'e kayan, hızla sönen bir sinüs.
        let mut phase = 0.0f32;
        for i in 0..(0.18 * RATE as f32) as usize {
            let t = i as f32 / RATE as f32;
            let f = 45.0 + 75.0 * (-t * 30.0).exp();
            phase += std::f32::consts::TAU * f / RATE as f32;
            out[start + i] += phase.sin() * (-t * 18.0).exp() * 0.8;
        }
        // Hi-hat: ara vuruşta kısa bir gürültü.
        let off = start + beat_samples / 2;
        for i in 0..(0.04 * RATE as f32) as usize {
            let t = i as f32 / RATE as f32;
            out[off + i] += white() * (-t * 90.0).exp() * 0.18;
        }
        // Bas: her vuruş boyunca, iki notada gidip gelen yumuşak bir kare dalga.
        let freq = if (beat / 2) % 2 == 0 { 55.0 } else { 65.4 };
        for i in 0..beat_samples {
            let t = i as f32 / RATE as f32;
            let s = if (t * freq).fract() < 0.5 { 1.0 } else { -1.0 };
            out[start + i] += s * 0.08 * (-t * 3.0).exp();
        }
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    for s in &mut out {
        *s = *s / peak * 0.8;
    }
    (out, len as f32 / RATE as f32)
}

fn load_font(renderer: &mut Renderer) -> FontId {
    let from_env = std::env::var("GIZMO_FONT").ok();
    for path in from_env.iter().map(String::as_str).chain([
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "C:/Windows/Fonts/arial.ttf",
    ]) {
        if let Ok(id) = renderer.load_font_file(path) {
            return id;
        }
    }
    renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır")
}
