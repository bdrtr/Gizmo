//! # Kare kancaları: ezmek ve eklemek
//!
//! Bir uygulamanın her karede kendi işini yapmasının yolu `App::set_update`. Adı "set" — ve
//! gerçekten *set*: kancayı **değiştiriyor**, zincirlemiyor.
//!
//! ## Tuzağın kendisi
//!
//! `with_simple_scene` kendi kancasını kuruyor, ve o kanca her karede dört iş yapıyor:
//!
//!   1. fare/WASD'yi kamera pozuna çeviriyor (`SimpleSceneState::fly_step`),
//!   2. o pozu sahnenin kamerasına yazıyor,
//!   3. CPU fiziğini ≤16 ms dilimlerle ilerletiyor,
//!   4. `TransformSyncSystem` ve `TransformPropagateSystem`'i koşturuyor.
//!
//! Yani `.with_simple_scene(..).set_update(..)` — kendi işi için `&mut World` isteyen bir
//! uygulamanın yazacağı en doğal şey — **dördünü birden sessizce atıyor**. Derleme geçiyor, kare
//! çiziliyor; yalnız kamera fareye cevap vermez ve `GlobalTransform` yayılmaz oluyor. Bu depodaki
//! altı demoda tam olarak böyle bulundu (2026-08-23).
//!
//! ## Üç kip, aynı sahne
//!
//! | kip | nasıl kuruluyor |
//! |-----|-----------------|
//! | `trap` | `.with_simple_scene(..).set_update(mine)` — tuzağın kendisi |
//! | `chained` | `.set_update(\|..\| { simple_scene_update(..); mine(..) })` — elle zincir |
//! | `added` | `.with_simple_scene(..).add_update_hook(mine)` — **eklemeli biçim** (2026-08-23) |
//!
//! ## Ölçülen: iki iddia, ikisi de sınandı, biri düştü
//!
//! Sahne üç parça: dönen bir **ebeveyn** küp, ona bağlı bir **çocuk** küp (yerel `Transform`'una
//! hiç dokunulmuyor — dünyadaki yeri yalnız yayılım koşarsa değişir), ve serbest düşen bir
//! **küp**. Ebeveyni döndüren bu demonun kendi kancası, yani üç kipte de koşuyor.
//!
//! Ölçüldü (2026-08-23, 300 kare, `GIZMO_HOOK_MODE=<kip>`):
//!
//! | kip | çocuğun yolu | beklenen yay | tek karede en büyük adım | düşenin indiği |
//! |-----|--------------|--------------|--------------------------|----------------|
//! | `trap` | 6,042 | 3,565 | **2,5000** | **0,000** |
//! | `chained` | 3,573 | 3,584 | 0,0425 | 6,962 |
//! | `added` | 3,536 | 3,549 | 0,0294 | 6,817 |
//!
//! Yarıçap üç kipte de 2,500..2,500 — yani çocuk her kipte düzgün bir daire çiziyor, yayılım
//! hiçbir kipte bozulmuyor.
//!
//! ### Fizik gerçekten duruyor
//!
//! `trap` kipinde düşen küp 300 karede **sıfır** birim iniyor. Öteki iki kipte ~6,9. Yani
//! `set_update`'in yuttuğu işlerden biri gerçekten kayboluyor ve sessizce kayboluyor: sahne
//! çiziliyor, kare akıyor, cisim havada duruyor.
//!
//! ### Yayılım durmuyor — bu iddia yanlıştı
//!
//! Beklentim `trap` kipinde çocuğun yolunun **sıfır** çıkmasıydı, çünkü bu depodaki
//! dokümantasyon (`demo::simple_scene_update` ve `App::set_update`) "`GlobalTransform` yayılmaz
//! olur" diyordu. Ölçüm bunu çürüttü: yol sıfır değil, **fazla** — 6,042, beklenen yayın
//! neredeyse iki katı.
//!
//! Fark tek bir sayıda toplanıyor: `6,042 − 3,565 = 2,477`, ve tek karede atılan en büyük adım
//! **2,5000**. Yani fazlalık tek bir sıçrama, ve büyüklüğü tam olarak çocuğun ebeveyne uzaklığı.
//! Bu, ilk karede `GlobalTransform`'un hâlâ `Mat4::IDENTITY` olması: kanca `(0,0,0)` okuyor,
//! ertesi kare `(2,5, 0, 0)` okuyor.
//!
//! Sebebi de basit: `TransformPropagateSystem` **üç** yerde koşuyor — `simple_scene_update`
//! içinde, `TransformPlugin` ile çizelgede, ve her karede `ensure_global_transforms` içinde
//! çizimden hemen önce. `set_update` bunlardan yalnız birincisini yutuyor. Kalan ikisi işi
//! yapmayı sürdürüyor; kaybolan tek şey, kullanıcının kancasının **kendi karesinde** güncel bir
//! `GlobalTransform` görmesi.
//!
//! Yani tuzağın bedeli "yayılım durur" değil, "yayılım bir kare geriden gelir". İkisi çok farklı
//! şeyler, ve aradaki farkı ancak ölçüm söylüyor.
//!
//! Kamera tarafı bu koşuda ölçülemedi: `fly_step` girdiden besleniyor, penceresiz koşuda girdi
//! yok. `simple_scene_update`'in tek sahibi olduğu iş o — `active_camera`'ya poz yazan başka
//! hiçbir yer yok — ama bu satır kod okumasına dayanıyor, ölçüme değil.
//!
//! ## Kontroller
//!   * `GIZMO_HOOK_MODE=trap|chained|added` — kipi seç (öntanımlı `added`)
//!   * `GIZMO_HOOK_SELFTEST=1` — 60 karede bir ölçümü konsola yaz
//!   * **Sağ-tık + fare / WASDQE** — kamera (yalnız `chained` ve `added` kiplerinde çalışır)

use gizmo::core::input::Input;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hangi kip koşuyor.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// `set_update` basit sahnenin kancasını eziyor.
    Trap,
    /// Kanca elle zincirleniyor.
    Chained,
    /// `add_update_hook` — eklemeli.
    Added,
}

fn mode() -> Mode {
    match std::env::var("GIZMO_HOOK_MODE").as_deref() {
        Ok("trap") => Mode::Trap,
        Ok("chained") => Mode::Chained,
        _ => Mode::Added,
    }
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Trap => "trap",
            Mode::Chained => "chained",
            Mode::Added => "added",
        }
    }
}

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct HookReport {
    /// Bu demonun kendi kancasının koştuğu kare sayısı.
    my_frames: u32,
    /// Ebeveynin toplam dönüşü, radyan.
    parent_spin: f32,
    /// Çocuğun dünyada kat ettiği toplam yol. Yayılım koşmazsa 0 kalır.
    child_path: f32,
    /// Çocuğun bir önceki karedeki dünya konumu.
    last_child: Option<Vec3>,
    /// Çocuğun ebeveyne uzaklığının en küçüğü ve en büyüğü. Daire çiziyorsa ikisi de 2,5.
    radius_min: f32,
    radius_max: f32,
    /// Tek karede atılan en büyük adım. Düzgün dönüşte ~2,5·SPIN·dt olmalı.
    max_step: f32,
    /// Serbest düşen küpün başlangıçtan itibaren indiği yükseklik.
    fall: f32,
}
gizmo::core::impl_component!(HookReport);

/// Ebeveyn ve çocuğun kimlikleri — kancanın her karede bulması için.
#[derive(Clone, Copy)]
struct Pair {
    parent: u32,
    child: u32,
    /// Serbest düşen küp — CPU fiziğinin gerçekten durup durmadığını ölçüyor.
    faller: u32,
    /// Düşenin başlangıç yüksekliği.
    faller_y0: f32,
}
gizmo::core::impl_component!(Pair);

/// Ebeveyni döndüren hız, radyan/saniye.
const SPIN: f32 = 1.2;

/// Bu demonun kendi kare işi. Üç kipte de aynı — değişen yalnız motorun işinin yanında koşup
/// koşmadığı.
fn my_hook(world: &mut World, _state: &mut SimpleSceneState, dt: f32, _input: &Input) {
    let Some(pair) = world.get_resource::<Pair>().map(|p| *p) else {
        return;
    };
    let Some(mut report) = world.get_resource::<HookReport>().map(|r| r.clone()) else {
        return;
    };

    report.my_frames += 1;
    report.parent_spin += SPIN * dt;

    // Ebeveynin YEREL dönüşünü yaz. Çocuğa dokunma.
    //
    // Sorgu üzerinden, çünkü motorda "şu varlığın şu bileşenini ver" diye tekil bir erişimci yok:
    // `World`'ün sunduğu tipli yol `query`/`query_mut`, ve tek varlık isteyen de bütünü gezip
    // kimliği karşılaştırıyor. Demonun konusu bu değil, ama demoyu yazarken çıktı.
    if let Some(mut q) = world.query_mut::<gizmo::core::query::Mut<Transform>>() {
        for (id, mut t) in q.iter_mut() {
            if id == pair.parent {
                t.rotation = Quat::from_rotation_y(report.parent_spin);
            }
        }
    }

    // Çocuğun DÜNYA konumunu oku — yalnız yayılım koştuysa değişir.
    let mut child_now = None;
    if let Some(q) = world.query::<&GlobalTransform>() {
        for (id, g) in q.iter() {
            if id == pair.child {
                child_now = Some(g.compute_matrix().w_axis.truncate());
            }
        }
    }
    if let Some(here) = child_now {
        if let Some(prev) = report.last_child {
            let step = (here - prev).length();
            report.child_path += step;
            report.max_step = report.max_step.max(step);
        }
        report.last_child = Some(here);
        let r = here.length(); // ebeveyn merkezde
        if report.radius_max == 0.0 {
            report.radius_min = r;
        }
        report.radius_min = report.radius_min.min(r);
        report.radius_max = report.radius_max.max(r);
    }

    // Düşenin ne kadar indiği. `simple_scene_update` fiziği elle ilerletiyor, ama
    // `PhysicsPlugin` de çizelgeye bir adım koyuyor — yani "tuzakta fizik durur" bir iddia, ve
    // iddialar ölçülür.
    let mut fall_y = None;
    if let Some(q) = world.query::<&Transform>() {
        for (id, t) in q.iter() {
            if id == pair.faller {
                fall_y = Some(t.position.y);
            }
        }
    }
    if let Some(y) = fall_y {
        report.fall = pair.faller_y0 - y;
    }

    if std::env::var("GIZMO_HOOK_SELFTEST").is_ok() && report.my_frames.is_multiple_of(60) {
        gizmo::gizmo_log!(
            Info,
            "kip {:8} · kare {:>4} · dönüş {:.3} rad · yol {:.3} (yay {:.3}) · yarıçap \
             {:.3}..{:.3} · en büyük adım {:.4}",
            mode().name(),
            report.my_frames,
            report.parent_spin,
            report.child_path,
            2.5 * report.parent_spin,
            report.radius_min,
            report.radius_max,
            report.max_step
        );
        gizmo::gizmo_log!(
            Info,
            "kip {:8} · kare {:>4} · düşenin indiği yükseklik {:.4}",
            mode().name(),
            report.my_frames,
            report.fall
        );
    }

    world.insert_resource(report);
}

fn main() {
    let m = mode();

    let app = App::<SimpleSceneState>::new("Gizmo Engine - Update Hooks", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Ebeveyn: başlangıçta merkezde, her kare dönüyor.
            let parent = scene.world.spawn();
            scene.world.add_component(parent, Transform::default());
            scene.world.add_component(parent, GlobalTransform::default());
            scene.world.add_component(parent, cube.clone());
            scene.world.add_component(
                parent,
                Material::new(white.clone()).with_pbr(Vec4::new(0.85, 0.45, 0.20, 1.0), 0.5, 0.0),
            );
            scene.world.add_component(parent, MeshRenderer::new());

            // Çocuk: ebeveynden 2,5 birim ötede, YEREL dönüşümü hiç değişmiyor.
            let child = scene.world.spawn();
            scene.world.add_component(
                child,
                Transform::new(Vec3::new(2.5, 0.0, 0.0)).with_scale(Vec3::splat(0.6)),
            );
            scene.world.add_component(child, GlobalTransform::default());
            scene
                .world
                .add_component(child, gizmo::core::component::Parent(parent.id()));
            scene.world.add_component(child, cube);
            scene.world.add_component(
                child,
                Material::new(white.clone()).with_pbr(Vec4::new(0.25, 0.60, 0.95, 1.0), 0.4, 0.0),
            );
            scene.world.add_component(child, MeshRenderer::new());

            // Kenarın İKİ ucu da yazılmalı. `TransformPropagateSystem` köklerden `Children`
            // üzerinden iniyor; `Parent` yalnız çocuğu kök sorgusunun dışında tutuyor. Yalnız
            // `Parent` yazmak sessizce hiçbir şey yapmıyor — aşağıdaki ölçüm notuna bakın.
            scene
                .world
                .add_component(parent, gizmo::core::component::Children(vec![child.id()]));

            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 24.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.6,
                ..Default::default()
            });

            // Serbest düşen küp: yalnız yerçekimi, çarpacak bir şey yok.
            const FALLER_Y: f32 = 6.0;
            let faller = scene.world.spawn_bundle((
                Transform::new(Vec3::new(-4.5, FALLER_Y, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                AssetManager::create_cube(device),
                Material::new(white.clone()).with_pbr(Vec4::new(0.75, 0.75, 0.30, 1.0), 0.4, 0.0),
                MeshRenderer::new(),
                Collider::box_collider(Vec3::splat(0.25)),
                RigidBody::new(1.0, true),
                Velocity::default(),
            ));

            scene.world.insert_resource(Pair {
                parent: parent.id(),
                child: child.id(),
                faller: faller.id(),
                faller_y0: FALLER_Y,
            });
            scene.world.insert_resource(HookReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 3.5, 9.0), Vec3::ZERO);
        });

    let app = match m {
        // Tuzak: basit sahnenin kancası burada yok oluyor.
        Mode::Trap => app.set_update(my_hook),
        // Elle zincir: `add_update_hook` gelmeden önce tek doğru cevap buydu.
        Mode::Chained => app.set_update(|world, state, dt, input| {
            demo::simple_scene_update(world, state, dt, input);
            my_hook(world, state, dt, input);
        }),
        // Eklemeli: motorun kancası duruyor, benimki ardına geliyor.
        Mode::Added => app.add_update_hook(my_hook),
    };

    app.set_ui(move |world, _state, ctx| {
        let r = world
            .get_resource::<HookReport>()
            .map(|r| r.clone())
            .unwrap_or_default();
        gizmo::egui::Area::new("hooks".into())
            .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
            .show(ctx, |ui| {
                gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(460.0);
                    ui.heading("Kare kancaları");
                    ui.label(format!("kip: {}", m.name()));
                    ui.separator();
                    ui.label(format!("kendi kancam: {} kare koştu", r.my_frames));
                    ui.label(format!("ebeveyn dönüşü: {:.3} rad", r.parent_spin));
                    ui.monospace(format!("çocuğun kat ettiği yol: {:.3}", r.child_path));
                    ui.monospace(format!("düşenin indiği yükseklik: {:.3}", r.fall));
                    ui.separator();
                    if r.child_path < 0.001 && r.my_frames > 30 {
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(230, 120, 80),
                            "-> yayılım KOŞMUYOR: set_update basit sahnenin kancasını ezdi",
                        );
                    } else if r.my_frames > 30 {
                        ui.colored_label(
                            gizmo::egui::Color32::from_rgb(120, 200, 130),
                            "-> yayılım koşuyor: motorun işi ve benimki bir arada",
                        );
                    }
                    ui.separator();
                    ui.label("set_update DEĞİŞTİRİR, zincirlemez.");
                    ui.label("add_update_hook ekler — motorun kancası yerinde kalır.");
                });
            });
    })
    .run()
    .expect("uygulama çalıştırılamadı");
}
