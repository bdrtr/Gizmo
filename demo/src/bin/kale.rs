//! # Kale — yol bulan düşmanlara karşı küçük bir kule savunması
//!
//! Düşmanlar soldaki kapıdan doğup sağdaki kaleye yürüyor. Altınla kule kur; kuleler menzildeki
//! en yakın düşmana ateş ediyor. Kuleler aynı zamanda **duvar**: düşmanlar etraflarından yeni
//! yol buluyor, ve yolu tamamen kapatan bir kule kurulamıyor. On düşman kaleye girerse biter.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık** (zeminde) | kule kur (10 altın) |
//! | **Dalga** düğmesi | sonraki dalgayı başlat |
//! | **R** | baştan başlat |
//!
//! ## Neden var
//!
//! Önceki üç oyun hiç dokunmadı: `gizmo-ai` (ızgara yol bulma, `NavAgent`), `gizmo-ui`
//! (düğme, tıklama), fareyle dünyada nokta seçme (`Camera::screen_to_ray`). Bu oyun üçünü de
//! oyunun merkezine koyuyor. Yazarken karşılaşılan her eksik aşağıda.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **İmlecin altındaki dünya noktası yoktu.** `Camera::screen_to_ray` pencere boyutunu,
//!   kameranın dünya konumunu ve imleci istiyor — üçü üç ayrı yerden — ve ışını zeminle elle
//!   kesmek gerekiyordu (`Ray`'de düzlem kesişimi yoktu): on beş satır. **Kapatıldı:**
//!   `gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y)` (bölünmüş ekranda imlecin
//!   altındaki kamera), `cursor_ray`, `Ray::intersect_plane`.
//! - **"İmleç arayüzün üstünde mi?" sorusu yoktu**, ve akla gelen cevap yanlıştı: düğmelerin
//!   `Interaction`'ına bakmak. Şeridin düğmesiz kısmına tıklamak altındaki zemine kule kurdu.
//!   **Kapatıldı:** `gizmo::ui::pointer_over_ui(world)` — her `Node`, düğme olsun olmasın.
//! - **Tıklama olayı yoktu** (`Interaction` her kare yeniden hesaplanan bir durum). **Kapatıldı**
//!   bu oyundan hemen önce: `ButtonBundle`'daki `Clicked`; "Dalga" düğmesi onu okuyor. Onun için
//!   de sorguda isteğe bağlı bileşen gerekti: `Option<Mut<Clicked>>`.
//! - *Açık, küçük:* ızgaraya engel eklenince ajanlar yolu kendileri yenilemiyor (yeniden
//!   planlama aralığı 0.5 s); oyun her ajanın `clear_path`'ini çağırıyor.
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu
//!   * `KALE_OTO=1` — birkaç kule kurup ilk dalgayı kendiliğinden başlat (ekran görüntüsü için)

use gizmo::ai::{NavAgent, NavGrid};
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::input::Input;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{pointer_over_ui, AlignItems, BackgroundColor, ButtonBundle, Clicked, NodeBundle, Style, UiPlugin, UiRect, Val};
use gizmo::winit::keyboard::KeyCode;

/// Alanın yarı boyutları (ızgara hücreleri 1 m).
const HALF_W: f32 = 10.0;
const HALF_D: f32 = 6.0;
const SPAWN: Vec3 = Vec3::new(-HALF_W + 0.5, 0.4, 0.5);
const GOAL: Vec3 = Vec3::new(HALF_W - 0.5, 0.4, 0.5);
const TOWER_COST: u32 = 10;
const TOWER_RANGE: f32 = 3.5;
const TOWER_COOLDOWN: f32 = 0.5;

#[derive(Clone, Copy)]
struct Enemy {
    hp: f32,
    max_hp: f32,
}
gizmo::core::impl_component!(Enemy);

#[derive(Clone, Copy)]
struct Tower {
    cooldown: f32,
}
gizmo::core::impl_component!(Tower);

/// Kısa ömürlü bir ateş izi.
#[derive(Clone, Copy)]
struct Flash(f32);
gizmo::core::impl_component!(Flash);

struct Game {
    physics: FixedPhysics,
    gold: u32,
    lives: u32,
    wave: u32,
    /// Bu dalgada daha doğacak düşman sayısı ve bir sonrakine kalan süre.
    to_spawn: u32,
    spawn_timer: f32,
    hud: u32,
    banner: u32,
    wave_button: u32,
    rng: Rng,
    auto: bool,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Kale", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.deband_dither = true;
            let font = load_font(r);

            // Zemin, kapı (kırmızı) ve kale (mavi).
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(HALF_W, 0.1, HALF_D)),
                GlobalTransform::default(),
                MeshSource("standard_cube".into()),
                MaterialDesc::new().with_pbr(Vec4::new(0.32, 0.42, 0.28, 1.0), 0.9, 0.0),
                MeshRenderer::new(),
            ));
            for (at, colour) in [(SPAWN, Vec4::new(0.7, 0.2, 0.15, 1.0)), (GOAL, Vec4::new(0.2, 0.35, 0.8, 1.0))] {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(at.x, 0.05, at.z)).with_scale(Vec3::new(0.5, 0.05, 0.5)),
                    GlobalTransform::default(),
                    MeshSource("standard_cube".into()),
                    MaterialDesc::new().with_unlit(colour).with_emissive(colour.truncate() * 0.5),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-1.0)),
                GlobalTransform::default(),
                DirectionalLight::new(Vec3::new(1.0, 0.96, 0.9), 3.0, LightRole::Sun),
            ));
            scene.world.spawn_bundle(CameraBundle {
                position: Vec3::new(0.0, 14.0, 9.0),
                yaw: -std::f32::consts::FRAC_PI_2,
                pitch: -1.0,
                primary: true,
                ..Default::default()
            });

            // Izgara: oyunun kendi alanına oturtulmuş, kuleler elle ekleniyor.
            let mut grid = NavGrid::centred_on(1.0, (HALF_W * 2.0) as i32, (HALF_D * 2.0) as i32, Vec3::ZERO);
            grid.needs_rebuild = false;
            scene.world.insert_resource(grid);

            // Arayüz: alt şeritte iki düğme.
            let bar = scene.world.spawn_bundle(NodeBundle {
                style: Style {
                    width: Val::Percent(100.0),
                    height: Val::Px(64.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    column_gap: Val::Px(12.0),
                    align_items: Some(AlignItems::Center),
                    ..Default::default()
                },
                background_color: BackgroundColor(Vec4::new(0.05, 0.06, 0.08, 0.7)),
                ..Default::default()
            });
            let wave_button = scene.world.spawn_bundle(ButtonBundle {
                style: Style { width: Val::Px(140.0), height: Val::Px(44.0), ..Default::default() },
                background_color: BackgroundColor(Vec4::new(0.25, 0.4, 0.25, 1.0)),
                ..Default::default()
            });
            scene.world.add_component(
                wave_button,
                Text::screen("Dalga", font, 22.0, Vec2::ZERO).with_anchor(TextAnchor::Center),
            );
            {
                use gizmo::core::hierarchy::HierarchyExt;
                scene.world.add_child(bar, wave_button);
            }

            let hud = scene
                .world
                .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopRight, Vec2::new(-16.0, 16.0))
                    .with_align(TextAlign::Right),))
                .id();
            let banner = scene
                .world
                .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.4), Vec2::ZERO)
                    .with_anchor(TextAnchor::Center)
                    .with_align(TextAlign::Center),))
                .id();

            if let Ok(mut audio) = AudioManager::new() {
                audio.load_sound_bytes("ates", Blip::new(Wave::Square, 1200.0, 0.05).with_slide(700.0).with_volume(0.12).wav());
                audio.load_sound_bytes("kur", Blip::new(Wave::Triangle, 300.0, 0.15).with_slide(450.0).with_volume(0.4).wav());
                audio.load_sound_bytes("olum", Blip::new(Wave::Noise, 900.0, 0.2).with_slide(200.0).with_volume(0.3).wav());
                audio.load_sound_bytes("kacak", Blip::new(Wave::Saw, 200.0, 0.4).with_slide(90.0).with_volume(0.3).wav());
                scene.world.insert_resource(audio);
            }

            scene.world.insert_resource(Game {
                physics: FixedPhysics::new(60),
                gold: 30,
                lives: 10,
                wave: 0,
                to_spawn: 0,
                spawn_timer: 0.0,
                hud,
                banner,
                wave_button: wave_button.id(),
                rng: Rng::new(7),
                auto: std::env::var_os("KALE_OTO").is_some(),
            });
        })
        .set_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    if std::mem::take(&mut game.auto) {
        for (x, z) in [(-3.0, 0.5), (-3.0, -0.5), (-3.0, 1.5), (2.0, -1.5), (2.0, 2.5), (5.0, 0.5)] {
            place_tower(world, &mut game, Vec3::new(x, 0.0, z));
        }
        start_wave(&mut game);
    }

    let wave_clicked = world.borrow::<Clicked>().get(game.wave_button).is_some_and(|c| c.0);
    if wave_clicked && game.to_spawn == 0 && game.lives > 0 {
        start_wave(&mut game);
    }
    // Arayüzün üstündeki bir tık — düğme ya da boş şerit — kule kurmasın.
    if input.is_mouse_button_just_pressed(0) && !pointer_over_ui(world) && game.lives > 0 {
        if let Some(at) = ground_under_cursor(world) {
            place_tower(world, &mut game, at);
        }
    }

    spawn_enemies(world, &mut game, dt);
    towers_fire(world, &mut game, dt);
    enemies_arrive(world, &mut game);
    fade_flashes(world, dt);

    game.physics.advance(world, dt);
    gizmo::ai::system::ai_frame(world, dt);
    sync_transforms(world);
    draw_hud(world, &game);
    world.insert_resource(game);
}

fn restart(world: &mut World, game: &mut Game) {
    world.despawn_all_with::<Enemy>();
    world.despawn_all_with::<Tower>();
    world.despawn_all_with::<Flash>();
    if let Some(mut grid) = world.get_resource_mut::<NavGrid>() {
        grid.obstacles.clear();
    }
    game.gold = 30;
    game.lives = 10;
    game.wave = 0;
    game.to_spawn = 0;
    game.physics.reset();
}

fn start_wave(game: &mut Game) {
    game.wave += 1;
    game.to_spawn = 4 + game.wave * 2;
    game.spawn_timer = 0.0;
}

/// İmlecin altındaki zemin noktası, hücre merkezine oturtulmuş.
fn ground_under_cursor(world: &World) -> Option<Vec3> {
    let p = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y)?;
    let cell = Vec3::new(p.x.floor() + 0.5, 0.0, p.z.floor() + 0.5);
    (cell.x.abs() < HALF_W && cell.z.abs() < HALF_D).then_some(cell)
}

/// Bir kule: yeterli altın varsa, hücre boşsa ve yol hâlâ açıksa.
fn place_tower(world: &mut World, game: &mut Game, at: Vec3) {
    if game.gold < TOWER_COST || at.distance(Vec3::new(SPAWN.x, 0.0, SPAWN.z)) < 1.0 || at.distance(Vec3::new(GOAL.x, 0.0, GOAL.z)) < 1.0 {
        return;
    }
    {
        let Some(mut grid) = world.get_resource_mut::<NavGrid>() else { return };
        let cell = grid.world_to_grid(at);
        if grid.obstacles.contains(&cell) {
            return;
        }
        grid.add_obstacle_world(at);
        if grid.find_path(SPAWN, GOAL).is_none() {
            grid.remove_obstacle_world(at);
            return;
        }
    }
    game.gold -= TOWER_COST;
    world.spawn_bundle((
        Transform::new(Vec3::new(at.x, 0.6, at.z)).with_scale(Vec3::new(0.35, 0.6, 0.35)),
        GlobalTransform::default(),
        MeshSource("cylinder".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.55, 0.55, 0.6, 1.0), 0.5, 0.2),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Collider::cylinder(0.35, 0.6),
        Tower { cooldown: 0.0 },
    ));
    // Yeni duvar: yürüyen herkes yolunu yeniden hesaplasın.
    if let Some(mut agents) = world.query_mut::<Mut<NavAgent>>() {
        for (_, mut agent) in agents.iter_mut() {
            agent.clear_path();
        }
    }
    play(world, "kur");
}

fn spawn_enemies(world: &mut World, game: &mut Game, dt: f32) {
    if game.to_spawn == 0 {
        return;
    }
    game.spawn_timer -= dt;
    if game.spawn_timer > 0.0 {
        return;
    }
    game.spawn_timer = 0.9;
    game.to_spawn -= 1;
    let hp = 3.0 + game.wave as f32;
    let jitter = Vec3::new(0.0, 0.0, game.rng.range(-0.3..0.3));
    let mut agent = NavAgent::new(1.6 + game.wave as f32 * 0.1, 8.0, 0.4);
    agent.set_target(GOAL);
    world.spawn_bundle((
        Transform::new(SPAWN + jitter).with_scale(Vec3::splat(0.3)),
        GlobalTransform::default(),
        MeshSource("sphere".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.85, 0.25, 0.2, 1.0), 0.4, 0.0),
        MeshRenderer::new(),
        RigidBody::new_kinematic(),
        Velocity::default(),
        Collider::sphere(0.3),
        agent,
        Enemy { hp, max_hp: hp },
    ));
}

fn towers_fire(world: &mut World, game: &mut Game, dt: f32) {
    let enemies: Vec<(u32, Vec3)> = {
        let es = world.borrow::<Enemy>();
        let ts = world.borrow::<Transform>();
        es.iter().filter_map(|(id, _)| ts.get(id).map(|t| (id, t.position))).collect()
    };
    let mut shots: Vec<(Vec3, Vec3, u32)> = Vec::new();
    {
        let Some(mut towers) = world.query_mut::<(Mut<Tower>, &Transform)>() else { return };
        for (_, (mut tower, tf)) in towers.iter_mut() {
            tower.cooldown -= dt;
            if tower.cooldown > 0.0 {
                continue;
            }
            let at = tf.position;
            let target = enemies
                .iter()
                .filter(|(_, p)| p.distance(at) < TOWER_RANGE)
                .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)));
            if let Some(&(enemy, p)) = target {
                tower.cooldown = TOWER_COOLDOWN;
                shots.push((at + Vec3::Y * 0.6, p, enemy));
            }
        }
    }
    for (from, to, enemy) in shots {
        let mid = (from + to) * 0.5;
        let len = from.distance(to);
        world.spawn_bundle((
            Transform::new(mid)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, (to - from).normalize_or_zero()))
                .with_scale(Vec3::new(0.04, len * 0.5, 0.04)),
            GlobalTransform::default(),
            MeshSource("cylinder".into()),
            MaterialDesc::new().with_unlit(Vec4::new(1.0, 0.9, 0.4, 1.0)).with_emissive(Vec3::new(3.0, 2.4, 0.8)),
            MeshRenderer::new().with_receives_shadows(false),
            Flash(0.08),
        ));
        play(world, "ates");
        let dead = {
            let mut es = world.borrow_mut::<Enemy>();
            match es.get_mut(enemy) {
                Some(mut e) => {
                    e.hp -= 1.0;
                    e.hp <= 0.0
                }
                None => false,
            }
        };
        if dead {
            world.despawn_by_id(enemy);
            game.gold += 3;
            play(world, "olum");
        } else {
            // Can azaldıkça soluklaşan renk.
            let frac = world.borrow::<Enemy>().get(enemy).map(|e| e.hp / e.max_hp).unwrap_or(1.0);
            if let Some(mut m) = world.borrow_mut::<Material>().get_mut(enemy) {
                m.albedo = Vec4::new(0.85, 0.25 + (1.0 - frac) * 0.6, 0.2 + (1.0 - frac) * 0.6, 1.0);
            }
        }
    }
}

fn enemies_arrive(world: &mut World, game: &mut Game) {
    let arrived: Vec<u32> = {
        let es = world.borrow::<Enemy>();
        let ts = world.borrow::<Transform>();
        es.iter()
            .filter(|(id, _)| ts.get(*id).is_some_and(|t| Vec3::new(t.position.x - GOAL.x, 0.0, t.position.z - GOAL.z).length() < 0.6))
            .map(|(id, _)| id)
            .collect()
    };
    for id in arrived {
        world.despawn_by_id(id);
        game.lives = game.lives.saturating_sub(1);
        play(world, "kacak");
    }
}

fn fade_flashes(world: &mut World, dt: f32) {
    let done: Vec<u32> = {
        let mut fs = world.borrow_mut::<Flash>();
        let mut done = Vec::new();
        for (id, mut f) in fs.iter_mut() {
            f.0 -= dt;
            if f.0 <= 0.0 {
                done.push(id);
            }
        }
        done
    };
    for id in done {
        world.despawn_by_id(id);
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let alive = world.borrow::<Enemy>().iter().count();
    let mut texts = world.borrow_mut::<Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!("Altın {}   Can {}\nDalga {}   Düşman {}", game.gold, game.lives, game.wave, alive + game.to_spawn as usize);
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if game.lives == 0 {
            format!("Kale düştü — {} dalga\nR ile yeniden", game.wave)
        } else if game.wave == 0 {
            "Zemine tıkla: kule (10 altın)\nDalga ile başla".into()
        } else {
            String::new()
        };
    }
}

const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "C:/Windows/Fonts/arial.ttf",
];

fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    let env = std::env::var("GIZMO_FONT").ok();
    for path in env.iter().map(String::as_str).chain(FONT_CANDIDATES.iter().copied()) {
        if let Ok(id) = renderer.load_font_file(path) {
            return id;
        }
    }
    renderer
        .load_font(gizmo::renderer::text::synthetic::synthetic_face())
        .expect("sentetik yüz her zaman ayrıştırılır")
}
