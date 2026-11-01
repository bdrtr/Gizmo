//! # 2D sprite'lar
//!
//! Motorun ilk 2D yolu (`docs/CAPABILITY_GAPS.md` §A1): bir ortografik kamera, `-Z`'ye bakan bir
//! düzlem, ve üzerinde [`Sprite`]'lar. Depoya tek bir görsel dosyası eklenmedi — her görüntü
//! aşağıda piksel piksel üretiliyor ve [`Renderer::load_sprite_rgba`] ile yükleniyor. Gerçek bir
//! oyunda yerine `load_sprite_image_file("player.png", SpriteFilter::Nearest)` gelir.
//!
//! | gösterilen | nasıl |
//! |---|---|
//! | sprite sheet animasyonu | oyuncu 4 karelik bir şeritten, [`SpriteAnimation`] — duruma göre `set_run` |
//! | yöne göre çevirme | `flip_x`, sola giderken |
//! | katman sırası | gökyüzü −10, uzak tepe −6, yakın tepe −5, zemin 0, coin 1, oyuncu 2 |
//! | paralaks | iki tepe katmanı kamerayı farklı oranlarda izliyor |
//! | gizleme | toplanan coin'e `IsHidden` ekleniyor — mesh'lerle aynı kural |
//! | dönüş / ölçek | coin'ler `Transform` ölçeğiyle dönüyor; negatif ölçek arka yüzü gösteriyor |
//! | ekran uzayı (HUD) | sol üstte enerji çubuğu, sağ üstte toplanan coin'ler |
//! | pikselli örnekleme | `SpriteFilter::Nearest`; gökyüzü gradyanı `Linear` |
//!
//! Kontroller: **A/D**, **←/→** ya da gamepad sol çubuğu ile yürü, **Boşluk** zıpla.
//!
//! ## Ne yok
//!
//! Bu bir sprite **çizimi**; etrafındaki 2D oyun altyapısı henüz yok: 2D fizik/çarpışma (coin
//! toplama burada elle bir mesafe testi), tilemap bileşeni (zemin 120 ayrı sprite — tek görüntü
//! olduğu için tek çizim çağrısı). Kare seçimi eskiden bu dosyada elle yapılıyordu; artık
//! `SpriteAnimation` bileşeni (motor her karede ilerletiyor) ve oyunun durumundan seçilen bir
//! aralık (`set_run`).

use gizmo::core::component::IsHidden;
use gizmo::core::system::System;
use gizmo::prelude::*;
use gizmo::renderer::components::{ProjectionMode, Sprite, SpriteAnimation};
use gizmo::renderer::sprite::SpriteFilter;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use gizmo::winit::keyboard::KeyCode;

/// Görüş hacminin dikey uzanımı, dünya birimi.
const VIEW_HEIGHT: f32 = 12.0;
/// Zeminin üst yüzeyi.
const GROUND_TOP: f32 = -4.5;
/// Bir tepe şeridinin genişliği. Görüş genişliğinden (16:9'da ~21,3) büyük olmalı ki üç kopya
/// her zaman ekranı kaplasın.
const HILL_WIDTH: f32 = 24.0;
/// Enerji çubuğunun dolu genişliği, piksel.
const BAR_WIDTH: f32 = 200.0;
/// Çubuğun renkleri. `Sprite::color` LİNEER uzayda: 0,35 ekranda sRGB 160 olur, yani
/// `(0,35, 0,9, 0,45)` doygun yeşil değil nane rengidir. Doygun bir renk için düşük kanallar
/// gerçekten düşük olmalı.
const BAR_FULL: Vec4 = Vec4::new(0.04, 0.75, 0.08, 1.0);
const BAR_LOW: Vec4 = Vec4::new(0.9, 0.06, 0.03, 1.0);

struct Game {
    player: u32,
    camera: u32,
    /// `(varlık, paralaks oranı, kopya sırası)`.
    hills: Vec<(u32, f32, f32)>,
    coins: Vec<(u32, bool)>,
    coin_icons: Vec<u32>,
    bar_fill: u32,
    x: f32,
    y: f32,
    vy: f32,
    facing_left: bool,
    walk_time: f32,
    energy: f32,
    time: f32,
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - 2D Sprites", 1280, 720)
        .with_simple_scene(|scene, state| {
            let r = &mut *scene.renderer;
            let load = |r: &mut gizmo::renderer::Renderer, (px, w, h): (Vec<u8>, u32, u32), f| {
                r.load_sprite_rgba(&px, w, h, f).expect("üretilen görüntü yüklenemedi")
            };
            let sky = load(r, sky_gradient(), SpriteFilter::Linear);
            let far_hills = load(r, hills(128, 32, 0.55, [70, 96, 140]), SpriteFilter::Nearest);
            let near_hills = load(r, hills(128, 32, 0.8, [52, 120, 84]), SpriteFilter::Nearest);
            let grass = load(r, grass_tile(), SpriteFilter::Nearest);
            let coin = load(r, coin_image(), SpriteFilter::Nearest);
            let hero = load(r, hero_sheet(), SpriteFilter::Nearest);
            let white = load(r, (vec![255; 4], 1, 1), SpriteFilter::Nearest);

            let world_sprite = |world: &mut World, pos: Vec3, sprite: Sprite| {
                world
                    .spawn_bundle((Transform::new(pos), GlobalTransform::default(), sprite))
                    .id()
            };

            // Gökyüzü kameraya bağlı: her kare kameranın x'ine taşınıyor (bkz. `update`).
            let sky_e = world_sprite(
                scene.world,
                Vec3::new(0.0, 0.0, -5.0),
                Sprite::world(sky, Vec2::new(40.0, VIEW_HEIGHT + 2.0)).with_layer(-10),
            );

            let mut hills_list = vec![(sky_e, 1.0, 0.0)];
            for (image, factor, layer, y) in [(far_hills, 0.8, -6, -1.5), (near_hills, 0.5, -5, -2.5)] {
                for copy in [-1.0, 0.0, 1.0] {
                    let e = world_sprite(
                        scene.world,
                        Vec3::new(copy * HILL_WIDTH, y, -4.0),
                        Sprite::world(image, Vec2::new(HILL_WIDTH, 6.0)).with_layer(layer),
                    );
                    hills_list.push((e, factor, copy));
                }
            }

            for i in -60..60 {
                world_sprite(
                    scene.world,
                    Vec3::new(i as f32 + 0.5, GROUND_TOP, 0.0),
                    Sprite::world(grass, Vec2::ONE).with_anchor(Vec2::new(0.5, 0.0)),
                );
                // Zeminin altını dolduran koyu toprak — aynı beyaz piksel, renk tonuyla.
                world_sprite(
                    scene.world,
                    Vec3::new(i as f32 + 0.5, GROUND_TOP - 1.0, 0.0),
                    Sprite::world(white, Vec2::new(1.0, 3.0))
                        .with_anchor(Vec2::new(0.5, 0.0))
                        .with_color(Vec4::new(0.16, 0.075, 0.03, 1.0)),
                );
            }

            let coins: Vec<(u32, bool)> = (0..12)
                .map(|i| {
                    let x = -25.0 + i as f32 * 5.0;
                    let y = GROUND_TOP + 1.2 + (i % 3) as f32 * 0.9;
                    let e = world_sprite(scene.world, Vec3::new(x, y, 0.5), Sprite::world(coin, Vec2::splat(0.8)).with_layer(1));
                    (e, false)
                })
                .collect();

            let player = world_sprite(
                scene.world,
                Vec3::new(0.0, GROUND_TOP, 1.0),
                Sprite::world(hero, Vec2::splat(1.6))
                    .with_anchor(Vec2::new(0.5, 1.0))
                    .with_frame(0, 4, 1)
                    .with_layer(2),
            );
            // 4 karelik şerit, saniyede 10 kare. Hangi aralığın oynayacağını `update` seçiyor.
            scene.world.add_component(scene.world.get_entity(player).expect("az önce doğdu"), SpriteAnimation::new(4, 1, 0, 3, 10.0));

            // ── HUD ──────────────────────────────────────────────────────────────────────
            let hud = |world: &mut World, sprite: Sprite| world.spawn_bundle((sprite,)).id();
            hud(
                scene.world,
                Sprite::screen(white, Vec2::new(BAR_WIDTH + 6.0, 26.0), Vec2::new(16.0, 16.0))
                    .with_color(Vec4::new(0.05, 0.05, 0.08, 0.85)),
            );
            let bar_fill = hud(
                scene.world,
                Sprite::screen(white, Vec2::new(BAR_WIDTH, 20.0), Vec2::new(19.0, 19.0))
                    .with_color(BAR_FULL)
                    .with_layer(1),
            );
            let coin_icons = (0..coins.len())
                .map(|i| {
                    hud(
                        scene.world,
                        Sprite::screen(coin, Vec2::splat(24.0), Vec2::new(16.0 + i as f32 * 28.0, 52.0))
                            .with_color(Vec4::new(0.25, 0.25, 0.3, 0.8)),
                    )
                })
                .collect();

            scene.spawn_camera(state, Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
            let mut camera = 0;
            if let Some(mut q) = scene.world.query_mut::<(Mut<Camera>, Mut<Transform>)>() {
                for (id, (mut cam, _)) in q.iter_mut() {
                    cam.projection = ProjectionMode::Orthographic { height: VIEW_HEIGHT };
                    camera = id;
                }
            }

            scene.world.insert_resource(Game {
                player,
                camera,
                hills: hills_list,
                coins,
                coin_icons,
                bar_fill,
                x: 0.0,
                y: GROUND_TOP,
                vy: 0.0,
                facing_left: false,
                walk_time: 0.0,
                energy: 1.0,
                time: 0.0,
            });
        })
        // Uçan kamera YOK: kamerayı oyuncu sürüyor. Bu yüzden `simple_scene_update` çağrılmıyor,
        // ama dördüncü işi — dönüşüm yayılımı — burada elle yapılıyor; yoksa `GlobalTransform`
        // hiç güncellenmez ve sprite'lar yerinden kıpırdamaz.
        .set_update(|world, _state, dt, input| {
            update(world, dt.min(0.05), input);
            gizmo::systems::transform::TransformSyncSystem.run(world, dt);
            gizmo::systems::transform::TransformPropagateSystem.run(world, dt);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

fn update(world: &mut World, dt: f32, input: &gizmo::core::input::Input) {
    let Some(mut g) = world.remove_resource::<Game>() else { return };
    g.time += dt;

    // Tuşları elle okumak yerine `move_axis`: gamepad'in sol çubuğunu da alıyor. Oklar ikinci
    // bir set; ikisi birden basılıyken hız ikiye katlanmasın diye büyük olan kazanıyor.
    let (wasd, _) = input.move_axis();
    let (arrows, _) = input.move_axis_with(gizmo::core::input::MoveKeys::ARROWS);
    let dir = if arrows.abs() > wasd.abs() { arrows } else { wasd };
    let on_ground = g.y <= GROUND_TOP + 1e-4;
    if on_ground && input.is_key_just_pressed(KeyCode::Space as u32) && g.energy > 0.15 {
        g.vy = 9.0;
        g.energy -= 0.15;
    }
    g.vy -= 25.0 * dt;
    g.y = (g.y + g.vy * dt).max(GROUND_TOP);
    if g.y <= GROUND_TOP {
        g.vy = 0.0;
    }
    g.x = (g.x + dir * 6.0 * dt).clamp(-58.0, 58.0);
    if dir != 0.0 {
        g.facing_left = dir < 0.0;
        g.walk_time += dt;
        g.energy = (g.energy - 0.08 * dt).max(0.0);
    } else {
        g.walk_time = 0.0;
        g.energy = (g.energy + 0.12 * dt).min(1.0);
    }

    // Coin toplama: elle bir mesafe testi — motorda henüz 2D çarpışma yok.
    let mut collected = Vec::new();
    for (i, (e, taken)) in g.coins.iter_mut().enumerate() {
        if *taken {
            continue;
        }
        let pos = world.borrow::<Transform>().get(*e).map(|t| t.position);
        if let Some(p) = pos {
            if (p.x - g.x).abs() < 0.8 && (p.y - (g.y + 0.8)).abs() < 1.0 {
                *taken = true;
                collected.push((*e, i));
            }
        }
    }
    for (e, i) in collected {
        if let Some(entity) = world.get_entity(e) {
            world.add_component(entity, IsHidden);
        }
        if let Some(mut s) = world.borrow_mut::<Sprite>().get_mut(g.coin_icons[i]) {
            s.color = Vec4::ONE;
        }
    }

    {
        let mut tf = world.borrow_mut::<Transform>();
        if let Some(mut t) = tf.get_mut(g.player) {
            t.position.x = g.x;
            t.position.y = g.y;
        }
        // Kamera yatayda oyuncuyu izliyor.
        if let Some(mut t) = tf.get_mut(g.camera) {
            t.position.x = g.x;
        }
        // Paralaks: oran 1 kamerayla birlikte gider (gökyüzü), 0 dünyaya çakılı kalır.
        for &(e, factor, copy) in &g.hills {
            let base = g.x * factor;
            let shift = ((g.x - base) / HILL_WIDTH).round() * HILL_WIDTH;
            if let Some(mut t) = tf.get_mut(e) {
                t.position.x = base + shift + copy * HILL_WIDTH;
            }
        }
        // Coin'ler x ölçeğiyle dönüyor; ölçek negatife geçince sprite arkasını gösteriyor.
        for &(e, _) in &g.coins {
            if let Some(mut t) = tf.get_mut(e) {
                t.scale.x = (g.time * 3.0 + t.position.x).cos();
            }
        }
    }

    // Durumdan bir aralık seçiliyor; kareyi motor ilerletiyor. Aynı aralığı her karede seçmek
    // döngüyü bozmaz — `set_run` yalnızca değişince başa sarar.
    if let Some(mut anim) = world.borrow_mut::<SpriteAnimation>().get_mut(g.player) {
        let (first, last) = if !on_ground { (3, 3) } else if g.walk_time > 0.0 { (0, 3) } else { (0, 0) };
        anim.set_run(first, last);
    }

    {
        let mut sprites = world.borrow_mut::<Sprite>();
        if let Some(mut s) = sprites.get_mut(g.player) {
            s.flip_x = g.facing_left;
        }
        if let Some(mut s) = sprites.get_mut(g.bar_fill) {
            s.size.x = (BAR_WIDTH * g.energy).max(0.0);
            let low = g.energy < 0.3;
            s.color = if low { BAR_LOW } else { BAR_FULL };
        }
    }

    world.insert_resource(g);
}

// ── Prosedürel görüntüler ───────────────────────────────────────────────────────────────
// Hepsi `(rgba, genişlik, yükseklik)`, üst satır önce.

fn image(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> (Vec<u8>, u32, u32) {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&f(x, y));
        }
    }
    (px, w, h)
}

fn sky_gradient() -> (Vec<u8>, u32, u32) {
    image(1, 32, |_, y| {
        let t = y as f32 / 31.0;
        let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
        [lerp(60.0, 250.0), lerp(110.0, 200.0), lerp(200.0, 170.0), 255]
    })
}

/// Üstü dalgalı bir siluet; `height` dolu kısmın en yüksek oranı.
fn hills(w: u32, h: u32, height: f32, rgb: [u8; 3]) -> (Vec<u8>, u32, u32) {
    image(w, h, |x, y| {
        let u = x as f32 / w as f32 * std::f32::consts::TAU;
        // Tam sayı frekanslar: şeridin iki ucu aynı yükseklikte biter, kopyalar dikişsiz birleşir.
        let crest = 0.5 + 0.25 * (u * 2.0).sin() + 0.15 * (u * 5.0 + 1.0).sin();
        let top = h as f32 * (1.0 - height * crest.clamp(0.1, 1.0));
        if (y as f32) >= top { [rgb[0], rgb[1], rgb[2], 255] } else { [0, 0, 0, 0] }
    })
}

fn grass_tile() -> (Vec<u8>, u32, u32) {
    image(16, 16, |x, y| {
        let blade = (x * 7 + 3) % 5 == 0;
        match y {
            0 if blade => [120, 210, 90, 255],
            0..=3 => [80, 170, 70, 255],
            4 if x % 3 == 0 => [80, 170, 70, 255],
            _ => {
                let speck = (x * 13 + y * 7) % 11 == 0;
                if speck { [110, 78, 50, 255] } else { [140, 98, 62, 255] }
            }
        }
    })
}

fn coin_image() -> (Vec<u8>, u32, u32) {
    image(12, 12, |x, y| {
        let (dx, dy) = (x as f32 - 5.5, y as f32 - 5.5);
        let d = (dx * dx + dy * dy).sqrt();
        if d > 5.6 {
            [0, 0, 0, 0]
        } else if d > 4.4 {
            [190, 130, 20, 255]
        } else if dx < -1.0 && dy < -1.0 {
            [255, 245, 170, 255]
        } else {
            [250, 200, 40, 255]
        }
    })
}

/// 4 kare × 16 piksel: yürüyen bir yaratık. Kare 3 aynı zamanda zıplama pozu.
fn hero_sheet() -> (Vec<u8>, u32, u32) {
    image(64, 16, |x, y| {
        let frame = x / 16;
        let (lx, ly) = ((x % 16) as i32, y as i32);
        // Bacaklar: her kare farklı açıklıkta.
        let stride = [0, 2, 0, -2][frame as usize];
        let leg = ly >= 13 && ((lx - (5 - stride)).abs() <= 1 || (lx - (10 + stride)).abs() <= 1);
        if leg {
            return [60, 40, 90, 255];
        }
        let bob = if frame % 2 == 1 { 1 } else { 0 };
        let (dx, dy) = (lx as f32 - 7.5, (ly - bob) as f32 - 7.5);
        let body = (dx * dx) / 36.0 + (dy * dy) / 30.0 <= 1.0;
        if !body {
            return [0, 0, 0, 0];
        }
        // Yüz sağa bakıyor; `flip_x` sola çeviriyor.
        let eye = (lx == 10 || lx == 12) && ly - bob == 6;
        if eye {
            [20, 20, 30, 255]
        } else if ly - bob <= 4 {
            [255, 170, 110, 255]
        } else {
            [240, 120, 70, 255]
        }
    })
}
