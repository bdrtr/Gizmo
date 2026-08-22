//! # Bevy'nin `gizmos/3d_gizmos` örneğinin Gizmo Engine portu
//!
//! Motorun **anlık-mod hata ayıklama çizimi** — her frame sıfırlanıp yeniden çizilen, sahnede
//! hiçbir varlığa karşılık gelmeyen çizgiler. Bir ızgara, bir küre, bir küp anahattı, dönen bir
//! dikdörtgen, bir artı işareti, hareket eden bir ok ve zamanla salınan bir eğri: hepsi
//! `Gizmos` kaynağına yazılıyor, renderer TAA'dan SONRA ekrana basıyor (hayalet izi olmasın diye).
//!
//! **Motorun çizim yüzeyi Bevy'ninkinden dar, ve port bunu saklamıyor.** Bevy'de `gizmos.grid`,
//! `.sphere`, `.rect`, `.cross`, `.arrow`, `.primitive_3d` gibi hazır çağrılar var; Gizmo'da
//! `Gizmos` yalnız dört şey biliyor: [`Gizmos::draw_line`], [`Gizmos::draw_box`],
//! [`Gizmos::draw_aabb`] ve [`Gizmos::draw_frustum`]. Dolayısıyla bu demodaki ızgara, küre, ok
//! ve eğri **çizgilerden kuruluyor** — ve o yardımcılar dosyanın altında duruyor, çünkü asıl
//! öğretici olan da bu: elde olan ilkel `draw_line`, geri kalanı üç satırlık geometri.
//!
//! Kalıcı `Gizmo` varlığı (Bevy'nin 30 000 çizgilik statik küresi) de motorda yok: burada her
//! şey her frame yeniden yazılıyor, ki `Gizmos::clear` + yeniden çizim tam olarak bu demoda
//! görülüyor.
//!
//! Sahnenin geri kalanı Bevy'nin `setup`'ıyla aynı: 5×5 zemin (`srgb(0.3, 0.5, 0.3)`), orijinde
//! 1×1×1 küp (`srgb(0.8, 0.7, 0.6)`), `(4, 8, 4)`'te nokta ışığı, `(0, 1.5, 6)`'dan orijine bakan
//! kamera.
//!
//!
//! **Bir motor tuzağı, ve Bevy'den gelen biri için görünmez:** `add_system` sistemi **sabit
//! adımlı** çizelgeye kaydeder — kare başına sıfır ya da birkaç kez koşar. Girdi kenarları
//! (`is_key_just_pressed`) ise kare başına bir kez yakalanıp temizlenir, yani o çizelgede
//! okunursa basışlar kaçar ya da iki kez işlenir. Girdiye bakan her sistem bu yüzden
//! `add_update_system` ile kaydediliyor (kare başına tam bir kez, gerçek `dt` ile).
//! ## Kontroller
//!   * **Space** — çizimi duraklat/sürdür
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::f32::consts::{FRAC_PI_2, TAU};

/// `Color::srgb(0.8, 0.7, 0.6)`'nın doğrusal karşılığı — küp.
const CUBE_ALBEDO: Vec4 = Vec4::new(0.603_827_2, 0.447_988_8, 0.318_546_9, 1.0);
/// `Color::srgb(0.3, 0.5, 0.3)`'ün doğrusal karşılığı — zemin.
const GROUND_ALBEDO: Vec4 = Vec4::new(0.073_239_45, 0.214_041_14, 0.073_239_45, 1.0);

const GRAY: [f32; 4] = [0.65, 0.65, 0.65, 1.0];
const PURPLE: [f32; 4] = [0.5, 0.0, 0.5, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const LIME: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const FUCHSIA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const YELLOW: [f32; 4] = [1.0, 1.0, 0.0, 1.0];

/// Duraklatma durumu — Space ile değişir, çizim sistemi buna bakar.
#[derive(Default, Clone, Copy)]
struct GizmoDemoState {
    paused: bool,
    /// Duraklatıldığında donan sahne saati.
    clock: f32,
}
gizmo::core::impl_component!(GizmoDemoState);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy 3D Gizmos", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            // Zemin 5×5.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 5.0),
                Material::new(white.clone()).with_pbr(GROUND_ALBEDO, 1.0, 0.0),
                MeshRenderer::new(),
            ));

            // Orijindeki küp (gizmo küp anahattı bunun etrafını saracak).
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white).with_pbr(CUBE_ALBEDO, 0.5, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(4.0, 8.0, 4.0),
                intensity: 30.0,
                radius: 40.0,
                ..Default::default()
            });

            // Anlık-mod çizim tamponu ve demo durumu — ikisi de kaynak.
            scene.world.insert_resource(Gizmos::default());
            scene.world.insert_resource(GizmoDemoState::default());

            scene.spawn_camera(state, Vec3::new(0.0, 1.5, 6.0), Vec3::ZERO);
        })
        .add_update_system(toggle_pause.in_phase(Phase::Update))
        .add_update_system(draw_example_collection.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Space duraklatır — donan şey saat, çizim değil (duraklatınca çizgiler ekranda kalır).
fn toggle_pause(mut state: ResMut<GizmoDemoState>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        state.paused = !state.paused;
    }
}

/// Bevy'nin `draw_example_collection`'ı: her frame tamponu sıfırla, her şeyi yeniden çiz.
fn draw_example_collection(
    mut gizmos: ResMut<Gizmos>,
    mut state: ResMut<GizmoDemoState>,
    time: Res<Time>,
) {
    if !state.paused {
        state.clock += time.dt();
    }
    let t = state.clock;

    // **Anlık mod bu satırdır**: geçen frame'in çizgileri atılır, sahne yeniden tarif edilir.
    gizmos.clear();

    // Zemine serilmiş 20×20 ızgara, 2 birim aralıklı.
    draw_grid(&mut gizmos, Vec3::ZERO, 20, 2.0, GRAY);

    // (10, 10, 10)'da mor bir küre ve onu taşıyan ikinci ızgara — Bevy'de de oradalar.
    draw_sphere(&mut gizmos, Vec3::splat(10.0), 1.0, PURPLE, 24);
    draw_grid(&mut gizmos, Vec3::splat(10.0), 20, 2.0, PURPLE);

    // Küpün anahattı — `draw_box` motorun kendi ilkel çağrısı.
    gizmos.draw_box(Vec3::new(-0.625, -0.125, -0.625), Vec3::new(0.625, 1.125, 0.625), BLACK);

    // Y ekseni etrafında ileri geri süzülen 2×2 dikdörtgen.
    let rect_x = t.cos() * 2.5;
    draw_rect(
        &mut gizmos,
        Vec3::new(rect_x, 1.0, 0.0),
        Quat::from_rotation_y(FRAC_PI_2),
        Vec2::splat(2.0),
        LIME,
    );

    // Artı işareti.
    draw_cross(&mut gizmos, Vec3::new(-1.0, 1.0, 1.0), 0.5, FUCHSIA);

    // Zamanla dönen ok — başı gerçek bir ok ucu (iki kısa çizgi).
    let angle = t * 0.6;
    draw_arrow(
        &mut gizmos,
        Vec3::new(0.0, 0.2, 0.0),
        Vec3::new(angle.cos() * 2.0, 0.2, angle.sin() * 2.0),
        YELLOW,
    );

    // Salınan sinüs eğrisi — parçalı doğrularla çizilen bir eğri.
    let mut prev = None;
    for i in 0..=64 {
        let x = -3.0 + i as f32 * (6.0 / 64.0);
        let p = Vec3::new(x, 1.5 + (x * 1.5 + t * 2.0).sin() * 0.4, -2.0);
        if let Some(q) = prev {
            gizmos.draw_line(q, p, [0.2, 0.7, 1.0, 1.0]);
        }
        prev = Some(p);
    }
}

// ── Bevy'de hazır gelen, burada çizgiden kurulan ilkel şekiller ───────────────────────────

/// `cells × cells` gözlü, `spacing` aralıklı yatay ızgara.
fn draw_grid(gizmos: &mut Gizmos, center: Vec3, cells: u32, spacing: f32, color: [f32; 4]) {
    let half = cells as f32 * spacing * 0.5;
    for i in 0..=cells {
        let offset = -half + i as f32 * spacing;
        gizmos.draw_line(
            center + Vec3::new(offset, 0.0, -half),
            center + Vec3::new(offset, 0.0, half),
            color,
        );
        gizmos.draw_line(
            center + Vec3::new(-half, 0.0, offset),
            center + Vec3::new(half, 0.0, offset),
            color,
        );
    }
}

/// Üç dik çemberden kurulan küre kafesi.
fn draw_sphere(gizmos: &mut Gizmos, center: Vec3, radius: f32, color: [f32; 4], segments: u32) {
    for axis in 0..3 {
        let mut prev = None;
        for i in 0..=segments {
            let a = i as f32 / segments as f32 * TAU;
            let (s, c) = (a.sin() * radius, a.cos() * radius);
            let p = center
                + match axis {
                    0 => Vec3::new(c, s, 0.0),
                    1 => Vec3::new(c, 0.0, s),
                    _ => Vec3::new(0.0, c, s),
                };
            if let Some(q) = prev {
                gizmos.draw_line(q, p, color);
            }
            prev = Some(p);
        }
    }
}

/// Verilen dönüşle yerleştirilmiş dikdörtgen anahattı.
fn draw_rect(gizmos: &mut Gizmos, center: Vec3, rotation: Quat, size: Vec2, color: [f32; 4]) {
    let (hx, hy) = (size.x * 0.5, size.y * 0.5);
    let corners = [
        Vec3::new(-hx, -hy, 0.0),
        Vec3::new(hx, -hy, 0.0),
        Vec3::new(hx, hy, 0.0),
        Vec3::new(-hx, hy, 0.0),
    ];
    for i in 0..4 {
        gizmos.draw_line(
            center + rotation * corners[i],
            center + rotation * corners[(i + 1) % 4],
            color,
        );
    }
}

/// Üç eksende de uzanan artı işareti.
fn draw_cross(gizmos: &mut Gizmos, center: Vec3, half: f32, color: [f32; 4]) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        gizmos.draw_line(center - axis * half, center + axis * half, color);
    }
}

/// Uçları geriye kırılmış iki kısa çizgiyle ok başı.
fn draw_arrow(gizmos: &mut Gizmos, from: Vec3, to: Vec3, color: [f32; 4]) {
    gizmos.draw_line(from, to, color);
    let dir = (to - from).normalize_or_zero();
    if dir == Vec3::ZERO {
        return;
    }
    // Ok başı, gövdeye dik iki kanat.
    let side = dir.cross(Vec3::Y).normalize_or_zero() * 0.15;
    let back = to - dir * 0.35;
    gizmos.draw_line(to, back + side, color);
    gizmos.draw_line(to, back - side, color);
}
