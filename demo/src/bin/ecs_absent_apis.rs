//! # Dört ECS yeteneği: dördü de var
//!
//! Bu demo dört ayrı yeteneği birden ele alıyor — değişmez bileşen, çalışma anında bileşen
//! tanımı, render dünyasına çıkarma, sistem sıcak yaması. Değişmez bileşen (2026-11-07),
//! çalışma anında tanım (2026-11-08), sıcak yama (2026-11-09) ve render dünyasına çıkarma
//! (2026-11-09, render thread'i 2026-11-10) artık Gizmo'da **var**.
//!
//! Ama "yok" tek başına bir bilgi değil, "var" da değil. Demo her biri için neyin olduğunu,
//! yerine ne konabileceğini ve ne kaybedildiğini ölçüyor.
//!
//! ## 1. Değişmez bileşen — VAR
//!
//! Bir bileşenin **değişmez** ilan edilebilmesi şu demek: `&mut` erişimi **derleme zamanında**
//! engellenir, tek değiştirme yolu yeniden takılmasıdır, ve bu sayede kancalar bileşenin tek
//! doğruluk kapısı hâline gelir.
//!
//! `impl_component!(Frozen; immutable)` bunu yapıyor: `query_mut::<&mut Frozen>()` derlenmiyor.
//! Demo dört varlığın `Frozen`'ını yeniden takarak değiştiriyor ve `on_set` kancasının her
//! değişimi duyduğunu sayıyor.
//!
//! ## 2. Çalışma anında bileşen tanımlamak — VAR
//!
//! Bir betik dilinden gelen bileşenleri çalışma anında tanımlayıp ham baytlarla okuyup yazmak.
//!
//! `World::register_component(ComponentDescriptor::new("Slot", Layout::new::<u32>()))` Rust türü
//! olmayan bir bileşen tanımlıyor ve bir `ComponentKey` döndürüyor; depolama, sorgular, kancalar
//! ve zamanlayıcı türleri de bu anahtarla tanıyor. Demo `Slot`'u dört varlığa `insert_by_key`
//! ile takıyor, bir `DynamicQuery` ile her birine 1 ekliyor ve toplamı okuyor. Bir zamanlayıcıda
//! koşacak hâli `DynamicSystem`.
//!
//! ## 3. Render dünyasına çıkarma — VAR
//!
//! Çıkarma iki dünya demek: oyun dünyası ve render dünyası, ve aradaki çıkarma adımı her kare
//! çizimin okuduğu veriyi kopyalar. Bu, render'ın oyun mantığıyla **paralel** koşabilmesini
//! sağlar.
//!
//! `RenderWorld` ikinci dünya: `render_extractor()`'ın saydığı bileşen ve kaynak türlerini, senkron
//! noktasında yalnız değişeni kopyalayarak tutuyor, ve çizim yalnız o kopyayı görüyor — render
//! thread'inde, bir sonraki karenin güncellemesiyle aynı anda. Oyunun kendi türü kopyada yok;
//! bir geçişin okuması gerekiyorsa `extractor_mut()` ile eklenir.
//!
//! Demo bunu kendi aşamasıyla (`Watched`) gösteriyor: senkron noktasında oyundaki ve kopyadaki
//! `Frozen` sayılıyor. Oyunda dört tane var; kopyada hiç yok, çünkü çizim `Frozen` okumuyor.
//!
//! ## 4. Çalışırken kod değiştirmek — VAR
//!
//! Sistem gövdelerini çalışırken değiştirmek. `hotpatching` özelliğiyle sistemler, koşullar,
//! `pipe` aşamaları, gözlemciler ve kancalar subsecond'ın atlama tablosundan çağrılıyor;
//! `dx serve --hot-patch` değişeni derleyip çalışan programa bağlıyor (`gizmo::core::hotpatch`).
//! Bu demo pencereli ve sürekli koşmadığı için onu `hot_systems` gösteriyor: bir sistemin
//! döndürdüğü cümle değiştirildi, 1,7 saniyede yeni cümle basıldı ve sayaç kaldığı yerden sürdü.
//!
//! Shader'lar ayrıca, kendi yollarıyla sıcak yükleniyor: her renderer shader'ı önce diskten bir
//! üstünü arıyor, ve stüdyo `demo/assets` altındaki `.wgsl` değişimlerinde boru hatlarını yeniden
//! kuruyor.
//!
//! ## Ölçüldü
//!
//! İlk ölçüm 2026-08-23'te, dördü de yokken yapıldı: `&mut Frozen` kurulabiliyordu ve değer
//! kancasız −1'e yazıldı; kayıt yalnız tipliydi. Bugün:
//!
//! | ne | ölçülen | ne söylüyor |
//! |----|---------|-------------|
//! | `&mut Frozen` | **derlenmiyor** | değişmezlik bir sözleşme |
//! | yeniden takmanın `on_set` sayısı | **4** | her değişim kapıdan geçiyor |
//! | çalışma anında tanımlanan bileşen | `Slot`, kayıt sayısı **+1** | tanım dinamik |
//! | `DynamicQuery` ile +1 sonrası toplam | 406 -> **410** | erişim de dinamik |
//! | çizimin gördüğü `Frozen` (kopyada) | **0** (oyunda 4) | çizim kopyayı görüyor, kopya çizimin okuduğunu taşıyor |
//! | `hot_systems`'de gövdesi değişen sistem | 1,7 s sonra yeni cümle, sayaç 5 → **6** | kod değişti, dünya yerinde |
//!
//! Son ikisi birer *katman*: çıkarmada ikinci dünya, sıcak yamada çalışma-anı kod değiştirme.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use std::alloc::Layout;
use std::sync::atomic::{AtomicUsize, Ordering};

use gizmo::app::RenderStage;
use gizmo::core::dynamic::{ComponentDescriptor, DynamicQuery};
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Değişmez bileşen: `&mut Frozen` derlenmiyor, tek değiştirme yolu yeniden takmak.
#[derive(Clone, Copy)]
struct Frozen(f32);
gizmo::core::impl_component!(Frozen; immutable);

/// `Frozen`'ın `on_set` kancasının kaç kez duyduğu.
static FROZEN_SETS: AtomicUsize = AtomicUsize::new(0);

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct AbsentReport {
    /// Yeniden takmanın ardından `Frozen`'ın değeri ve `on_set`'in kaç kez duyduğu.
    frozen_value: f32,
    frozen_sets: usize,
    /// Kayıtlı bileşen sayısı, `Slot` tanımlanmadan önce ve sonra.
    registered_before: usize,
    registered_after: usize,
    /// `Slot`'ların toplamı, `DynamicQuery` ile +1'den önce ve sonra.
    slot_sum_before: u32,
    slot_sum_after: u32,
    /// Senkron noktasında sayılan `Frozen`'lar: oyun dünyasında ve çizimin gördüğü kopyada.
    frozen_in_game: usize,
    frozen_in_copy: usize,
    /// Kopyanın taşıdığı tür sayısı (bileşen, kaynak ve varlık haritası).
    copied_types: usize,
}

/// Çizim oyun dünyasının kopyasından; senkron noktasında oyundaki ve kopyadaki `Frozen` sayılıp
/// rapora yazılıyor.
struct Watched {
    copy: RenderWorld,
}

impl RenderStage for Watched {
    fn prepare(&mut self, game: &mut World, renderer: &mut Renderer) {
        self.copy.prepare(game, renderer);
        let in_game = game.query::<&Frozen>().iter().count();
        let in_copy = self.copy.world().query::<&Frozen>().iter().count();
        let copied = self.copy.extractor_mut().copies().len();
        if let Some(mut report) = game.get_resource_mut::<AbsentReport>() {
            report.frozen_in_game = in_game;
            report.frozen_in_copy = in_copy;
            report.copied_types = copied;
        }
    }

    fn draw(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut Renderer) {
        self.copy.draw(encoder, view, renderer);
    }
}
gizmo::core::impl_component!(AbsentReport);

/// `Slot`'ların toplamı; her terimi `u32` olan bir sorgu ister.
fn slot_sum(query: &DynamicQuery, world: &World) -> u32 {
    let mut sum = 0;
    // SAFETY: 0. terim `Slot`, bir `u32`.
    query.iter(world).for_each(|row| sum += unsafe { *row.ptr(0).cast::<u32>() });
    sum
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - ECS Absent APIs", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // 2. Çalışma anında tanım: Rust türü olmayan bir bileşen, adı ve boyutuyla.
            let mut report = AbsentReport {
                registered_before: scene.world.registered_component_count(),
                ..Default::default()
            };
            let slot = scene
                .world
                .register_component(ComponentDescriptor::new("Slot", Layout::new::<u32>()))
                .expect("bu dünyada Slot adında başka bileşen yok");
            report.registered_after = scene.world.registered_component_count();

            scene.world.register_on_set::<Frozen>(|_, _| {
                FROZEN_SETS.fetch_add(1, Ordering::Relaxed);
            });
            let mut cubes = Vec::new();
            for i in 0..4 {
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.5) * 1.6, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.45, 0.70, 0.55, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Frozen(10.0 + i as f32),
                ));
                let mut value = 100 + i as u32;
                // SAFETY: hizalı bir `u32`, `Slot`'un düzeni; düz veri, sonra kullanılmıyor.
                unsafe { scene.world.insert_by_key(entity, slot, (&raw mut value).cast()) }
                    .expect("varlık canlı, Slot kayıtlı");
                cubes.push(entity);
            }

            // 1. Değişmez bileşen: `&mut Frozen` derlenmiyor; değiştirmek yeniden takmak demek,
            //    ve her takma `on_set`'ten geçiyor.
            FROZEN_SETS.store(0, Ordering::Relaxed);
            for &entity in &cubes {
                scene.world.add_component(entity, Frozen(-1.0));
            }
            report.frozen_sets = FROZEN_SETS.load(Ordering::Relaxed);
            report.frozen_value = scene.world.get::<Frozen>(cubes[0]).map_or(0.0, |f| f.0);

            // 2b. Dinamik erişim: `Slot`'u anahtarla okuyan ve yazan sorgular.
            let read = DynamicQuery::builder().read(slot).build(scene.world).expect("Slot kayıtlı");
            let bump = DynamicQuery::builder().write(slot).build(scene.world).expect("Slot kayıtlı");
            report.slot_sum_before = slot_sum(&read, scene.world);
            let mut rows = bump.iter_mut(scene.world);
            while let Some(mut row) = rows.fetch_next() {
                // SAFETY: 0. terim `Slot`, bir `u32`.
                unsafe { *row.ptr_mut(0).cast::<u32>() += 1 };
            }
            report.slot_sum_after = slot_sum(&read, scene.world);

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

            gizmo::gizmo_log!(
                Info,
                "Frozen yeniden takıldı: {} (on_set {} kez) · kayıt {} -> {} · Slot toplamı {} -> {}",
                report.frozen_value,
                report.frozen_sets,
                report.registered_before,
                report.registered_after,
                report.slot_sum_before,
                report.slot_sum_after
            );
            scene.world.insert_resource(report);
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO);
        })
        .add_update_system(noop.in_phase(Phase::Update))
        // 3. Çıkarma: çizim oyun dünyasının kopyasından, render thread'inde.
        .set_render_stage(Watched { copy: RenderWorld::new() })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<AbsentReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("absent".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(500.0);
                        ui.heading("Dört ECS yeteneği: dördü de var");
                        ui.separator();
                        ui.label("1. değişmez bileşen — VAR");
                        ui.monospace(format!(
                            "   &mut Frozen derlenmiyor · yeniden takıldı: {} · on_set {} kez",
                            r.frozen_value, r.frozen_sets
                        ));
                        ui.separator();
                        ui.label("2. çalışma anında bileşen tanımı — VAR");
                        ui.monospace(format!(
                            "   kayıtlı bileşen: {} -> {} (Slot)",
                            r.registered_before, r.registered_after
                        ));
                        ui.monospace(format!(
                            "   DynamicQuery ile +1: toplam {} -> {}",
                            r.slot_sum_before, r.slot_sum_after
                        ));
                        ui.separator();
                        ui.label("3. render dünyasına çıkarma — VAR (RenderWorld)");
                        ui.monospace(format!(
                            "   Frozen: oyunda {} · çizimin kopyasında {} · kopya {} tür taşıyor",
                            r.frozen_in_game, r.frozen_in_copy, r.copied_types
                        ));
                        ui.separator();
                        ui.label("4. sistem sıcak yaması — VAR (hotpatching + dx serve --hot-patch)");
                        ui.label("   gösterimi: hot_systems. Shader'lar da sıcak yükleniyor.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Çizelgenin boş kalmaması için — ölçümler kurulumda ve senkron noktasında yapılıyor.
fn noop(mut report: ResMut<AbsentReport>) {
    let _ = &mut *report;
}

/// `World`'ün bu demoda kullanılan yüzeyi tek yerde dursun diye.
#[allow(dead_code)]
fn _world_surface_note(_: &World) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The draw sees the copy, and the copy carries what the draw reads.** The game's four
    /// `Frozen`s are not copied — the draw reads no `Frozen` — while the transforms beside them are.
    #[test]
    fn the_copy_the_draw_sees_does_not_carry_the_games_own_component() {
        let mut game = World::new();
        for i in 0..4 {
            game.spawn_bundle((Transform::new(Vec3::new(i as f32, 0.0, 0.0)), Frozen(i as f32)));
        }
        let mut copy = RenderWorld::new();
        copy.extract(&mut game);
        assert_eq!(game.query::<&Frozen>().iter().count(), 4);
        assert_eq!(copy.world().query::<&Frozen>().iter().count(), 0, "the copy carries the game's own Frozen");
        assert_eq!(copy.world().query::<&Transform>().iter().count(), 4, "the copy lost the transforms the draw reads");
    }
}
