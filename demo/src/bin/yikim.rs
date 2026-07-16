//! # YIKIM USTASI — top/gülle ile fizik-yıkım oyunu (temiz sürüm)
//!
//! Karşındaki yapıların **altın hedef** bloklarını ağır gülleler fırlatarak platformdan
//! düşür. Sınırlı atış hakkın var, üç bölüm giderek zorlaşır.
//!
//! Bu sürüm motorun yüksek-seviye olanaklarını kullanır. NEYİN motora, NEYİN oyuna ait
//! olduğu konusunda dürüst olalım:
//!   * **`Prefab`** — YAPI (blok/hedef/platform) tek blueprint'ten; collider `Transform.scale`'den
//!     OTOMATİK türetilir (`auto_box_collider`, boyut bir kez). Gülle/konfeti Prefab DEĞİL:
//!     her örneğin kendi hızı olduğundan (Prefab hızı gömemez) doğrudan `spawn_bundle` — ama
//!     materyalleri setup'ta BİR KEZ kurulur (her atışta yeniden üretilmez).
//!   * **Hedefler = `Target` bileşeni** — her kare SORGUYLA taranır; kalıcı `Vec<TargetInfo>`
//!     senkronu YOK (eski sürümdeki elle liste gitti). Düşenler bir kare-içi tampona toplanıp
//!     despawn edilir (sorgu iterasyonunda mutasyon yapılamaz) — skorlama+konfeti despawn'a
//!     bağlı olduğundan bu adım kasıtlı olarak oyun kodunda, tam otomatik değil.
//!   * **`DespawnAfter` / `DespawnBelowY`** — gülle+konfetinin NORMAL ömür temizliği (motor).
//!   * **`Fx` işareti + `despawn_all_with::<Fx>()`** — AYRICA bölüm yenilemede uçan gülle/konfeti
//!     ANINDA silinir (ömür-temizliğini beklemeden). İki tamamlayıcı yol: ömür + toplu-sil.
//!   * **`is_key_just_*`** — kenar-tespiti motordan (elle prev_* takibi yok).
//!   * **`Camera::forward_from`** — nişan yönü paylaşılan yardımcıdan.
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — motor `with_scene_render()` tek-satır
//!     kısayolunu SUNAR ama onu BİLEREK kullanmıyoruz: o kısayol SSR/SSGI/volumetric/TAA'yı da
//!     kapatır; bu showcase o efektleri (altın hedeflerde yansıma, fireball volumetric parıltısı,
//!     TAA keskinliği) İSTER. `gpu_physics` zaten motor-varsayılanı `None` (opt-in) olduğundan
//!     render'da state-mutasyonu YOK — çıplak `default_render_pass` doğru ve dürüst kurulum.
//!
//! ## Kontroller
//!   * **Fare / W A S D / Ok tuşları** — nişan al
//!   * **SPACE** (basılı tut → bırak) — güç ölçerine göre fırlat · **Sol tık** — hızlı atış
//!   * **R** — bölümü tekrarla · **N** — (temizlenince) sonraki bölüm

use gizmo::egui;
use gizmo::prelude::*;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_3};

// ------------------------------------------------------------------ ayarlar
const CANNON: Vec3 = Vec3::new(0.0, 4.5, 0.0);
const STRUCT_Z: f32 = 26.0;
const KILL_Y: f32 = -2.0; // hedef bu yüksekliğin altına inince "düştü"
const KILL_DIST: f32 = 6.0; // ya da başlangıcından bu kadar uzaklaşınca

const BALL_R: f32 = 0.6;
const BALL_MASS: f32 = 9.0;
const MIN_SPEED: f32 = 22.0;
const MAX_SPEED: f32 = 45.0;
const CHARGE_TIME: f32 = 1.1;

const H: f32 = 0.5; // blok yarı-boyu (1×1×1 küp)
const BLOCK_MASS: f32 = 1.4;

const YAW_MIN: f32 = FRAC_PI_2 - 0.95;
const YAW_MAX: f32 = FRAC_PI_2 + 0.95;
const PITCH_MIN: f32 = -0.35;
const PITCH_MAX: f32 = 1.15;

const SHOTS: [i32; 3] = [5, 5, 6];
const LEVEL_NAMES: [&str; 3] = ["1 · DUVAR", "2 · İKİZ KULE", "3 · GÖKDELEN"];

// ---------------------------------------------------------- ECS bileşenleri
/// Altın hedef — `start`'tan KILL_DIST uzaklaşınca ya da KILL_Y altına inince düşmüş sayılır.
#[derive(Clone, Copy)]
struct Target {
    start: Vec3,
}
gizmo::core::impl_component!(Target);

/// Yapı varlığı (blok/hedef/platform) — bölüm yenilemede `despawn_all_with` ile topluca silinir.
#[derive(Clone, Copy)]
struct LevelItem;
gizmo::core::impl_component!(LevelItem);

/// Geçici efekt (gülle/konfeti) — DespawnAfter/BelowY ile kendiliğinden VE bölüm yenilemede silinir.
#[derive(Clone, Copy)]
struct Fx;
gizmo::core::impl_component!(Fx);

// --------------------------------------------------------------- durum
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Aiming,
    Cleared,
    Failed,
    Won,
}

struct Game {
    // varlık blueprint'leri + materyaller (bir kez kurulur)
    cube: Mesh,
    ball_mesh: Mesh,
    ball_mat: Material,        // gülle materyali — bir kez, her atışta değil
    confetti_mats: Vec<Material>, // konfeti palet materyalleri — bir kez
    stone: Prefab,
    gold: Prefab,
    platform: Prefab,
    // nişan
    yaw: f32,
    pitch: f32,
    // oynanış
    level: usize,
    score: u32,
    shots: i32,
    targets_left: usize,
    phase: Phase,
    charge: f32,
    charging: bool,
    fail_timer: f32,
    // muhtelif
    rng: u32,
}

impl Game {
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) & 0xFFFFFF) as f32 / 16_777_216.0
    }
    fn rand_range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }
}

fn aim_dir(yaw: f32, pitch: f32) -> Vec3 {
    Camera::forward_from(yaw, pitch)
}

// ------------------------------------------------------- yapı spawn'lama
fn block(g: &Game, world: &mut World, pos: Vec3, half: Vec3, color: Vec4, mass: f32) {
    let e = g
        .stone
        .clone()
        .with_pbr(color, 0.75, 0.05)
        .spawn_with_mass(world, Transform::new(pos).with_scale(half), mass);
    world.add_component(e, LevelItem);
}

fn target(g: &Game, world: &mut World, pos: Vec3) {
    let e = g
        .gold
        .spawn(world, Transform::new(pos).with_scale(Vec3::splat(H)));
    world.add_component(e, Target { start: pos });
    world.add_component(e, LevelItem);
    world.add_component(e, PointLight::new(Vec3::new(1.0, 0.72, 0.2), 6.0, 8.0)); // parıltı
}

// --------------------------------------------------------------- bölümler
fn load_level(g: &mut Game, world: &mut World, idx: usize) {
    world.despawn_all_with::<LevelItem>();
    world.despawn_all_with::<Fx>();

    // platform (statik, üstü y=0)
    let p = g.platform.spawn(
        world,
        Transform::new(Vec3::new(0.0, -0.6, STRUCT_Z)).with_scale(Vec3::new(12.0, 0.6, 10.0)),
    );
    world.add_component(p, LevelItem);

    let a = Vec4::new(0.62, 0.60, 0.58, 1.0);
    let b = Vec4::new(0.70, 0.45, 0.34, 1.0);
    let c = Vec4::new(0.48, 0.55, 0.62, 1.0);
    let hh = Vec3::splat(H);

    match idx {
        // ── Bölüm 1: DUVAR (6 geniş × 2 derin × 4 kat) ──
        0 => {
            for row in 0..4 {
                for col in 0..6 {
                    for dz in [-0.5_f32, 0.5] {
                        let color = if (row + col) % 2 == 0 { a } else { b };
                        block(g, world, Vec3::new(-2.5 + col as f32, H + row as f32, STRUCT_Z + dz), hh, color, BLOCK_MASS);
                    }
                }
            }
            for &x in &[-2.0_f32, 0.0, 2.0] {
                target(g, world, Vec3::new(x, 4.0 + H, STRUCT_Z));
            }
        }
        // ── Bölüm 2: İKİZ KULE (iki 2×2×4, kirişsiz) ──
        1 => {
            for &base_x in &[-3.5_f32, 3.5] {
                for row in 0..4 {
                    for dx in [-0.5_f32, 0.5] {
                        for dz in [-0.5_f32, 0.5] {
                            let color = if row % 2 == 0 { c } else { a };
                            block(g, world, Vec3::new(base_x + dx, H + row as f32, STRUCT_Z + dz), hh, color, BLOCK_MASS);
                        }
                    }
                }
            }
            target(g, world, Vec3::new(-3.5, 4.0 + H, STRUCT_Z));
            target(g, world, Vec3::new(3.5, 4.0 + H, STRUCT_Z));
            target(g, world, Vec3::new(0.0, H, STRUCT_Z));
        }
        // ── Bölüm 3: GÖKDELEN (3×3×8) + KISA BLOK (3×2×3) ──
        _ => {
            for row in 0..8 {
                for ix in -1..=1 {
                    for iz in -1..=1 {
                        let color = if (row + (ix + 1) as usize + (iz + 1) as usize).is_multiple_of(2) { c } else { a };
                        block(g, world, Vec3::new(-4.0 + ix as f32, H + row as f32, STRUCT_Z + iz as f32), hh, color, BLOCK_MASS);
                    }
                }
            }
            target(g, world, Vec3::new(-4.5, 8.0 + H, STRUCT_Z));
            target(g, world, Vec3::new(-3.5, 8.0 + H, STRUCT_Z));

            for row in 0..3 {
                for ix in -1..=1 {
                    for dz in [-0.5_f32, 0.5] {
                        let color = if (row + (ix + 1) as usize).is_multiple_of(2) { b } else { a };
                        block(g, world, Vec3::new(4.0 + ix as f32, H + row as f32, STRUCT_Z + dz), hh, color, BLOCK_MASS);
                    }
                }
            }
            target(g, world, Vec3::new(4.0, 3.0 + H, STRUCT_Z));
        }
    }

    g.level = idx;
    g.shots = SHOTS[idx];
    g.targets_left = count_targets(world);
    g.phase = Phase::Aiming;
    g.charge = 0.0;
    g.charging = false;
    g.fail_timer = 0.0;
    g.yaw = FRAC_PI_2;
    g.pitch = 0.08;
}

fn count_targets(world: &World) -> usize {
    world.query::<&Target>().map(|q| q.iter().count()).unwrap_or(0)
}

// --------------------------------------------------------------- ateş + efekt
fn fire(g: &mut Game, world: &mut World, power: f32) {
    let dir = aim_dir(g.yaw, g.pitch);
    let speed = MIN_SPEED + power.clamp(0.0, 1.0) * (MAX_SPEED - MIN_SPEED);
    // CCD yok: 45 m/s @ 240 Hz = 0.19 birim/adım ≪ 1 birim blok → substep tünellemeyi zaten önler.
    let e = world.spawn_bundle((
        Transform::new(CANNON + dir * 2.5),
        g.ball_mesh.clone(),
        g.ball_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::dynamic(BALL_MASS)
            .with_collider(Collider::sphere(BALL_R))
            .with_velocity(dir * speed)
            .with_friction(0.6)
            .with_restitution(0.15),
    ));
    world.add_component(e, PointLight::new(Vec3::new(1.0, 0.45, 0.1), 9.0, 14.0));
    world.add_component(e, DespawnAfter::secs(7.0));
    world.add_component(e, DespawnBelowY::new(-60.0));
    world.add_component(e, Fx);

    g.shots -= 1;
    if g.shots <= 0 {
        g.fail_timer = 4.0; // son güllenin oturması için süre tanı
    }
}

fn confetti(g: &mut Game, world: &mut World, at: Vec3) {
    let n = g.confetti_mats.len();
    for i in 0..7 {
        let vel = Vec3::new(g.rand_range(-4.0, 4.0), g.rand_range(4.0, 9.0), g.rand_range(-4.0, 4.0));
        let spin = Vec3::new(g.rand_range(-8.0, 8.0), g.rand_range(-8.0, 8.0), g.rand_range(-8.0, 8.0));
        let mat = g.confetti_mats[i % n].clone(); // önceden-kurulu palet materyali
        let e = world.spawn_bundle((
            Transform::new(at).with_scale(Vec3::splat(0.14)),
            g.cube.clone(),
            mat,
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(0.05)
                .with_collider(Collider::box_collider(Vec3::splat(0.14)))
                .with_velocity(vel)
                .with_angular_velocity(spin)
                .with_restitution(0.4),
        ));
        world.add_component(e, DespawnAfter::secs(6.0));
        world.add_component(e, DespawnBelowY::new(-60.0));
        world.add_component(e, Fx);
    }
}

// --------------------------------------------------------------- setup
fn setup(world: &mut World, renderer: &Renderer) -> Game {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let checker = assets.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);

    let cube = AssetManager::create_cube(&renderer.device);
    let ball_mesh = AssetManager::create_sphere(&renderer.device, BALL_R, 28, 28);
    let base_mat = Material::new(white.clone()).with_pbr(Vec4::ONE, 0.6, 0.0);

    // Blueprint'ler: collider Transform.scale'den otomatik (kutu); gülle/konfeti runtime spawn.
    let stone = Prefab::new(cube.clone(), base_mat.clone())
        .with_body(RigidBodyBundle::dynamic(BLOCK_MASS).with_friction(0.85).with_restitution(0.0).with_damping(0.06, 0.12))
        .auto_box_collider();
    let gold = Prefab::new(cube.clone(), base_mat.clone())
        .with_pbr(Vec4::new(1.0, 0.78, 0.16, 1.0), 0.22, 1.0)
        .with_body(RigidBodyBundle::dynamic(1.4).with_friction(0.85).with_restitution(0.0).with_damping(0.06, 0.12))
        .auto_box_collider();
    let platform = Prefab::new(cube.clone(), Material::new(checker).with_pbr(Vec4::new(0.34, 0.36, 0.40, 1.0), 0.9, 0.05))
        .with_body(RigidBodyBundle::static_body())
        .auto_box_collider();

    // Runtime materyaller BİR KEZ (her atış/parçacıkta yeniden üretme).
    let ball_mat = base_mat.clone().with_pbr(Vec4::new(0.18, 0.19, 0.22, 1.0), 0.3, 1.0);
    let confetti_mats: Vec<Material> = [
        Vec4::new(1.0, 0.85, 0.2, 1.0),
        Vec4::new(1.0, 0.35, 0.25, 1.0),
        Vec4::new(0.3, 0.9, 0.5, 1.0),
        Vec4::new(0.35, 0.7, 1.0, 1.0),
        Vec4::new(1.0, 0.5, 0.9, 1.0),
    ]
    .iter()
    .map(|&c| base_mat.clone().with_unlit(c))
    .collect();

    // Gökyüzü kubbesi (ters küp, unlit). scale·√3 ≤ far(4000) olmalı → 2000 (köşe 3464<4000).
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)),
        AssetManager::create_inverted_cube(&renderer.device),
        Material::new(white.clone()).with_unlit(Vec4::new(0.45, 0.64, 0.86, 1.0)),
        MeshRenderer::new(),
    ));
    // Uzak vadi zemini (devrilen bloklar buraya düşer)
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -55.0, STRUCT_Z)).with_scale(Vec3::new(600.0, 1.0, 600.0)),
        cube.clone(),
        Material::new(white).with_pbr(Vec4::new(0.26, 0.30, 0.30, 1.0), 1.0, 0.0),
        MeshRenderer::new(),
        RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::new(600.0, 1.0, 600.0))),
    ));
    // Güneş + dolgu
    world.spawn_bundle((
        Transform::new(Vec3::new(14.0, 40.0, 10.0)).with_rotation(Quat::from_rotation_x(-0.95)),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 3.4, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-2.2)),
        DirectionalLight::new(Vec3::new(0.5, 0.6, 0.8), 0.8, LightRole::Sun),
    ));
    // Kamera (sabit topçu)
    world.spawn_bundle((
        Transform::new(CANNON),
        Camera::new(FRAC_PI_3, 0.1, 4000.0, FRAC_PI_2, 0.08, true),
    ));

    let mut g = Game {
        cube,
        ball_mesh,
        ball_mat,
        confetti_mats,
        stone,
        gold,
        platform,
        yaw: FRAC_PI_2,
        pitch: 0.08,
        level: 0,
        score: 0,
        shots: 0,
        targets_left: 0,
        phase: Phase::Aiming,
        charge: 0.0,
        charging: false,
        fail_timer: 0.0,
        rng: 0x1234_5678,
    };
    load_level(&mut g, world, 0);
    println!("🎯 YIKIM USTASI — altın hedefleri düşür! (SPACE=ateş, Fare/WASD=nişan)");
    g
}

// --------------------------------------------------------------- update
fn update(world: &mut World, g: &mut Game, dt: f32, input: &Input) {
    let dt = dt.min(0.05);
    let k = |c: KeyCode| input.is_key_pressed(c as u32);

    // --- nişan (fare + klavye; "sağ" = dünya -X → fare-sağ/D = yaw+) ---
    // Fare-look YALNIZ sağ-tık basılıyken (pointer-lock yok → serbest fare aim'i
    // stray-delta ile kaydırıyordu = "durmadan yükseliyorum"). Sağ-tık = fareyle nişan.
    if input.is_mouse_button_pressed(1) {
        let md = input.mouse_delta();
        g.yaw += md.0 * 0.0042;
        g.pitch -= md.1 * 0.0042;
    }
    let s = 1.3 * dt;
    if k(KeyCode::KeyA) || k(KeyCode::ArrowLeft) { g.yaw -= s; }
    if k(KeyCode::KeyD) || k(KeyCode::ArrowRight) { g.yaw += s; }
    if k(KeyCode::KeyW) || k(KeyCode::ArrowUp) { g.pitch += s; }
    if k(KeyCode::KeyS) || k(KeyCode::ArrowDown) { g.pitch -= s; }
    g.yaw = g.yaw.clamp(YAW_MIN, YAW_MAX);
    g.pitch = g.pitch.clamp(PITCH_MIN, PITCH_MAX);

    // --- ateş / güç ölçeri (kenar-tespiti motorun is_key_just_* API'sinden) ---
    let mut shot: Option<f32> = None;
    if g.phase == Phase::Aiming && g.shots > 0 {
        if k(KeyCode::Space) {
            g.charging = true;
            g.charge = (g.charge + dt / CHARGE_TIME).min(1.0);
        }
        // just_released ayrı if (tek-kare fast-tap'ta is_key_pressed VE just_released aynı kare true).
        if g.charging && input.is_key_just_released(KeyCode::Space as u32) {
            shot = Some(g.charge.max(0.12));
            g.charging = false;
            g.charge = 0.0;
        }
        if input.is_mouse_button_just_pressed(0) {
            shot = Some(0.7);
            g.charging = false;
            g.charge = 0.0;
        }
    }
    if g.phase != Phase::Aiming {
        g.charging = false;
        g.charge = 0.0;
    }
    if let Some(power) = shot {
        fire(g, world, power);
    }

    // --- bölüm tekrar / sonraki ---
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        let lvl = if g.phase == Phase::Won { 0 } else { g.level };
        if g.phase == Phase::Won { g.score = 0; }
        load_level(g, world, lvl);
    } else if input.is_key_just_pressed(KeyCode::KeyN as u32) {
        match g.phase {
            Phase::Cleared if g.level + 1 < 3 => load_level(g, world, g.level + 1),
            Phase::Cleared => g.phase = Phase::Won,
            Phase::Won => { g.score = 0; load_level(g, world, 0); }
            _ => {}
        }
    }

    // --- hedef yıkım kontrolü + kalan sayımı (TEK ECS sorgusu) ---
    let mut killed: Vec<(u32, Vec3)> = Vec::new();
    let mut alive = 0usize;
    if let Some(q) = world.query::<(&Target, &Transform)>() {
        for (id, (tgt, tr)) in q.iter() {
            let p = tr.position;
            if p.y < KILL_Y || (p - tgt.start).length() > KILL_DIST {
                killed.push((id, if p.y < KILL_Y { tgt.start } else { p }));
            } else {
                alive += 1;
            }
        }
    }
    for (id, pos) in killed {
        world.despawn_by_id(id);
        g.score += 100;
        confetti(g, world, pos);
    }
    g.targets_left = alive;

    // --- kazanma / kaybetme ---
    if g.phase == Phase::Aiming {
        if g.targets_left == 0 {
            g.phase = Phase::Cleared;
            g.score += (g.shots.max(0) as u32) * 25; // kalan gülle bonusu
        } else if g.shots <= 0 {
            g.fail_timer -= dt;
            if g.fail_timer <= 0.0 {
                g.phase = Phase::Failed;
            }
        }
    }

    // --- kamerayı nişana göre güncelle ---
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut tr, mut cam)) in q.iter_mut() {
            tr.position = CANNON;
            tr.rotation = Quat::from_rotation_y(-g.yaw);
            cam.yaw = g.yaw;
            cam.pitch = g.pitch;
        }
    }
}

// --------------------------------------------------------------- HUD
fn ui(_world: &mut World, g: &mut Game, ctx: &egui::Context) {
    use egui::{Align2, Color32, RichText};

    // FPS HUD-only veri → egui'nin kendi yumuşatılmış frame süresinden (Game state'e karışmaz).
    let fps = 1.0 / ctx.input(|i| i.stable_dt).max(1e-4);

    egui::Window::new("bilgi")
        .title_bar(false)
        .resizable(false)
        .anchor(Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            ui.label(RichText::new(format!("YIKIM USTASI — BÖLÜM {}", LEVEL_NAMES[g.level.min(2)]))
                .strong().size(20.0).color(Color32::from_rgb(255, 210, 90)));
            ui.separator();
            ui.label(RichText::new(format!("SKOR: {}", g.score)).size(18.0).color(Color32::WHITE));
            ui.label(RichText::new(format!("🎯 Kalan hedef: {}", g.targets_left)).size(16.0).color(Color32::from_rgb(255, 220, 120)));
            let ammo_col = if g.shots <= 1 { Color32::from_rgb(255, 120, 100) } else { Color32::from_rgb(150, 220, 255) };
            ui.label(RichText::new(format!("💣 Gülle: {}", g.shots.max(0))).size(16.0).color(ammo_col));
            ui.separator();
            ui.label(RichText::new(format!("FPS: {:.0}", fps)).color(Color32::from_rgb(160, 255, 160)));
            ui.small("Nişan: WASD/Oklar  ya da  SAĞ-tık+fare  •  SPACE: güçlü atış  •  Sol tık: hızlı  •  R: tekrar");
        });

    if g.charging && g.charge > 0.0 {
        egui::Window::new("guc")
            .title_bar(false).resizable(false)
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(210)))
            .show(ctx, |ui| {
                ui.label(RichText::new("GÜÇ").strong().color(Color32::from_rgb(255, 200, 80)));
                ui.add(egui::ProgressBar::new(g.charge).desired_width(260.0).text(format!("{:.0}%", g.charge * 100.0)));
            });
    }

    // crosshair
    let c = ctx.content_rect().center();
    egui::Area::new(egui::Id::new("crosshair"))
        .fixed_pos(egui::pos2(c.x - 8.0, c.y - 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("+").size(28.0).strong().color(Color32::WHITE));
        });

    let banner = match g.phase {
        Phase::Cleared => Some(("BÖLÜM TEMİZLENDİ!  ▶  N = sonraki bölüm", Color32::from_rgb(120, 255, 140))),
        Phase::Failed => Some(("GÜLLELER BİTTİ!  ↺  R = tekrar dene", Color32::from_rgb(255, 120, 110))),
        Phase::Won => Some(("🏆 TÜM BÖLÜMLER BİTTİ — USTA OLDUN!  R = baştan", Color32::from_rgb(255, 220, 90))),
        Phase::Aiming => None,
    };
    if let Some((text, col)) = banner {
        egui::Window::new("banner")
            .title_bar(false).resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(225)))
            .show(ctx, |ui| {
                ui.label(RichText::new(text).size(26.0).strong().color(col));
                ui.label(RichText::new(format!("Toplam skor: {}", g.score)).size(18.0).color(Color32::WHITE));
            });
    }
}

// --------------------------------------------------------------- render + main
// Render'ı motorun TAM deferred boru hattına (`default_render_pass`) devrediyoruz.
// Bilinçli seçim: motorun `with_scene_render()` tek-satır kısayolu VAR ama onu KULLANMIYORUZ —
// o kısayol SSR/SSGI/volumetric/TAA'yı da kapatır; bu showcase bu efektleri AÇIK ister
// (deferred=Some varsayılanı zaten hepsini aktif eder). `gpu_physics` motor-varsayılanı `None`
// (opt-in; yikim `enable_gpu_physics` çağırmaz) olduğundan burada state-mutasyonu GEREKMEZ.
fn render(
    world: &mut World,
    _g: &Game,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _t: f32,
) {
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<Game>::new("Gizmo — YIKIM USTASI", 1360, 768)
        .add_plugin(PhysicsPlugin::new())
        .set_setup(setup)
        .set_update(update)
        .set_ui(ui)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
