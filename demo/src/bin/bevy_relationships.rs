//! # Bevy'nin `ecs/relationships` örneğinin Gizmo Engine portu
//!
//! Varlıklar arası **ilişki**: "bu birim şunu hedefliyor" (`Targeting`) ve tersi, "bu birimi
//! şunlar hedefliyor" (`TargetedBy`). Bevy'de bu iki bileşen ve iki öznitelik —
//! `#[relationship(relationship_target = TargetedBy)]` — ve ters indeksi motor kendi tutuyor.
//!
//! ## Motorda genel ilişki yok, ama kanca var
//!
//! | Bevy | Gizmo |
//! |------|-------|
//! | `Relationship` / `RelationshipTarget` trait'leri | **yok** |
//! | `#[relationship]` türetmesi | **yok** |
//! | ters indeksi motorun tutması | **yok** |
//! | yerleşik `ChildOf` / `Children` | var, ama **sabit**: [`Parent`] / [`Children`], başka ilişki tanımlanamıyor |
//! | ilişki ağacında gezinme yardımcıları | **yok** |
//!
//! Ama ters indeksi **elle kurmanın** aracı motorda var: bileşen yaşam döngüsü kancaları
//! ([`bevy_component_hooks`](../bevy_component_hooks/index.html) demosunda ölçüldü). Bu demo
//! Bevy'nin türetmesinin yaptığı işi elle yapıyor — `Targeting` takılınca ters indeksi güncelleyen
//! bir `on_add`, sökülünce temizleyen bir `on_remove`.
//!
//! ## Ve elle kurulan indeksin bir deliği var — ölçülüyor
//!
//! Kancalar her yoldan ateşlenmiyor. `bevy_component_hooks` bunu ölçtü: `add_bundle`'ın Table
//! yolu, `spawn_batch`'in ilk varlıktan sonrası ve `clone_entity` **sessiz**. Bevy'nin motor
//! içinde tuttuğu indeks bu yollardan da geçer; elle kurulan indeks geçmez.
//!
//! Demo bunu doğrudan ölçüyor: aynı ilişkiyi iki farklı yoldan kuruyor ve indeksi sayıyor.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23, 6 birim çember üstünde + 4 birim toplu doğurmayla):
//!
//! | | dünyada gerçekte | ters indekste |
//! |---|------------------|---------------|
//! | A) 6 × `add_component` | 6 | **6** |
//! | B) + `spawn_batch` ile 4 tane daha | 10 | **7** |
//!
//! Dünya dört ilişki kazandı, indeks **bir**. Üçü sessizce kayıp — ve sayı tesadüf değil:
//! `spawn_batch` dört varlığın yalnız ilkinde kanca ateşliyor, geri kalan üçünü doğrudan
//! arketip sütunlarına yazıyor. `bevy_component_hooks`'un tablosunda ölçülen `1/4` oranının
//! buradaki bedeli bu.
//!
//! Bevy'de bu delik yok, çünkü indeksi motor tutuyor ve her yol motorun içinden geçiyor. Gizmo'da
//! ilişki kurmak isteyen bir oyunun ya **yalnız kancanın gördüğü yolları** kullanması, ya da
//! indeksi kancaya değil bir sisteme (her kare tarayan) bağlaması gerekiyor.
//!
//! **Boşluk** tuşu bunu gösteriyor: hedefleri `add_component` ile yeniden dağıtmak indeksi
//! toparlıyor, çünkü o yol kanca ateşliyor.
//!
//! ## Kontroller
//!   * **Boşluk** — hedefleri yeniden dağıt
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// İlişkinin **kaynağı**: bu varlık şu varlığı hedefliyor. Bevy'de `#[relationship]`.
#[derive(Clone, Copy)]
struct Targeting(u32);
gizmo::core::impl_component!(Targeting);

/// Bir birim.
#[derive(Clone, Copy)]
struct Unit;
gizmo::core::impl_component!(Unit);

/// İlişkinin **tersi**: hedef -> onu hedefleyenler. Bevy'de bunu motor tutuyor, burada biz.
type ReverseIndex = Arc<Mutex<HashMap<u32, Vec<u32>>>>;

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct RelReport {
    /// Kanca yoluyla kurulan ilişki sayısı.
    via_hook: usize,
    /// Sessiz yoldan (toplu doğurma) kurulan ve indekse **girmeyen** ilişki sayısı.
    missed: usize,
    /// Dünyada gerçekte kaç `Targeting` var.
    actual: usize,
    /// Ters indekste kaç kayıt var.
    indexed: usize,
    lines: Vec<String>,
}
gizmo::core::impl_component!(RelReport);

const UNITS: usize = 6;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Relationships", 1280, 720)
        .with_simple_scene(|scene, state| {
            let index: ReverseIndex = Arc::new(Mutex::new(HashMap::new()));

            // Bevy'nin `#[relationship]` türetmesinin yaptığı işin elle karşılığı: ters indeksi
            // ayakta tutan iki kanca.
            {
                let idx = index.clone();
                scene.world.register_on_add::<Targeting>(Box::new(move |world, entity| {
                    let target = world
                        .query::<&Targeting>()
                        .and_then(|q| q.iter().find(|(e, _)| *e == entity.id()).map(|(_, t)| t.0));
                    if let Some(target) = target {
                        idx.lock().unwrap().entry(target).or_default().push(entity.id());
                    }
                }));
                let idx = index.clone();
                scene.world.register_on_remove::<Targeting>(Box::new(move |_world, entity| {
                    let id = entity.id();
                    for list in idx.lock().unwrap().values_mut() {
                        list.retain(|e| *e != id);
                    }
                }));
            }

            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Birimleri bir çember üstüne diz.
            let mut ids = Vec::new();
            for i in 0..UNITS {
                let angle = i as f32 / UNITS as f32 * std::f32::consts::TAU;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(angle.cos() * 3.2, 0.0, angle.sin() * 3.2))
                        .with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.75, 0.72, 0.68, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Unit,
                ));
                ids.push(entity);
            }

            // A) Kancanın gördüğü yol: tek tek `add_component`.
            for (i, entity) in ids.iter().enumerate() {
                let target = ids[(i + 1) % ids.len()].id();
                scene.world.add_component(*entity, Targeting(target));
            }
            let via_hook = index.lock().unwrap().values().map(|v| v.len()).sum::<usize>();

            // B) Kancanın GÖRMEDİĞİ yol: toplu doğurma. Bevy'nin motor içi indeksi bunu da
            //    yakalardı; elle kurulan indeks yakalamıyor.
            let target = ids[0].id();
            let _: Vec<_> = scene
                .world
                .spawn_batch((0..4).map(|_| (Targeting(target), Unit)))
                .collect();

            let indexed = index.lock().unwrap().values().map(|v| v.len()).sum::<usize>();
            let actual = scene
                .world
                .query::<&Targeting>()
                .map(|q| q.iter().count())
                .unwrap_or(0);

            let mut lines = Vec::new();
            for (target, sources) in index.lock().unwrap().iter() {
                lines.push(format!("  {} <- {:?}", target, sources));
            }
            lines.sort();

            gizmo::gizmo_log!(
                Info,
                "kancayla kurulan {} · toplu doğurmadan sonra indeks {} · dünyada gerçekte {} · KAÇAN {}",
                via_hook,
                indexed,
                actual,
                actual - indexed
            );

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.65),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(RelReport {
                via_hook,
                missed: actual - indexed,
                actual,
                indexed,
                lines,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 5.0, 8.0), Vec3::ZERO);
        })
        .set_update(|world, state, dt, input| {
            // Bkz. `demo::simple_scene_update` — `set_update` basit sahnenin kancasını eziyor.
            demo::simple_scene_update(world, state, dt, input);
            redistribute(world, input);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<RelReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("rel".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("İlişkiler");
                        ui.label("Relationship / RelationshipTarget trait'leri motorda YOK.");
                        ui.label("ters indeksi kancalarla elle kurmak gerekiyor.");
                        ui.separator();
                        ui.label(format!("kancayla kurulan ilişki : {}", r.via_hook));
                        ui.label(format!("dünyada gerçekte        : {}", r.actual));
                        ui.label(format!("ters indekste           : {}", r.indexed));
                        if r.missed > 0 {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 90, 80),
                                format!("İNDEKSE GİRMEYEN         : {}", r.missed),
                            );
                            ui.label("(spawn_batch kanca ateşlemiyor — indeks sessizce eksik)");
                        }
                        ui.separator();
                        for line in r.lines.iter().take(8) {
                            ui.monospace(line);
                        }
                        ui.separator();
                        ui.label("boşluk — hedefleri yeniden dağıt");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **Boşluk**: hedefleri kaydır. `add_component` kanca ateşlediği için indeks kendini toparlıyor.
fn redistribute(world: &mut World, input: &Input) {
    if !input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        return;
    }
    let ids: Vec<u32> = world
        .query::<&Unit>()
        .map(|q| q.iter().map(|(e, _)| e).collect())
        .unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        let target = ids[(i + 2) % ids.len()];
        if let Some(entity) = world.entity(*id) {
            world.add_component(entity, Targeting(target));
        }
    }
}
