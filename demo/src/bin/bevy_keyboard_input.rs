//! # Bevy'nin `input/keyboard_input` örneğinin Gizmo Engine portu
//!
//! Klavyenin üç ayrı sorusu: tuş **şu an basılı mı**, **bu karede mi basıldı**, **bu karede mi
//! bırakıldı**. Üçü de motorda var — `is_key_pressed`, `is_key_just_pressed`,
//! `is_key_just_released` — ve aralarındaki fark bir oyunun her yerinde işe yarar: sürekli
//! hareket birincisini ister, zıplama ikincisini, "bırakınca ateşle" üçüncüsünü.
//!
//! Demo A tuşunu izliyor (Bevy örneğindeki gibi) ve üç kenarı hem günlüğe yazıyor hem ekranda
//! gösteriyor: küp basılıyken büyüyor, basış anında renk değiştiriyor, bırakılınca eski rengine
//! dönüyor. Ayrıca son 40 karenin kenar geçmişi HUD'da duruyor — "bir kenar tek karelik bir
//! olaydır" cümlesinin görünür hâli.
//!
//! ## Bevy'nin ikinci yarısı motorda yok, ve karşılığı da yok
//!
//! Bevy aynı örnekte `ButtonInput<Key>` ile **mantıksal** tuşu da sorguluyor: `'?'` karakteri,
//! klavye düzeninden bağımsız. Gizmo'nun `Input`'u yalnız **fiziksel** tuş kodlarını tutuyor
//! (winit'in `KeyCode`'u, yani tuşun klavyedeki YERİ). Türkçe Q ve F klavyelerde aynı harf farklı
//! yerde olduğu için bu gerçek bir fark: motor bugün "hangi harf yazıldı" sorusunu cevaplayamaz,
//! yalnız "hangi konumdaki tuşa basıldı" sorusunu cevaplar. Bu port o yarıyı taklit etmiyor;
//! motorun boşluğu olarak duruyor.
//!
//! **Zamanlama:** girdi okuyan sistem `add_update_system` ile kaydedilir — kare çizelgesi
//! kenarların yakalanıp temizlendiği yerdir (bkz. `bevy_fixed_timestep`).
//!
//! ## Kontroller
//!   * **A** — izlenen tuş · **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İzlenen tuşa tepki veren küpün işareti.
#[derive(Clone, Copy)]
struct KeyCube;
gizmo::core::impl_component!(KeyCube);

/// Kenarların sayıldığı ve son karelerinin saklandığı kaynak.
#[derive(Clone, Copy)]
struct KeyLog {
    pressed_now: bool,
    just_pressed: u32,
    just_released: u32,
    /// Son 40 karenin durumu: basılı mı, ve o karede bir kenar oldu mu.
    history: [KeyFrame; 40],
    cursor: usize,
}

/// Tek bir karenin klavye durumu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyFrame {
    Up,
    Down,
    Pressed,
    Released,
}

impl Default for KeyLog {
    fn default() -> Self {
        Self {
            pressed_now: false,
            just_pressed: 0,
            just_released: 0,
            history: [KeyFrame::Up; 40],
            cursor: 0,
        }
    }
}

gizmo::core::impl_component!(KeyLog);

/// Basılıyken küpün ölçeği, bırakılmışken olduğunun kaç katı.
const PRESSED_SCALE: f32 = 1.6;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Keyboard Input", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.6, 0.0)).with_scale(Vec3::splat(0.35)),
                GlobalTransform::default(),
                AssetManager::create_cube(&scene.renderer.device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.25, 0.5, 0.9, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                KeyCube,
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 8.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.7),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(KeyLog::default());
            scene.spawn_camera(state, Vec3::new(0.0, 1.8, 4.5), Vec3::new(0.0, 0.6, 0.0));
        })
        // Kenarlar kare çizelgesinde okunur.
        .add_update_system(keyboard_input_system.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(log) = world.get_resource::<KeyLog>().map(|l| *l) else {
                return;
            };
            gizmo::egui::Area::new("keyboard".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("A tuşunun üç sorusu");
                    ui.label(format!(
                        "şu an basılı (is_key_pressed): {}",
                        if log.pressed_now { "evet" } else { "hayır" }
                    ));
                    ui.label(format!("bu karede basıldı sayısı: {}", log.just_pressed));
                    ui.label(format!("bu karede bırakıldı sayısı: {}", log.just_released));
                    ui.separator();
                    // ASCII: egui'nin varsayılan fontunda blok/ok karakterleri yok, kutu çizerdi.
                    ui.label("son 40 kare (. boşta · = basılı · v basış · ^ bırakış):");
                    // En eskiden en yeniye: halka tamponu imleçten itibaren oku.
                    let mut line = String::with_capacity(40);
                    for i in 0..log.history.len() {
                        line.push(match log.history[(log.cursor + i) % log.history.len()] {
                            KeyFrame::Up => '.',
                            KeyFrame::Down => '=',
                            KeyFrame::Pressed => 'v',
                            KeyFrame::Released => '^',
                        });
                    }
                    ui.monospace(line);
                    ui.separator();
                    ui.label("Bevy'nin mantıksal tuş ('?') yarısı motorda yok:");
                    ui.label("Input yalnız fiziksel tuş KONUMUNU biliyor.");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bevy'nin `keyboard_input_system`'i: aynı tuşun üç kenarını da ayrı ayrı sorar.
fn keyboard_input_system(
    mut cubes: Query<(Mut<Transform>, Mut<Material>, With<KeyCube>)>,
    mut log: ResMut<KeyLog>,
    input: Res<Input>,
) {
    const KEY_A: u32 = gizmo::winit::keyboard::KeyCode::KeyA as u32;

    let pressed = input.is_key_pressed(KEY_A);
    let just_pressed = input.is_key_just_pressed(KEY_A);
    let just_released = input.is_key_just_released(KEY_A);

    if just_pressed {
        log.just_pressed += 1;
        gizmo::gizmo_log!(Info, "'A' bu karede basıldı");
    }
    if just_released {
        log.just_released += 1;
        gizmo::gizmo_log!(Info, "'A' bu karede bırakıldı");
    }
    log.pressed_now = pressed;

    // Halka tamponu: en eski kare imlecin gösterdiği yer.
    let frame = match (pressed, just_pressed, just_released) {
        (_, true, _) => KeyFrame::Pressed,
        (_, _, true) => KeyFrame::Released,
        (true, _, _) => KeyFrame::Down,
        _ => KeyFrame::Up,
    };
    let cursor = log.cursor;
    log.history[cursor] = frame;
    log.cursor = (cursor + 1) % log.history.len();

    // Görsel karşılık: basılıyken büyük ve sıcak renkli, bırakılmışken küçük ve soğuk.
    let (scale, albedo) = if pressed {
        (0.35 * PRESSED_SCALE, Vec4::new(0.95, 0.55, 0.2, 1.0))
    } else {
        (0.35, Vec4::new(0.25, 0.5, 0.9, 1.0))
    };
    for (_entity, (mut transform, mut material, _)) in cubes.iter_mut() {
        transform.scale = Vec3::splat(scale);
        material.albedo = albedo;
    }
}
