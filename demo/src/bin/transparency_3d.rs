//! # Saydamlık ve alfa kesme
//!
//! Aynı alfa değeri, dört farklı davranış. Bir sistem sahnedeki BÜTÜN malzemelerin alfasını
//! zamanla 0 ile 1 arasında gezdiriyor; nesneler buna türüne göre tepki veriyor:
//!
//!   * **Opak** — alfayı hiç dinlemez, hep görünür (zemin ve kırmızı küre).
//!   * **Kesme** (`alpha_cutoff`) — eşiğin altında tamamen kaybolur, üstünde tamamen görünür:
//!     yumuşak geçiş yok, ani. İki küre iki farklı eşikle (0,5 ve 0,1), o yüzden farklı anlarda
//!     belirip kayboluyorlar.
//!   * **Harmanlama** (`is_transparent`) — düzgünce solup geri geliyor.
//!
//! ## Motorun modları, ve kesmenin hangi yolda çalıştığı
//!
//! Gizmo'da harmanlama [`Material::is_transparent`], kesme [`Material::alpha_cutoff`];
//! **alfayı MSAA örneklerine dağıtan mod (alpha-to-coverage) yok**, o yüzden onun yerinde
//! burada ikinci bir kesme küpü duruyor.
//!
//! **Kesmenin durumu bu demoyu yazarken ölçüldü ve tek cümleyle özetlenemiyor:**
//!
//!   * `Material::alpha_cutoff` örnek verisiyle taşınır (`ambient.w`) ve **baked-lit** ile
//!     **ileri PBR** yollarında uygulanır. İkincisi bu demoyla geldi: ileri shader'ın sabit 0,01
//!     tabanı vardı, malzemenin kendi eşiğini hiç okumuyordu.
//!   * **Işıksız (unlit) yolda da uygulanmaz** — aynı üç satır oraya da eklenebilir, ölçüldü ve
//!     çalışıyor, ama bu demonun kapsamında bırakılmadı.
//!   * **Deferred (G-buffer) yolunda uygulanmaz.** O shader eşiği başka bir kaynaktan okuyor —
//!     malzeme uniform'undaki `uv_scale.z` — ve orayı yalnız glTF malzeme yükleyicisi dolduruyor;
//!     koddan kurulan bir malzemenin eşiği oraya hiç ulaşmıyor. Dahası derinlik ön-geçişinin
//!     (`z_prepass`) **fragment aşaması yok** (`fragment: None`), yani alfaya bakmadan derinlik
//!     yazıyor: G-buffer'da kesilen bir yüzey geride yazılmış derinlik bırakır ve orada kara bir
//!     delik görünür. Ölçüldü — bu yüzden eşiği yalnız G-buffer'a eklemek düzeltme değil, hata
//!     takası olurdu.
//!
//! Bu sahne varsayılan olarak deferred yolundan geçtiği için **kesme küreleri ve küpü yanıp
//! sönmez**; alfa düştükçe opak kalırlar. Motorun kendi belgesinin vaadi de bu değil zaten:
//! kesme, "opak geçişte kalır, derinlik yazılır, sıra önemsizdir" diye tarif ediliyor — ve bu
//! demoyla birlikte artık gerçekten öyle: alfası düşen bir kesme malzemesi eskiden sıralanan
//! saydam kovaya kayıyordu, şimdi kaymıyor.
//!
//! Sayısal notlar: zemin 6×6 `srgb(0.3,0.5,0.3)`; kesme küreleri `(±1, 0.5, −1.5)`'te
//! `srgba(0.2,0.7,0.1,·)`; harmanlanan küp `(0, 0.5, 0)`'da `srgba(0.5,0.5,1,·)`; opak küre
//! `(0, 0.5, −1.5)`'te `srgb(0.7,0.2,0.1)`; ışık `(4,8,4)`, kamera `(−2,3,5)`.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera · **Shift** — hızlı hareket

use gizmo::core::query::{Mut, Query};
use gizmo::core::system::{IntoSystemConfig, Phase, Res};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// `srgb(0.3, 0.5, 0.3)` doğrusalda — zemin.
const GROUND: Vec4 = Vec4::new(0.073_239_45, 0.214_041_14, 0.073_239_45, 1.0);
/// `srgb(0.2, 0.7, 0.1)` doğrusalda — kesme küreleri.
const MASKED: Vec3 = Vec3::new(0.033_105, 0.447_988_8, 0.010_022_7);
/// `srgb(0.5, 0.5, 1.0)` doğrusalda — harmanlanan küp.
const BLENDED: Vec3 = Vec3::new(0.214_041_14, 0.214_041_14, 1.0);
/// `srgb(0.7, 0.2, 0.1)` doğrusalda — opak küre.
const OPAQUE: Vec4 = Vec4::new(0.447_988_8, 0.033_105, 0.010_022_7, 1.0);

/// Alfası zamanla gezdirilen malzemelerin işareti (opak olanlar bunu taşımaz).
#[derive(Clone, Copy)]
struct Fading;
gizmo::core::impl_component!(Fading);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Transparency 3D", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let sphere = AssetManager::create_sphere(&scene.renderer.device, 0.5, 16, 24);
            let cube = AssetManager::create_cube(&scene.renderer.device);

            // Opak zemin.
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 6.0),
                Material::new(white.clone()).with_pbr(GROUND, 1.0, 0.0),
                MeshRenderer::new(),
            ));

            // Kesme küresi, eşik 0,5.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(1.0, 0.5, -1.5)),
                GlobalTransform::default(),
                sphere.clone(),
                Material::new(white.clone())
                    .with_pbr(MASKED.extend(1.0), 0.5, 0.0)
                    .with_alpha_cutoff(0.5),
                MeshRenderer::new(),
                Fading,
            ));

            // Işıktan etkilenmeyen kesme küresi, eşik 0,1.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-1.0, 0.5, -1.5)),
                GlobalTransform::default(),
                sphere.clone(),
                Material::new(white.clone())
                    .with_unlit(MASKED.extend(1.0))
                    .with_alpha_cutoff(0.1),
                MeshRenderer::new(),
                Fading,
            ));

            // Harmanlanan küp: alfa 1'in altına inince motor onu saydam kovaya alır.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                cube.clone(),
                Material::new(white.clone())
                    .with_pbr(BLENDED.extend(0.5), 0.5, 0.0)
                    .with_transparent(true),
                MeshRenderer::new(),
                Fading,
            ));

            // Alpha-to-coverage motorda yok; yerinde ikinci bir kesme küpü duruyor
            // (eşik 0,5) — kesme ile harmanlamanın farkı yan yana görünsün.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(-1.5, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                cube,
                Material::new(white.clone())
                    .with_pbr(Vec4::new(0.214, 1.0, 0.214, 1.0), 0.5, 0.0)
                    .with_alpha_cutoff(0.5),
                MeshRenderer::new(),
                Fading,
            ));

            // Opak küre — alfa gezinirken hiç değişmeyen referans.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.5, -1.5)),
                GlobalTransform::default(),
                sphere,
                Material::new(white).with_pbr(OPAQUE, 0.5, 0.0),
                MeshRenderer::new(),
            ));

            scene.world.spawn_bundle(PointLightBundle {
                position: Vec3::new(4.0, 8.0, 4.0),
                intensity: 25.0,
                radius: 30.0,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(-2.0, 3.0, 5.0), Vec3::ZERO);
        })
        .add_update_system(fade_transparency.in_phase(Phase::Update))
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Alfa `sin(t)/2 + 0.5` ile 0 ↔ 1 arasında gezer.
///
/// Yalnız `Fading` işaretini taşıyanlar değişiyor, çünkü Gizmo'da malzeme varlık değil
/// **bileşen**: her nesnenin kendi kopyası var ve opak olanları da dolaşmak onları da
/// saydamlaştırırdı.
fn fade_transparency(mut materials: Query<(Mut<Material>, Mut<Fading>)>, time: Res<Time>) {
    let alpha = (time.elapsed() as f32).sin() / 2.0 + 0.5;
    for (_entity, (mut material, _)) in materials.iter_mut() {
        material.albedo.w = alpha;
    }
}
