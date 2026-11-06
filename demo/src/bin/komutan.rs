//! # Komutan — küçük bir gerçek zamanlı strateji çatışması
//!
//! Mavi bölüğün on iki askeri sende: fareyle kutu çizip seç, sağ tıkla yürüt; menzile giren
//! kırmızılara kendileri ateş ediyorlar. Haritanın öbür ucundaki kırmızı karargâhı koruyan on dört
//! askeri yok et. Motorun on beşinci oyunu ve ilk **birlik yöneten**i: ekrandaki bir dikdörtgenle
//! dünyadaki birimleri seçmek, birçok birimi aynı anda bir noktaya yürütmek, her birimin üstünde
//! bir can çubuğu.
//!
//! | girdi | iş |
//! |---|---|
//! | **sol tık / sürükle** | birim seç / kutuyla seç |
//! | **sağ tık** | seçilenleri oraya yürüt |
//! | **WASD · sol çubuk** | kamerayı kaydır |
//! | **tekerlek** | yakınlaş / uzaklaş |
//! | **R** | yeniden başla |
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **Dünyadaki bir noktanın ekranda nerede olduğu sorulamıyordu.** İmleçten dünyaya ışın vardı
//!   (`cursor_ray`, `cursor_on_plane`), tersi yoktu; kutu seçimi birimleri ekrana izdüşürmek, can
//!   çubuğu birimin üstündeki pikseli bilmek zorunda. **Kapatıldı:**
//!   `gizmo::pointer::world_to_screen(world, nokta)` — kameranın arkasındaysa `None`, bölünmüş
//!   ekranda kameranın kendi dikdörtgeninde.
//! - **…ve imleç ışını bayat bir kamera konumundan çıkıyordu.** `cursor_ray` kameranın
//!   `GlobalTransform`'unu okuyordu; oyun kodunda o bir kare geride, bu karede doğan bir kamera
//!   için başlangıç noktası. `world_to_screen`'in örneği hiçbir şey izdüşüremeyince bulundu.
//!   **Düzeltildi:** kameranın bu karedeki yeri (`world_position`).
//! - **Ajanların birbirini itme mesafesi sabitti.** Ayrışma yarıçapı 1,5 m ve ağırlığı 1,5 koda
//!   gömülüydü; 1,2 m aralıklı bir düzene varan birimler birbirini 1,5 m'ye itip dağıtırdı.
//!   **Kapatıldı:** `NavAgent::with_separation(yarıçap, ağırlık)` (alanlar `separation_radius`,
//!   `separation_weight`; eski sahne dosyaları varsayılanla yükleniyor; inspector'da da var).
//!
//! Gerisi ilk denemede çalıştı: `NavGrid` + `NavAgent` ile yirmi altı birimin yol bulması,
//! birimlerin üstünde ekrana izdüşürülmüş arayüz düğümleri olarak can çubukları (`inset` her
//! kare), `IsHidden` ile seçim halkaları, mutlak konumlu yarı saydam bir düğüm olarak seçim kutusu.
//!
//! ## Ortam değişkenleri
//!   * `KOMUTAN_OTO=1` — kendi kendine oynar (ekran görüntüsü için; `komutan --test` pencere açmadan
//!     böyle bir çatışmayı ve emirsiz bir dakikayı oynar)
//!   * `GIZMO_FONT=<yol>` — HUD fontu

use gizmo::ai::{NavAgent, NavGrid};
use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::hierarchy::HierarchyExt;
use gizmo::core::input::Input;
use gizmo::math::Rng;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use gizmo::ui::{BackgroundColor, NodeBundle, PositionType, Style, UiPlugin, UiRect, Val, ZIndex};

const HALF_W: f32 = 22.0;
const HALF_D: f32 = 16.0;
const RANGE: f32 = 4.5;
const RELOAD: f32 = 0.8;
const DAMAGE: f32 = 1.0;
const HP: f32 = 6.0;
const ENEMY_BASE: Vec3 = Vec3::new(15.0, 0.0, -9.0);
/// Sürüklemenin kutu sayılması için gereken piksel.
const DRAG_PX: f32 = 6.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Side {
    Blue,
    Red,
}

#[derive(Clone, Copy)]
struct Unit {
    side: Side,
    hp: f32,
    reload: f32,
    /// Bu birimin can çubuğu (arayüz düğümü) ve seçim halkası.
    bar: u32,
    fill: u32,
    ring: u32,
}
gizmo::core::impl_component!(Unit);

#[derive(Clone, Copy)]
struct Tracer {
    life: f32,
}
gizmo::core::impl_component!(Tracer);

struct Game {
    physics: FixedPhysics,
    selected: Vec<u32>,
    /// Sol tuşun basıldığı piksel (sürükleme sürüyorsa).
    drag_from: Option<Vec2>,
    marquee: u32,
    hud: u32,
    banner: u32,
    rng: Rng,
    auto: bool,
    auto_clock: f32,
    over: Option<Side>,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -0.1, 0.0)).with_scale(Vec3::new(HALF_W, 0.1, HALF_D)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.36, 0.42, 0.27, 1.0), 0.95, 0.0),
        MeshRenderer::new(),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.95)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 3.0, LightRole::Sun),
    ));
    world.spawn_bundle(CameraBundle {
        position: Vec3::new(-8.0, 18.0, 12.0),
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: -1.0,
        primary: true,
        ..Default::default()
    });
    let hud = world
        .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        .id();
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        .id();
    // Seçim kutusu: sürükleme sürerken görünen yarı saydam bir arayüz dikdörtgeni.
    let marquee = world
        .spawn_bundle(NodeBundle {
            style: Style { position_type: PositionType::Absolute, width: Val::Px(0.0), height: Val::Px(0.0), ..Default::default() },
            background_color: BackgroundColor(Vec4::new(0.4, 0.8, 1.0, 0.0)),
            ..Default::default()
        })
        .id();
    world.add_component(world.get_entity(marquee).expect("just spawned"), ZIndex(10));
    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("ates", Blip::new(Wave::Noise, 900.0, 0.05).with_volume(0.08).wav());
        audio.load_sound_bytes("emir", Blip::new(Wave::Square, 660.0, 0.06).with_slide(880.0).with_volume(0.12).wav());
        audio.load_sound_bytes("dus", Blip::new(Wave::Sine, 300.0, 0.25).with_slide(90.0).with_volume(0.2).wav());
        world.insert_resource(audio);
    }
    let mut game = Game {
        physics: FixedPhysics::new(60),
        selected: Vec::new(),
        drag_from: None,
        marquee,
        hud,
        banner,
        rng: Rng::new(0xC0FFEE),
        auto: std::env::var_os("KOMUTAN_OTO").is_some(),
        auto_clock: 0.0,
        over: None,
    };
    restart(world, &mut game);
    game
}

/// `--test`: pencere açmadan iki çatışma. Otomatik komutan bütün bölüğü karargâha yürütür: birimler
/// yol bulup yürümeli, menzile giren düşmanı vurmalı, çatışma bir sonuca varmalı ve kırmızı kayıp
/// vermeli. (On iki maviyle on dört eşit kırmızıya topyekûn saldırı kaybediyor — oyun taktik
/// istiyor; burada sınanan çatışmanın işlediği.) Hiç emir vermeyen komutanın bölüğü bir dakika
/// boyunca kimseyle çatışmamalı.
fn battle(auto: bool, limit: f32) -> (Option<Side>, usize, usize, f32) {
    let mut world = World::new();
    let mut game = setup(&mut world, HeadlessFrames::font(), false);
    game.auto = auto;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world).add_plugin(UiPlugin);
    while run.elapsed < limit && run.world.get_resource::<Game>().is_some_and(|g| g.over.is_none()) {
        run.frame(1.0 / 60.0, update);
    }
    let alive = |side: Side| run.world.borrow::<Unit>().iter().filter(|u| u.side == side && u.hp > 0.0).count();
    (run.world.get_resource::<Game>().and_then(|g| g.over), alive(Side::Blue), alive(Side::Red), run.elapsed)
}

fn self_test() -> bool {
    let (won, blue, red, time) = battle(true, 240.0);
    let (idle, _, idle_red, _) = battle(false, 60.0);
    let ok = won.is_some() && red < 14 && idle.is_none() && idle_red == 14;
    println!(
        "komutan --test: otomatik {:?} ({time:.0} s, mavi {blue} sağ, kırmızı {red}); emirsiz bir dakikada {:?}, kırmızı {idle_red} sağ → {}",
        won,
        idle,
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Komutan", 1280, 720)
        .add_plugin(UiPlugin)
        .with_simple_scene(|scene, _state| {
            let font = load_font(scene.renderer);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn restart(world: &mut World, game: &mut Game) {
    let units: Vec<u32> = world.query::<(Entity, &Unit)>().unwrap().iter().map(|(entity, _)| entity.id()).collect();
    for id in units {
        despawn_unit(world, id);
    }
    world.despawn_all_with::<Tracer>();
    world.despawn_all_with::<Rock>();
    game.selected.clear();
    game.over = None;
    game.auto_clock = 0.0;
    game.physics.reset();

    // Kayalar: ızgarada engel, sahnede gri kutular. İki üs çevresi boş.
    let mut grid = NavGrid::centred_on(1.0, (HALF_W * 2.0) as i32, (HALF_D * 2.0) as i32, Vec3::ZERO);
    grid.needs_rebuild = false;
    for _ in 0..40 {
        let at = Vec3::new(game.rng.range(-HALF_W + 2.0..HALF_W - 2.0).floor() + 0.5, 0.0, game.rng.range(-HALF_D + 2.0..HALF_D - 2.0).floor() + 0.5);
        if at.distance(Vec3::new(-15.0, 0.0, 9.0)) < 6.0 || at.distance(ENEMY_BASE) < 6.0 {
            continue;
        }
        grid.add_obstacle_world(at);
        world.spawn_bundle((
            Transform::new(at + Vec3::Y * 0.5).with_scale(Vec3::new(0.5, 0.5, 0.5)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_pbr(Vec4::new(0.45, 0.44, 0.42, 1.0), 0.9, 0.0),
            MeshRenderer::new(),
            Rock,
        ));
    }
    world.insert_resource(grid);

    for i in 0..12 {
        let at = Vec3::new(-17.0 + (i % 4) as f32 * 1.4, 0.0, 7.0 + (i / 4) as f32 * 1.4);
        spawn_unit(world, Side::Blue, at);
    }
    for i in 0..14 {
        let at = ENEMY_BASE + Vec3::new((i % 5) as f32 * 1.4 - 2.8, 0.0, (i / 5) as f32 * 1.4 - 1.4);
        spawn_unit(world, Side::Red, at);
    }
}

#[derive(Clone, Copy)]
struct Rock;
gizmo::core::impl_component!(Rock);

fn spawn_unit(world: &mut World, side: Side, at: Vec3) {
    let colour = match side {
        Side::Blue => Vec4::new(0.2, 0.4, 0.9, 1.0),
        Side::Red => Vec4::new(0.85, 0.2, 0.18, 1.0),
    };
    // Can çubuğu: kenarlıklı koyu bir kutu ve içinde dolan renkli bir şerit.
    let bar = world.spawn_bundle(NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            width: Val::Px(30.0),
            height: Val::Px(5.0),
            padding: UiRect::all(Val::Px(1.0)),
            ..Default::default()
        },
        background_color: BackgroundColor(Vec4::new(0.0, 0.0, 0.0, 0.6)),
        ..Default::default()
    });
    let fill = world.spawn_bundle(NodeBundle {
        style: Style { width: Val::Percent(100.0), height: Val::Percent(100.0), ..Default::default() },
        background_color: BackgroundColor(colour),
        ..Default::default()
    });
    world.add_child(bar, fill);
    let ring = world.spawn_bundle((
        Transform::new(at + Vec3::Y * 0.02).with_scale(Vec3::new(0.55, 0.01, 0.55)),
        GlobalTransform::default(),
        MeshSource("cylinder".into()),
        MaterialDesc::new().with_unlit(Vec4::new(0.3, 1.0, 0.4, 1.0)),
        MeshRenderer::new(),
        gizmo::core::component::IsHidden,
    ));
    world.spawn_bundle((
        Transform::new(at + Vec3::Y * 0.35).with_scale(Vec3::new(0.3, 0.35, 0.3)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.5, 0.0),
        MeshRenderer::new(),
        RigidBody::new_kinematic(),
        Velocity::default(),
        Collider::sphere(0.3),
        // Düzen 1,2 m aralıklı: ayrışma ondan kısa, yoksa yerine varan bölük kendini dağıtır.
        NavAgent::new(3.0, 12.0, 0.6).with_separation(1.0, 1.5),
        Unit { side, hp: HP, reload: 0.0, bar: bar.id(), fill: fill.id(), ring: ring.id() },
    ));
}

fn despawn_unit(world: &mut World, id: u32) {
    let Some(unit) = world.borrow::<Unit>().get(id).copied() else { return };
    for part in [unit.bar, unit.ring] {
        if let Some(e) = world.get_entity(part) {
            world.despawn_recursive(e);
        }
    }
    if let Some(e) = world.get_entity(id) {
        world.despawn(e);
    }
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };
    let dt = dt.min(0.05);
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        restart(world, &mut game);
    }
    camera_controls(world, input, dt);
    if game.auto {
        autoplay(world, &mut game, dt);
    } else if game.over.is_none() {
        select_and_order(world, &mut game, input);
    }
    combat(world, &mut game, dt);
    game.physics.advance(world, dt);
    gizmo::ai::system::ai_frame(world, dt);
    sync_transforms(world);
    overlays(world, &game);

    let (blue, red) = count(world);
    if game.over.is_none() {
        if red == 0 {
            game.over = Some(Side::Blue);
        } else if blue == 0 {
            game.over = Some(Side::Red);
        }
    }
    let hud = format!(
        "Mavi {blue}   Kırmızı {red}   Seçili {}\nSol: seç / kutu   Sağ: yürüt   WASD: kamera   R: baştan",
        game.selected.len()
    );
    set_text(world, game.hud, &hud);
    let banner = match game.over {
        Some(Side::Blue) => "Zafer!\nR ile yeniden",
        Some(Side::Red) => "Bölük düştü.\nR ile yeniden",
        None => "",
    };
    set_text(world, game.banner, banner);
    world.insert_resource(game);
}

fn count(world: &World) -> (usize, usize) {
    let units = world.borrow::<Unit>();
    let blue = units.iter().filter(|u| u.side == Side::Blue).count();
    (blue, units.iter().count() - blue)
}

/// Sol tuş: tıklama tek birim, sürükleme kutu; sağ tuş: seçilenlere yürüme emri.
fn select_and_order(world: &mut World, game: &mut Game, input: &Input) {
    let (mx, my) = input.mouse_position();
    let mouse = Vec2::new(mx, my);
    if input.is_mouse_button_just_pressed(0) {
        game.drag_from = Some(mouse);
    }
    if input.is_mouse_button_just_released(0) {
        if let Some(from) = game.drag_from.take() {
            let (lo, hi) = (from.min(mouse), from.max(mouse));
            let boxed = (hi - lo).max_element() > DRAG_PX;
            let mut picked = Vec::new();
            let mut nearest: Option<(f32, u32)> = None;
            for (id, at) in blue_units(world) {
                let Some(p) = gizmo::pointer::world_to_screen(world, at + Vec3::Y * 0.35) else { continue };
                if boxed {
                    if p.cmpge(lo).all() && p.cmple(hi).all() {
                        picked.push(id);
                    }
                } else {
                    let d = p.distance(mouse);
                    if d < 24.0 && nearest.is_none_or(|(best, _)| d < best) {
                        nearest = Some((d, id));
                    }
                }
            }
            picked.extend(nearest.map(|(_, id)| id));
            game.selected = picked;
        }
    }
    if input.is_mouse_button_just_pressed(1) {
        if let Some(to) = gizmo::pointer::cursor_on_plane(world, Vec3::ZERO, Vec3::Y) {
            order(world, game, to);
        }
    }
}

fn blue_units(world: &World) -> Vec<(u32, Vec3)> {
    let transforms = world.borrow::<Transform>();
    world.query::<(Entity, &Unit)>().unwrap().iter()
        .filter(|(_, u)| u.side == Side::Blue)
        .filter_map(|(entity, _)| transforms.get(entity).map(|t| (entity.id(), t.position)))
        .collect()
}

/// Seçilenleri `to` çevresinde bir kare düzene yürütür — hepsi aynı noktaya koşup itişmesin.
fn order(world: &mut World, game: &mut Game, to: Vec3) {
    game.selected.retain(|&id| world.borrow::<Unit>().get(id).is_some());
    let n = game.selected.len();
    if n == 0 {
        return;
    }
    let side = (n as f32).sqrt().ceil() as usize;
    for (i, &id) in game.selected.iter().enumerate() {
        let offset = Vec3::new((i % side) as f32 - (side - 1) as f32 / 2.0, 0.0, (i / side) as f32 - (side - 1) as f32 / 2.0) * 1.2;
        if let Some(mut agent) = world.borrow_mut::<NavAgent>().get_mut(id) {
            agent.set_target(to + offset);
        }
    }
    play(world, "emir");
}

/// Menzildeki en yakın düşmana ateş; kırmızılar da birimleri menzile girince karşılık veriyor.
fn combat(world: &mut World, game: &mut Game, dt: f32) {
    let all: Vec<(u32, Side, Vec3)> = {
        let transforms = world.borrow::<Transform>();
        world.query::<(Entity, &Unit)>().unwrap().iter().filter_map(|(entity, u)| transforms.get(entity).map(|t| (entity.id(), u.side, t.position))).collect()
    };
    let mut hits: Vec<(u32, Vec3, Vec3)> = Vec::new();
    {
        let mut units = world.borrow_mut::<Unit>();
        for &(id, side, at) in &all {
            let Some(mut unit) = units.get_mut(id) else { continue };
            unit.reload = (unit.reload - dt).max(0.0);
            if unit.reload > 0.0 {
                continue;
            }
            let target = all
                .iter()
                .filter(|(_, s, _)| *s != side)
                .map(|&(t, _, p)| (t, p, p.distance(at)))
                .filter(|&(_, _, d)| d <= RANGE)
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((t, p, _)) = target {
                unit.reload = RELOAD;
                hits.push((t, at, p));
            }
        }
    }
    let mut dead = Vec::new();
    for &(target, from, to) in &hits {
        if let Some(mut u) = world.borrow_mut::<Unit>().get_mut(target) {
            u.hp -= DAMAGE;
            if u.hp <= 0.0 && !dead.contains(&target) {
                dead.push(target);
            }
        }
        let mid = (from + to) * 0.5 + Vec3::Y * 0.4;
        let len = from.distance(to);
        let yaw = (to.x - from.x).atan2(to.z - from.z);
        world.spawn_bundle((
            Transform::new(mid).with_rotation(Quat::from_rotation_y(yaw)).with_scale(Vec3::new(0.03, 0.03, len * 0.5)),
            GlobalTransform::default(),
            MeshSource("standard_cube".into()),
            MaterialDesc::new().with_unlit(Vec4::new(1.0, 0.9, 0.5, 1.0)),
            MeshRenderer::new(),
            Tracer { life: 0.08 },
        ));
    }
    if !hits.is_empty() {
        play(world, "ates");
    }
    for id in dead {
        despawn_unit(world, id);
        game.selected.retain(|&s| s != id);
        play(world, "dus");
    }
    let mut gone = Vec::new();
    for (entity, mut t) in world.query_mut::<(Entity, Mut<Tracer>)>().unwrap().iter_mut() {
        t.life -= dt;
        if t.life <= 0.0 {
            gone.push(entity);
        }
    }
    for entity in gone {
        world.despawn(entity);
    }
}

/// Can çubukları birimlerin üstünde (ekrana izdüşüm), seçim halkaları ayaklarında, seçim kutusu
/// sürüklenirken.
fn overlays(world: &mut World, game: &Game) {
    let units: Vec<(u32, Unit, Vec3)> = {
        let transforms = world.borrow::<Transform>();
        world.query::<(Entity, &Unit)>().unwrap().iter().filter_map(|(entity, u)| transforms.get(entity).map(|t| (entity.id(), *u, t.position))).collect()
    };
    for (id, unit, at) in units {
        let screen = gizmo::pointer::world_to_screen(world, at + Vec3::Y * 1.0);
        if let Some(mut style) = world.borrow_mut::<Style>().get_mut(unit.bar) {
            match screen {
                Some(p) => {
                    style.display = gizmo::ui::Display::Flex;
                    style.inset = UiRect::new(Val::Px(p.x - 15.0), Val::Auto, Val::Px(p.y - 3.0), Val::Auto);
                }
                None => style.display = gizmo::ui::Display::None,
            }
        }
        if let Some(mut style) = world.borrow_mut::<Style>().get_mut(unit.fill) {
            style.width = Val::Percent((unit.hp / HP * 100.0).clamp(0.0, 100.0));
        }
        if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(unit.ring) {
            t.position = Vec3::new(at.x, 0.02, at.z);
        }
        let selected = game.selected.contains(&id);
        if let Some(e) = world.get_entity(unit.ring) {
            let hidden = world.borrow::<gizmo::core::component::IsHidden>().get(unit.ring).is_some();
            if selected && hidden {
                world.remove_component::<gizmo::core::component::IsHidden>(e);
            } else if !selected && !hidden {
                world.add_component(e, gizmo::core::component::IsHidden);
            }
        }
    }
    // Seçim kutusu.
    let input_mouse = world.get_resource::<Input>().map(|i| i.mouse_position());
    let rect = match (game.drag_from, input_mouse) {
        (Some(from), Some((mx, my))) => {
            let to = Vec2::new(mx, my);
            let (lo, hi) = (from.min(to), from.max(to));
            ((hi - lo).max_element() > DRAG_PX).then_some((lo, hi))
        }
        _ => None,
    };
    if let Some(mut style) = world.borrow_mut::<Style>().get_mut(game.marquee) {
        let (lo, hi) = rect.unwrap_or((Vec2::ZERO, Vec2::ZERO));
        style.inset = UiRect::new(Val::Px(lo.x), Val::Auto, Val::Px(lo.y), Val::Auto);
        style.width = Val::Px(hi.x - lo.x);
        style.height = Val::Px(hi.y - lo.y);
    }
    if let Some(mut bg) = world.borrow_mut::<BackgroundColor>().get_mut(game.marquee) {
        bg.0.w = if rect.is_some() { 0.25 } else { 0.0 };
    }
}

fn camera_controls(world: &mut World, input: &Input, dt: f32) {
    let (mx, my) = input.move_axis();
    let wheel = input.mouse_scroll();
    let Some(id) = world.query::<(Entity, &Camera)>().unwrap().iter().next().map(|(entity, _)| entity.id()) else { return };
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(id) {
        t.position += Vec3::new(mx, 0.0, -my) * 14.0 * dt;
        t.position.y = (t.position.y - wheel * 1.5).clamp(8.0, 30.0);
        t.position.x = t.position.x.clamp(-HALF_W, HALF_W);
        t.position.z = t.position.z.clamp(-HALF_D + 6.0, HALF_D + 12.0);
    }
}

/// Kendi kendine: önce herkesi kutuyla seçer gibi seçer, sonra kırmızı üsse yürütür.
fn autoplay(world: &mut World, game: &mut Game, dt: f32) {
    game.auto_clock += dt;
    if game.over.is_some() {
        if game.auto_clock > 4.0 {
            restart(world, game);
        }
        return;
    }
    if game.selected.is_empty() || game.auto_clock > 3.0 {
        game.selected = blue_units(world).into_iter().map(|(id, _)| id).collect();
        let reds: Vec<Vec3> = {
            let transforms = world.borrow::<Transform>();
            world.query::<(Entity, &Unit)>().unwrap().iter().filter(|(_, u)| u.side == Side::Red).filter_map(|(entity, _)| transforms.get(entity).map(|t| t.position)).collect()
        };
        let target = reds.first().copied().unwrap_or(ENEMY_BASE);
        order(world, game, target);
        game.auto_clock = 0.0;
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn set_text(world: &mut World, id: u32, s: &str) {
    if let Some(mut t) = world.borrow_mut::<Text>().get_mut(id) {
        if t.content != s {
            t.content = s.to_string();
        }
    }
}


fn load_font(renderer: &mut gizmo::renderer::Renderer) -> FontId {
    // A font the machine has (`GIZMO_FONT`, or the platform's usual faces); without one, the
    // synthetic test face, so the game still runs.
    renderer.load_system_font().unwrap_or_else(|_| {
        renderer
            .load_font(gizmo::renderer::text::synthetic::synthetic_face())
            .expect("sentetik yüz her zaman ayrıştırılır")
    })
}
