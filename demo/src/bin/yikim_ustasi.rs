//! # YIKIM USTASI — Gizmo motorunun gücünü sergileyen fizik-yıkım oyunu
//!
//! Birinci-şahıs top/gülle oyunu: karşındaki kule/kale yapılarının **altın hedef**
//! bloklarını AĞIR gülleler fırlatarak platformdan düşür. Sınırlı atış hakkın var,
//! üç bölüm giderek zorlaşır.
//!
//! Motorun GERÇEKTEN sağlam yanlarını sergiler:
//!   * **Rijit dinamik + çarpışma** — gülle bloğa çarpar, momentum aktarılır,
//!     yapı gerçekçi devrilir; yüzlerce blok dağılıp yerleşir.
//!   * **Runtime spawn/despawn** — gülleler ve hedef patlayınca fışkıran konfeti
//!     her kare ECS'e eklenir; fizik dünyası otomatik senkronize olur.
//!   * **Dinamik ışıklandırma** — her güllede turuncu ışık, her hedefte altın
//!     ışık; devrilen hedefler ışığı da taşır (hareketli ışıklar).
//!   * **Deferred PBR + gölge** ve egui HUD (skor / güç ölçeri / banner).
//!
//! ## DÜRÜSTLÜK NOTU
//!   1. **Solver dinlenen-istif kararsızlığı — ÇÖZÜLDÜ (2026-07-15).** Eskiden TGS çözücü
//!      dinlenen istife enerji pompalıyordu (yanal buckling; uyanık N=16 kule ~13.5s'de
//!      patlıyordu) ve bu demo bloklarını UYKUDA spawn'layarak (`spawn_asleep` hilesi) solver'ı
//!      atlatıyordu. Manifold BLOCK solver (bkz. gizmo-physics-rigid solver/block.rs) bunu
//!      düzeltti → bloklar artık UYANIK spawn'lanıyor, kuleyi GERÇEK solver dik tutuyor. (Aşırı
//!      32+ kat tek-sütun kuleler hâlâ ayrı bir iş, ama bu demonun yapıları rahatça kararlı.)
//!   2. **Gökyüzü GERÇEK environment-pass DEĞİL.** unlit ters küp + elle scale
//!      (2000); köşeleri far-plane'i aşmasın diye elle ayarlı. Motorda gerçek
//!      `Material::with_skybox()` yolu var (kcc_scene kullanır) — burada henüz
//!      ona geçilmedi. Yani "skybox" abartı; şu an düz-renk kubbe.
//!   * NOT (CCD): önceki turda "CCD gülleyi donduruyor" sandım — 6 izole repro
//!     bunun YANLIŞ olduğunu gösterdi (CCD dondurmaz). Bu hızlarda (≤45 m/s,
//!     240Hz alt-adım) tünelleme zaten substep ile önlenir → CCD gereksiz, o
//!     yüzden kapalı; "hızlı gülle CCD ile tünellemez" reklamı burada geçerli
//!     değil (özellik motorda çalışıyor ama bu demoda kullanılmıyor).
//!
//! ## Kontroller
//!   * **Fare / Ok tuşları / W A S D** — nişan al (yaw + pitch)
//!   * **SPACE (basılı tut → bırak)** — gülleyi güç ölçerine göre fırlat
//!   * **Sol tık** — hızlı atış (%70 güç)
//!   * **R** — bölümü yeniden başlat · **N** — (temizlenince) sonraki bölüm

use gizmo::egui;
use gizmo::prelude::*;
use std::f32::consts::FRAC_PI_2;

// ------------------------------------------------------------------ sabitler
const CANNON_POS: Vec3 = Vec3::new(0.0, 4.5, 0.0);
const STRUCT_Z: f32 = 26.0; // yapıların merkez z'si (topçudan ileride)
const PLATFORM_TOP: f32 = 0.0; // istifler bu düzlemde durur
const KILL_Y: f32 = -2.0; // hedef bu yüksekliğin altına inince "düştü"
const KILL_DIST: f32 = 6.0; // ya da başlangıcından bu kadar uzaklaşınca

const BALL_R: f32 = 0.6;
const BALL_MASS: f32 = 9.0;
const MIN_SPEED: f32 = 22.0;
const MAX_SPEED: f32 = 45.0;
const CHARGE_TIME: f32 = 1.1; // 0→tam güç saniye

const BLOCK_H: f32 = 0.5; // blok yarı-genişliği (1×1×1 küp)
const BLOCK_MASS: f32 = 1.4;

const YAW_MIN: f32 = FRAC_PI_2 - 0.95;
const YAW_MAX: f32 = FRAC_PI_2 + 0.95;
const PITCH_MIN: f32 = -0.35;
const PITCH_MAX: f32 = 1.15;

// ------------------------------------------------------------------- durum
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Aiming,
    Cleared,
    Failed,
    AllCleared,
}

struct TargetInfo {
    entity: Entity,
    start: Vec3,
    alive: bool,
}

struct Game {
    // varlıklar
    cube: Mesh,
    ball_mesh: Mesh,
    base_mat: Material,    // beyaz doku üzerine tint'lenecek şablon
    ground_mat: Material,  // dama tahtası platform
    // nişan
    yaw: f32,
    pitch: f32,
    // oynanış
    level: usize,
    score: u32,
    shots_left: i32,
    phase: Phase,
    charge: f32,
    charging: bool,
    fail_timer: Option<f32>,
    // izlenen varlıklar
    targets: Vec<TargetInfo>,
    targets_alive: usize,
    level_entities: Vec<Entity>,
    // Geçici varlıklar (gülle+konfeti) SADECE bölüm yenilenince topluca silinsin diye tutulur;
    // "7 sn sonra sil" / "y<-60 düşünce sil" temizliğini artık DespawnAfter/DespawnBelowY
    // komponentleri + LifetimeSystem otomatik yapıyor (elle döngü YOK).
    transient: Vec<Entity>,
    // muhtelif
    time: f32,
    fps: f32,
    rng: u32,
    // gösteri modu (attract): oyun kendi kendine oynar (YIKIM_AUTOPLAY=1)
    autoplay: bool,
    auto_timer: f32,
}

impl Game {
    /// Basit LCG — konfeti/serpinti çeşitliliği için (harici bağımlılık yok).
    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.rng >> 8) & 0xFFFFFF) as f32 / 16_777_216.0
    }
    fn rand_range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }

    fn tint(&self, color: Vec4, rough: f32, metal: f32) -> Material {
        self.base_mat.clone().with_pbr(color, rough, metal)
    }
}

// -------------------------------------------------------------- blok kurulumu
fn spawn_block(g: &mut Game, world: &mut World, pos: Vec3, half: Vec3, color: Vec4, mass: f32) {
    let mat = g.tint(color, 0.75, 0.05);
    let e = world
        .spawn_bundle((
            Transform::new(pos).with_scale(half),
            g.cube.clone(),
            mat,
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(mass)
                .with_collider(Collider::box_collider(half))
                .with_friction(0.85)
                .with_restitution(0.0)
                .with_damping(0.06, 0.12),
        ));
    g.level_entities.push(e);
}

fn spawn_static(g: &mut Game, world: &mut World, pos: Vec3, half: Vec3, mat: Material) {
    let e = world
        .spawn_bundle((
            Transform::new(pos).with_scale(half),
            g.cube.clone(),
            mat,
            MeshRenderer::new(),
            RigidBodyBundle::static_body().with_collider(Collider::box_collider(half)),
        ));
    g.level_entities.push(e);
}

fn spawn_target(g: &mut Game, world: &mut World, pos: Vec3) {
    let gold = g.tint(Vec4::new(1.0, 0.78, 0.16, 1.0), 0.22, 1.0);
    let e = world
        .spawn_bundle((
            Transform::new(pos).with_scale(Vec3::splat(BLOCK_H)),
            g.cube.clone(),
            gold,
            MeshRenderer::new(),
            RigidBodyBundle::dynamic(1.0)
                .with_collider(Collider::box_collider(Vec3::splat(BLOCK_H)))
                .with_friction(0.7)
                .with_restitution(0.15)
                .with_damping(0.06, 0.12),
        ));
    // hedefi ışıldat — devrilince ışığı da taşır (hareketli dinamik ışık)
    world.add_component(e, PointLight::new(Vec3::new(1.0, 0.72, 0.2), 6.0, 8.0));
    g.level_entities.push(e);
    g.targets.push(TargetInfo {
        entity: e,
        start: pos,
        alive: true,
    });
}

// --------------------------------------------------------------- bölümler
fn load_level(g: &mut Game, world: &mut World, idx: usize) {
    // eski bölüm + geçici varlıkları temizle
    for e in g.level_entities.drain(..) {
        world.despawn(e);
    }
    // Geçici varlıkları topluca temizle; bazıları LifetimeSystem tarafından çoktan
    // silinmiş olabilir → is_alive ile koru (bayat entity'ye dokunma).
    for e in g.transient.drain(..) {
        if world.is_alive(e) {
            world.despawn(e);
        }
    }
    g.targets.clear();

    // platform (statik, üstü PLATFORM_TOP=0)
    spawn_static(
        g,
        world,
        Vec3::new(0.0, PLATFORM_TOP - 0.6, STRUCT_Z),
        Vec3::new(12.0, 0.6, 10.0),
        g.ground_mat.clone(),
    );

    let stone_a = Vec4::new(0.62, 0.60, 0.58, 1.0);
    let stone_b = Vec4::new(0.70, 0.45, 0.34, 1.0);
    let stone_c = Vec4::new(0.48, 0.55, 0.62, 1.0);

    let shots = match idx {
        0 => 5,
        1 => 6,
        _ => 8,
    };
    g.shots_left = shots;

    match idx {
        // ---- Bölüm 1: DUVAR ----
        0 => {
            for row in 0..5 {
                for col in 0..6 {
                    let x = -2.5 + col as f32;
                    let y = PLATFORM_TOP + BLOCK_H + row as f32;
                    let c = if (row + col) % 2 == 0 { stone_a } else { stone_b };
                    spawn_block(
                        g,
                        world,
                        Vec3::new(x, y, STRUCT_Z),
                        Vec3::splat(BLOCK_H),
                        c,
                        BLOCK_MASS,
                    );
                }
            }
            for &x in &[-2.0_f32, 0.0, 2.0] {
                spawn_target(g, world, Vec3::new(x, PLATFORM_TOP + 5.0 + BLOCK_H, STRUCT_Z));
            }
        }
        // ---- Bölüm 2: İKİZ SÜTUNLAR + KİRİŞ ----
        1 => {
            for &base_x in &[-3.5_f32, 3.5] {
                for row in 0..6 {
                    for dx in [-0.5_f32, 0.5] {
                        let y = PLATFORM_TOP + BLOCK_H + row as f32;
                        let c = if row % 2 == 0 { stone_c } else { stone_a };
                        spawn_block(
                            g,
                            world,
                            Vec3::new(base_x + dx, y, STRUCT_Z),
                            Vec3::splat(BLOCK_H),
                            c,
                            BLOCK_MASS,
                        );
                    }
                }
            }
            // kiriş — iki sütunun tepesine uzanır
            let lintel_y = PLATFORM_TOP + 6.0 + 0.4;
            spawn_block(
                g,
                world,
                Vec3::new(0.0, lintel_y, STRUCT_Z),
                Vec3::new(4.2, 0.4, 0.6),
                stone_b,
                3.0,
            );
            spawn_target(g, world, Vec3::new(0.0, lintel_y + 0.9, STRUCT_Z));
            spawn_target(g, world, Vec3::new(-3.5, PLATFORM_TOP + 6.0 + BLOCK_H, STRUCT_Z));
            spawn_target(g, world, Vec3::new(3.5, PLATFORM_TOP + 6.0 + BLOCK_H, STRUCT_Z));
        }
        // ---- Bölüm 3: GÖKDELEN + PİRAMİT ----
        _ => {
            // ince yüksek kule (2×1 taban, 12 kat) — TGS istif kararlılığı
            for row in 0..12 {
                for dx in [-0.5_f32, 0.5] {
                    let y = PLATFORM_TOP + BLOCK_H + row as f32;
                    let c = if row % 2 == 0 { stone_c } else { stone_a };
                    spawn_block(
                        g,
                        world,
                        Vec3::new(-4.0 + dx, y, STRUCT_Z),
                        Vec3::splat(BLOCK_H),
                        c,
                        BLOCK_MASS,
                    );
                }
            }
            spawn_target(g, world, Vec3::new(-4.5, PLATFORM_TOP + 12.0 + BLOCK_H, STRUCT_Z));
            spawn_target(g, world, Vec3::new(-3.5, PLATFORM_TOP + 12.0 + BLOCK_H, STRUCT_Z));

            // piramit (5-4-3-2-1)
            for row in 0..5 {
                let count = 5 - row;
                let start = 4.0 - (count as f32 - 1.0) * 0.5;
                for col in 0..count {
                    let x = start + col as f32;
                    let y = PLATFORM_TOP + BLOCK_H + row as f32;
                    let c = if (row + col) % 2 == 0 { stone_b } else { stone_a };
                    spawn_block(
                        g,
                        world,
                        Vec3::new(x, y, STRUCT_Z),
                        Vec3::splat(BLOCK_H),
                        c,
                        BLOCK_MASS,
                    );
                }
            }
            spawn_target(g, world, Vec3::new(4.0, PLATFORM_TOP + 5.0 + BLOCK_H, STRUCT_Z));
            spawn_target(g, world, Vec3::new(2.5, PLATFORM_TOP + 2.0 + BLOCK_H, STRUCT_Z));
            spawn_target(g, world, Vec3::new(5.5, PLATFORM_TOP + 2.0 + BLOCK_H, STRUCT_Z));
        }
    }

    g.targets_alive = g.targets.len();
    g.level = idx;
    g.phase = Phase::Aiming;
    g.charge = 0.0;
    g.charging = false;
    g.fail_timer = None;
    g.yaw = FRAC_PI_2;
    g.pitch = 0.08;
    // hata-ayıklama düğmesi: başlangıç pitch'ini zorla (gök doğrulaması için)
    if let Ok(v) = std::env::var("YIKIM_PITCH") {
        if let Ok(p) = v.parse::<f32>() {
            g.pitch = p;
        }
    }
}

// --------------------------------------------------------------- setup
fn setup(world: &mut World, renderer: &Renderer) -> Game {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let checker = assets.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let cube = AssetManager::create_cube(&renderer.device);
    let ball_mesh = AssetManager::create_sphere(&renderer.device, BALL_R, 28, 28);

    let base_mat = Material::new(white.clone()).with_pbr(Vec4::ONE, 0.6, 0.0);
    let ground_mat =
        Material::new(checker).with_pbr(Vec4::new(0.34, 0.36, 0.40, 1.0), 0.9, 0.05);

    // --- gökyüzü kubbesi (ters küp, unlit mavi) ---
    // ÖNEMLİ: küpün köşeleri (scale·√3) far-plane'i (4000) AŞMAMALI, yoksa köşeler
    // kırpılıp gökte siyah "yırtık" oluşur. scale 2000 → köşe 3464 < 4000 (güvenli).
    let sky = Material::new(white.clone()).with_unlit(Vec4::new(0.45, 0.64, 0.86, 1.0));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)),
        AssetManager::create_inverted_cube(&renderer.device),
        sky,
        MeshRenderer::new(),
    ));

    // --- uzak vadi zemini (aşağıda; devrilen bloklar buraya düşer) ---
    // Puslu gri-yeşil, kara bir "boşluk" gibi değil uzak arazi gibi okunsun.
    let pit = Material::new(white.clone()).with_pbr(Vec4::new(0.26, 0.30, 0.30, 1.0), 1.0, 0.0);
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, -55.0, STRUCT_Z)).with_scale(Vec3::new(600.0, 1.0, 600.0)),
        cube.clone(),
        pit,
        MeshRenderer::new(),
        RigidBodyBundle::static_body().with_collider(Collider::box_collider(Vec3::new(600.0, 1.0, 600.0))),
    ));

    // --- güneş + dolgu ışığı ---
    world.spawn_bundle((
        Transform::new(Vec3::new(14.0, 40.0, 10.0)).with_rotation(Quat::from_rotation_x(-0.95)),
        DirectionalLight::new(Vec3::new(1.0, 0.96, 0.88), 3.4, LightRole::Sun),
    ));
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-2.2)),
        DirectionalLight::new(Vec3::new(0.5, 0.6, 0.8), 0.8, LightRole::Sun),
    ));

    // --- kamera (birinci-şahıs topçu) ---
    world.spawn_bundle((
        Transform::new(CANNON_POS),
        Camera::new(FRAC_PI_3_LOCAL, 0.1, 4000.0, FRAC_PI_2, 0.08, true),
    ));

    let mut g = Game {
        cube,
        ball_mesh,
        base_mat,
        ground_mat,
        yaw: FRAC_PI_2,
        pitch: 0.08,
        level: 0,
        score: 0,
        shots_left: 0,
        phase: Phase::Aiming,
        charge: 0.0,
        charging: false,
        fail_timer: None,
        targets: Vec::new(),
        targets_alive: 0,
        level_entities: Vec::new(),
        transient: Vec::new(),
        time: 0.0,
        fps: 60.0,
        rng: 0x1234_5678,
        autoplay: std::env::var("YIKIM_AUTOPLAY").is_ok(),
        auto_timer: 1.2,
    };

    load_level(&mut g, world, 0);
    println!("🎯 YIKIM USTASI — altın hedefleri platformdan düşür! (SPACE=ateş, Fare=nişan)");
    g
}

const FRAC_PI_3_LOCAL: f32 = std::f32::consts::FRAC_PI_3;

// --------------------------------------------------------------- ateş
fn fire(g: &mut Game, world: &mut World, power: f32) {
    let speed = MIN_SPEED + power.clamp(0.0, 1.0) * (MAX_SPEED - MIN_SPEED);
    // Nişan yönü motorun paylaşılan yardımından (elle "front" matematiği YOK). Standart
    // bir FP oyunu tüm fare-look'u `FpsLook` komponentine bırakabilir; bu demo sabit-top +
    // autoplay + aim-clamp şeması yüzünden yaw/pitch'i kendi tutuyor.
    let dir = Camera::forward_from(g.yaw, g.pitch);
    let pos = CANNON_POS + dir * 2.5;
    let ball_mat = g.tint(Vec4::new(0.18, 0.19, 0.22, 1.0), 0.3, 1.0);
    let e = world
        .spawn_bundle((
            Transform::new(pos),
            g.ball_mesh.clone(),
            ball_mat,
            MeshRenderer::new(),
            // CCD YOK — çünkü bu hızlarda GEREKSİZ: 45 m/s @ 240Hz = 0.19 birim/adım ≪
            // 1 birim blok → tünelleme zaten olmaz (izole reproda CCD'siz de tünellemedi).
            // (Önceki yanlış gerekçem "CCD donduruyor"du; 6 repro bunu çürüttü — CCD çalışıyor,
            //  yalnızca burada gerekmediği için açmıyorum. >240 m/s istenirse .with_ccd() ekle.)
            RigidBodyBundle::dynamic(BALL_MASS)
                .with_collider(Collider::sphere(BALL_R))
                .with_velocity(dir * speed)
                .with_friction(0.6)
                .with_restitution(0.15),
        ));
    // güllenin sıcak izi
    world.add_component(e, PointLight::new(Vec3::new(1.0, 0.45, 0.1), 9.0, 14.0));
    // Otomatik temizlik: 7 sn sonra ya da uçuruma düşünce sil (motor halleder).
    world.add_component(e, DespawnAfter::secs(7.0));
    world.add_component(e, DespawnBelowY::new(-60.0));
    g.transient.push(e);
    g.shots_left -= 1;
    if g.shots_left <= 0 && g.fail_timer.is_none() {
        g.fail_timer = Some(4.0); // son güllenin oturması için süre tanı
    }
}

fn confetti_burst(g: &mut Game, world: &mut World, at: Vec3) {
    let palette = [
        Vec4::new(1.0, 0.85, 0.2, 1.0),
        Vec4::new(1.0, 0.35, 0.25, 1.0),
        Vec4::new(0.3, 0.9, 0.5, 1.0),
        Vec4::new(0.35, 0.7, 1.0, 1.0),
        Vec4::new(1.0, 0.5, 0.9, 1.0),
    ];
    for i in 0..7 {
        let color = palette[i % palette.len()];
        let mat = g.base_mat.clone().with_unlit(color);
        let vel = Vec3::new(
            g.rand_range(-4.0, 4.0),
            g.rand_range(4.0, 9.0),
            g.rand_range(-4.0, 4.0),
        );
        let spin = Vec3::new(
            g.rand_range(-8.0, 8.0),
            g.rand_range(-8.0, 8.0),
            g.rand_range(-8.0, 8.0),
        );
        let e = world
            .spawn_bundle((
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
        world.add_component(e, DespawnAfter::secs(7.0));
        world.add_component(e, DespawnBelowY::new(-60.0));
        g.transient.push(e);
    }
}

// --------------------------------------------------------------- update
fn update(world: &mut World, g: &mut Game, dt: f32, input: &Input) {
    let dt = dt.min(0.05);
    g.time += dt;
    g.fps = g.fps * 0.9 + (1.0 / dt.max(1e-4)) * 0.1;

    // --- nişan (fare + klavye) ---
    // Bu kamerada "sağ" = dünya -X (get_right=(-sin yaw,0,cos yaw)); yaw ARTINCA
    // bakış sağa döner. Bu yüzden: fare-sağ/D → yaw+=, fare-sol/A → yaw-=.
    let md = input.mouse_delta();
    g.yaw += md.0 * 0.0042; // fare sağ → sağa bak
    g.pitch -= md.1 * 0.0042; // fare yukarı → yukarı bak
    let k = |c: KeyCode| input.is_key_pressed(c as u32);
    let aim_spd = 1.3 * dt;
    if k(KeyCode::ArrowLeft) || k(KeyCode::KeyA) {
        g.yaw -= aim_spd;
    }
    if k(KeyCode::ArrowRight) || k(KeyCode::KeyD) {
        g.yaw += aim_spd;
    }
    if k(KeyCode::ArrowUp) || k(KeyCode::KeyW) {
        g.pitch += aim_spd;
    }
    if k(KeyCode::ArrowDown) || k(KeyCode::KeyS) {
        g.pitch -= aim_spd;
    }
    g.yaw = g.yaw.clamp(YAW_MIN, YAW_MAX);
    g.pitch = g.pitch.clamp(PITCH_MIN, PITCH_MAX);

    // --- ateş / güç ölçeri ---
    // Kenar-tespiti motorun `is_key_just_*` / `is_mouse_button_just_*` API'sinden gelir
    // (elle prev_* takibi GEREKMEZ).
    let mut shot: Option<f32> = None;
    if g.phase == Phase::Aiming && g.shots_left > 0 {
        if k(KeyCode::Space) {
            g.charging = true;
            g.charge = (g.charge + dt / CHARGE_TIME).min(1.0);
        } else if g.charging && input.is_key_just_released(KeyCode::Space as u32) {
            shot = Some(g.charge.max(0.12)); // bırakınca ateşle
            g.charging = false;
            g.charge = 0.0;
        }
        if input.is_mouse_button_just_pressed(0) {
            shot = Some(0.7); // hızlı atış
        }
    }

    if let Some(power) = shot {
        fire(g, world, power);
    }

    // --- R / N ---
    if input.is_key_just_pressed(KeyCode::KeyR as u32) {
        let lvl = if g.phase == Phase::AllCleared { 0 } else { g.level };
        if g.phase == Phase::AllCleared {
            g.score = 0;
        }
        load_level(g, world, lvl);
    } else if input.is_key_just_pressed(KeyCode::KeyN as u32) {
        match g.phase {
            Phase::Cleared => {
                let next = g.level + 1;
                if next >= 3 {
                    g.phase = Phase::AllCleared;
                } else {
                    load_level(g, world, next);
                }
            }
            Phase::AllCleared => {
                g.score = 0;
                load_level(g, world, 0);
            }
            _ => {}
        }
    }

    // --- gösteri modu (attract): oyun kendi kendine oynar ---
    if g.autoplay {
        g.auto_timer -= dt;
        match g.phase {
            Phase::Aiming => {
                if g.auto_timer <= 0.0 && g.shots_left > 0 {
                    let aim_h = match g.level {
                        0 => 5.5,
                        1 => 6.5,
                        _ => 8.5,
                    };
                    let target = Vec3::new(g.rand_range(-3.0, 3.0), aim_h, STRUCT_Z);
                    let dir = (target - CANNON_POS).normalize();
                    g.yaw = dir.z.atan2(dir.x).clamp(YAW_MIN, YAW_MAX);
                    g.pitch = (dir.y.asin() + 0.06).clamp(PITCH_MIN, PITCH_MAX);
                    fire(g, world, 1.0);
                    g.auto_timer = 1.5;
                }
            }
            Phase::Cleared => {
                if g.auto_timer <= 0.0 {
                    let next = g.level + 1;
                    if next >= 3 {
                        g.phase = Phase::AllCleared;
                        g.auto_timer = 3.0;
                    } else {
                        load_level(g, world, next);
                        g.auto_timer = 1.5;
                    }
                }
            }
            Phase::Failed => {
                if g.auto_timer <= 0.0 {
                    let l = g.level;
                    load_level(g, world, l);
                    g.auto_timer = 1.5;
                }
            }
            Phase::AllCleared => {
                if g.auto_timer <= 0.0 {
                    g.score = 0;
                    load_level(g, world, 0);
                    g.auto_timer = 1.5;
                }
            }
        }
    }

    // --- hedef yıkım kontrolü ---
    let mut destroyed: Vec<(usize, Vec3)> = Vec::new();
    {
        let ts = world.borrow::<Transform>();
        for (i, t) in g.targets.iter().enumerate() {
            if !t.alive {
                continue;
            }
            if let Some(tr) = ts.get(t.entity.id()) {
                let p = tr.position;
                if p.y < KILL_Y || (p - t.start).length() > KILL_DIST {
                    destroyed.push((i, p));
                }
            }
        }
    }
    for (i, pos) in destroyed {
        g.targets[i].alive = false;
        g.targets_alive = g.targets_alive.saturating_sub(1);
        g.score += 100;
        let pos = if pos.y < KILL_Y {
            g.targets[i].start
        } else {
            pos
        };
        confetti_burst(g, world, pos);
        world.despawn(g.targets[i].entity);
    }

    // --- kazanma / kaybetme ---
    if g.phase == Phase::Aiming {
        if g.targets_alive == 0 {
            g.phase = Phase::Cleared;
            let bonus = (g.shots_left.max(0) as u32) * 25;
            g.score += bonus;
        } else if let Some(t) = g.fail_timer.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                g.phase = Phase::Failed;
            }
        }
    }

    // (Geçici varlık temizliği artık DespawnAfter/DespawnBelowY + LifetimeSystem'de —
    // elle döngü kaldırıldı. `transient` yalnız bölüm-yenilemede topluca silmek için.)

    // --- kamerayı nişana göre güncelle ---
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut tr, mut cam)) in q.iter_mut() {
            tr.position = CANNON_POS;
            tr.rotation = Quat::from_rotation_y(-g.yaw);
            cam.yaw = g.yaw;
            cam.pitch = g.pitch;
        }
    }
}

// --------------------------------------------------------------- HUD
fn ui(_world: &mut World, g: &mut Game, ctx: &egui::Context) {
    use egui::{Align2, Color32, RichText};

    // sol-üst bilgi paneli
    egui::Window::new("bilgi")
        .title_bar(false)
        .resizable(false)
        .anchor(Align2::LEFT_TOP, egui::vec2(12.0, 12.0))
        .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(200)))
        .show(ctx, |ui| {
            let names = ["1 · DUVAR", "2 · İKİZ KULE", "3 · GÖKDELEN"];
            ui.label(
                RichText::new(format!("YIKIM USTASI  —  BÖLÜM {}", names[g.level.min(2)]))
                    .strong()
                    .size(20.0)
                    .color(Color32::from_rgb(255, 210, 90)),
            );
            ui.separator();
            ui.label(RichText::new(format!("SKOR: {}", g.score)).size(18.0).color(Color32::WHITE));
            ui.label(
                RichText::new(format!("🎯 Kalan hedef: {}", g.targets_alive))
                    .size(16.0)
                    .color(Color32::from_rgb(255, 220, 120)),
            );
            let ammo_col = if g.shots_left <= 1 {
                Color32::from_rgb(255, 120, 100)
            } else {
                Color32::from_rgb(150, 220, 255)
            };
            ui.label(RichText::new(format!("💣 Gülle: {}", g.shots_left.max(0))).size(16.0).color(ammo_col));
            ui.separator();
            ui.label(RichText::new(format!("FPS: {:.0}", g.fps)).color(Color32::from_rgb(160, 255, 160)));
            ui.small("Fare / WASD: nişan  •  SPACE: güçlü atış  •  Sol tık: hızlı  •  R: tekrar");
        });

    // güç ölçeri (şarj sırasında, altta ortada)
    if g.charging && g.charge > 0.0 {
        egui::Window::new("guc")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(210)))
            .show(ctx, |ui| {
                ui.label(RichText::new("GÜÇ").strong().color(Color32::from_rgb(255, 200, 80)));
                ui.add(
                    egui::ProgressBar::new(g.charge)
                        .desired_width(260.0)
                        .text(format!("{:.0}%", g.charge * 100.0)),
                );
            });
    }

    // crosshair (ekran merkezi)
    let rect = ctx.content_rect();
    let c = rect.center();
    egui::Area::new(egui::Id::new("crosshair"))
        .fixed_pos(egui::pos2(c.x - 8.0, c.y - 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.label(RichText::new("+").size(28.0).strong().color(Color32::from_rgb(255, 255, 255)));
        });

    // orta banner
    let banner = match g.phase {
        Phase::Cleared => Some((
            "BÖLÜM TEMİZLENDİ!  ▶  N = sonraki bölüm",
            Color32::from_rgb(120, 255, 140),
        )),
        Phase::Failed => Some((
            "GÜLLELER BİTTİ!  ↺  R = tekrar dene",
            Color32::from_rgb(255, 120, 110),
        )),
        Phase::AllCleared => Some((
            "🏆 TÜM BÖLÜMLER BİTTİ — USTA OLDUN!  R = baştan",
            Color32::from_rgb(255, 220, 90),
        )),
        Phase::Aiming => None,
    };
    if let Some((text, col)) = banner {
        egui::Window::new("banner")
            .title_bar(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
            .frame(egui::Frame::window(&ctx.global_style()).fill(Color32::from_black_alpha(225)))
            .show(ctx, |ui| {
                ui.label(RichText::new(text).size(26.0).strong().color(col));
                if g.phase == Phase::AllCleared || g.phase == Phase::Cleared {
                    ui.label(RichText::new(format!("Toplam skor: {}", g.score)).size(18.0).color(Color32::WHITE));
                }
            });
    }
}

// --------------------------------------------------------------- render
fn render(
    world: &mut World,
    _g: &Game,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    // gizmo-app'in panik hook'u mesajı tracing::error'a yazar; subscriber olmadan
    // SESSİZ kalır. ERROR-seviye subscriber → olası çökmeler terminale basılır.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .try_init();
    App::<Game>::new("Gizmo — YIKIM USTASI", 1360, 768)
        .add_plugin(PhysicsPlugin::new())
        .add_plugin(LifetimePlugin) // gülle/konfeti ömrü + kill-plane otomatik
        .set_setup(setup)
        .set_update(update)
        .set_ui(ui)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
