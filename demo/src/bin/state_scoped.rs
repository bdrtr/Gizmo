//! # Duruma ait varlıklar
//!
//! Bir duruma **ait** varlıklar: menü açılınca doğan, menüden çıkınca kendiliğinden silinen
//! nesneler.
//!
//! ([`states`](../states/index.html) demosu `State`/`in_state`'in kendisini anlatıyor;
//! burada ondan farklı olan tek şeye bakılıyor: **varlık ömrünün duruma bağlanması**.)
//!
//! ## Motorda otomatik temizlik yok
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | durumdan çıkınca varlığı kendiliğinden silmek | **yok** |
//! | duruma girince varlığı kendiliğinden silmek | **yok** |
//! | duruma giriş/çıkışta koşan ayrı çizelgeler | **yok** |
//! | ilk duruma girişin de bildirilmesi | **bildirilmez** — bkz. aşağıdaki tuzak |
//! | durum kaynağı ve duruma göre sistem süzme | `State<S>` + `in_state` — **var** |
//!
//! Yani "durumdan çıkınca sil" davranışı elde kalıyor. Demo onu bir bileşen (`ScopedTo`) ve bir
//! sistemle kuruyor, sonra **çalışıp çalışmadığını sayıyor**.
//!
//! ## Tuzak: ilk duruma giriş hiç bildirilmiyor
//!
//! `State::new(initial)` makineyi zaten `initial` **içinde** başlatıyor ve ilk
//! `apply_transitions()` `false` dönüyor. Tipin kendi belgesi bunu yazıyor: *"any one-off setup
//! for `initial` has to be run by hand."*
//!
//! Bu, duruma ait varlıklar için tam olarak yanlış tarafa düşen bir tuzak: duruma girişe
//! bağladığınız kurulum, oyun `A` ile başladığında **hiç koşmuyor**. Demo bunu iki koşuda ölçüyor
//! — biri elle önyükleme yapıyor, öteki yapmıyor.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23, durum başına 3 nesne, 120 karede bir geçiş, ölçüm her durumun ortasında):
//!
//! | kare | durum | yaşayan | doğan | silinen | önyükleme |
//! |------|-------|---------|-------|---------|-----------|
//! | 60 | A | **3** | 3 | 0 | açık |
//! | 180 | B | 3 | 6 | 3 | açık |
//! | 300 | C(1) | 3 | 9 | 6 | açık |
//! | 420 | A | 3 | 12 | 9 | açık |
//! | 60 | A | **0** | **0** | 0 | **kapalı** |
//! | 180 | B | 3 | 3 | 0 | kapalı |
//! | 300 | C(1) | 3 | 6 | 3 | kapalı |
//! | 420 | A | 3 | 9 | 6 | kapalı |
//!
//! İki şey okunuyor:
//!
//! **Sızıntı yok.** `doğan − silinen` her satırda tam 3 — yani elle yazılan temizlik, durumdan
//! çıkanı gerçekten siliyor. Yaşayan sayı hiçbir durumda 3'ü aşmıyor.
//!
//! **Ama önyükleme kapatılınca ilk durum boş açılıyor**: 120 kare boyunca `A` sahnesinde
//! **sıfır** nesne var, ilk nesne ancak ilk geçişte doğuyor. Bu, "geçiş oldu mu" sinyaline
//! güvenen her kurulumun düşeceği tuzak.
//!
//! ## İkinci bulgu: kapsam yönetimi çizelgeye sığmıyor
//!
//! Sorgu ham `u32` id üretiyor. Onu `Entity`'ye çevirmenin güvenli yolu `World::entity(id)` — ve
//! o `&World` istiyor. Çizelgeye kayıtlı bir sistem `&World` alamıyor, ve `.exclusive()` bunu
//! **değiştirmiyor**: belgesinin kendi ifadesiyle o "bir *eşzamanlılık* bariyeri, yetki
//! yükseltmesi değil". Yani bu demonun temizlik işi `set_update` kancasında — motorun kendi
//! `LifetimeSystem`'i de aynı sebeple `query_unchecked` + `world.entity(id)` kullanıyor.
//!
//! (`Entity::new(id, 0)` uydurmak derleniyor ve yanlış: id geri dönüştürülmüşse nesil tutmaz.
//! `World::entity`'nin belgesi bunu "birkaç denetim hatasının kökü" diye anıyor.)
//!
//! ## Kontroller
//!   * Demo durumları kendi çeviriyor (2 saniyede bir A → B → C → A)
//!   * **1 / 2 / 3** — doğrudan A / B / C durumuna geç
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::state::State;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Demonun durumu — `C` yük taşıyor, yani durum bir enum değeri, bir etiket değil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GameState {
    A,
    B,
    C(u8),
}

impl GameState {
    fn label(&self) -> String {
        match self {
            GameState::A => "A".into(),
            GameState::B => "B".into(),
            GameState::C(n) => format!("C({n})"),
        }
    }
    fn colour(&self) -> Vec4 {
        match self {
            GameState::A => Vec4::new(0.90, 0.45, 0.25, 1.0),
            GameState::B => Vec4::new(0.25, 0.60, 0.90, 1.0),
            GameState::C(_) => Vec4::new(0.45, 0.85, 0.40, 1.0),
        }
    }
    fn next(&self) -> GameState {
        match self {
            GameState::A => GameState::B,
            GameState::B => GameState::C(1),
            GameState::C(_) => GameState::A,
        }
    }
}

/// Durum kapsamı, elle: bu varlık şu duruma ait, o durumdan çıkınca silinsin.
#[derive(Clone, Copy)]
struct ScopedTo(GameState);
gizmo::core::impl_component!(ScopedTo);

/// Ölçüm defteri.
#[derive(Clone)]
struct ScopeReport {
    state: GameState,
    /// Şu an yaşayan kapsamlı varlık sayısı — sızıntı olsa burada büyürdü.
    alive: usize,
    /// Kaç geçiş oldu.
    transitions: u32,
    /// Toplam doğan / silinen.
    spawned: u32,
    despawned: u32,
    /// İlk duruma giriş elle önyüklendi mi.
    bootstrapped: bool,
    /// Her durumda görülen en yüksek yaşayan sayısı — sızıntının göstergesi.
    peak_alive: usize,
    frame: u32,
}
gizmo::core::impl_component!(ScopeReport);

/// Bir duruma kaç nesne ait olacak.
const PER_STATE: usize = 3;
/// Kaç karede bir durum değişecek.
const PERIOD: u32 = 120;

/// Ortamdan: elle önyükleme yapılsın mı. Kapatınca ilk durumun nesneleri hiç doğmuyor.
fn bootstrap_enabled() -> bool {
    std::env::var("GIZMO_SCOPED_NO_BOOTSTRAP").is_err()
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - State Scoped", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            scene.world.insert_resource(Assets {
                cube: AssetManager::create_cube(device),
                white: white.clone(),
            });
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.2, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.65),
                intensity: 2.6,
                ..Default::default()
            });

            let initial = GameState::A;
            scene.world.insert_resource(State::new(initial));
            scene.world.insert_resource(ScopeReport {
                state: initial,
                alive: 0,
                transitions: 0,
                spawned: 0,
                despawned: 0,
                bootstrapped: bootstrap_enabled(),
                peak_alive: 0,
                frame: 0,
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.5), Vec3::ZERO);
        })
        // Durum makinesini süren şey: motorda böyle bir faz YOK, uygulama kendi çağırıyor.
        .add_update_system(drive_states.in_phase(Phase::PreUpdate).label("states"))
        // Kapsam yönetimi ÇİZELGEDE DEĞİL, `set_update`'te: sorgunun verdiği ham id'yi güvenle
        // `Entity`'ye çevirmek `&mut World` istiyor, ve çizelgeye kayıtlı hiçbir sistem onu
        // alamıyor (aşağıdaki başlık notuna bakın).
        .set_update(|world, state, dt, input| {
            // Bkz. `demo::simple_scene_update` — `set_update` basit sahnenin kancasını eziyor.
            demo::simple_scene_update(world, state, dt, input);
            scoped_lifecycle(world);
        })
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<ScopeReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("scoped".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Duruma ait varlıklar");
                        ui.label(format!("durum: {}", r.state.label()));
                        ui.label(format!("yaşayan kapsamlı varlık: {} (beklenen {})", r.alive, PER_STATE));
                        ui.label(format!("zirve: {} — sızıntı olsa burada büyürdü", r.peak_alive));
                        ui.separator();
                        ui.label(format!("geçiş: {} · doğan: {} · silinen: {}", r.transitions, r.spawned, r.despawned));
                        ui.label(format!(
                            "ilk duruma elle önyükleme: {}",
                            if r.bootstrapped { "AÇIK" } else { "KAPALI" }
                        ));
                        ui.separator();
                        ui.label("kendiliğinden durum temizliği motorda YOK.");
                        ui.label("temizliği yapan sistem bu demonun kendisi.");
                        ui.label("ve State::new ilk duruma girişi HİÇ bildirmiyor —");
                        ui.label("o kurulumu elle yapmak zorundasınız.");
                        ui.separator();
                        ui.label("1 / 2 / 3 — duruma atla");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Sahne varlıkları — her durumda yeniden doğurmak için elde tutuluyor.
#[derive(Clone)]
struct Assets {
    cube: Mesh,
    white: std::sync::Arc<gizmo::wgpu::BindGroup>,
}
gizmo::core::impl_component!(Assets);

/// Durumu ilerleten sistem. Motorda bunu yapan bir faz yok.
fn drive_states(
    mut state: ResMut<State<GameState>>,
    mut report: ResMut<ScopeReport>,
    input: Res<Input>,
) {
    report.frame += 1;

    use gizmo::winit::keyboard::KeyCode;
    if input.is_key_just_pressed(KeyCode::Digit1 as u32) {
        state.set(GameState::A);
    } else if input.is_key_just_pressed(KeyCode::Digit2 as u32) {
        state.set(GameState::B);
    } else if input.is_key_just_pressed(KeyCode::Digit3 as u32) {
        state.set(GameState::C(1));
    } else if report.frame.is_multiple_of(PERIOD) {
        let next = state.get().next();
        state.set(next);
    }

    // `apply_transitions` ertelenmiş geçişi uygular ve "oldu mu" bilgisini döndürür. Duruma
    // giriş/çıkış hakkında elde olan tek bilgi bu.
    if state.apply_transitions() {
        report.transitions += 1;
    }
    report.state = *state.get();
}

/// Durum temizliği, elle: durum değişince eskiye ait olanı sil, yeniye ait olanı doğur.
///
/// **Neden dışlayıcı (`set_update`) ve neden çizelgede değil:** sorgu ham `u32` id üretiyor, ve
/// onu `Entity`'ye çevirmenin güvenli tek yolu `World::entity(id)` — o da `&World` istiyor.
/// Çizelgeye kayıtlı bir sistem `&World`'ü parametre olarak alamıyor (`Query`/`Res` alıyor), ve
/// `.exclusive()` bunu değiştirmiyor: o bir **eşzamanlılık bariyeri**, yetki yükseltmesi değil —
/// `System::run` yine `&World` görüyor ama sisteme geçirilmiyor. Motorun kendi
/// `LifetimeSystem`'i de aynı yerden geçiyor: `query_unchecked` + `world.entity(id)`.
///
/// `Entity::new(id, 0)` uydurmak çalışıyor gibi görünür ve yanlıştır — id geri dönüştürülmüşse
/// nesil tutmaz. `World::entity`'nin belgesi bunu "birkaç denetim hatasının kökü" diye anıyor.
fn scoped_lifecycle(world: &mut World) {
    let Some(current) = world.get_resource::<ScopeReport>().map(|r| r.state) else {
        return;
    };

    // 1. Çıkış temizliği: bu duruma ait OLMAYAN her kapsamlı varlığı sil.
    let mut alive = 0usize;
    let mut doomed: Vec<gizmo::core::Entity> = Vec::new();
    if let Some(q) = world.query::<&ScopedTo>() {
        for (id, scope) in q.iter() {
            if scope.0 == current {
                alive += 1;
            } else {
                // Ham id -> Entity, nesil kontrollü. `Entity::new(id, 0)` DEĞİL.
                if let Some(entity) = world.entity(id) {
                    doomed.push(entity);
                }
            }
        }
    }
    let removed = doomed.len() as u32;
    for entity in doomed {
        world.despawn(entity);
    }

    // 2. Giriş kurulumu. İlk karede de koşuyor — çünkü `State::new` ilk duruma girişi
    //    BİLDİRMİYOR, yani "geçiş oldu" sinyalini beklemek ilk durumu boş bırakırdı.
    let (bootstrapped, transitions) = world
        .get_resource::<ScopeReport>()
        .map(|r| (r.bootstrapped, r.transitions))
        .unwrap_or((true, 0));
    let mut spawned = 0u32;
    if alive == 0 && (bootstrapped || transitions > 0) {
        let assets = world.get_resource::<Assets>().map(|a| a.clone());
        if let Some(assets) = assets {
            for i in 0..PER_STATE {
                let x = (i as f32 - (PER_STATE as f32 - 1.0) * 0.5) * 1.8;
                world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.6)),
                    GlobalTransform::default(),
                    assets.cube.clone(),
                    Material::new(assets.white.clone()).with_pbr(current.colour(), 0.5, 0.0),
                    MeshRenderer::new(),
                    ScopedTo(current),
                ));
            }
            spawned = PER_STATE as u32;
        }
    }

    let Some(mut report) = world.get_resource_mut::<ScopeReport>() else {
        return;
    };
    report.despawned += removed;
    report.spawned += spawned;
    report.alive = alive;
    report.peak_alive = report.peak_alive.max(alive);

    if std::env::var("GIZMO_SCOPED_SELFTEST").is_ok() && (report.frame + PERIOD / 2).is_multiple_of(PERIOD) {
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} · durum {:<5} · yaşayan {} · zirve {} · geçiş {} · doğan {} · silinen {} · önyükleme {}",
            report.frame,
            current.label(),
            report.alive,
            report.peak_alive,
            report.transitions,
            report.spawned,
            report.despawned,
            report.bootstrapped
        );
    }
}
