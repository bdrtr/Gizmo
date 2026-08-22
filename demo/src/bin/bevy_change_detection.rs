//! # Bevy'nin `ecs/change_detection` örneğinin Gizmo Engine portu
//!
//! Bir bileşen değiştiğinde ona **tepki vermek**: değişeni bulmak için bütün varlıkları taramak
//! yerine, sorgunun kendisine "yalnız değişenler" dedirtmek. Gizmo'da bunun karşılığı
//! [`Changed<T>`] ve [`Added<T>`] filtreleri.
//!
//! Sahnede sekiz küp var. Bir sistem her karede rastgele birkaçının değerini değiştiriyor;
//! ikinci bir sistem `Changed<Tracked>` ile **yalnız o karede değişenleri** buluyor ve onları
//! kırmızıya boyuyor. Boya her karede soluyor, yani ekranda parlayan küp = bu karede değişmiş küp.
//!
//! ## Motorun modeli, ve iki gerçek incelik
//!
//! **1. Pencere "son çalıştırmadan beri"dir, ve onu çizelge sürer.** `Schedule` her koşusunda
//! `world.begin_change_frame(last_run_tick)` çağırıp karşılaştırma referansını bir önceki
//! koşuya ayarlıyor. Bu yüzden `Changed<T>` "geçen kareden bu yana" demek — ve referans paralel
//! grupların ÖNÜNDE bir kez ayarlandığı için sistemler arasında yarış yok.
//!
//! **2. Değişiklik damgası `Mut<T>`'nin `deref_mut`'ında basılır, değeri karşılaştırarak değil.**
//! Yani aynı değeri geri yazmak da "değişti" sayılır. Bevy bunun için `set_if_neq` veriyor;
//! Gizmo'da böyle bir yardımcı yok, o yüzden bu demo koşulu **elle** kontrol ediyor —
//! `if new != current { *component = new }`. Aradaki fark ekranda görünür: kontrolsüz yazan bir
//! sistem her kareyi "değişti" diye işaretlerdi.
//!
//! Ve motorun kendi belgelediği tuzak da burada gösteriliyor: [`Mut::bypass_change_detection`]
//! damga basmadan yazar. Demonun üçüncü sistemi **sondaki küpü** hep o yoldan değiştiriyor —
//! değeri akıyor ama `Changed` onu hiç görmüyor, ve o küp ekranda **hiç parlamıyor**. Bir turdan
//! ölçülen HUD:
//!
//! ```text
//! toplam görülen değişiklik: 21
//! damgasız yazma (bypass):    2 kez — filtre hiçbirini görmedi
//! aynı değeri yazmaktan kaçınıldı: 149 kez
//! ```
//!
//! Son satır ikinci inceliğin ölçüsü: elle eşitlik kontrolü olmasa o 149 yazma da "değişiklik"
//! sayılır ve filtre gürültüye boğulurdu.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::query::{Changed, Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İzlenen değer — Bevy'nin `MyComponent(f32)`'si.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Tracked(f32);
gizmo::core::impl_component!(Tracked);

/// Değişiklik damgası basmadan yazılan küpün işareti.
#[derive(Clone, Copy)]
struct Sneaky;
gizmo::core::impl_component!(Sneaky);

/// Sayaçlar.
#[derive(Default, Clone, Copy)]
struct Counters {
    /// `Changed<Tracked>`'in bu karede bulduğu küp sayısı.
    seen_this_frame: usize,
    /// Filtrenin şimdiye kadar gördüğü toplam değişiklik.
    seen_total: u64,
    /// Damgasız yazılan (ve bu yüzden hiç görülmeyen) yazma sayısı.
    sneaky_writes: u64,
    /// Aynı değeri geri yazmanın önlendiği kaç kez oldu.
    skipped_equal: u64,
}
gizmo::core::impl_component!(Counters);

/// Küp sayısı.
const CUBES: usize = 8;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Change Detection", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);

            for i in 0..CUBES {
                let x = (i as f32 - (CUBES as f32 - 1.0) / 2.0) * 1.1;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.5, 0.0)).with_scale(Vec3::splat(0.4)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(COOL, 0.5, 0.0),
                    MeshRenderer::new(),
                    Tracked(0.0),
                ));
                // Sondaki küp hep damgasız yazılıyor — `Changed` onu hiç görmeyecek.
                if i == CUBES - 1 {
                    scene.world.add_component(entity, Sneaky);
                }
            }

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 12.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(Counters::default());
            scene.world.insert_resource(Rng(0x1357_9BDF));
            scene.spawn_camera(state, Vec3::new(0.0, 2.2, 6.5), Vec3::new(0.0, 0.5, 0.0));
        })
        .add_update_system(change_component.in_phase(Phase::Update))
        .add_update_system(change_without_stamping.in_phase(Phase::Update))
        // Tespit sistemi yazanlardan SONRA kaydediliyor; ama sıralamaya da gerek yok — pencere
        // "son çalıştırmadan beri" olduğu için bu karede yazılan her şey bu karede görünür.
        .add_update_system(change_detection.in_phase(Phase::Update))
        .add_update_system(fade_highlight.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(counters) = world.get_resource::<Counters>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("change".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Değişiklik tespiti");
                    ui.label(format!(
                        "bu karede Changed<Tracked>: {} küp",
                        counters.seen_this_frame
                    ));
                    ui.label(format!("toplam görülen değişiklik: {}", counters.seen_total));
                    ui.separator();
                    ui.label(format!(
                        "damgasız yazma (bypass): {} kez — filtre hiçbirini görmedi",
                        counters.sneaky_writes
                    ));
                    ui.label(format!(
                        "aynı değeri yazmaktan kaçınıldı: {} kez",
                        counters.skipped_equal
                    ));
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Değişmemiş küpün rengi.
const COOL: Vec4 = Vec4::new(0.25, 0.45, 0.85, 1.0);
/// Bu karede değişen küpün rengi.
const HOT: Vec4 = Vec4::new(1.0, 0.35, 0.15, 1.0);

/// Rastgeleliğin dört satırlık kaynağı — belirleyici olsun diye tohumlu.
#[derive(Clone, Copy)]
struct Rng(u32);
gizmo::core::impl_component!(Rng);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32
    }
}

/// 1. sistem: küplerin bir kısmının değerini değiştirir — ama **yalnız gerçekten farklıysa**.
///
/// Damga `deref_mut`'ta basıldığı için aynı değeri geri yazmak da "değişti" sayılırdı; Bevy'nin
/// `set_if_neq`'inin motordaki karşılığı, koşulu elle yazmak.
fn change_component(
    mut tracked: Query<(Mut<Tracked>, Without<Sneaky>)>,
    mut rng: ResMut<Rng>,
    mut counters: ResMut<Counters>,
    time: Res<Time>,
) {
    let now = (time.elapsed() as f32 * 2.0).round();
    for (_entity, (mut component, _)) in tracked.iter_mut() {
        if rng.next() > 0.08 {
            continue;
        }
        let new = Tracked(now);
        if *component == new {
            counters.skipped_equal += 1;
            continue;
        }
        *component = new;
    }
}

/// 2. sistem: aynı bileşeni **damga basmadan** yazar.
///
/// Motorun kendi belgesi bunu açıkça uyarıyor: bu yol "`Changed<T>`'ye tepki veren her sistemi
/// sessizce aç bırakır". Demoda bu, sondaki küpün hiç parlamaması demek.
fn change_without_stamping(
    mut sneaky: Query<(Mut<Tracked>, With<Sneaky>)>,
    mut counters: ResMut<Counters>,
    time: Res<Time>,
) {
    let now = (time.elapsed() as f32 * 2.0).round();
    for (_entity, (mut component, _)) in sneaky.iter_mut() {
        let target = component.bypass_change_detection();
        if target.0 != now {
            target.0 = now;
            counters.sneaky_writes += 1;
        }
    }
}

/// 3. sistem: **yalnız değişenleri** bulur ve onları boyar.
///
/// Sorgunun kendisi filtreliyor — sistem içinde "değişti mi" diye bir kontrol yok.
fn change_detection(
    mut changed: Query<(Mut<Material>, &Tracked, Changed<Tracked>)>,
    mut counters: ResMut<Counters>,
) {
    let mut seen = 0;
    for (_entity, (mut material, tracked, _)) in changed.iter_mut() {
        material.albedo = HOT;
        seen += 1;
        gizmo::gizmo_log!(Info, "değişiklik: Tracked({})", tracked.0);
    }
    counters.seen_this_frame = seen;
    counters.seen_total += seen as u64;
}

/// Boyayı her karede biraz söndürür — parlayan küp "bu karede değişti" demek olsun diye.
fn fade_highlight(mut materials: Query<(Mut<Material>, &Tracked)>, time: Res<Time>) {
    let k = (time.dt() * 12.0).min(1.0);
    for (_entity, (mut material, _)) in materials.iter_mut() {
        // Doğrudan alan yazmak damga basar; burada sorun değil, çünkü `Changed<Tracked>` izliyor,
        // `Changed<Material>` değil.
        let current = material.albedo;
        material.albedo = current + (COOL - current) * k;
    }
}
