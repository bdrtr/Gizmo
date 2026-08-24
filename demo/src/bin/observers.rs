//! # Gözlemciler ve zincirleme tetikleme
//!
//! Gözlemciler: bir şey **olduğu anda** koşan geri çağrılar — olay kuyruğunun bir kare sonra
//! okunmasının tersi. Örnek bir mayın tarlası: bir mayına tıklayınca patlıyor, patlaması menzili
//! içindeki mayınları tetikliyor, onlar da kendi komşularını — zincirleme.
//!
//! ## Motorda gözlemci var, ama üç sınırı var ve üçü de burada görünüyor
//!
//! **1. ~~Geri çağrıya dünya verilmiyor.~~ 2026-08-24'te verildi — ve demo iki kipi de tutuyor.**
//! Motorun belgesi eskiden şöyle diyordu: *"hiçbiri geri çağrıya bir dünya vermez. Bir şey
//! değiştirmesi gereken geri çağrı, yakaladığı bir durumdan geçmek zorundadır."* Yani zincir
//! yakalanmış bir kuyruktan yürümek zorundaydı — gözlemci patlamayı kuyruğa yazıyor, kuyruğu
//! `&mut World` gören bir yer bir sonraki karede boşaltıyordu. Görünür sonucu: zincir **kare
//! kare** ilerliyordu.
//!
//! `observe`'un dinleyicisi artık `&mut World` alıyor, yani zinciri kendi içinden yürütebiliyor:
//! `world.trigger` dinleyicinin içinden yeniden çağrılıyor ve dağıtım özyineliyor. **M** ile iki
//! kip arasında geçiliyor, çünkü zinciri halka halka İZLEMEK demonun yarısı — ama artık bir
//! zorunluluk değil, bir tercih.
//!
//! Özyineleme kendiliğinden bitiyor ve sebebi motorda: dağıtım süresince o an bildirilen
//! varlığın dinleyicileri dünyadan **ayrılıyor**, yani bir dinleyicinin aynı olayı kendi
//! varlığına geri göndermesi sonsuz döngü değil sessiz bir duruş. Kancalarda da aynı şekil.
//!
//! **2. ~~`On<Remove, T>` var ama hiçbir yere bağlı değil.~~ 2026-08-24'te bağlandı.** Bu satır
//! eskiden motorun `observer.rs`'ini alıntılıyordu: `Remove` ve `Replace` işaretçileri *"henüz
//! dağıtım yolu yok"*, ve `add_observer` her zaman `Insert` kuruyor. Artık `add_observer`
//! işaretin kendisi üzerinden genel: kapanın **kendi imzasındaki** `On<E, T>` hangi kanca
//! listesine yazılacağını seçiyor, ve üçü bir bileşenin ömrünü bölüyor — bir yazma `Insert`
//! **ya da** `Replace` tetikliyor, ikisini birden değil, hiçbirini de değil; bir kopma `Remove`.
//! Demo üçünü de sayıyor. Ham kanca (`register_on_remove` ve kardeşleri) kaçış kapısı olarak
//! duruyor: gözlemcinin alamadığı `&mut World`'ü isteyen oradan iniyor.
//!
//! **3. Genel (varlığa bağlı olmayan) gözlemci yok, koşul da yok.** Motorda iki kapı var:
//! bileşen ekleme (`add_observer`) ve **bir varlığa** bağlı olay (`observe`). Hedefsiz bir
//! olayı dinlemenin — ya da bir gözlemciye koşul takmanın — yolu **yok**; demo hedefsiz olayı
//! düz bir fonksiyona çeviriyor.
//!
//! ## Motorun fazlası: olay hiyerarşide yukarı kabarıyor
//!
//! [`EntityEvent::can_propagate`] `true` dönerse `trigger` olayı `Parent` zincirinde yukarı
//! taşıyor ve her atanın dinleyicisini çalıştırıyor. Demo bunu kullanıyor: bütün mayınlar
//! görünmez bir "tarla" varlığının çocuğu, ve tarlanın dinleyicisi her patlamayı sayıyor. Yani
//! HUD'daki toplam sayaç tek bir dinleyiciden geliyor, mayınlardan değil.
//!
//! ## Ölçüldü (2026-08-24, 70 mayın, `GIZMO_OBSERVER_SELFTEST=1`)
//!
//! **İki kip aynı zincire ulaşıyor, farklı sürede.** Merkezdeki aynı mayından başlatıldığında:
//!
//! | kip | kare | patlayan | kabaran |
//! |-----|------|----------|---------|
//! | kuyruk (`M` ile) | **8** | 26/70 | 26 |
//! | anında (`GIZMO_OBSERVER_IMMEDIATE=1`) | **1** | 26/70 | 26 |
//!
//! Patlayan kümesi birebir aynı — 26 mayın, tarlanın merkezdeki mayına bağlı parçası. Değişen
//! yalnız ne zaman bittiği.
//!
//! Kabaran sayısının da eşit çıkması ayrı bir düzeltmenin sonucu: anında kip menzildeki
//! mayınların listesini bir anlık görüntü olarak topluyor, ve listedeki ikinciyi tetiklerken
//! birincinin zinciri onu çoktan patlatmış olabiliyor. Olay hedef patlamış olsa bile yukarı
//! kabardığı için ilk ölçüm 26 yerine **73** verdi; tetiklemeden önce yeniden bakınca 26'ya
//! indi. Kuyruk kipi bu ayıklamayı zaten boşaltma sırasında yapıyordu.
//!
//! ### Kuyruk kipinin halka halka tablosu (2026-08-23'ten beri değişmedi)
//!
//! Merkezdeki tek mayından başlayan zincir, halka halka:
//!
//! | halka | patlayan | tarlaya kabaran |
//! |-------|----------|-----------------|
//! | 1 | 1 | 3 |
//! | 2 | 3 | 5 |
//! | 3 | 5 | 13 |
//! | 4 | 13 | 16 |
//! | 5 | 16 | 22 |
//! | 6 | 22 | 25 |
//! | 7 | 25 | 26 |
//! | 8 | 26 | 26 |
//!
//! Sekiz halka, yani **sekiz kare**, ve 70 mayının 26'sı — tarlanın merkezdeki mayına bağlı olan
//! parçası. Zincir kopukluğun olduğu yerde duruyor, ekranda da öyle görünüyor.
//!
//! Kabaran sayacın patlayan sayısını bu kipte **bir halka önden** götürmesi kuyruğun doğrudan
//! sonucu: kabarma `trigger` anında, patlama ise kuyruk bir sonraki karede boşaltılırken oluyor.
//! Anında kipte ikisi aynı çağrının içinde, yani böyle bir gecikme yok.
//!
//! ## Doğrulama
//!
//! `GIZMO_OBSERVER_SELFTEST=1` fareyi beklemeden 90. karede merkeze en yakın mayını tetikler ve
//! zincirin her halkasını günlüğe basar — yukarıdaki tablo böyle çıktı.
//!
//! ## Kontroller
//!   * **Sol tık** — mayına bas
//!   * **M** — zincir kipi: kuyruk (halka başına bir kare) ↔ anında (tek kare)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::observer::{Insert, On, Remove, Replace};
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::{Arc, Mutex};

/// Bir mayın: yeri ve etki yarıçapı.
#[derive(Clone, Copy)]
struct Mine {
    position: Vec3,
    radius: f32,
    exploded: bool,
}
gizmo::core::impl_component!(Mine);

/// Görünmez tarla kökü — bütün mayınların ebeveyni, ve kabaran olayın son durağı.
#[derive(Clone, Copy)]
struct Field;
gizmo::core::impl_component!(Field);

/// Zincirin hangi kiple yürüdüğü. **Dinleyici bunu dünyadan okuyor** — yani kaynağın kendisi
/// dinleyicinin artık `&mut World` aldığının en kısa kanıtı.
#[derive(Clone, Copy, Default)]
struct ChainMode {
    /// `true`: dinleyici zinciri kendi içinden yürütür, hepsi tek karede biter.
    /// `false`: dinleyici yalnız kuyruğa yazar, zincir kare kare ilerler (2026-08-24 öncesi
    /// tek seçenek buydu, çünkü dinleyicinin dünyası yoktu).
    immediate: bool,
}
gizmo::core::impl_component!(ChainMode);

/// `Explode`: bir varlığa yönelmiş özel olay.
#[derive(Clone, Copy)]
struct Explode {
    target: gizmo::core::entity::Entity,
}

impl gizmo::core::observer::EntityEvent for Explode {
    fn target(&self) -> gizmo::core::entity::Entity {
        self.target
    }

    /// Motorun fazlası: `true` dönünce `trigger` olayı `Parent` zincirinde yukarı taşıyor.
    /// Tarlanın dinleyicisi toplam sayacı böyle tutuyor.
    fn can_propagate(&self) -> bool {
        true
    }
}

/// Gözlemcinin dünyayı göremediği yerde kullandığı kanal.
///
/// Geri çağrı dünyayı görmediği için tek yol yakalanmış paylaşılan bir durum.
#[derive(Clone, Default)]
struct Fuse {
    /// Bu karede patlayan mayınların kimlikleri.
    fired: Arc<Mutex<Vec<u32>>>,
    /// Tarlanın dinleyicisinin saydığı toplam — kabarmanın kanıtı.
    bubbled: Arc<Mutex<u32>>,
    /// Bir bileşenin ömrünün üç evresi, üçü de `add_observer` ile — `On<Insert, Mine>`,
    /// `On<Replace, Mine>`, `On<Remove, Mine>`. `replaced` ile `indexed` toplamı, `Mine`'a
    /// yapılan bütün yazmaları verir; ikisi örtüşmez.
    indexed: Arc<Mutex<u32>>,
    replaced: Arc<Mutex<u32>>,
    removed: Arc<Mutex<u32>>,
}
gizmo::core::impl_component!(Fuse);

/// HUD'un okuduğu defter.
#[derive(Default, Clone)]
struct Report {
    mines: usize,
    exploded: usize,
    /// Zincirin kaç halkası koştu.
    rings: u32,
    bubbled: u32,
    indexed: u32,
    replaced: u32,
    removed: u32,
    /// Zincirin hangi kiple yürüdüğü — HUD bunu yazıyor.
    immediate: bool,
    frame: u32,
    /// İmlecin zemin düzlemine düştüğü nokta — HUD'da ve tıklamada kullanılıyor.
    aim: Vec3,
}
gizmo::core::impl_component!(Report);

/// Sağlam ve patlamış mayının rengi.
const ARMED: Vec4 = Vec4::new(0.30, 0.34, 0.40, 1.0);
const FIRED: Vec4 = Vec4::new(0.95, 0.35, 0.15, 1.0);
/// Tarlanın yarı boyu ve mayın sayısı.
const FIELD_HALF: f32 = 8.0;
const MINE_COUNT: usize = 70;
/// Kendi kendine sınamanın karesi.
const SELFTEST_FRAME: u32 = 90;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Observers", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 26.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.10, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.9),
                intensity: 2.4,
                ..Default::default()
            });

            let fuse = Fuse::default();
            scene.world.insert_resource(fuse.clone());
            scene.world.insert_resource(Report::default());
            // Kip: kendi kendini sınayan koşu ANINDA kiple başlıyor, çünkü ölçülecek asıl şey
            // o. Elle oynarken varsayılan kuyruk kipi — zinciri halka halka görmek için.
            scene.world.insert_resource(ChainMode {
                immediate: std::env::var("GIZMO_OBSERVER_IMMEDIATE").is_ok(),
            });

            // Bir bileşenin ömrünün üç evresi, üç gözlemci. Hangi kanca listesine yazıldığını
            // kapanın kendi argüman tipi seçiyor — turbofish yok, işaret imzada duruyor.
            let indexed = fuse.indexed.clone();
            scene.world.add_observer(move |_: On<Insert, Mine>| {
                *indexed.lock().expect("sayaç kilidi") += 1;
            });
            // `Mine` üzerine yeniden yazılınca. `Insert` ile ÖRTÜŞMEZ: ilk yazma insert,
            // sonrakiler replace.
            let replaced = fuse.replaced.clone();
            scene.world.add_observer(move |_: On<Replace, Mine>| {
                *replaced.lock().expect("sayaç kilidi") += 1;
            });
            // Çıkarma tarafı — 2026-08-24'e kadar burada ham bir `register_on_remove` kancası
            // vardı, çünkü `On<Remove, T>` hiçbir yere dağıtılmıyordu.
            let removed = fuse.removed.clone();
            scene.world.add_observer(move |_: On<Remove, Mine>| {
                *removed.lock().expect("sayaç kilidi") += 1;
            });

            build_field(scene.world, device, &white, &fuse);
            scene.spawn_camera(state, Vec3::new(0.0, 14.0, 15.0), Vec3::ZERO);
        })
        .add_update_system(aim_at_ground.in_phase(Phase::Update))
        // Zinciri yürüten yer: gözlemci dünyayı göremediği için kuyruğu burada boşaltıyoruz.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let (clicked, toggled) = world
                .get_resource::<gizmo::core::input::Input>()
                .map(|i| {
                    (
                        i.is_mouse_button_just_pressed(0),
                        i.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyM as u32),
                    )
                })
                .unwrap_or((false, false));
            if toggled {
                if let Some(mut mode) = world.get_resource_mut::<ChainMode>() {
                    mode.immediate = !mode.immediate;
                }
            }
            drive_chain(world, clicked);

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(report) = world.get_resource::<Report>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("obs".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.set_min_width(430.0);
                    ui.heading("Gözlemciler");
                    ui.label(format!(
                        "mayın {} · patlayan {} · zincirin halkası {}",
                        report.mines, report.exploded, report.rings
                    ));
                    ui.separator();
                    ui.label(format!(
                        "On<Insert, Mine>  : {} ekleme",
                        report.indexed
                    ));
                    ui.label(format!(
                        "On<Replace, Mine> : {} üzerine yazma",
                        report.replaced
                    ));
                    ui.label(format!(
                        "On<Remove, Mine>  : {} çıkarma",
                        report.removed
                    ));
                    ui.label(format!(
                        "tarlaya KABARAN olay: {} (tek dinleyici)",
                        report.bubbled
                    ));
                    ui.separator();
                    ui.label(if report.immediate {
                        "kip: ANINDA — dinleyici zinciri kendi içinden yürütüyor (M)"
                    } else {
                        "kip: KUYRUK — dinleyici kuyruğa yazıyor, halka başına bir kare (M)"
                    });
                    ui.label("üç evre de add_observer ile — işaret kapanın imzasında");
                    ui.label("Insert ile Replace örtüşmez: yazma sayısı ikisinin toplamı");
                    ui.separator();
                    ui.label("sol tık — mayına bas");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// İmleci zemin düzlemine düşürür ve kareyi sayar.
///
/// Işın fizik dünyasına değil, doğrudan `y = 0` düzlemine atılıyor: mayınların çarpışma şekli
/// yok, ve bir düzlem kesişimi için de gerekmiyor.
fn aim_at_ground(
    mut report: ResMut<Report>,
    cameras: Query<(&Camera, &Transform)>,
    window: Res<WindowInfo>,
    input: Res<Input>,
) {
    report.frame += 1;

    let mut camera_data = None;
    for (_entity, (camera, transform)) in cameras.iter() {
        if camera.primary {
            camera_data = Some((*camera, transform.position));
            break;
        }
    }
    let Some((camera, camera_pos)) = camera_data else {
        return;
    };
    let (mx, my) = input.mouse_position();
    if mx < 0.0 || my < 0.0 || mx > window.width || my > window.height {
        return;
    }
    let ray = camera.screen_to_ray((mx, my), (window.width, window.height), camera_pos);
    let origin = Vec3::from(ray.origin);
    let direction = Vec3::from(ray.direction);
    if direction.y.abs() < 1e-5 {
        return;
    }
    let t = -origin.y / direction.y;
    if t > 0.0 {
        report.aim = origin + direction * t;
    }
}

/// **Kuyruk kipi**: kuyruğu boşalt, patlayanları boya, menzildekileri tetikle — halka başına
/// bir kare. Anında kipte kuyruk hep boş kalır ve bu fonksiyon yalnız defteri günceller.
///
/// Bu, 2026-08-24 öncesi tek seçenekti: dinleyicinin dünyası yoktu, yani gerçek iş `&mut World`
/// gören bir yere ertelenmek zorundaydı, ve o yer bir sonraki kareydi. Kip olarak duruyor
/// çünkü zinciri halka halka İZLEMEK demonun yarısı — ama artık zorunluluk değil, tercih.
fn drive_chain(world: &mut gizmo::core::World, clicked: bool) {
    // Kendi kendine sınama: fare yoksa merkeze en yakın mayını tetikle.
    let (frame, aim) = world
        .get_resource::<Report>()
        .map(|r| (r.frame, r.aim))
        .unwrap_or((0, Vec3::ZERO));
    if std::env::var("GIZMO_OBSERVER_SELFTEST").is_ok() && frame == SELFTEST_FRAME {
        if let Some(entity) = nearest_mine(world, Vec3::ZERO) {
            trigger_explode(world, entity);
        }
    }
    // Tıklama: imlecin düştüğü noktayı KAPSAYAN mayına bas — yarıçapa bakılıyor.
    if clicked {
        if let Some(entity) = mine_under(world, aim) {
            trigger_explode(world, entity);
        }
    }

    let Some(fuse) = world.get_resource::<Fuse>().map(|f| f.clone()) else {
        return;
    };

    // Anında kipte zincir yukarıdaki `trigger_explode` içinde ZATEN bitti. Sınama çıktısı
    // burada, tek satır — kuyruk kipinin halka halka tablosunun karşılığı.
    let immediate_mode = world
        .get_resource::<ChainMode>()
        .map(|m| m.immediate)
        .unwrap_or(false);
    if immediate_mode && std::env::var("GIZMO_OBSERVER_SELFTEST").is_ok() && frame == SELFTEST_FRAME
    {
        sync_report(world, &fuse, false);
        let report = world.get_resource::<Report>().map(|r| r.clone());
        if let Some(r) = report {
            gizmo::gizmo_log!(
                Info,
                "ANINDA · tek karede · patlayan {}/{} · kabaran {} · halka 1",
                r.exploded,
                r.mines,
                r.bubbled
            );
        }
        return;
    }
    let fired: Vec<u32> = {
        let mut queue = fuse.fired.lock().expect("fitil kilidi");
        std::mem::take(&mut *queue)
    };
    if fired.is_empty() {
        sync_report(world, &fuse, false);
        return;
    }

    // Patlayanları işaretle ve menzillerini topla.
    let mut blasts = Vec::new();
    for id in &fired {
        let Some(entity) = world.get_entity(*id) else {
            continue;
        };
        let mine = world
            .query::<&Mine>()
            .and_then(|q| q.iter().find(|(e, _)| *e == *id).map(|(_, m)| *m));
        let Some(mine) = mine else { continue };
        if mine.exploded {
            continue;
        }
        blasts.push((mine.position, mine.radius));
        world.add_component(
            entity,
            Mine {
                exploded: true,
                ..mine
            },
        );
        if let Some(mut query) = world.query_mut::<Mut<Material>>() {
            if let Some(mut material) = query.get_mut_entity(entity) {
                material.albedo = FIRED;
            }
        }
    }

    // Menzil içindeki sağlam mayınları tetikle — zincirin bir sonraki halkası.
    let mut next = Vec::new();
    if let Some(query) = world.query::<&Mine>() {
        for (id, mine) in query.iter() {
            if mine.exploded {
                continue;
            }
            for (centre, radius) in &blasts {
                if (mine.position - *centre).length() < mine.radius + *radius {
                    next.push(id);
                    break;
                }
            }
        }
    }
    for id in next {
        if let Some(entity) = world.get_entity(id) {
            trigger_explode(world, entity);
        }
    }

    sync_report(world, &fuse, true);
}

/// Bir mayını **dinleyicinin içinden** patlatır: işaretle, boya, menzildekileri tetikle.
///
/// `drive_chain`'in mayın başına yaptığı işin aynısı — fark, bunun bir kare beklemeden, olayın
/// dağıtımı sürerken koşması. `world.trigger` buradan yeniden çağrılıyor, yani dağıtım kendi
/// içinde özyineliyor ve bütün zincir **tek** `trigger` çağrısında bitiyor.
///
/// İki şey özyinelemeyi bitiriyor ve ikisi de kasıtlı:
///
///   * patlamış bir mayın erken dönüyor, yani halka yeniden ziyaret edilmiyor;
///   * motor, o an bildirilen varlığın dinleyicilerini dağıtım süresince dünyadan **ayırıyor**,
///     yani bir dinleyicinin aynı olayı kendi varlığına geri göndermesi sonsuz döngü değil,
///     sessiz bir duruş. Başka varlıkların dinleyicileri canlı ve iç içe koşuyor.
fn explode_now(world: &mut gizmo::core::World, entity: gizmo::core::entity::Entity) {
    let id = entity.id();
    let mine = world
        .query::<&Mine>()
        .and_then(|q| q.iter().find(|(e, _)| *e == id).map(|(_, m)| *m));
    let Some(mine) = mine else { return };
    if mine.exploded {
        return;
    }

    world.add_component(entity, Mine { exploded: true, ..mine });
    if let Some(mut query) = world.query_mut::<Mut<Material>>() {
        if let Some(mut material) = query.get_mut_entity(entity) {
            material.albedo = FIRED;
        }
    }

    let mut next = Vec::new();
    if let Some(query) = world.query::<&Mine>() {
        for (other, candidate) in query.iter() {
            if candidate.exploded {
                continue;
            }
            if (candidate.position - mine.position).length() < candidate.radius + mine.radius {
                next.push(other);
            }
        }
    }
    for other in next {
        // `next` bir ANLIK GÖRÜNTÜ: listedeki ikinci mayını tetiklerken birincinin zinciri
        // onu çoktan patlatmış olabiliyor. Tetiklemeden önce yeniden bakmak gerekiyor, çünkü
        // olay hedef patlamış olsa bile `Parent` zincirinde yukarı KABARIYOR — dinleyici erken
        // dönüyor ama yürüyüş sürüyor. İlk ölçümde bu, kuyruk kipinin 26 kabarmasını anında
        // kipte 73'e çıkarmıştı: aynı zincir, üç katı olay.
        let still_armed = world
            .query::<&Mine>()
            .and_then(|q| q.iter().find(|(e, _)| *e == other).map(|(_, m)| !m.exploded))
            .unwrap_or(false);
        if !still_armed {
            continue;
        }
        if let Some(target) = world.get_entity(other) {
            world.trigger(Explode { target });
        }
    }
}

/// Bir mayına `Explode` olayını gönderir.
fn trigger_explode(world: &mut gizmo::core::World, entity: gizmo::core::entity::Entity) {
    world.trigger(Explode { target: entity });
}

/// `point`'i yarıçapı içine alan sağlam mayın.
fn mine_under(
    world: &gizmo::core::World,
    point: Vec3,
) -> Option<gizmo::core::entity::Entity> {
    let query = world.query::<&Mine>()?;
    for (id, mine) in query.iter() {
        if !mine.exploded && (mine.position - point).length() <= mine.radius {
            return world.get_entity(id);
        }
    }
    None
}

/// Merkeze en yakın sağlam mayın.
fn nearest_mine(
    world: &gizmo::core::World,
    point: Vec3,
) -> Option<gizmo::core::entity::Entity> {
    let query = world.query::<&Mine>()?;
    let mut best: Option<(f32, u32)> = None;
    for (id, mine) in query.iter() {
        if mine.exploded {
            continue;
        }
        let d = (mine.position - point).length();
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, id));
        }
    }
    let (_, id) = best?;
    world.get_entity(id)
}

/// Defteri sayaçlardan doldurur.
fn sync_report(world: &mut gizmo::core::World, fuse: &Fuse, advanced: bool) {
    let (mines, exploded) = world
        .query::<&Mine>()
        .map(|q| {
            let all: Vec<Mine> = q.iter().map(|(_, m)| *m).collect();
            (all.len(), all.iter().filter(|m| m.exploded).count())
        })
        .unwrap_or((0, 0));
    let bubbled = *fuse.bubbled.lock().expect("sayaç kilidi");
    let indexed = *fuse.indexed.lock().expect("sayaç kilidi");
    let replaced = *fuse.replaced.lock().expect("sayaç kilidi");
    let removed = *fuse.removed.lock().expect("sayaç kilidi");
    let immediate = world
        .get_resource::<ChainMode>()
        .map(|m| m.immediate)
        .unwrap_or(false);

    if let Some(mut report) = world.get_resource_mut::<Report>() {
        report.mines = mines;
        report.exploded = exploded;
        report.bubbled = bubbled;
        report.indexed = indexed;
        report.replaced = replaced;
        report.removed = removed;
        report.immediate = immediate;
        if advanced {
            report.rings += 1;
            if std::env::var("GIZMO_OBSERVER_SELFTEST").is_ok() {
                gizmo::gizmo_log!(
                    Info,
                    "halka {} · patlayan {}/{} · kabaran {} · Insert {} Replace {} Remove {}",
                    report.rings,
                    exploded,
                    mines,
                    bubbled,
                    indexed,
                    replaced,
                    removed
                );
                // Ölçülebilir bağ: bir mayın patlarken `Mine` üzerine `add_component` ile
                // yeniden yazılıyor, ve bu bir REPLACE. Yani sayaç patlayan sayısını takip
                // etmeli — etmiyorsa ya evre ayrımı yanlış ya da bir yazma kaçtı.
                debug_assert_eq!(
                    replaced as usize, exploded,
                    "Replace sayacı patlayan sayısından koptu"
                );
            }
        }
    }
}

/// Mayın tarlasını kurar: görünmez bir kök, altında `MINE_COUNT` mayın.
fn build_field(
    world: &mut gizmo::core::World,
    device: &gizmo::wgpu::Device,
    texture: &std::sync::Arc<gizmo::wgpu::BindGroup>,
    fuse: &Fuse,
) {
    use gizmo::core::hierarchy::HierarchyExt;

    // Tarlanın kökü. Olay buraya kadar kabarıyor.
    let field = world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        GlobalTransform::default(),
        Field,
    ));
    let bubbled = fuse.bubbled.clone();
    world.observe::<Explode, _>(field, move |_world, _on| {
        *bubbled.lock().expect("sayaç kilidi") += 1;
    });

    let sphere = AssetManager::create_sphere(device, 1.0, 14, 20);
    let mut rng = 0x2545_F491u32;
    let mut roll = |low: f32, high: f32| {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        low + (rng as f32 / u32::MAX as f32) * (high - low)
    };

    for _ in 0..MINE_COUNT {
        let position = Vec3::new(
            roll(-FIELD_HALF, FIELD_HALF),
            0.0,
            roll(-FIELD_HALF, FIELD_HALF),
        );
        let radius = roll(0.7, 1.6);
        let entity = world.spawn_bundle((
            Transform::new(position).with_scale(Vec3::splat(radius)),
            GlobalTransform::default(),
            sphere.clone(),
            Material::new(texture.clone()).with_pbr(ARMED, 0.5, 0.1),
            MeshRenderer::new(),
            Mine {
                position,
                radius,
                exploded: false,
            },
        ));
        world.add_child(field, entity);

        // Varlığa bağlı dinleyici (`observe`) — ve artık `&mut World` alıyor. İki kip de
        // buradan çıkıyor: kipi dünyadan okumak zaten yeni yeteneğin kendisi.
        let fired = fuse.fired.clone();
        world.observe::<Explode, _>(entity, move |world, on| {
            let immediate = world
                .get_resource::<ChainMode>()
                .map(|m| m.immediate)
                .unwrap_or(false);
            if !immediate {
                fired.lock().expect("fitil kilidi").push(on.entity.id());
                return;
            }
            explode_now(world, on.entity);
        });
    }
}

