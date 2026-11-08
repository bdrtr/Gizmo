//! # ECS genişletme noktaları
//!
//! Bu demo **dört** ayrı yeteneği birden ele alıyor: kendi sistem parametreni yazmak, kendi sorgu
//! terimini yazmak, sistemleri boru hattına dizmek, bir sistemi çizelge dışından tek seferlik
//! koşturmak.
//!
//! 2026-08-23'teki ilk ölçümde dördünün de karşılığı yoktu: `SystemParam`, `WorldQuery` ve
//! `ReadOnlyQuery` mühürlüydü, `pipe` ve `run_system` yoktu, ve demo dördünün yerine geçen vekilleri
//! ölçüyordu. Bugün dördü de var:
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | kendi sistem parametreni yazmak | `unsafe impl SystemParam` (2026-11-08) — erişim beyanı sözleşmede |
//! | kendi sorgu terimini yazmak | `unsafe impl WorldQuery` + `ReadOnlyQuery` (2026-11-08) — yalnız `FetchComponent` mühürlü, o da `&T`/`&mut T`'nin iç yarısı |
//! | bir sistemin çıktısını ötekine vermek | `a.pipe(b)`, `b` ilk parametresinde `In<T>` alıyor (2026-11-07) |
//! | bir sistemi çizelge dışından bir kez koşturmak | `World::run_system(sistem)` (2026-11-05) |
//!
//! Neden `unsafe`: çizelge bir parametrenin ya da sorgu teriminin neye eriştiğini bilmek zorunda,
//! yoksa paralel çalıştırma güvenli olmaz. Dışarıda yazılan bir terim bunu kendisi beyan ediyor, ve
//! beyanın doğru olduğu sözünü `unsafe impl` veriyor — Bevy'deki gibi.
//!
//! ## Demoda
//!
//! - **`Count<T>`** — dünyadan `T` taşıyan varlıkları sayan bir sistem parametresi; `T`'yi okuduğunu
//!   beyan ediyor.
//! - **`Has<T>`** — satırda `T` olup olmadığını `bool` olarak veren sorgu terimi; bileşen belleğine
//!   dokunmadığı için hiçbir şey beyan etmiyor. Tek sorgu dönenleri de donmuşları da sayıyor.
//! - **`mean_x.pipe(store_mean)`** — dönen küplerin ortalama x'i bir sistemden ötekine.
//! - **`world.run_system(run_once)`** — kurulumda bir kez, sonra her boşlukta bir kez.
//!
//! ## Ölçüldü
//!
//! | ne | beklenen | ne söylüyor |
//! |----|----------|-------------|
//! | `Count<Spinner>` | 6 | parametre dünyayı okuyor |
//! | `Has<Frozen>`: dönen · donmuş | 3 · 3 | terim iki grubu tek sorguda ayırıyor |
//! | `pipe` ile ortalama x | (−4,0 + −0,8 + 2,4)/3 = **−0,800** | çıktı ötekinin girdisi |
//! | `run_system` koşu sayısı, boşluksuz | **1** | çizelgeye girmeden bir kez |
//!
//! ## Kontroller
//!   * **Boşluk** — tek seferlik sistemi yeniden koştur
//!   * **Sağ-tık + fare / WASDQE** — kamera

use std::marker::PhantomData;

use gizmo::core::archetype::sparse_set::ComponentSparseSet;
use gizmo::core::archetype::Archetype;
use gizmo::core::component::{Component, StorageType};
use gizmo::core::component_key::ComponentKey;
use gizmo::core::query::{Query, QueryData, ReadOnlyQuery, With, WorldQuery};
use gizmo::core::system::{AccessInfo, In, IntoSystemConfig, Phase, ResMut, SystemParam, SystemParamFetchError, SystemPipe};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

#[derive(Clone, Copy)]
struct Spinner;
gizmo::core::impl_component!(Spinner);
#[derive(Clone, Copy)]
struct Frozen;
gizmo::core::impl_component!(Frozen);

#[derive(Default, Clone, Copy)]
struct Counters {
    /// `Count<Spinner>`'ın verdiği.
    counted: usize,
    /// `Has<Frozen>` ile ayrılan iki grup.
    spinning: usize,
    frozen: usize,
    /// `pipe` ile gelen ortalama x.
    piped: f32,
    /// `run_system` ile koşan sistemin kaç kez koştuğu.
    once_ran: u32,
    frame: u32,
}
gizmo::core::impl_component!(Counters);

/// Dünyada `T` taşıyan varlık sayısı, sistem başlarken sayılmış.
struct Count<T>(usize, PhantomData<T>);

// SAFETY: `fetch` yalnız `T`'nin sütununu okuyor, `get_access_info` de bu okumayı beyan ediyor.
unsafe impl<T: Component> SystemParam for Count<T> {
    type Item<'w> = Count<T>;
    type State = ();

    fn fetch<'w>(world: &'w World, _dt: f32, _state: &'w mut ()) -> Result<Count<T>, SystemParamFetchError> {
        Ok(Count(world.query::<&T>().iter().count(), PhantomData))
    }

    fn get_access_info(info: &mut AccessInfo) {
        info.component_reads.push(ComponentKey::of::<T>());
    }
}

/// Satırda `T` var mı — değerini okumadan.
struct Has<T>(PhantomData<T>);

/// Bir satırın cevabı nerede: bütün arketip için belli, ya da sorulacak bir seyrek küme.
#[derive(Clone, Copy)]
enum HasFetch {
    Archetype(bool),
    Sparse(*const ComponentSparseSet),
}

// SAFETY: yalnız bir arketipin `T` sütunu olup olmadığını ve seyrek kümenin varlığı tutup tutmadığını
// okuyor, hiçbir bileşenin değerini ya da tikini değil — beyan edilecek erişim yok. Her arketipi ve
// her satırı kabul ediyor; `StaticType` kendisi; `bool` verdiği için salt okunur.
unsafe impl<T: Component> WorldQuery for Has<T> {
    type StaticType = Has<T>;
    type Fetch<'w> = HasFetch;
    type Item<'w> = bool;
    type Slice<'w> = ();

    unsafe fn fetch_raw<'w>(world: &'w World, arch: &Archetype, _tick: u32) -> Option<Self::Fetch<'w>> {
        let key = ComponentKey::of::<T>();
        Some(match T::storage_type() {
            StorageType::Table => HasFetch::Archetype(arch.has_component(key)),
            StorageType::SparseSet => world.sparse_set(key).map_or(HasFetch::Archetype(false), |set| HasFetch::Sparse(set)),
        })
    }
    fn check_aliasing(_types: &mut Vec<(ComponentKey, bool)>) {}
    fn matches_archetype(_arch: &Archetype) -> bool {
        true
    }
    unsafe fn get_item<'w>(fetch: Self::Fetch<'w>, _row: usize, entity_id: u32) -> Self::Item<'w> {
        match fetch {
            HasFetch::Archetype(has) => has,
            // SAFETY: küme dünyanın, sorgu boyunca ödünç alınmış.
            HasFetch::Sparse(set) => unsafe { (*set).contains(entity_id) },
        }
    }
    unsafe fn filter_row<'w>(_fetch: Self::Fetch<'w>, _row: usize, _entity_id: u32, _tick: u32) -> bool {
        true
    }
    unsafe fn get_slice<'w>(_fetch: Self::Fetch<'w>, _len: usize) -> Self::Slice<'w> {}
}

// SAFETY: `bool` veriyor, hiçbir yazma beyan etmiyor.
unsafe impl<T: Component> ReadOnlyQuery for Has<T> {}
impl<T: Component> QueryData for Has<T> {}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - ECS Extension Points", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Altı küp: üçü dönen, üçü donmuş. Filtreli sorgu ikisini ayıracak.
            for i in 0..6 {
                let x = (i as f32 - 2.5) * 1.6;
                let frozen = i % 2 == 1;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        if frozen {
                            Vec4::new(0.35, 0.55, 0.9, 1.0)
                        } else {
                            Vec4::new(0.9, 0.6, 0.3, 1.0)
                        },
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Spinner,
                ));
                if frozen {
                    scene.world.add_component(entity, Frozen);
                }
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.4, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.6),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(Counters::default());
            // Çizelgede olmayan bir sistem, şimdi ve bir kez.
            scene.world.run_system(run_once);
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.0), Vec3::ZERO);
        })
        .add_update_system(counted.in_phase(Phase::Update))
        .add_update_system(mean_x.pipe(store_mean).in_phase(Phase::Update))
        .add_update_hook(|world, _state, _dt, input| {
            if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
                world.run_system(run_once);
            }
        })
        .set_ui(|world, _state, ctx| {
            let Some(c) = world.get_resource::<Counters>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("ext".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("ECS genişletme noktaları");
                        ui.label("dördü de dışarıdan yazılabiliyor ya da var");
                        ui.separator();
                        ui.label(format!("Count<Spinner> (kendi parametremiz): {}", c.counted));
                        ui.label(format!(
                            "Has<Frozen> (kendi sorgu terimimiz): {} dönen · {} donmuş",
                            c.spinning, c.frozen
                        ));
                        ui.label(format!("mean_x.pipe(store_mean): {:.3}", c.piped));
                        ui.label(format!("run_system ile koşan sistem: {} kez", c.once_ran));
                        ui.separator();
                        ui.label("boşluk — run_system ile bir kez daha");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kendi parametremiz ve kendi sorgu terimimiz, yerleşik olanların yanında.
fn counted(spinners: Count<Spinner>, cubes: Query<Has<Frozen>, With<Spinner>>, mut counters: ResMut<Counters>) {
    counters.frame += 1;
    counters.counted = spinners.0;
    counters.frozen = cubes.iter().filter(|&frozen| frozen).count();
    counters.spinning = cubes.iter().count() - counters.frozen;
    if counters.frame == 120 {
        gizmo::gizmo_log!(
            Info,
            "kare 120 · Count {} · dönen {} · donmuş {} · pipe {:.3} · run_system {} kez",
            counters.counted,
            counters.spinning,
            counters.frozen,
            counters.piped,
            counters.once_ran
        );
    }
}

/// Boru hattının ilk ucu: dönen (donmamış) küplerin ortalama x'i.
fn mean_x(cubes: Query<(&Transform, Has<Frozen>), With<Spinner>>) -> f32 {
    let xs: Vec<f32> = cubes.iter().filter(|(_, frozen)| !frozen).map(|(t, _)| t.position.x).collect();
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f32>() / xs.len() as f32
    }
}

/// İkinci ucu: ilkinin döndürdüğü, `In` ile.
fn store_mean(In(mean): In<f32>, mut counters: ResMut<Counters>) {
    counters.piped = mean;
}

/// Çizelgede olmayan sistem — `World::run_system` ile koşuyor.
fn run_once(mut counters: ResMut<Counters>) {
    counters.once_ran += 1;
    gizmo::gizmo_log!(Info, "run_system: {}. koşu", counters.once_ran);
}
