//! # Poligon — birinci şahıs atış poligonu
//!
//! Fareyle bak, WASD ile yürü, sol tıkla ateş et. Kaidelerin üstündeki kutu yığınlarını
//! devir; her düşen kutu bir puan. Kırk mermin var.
//!
//! | girdi | iş |
//! |---|---|
//! | **fare** | bakış (imleç kilitli ve gizli) |
//! | **WASD** · **Space** | yürü · zıpla |
//! | **sol tık** | ateş |
//! | **Esc** | imleci serbest bırak / geri kilitle |
//! | **R** | baştan başlat |
//! | **V** | salon yankısını aç / kapat |
//!
//! ## Neden var
//!
//! Önceki dört oyunun hiçbiri fareyle bakmadı. Bu oyun birinci şahıs kameranın ihtiyaç
//! duyduğu her şeyi kullanıyor: imleci kilitlemek, ham fare hareketi, karakter denetleyicisi,
//! kameranın ortasından fizik ışını, ışının çarptığı cisme itki. Yazarken karşılaşılan her
//! eksik aşağıda.
//!
//! ## Motorda bulunan eksikler (bu oyunu yazarken)
//!
//! - **İmleç kilitlenemiyordu.** Motorda `set_cursor_grab` / `set_cursor_visible` çağrısı hiç
//!   yoktu: fareyle bakan bir oyunun imleci pencereden çıkıp sahnenin üstünde görünüyordu.
//!   **Kapatıldı:** `gizmo::core::window::CursorOptions` kaynağı (`mouse_look()` /
//!   `free()`); pencereli uygulama değiştiği kare pencereye uyguluyor (`Locked` olmayan
//!   platformda `Confined`). Bakış zaten ham fare hareketinden (`Input::mouse_delta`) geliyordu.
//! - **Bir ECS varlığına itki uygulamanın yolu yoktu.** Gövde, konum ve hız ayrı bileşenler;
//!   itkinin formülü `Integrator`'da ve cephe onu dışa açmıyor; uyuyan gövde elle
//!   uyandırılmalı. **Kapatıldı:** `gizmo::physics::apply_impulse(world, id, itki, nokta)` —
//!   ışının çarptığı kimlikle doğrudan.
//! - **Ekran sarsıntısı yoktu.** Atışın bir tepkisi olsun diye kamerayı oyun kodunda sarsmak,
//!   bakış açısını ve nişan ışınını da sarsmak demekti — ve geri alınmazsa kamerayı kaydırmak.
//!   **Kapatıldı:** `CameraShake` bileşeni (`add(travma)`, travma² kadar yumuşak gürültü, saniyede
//!   `decay` kadar söner); çizim yolu kamerayı yalnızca o karenin resmi için oynatıp çizimden sonra
//!   bit bit geri koyuyor. Her atış küçük, her isabet biraz daha büyük bir sarsıntı.
//!
//! - **Yankı yoktu.** Kapalı bir atış poligonunda bir silah sesi duvarlardan geri gelir; motorun
//!   sesinde mekân yoktu (yalnızca su altı boğukluğu). **Kapatıldı:** `AudioManager::set_reverb`
//!   (`Reverb::room` / `hall` / `cave` ya da kendi biçimin) — Freeverb, her sesin içinde bir aşama,
//!   çalan seslere de anında; sesin bittiği yerde kesilmiyor, kuyruğu sönene kadar çalıyor. Burada
//!   salon yankısı açık başlıyor, **V** kapatıp açıyor ki fark duyulsun.
//! - **Seri atışta ses patlıyordu.** Her atış 0.35 gürültü artı yankısının kuyruğu; art arda
//!   atışlarda toplam 1'i geçiyor ve cihaz onu kare dalgaya kırpıyor. Bir sesin seviyesini
//!   kısmak bunu çözmüyor, toplamın seviyesi kimsenin elinde değildi. **Kapatıldı:**
//!   `AudioManager::set_limiter(Some(Limiter::default()))` — toplamın üstünde bir limiter, tavan
//!   −1 dBFS; toplam tavanı geçmedikçe sese dokunmuyor (bit bit aynı).
//!
//! *Sonradan bulundu (2026-11-05, `poligon --test`):* oyuncu iki kat hızlı yürüyordu (10 m/s).
//! `FixedPhysics::advance` karakter denetleyicisini her adımda koşmaya başlamıştı (`pist` için),
//! bu oyun ise onu ayrıca kare başına bir kez daha çağırmayı sürdürüyordu. **Düzeltildi.**
//!
//! ## Ortam değişkenleri
//!   * `GIZMO_FONT=<yol>` — HUD fontu
//!   * `POLIGON_OTO=1` — kendiliğinden birkaç atış (ekran görüntüsü için; `poligon --test` pencere
//!     açmadan bu atışları ve yürüme hızını sınar)

use gizmo::audio::synth::{Blip, Wave};
use gizmo::audio::AudioManager;
use gizmo::core::input::Input;
use gizmo::core::window::CursorOptions;
use gizmo::physics::components::CharacterController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::{Collider, RigidBody, Velocity};
use gizmo::prelude::*;
use gizmo::renderer::components::{Text, TextAlign, TextAnchor};
use gizmo::renderer::text::FontId;
use gizmo::simple::{HeadlessFrames, sync_transforms, FixedPhysics, SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

const AMMO: u32 = 40;
const EYE: f32 = 0.6;
const SHOT_IMPULSE: f32 = 6.0;

#[derive(Clone, Copy)]
struct Crate {
    /// Kaidenin üst yüzü — bunun altına düşen kutu devrilmiş sayılır.
    pedestal_top: f32,
}
gizmo::core::impl_component!(Crate);

struct Game {
    physics: FixedPhysics,
    player: Entity,
    camera: Entity,
    yaw: f32,
    pitch: f32,
    shots: u32,
    hud: Entity,
    banner: Entity,
    looking: bool,
    auto_shots: u32,
}

/// Sahneyi ve oyun durumunu kurar (pencere de GPU da gerekmez; `--test` aynısını kullanır).
/// `sound` kapalıyken ses aygıtı açılmaz.
fn setup(world: &mut World, font: FontId, sound: bool) -> Game {

    // Zemin ve arka duvar.
    spawn_static(world, Vec3::new(0.0, -0.5, -8.0), Vec3::new(12.0, 0.5, 14.0), Vec4::new(0.45, 0.45, 0.42, 1.0));
    spawn_static(world, Vec3::new(0.0, 3.0, -21.0), Vec3::new(12.0, 3.0, 0.5), Vec4::new(0.55, 0.5, 0.45, 1.0));
    // Dört kaide, her birinde bir kutu yığını.
    for (i, x) in [-6.0f32, -2.0, 2.0, 6.0].into_iter().enumerate() {
        let z = -12.0 - i as f32 * 1.5;
        let top = 1.0;
        spawn_static(world, Vec3::new(x, top * 0.5, z), Vec3::new(0.9, top * 0.5, 0.9), Vec4::new(0.3, 0.32, 0.36, 1.0));
        for level in 0..3 {
            for (dx, dz) in [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3)].into_iter().take(4 - level) {
                spawn_crate(world, Vec3::new(x + dx, top + 0.25 + level as f32 * 0.5, z + dz), top);
            }
        }
    }

    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(-0.9)),
        GlobalTransform::default(),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.88), 3.0, LightRole::Sun),
    ));

    // Oyuncu: bir kapsül karakter. Kamera başında.
    let player = world
        .spawn_bundle((
            Transform::new(Vec3::new(0.0, 0.9, 0.0)),
            GlobalTransform::default(),
            CharacterController { speed: 5.0, jump_speed: 5.0, ..Default::default() },
            Collider::capsule(0.3, 0.3),
            Velocity::default(),
        ))
        ;
    let camera = world
        .spawn_bundle(CameraBundle {
            position: Vec3::new(0.0, 0.9 + EYE, 0.0),
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        })
        ;
    // Sarsıntı yalnızca resimde: nişan ışını ve bakış açısı onu hiç görmüyor.
    let mut shake = CameraShake::default();
    shake.max_offset = 0.04;
    shake.max_angle = 0.012;
    shake.decay = 3.0;
    if let Some(e) = world.live(camera) {
        world.add_component(e, shake);
    }

    let _crosshair = world.spawn_bundle((Text::pinned("+", font, 28.0, TextAnchor::Center, Vec2::ZERO),));
    let hud = world
        .spawn_bundle((Text::pinned("", font, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0)),))
        ;
    let banner = world
        .spawn_bundle((Text::pinned_at("", font, 40.0, Vec2::new(0.5, 0.35), Vec2::ZERO)
            .with_anchor(TextAnchor::Center)
            .with_align(TextAlign::Center),))
        ;

    if let Some(mut audio) = sound.then(AudioManager::new).and_then(Result::ok) {
        audio.load_sound_bytes("ates", Blip::new(Wave::Noise, 3000.0, 0.12).with_slide(400.0).with_volume(0.35).wav());
        audio.load_sound_bytes("isabet", Blip::new(Wave::Square, 700.0, 0.06).with_slide(1100.0).with_volume(0.2).wav());
        // Kapalı bir poligon: atışlar duvarlardan geri geliyor.
        audio.set_reverb(Some(gizmo::audio::reverb::Reverb::hall()));
        // Seri atışta toplam 1'i geçmesin: kırpılmak yerine kısılsın.
        audio.set_limiter(Some(gizmo::audio::limiter::Limiter::default()));
        world.insert_resource(audio);
    }

    // Fareyle bakış: imleç kilitli ve gizli.
    world.insert_resource(CursorOptions::mouse_look());
    Game {
        physics: FixedPhysics::new(120),
        player,
        camera,
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        shots: 0,
        hud,
        banner,
        looking: true,
        auto_shots: if std::env::var_os("POLIGON_OTO").is_some() { 6 } else { 0 },
    }
}

/// Kaidelerinden düşmüş kutular ve toplam kutu.
fn crates_down(world: &World) -> (usize, usize) {
    let tfs = world.query::<&Transform>();
    let down = world.query::<(Entity, &Crate)>().iter().filter(|(entity, c)| tfs.get(*entity).is_some_and(|t| t.position.y < c.pedestal_top - 0.1)).count();
    (down, world.query::<(Entity, &Crate)>().iter().count())
}

/// `--test`: pencere açmadan iki sınama. W basılıyken oyuncu karakter denetleyicisinin hızıyla
/// yürümeli (5 m/s); otomatik pilotun altı atışı kutu devirmeli.
fn self_test() -> bool {
    let font = HeadlessFrames::font();
    let mut world = World::new();
    let game = setup(&mut world, font, false);
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    let at = |w: &World| w.get_resource::<Game>().and_then(|g| w.query::<&Transform>().get(g.player).map(|t| t.position)).unwrap_or_default();
    while run.elapsed < 1.0 {
        run.frame(1.0 / 60.0, update);
    }
    run.input.on_key_pressed(KeyCode::KeyW as u32);
    while run.elapsed < 2.0 {
        run.frame(1.0 / 60.0, update);
    }
    let from = at(&run.world);
    while run.elapsed < 3.0 {
        run.frame(1.0 / 60.0, update);
    }
    let to = at(&run.world);
    let speed = Vec2::new(to.x - from.x, to.z - from.z).length();

    let mut world = World::new();
    let mut game = setup(&mut world, font, false);
    game.auto_shots = 6;
    world.insert_resource(game);
    let mut run = HeadlessFrames::new(world);
    while run.elapsed < 5.0 {
        run.frame(1.0 / 60.0, update);
    }
    let (down, total) = crates_down(&run.world);
    let ok = (speed - 5.0).abs() < 0.5 && down > 0;
    println!(
        "poligon --test: W ile {speed:.2} m/s yürüdü (denetleyicinin hızı 5.00); altı atış {down}/{total} kutu düşürdü → {}",
        if ok { "tamam" } else { "BAŞARISIZ" }
    );
    ok
}

fn main() {
    if std::env::args().any(|a| a == "--test") {
        std::process::exit(if self_test() { 0 } else { 1 });
    }
    App::<SimpleSceneState>::new("Gizmo Engine - Poligon", 1280, 720)
        .with_simple_scene(|scene, _state| {
            let r = &mut *scene.renderer;
            r.deband_dither = true;
            let font = load_font(r);
            let game = setup(scene.world, font, true);
            scene.world.insert_resource(game);
        })
        .replace_update(update)
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn spawn_static(world: &mut World, at: Vec3, half: Vec3, colour: Vec4) {
    world.spawn_bundle((
        Transform::new(at).with_scale(half),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(colour, 0.8, 0.0),
        MeshRenderer::new(),
        RigidBody::new_static(),
        Collider::box_collider(half),
    ));
}

fn spawn_crate(world: &mut World, at: Vec3, pedestal_top: f32) {
    world.spawn_bundle((
        Transform::new(at).with_scale(Vec3::splat(0.24)),
        GlobalTransform::default(),
        MeshSource("standard_cube".into()),
        MaterialDesc::new().with_pbr(Vec4::new(0.7, 0.5, 0.25, 1.0), 0.7, 0.0),
        MeshRenderer::new(),
        RigidBody::new(2.0, true),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(0.24)),
        Crate { pedestal_top },
    ));
}

fn update(world: &mut World, _state: &mut SimpleSceneState, dt: f32, input: &Input) {
    let Some(mut game) = world.remove_resource::<Game>() else { return };

    if input.is_key_just_pressed(KeyCode::KeyV as u32) {
        if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
            let next = if audio.reverb().is_some() { None } else { Some(gizmo::audio::reverb::Reverb::hall()) };
            audio.set_reverb(next);
        }
    }
    if input.is_key_just_pressed(KeyCode::Escape as u32) {
        game.looking = !game.looking;
        world.insert_resource(if game.looking { CursorOptions::mouse_look() } else { CursorOptions::free() });
    }

    // Bakış: ham fare hareketi.
    if game.looking {
        let (dx, dy) = input.mouse_delta();
        game.yaw += dx * 0.0025;
        game.pitch = (game.pitch - dy * 0.0025).clamp(-1.45, 1.45);
    }
    let forward = Camera::forward_from(game.yaw, game.pitch);
    let flat_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = Camera::right_from(game.yaw);

    // Yürüme.
    {
        let (mx, my) = input.move_axis();
        let mut kccs = world.query_mut::<&mut CharacterController>();
        if let Some(mut kcc) = kccs.get_mut(game.player) {
            kcc.target_velocity = (right * mx + flat_forward * my) * kcc.speed;
            if input.is_key_just_pressed(KeyCode::Space as u32) {
                kcc.jump_buffer_timer = kcc.jump_buffer_time;
            }
        }
    }

    // Ateş.
    let fire = (game.looking && input.is_mouse_button_just_pressed(0)) || game.auto_shots > 0;
    if fire && game.shots < AMMO {
        if game.auto_shots > 0 {
            game.auto_shots -= 1;
            // Otomatik atış: en yakın kaideye doğru biraz sağa sola, yığınların üst kasalarına.
            // En alttaki kasa yanındakilere ve üstündekilerin yüküne sıkışık: bir atış onu
            // birkaç santim kaydırır, devirmez. 2026-11-10'a kadar deviriyordu — kutu–kutu teması
            // yığınları gerçekte olduğundan zayıf tutuyordu (`analytical.rs`,
            // `t1_an_offset_box_stands_while_its_centre_is_over_the_one_below`) ve nişan da ona
            // göre aşağıdaydı (-0.02).
            game.yaw = -std::f32::consts::FRAC_PI_2 + (game.auto_shots as f32 - 3.0) * 0.12;
            game.pitch = 0.03;
        }
        game.shots += 1;
        play(world, "ates");
        kick(world, &game, 0.35);
        shoot(world, &game, Camera::forward_from(game.yaw, game.pitch));
    }

    // `advance` karakter denetleyicisini her adımda kendisi koşuyor. Burada bir kez daha, kare
    // süresiyle çağrılıyordu: oyuncu 5 m/s yerine 10 m/s yürüyordu (`poligon --test` ölçtü).
    game.physics.advance(world, dt);
    sync_transforms(world);

    // Kamera oyuncunun başında.
    let head = world.query::<&Transform>().get(game.player).map(|t| t.position + Vec3::Y * EYE);
    if let Some(head) = head {
        if let Some(mut t) = world.query_mut::<&mut Transform>().get_mut(game.camera) {
            t.position = head;
        }
    }
    if let Some(mut cam) = world.query_mut::<&mut Camera>().get_mut(game.camera) {
        cam.yaw = game.yaw;
        cam.pitch = game.pitch;
    }

    draw_hud(world, &game);
    world.insert_resource(game);
}

/// Kameranın ortasından bir ışın; çarptığı dinamik cisme çarpma noktasında bir itki.
fn shoot(world: &mut World, game: &Game, dir: Vec3) {
    let Some(origin) = world.query::<&Transform>().get(game.camera).map(|t| t.position) else { return };
    let hit = {
        let Some(physics) = world.get_resource::<PhysicsWorld>() else { return };
        let ray = gizmo::physics::raycast::Ray::new(origin, dir);
        physics.raycast_excluding(&ray, 100.0, gizmo::physics::BodyHandle::from_id(game.player.id()))
    };
    let Some(hit) = hit else { return };
    let Some(id) = world.entity(hit.entity.id()) else { return };
    if world.query::<&Crate>().get(id).is_none() {
        return;
    }
    if gizmo::physics::apply_impulse(world, id, dir * SHOT_IMPULSE, hit.point) {
        play(world, "isabet");
        kick(world, game, 0.25);
    }
}

/// Kameraya travma ekler.
fn kick(world: &mut World, game: &Game, trauma: f32) {
    if let Some(mut shake) = world.query_mut::<&mut CameraShake>().get_mut(game.camera) {
        shake.add(trauma);
    }
}

fn play(world: &mut World, name: &str) {
    if let Some(mut audio) = world.get_resource_mut::<AudioManager>() {
        let _ = audio.play(name);
    }
}

fn draw_hud(world: &mut World, game: &Game) {
    let (down, total) = {
        let tfs = world.query::<&Transform>();
        let total = world.query::<(Entity, &Crate)>().iter().count();
        let down = world.query::<(Entity, &Crate)>().iter()
            .filter(|(entity, c)| tfs.get(*entity).is_some_and(|t| t.position.y < c.pedestal_top - 0.1))
            .count();
        (down, total)
    };
    let mut texts = world.query_mut::<&mut Text>();
    if let Some(mut t) = texts.get_mut(game.hud) {
        t.content = format!("Düşen {down}/{total}   Mermi {}", AMMO - game.shots);
    }
    if let Some(mut t) = texts.get_mut(game.banner) {
        t.content = if down == total {
            "Hepsi düştü!".into()
        } else if game.shots >= AMMO {
            format!("Mermi bitti — {down}/{total}")
        } else if !game.looking {
            "Duraklatıldı — Esc ile devam".into()
        } else {
            String::new()
        };
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
