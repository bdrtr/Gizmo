//! # Bevy'nin `ecs/parallel_query` örneğinin Gizmo Engine portu
//!
//! Aynı sorguyu iş parçacıklarına dağıtmak. Gizmo'da bunun adı [`Query::par_for_each_mut`]:
//! eşleşen arketipleri rayon'un iş-çalma havuzuna veriyor, satırlar tanım gereği ayrık olduğu
//! için kilit yok.
//!
//! Bevy'nin örneği 128 sprite'ı paralel taşıyor ve **kendi yorumunda** şunu söylüyor: *"128
//! eleman üzerinde toplama gibi ucuz bir işlem için paralel yineleyici normalinden hızlı
//! olmayacaktır."* Bu port o cümleyi **ölçüyor**: aynı iş yükü, **P** tuşuyla seri ↔ paralel,
//! ve iki geçişin süresi HUD'da yan yana. Küp sayısı da **+/−** ile değişiyor, çünkü cevabın
//! kendisi sayıya bağlı.
//!
//! ## Ölçüm (bu makine, release, 2026-08-23)
//!
//! `GIZMO_PAR_CUBES=<n> GIZMO_PAR_SELFTEST=1` ile, hareket geçişinin 120 karelik ortalaması:
//!
//! | küp    | seri     | paralel | hızlanma  |
//! |--------|----------|---------|-----------|
//! | 128    | 2,9 µs   | 22,3 µs | **×0,13** |
//! | 4 096  | 84,2 µs  | 87,8 µs | ×0,96     |
//! | 65 536 | 3 232 µs | 677 µs  | ×4,77     |
//!
//! Yani Bevy'nin uyarısı doğru, ve **ölçüsü de bu**: 128 elemanda paralel yineleme seri olandan
//! sekiz kat **yavaş**, çünkü havuzu uyandırmanın sabit bedeli (~20 µs) işin kendisinden büyük.
//! Başabaş noktası bu iş yükünde 4 096 civarında; kazanç ancak on binlerde açılıyor. Sayıyı
//! bilmeden `par_for_each_mut` yazmak, çoğu sahnede yavaşlatmak demektir.
//!
//! Sahne 2B değil: motorda sprite hattı yok (bilinen boşluk), o yüzden Bevy'nin ekranda
//! sekmelerinin karşılığı bir kutunun içinde sekip duran küpler.
//!
//! ## Ölçmenin üç tuzağı, üçü de demoda kapatıldı
//!
//!   * **Sistem süresi kare süresi değil.** Ölçülen şey yalnız hareket geçişinin kendisi
//!     (`Instant`), render ya da fizik değil.
//!   * **Tek kare bir ölçüm değil.** HUD 120 karelik kayan ortalama gösteriyor; tek kare, ilk
//!     kare ve önbellek ısınması gürültüsüyle doludur.
//!   * **Başsız koşuda P'ye basacak kimse yok.** `GIZMO_PAR_SELFTEST=1` kipi 240 karede bir
//!     çevirir ve o anki ortalamaları günlüğe basar; yukarıdaki tablo böyle çıktı.
//!
//! ## Paralelliğin arketip sınırı
//!
//! [`Query::par_for_each_mut`] önce **eşleşen arketipleri** rayon'a veriyor, sonra her arketibin
//! satırlarını da bölüyor (`into_par_iter`). İkinci kısım olmasaydı bu demo hiç hızlanmazdı:
//! bütün küpler tek bir arketipte (aynı bileşen kümesi), yani dağıtılacak tek bir iş parçası
//! olurdu. ×4,77 ölçümü satırların gerçekten bölündüğünün kanıtı.
//!
//! ## Kontroller
//!   * **P** — seri/paralel · **+ / −** — küp sayısını iki katına çıkar / yarıya indir
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Bevy'nin `Velocity`'si — burada üç boyutlu.
#[derive(Clone, Copy)]
struct Velocity(Vec3);
gizmo::core::impl_component!(Velocity);

/// Ölçüm defteri.
#[derive(Clone, Copy)]
struct Bench {
    parallel: bool,
    /// Son 120 karenin kayan ortalaması, mikrosaniye.
    serial_us: f32,
    parallel_us: f32,
    /// O anki küp sayısı.
    cubes: usize,
    /// Sayı değişimi istendi mi (mesh gerektiği için render kapanışında yapılır).
    resize: i32,
    /// Kaç kare koştu — kendi kendine ölçüm kipinin sayacı.
    frame: u32,
}

impl Default for Bench {
    fn default() -> Self {
        Self {
            parallel: true,
            serial_us: 0.0,
            parallel_us: 0.0,
            cubes: start_cubes(),
            resize: 0,
            frame: 0,
        }
    }
}

gizmo::core::impl_component!(Bench);

/// Küplerin ortak mesh'i ve dokusu — sayı değişince yeniden üretilmesin diye burada duruyor.
#[derive(Clone)]
struct Prototype {
    mesh: Mesh,
    texture: std::sync::Arc<gizmo::wgpu::BindGroup>,
}
gizmo::core::impl_component!(Prototype);

/// Başlangıçtaki küp sayısı — Bevy'nin 128'i az geliyor, ölçüm ancak burada anlam kazanıyor.
/// `GIZMO_PAR_CUBES` ile değiştirilebilir: cevap sayıya bağlı olduğu için ölçüm de öyle olmalı.
fn start_cubes() -> usize {
    std::env::var("GIZMO_PAR_CUBES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096)
}

/// Kendi kendine ölçüm: kip her bu kadar karede bir değişir ve o anki ortalamalar günlüğe düşer.
/// Başsız bir koşuda P tuşuna basacak kimse yok, ve iki sayıyı yan yana koymadan demo bir şey
/// söylemiyor.
const SELFTEST_PERIOD: u32 = 240;
/// Kutunun yarı boyu.
const BOX_HALF: f32 = 6.0;
/// Kayan ortalamanın penceresi.
const WINDOW: f32 = 120.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Parallel Query", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);
            let mut rng = Xorshift::new(0x9E37_79B9);

            for _ in 0..start_cubes() {
                spawn_cube(scene.world, &cube, &white, &mut rng);
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.5),
                intensity: 2.0,
                ..Default::default()
            });

            scene.world.insert_resource(Prototype {
                mesh: cube.clone(),
                texture: white.clone(),
            });
            scene.world.insert_resource(Bench::default());
            scene.spawn_camera(state, Vec3::new(0.0, 4.0, 18.0), Vec3::ZERO);
        })
        .add_update_system(move_and_bounce.in_phase(Phase::Update))
        .add_update_system(bench_input.in_phase(Phase::Update))
        // Küp eklemek mesh ister, mesh cihaz ister — bu yüzden sayı değişimi burada.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let resize = world.get_resource::<Bench>().map(|b| b.resize).unwrap_or(0);
            if resize != 0 {
                apply_resize(world, resize);
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(|world, _state, ctx| {
            let Some(bench) = world.get_resource::<Bench>().map(|b| *b) else {
                return;
            };
            gizmo::egui::Area::new("par".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Paralel sorgu");
                    ui.label(format!("küp: {}", bench.cubes));
                    ui.label(format!(
                        "kip: {}",
                        if bench.parallel {
                            "PARALEL (par_for_each_mut)"
                        } else {
                            "SERİ (iter_mut)"
                        }
                    ));
                    ui.separator();
                    ui.label("hareket geçişinin süresi (120 karelik ortalama):");
                    ui.label(format!("  seri   : {:8.1} µs", bench.serial_us));
                    ui.label(format!("  paralel: {:8.1} µs", bench.parallel_us));
                    if bench.serial_us > 0.0 && bench.parallel_us > 0.0 {
                        ui.label(format!(
                            "  hızlanma: ×{:.2}",
                            bench.serial_us / bench.parallel_us.max(0.001)
                        ));
                    } else {
                        ui.label("  (iki kipi de bir süre çalıştırın)");
                    }
                    ui.separator();
                    ui.label("P — kip · +/− — küp sayısı");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Hareket + sekme. Kip P ile değişiyor; ölçülen şey yalnız bu geçiş.
fn move_and_bounce(mut cubes: Query<(Mut<Transform>, Mut<Velocity>)>, mut bench: ResMut<Bench>) {
    let started = std::time::Instant::now();
    let parallel = bench.parallel;

    if parallel {
        // Kilitsiz: satırlar ayrık olduğu için her iş parçacığı kendi küpüne dokunuyor.
        cubes.par_for_each_mut(|(_entity, (mut transform, mut velocity))| {
            step(&mut transform, &mut velocity);
        });
    } else {
        for (_entity, (mut transform, mut velocity)) in cubes.iter_mut() {
            step(&mut transform, &mut velocity);
        }
    }

    let micros = started.elapsed().as_secs_f32() * 1_000_000.0;
    // Kayan ortalama: tek kare ölçüm değildir.
    if parallel {
        bench.parallel_us += (micros - bench.parallel_us) / WINDOW;
    } else {
        bench.serial_us += (micros - bench.serial_us) / WINDOW;
    }

    bench.frame += 1;
    if std::env::var("GIZMO_PAR_SELFTEST").is_ok() && bench.frame.is_multiple_of(SELFTEST_PERIOD) {
        gizmo::gizmo_log!(
            Info,
            "küp {} · seri {:.1} µs · paralel {:.1} µs · hızlanma ×{:.2}",
            bench.cubes,
            bench.serial_us,
            bench.parallel_us,
            bench.serial_us / bench.parallel_us.max(0.001)
        );
        bench.parallel = !bench.parallel;
    }
}

/// Bir küpün tek adımı: sabit bir yol boyunca ilerle, kutunun duvarında sek.
///
/// Bevy'nin `move_system` + `bounce_system`'i burada tek fonksiyon: paralel geçişte iki ayrı
/// sorgu açmak (biri okuyan biri yazan) aynı bileşene iki erişim demek olurdu.
fn step(transform: &mut Transform, velocity: &mut Velocity) {
    // Zaman adımı sabit: ölçüm kare süresine bağlı olmasın diye.
    transform.position += velocity.0 * (1.0 / 60.0);
    for axis in 0..3 {
        let p = transform.position[axis];
        if p.abs() > BOX_HALF {
            transform.position[axis] = BOX_HALF * p.signum();
            velocity.0[axis] = -velocity.0[axis];
        }
    }
}

/// P kipi değiştirir, +/− küp sayısını.
fn bench_input(mut bench: ResMut<Bench>, input: Res<Input>) {
    use gizmo::winit::keyboard::KeyCode;
    if input.is_key_just_pressed(KeyCode::KeyP as u32) {
        bench.parallel = !bench.parallel;
    }
    if input.is_key_just_pressed(KeyCode::Equal as u32)
        || input.is_key_just_pressed(KeyCode::NumpadAdd as u32)
    {
        bench.resize = 1;
    }
    if input.is_key_just_pressed(KeyCode::Minus as u32)
        || input.is_key_just_pressed(KeyCode::NumpadSubtract as u32)
    {
        bench.resize = -1;
    }
}

/// Küp sayısını iki katına çıkarır ya da yarıya indirir.
///
/// Mesh ve doku yeniden üretilmiyor: ikisi de sahne kurulurken bir kez yaratılıp [`Prototype`]
/// kaynağında duruyor. Aynı `Mesh`'i paylaşmak toplu çizimin (instancing) de şartı.
fn apply_resize(world: &mut World, direction: i32) {
    let current = world.get_resource::<Bench>().map(|b| b.cubes).unwrap_or(0);
    let target = if direction > 0 {
        (current * 2).min(131_072)
    } else {
        (current / 2).max(64)
    };

    if target > current {
        let Some(prototype) = world.get_resource::<Prototype>().map(|p| p.clone()) else {
            return;
        };
        let mut rng = Xorshift::new(target as u32 | 1);
        for _ in current..target {
            spawn_cube(world, &prototype.mesh, &prototype.texture, &mut rng);
        }
    } else {
        // Sorgu dünyayı ödünç alıyor; silmeden önce listeyi toplayıp bırakmak gerek.
        let doomed: Vec<u32> = match world.query::<&Velocity>() {
            Some(query) => query
                .iter()
                .map(|(entity, _)| entity)
                .take(current - target)
                .collect(),
            None => Vec::new(),
        };
        for entity in doomed {
            world.despawn_by_id(entity);
        }
    }

    if let Some(mut bench) = world.get_resource_mut::<Bench>() {
        bench.cubes = target;
        bench.resize = 0;
    }
}

/// Tek bir küp: kutunun içinde rastgele yer ve hız.
fn spawn_cube(
    world: &mut World,
    mesh: &Mesh,
    texture: &std::sync::Arc<gizmo::wgpu::BindGroup>,
    rng: &mut Xorshift,
) {
    let position = Vec3::new(
        rng.range(-BOX_HALF, BOX_HALF),
        rng.range(-BOX_HALF, BOX_HALF),
        rng.range(-BOX_HALF, BOX_HALF),
    );
    let velocity = Vec3::new(
        rng.range(-3.0, 3.0),
        rng.range(-3.0, 3.0),
        rng.range(-3.0, 3.0),
    );
    let albedo = Vec4::new(
        rng.range(0.2, 0.9),
        rng.range(0.3, 0.9),
        rng.range(0.4, 1.0),
        1.0,
    );
    world.spawn_bundle((
        Transform::new(position).with_scale(Vec3::splat(0.06)),
        GlobalTransform::default(),
        mesh.clone(),
        Material::new(texture.clone()).with_pbr(albedo, 0.5, 0.0),
        MeshRenderer::new(),
        Velocity(velocity),
    ));
}

/// Dört satırlık xorshift — dağılımın belirleyici olması yeter.
struct Xorshift(u32);

impl Xorshift {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        low + (self.0 as f32 / u32::MAX as f32) * (high - low)
    }
}
