//! # Bitişik dilimlerle sorgu gezinmesi
//!
//! Bileşenleri satır satır değil, **bitişik dilimler** hâlinde gezmek — SIMD ve önbellek dostu
//! toplu işler için.
//!
//! ## Motorda var, ve adı `iter_chunks`
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | bitişik dilim gezinmesi | [`Query::iter_chunks`] |
//! | değişken hâli | [`Query::iter_chunks_mut`] |
//! | dilim başına `&[T]` | var |
//! | filtreli sorguda ne olur | **panikliyor** — aşağıya bakın |
//!
//! ## Asıl bulgu: dilim = **arketip**, ve filtre dilime sığmıyor
//!
//! Gizmo'nun dilimi bir arketipin **tamamı**. Bunun iki sonucu var:
//!
//! 1. **Dilim sayısı arketip sayısıdır.** Aynı bileşeni taşıyan ama farklı bileşen kümeleri olan
//!    varlıklar ayrı dilimlere düşer. Demo bunu ölçüyor: aynı sayıda varlık, farklı arketip
//!    dağılımı, farklı dilim sayısı.
//!
//! 2. **Satır başına filtre isteyen sorgu panikliyor.** `Changed`/`Added`/`Or` ve SparseSet
//!    `With`/`Without` satır satır seçiyor; dilim ise arketipin tamamını veriyor. Motor bunu
//!    sessizce filtresiz sonuç döndürerek geçiştirmiyor — **gürültülü reddediyor**. Table
//!    depolu `With`/`Without` güvenli, çünkü orada arketip eşleşmesi yetiyor.
//!
//! İkincisi tasarım gereği ve doğru olan davranış.
//!
//! ## Ölçüldü
//!
//! 80 varlık, üç arketipe bölünmüş: 40'ı `GroupA`, 25'i `GroupB`, 15'i hiçbiri.
//! Ölçüldü (2026-08-23):
//!
//! | gezinme | dilimler | toplam |
//! |---------|----------|--------|
//! | `iter_chunks()`, filtresiz | **`[15, 40, 25]`** — tam üç arketip | 80 |
//! | `iter_chunks()` + Table `With<GroupA>` | **`[40]`** | 40 |
//! | `iter()`, satır satır | — | 80 |
//! | değer toplamı, dilim yolu | **3160** | |
//! | değer toplamı, satır yolu | **3160** | |
//!
//! Toplamlar birebir aynı (0..79'un toplamı = 3160), yani dilim yolu satır yoluyla aynı veriyi
//! veriyor. Ve dilim sayısı gerçekten arketip sayısı: aynı `Sample` bileşenini taşıyan 80 varlık,
//! yanlarında ne taşıdıklarına göre **üç** ayrı dilime düşüyor.
//!
//! ### Panik iddiası: tekrarlanmadı, denendi
//!
//! Belge "satır başına filtre isteyen sorguda panikler" diyor. Demo bunu yazıp geçmiyor —
//! `Changed<Sample>` taşıyan bir sorguda `iter_chunks()` çağırıp paniği yakalıyor. Sonuç:
//! **panikledi**. Yani koruma gerçekten orada, ve sessizce filtresiz sonuç dönmüyor.
//!
//! ### Küçük bir ayrıntı: dilim varlık id'lerini de taşıyor
//!
//! Gizmo'nun dilimi `(&[u32], &[T])` — bileşen dilimiyle **yan yana** varlık id'leri. Toplu
//! işlerken hangi satırın kime ait olduğunu kaybetmiyorsunuz; ayrı bir eşleme tutmak gerekmiyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::query::{Query, With, Without};
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Toplu işlenecek sayısal yük.
#[derive(Clone, Copy, Default)]
struct Sample(f32);
gizmo::core::impl_component!(Sample);

/// Arketipi bölen işaretler — Table depolu, yani dilim gezinmesiyle uyumlu.
#[derive(Clone, Copy)]
struct GroupA;
gizmo::core::impl_component!(GroupA);
#[derive(Clone, Copy)]
struct GroupB;
gizmo::core::impl_component!(GroupB);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct ChunkReport {
    frame: u32,
    /// Filtresiz gezinme: dilim sayısı ve her dilimin uzunluğu.
    all_chunks: Vec<usize>,
    all_total: usize,
    /// Table depolu `With` ile: çalışıyor.
    filtered_chunks: Vec<usize>,
    filtered_total: usize,
    /// Satır satır gezinmenin verdiği toplam — dilim toplamıyla eşleşmeli.
    row_total: usize,
    /// `Changed<Sample>` ile dilim gezinmesi denendiğinde ne oldu.
    changed_verdict: String,
    /// Dilimlerdeki değerlerin toplamı, satır satırın toplamıyla karşılaştırma için.
    chunk_sum: f32,
    row_sum: f32,
}
gizmo::core::impl_component!(ChunkReport);

/// Kaç varlık, ve hangi grupta.
const IN_A: usize = 40;
const IN_B: usize = 25;
const IN_NEITHER: usize = 15;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Contiguous Query", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            let total = IN_A + IN_B + IN_NEITHER;
            for i in 0..total {
                let column = i % 20;
                let row = i / 20;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(
                        (column as f32 - 9.5) * 0.55,
                        0.0,
                        (row as f32 - 2.0) * 0.55,
                    ))
                    .with_scale(Vec3::splat(0.22)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        if i < IN_A {
                            Vec4::new(0.90, 0.55, 0.25, 1.0)
                        } else if i < IN_A + IN_B {
                            Vec4::new(0.30, 0.65, 0.90, 1.0)
                        } else {
                            Vec4::new(0.55, 0.55, 0.60, 1.0)
                        },
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Sample(i as f32),
                ));
                // Arketipi üçe böl: A, B, hiçbiri.
                if i < IN_A {
                    scene.world.add_component(entity, GroupA);
                } else if i < IN_A + IN_B {
                    scene.world.add_component(entity, GroupB);
                }
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -0.5, 0.0)),
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
            scene.world.insert_resource(ChunkReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 5.5, 7.0), Vec3::ZERO);
        })
        .add_update_system(measure.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<ChunkReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("chunks".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(480.0);
                        ui.heading("Bitişik dilimlerle gezinme");
                        ui.label("Gizmo: iter_chunks");
                        ui.separator();
                        ui.monospace(format!(
                            "filtresiz : {} dilim {:?} · toplam {}",
                            r.all_chunks.len(),
                            r.all_chunks,
                            r.all_total
                        ));
                        ui.monospace(format!(
                            "With<A>   : {} dilim {:?} · toplam {}",
                            r.filtered_chunks.len(),
                            r.filtered_chunks,
                            r.filtered_total
                        ));
                        ui.monospace(format!("satır satır: {}", r.row_total));
                        ui.separator();
                        ui.label(format!("dilim toplamı : {:.0}", r.chunk_sum));
                        ui.label(format!("satır toplamı : {:.0}", r.row_sum));
                        if (r.chunk_sum - r.row_sum).abs() < 0.5 {
                            ui.label("-> iki yol aynı sonucu veriyor");
                        }
                        ui.separator();
                        ui.label("dilim = ARKETİP. Aynı bileşeni taşıyan varlıklar farklı");
                        ui.label("bileşen kümelerindeyse ayrı dilimlere düşüyor.");
                        ui.label("Changed/Added/Or ve sparse With/Without -> PANİK.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Üç gezinmeyi karşılaştırır: filtresiz dilim, Table-`With` dilim, satır satır.
fn measure(
    all: Query<(&Sample, Without<GroupA>, Without<GroupB>)>,
    group_a: Query<(&Sample, With<GroupA>)>,
    everything: Query<&Sample>,
    changed: Query<(&Sample, gizmo::core::query::Changed<Sample>)>,
    mut report: ResMut<ChunkReport>,
) {
    report.frame += 1;
    if report.frame > 2 {
        return; // arketipler oturdu; her karede yeniden ölçmenin anlamı yok
    }

    // 1. Filtresiz: her arketip bir dilim.
    let mut all_chunks = Vec::new();
    let mut chunk_sum = 0.0f32;
    // Dilim `(&[u32], &[Sample])` — varlık id'leri bileşen dilimiyle YAN YANA geliyor, yani
    // toplu işlerken hangi varlığa ait olduğunu kaybetmiyorsunuz.
    for (ids, samples) in everything.iter_chunks() {
        debug_assert_eq!(ids.len(), samples.len(), "id dilimi ile bileşen dilimi aynı boyda olmalı");
        all_chunks.push(samples.len());
        chunk_sum += samples.iter().map(|s| s.0).sum::<f32>();
    }
    report.all_total = all_chunks.iter().sum();
    report.all_chunks = all_chunks;
    report.chunk_sum = chunk_sum;

    // 2. Table depolu `With` — dilim gezinmesiyle uyumlu, çünkü arketip eşleşmesi yetiyor.
    let mut filtered_chunks = Vec::new();
    for chunk in group_a.iter_chunks() {
        filtered_chunks.push(chunk.0.len());
    }
    report.filtered_total = filtered_chunks.iter().sum();
    report.filtered_chunks = filtered_chunks;

    // 3. Satır satır — karşılaştırma tabanı.
    let mut row_total = 0usize;
    let mut row_sum = 0.0f32;
    for (_entity, sample) in everything.iter() {
        row_total += 1;
        row_sum += sample.0;
    }
    report.row_total = row_total;
    report.row_sum = row_sum;

    let _ = all.iter().count();

    // Belge "satır başına filtre isteyen sorguda panikler" diyor. Tekrarlamak yerine
    // DENİYORUZ: panik yakalanıyor, mesajı bastırılıyor, ve sonuç deftere yazılıyor.
    report.changed_verdict = probe_row_filter(&changed);

    gizmo::gizmo_log!(
        Info,
        "dilimler {:?} (toplam {}) · With<GroupA> {:?} (toplam {}) · satır satır {} · toplamlar {:.0} / {:.0} · Changed<Sample> dilimi: {}",
        report.all_chunks,
        report.all_total,
        report.filtered_chunks,
        report.filtered_total,
        report.row_total,
        report.chunk_sum,
        report.row_sum,
        report.changed_verdict
    );
}

/// `Changed<T>` taşıyan bir sorguda `iter_chunks` gerçekten panikliyor mu — deneyip görüyor.
///
/// Paniği yakalamak burada meşru: motorun kendi koyduğu, belgelenmiş bir koruma bu, bozulmuş bir
/// durum değil. Panik kancası ölçüm süresince susturuluyor ki günlük kirlenmesin.
fn probe_row_filter(
    query: &Query<'_, (&Sample, gizmo::core::query::Changed<Sample>)>,
) -> String {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        query.iter_chunks().count()
    }));
    std::panic::set_hook(previous);
    match outcome {
        Ok(n) => format!("PANİKLEMEDİ, {n} dilim döndü"),
        Err(_) => "panikledi (beklenen)".to_string(),
    }
}
