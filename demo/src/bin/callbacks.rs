//! # Bileşende saklanan mantık
//!
//! **Mantığı bir bileşende saklamak** ve istendiğinde çalıştırmak: ara sahneler, betikli olaylar,
//! tek seferlik arayüz etkileşimleri için.
//!
//! ## Motorda sistem kaydı yok — ama kapatma (closure) var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | bir sistemi kaydedip kimliğini almak | **yok** |
//! | saklanan kimlikle o sistemi çalıştırmak | **yok** |
//! | çizelgeye girmemiş sistemi çağırmak | **yok** ([`ecs_extension_points`](../ecs_extension_points/index.html)) |
//! | bileşende kod saklamak | var — `Box<dyn FnMut(&mut World, Entity)>` |
//!
//! Yani "sistem" saklanamıyor, ama **kapatma** saklanabiliyor. Fark üç yerde:
//!
//! 1. **Parametre çıkarımı yok.** Kayıtlı bir sistem parametrelerini (`Query<&Callback>` gibi)
//!    kendi imzasında ilan ederdi; burada saklanan şey `&mut World` alan bir kapatma, sorgusunu
//!    kendi kuruyor.
//! 2. **Çizelgeye görünmez.** Kapatma çizelgenin bildiği bir şey değil, yani paralelleştirilemiyor
//!    ve erişim çakışması statik olarak kontrol edilmiyor.
//! 3. **Bileşen `Clone` olmak zorunda.** Motorun `Component`'i `Clone` istiyor, `Box<dyn FnMut>`
//!    `Clone` değil — bu yüzden kapatma `Arc<Mutex<…>>` içinde.
//!
//! Buna karşılık **dışlayıcı kapatma bedava geliyor**: `|world: &mut World|` burada özel bir
//! durum değil, tek biçim o.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23, `GIZMO_CALLBACKS_SELFTEST=1` — üçü de aynı karede çalışıyor):
//!
//! | geri çağrı | yaptığı iş | döndürdüğü |
//! |------------|------------|------------|
//! | `önemsiz` | hiçbir şeye dokunmuyor | `sabit bir cevap` |
//! | `dünyayı değiştiren` | yeni bir varlık doğuruyor | `2 -> yeni varlık 6 doğurdu` |
//! | `sorgu kuran` | dünyayı sorgulayıp sayıyor | `dünyada 3 geri çağrı var` |
//!
//! Üçü de çalışıyor, ve üçüncüsü dünyadaki **3** geri çağrıyı sayıyor.
//!
//! Tablonun asıl söylediği şey orta sütunda: üç ayrı iş — hiçbir şeye dokunmamak, dünyayı
//! okumak, dünyayı değiştirmek — burada **tek bir imzaya** iniyor
//! (`FnMut(&mut World, Entity) -> String`). Sorgu alan bir biçim ve dışlayıcı bir biçim diye
//! ayrı ayrı yazılamıyor; sorgusunu kuran, kapatmanın kendisi.
//!
//! **Çalışma sırası doğum sırası değil**, ve bu bir hata değil: bekleyen geri çağrı `Pending`
//! bileşeni sökülünce arketip değiştiriyor, sorgunun ürettiği sıra da onunla kayıyor. Yani sıra
//! sorgudan geliyor ve **çalıştıkça** değişiyor. Belirli bir sıra isteyen bir kullanım (bir ara
//! sahnenin adımları, mesela) sırayı bileşende taşımalı.
//!
//! ## Kontroller
//!   * **Boşluk** — sıradaki geri çağrıyı çalıştır
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::entity::Entity;
use gizmo::core::input::Input;
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::{Arc, Mutex};

/// Mantığı taşıyan bileşen.
///
/// Kapatma `Arc<Mutex<…>>` içinde — çünkü `Component` `Clone` istiyor ve
/// `Box<dyn FnMut>` `Clone` değil. Sarmalayıcının varlık sebebi bu, tercih değil.
#[derive(Clone)]
struct Callback {
    name: &'static str,
    run: Arc<Mutex<Box<dyn FnMut(&mut World, Entity) -> String + Send + Sync>>>,
}
gizmo::core::impl_component!(Callback);

/// Bir geri çağrının çalıştırılabilir olduğunu işaretler.
#[derive(Clone, Copy)]
struct Pending;
gizmo::core::impl_component!(Pending);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct CallbackReport {
    ran: u32,
    log: Vec<String>,
    total: usize,
}
gizmo::core::impl_component!(CallbackReport);

/// Yardımcı: kapatmayı bileşene sarar.
fn callback(
    name: &'static str,
    f: impl FnMut(&mut World, Entity) -> String + Send + Sync + 'static,
) -> Callback {
    Callback {
        name,
        run: Arc::new(Mutex::new(Box::new(f))),
    }
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Callbacks", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Üç geri çağrı — üçü de aynı biçimde, çünkü tek biçim var.
            let callbacks = [
                // 1. Önemsiz olan.
                callback("önemsiz", |_world, _entity| "sabit bir cevap".to_string()),
                // 2. "Sıradan sistem" biçimi: `Query<&Callback>` alıp sayıyor. Burada sorguyu
                //    kapatma kendi kuruyor — parametre çıkarımı yok.
                callback("sorgu kuran", |world, _entity| {
                    let n = world
                        .query::<&Callback>()
                        .map(|q| q.iter().count())
                        .unwrap_or(0);
                    format!("dünyada {n} geri çağrı var")
                }),
                // 3. "Dışlayıcı sistem" biçimi: `|world: &mut World|`. Burada ayrıcalıklı bir
                //    durum değil — zaten hepsi böyle.
                callback("dünyayı değiştiren", |world, entity| {
                    let spawned = world.spawn_bundle((Transform::new(Vec3::new(0.0, 2.5, 0.0)),));
                    format!("{} -> yeni varlık {} doğurdu", entity.id(), spawned.id())
                }),
            ];

            let total = callbacks.len();
            for (i, cb) in callbacks.into_iter().enumerate() {
                let x = (i as f32 - (total as f32 - 1.0) * 0.5) * 2.0;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.6)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.35, 0.65, 0.85, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    cb,
                    Pending,
                ));
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });
            scene.world.insert_resource(CallbackReport {
                total,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO);
        })
        .set_update(|world, state, dt, input| {
            // ÖNCE bu: `set_update` basit sahnenin kendi kancasını DEĞİŞTİRİYOR, eklemiyor.
            // Bu satır olmadan kamera denetimi, fizik adımı ve transform yayılımı sessizce gider.
            demo::simple_scene_update(world, state, dt, input);
            run_next(world, input);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<CallbackReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("cb".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Bileşende saklanan mantık");
                        ui.label("register_system / run_system motorda YOK.");
                        ui.label("saklanabilen şey sistem değil, kapatma.");
                        ui.separator();
                        ui.label(format!("çalışan: {} / {}", r.ran, r.total));
                        for line in &r.log {
                            ui.monospace(line);
                        }
                        ui.separator();
                        ui.label("Component Clone istiyor, Box<dyn FnMut> Clone değil ->");
                        ui.label("kapatma Arc<Mutex<..>> içinde.");
                        ui.separator();
                        ui.label("boşluk — sıradakini çalıştır");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **Boşluk**: bekleyen ilk geri çağrıyı çalıştırır.
///
/// Bunun `set_update`'te olmasının sebebi `state_scoped`'takiyle aynı: kapatma `&mut World`
/// istiyor ve çizelgeye kayıtlı hiçbir sistem onu alamıyor.
fn run_next(world: &mut World, input: &Input) {
    let fire = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32)
        || std::env::var("GIZMO_CALLBACKS_SELFTEST").is_ok();
    if !fire {
        return;
    }

    // Bekleyen ilk geri çağrıyı bul. Bileşeni klonluyoruz — `Arc` sayesinde ucuz, ve kapatmayı
    // çalıştırırken dünyayı ödünç almamış oluyoruz.
    let next = world.query::<(&Callback, &Pending)>().and_then(|q| {
        q.iter()
            .next()
            .map(|(id, (cb, _))| (id, cb.clone()))
    });
    let Some((id, cb)) = next else {
        return;
    };
    let Some(entity) = world.entity(id) else {
        return;
    };

    world.remove_component::<Pending>(entity);
    let message = {
        let mut guard = cb.run.lock().unwrap();
        (guard)(world, entity)
    };

    if let Some(mut report) = world.get_resource_mut::<CallbackReport>() {
        report.ran += 1;
        report.log.push(format!("{}: {}", cb.name, message));
        gizmo::gizmo_log!(Info, "geri çağrı [{}] -> {}", cb.name, message);
    }
}
