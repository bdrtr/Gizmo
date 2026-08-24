//! # Varlığı devre dışı bırakmak
//!
//! Bir varlığı **yok etmeden devre dışı bırakmak**: yok sayılsın ama verisi dursun, sonra geri
//! açılabilsin. İşin püf noktası işaret bileşeni değil, **örtük filtre** — ve o filtre 2026-08-24'te
//! geldi.
//!
//! ## Ne var
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | devre dışı işaret bileşeni | kendi işaretinizi yazıyorsunuz — motor bir tane dayatmıyor |
//! | her **sistem** sorgusuna örtük filtre | `World::default_query_filters_mut().add::<Disabled>()` |
//! | örtük filtreyi tek bir sorgu için kapatmak | `IgnoreDefaultFilters` sorgu terimi |
//!
//! İşaret bileşenini motor dayatmıyor, çünkü filtre **genel**: kendi türünüzü kaydediyorsunuz,
//! birden fazla da olabiliyor.
//!
//! ## Nerede uygulanıyor — ve ilk tasarım burada YANLIŞTI
//!
//! **Sisteme parametre olarak bildirilen** sorgu filtreleniyor. Başka hiçbir şey: `World::query`,
//! `borrow`, `query_entity` — ve asıl önemlisi — `query_unchecked` / `borrow_mut_unchecked` değil.
//!
//! İlk uygulama `query_unchecked`'i filtreliyordu, çünkü parametre yolu oradan geçiyor. **İnceleme
//! bunu reddetti ve haklıydı:** o tek `pub unsafe fn` aynı zamanda motorun "paylaşımlıdan `&mut`"
//! kaçış kapısı — **37 dosyada 98 çağrı**, ve **dokuzu editör paneli, yedisi müfettiş** — yani
//! `&World` alan egui çizim fonksiyonları; sistemle uzaktan yakından ilgisi yok. Orada filtrelemek bir çizgi
//! çizmiyor, "paylaşımlı borç üstünden `&mut` isteyen" popülasyonu filtreliyordu. Yayınlanmadan
//! önce kodda izi sürülen dört sonuç:
//!
//!   * devre dışı bir varlık seçilince **editör müfettişi boşalıyordu** (ve sahne görünümündeki
//!     manipülatör de), yani onu geri açacak arayüz kalmıyordu;
//!   * **dönüşüm yayılımı alt ağaçları donduruyordu** — BFS çocukları ebeveynin `GlobalTransform`
//!     getirisinin `if let`'i İÇİNDE kuyruğa alıyor, yani devre dışı bir düğüm altındaki **açık**
//!     torunları da bayat dünya konumunda bırakıyordu;
//!   * `sync_bodies` devre dışı bir katı cismi duraklatmıyor, **yok ediyordu**: gelen listede
//!     olmayan cisim siliniyor, bileşen dizileri permüte oluyor, eklemler boşta kalıyor ve daha
//!     önceki geri-sarma anlık görüntüleri geçersizleşiyor. "Devre dışı" verinin DURMASI demekti;
//!   * ağ geri-sarması **filtresiz yakalayıp filtreli geri yüklüyordu** — bu bir desenkronizasyon.
//!
//! Yani sınır parametrenin kendisi, ve tek bir fonksiyon: `SystemParam for Query`'nin `fetch`'i.
//! Kapsadığı şey ölçüldü: `Query` parametresi bildiren **44** demo dosyası ve **3** motor dosyası.
//! Oyun kodu sistem yazar; motor dünyayı doğrudan okur.
//!
//! ## Bunun yapMADIĞI şey — açıkça
//!
//! Bir varlığı devre dışı bırakmak onu **sizin sistemlerinizin sorgularından** gizler. Motoru
//! **durdurmaz**: fizik onu simüle etmeye, dönüşüm yayılımı güncellemeye, ses çalmaya, geri-sarma
//! yakalamaya devam eder. Hepsi yukarıdaki kapıdan okuyor, ve yukarıdaki dört sonuç da öyle
//! kalmalarının sebebi. Devre dışı bir varlığın hareketini de durdurmak istiyorsanız onu siz
//! durduracaksınız — işaret bir **görüş**, bir **durum** değil.
//!
//! Bu demo kuralın ikinci yarısına dayanıyor: `Z` tuşunun işaret takıp söken kodu `set_update`
//! içinde ve `world.query::<&Spinner>()` kullanıyor. Filtrelenmiş olsaydı devre dışı küpleri
//! bulamaz, yani **geri açamazdı**.
//!
//! ## Ölçüm
//!
//! Bu demo eskiden iki sistem koşuyordu: biri `Without<Disabled>` yazmayı hatırlayan, öteki unutan
//! (2026-08-23 ölçümü: unutan 240. karede **543** fazladan işleme). Örtük filtreden sonra o hata
//! yazılamıyor — motorun kendi testi (`the_forgetful_system_can_no_longer_be_written`) aynı 240
//! kareyi koşup ikisinin farkının sıfır olduğunu iddia ediyor.
//!
//! HUD'daki sayı iki **farklı** görüşü karşılaştırıyor: `IgnoreDefaultFilters` yazan sorgu 9 küp,
//! sıradan sorgu 6. Buradaki eski sayaç aynı görüşü kendisiyle karşılaştırıyordu ve filtre bozuk
//! olsa da 0 gösterirdi — inceleme onu da yakaladı.
//!
//! ## Kontroller
//!   * **Z** — ortadaki üç küpü devre dışı bırak / geri aç
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{IgnoreDefaultFilters, Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Devre dışı işareti. Bileşen sizin; motor kaydettiğiniz türü **her sistem sorgusundan** düşürüyor.
///
/// `Table` depolamalı olmak zorunda (varsayılan): test arketip başına yapılıyor ve `SparseSet` bir
/// bileşen arketipte yaşamıyor. `DefaultQueryFilters::add` sparse bir işareti sessizce kabul edip
/// hiçbir şey filtrelemek yerine panikliyor.
#[derive(Clone, Copy)]
struct Disabled;
gizmo::core::impl_component!(Disabled);

/// Dönen küp.
#[derive(Clone, Copy)]
struct Spinner(f32);
gizmo::core::impl_component!(Spinner);

/// Ölçüm defteri.
#[derive(Default, Clone, Copy)]
struct DisableReport {
    frame: u32,
    disabled_now: bool,
    /// Filtreyi HATIRLAYAN sorgunun gördüğü varlık sayısı.
    filtered: usize,
    /// Filtreyi UNUTAN sorgunun gördüğü varlık sayısı.
    unfiltered: usize,
    /// İstisna yazan sorgu ile sıradan sorgunun gördüğü sayı arasındaki fark — yani filtrenin
    /// o karede gizlediği varlık sayısı. İki FARKLI görüşü karşılaştırıyor; aynı görüşü kendisiyle
    /// karşılaştıran bir sayaç filtre bozuk olsa da 0 gösterirdi.
    hidden_now: usize,
}
gizmo::core::impl_component!(DisableReport);

const TOTAL: usize = 9;
const DISABLE_FROM: usize = 3;
const DISABLE_TO: usize = 6;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Entity Disabling", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            for i in 0..TOTAL {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 4.0) * 1.25, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.80, 0.60, 0.30, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Spinner(i as f32 * 0.3),
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
            // İŞİN TAMAMI BU SATIR. Bundan sonra `Disabled` taşıyan varlık hiçbir sistem
            // sorgusunda görünmüyor — ne bu demonun sistemlerinde, ne motorun kendi fizik ve
            // dönüşüm sistemlerinde.
            scene.world.default_query_filters_mut().add::<Disabled>();
            scene.world.insert_resource(DisableReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.5), Vec3::ZERO);
        })
        // Devre dışı bırakma `set_update`'te: sorgu ham id veriyor ve onu `Entity`'ye güvenle
        // çevirmek `World::entity(id)` istiyor — çizelgeye kayıtlı bir sistem `&World` alamıyor.
        .set_update(|world, state, dt, input| {
            // Bkz. `demo::simple_scene_update` — `set_update` basit sahnenin kancasını eziyor.
            demo::simple_scene_update(world, state, dt, input);
            toggle(world, input);
        })
        // `.after("toggle")` yoktu: `toggle` bir sistem değil, `set_update` kancasının içinde bir
        // fonksiyon çağrısı — eşleşmeyen bir etiket zamanlayıcı tarafından uyarıyla DÜŞÜRÜLÜYOR.
        // İnceleme yakaladı; sıralama zaten kanca ile çizelge arasında, kaydın içinde değil.
        //
        // SIRADAN sistem: `Without<Disabled>` YAZMIYOR ve yine de devre dışı olanları görmüyor.
        .add_update_system(spin_filtered.in_phase(Phase::Update))
        // İSTİSNA: `IgnoreDefaultFilters` ile hepsini gören sistem — imzasında yazılı.
        .add_update_system(count_all.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<DisableReport>().map(|r| *r) else {
                return;
            };
            gizmo::egui::Area::new("dis".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(480.0);
                        ui.heading("Varlığı devre dışı bırakmak");
                        ui.label(format!(
                            "{} küp · ortadaki {} tanesi: {}",
                            TOTAL,
                            DISABLE_TO - DISABLE_FROM,
                            if r.disabled_now { "DEVRE DIŞI" } else { "açık" }
                        ));
                        ui.separator();
                        ui.monospace(format!("sıradan sistem          : {} varlık", r.filtered));
                        ui.monospace(format!("IgnoreDefaultFilters ile: {} varlık", r.unfiltered));
                        ui.separator();
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(140, 200, 140),
                            format!("filtrenin gizlediği: {} varlık", r.hidden_now),
                        );
                        ui.separator();
                        ui.label("filtre motorun içinden geliyor: unutulacak bir şey yok.");
                        ui.label("istisna imzada — IgnoreDefaultFilters.");
                        ui.label("Z'nin kodu set_update'te: dünyayı tutan kod filtresiz görür.");
                        ui.separator();
                        ui.label("Z — devre dışı bırak / geri aç");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// **D**: ortadaki küplere `Disabled` takar / söker.
fn toggle(world: &mut gizmo::core::World, input: &Input) {
    let Some(mut report) = world.get_resource_mut::<DisableReport>() else {
        return;
    };
    report.frame += 1;
    let frame = report.frame;
    let want = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyZ as u32)
        || (frame == 60 && std::env::var("GIZMO_DISABLE_SELFTEST").is_ok());
    if !want {
        return;
    }
    let now = !report.disabled_now;
    report.disabled_now = now;
    drop(report);

    let mut ids: Vec<u32> = world
        .query::<&Spinner>()
        .map(|q| q.iter().map(|(id, _)| id).collect())
        .unwrap_or_default();
    ids.sort_unstable();
    for (index, id) in ids.iter().enumerate() {
        if !(DISABLE_FROM..DISABLE_TO).contains(&index) {
            continue;
        }
        // Nesil kontrollü: uydurma yok.
        let Some(entity) = world.entity(*id) else { continue };
        if now {
            world.add_component(entity, Disabled);
        } else {
            world.remove_component::<Disabled>(entity);
        }
    }
}

/// Sıradan bir sistem — ve dikkat: `Without<Disabled>` **yazmıyor**.
///
/// Devre dışı küpleri görmemesinin sebebi bu imzada değil, dünyada kayıtlı filtre. Eskiden burada
/// `Without<Disabled>` vardı ve onu unutmak bir hataydı; şimdi unutulacak bir şey yok.
fn spin_filtered(
    mut spinners: Query<(Mut<Transform>, &Spinner, With<Spinner>)>,
    mut report: ResMut<DisableReport>,
    time: Res<Time>,
) {
    let dt = time.dt();
    let mut seen = 0usize;
    for (_entity, (mut transform, spinner, _)) in spinners.iter_mut() {
        seen += 1;
        // Her küpün kendi hızı — `Spinner`'ın yükü burada iş görüyor.
        transform.rotation *= Quat::from_rotation_y(dt * (0.6 + spinner.0));
    }
    report.filtered = seen;
}

/// İstisna: hepsini gören sistem.
///
/// `IgnoreDefaultFilters` dünyanın kayıtlı filtrelerini **bu sorgu için** kapatıyor, ve imzada
/// durduğu için sistemi okuyan biri görüşünün diğerlerinden farklı olduğunu satırdan çıkarabiliyor.
/// Devre dışı olanları geri açan sistem böyle yazılır — bu demoda o iş `set_update`'te, dünyayı
/// elinde tutan koddan yapılıyor, ki o zaten filtresiz.
///
/// Eskiden burada filtreyi "unutan" ikinci bir sistem vardı ve devre dışı olan her varlığı her
/// karede sayıyordu (240. karede 543 fazladan işleme). Örtük filtre geldikten sonra o sistem
/// **yazılamıyor**, ve o ölçüm motorun testine taşındı:
/// `the_forgetful_system_can_no_longer_be_written` aynı 240 karede farkın sıfır olduğunu ileri
/// sürüyor. Buradaki `hidden_now` onun yerine geçen ölçüm: iki FARKLI görüşün farkı.
fn count_all(
    all: Query<(&Spinner, With<Spinner>, IgnoreDefaultFilters)>,
    ordinary: Query<(&Spinner, With<Spinner>)>,
    mut report: ResMut<DisableReport>,
) {
    report.unfiltered = all.iter().count();
    // İki görüşün FARKI: istisnayı yazan sorgu ile sıradan sorgu arasındaki varlık sayısı. Bu,
    // devre dışı bırakılanların sayısı — ve sıfırdan büyük olması filtrenin ÇALIŞTIĞINI söylüyor.
    //
    // Burada eskiden `ordinary` ile `spin_filtered`'ın gördüğü sayı karşılaştırılıyordu ve o
    // ölçüm değersizdi: ikisi de filtreli sistem parametresi, yani fark yapıca sıfır — filtre
    // hiçbir şey yapmasa da sıfır çıkardı. İnceleme yakaladı (2026-08-24).
    report.hidden_now = report.unfiltered.saturating_sub(ordinary.iter().count());

    if std::env::var("GIZMO_DISABLE_SELFTEST").is_ok() && report.frame.is_multiple_of(60) {
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} · devre dışı: {} · sıradan sistem {} · IgnoreDefaultFilters {} · gizlenen {}",
            report.frame,
            report.disabled_now,
            report.filtered,
            report.unfiltered,
            report.hidden_now
        );
    }
}
