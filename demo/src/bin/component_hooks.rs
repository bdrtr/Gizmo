//! # Bileşen yaşam döngüsü kancaları
//!
//! Bir bileşen bir varlığa **takıldığında, yazıldığında, söküldüğünde** kod çalıştırmak.
//!
//! ## Motorda kancalar var — dördü de kayıtlı API
//!
//! | yetenek | Gizmo |
//! |--------------|-------|
//! | bileşen ilk takıldığında | [`World::register_on_add`] |
//! | **var olan bir değerin üstüne yazıldığında** | [`World::register_on_replace`] — 2026-08-24 |
//! | takma ve üstüne yazmayı tek kancada toplamak | `on_set`, ikisini de görüyor |
//! | eski değerin üstüne yazılmadan **hemen önce** | **yok** — `on_replace` yazmadan sonra koşuyor, eski değer o an çoktan düşürülmüş |
//! | bileşen söküldüğünde | [`World::register_on_remove`] |
//! | varlık yok edildiğinde | [`World::register_despawn_hook`] — ama **bileşen başına değil, küresel** |
//! | her *yazma*da | [`World::register_on_set`] |
//!
//! Motor bileşen başına bir `Vec` tutuyor: aynı türe kaç kanca isterseniz. Öte yandan yok etme
//! kancası bileşene değil, dünyaya bağlı.
//!
//! ## Asıl bulgu: kanca bir denetim izi DEĞİL
//!
//! Modülün kendi dokümanı bunu açıkça yazıyor — "bir klonlanan prefab `on_add` ya da `on_set`
//! ateşlemeden ortaya çıkar". Demo bunu **tekrarlamıyor, ölçüyor**: on bir farklı yoldan bileşen
//! takıp söküyor ve her yoldan sonra sayaçları okuyor.
//!
//! Ölçüldü (2026-08-24, `Tracked` Table depolu, `TrackedSparse` SparseSet depolu). **rep**
//! sütunu `on_replace`, dördüncü kanca listesi — 2026-08-24'te eklendi:
//!
//! | yol | add | set | rep | remove | despawn |
//! |-----|-----|-----|-----|--------|---------|
//! | `spawn_bundle((Tracked,))` | 1 | 1 | 0 | 0 | 0 |
//! | `add_component` (yeni) | 1 | 1 | 0 | 0 | 0 |
//! | `add_component` (üstüne yaz) | **0** | 1 | **1** | 0 | 0 |
//! | `remove_component` | 0 | 0 | 0 | 1 | 0 |
//! | `add_bundle` (hepsi Table) | **0** | **0** | **0** | **0** | **0** |
//! | `remove_bundle::<(Tracked,)>` | 0 | 0 | 0 | **1** | 0 |
//! | `insert_batch` (3 varlık) | 3 | 3 | 0 | 0 | 0 |
//! | `remove_batch` (3 varlık) | 0 | 0 | 0 | 3 | 0 |
//! | `spawn_batch` (**4** varlık, Table) | **1** | **1** | 0 | 0 | 0 |
//! | `clone_entity` (3 kopya) | **0** | **0** | **0** | **0** | **0** |
//! | `despawn` | 0 | 0 | 0 | 1 | 1 |
//!
//! **İki satır 2026-08-23'ten beri değişti ve ikisi de bu demonun ölçtüğü şeydi.**
//!
//! `remove_bundle` (Table) **0/0/0/0** idi — demet Table bileşenlerini arketip göçüyle sessizce
//! koparıyordu. `On<Remove, T>`'ye dağıtım yolu açılınca aynı bileşenin iki farklı yolla
//! kaldırılınca iki farklı cevap vermesi içeride bir tuhaflık olmaktan çıkıp bozulmuş bir söz
//! hâline geldi ve düzeltildi; satır artık **remove = 1**.
//!
//! `add_component` (üstüne yaz) satırına **rep = 1** eklendi. `on_set` her yazmada koşuyor,
//! `on_add` yalnız ilkinde; aradaki farkı — "üstüne yazıldı" — çağıranın kendi defterini
//! tutmadan görmesinin yolu yoktu. `on_replace` o farkı dağıtım anında ayırıyor: her yazma
//! `on_add` **ya da** `on_replace` tetikliyor, ikisini birden değil.
//!
//! Geriye **iki** sessiz yol kalıyor, ve ikisi de sütunlara doğrudan yazan toplu yollar:
//! `add_bundle` (hepsi Table) ve `clone_entity`. `spawn_batch` de dört varlık doğurup **bir**
//! kanca ateşliyor — geri kalan üçü sütunlara doğrudan yazılıyor. Yani kanca hâlâ bir denetim
//! izi değil; sessiz yolların listesi kısaldı, sıfırlanmadı.
//!
//! ## Kancaya değer gelmiyor — ve `World`'de tipli okuyucu yok
//!
//! `on_set` yalnız varlığı alıyor; ne eski ne yeni değeri. Görmek için dünyadan geri okumak
//! gerekiyor, ve motorda `world.get::<T>(entity)` gibi bir şey **yok** — `World`'ün tipli erişimi
//! `query`/`query_mut` ile başlıyor, tek varlık için bile. Demo bunu yapıyor: `on_set` kancası
//! `world.query::<&Tracked>()` kurup varlığı arıyor, ve okuduğu son değer **11** çıkıyor
//! (`clone_entity`'nin kaynağına verilen değer). Yani okunabiliyor, ama tek bir bileşen için
//! bütün bir sorgu kurmak gerekiyor.
//!
//! (Küçük bir tuzak: sorgunun ürettiği ilk alan `Entity` değil `u32` — ham id. Karşılaştırma
//! `entity.id()` ile yapılıyor.)
//!
//! Ekrandaki küpler bunu gösteriyor: **yeşil** = o yol kanca ateşledi, **kırmızı** = sessiz geçti.
//!
//! ## Neden önemli
//!
//! "Bir indeksi bileşenle senkron tut" — kancaların baş gerekçesi bu, ve Gizmo'da da geçerli
//! bir kullanım. Ama sessiz yolların varlığı şu demek: indeksinizi kancayla tutuyorsanız,
//! `add_bundle`/`spawn_batch`/`clone_entity` kullanan her kod yolu indeksinizi **sessizce**
//! bozuyor. Yavaş yolu seçmek (`spawn_bundle` döngüsü) doğruluk için gerekli olabiliyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::entity::Entity;
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Table depolu izlenen bileşen — motorun hızlı yolları bunun için var.
#[derive(Clone, Copy, Default)]
struct Tracked(u32);
gizmo::core::impl_component!(Tracked);

/// SparseSet depolu izlenen bileşen — hızlı yollar bunu yazamıyor, yavaş yola düşüyorlar.
/// Yükü yok: burada tek işi sparse kod yollarını uyandırmak.
#[derive(Clone, Copy, Default)]
struct TrackedSparse;
gizmo::core::impl_component!(TrackedSparse; gizmo::core::component::StorageType::SparseSet);

/// Kancaların dışarı rapor verdiği sayaçlar.
#[derive(Default)]
struct Tally {
    add: AtomicUsize,
    set: AtomicUsize,
    /// Dördüncü liste, 2026-08-24'te geldi: **var olan bir değerin üstüne yazıldığında**.
    /// `on_set` her yazmada koşuyor, bu yalnız üstüne yazmada — ikisi birlikte `on_add` ile
    /// örtüşmeyen bir bölüntü yapıyor.
    replace: AtomicUsize,
    remove: AtomicUsize,
    despawn: AtomicUsize,
    /// `on_set` kancasının dünyadan **geri okuduğu** son değer. Kancaya yalnız varlık
    /// veriliyor; değeri görmenin tek yolu bu.
    last_value: AtomicUsize,
}

impl Tally {
    fn read(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.add.load(Ordering::Relaxed),
            self.set.load(Ordering::Relaxed),
            self.replace.load(Ordering::Relaxed),
            self.remove.load(Ordering::Relaxed),
            self.despawn.load(Ordering::Relaxed),
        )
    }
}

/// Bir yolun ölçüm satırı.
#[derive(Clone)]
struct Row {
    path: &'static str,
    add: usize,
    set: usize,
    replace: usize,
    remove: usize,
    despawn: usize,
}

impl Row {
    fn silent(&self) -> bool {
        self.add + self.set + self.replace + self.remove + self.despawn == 0
    }
}

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct HookReport {
    rows: Vec<Row>,
    /// `on_set` kancasının dünyadan geri okuyabildiği son değer.
    last_value: usize,
}
gizmo::core::impl_component!(HookReport);

/// İki okuma arasındaki farkı bir satıra çevirir.
fn delta(
    path: &'static str,
    before: (usize, usize, usize, usize, usize),
    after: (usize, usize, usize, usize, usize),
) -> Row {
    Row {
        path,
        add: after.0 - before.0,
        set: after.1 - before.1,
        replace: after.2 - before.2,
        remove: after.3 - before.3,
        despawn: after.4 - before.4,
    }
}

/// Ölçümü yapan yer: on bir yol, her birinden sonra sayaçlar okunuyor.
fn probe(world: &mut World, tally: &Arc<Tally>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut step = |world: &mut World, path: &'static str, f: &mut dyn FnMut(&mut World)| {
        let before = tally.read();
        f(world);
        rows.push(delta(path, before, tally.read()));
    };

    // 1. Tek tek doğurma — bileşenler tek tek `add_component`'tan geçiyor.
    let mut a = Entity::INVALID;
    step(world, "spawn_bundle((Tracked,))", &mut |w| {
        a = w.spawn_bundle((Tracked(1),));
    });

    // 2. Yeni bir bileşen takmak: add + set beklenir.
    step(world, "add_component (yeni)", &mut |w| {
        w.add_component(a, TrackedSparse);
    });

    // 3. Var olanın üstüne yazmak: yalnız set beklenir, add DEĞİL.
    step(world, "add_component (üstüne yaz)", &mut |w| {
        w.add_component(a, TrackedSparse);
    });

    // 4. Sökmek.
    step(world, "remove_component", &mut |w| {
        w.remove_component::<TrackedSparse>(a);
    });

    // 5. Demet takmak — hepsi Table ise doküman "hiçbir kanca" diyor.
    let mut b = Entity::INVALID;
    step(world, "add_bundle (hepsi Table)", &mut |w| {
        b = w.spawn();
        w.add_bundle(b, (Tracked(7),));
    });

    // 6. Demet sökmek — doküman "yalnız sparse üyeler" diyor.
    step(world, "remove_bundle (Table)", &mut |w| {
        w.remove_bundle::<(Tracked,)>(b);
    });

    // 7/8. Toplu takma ve sökme.
    let mut batch: Vec<Entity> = Vec::new();
    step(world, "insert_batch (3 varlık)", &mut |w| {
        batch = (0..3).map(|_| w.spawn()).collect();
        w.insert_batch(&batch, Tracked(9));
    });
    step(world, "remove_batch (3 varlık)", &mut |w| {
        w.remove_batch::<Tracked>(&batch);
    });

    // 9. Toplu doğurma — doküman "ilkinden sonrası sessiz" diyor.
    step(world, "spawn_batch (4 varlık, Table)", &mut |w| {
        let _: Vec<Entity> = w.spawn_batch((0..4).map(|i| (Tracked(i),))).collect();
    });

    // 10. Klonlama — doküman "hiçbir yerde kanca çağrısı yok" diyor.
    let source = world.spawn_bundle((Tracked(11),));
    step(world, "clone_entity (3 kopya)", &mut |w| {
        let _ = w.clone_entity(source.id(), 3);
    });

    // 11. Yok etmek: despawn kancası + taşıdığı her bileşen türünün on_remove'u.
    step(world, "despawn (Tracked taşıyan)", &mut |w| {
        w.despawn(source);
    });

    rows
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Component Hooks", 1280, 720)
        .with_simple_scene(|scene, state| {
            let tally = Arc::new(Tally::default());

            // Aynı türe kaç kanca isterseniz: motor bileşen başına bir `Vec` tutuyor.
            // Burada tür başına birer tane yetiyor.
            for _ in 0..1 {
                let t = tally.clone();
                scene.world.register_on_add::<Tracked>(Box::new(move |_, _| {
                    t.add.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_set::<Tracked>(Box::new(move |world, entity| {
                    t.set.fetch_add(1, Ordering::Relaxed);
                    // Kancaya YALNIZ varlık geliyor — ne eski ne yeni değer. Görmek istiyorsan
                    // dünyadan geri okuyacaksın. Ve motorda tipli tek-varlık okuyucusu yok:
                    // `World::query` kurup varlığı aramak zorundasın.
                    let read_back = world.query::<&Tracked>().and_then(|q| {
                        q.iter().find(|(e, _)| *e == entity.id()).map(|(_, c)| c.0)
                    });
                    if let Some(v) = read_back {
                        t.last_value.store(v as usize, Ordering::Relaxed);
                    }
                }));
                let t = tally.clone();
                scene.world.register_on_replace::<Tracked>(Box::new(move |_, _| {
                    t.replace.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_remove::<Tracked>(Box::new(move |_, _| {
                    t.remove.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_add::<TrackedSparse>(Box::new(move |_, _| {
                    t.add.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_set::<TrackedSparse>(Box::new(move |_, _| {
                    t.set.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_replace::<TrackedSparse>(Box::new(move |_, _| {
                    t.replace.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_on_remove::<TrackedSparse>(Box::new(move |_, _| {
                    t.remove.fetch_add(1, Ordering::Relaxed);
                }));
                let t = tally.clone();
                scene.world.register_despawn_hook(Box::new(move |_, _| {
                    t.despawn.fetch_add(1, Ordering::Relaxed);
                }));
            }

            let rows = probe(scene.world, &tally);

            gizmo::gizmo_log!(Info, "{:<32} add set rep rem desp", "yol");
            for row in &rows {
                gizmo::gizmo_log!(
                    Info,
                    "{:<32} {:>3} {:>3} {:>3} {:>3} {:>4}  {}",
                    row.path,
                    row.add,
                    row.set,
                    row.replace,
                    row.remove,
                    row.despawn,
                    if row.silent() { "<- SESSİZ" } else { "" }
                );
            }

            // Ekrandaki karşılığı: yol başına bir küp, sessizse kırmızı.
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);
            let count = rows.len();
            for (index, row) in rows.iter().enumerate() {
                let x = (index as f32 - (count as f32 - 1.0) * 0.5) * 1.15;
                let fired = row.add + row.set + row.replace + row.remove + row.despawn;
                scene.world.spawn_bundle((
                    // Yükseklik = ateşlenen kanca sayısı; sıfırsa yassı kalıyor.
                    Transform::new(Vec3::new(x, fired as f32 * 0.22, 0.0))
                        .with_scale(Vec3::new(0.45, 0.2 + fired as f32 * 0.22, 0.45)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        if row.silent() {
                            Vec4::new(0.85, 0.22, 0.20, 1.0)
                        } else {
                            Vec4::new(0.25, 0.78, 0.35, 1.0)
                        },
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.7),
                intensity: 2.6,
                ..Default::default()
            });

            let last_value = tally.last_value.load(Ordering::Relaxed);
            gizmo::gizmo_log!(
                Info,
                "on_set kancasının dünyadan GERİ OKUDUĞU son Tracked değeri: {}",
                last_value
            );
            scene.world.insert_resource(HookReport { rows, last_value });
            scene.spawn_camera(state, Vec3::new(0.0, 3.5, 9.5), Vec3::new(0.0, 0.6, 0.0));
        })
        .set_ui(|world, _state, ctx| {
            let Some(report) = world.get_resource::<HookReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("hooks".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(500.0);
                        ui.heading("Bileşen yaşam döngüsü kancaları");
                        ui.label("hangi yol kanca ateşliyor — ölçüldü, varsayılmadı");
                        ui.separator();
                        ui.monospace(format!(
                            "{:<30}{:>4}{:>4}{:>4}{:>4}{:>5}",
                            "yol", "add", "set", "rep", "rem", "desp"
                        ));
                        for row in &report.rows {
                            let text = format!(
                                "{:<30}{:>4}{:>4}{:>4}{:>4}{:>5}",
                                row.path, row.add, row.set, row.replace, row.remove, row.despawn
                            );
                            if row.silent() {
                                ui.colored_label(gizmo::egui::Color32::from_rgb(230, 90, 80), text);
                            } else {
                                ui.monospace(text);
                            }
                        }
                        ui.separator();
                        ui.label(format!(
                            "on_set'in geri okuyabildiği son değer: {}",
                            report.last_value
                        ));
                        ui.label("(kancaya yalnız varlık geliyor — değer için sorgu kurmak şart,");
                        ui.label(" ve World'de tipli tek-varlık okuyucusu yok.)");
                        ui.separator();
                        ui.label("kırmızı satır = kanca ateşlemeyen yol.");
                        ui.label("kancayla indeks tutuyorsanız bu yollar indeksi sessizce bozar.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
